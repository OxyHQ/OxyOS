//! An application's own data (docs/adr/0061-crashes-and-resetting-an-application.mdx, decision
//! 4): the one definition of what is an application's on the person's disk, which Reset erases,
//! Settings' Storage counts and an uninstall removes (docs/adr/0030-installing-applications.mdx,
//! decision 11).
//!
//! - **Its data directory**, `oxyos/apps/<id>` under `$XDG_DATA_HOME` ([`data_directory`]): Expo's
//!   document directory, its AsyncStorage (`async-storage.json`, `oxyos-app-storage`), whatever it
//!   keeps of its own.
//! - **Its cache directory**, `oxyos/apps/<id>` under `$XDG_CACHE_HOME` ([`cache_directory`]).
//!
//! Nothing else is: the person's documents and files, their keychain, their decisions about the
//! application (its capabilities, the dock, Open at Login, its notification sounds) and its crash
//! reports are the person's, never the application's own.
//!
//! **Erasing** ([`OwnData::set_aside`]) renames each directory aside, into `.erasing` beside it (no
//! id begins with a dot, so it is never an application's), the cache first and the data last: that
//! rename is the moment the application is reset. A crash at any point leaves the application as it
//! was or reset, never with half of its data; what was set aside is deleted afterwards
//! ([`SetAside::delete`]), and whatever a crash left there is deleted at the next session's start
//! ([`sweep`]). Nothing is followed: a link in an application's directory, or the directory itself
//! a link, is removed as a link, so nothing outside it is ever touched.

use crate::{cache_directory, data_directory};
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

/// The folder beside the applications' own directories that what is being erased is moved into.
pub const ERASING: &str = ".erasing";

/// Where an application's own data is: its directories, which may not exist yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnData {
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl OwnData {
    /// `id`'s own data in the session's base directories; none for an id that is not one, or
    /// without a home.
    pub fn of(id: &str) -> Option<Self> {
        Some(Self {
            data: data_directory(id)?,
            cache: cache_directory(id)?,
        })
    }

    /// Its directories, the cache first: the order they are set aside in, the data's last.
    pub fn directories(&self) -> [&Path; 2] {
        [&self.cache, &self.data]
    }

    /// Moves each directory that is there aside, into `.erasing` beside it, renamed so that two
    /// erasures never meet: the application has none of its data from then on. A directory that is
    /// not there is nothing to erase. Fails, leaving the data where it is, if `.erasing` is not a
    /// directory of its own (a link, a file) or a rename cannot be made (another file system).
    pub fn set_aside(&self) -> io::Result<SetAside> {
        let mut moved = Vec::new();
        for directory in self.directories() {
            let Some(parent) = directory.parent() else {
                continue;
            };
            if std::fs::symlink_metadata(directory).is_err() {
                continue;
            }
            let erasing = erasing_folder(parent)?;
            let name = directory
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let aside = unique(&erasing, &name);
            std::fs::rename(directory, &aside)?;
            moved.push(aside);
        }
        Ok(SetAside(moved))
    }
}

/// What an erasure moved aside, deleted by [`SetAside::delete`] (off the session's turn: a large
/// cache takes time).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SetAside(pub Vec<PathBuf>);

impl SetAside {
    /// Deletes what was set aside, links removed as links, never followed; and each `.erasing`
    /// once nothing else is in it.
    pub fn delete(self) -> io::Result<()> {
        let mut result = Ok(());
        for path in self.0 {
            if let Err(error) = remove(&path) {
                result = Err(error);
            }
            // Another erasure's may still be in it: then it stays, for that one to remove.
            if let Some(erasing) = path.parent() {
                let _ = std::fs::remove_dir(erasing);
            }
        }
        result
    }
}

/// The folders the applications' own directories are in: `oxyos/apps` under `$XDG_DATA_HOME` and
/// under `$XDG_CACHE_HOME`.
pub fn bases() -> Vec<PathBuf> {
    // Any valid id's directory's parent is the base.
    [data_directory("x"), cache_directory("x")]
        .into_iter()
        .flatten()
        .filter_map(|directory| directory.parent().map(Path::to_path_buf))
        .collect()
}

/// Deletes what erasures a crash interrupted left in each base's `.erasing`, and the folder. Only a
/// real folder is looked into: a link named `.erasing` is removed as a link.
pub fn sweep(bases: &[PathBuf]) -> io::Result<()> {
    let mut result = Ok(());
    for base in bases {
        let erasing = base.join(ERASING);
        match std::fs::symlink_metadata(&erasing) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => result = Err(error),
            Ok(_) => {
                if let Err(error) = remove(&erasing) {
                    result = Err(error);
                }
            }
        }
    }
    result
}

/// `parent`'s `.erasing`, made private if it is not there; an error if it is there and not a folder
/// of its own.
fn erasing_folder(parent: &Path) -> io::Result<PathBuf> {
    let erasing = parent.join(ERASING);
    match std::fs::symlink_metadata(&erasing) {
        Ok(metadata) if metadata.is_dir() => Ok(erasing),
        Ok(_) => Err(io::Error::other(format!(
            "{} is not a folder: nothing is erased",
            erasing.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new().mode(0o700).create(&erasing)?;
            Ok(erasing)
        }
        Err(error) => Err(error),
    }
}

/// A name in `folder` for `name` that nothing has: `name.N`.
fn unique(folder: &Path, name: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    (0u32..)
        .map(|n| folder.join(format!("{name}.{now}.{n}")))
        .find(|path| std::fs::symlink_metadata(path).is_err())
        .expect("a free name")
}

/// Removes `path`: a folder with all it holds, anything else (a link among them) itself.
fn remove(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
