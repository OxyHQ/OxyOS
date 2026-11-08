//! The Trash, as the freedesktop.org Trash specification (1.0) keeps it
//! (docs/adr/0028-file-access.mdx), so that every desktop's file manager on the machine sees the
//! same Trash.
//!
//! - **Which Trash.** An item in the home goes to the home's (`$XDG_DATA_HOME/Trash`); an item on a
//!   mounted volume to that volume's, at its root: `.Trash/UID` when the volume has a `.Trash`
//!   that is a folder, not a link and sticky (the specification's first method), else
//!   `.Trash-UID` (its second), made with mode 0700 when needed.
//! - **Putting an item in it.** Its `.trashinfo` is created first, exclusively (so two programs
//!   never take the same name), with the item's original path (absolute in the home's Trash,
//!   relative to the volume's root in a volume's; percent-encoded) and when it was deleted, in
//!   local time; then the item is moved into `files/` under the same name. A name already taken
//!   there takes a number (`report 2.txt`). If the move fails, the info goes again.
//! - **An item in the Trash** is named by its path in `files/`, its id: only the Trash's own
//!   operations reach it ([`Scope`] refuses it everywhere else).
//! - **Restoring** moves it back to where it was, never over an item there, and removes its info.
//!   **Removing** it, or emptying the Trash, deletes it for good: the only deletion there is.
//! - **What it occupies** (docs/adr/0059-storage.mdx): each item's blocks, as `du -B1` counts them;
//!   a folder's kept in the specification's directory size cache (`directorysizes`, beside
//!   `files/`), valid while its info is the one it was measured with, since nothing changes a
//!   folder once it is in the Trash. Other desktops' file managers read and write the same cache.
//! - **Erasing what has been in it too long** (*Empty Trash Automatically*, docs/adr/0059-storage.mdx):
//!   the items deleted more than [`KEPT_FOR_MS`] ago, by their info's deletion date, go for good;
//!   one whose info gives no date is kept, since nothing says how long it has been there.

use crate::entries::{self, Entry};
use crate::scope::Access;
use crate::{Error, Scope};
use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// How long an item stays in the Trash before *Empty Trash Automatically* erases it: 30 days, as
/// a Mac keeps it.
pub const KEPT_FOR_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// A Trash directory: `files/` and `info/` inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashDir {
    /// The Trash itself.
    pub dir: PathBuf,
    /// The volume's root its paths are relative to, or none for the home's Trash (absolute).
    pub top: Option<PathBuf>,
}

impl TrashDir {
    pub fn files(&self) -> PathBuf {
        self.dir.join("files")
    }

    pub fn info(&self) -> PathBuf {
        self.dir.join("info")
    }

    /// The info file of the item named `name` in `files/`.
    fn info_of(&self, name: &str) -> PathBuf {
        self.info().join(format!("{name}.trashinfo"))
    }

    /// Makes its folders, private to the user, if they are not there.
    pub fn make(&self) -> Result<(), Error> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(self.files())?;
        builder.create(self.info())?;
        Ok(())
    }
}

/// The home's Trash.
pub fn home_trash(scope: &Scope) -> TrashDir {
    TrashDir {
        dir: scope.home_trash(),
        top: None,
    }
}

/// The Trash of the volume mounted at `top`, by the specification's first method if the volume
/// has an administrator's `.Trash` (a sticky folder, not a link), else its second.
pub fn volume_trash(scope: &Scope, top: &Path) -> TrashDir {
    let shared = top.join(".Trash");
    let usable = std::fs::symlink_metadata(&shared)
        .is_ok_and(|metadata| metadata.is_dir() && metadata.permissions().mode() & 0o1000 != 0);
    let dir = if usable {
        shared.join(scope.uid().to_string())
    } else {
        top.join(format!(".Trash-{}", scope.uid()))
    };
    TrashDir {
        dir,
        top: Some(top.to_path_buf()),
    }
}

/// Every Trash the user has now: the home's, and each mounted volume's that exists (both of a
/// volume's when both do).
pub fn trashes(scope: &Scope) -> Vec<TrashDir> {
    let mut all = vec![home_trash(scope)];
    for volume in scope.volumes() {
        for dir in [
            volume.path.join(".Trash").join(scope.uid().to_string()),
            volume.path.join(format!(".Trash-{}", scope.uid())),
        ] {
            if dir.is_dir() {
                all.push(TrashDir {
                    dir,
                    top: Some(volume.path.clone()),
                });
            }
        }
    }
    all
}

/// The Trash an item (resolved by [`Scope::item`]) goes to.
pub fn trash_for(scope: &Scope, item: &Path) -> TrashDir {
    match scope.root_of(item, &scope.volumes()) {
        Some(crate::scope::Root::Volume(top)) => volume_trash(scope, &top),
        _ => home_trash(scope),
    }
}

/// Puts `item` (resolved by [`Scope::item`]) in its Trash; its id there.
pub fn trash(scope: &Scope, item: &Path) -> Result<PathBuf, Error> {
    let trash = trash_for(scope, item);
    trash.make()?;
    let name = item
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Error::InvalidName)?;
    let original = match &trash.top {
        Some(top) => item.strip_prefix(top).unwrap_or(item).to_path_buf(),
        None => item.to_path_buf(),
    };
    let info = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        encode(&original.to_string_lossy()),
        jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S")
    );
    for n in 1u32.. {
        let candidate = numbered(name, n);
        if candidate.len() > 255 - ".trashinfo".len() {
            return Err(Error::InvalidName);
        }
        let info_path = trash.info_of(&candidate);
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&info_path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        let target = trash.files().join(&candidate);
        // An item left in files/ without its info (a program stopped between the two) keeps
        // its name.
        if std::fs::symlink_metadata(&target).is_ok() {
            drop(file);
            let _ = std::fs::remove_file(&info_path);
            continue;
        }
        let written = file
            .write_all(info.as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = written {
            let _ = std::fs::remove_file(&info_path);
            return Err(error.into());
        }
        if let Err(error) = entries::rename_new(item, &target) {
            let _ = std::fs::remove_file(&info_path);
            if matches!(error, Error::Exists) {
                continue;
            }
            return Err(error);
        }
        return Ok(target);
    }
    unreachable!("a free name is found before u32 runs out")
}

/// `name` the `n`th time it is taken: itself, then `report 2.txt`, `report 3.txt`...
fn numbered(name: &str, n: u32) -> String {
    if n == 1 {
        return name.to_string();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} {n}{}", &name[..dot], &name[dot..]),
        _ => format!("{name} {n}"),
    }
}

/// An item in a Trash.
#[derive(Clone, Debug, PartialEq)]
pub struct Trashed {
    /// Its path in the Trash's `files/`.
    pub id: PathBuf,
    /// The item as it is in the Trash (its name there may have a number).
    pub entry: Entry,
    /// Its name where it was.
    pub name: String,
    /// Where it was, if its info says.
    pub original: Option<PathBuf>,
    /// When it was put in the Trash, in milliseconds since 1970, if its info says.
    pub deleted_ms: Option<i64>,
}

impl Trashed {
    pub fn json(&self) -> serde_json::Value {
        let mut value = self.entry.json();
        value["id"] = self.id.to_string_lossy().into();
        value["name"] = self.name.clone().into();
        value["original"] = self
            .original
            .as_ref()
            .map(|path| path.to_string_lossy().into())
            .unwrap_or(serde_json::Value::Null);
        value["deleted"] = self.deleted_ms.into();
        value
    }
}

/// Everything in every Trash (an item whose info is missing has no original place).
pub fn list(scope: &Scope) -> Vec<Trashed> {
    let mut all = Vec::new();
    for trash in trashes(scope) {
        let Ok(items) = entries::list(&trash.files()) else {
            continue;
        };
        for entry in items {
            let (original, deleted_ms) = read_info(&trash, &entry.name);
            all.push(Trashed {
                id: entry.path.clone(),
                name: original
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .and_then(|name| name.to_str())
                    .unwrap_or(&entry.name)
                    .to_string(),
                original,
                deleted_ms,
                entry,
            });
        }
    }
    all
}

/// How many items every Trash holds.
pub fn count(scope: &Scope) -> usize {
    trashes(scope)
        .iter()
        .filter_map(|trash| std::fs::read_dir(trash.files()).ok())
        .map(|items| items.count())
        .sum()
}

/// What one sweep of [`erase_older`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// Items erased for good.
    pub erased: usize,
    /// When the earliest of the items left was put in the Trash (milliseconds since 1970): the
    /// next one due. None when no item left has a date.
    pub next: Option<i64>,
}

/// Erases, from every Trash, the items put in it before `before` (milliseconds since 1970, their
/// info's deletion date), and says when the earliest of the rest was put in. An item whose info
/// gives no date is kept. An item that cannot be erased is said and kept, and the others go.
pub fn erase_older(scope: &Scope, before: i64) -> Result<Swept, Error> {
    let mut swept = Swept::default();
    for trash in trashes(scope) {
        let Ok(items) = std::fs::read_dir(trash.files()) else {
            continue;
        };
        for item in items.flatten() {
            let Some(name) = item.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let (_, deleted) = read_info(&trash, &name);
            match deleted {
                Some(deleted) if deleted < before => match remove_item(&trash, &name) {
                    Ok(()) => swept.erased += 1,
                    Err(error) => eprintln!("[trash] {name} not erased: {error:?}"),
                },
                Some(deleted) => swept.next = Some(swept.next.map_or(deleted, |n| n.min(deleted))),
                None => {}
            }
        }
    }
    Ok(swept)
}

/// The directory size cache's name in a Trash (the specification's `directorysizes`).
const DIRECTORY_SIZES: &str = "directorysizes";

/// What `trash` occupies on the disk: each item's blocks, a folder's from the directory size cache
/// while its info is unchanged, else measured and kept there; or the measuring's interruption when
/// `stop` said so.
pub fn occupied(trash: &TrashDir, stop: &dyn Fn() -> bool) -> std::io::Result<u64> {
    let cache_path = trash.dir.join(DIRECTORY_SIZES);
    let cached = read_directory_sizes(&cache_path);
    let mut kept: BTreeMap<String, (u64, i64)> = BTreeMap::new();
    let mut total = 0;
    let items = match std::fs::read_dir(trash.files()) {
        Ok(items) => items,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    for item in items.flatten() {
        let Some(name) = item.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Ok(metadata) = item.metadata() else {
            continue;
        };
        if !metadata.is_dir() {
            total += crate::usage::blocks(&metadata);
            continue;
        }
        // The cache's key: when the folder's info was written, in seconds.
        let info_time = std::fs::metadata(trash.info_of(&name))
            .ok()
            .map(|info| info.mtime());
        let size = match (info_time, cached.get(&name)) {
            (Some(time), Some(&(size, at))) if at == time => size,
            _ => crate::usage::allocated(&item.path(), stop)?,
        };
        if let Some(time) = info_time {
            kept.insert(name, (size, time));
        }
        total += size;
    }
    if kept != cached {
        if let Err(error) = write_directory_sizes(&trash.dir, &kept) {
            eprintln!("[trash] {} not kept: {error}", cache_path.display());
        }
    }
    Ok(total)
}

/// The directory size cache: each line `SIZE MTIME NAME`, the name percent-encoded; a line that is
/// not one is passed over.
fn read_directory_sizes(path: &Path) -> BTreeMap<String, (u64, i64)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ' ');
            let size = parts.next()?.parse().ok()?;
            let time = parts.next()?.parse().ok()?;
            let name = decode(parts.next()?);
            (!name.is_empty() && !name.contains('/')).then_some((name, (size, time)))
        })
        .collect()
}

/// Writes the directory size cache whole, through a temporary file renamed over it, as the
/// specification asks.
fn write_directory_sizes(dir: &Path, sizes: &BTreeMap<String, (u64, i64)>) -> std::io::Result<()> {
    let mut text = String::new();
    for (name, (size, time)) in sizes {
        text.push_str(&format!("{size} {time} {}\n", encode(name)));
    }
    let staged = dir.join(format!(".{DIRECTORY_SIZES}.{}", std::process::id()));
    std::fs::write(&staged, text)?;
    std::fs::rename(&staged, dir.join(DIRECTORY_SIZES))
}

/// What the info of `name` in `trash` says: where it was, and when it was deleted.
fn read_info(trash: &TrashDir, name: &str) -> (Option<PathBuf>, Option<i64>) {
    let Ok(text) = std::fs::read_to_string(trash.info_of(name)) else {
        return (None, None);
    };
    let mut in_group = false;
    let (mut original, mut deleted) = (None, None);
    for line in text.lines() {
        let line = line.trim_end();
        if line.starts_with('[') {
            in_group = line == "[Trash Info]";
            continue;
        }
        if !in_group {
            continue;
        }
        if let Some(path) = line.strip_prefix("Path=") {
            let path = PathBuf::from(decode(path));
            original = match &trash.top {
                Some(top) if path.is_relative() => Some(top.join(path)),
                _ if path.is_absolute() => Some(path),
                _ => None,
            };
        } else if let Some(date) = line.strip_prefix("DeletionDate=") {
            deleted = date
                .parse::<jiff::civil::DateTime>()
                .ok()
                .and_then(|date| date.to_zoned(jiff::tz::TimeZone::system()).ok())
                .map(|zoned| zoned.timestamp().as_millisecond());
        }
    }
    (original, deleted)
}

/// The Trash and name an id names: a path directly in a Trash's `files/` of the user's.
fn find(scope: &Scope, id: &str) -> Result<(TrashDir, String), Error> {
    let path = Path::new(id);
    let (Some(folder), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str()))
    else {
        return Err(Error::Refused("not an item in the Trash".into()));
    };
    if name == "." || name == ".." || id.split('/').any(|part| part == "..") {
        return Err(Error::Refused("not an item in the Trash".into()));
    }
    let trash = trashes(scope)
        .into_iter()
        .find(|trash| trash.files() == folder)
        .ok_or_else(|| Error::Refused("not an item in the Trash".into()))?;
    std::fs::symlink_metadata(path)?;
    Ok((trash, name.to_string()))
}

/// Puts the item `id` back where it was; where that is. An item there already stays, and so
/// does this one (`Exists`); one whose info is gone cannot go back (`NotFound`).
pub fn restore(scope: &Scope, id: &str) -> Result<PathBuf, Error> {
    let (trash, name) = find(scope, id)?;
    let (Some(original), _) = read_info(&trash, &name) else {
        return Err(Error::NotFound);
    };
    // Back only into the home or the volume, never anywhere its info could name.
    let parent = original
        .parent()
        .and_then(|parent| parent.to_str())
        .ok_or_else(|| Error::Refused("the root".into()))?;
    let folder = scope.folder(parent, Access::Write)?;
    let target = folder.join(original.file_name().ok_or(Error::InvalidName)?);
    entries::rename_new(&trash.files().join(&name), &target)?;
    let _ = std::fs::remove_file(trash.info_of(&name));
    Ok(target)
}

/// Deletes the item `id` from the Trash, for good.
pub fn remove(scope: &Scope, id: &str) -> Result<(), Error> {
    let (trash, name) = find(scope, id)?;
    remove_item(&trash, &name)
}

fn remove_item(trash: &TrashDir, name: &str) -> Result<(), Error> {
    let path = trash.files().join(name);
    let metadata = std::fs::symlink_metadata(&path)?;
    if metadata.is_dir() {
        make_removable(&path);
        std::fs::remove_dir_all(&path)?;
    } else {
        std::fs::remove_file(&path)?;
    }
    let _ = std::fs::remove_file(trash.info_of(name));
    Ok(())
}

/// A folder in the Trash the user cannot write into (a read-only copy) can still be emptied by
/// its owner: its folders are made writable first, as file managers do.
fn make_removable(path: &Path) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if !metadata.is_dir() || metadata.uid() != rustix::process::getuid().as_raw() {
        return;
    }
    let mode = metadata.permissions().mode();
    if mode & 0o700 != 0o700 {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o700));
    }
    if let Ok(items) = std::fs::read_dir(path) {
        for item in items.flatten() {
            make_removable(&item.path());
        }
    }
}

/// Deletes everything in every Trash, for good, and the info of items no longer there; how many
/// items went.
pub fn empty(scope: &Scope) -> Result<usize, Error> {
    let mut removed = 0;
    for trash in trashes(scope) {
        if let Ok(items) = std::fs::read_dir(trash.files()) {
            for item in items.flatten() {
                if let Some(name) = item.file_name().to_str() {
                    remove_item(&trash, name)?;
                    removed += 1;
                }
            }
        }
        if let Ok(infos) = std::fs::read_dir(trash.info()) {
            for info in infos.flatten() {
                let _ = std::fs::remove_file(info.path());
            }
        }
    }
    Ok(removed)
}

/// A path as the specification stores it: percent-encoded (RFC 2396), its `/` kept.
pub fn encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()/".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A stored path decoded; a `%` not followed by two hexadecimal digits is itself.
pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |byte: u8| char::from(byte).to_digit(16);
        if bytes[at] == b'%' && at + 3 <= bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[at + 1]), hex(bytes[at + 2])) {
                out.push((high * 16 + low) as u8);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
