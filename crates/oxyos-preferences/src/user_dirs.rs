//! The user's folders as xdg-user-dirs keeps them, in `$XDG_CONFIG_HOME/user-dirs.dirs`: where
//! the Desktop, Documents and the other well-known folders are (`XDG_DESKTOP_DIR="$HOME/Desktop"`),
//! which a file manager shows (docs/adr/0028-file-access.mdx) and screenshots are saved to
//! (docs/adr/0024-system-shortcuts.mdx, decision 10).

use std::path::{Path, PathBuf};

/// A variable's folder in `user-dirs.dirs`'s text: `NAME="$HOME/relative"` or `NAME="/absolute"`,
/// its `\"`, `\\` and `\$` unescaped; the last line naming it wins. None when no line names it, or
/// names something else (a relative path).
pub fn named_folder(file: &str, variable: &str, home: &Path) -> Option<PathBuf> {
    let mut found = None;
    for line in file.lines() {
        let line = line.trim();
        let Some(value) = line
            .strip_prefix(variable)
            .and_then(|rest| rest.trim_start().strip_prefix('='))
        else {
            continue;
        };
        let Some(quoted) = value
            .trim()
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
        else {
            continue;
        };
        let mut text = String::new();
        let mut chars = quoted.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(next) = chars.next() {
                    text.push(next);
                }
            } else {
                text.push(c);
            }
        }
        found = if let Some(rest) = text.strip_prefix("$HOME") {
            let rest = rest.trim_start_matches('/');
            Some(if rest.is_empty() {
                home.to_path_buf()
            } else {
                home.join(rest)
            })
        } else if text.starts_with('/') {
            Some(PathBuf::from(text))
        } else {
            None
        };
    }
    found
}

/// Where the folder `variable` is (`XDG_DESKTOP_DIR`), as `config_home`'s `user-dirs.dirs` names
/// it, else `default` in the home, where xdg-user-dirs makes it when nothing says otherwise. It
/// need not exist.
pub fn folder(config_home: &Path, home: &Path, variable: &str, default: &str) -> PathBuf {
    let named = std::fs::read_to_string(config_home.join("user-dirs.dirs")).unwrap_or_default();
    named_folder(&named, variable, home).unwrap_or_else(|| home.join(default))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_the_last_line_naming_it_or_the_default() {
        let home = Path::new("/home/ada");
        let file = "# written by xdg-user-dirs-update\n\
                    XDG_DESKTOP_DIR=\"$HOME/Escritorio\"\n\
                    XDG_DOCUMENTS_DIR=\"/srv/docs\"\n\
                    XDG_MUSIC_DIR=\"relative\"\n\
                    XDG_DESKTOP_DIR=\"$HOME/Mi \\\"escritorio\\\"\"\n";
        assert_eq!(
            named_folder(file, "XDG_DESKTOP_DIR", home),
            Some(PathBuf::from("/home/ada/Mi \"escritorio\""))
        );
        assert_eq!(
            named_folder(file, "XDG_DOCUMENTS_DIR", home),
            Some(PathBuf::from("/srv/docs"))
        );
        assert_eq!(named_folder(file, "XDG_MUSIC_DIR", home), None);
        assert_eq!(named_folder(file, "XDG_VIDEOS_DIR", home), None);
        assert_eq!(
            named_folder("XDG_DESKTOP_DIR=\"$HOME/\"", "XDG_DESKTOP_DIR", home),
            Some(PathBuf::from("/home/ada"))
        );
        let nowhere = Path::new("/nonexistent/config");
        assert_eq!(
            folder(nowhere, home, "XDG_DESKTOP_DIR", "Desktop"),
            PathBuf::from("/home/ada/Desktop")
        );
    }
}
