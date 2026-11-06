//! Files dropped on an application's icon in the dock, opened with it (docs/adr/0034-the-dock.mdx,
//! the shell's OpenFiles): what the program running the compositor opens of such a request. Only
//! the person's own drop opens anything; each `file://` file the person's scope reaches and that
//! exists is opened with the application by its media type, which the launcher checks the
//! application declares; anything else is passed over and said in the log.

use oxyos_compositor_core::OpenFilesRequest;
use oxyos_files::{Access, Scope};
use std::path::PathBuf;

/// A file to open: its path, as the scope reached it, and its media type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileToOpen {
    pub path: PathBuf,
    pub kind: &'static str,
}

/// A file's media type as the platform names it: a folder's `inode/directory`, else its name's
/// (oxyos-files' table), else `application/octet-stream`.
pub fn media_type(path: &std::path::Path) -> &'static str {
    if path.is_dir() {
        return "inode/directory";
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(oxyos_files::mime::of_name)
        .unwrap_or("application/octet-stream")
}

/// The files of `request` to open with its application: none unless it cited the shell's input
/// and there is a scope; of its URIs, those the scope reaches for reading that exist, each with
/// its type. `log` is told each one passed over and why.
pub fn files_to_open(
    scope: Option<&Scope>,
    request: &OpenFilesRequest,
    log: &mut dyn FnMut(String),
) -> Vec<FileToOpen> {
    if !request.with_input {
        log(format!(
            "files not opened with {}: the request cited no input of the shell's",
            request.application
        ));
        return Vec::new();
    }
    let Some(scope) = scope else {
        log("files not opened: no scope of the person's files".into());
        return Vec::new();
    };
    request
        .uris
        .iter()
        .filter_map(|uri| {
            let reached = oxyos_files::uri::path_of(uri)
                .and_then(|path| scope.reach(&path, Access::Read))
                .and_then(|path| {
                    std::fs::metadata(&path)?;
                    Ok(path)
                });
            match reached {
                Ok(path) => Some(FileToOpen {
                    kind: media_type(&path),
                    path,
                }),
                Err(error) => {
                    log(format!("{uri} not opened: {error:?}"));
                    None
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("oxyos-open-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_the_persons_drop_opens_and_only_files_the_scope_reaches() {
        let home = scratch("home");
        let mountinfo = home.join("mountinfo");
        std::fs::write(&mountinfo, "").unwrap();
        std::fs::write(home.join("foto.png"), b"png").unwrap();
        std::fs::create_dir_all(home.join("Fotos")).unwrap();
        let scope = Scope::new(
            &home,
            &home.join(".local/share"),
            &home.join(".config"),
            Vec::new(),
            mountinfo,
            rustix::process::getuid().as_raw(),
        )
        .unwrap();
        let uri = |path: PathBuf| oxyos_files::uri::of(&path, path.is_dir());
        let request = |with_input| OpenFilesRequest {
            application: "so.oxy.allo".into(),
            uris: vec![
                uri(home.join("foto.png")),
                uri(home.join("Fotos")),
                // Not there, outside the home, not a file URI.
                uri(home.join("gone.txt")),
                "file:///etc/passwd".into(),
                "https://oxy.so/a.png".into(),
            ],
            with_input,
        };
        let mut said = Vec::new();
        let files = files_to_open(Some(&scope), &request(true), &mut |line| said.push(line));
        let canonical = |path: PathBuf| std::fs::canonicalize(path).unwrap();
        assert_eq!(
            files,
            [
                FileToOpen {
                    path: canonical(home.join("foto.png")),
                    kind: "image/png"
                },
                FileToOpen {
                    path: canonical(home.join("Fotos")),
                    kind: "inode/directory"
                },
            ]
        );
        assert_eq!(said.len(), 3, "{said:?}");
        // Without the shell's input, nothing; nor without a scope.
        let mut said = Vec::new();
        assert!(
            files_to_open(Some(&scope), &request(false), &mut |line| said.push(line)).is_empty()
        );
        assert!(files_to_open(None, &request(true), &mut |line| said.push(line)).is_empty());
        assert_eq!(said.len(), 2);
        let _ = std::fs::remove_dir_all(&home);
    }
}
