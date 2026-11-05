//! The places a file manager's sidebar shows (docs/adr/0028-file-access.mdx): the home, its
//! favourite folders and the volumes mounted for the user; and the home's standard folders, which
//! stay where they are.

use crate::Scope;
use oxyos_preferences::user_dirs::named_folder;
use std::path::PathBuf;

/// A favourite folder: one of the XDG user directories (xdg-user-dirs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Favourite {
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Music,
    Videos,
}

impl Favourite {
    /// In the order a sidebar lists them.
    pub const ALL: [Favourite; 6] = [
        Favourite::Desktop,
        Favourite::Documents,
        Favourite::Downloads,
        Favourite::Pictures,
        Favourite::Music,
        Favourite::Videos,
    ];

    /// Its name in answers.
    pub fn key(self) -> &'static str {
        match self {
            Favourite::Desktop => "desktop",
            Favourite::Documents => "documents",
            Favourite::Downloads => "downloads",
            Favourite::Pictures => "pictures",
            Favourite::Music => "music",
            Favourite::Videos => "videos",
        }
    }

    /// Its name in `user-dirs.dirs` (`XDG_DESKTOP_DIR`...).
    fn variable(self) -> &'static str {
        match self {
            Favourite::Desktop => "XDG_DESKTOP_DIR",
            Favourite::Documents => "XDG_DOCUMENTS_DIR",
            Favourite::Downloads => "XDG_DOWNLOAD_DIR",
            Favourite::Pictures => "XDG_PICTURES_DIR",
            Favourite::Music => "XDG_MUSIC_DIR",
            Favourite::Videos => "XDG_VIDEOS_DIR",
        }
    }

    /// The folder xdg-user-dirs makes when nothing says otherwise.
    fn default_folder(self) -> &'static str {
        match self {
            Favourite::Desktop => "Desktop",
            Favourite::Documents => "Documents",
            Favourite::Downloads => "Downloads",
            Favourite::Pictures => "Pictures",
            Favourite::Music => "Music",
            Favourite::Videos => "Videos",
        }
    }
}

/// The XDG user directories a sidebar does not show, each by its variable in `user-dirs.dirs` and
/// the folder xdg-user-dirs makes when nothing says otherwise.
const UNSHOWN_USER_DIRS: [(&str, &str); 2] = [
    ("XDG_TEMPLATES_DIR", "Templates"),
    ("XDG_PUBLICSHARE_DIR", "Public"),
];

/// The text of the user's `user-dirs.dirs`, empty when there is none.
fn user_dirs_file(scope: &Scope) -> String {
    std::fs::read_to_string(scope.config_home().join("user-dirs.dirs")).unwrap_or_default()
}

/// The home's standard folders that are there, every XDG user directory (the favourites, the
/// templates and the public folder), as the items that name them (a link that is one is that
/// link) and as what they lead to, each inside the home: they stay where they are, never
/// deleted, moved, renamed, replaced or put in the Trash, as a Mac's home folders are. A
/// directory the file sets to the home itself is disabled there, as the specification says.
pub fn standard_folders(scope: &Scope) -> Vec<PathBuf> {
    let named = user_dirs_file(scope);
    let home = scope.home();
    Favourite::ALL
        .iter()
        .map(|favourite| (favourite.variable(), favourite.default_folder()))
        .chain(UNSHOWN_USER_DIRS)
        .flat_map(|(variable, default)| {
            let folder = named_folder(&named, variable, home).unwrap_or_else(|| home.join(default));
            // The item that names it, as `Scope::entry` names an item: its folder's canonical
            // path and its own name.
            let item = folder
                .parent()
                .zip(folder.file_name())
                .and_then(|(parent, name)| {
                    let item = std::fs::canonicalize(parent).ok()?.join(name);
                    std::fs::symlink_metadata(&item).ok().map(|_| item)
                });
            let canonical = std::fs::canonicalize(&folder).ok();
            [item, canonical]
        })
        .flatten()
        .filter(|folder| folder.starts_with(home) && folder != home)
        .collect()
}

/// The favourite folders that exist, each where `user-dirs.dirs` puts it (`"$HOME/..."` or an
/// absolute path, as xdg-user-dirs writes them) or at its default, and only inside the home. A
/// directory the file sets to the home itself is disabled there, as the specification says.
pub fn favourites(scope: &Scope) -> Vec<(Favourite, PathBuf)> {
    let named = user_dirs_file(scope);
    Favourite::ALL
        .iter()
        .filter_map(|&favourite| {
            let folder = named_folder(&named, favourite.variable(), scope.home())
                .unwrap_or_else(|| scope.home().join(favourite.default_folder()));
            let canonical = std::fs::canonicalize(&folder).ok()?;
            (canonical.is_dir() && canonical.starts_with(scope.home()) && canonical != scope.home())
                .then_some((favourite, canonical))
        })
        .collect()
}

/// A volume mounted for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    /// Its name: its mount point's (udisks names it from the volume's label).
    pub name: String,
    /// Its mount point.
    pub path: PathBuf,
}

/// The volumes the mount table (`/proc/self/mountinfo`'s text) has mounted directly inside one of
/// `media` (`/media/USER`, `/run/media/USER`), in the table's order, each once.
pub fn volumes(media: &[PathBuf], mountinfo: &str) -> Vec<Volume> {
    let mut found: Vec<Volume> = Vec::new();
    for line in mountinfo.lines() {
        // ID PARENT MAJOR:MINOR ROOT MOUNT-POINT OPTIONS ... - TYPE SOURCE OPTIONS
        let Some(point) = line.split(' ').nth(4).map(unescape) else {
            continue;
        };
        let path = PathBuf::from(&point);
        let inside = path
            .parent()
            .is_some_and(|parent| media.iter().any(|media| media == parent));
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if inside && !found.iter().any(|volume| volume.path == path) {
            found.push(Volume {
                name: name.to_string(),
                path,
            });
        }
    }
    found
}

/// A mount table's field, its octal escapes (`\040` for a space) decoded.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' && at + 4 <= bytes.len() {
            let digits = &bytes[at + 1..at + 4];
            if digits.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
                let value = digits
                    .iter()
                    .fold(0u32, |value, digit| value * 8 + u32::from(digit - b'0'));
                if let Ok(value) = u8::try_from(value) {
                    out.push(value);
                    at += 4;
                    continue;
                }
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
