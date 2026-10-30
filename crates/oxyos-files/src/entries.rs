//! A folder's items and the changes an application makes to them (docs/adr/0027-files.mdx,
//! decision 3): listing, a new folder, renaming and moving. None of them ever replaces an item:
//! a name already taken is `Exists`. Removing is the Trash's alone (`trash`).

use crate::{valid_name, Error};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// An item of a folder, as a file manager shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    /// A folder, or a link to one.
    pub folder: bool,
    /// A symbolic link (its other fields are what it leads to, or its own when that is gone).
    pub link: bool,
    /// Bytes (0 for a folder).
    pub size: u64,
    /// When its content last changed, in milliseconds since 1970 (UTC).
    pub modified_ms: i64,
    /// Its name begins with a dot.
    pub hidden: bool,
}

impl Entry {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "path": self.path.to_string_lossy(),
            "folder": self.folder,
            "link": self.link,
            "size": self.size,
            "modified": self.modified_ms,
            "hidden": self.hidden,
        })
    }
}

/// The item at `path`, which must exist (a link that leads nowhere is an item too).
pub fn entry(path: &Path) -> Result<Entry, Error> {
    let own = std::fs::symlink_metadata(path)?;
    let link = own.file_type().is_symlink();
    let metadata = if link {
        std::fs::metadata(path).unwrap_or_else(|_| own.clone())
    } else {
        own
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Error::InvalidName)?
        .to_string();
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        });
    let folder = metadata.is_dir();
    Ok(Entry {
        hidden: name.starts_with('.'),
        name,
        path: path.to_path_buf(),
        folder,
        link,
        size: if folder { 0 } else { metadata.len() },
        modified_ms,
    })
}

/// The items of `folder`, in no particular order. Items whose names are not UTF-8 are left out
/// (an application could not name them back), as are items gone while it was read.
pub fn list(folder: &Path) -> Result<Vec<Entry>, Error> {
    let mut entries = Vec::new();
    for item in std::fs::read_dir(folder)? {
        let Ok(item) = item else { continue };
        if item.file_name().to_str().is_none() {
            continue;
        }
        match entry(&item.path()) {
            Ok(entry) => entries.push(entry),
            Err(Error::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(entries)
}

/// A new, empty folder named `name` in `parent`.
pub fn create_folder(parent: &Path, name: &str) -> Result<Entry, Error> {
    valid_name(name)?;
    let path = parent.join(name);
    std::fs::create_dir(&path)?;
    entry(&path)
}

/// `item` renamed to `name` in its folder.
pub fn rename(item: &Path, name: &str) -> Result<Entry, Error> {
    valid_name(name)?;
    let to = item.with_file_name(name);
    if to != item {
        rename_new(item, &to)?;
    }
    entry(&to)
}

/// `item` moved into `folder`, keeping its name; where it is now. A folder never goes into itself
/// or anything inside it, and an item already in `folder` stays.
pub fn move_into(item: &Path, folder: &Path) -> Result<PathBuf, Error> {
    let name = item.file_name().ok_or(Error::InvalidName)?;
    if folder.starts_with(item) {
        return Err(Error::Refused("a folder cannot go inside itself".into()));
    }
    let to = folder.join(name);
    if to != item {
        rename_new(item, &to)?;
    }
    Ok(to)
}

/// Renames `from` to `to` only if nothing is at `to` (renameat2's `RENAME_NOREPLACE`). On a file
/// system that cannot do that (it answers `EINVAL`), `to` is looked at first: a program racing to
/// create it then could be replaced, which no program of the user's own does on purpose.
pub(crate) fn rename_new(from: &Path, to: &Path) -> Result<(), Error> {
    use rustix::fs::{renameat_with, RenameFlags, CWD};
    match renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        Err(rustix::io::Errno::INVAL) => {
            if std::fs::symlink_metadata(to).is_ok() {
                return Err(Error::Exists);
            }
            Ok(std::fs::rename(from, to)?)
        }
        Err(errno) => Err(errno.into()),
    }
}
