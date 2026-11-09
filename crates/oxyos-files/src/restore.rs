//! An item got back from a backup, written by the person's own process (docs/adr/0059-backups.mdx,
//! decision 7): the backup service streams it through a pipe ([`crate::backups::Retrieval`]), a
//! file's bytes or a folder as a tar archive, and it is written here, as the person, inside what
//! the application reaches (docs/adr/0028-file-access.mdx, decision 2). The service never writes
//! into a home as root, so no link in a home can redirect it.
//!
//! - **Staged, then put in place.** What comes is written beside where it goes, under a hidden
//!   name of its own (`.oxyos-restore-PID-N`), and only once the service said all of it was
//!   written is it put in place, in one step: an item that fails or is cancelled half way leaves
//!   nothing behind.
//! - **Keep Both, a Mac's way.** Where an item of the same name is there, the restored item goes
//!   beside it as a Mac's Time Machine names it: `NAME (original).EXT` (`notes (original).txt`,
//!   `Taxes 2026 (original)`), then `NAME (original 2).EXT` and on, the item there now untouched.
//!   *original* reads the same in English and Spanish.
//! - **Replace, when asked**: the restored item is exchanged with the one there in one step
//!   (`RENAME_EXCHANGE`, as `overwrite` is, ADR 0028 decision 4), which is then deleted. The home's
//!   standard folders, a volume's root and the application's folders are never replaced
//!   ([`crate::Scope::entry`]).
//! - **A folder's archive is read strictly** (the `tar` crate): every item must be inside the
//!   folder asked for (no absolute path, no `..`, nothing named outside it), and nothing is ever
//!   written through a link. A link is restored only when it leads to something inside the item
//!   restored; one that leads out of it, and devices, pipes and sockets, are left out and counted
//!   (`skipped`). Each file and folder gets its mode (its permission bits; never set-id) and its
//!   time of change back; it is the person's, as everything they write.

use crate::backups::{Backup, Backups, Retrieval};
use crate::entries::rename_new;
use crate::scope::{Access, Scope};
use crate::{uri, valid_name, Error};
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The word a Mac's Time Machine adds to an item restored beside one of the same name.
const KEPT_WORD: &str = "original";

/// How often progress is said, at most.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// What a restore is asked to do.
pub struct Restore<'a> {
    pub backup: &'a Backup,
    /// The item, as the application names it: where it is, or was.
    pub uri: &'a str,
    /// The folder to restore into; none: the item's own folder.
    pub to: Option<&'a str>,
    /// Replace an item of the same name there, rather than keep both.
    pub replace: bool,
}

/// The path a backup holds the item `uri` names at: the item itself (a link, not what it leads
/// to), its folders' links resolved, as restic read it. For a folder the person has open (their
/// home among them) that is the folder.
pub fn backed_up_path(scope: &Scope, item_uri: &str) -> Result<PathBuf, Error> {
    let path = uri::path_of(item_uri)?;
    let named = Path::new(&path);
    if std::fs::symlink_metadata(named).is_ok_and(|found| !found.file_type().is_symlink())
        || named.parent().is_none()
    {
        return scope.reach(&path, Access::Read);
    }
    let (Some(parent), Some(name)) = (named.parent(), named.file_name()) else {
        return Err(Error::InvalidName);
    };
    Ok(scope
        .reach(parent.to_str().unwrap_or_default(), Access::Read)?
        .join(name))
}

/// The name an item restored beside one called `name` takes: its `n`th (from 1) try.
pub fn kept_name(name: &str, folder: bool, n: u32) -> String {
    let suffix = if n <= 1 {
        format!(" ({KEPT_WORD})")
    } else {
        format!(" ({KEPT_WORD} {n})")
    };
    // A file's extension stays last; a name that starts with its only dot (`.bashrc`) has none.
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if !folder && dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    // At most 255 bytes, the stem shortened at a character's edge.
    let room = 255usize.saturating_sub(suffix.len() + extension.len());
    let mut end = stem.len().min(room);
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{suffix}{extension}", &stem[..end])
}

/// Restores an item: `{uri, skipped}`, where it is now (under the URI given, or `to`) and how
/// many items of a folder were left out. `progress(written, total)` is told as it goes;
/// `cancel` stops it at its next step.
pub fn restore(
    scope: &Scope,
    backups: &Backups,
    restore: &Restore,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<Value, Error> {
    let source = backed_up_path(scope, restore.uri)?;
    let path = uri::path_of(restore.uri)?;
    let name = Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Error::InvalidName)?
        .to_owned();
    valid_name(&name)?;
    let (folder, folder_uri) = match restore.to {
        Some(to) => (
            scope.folder(&uri::path_of(to)?, Access::Write)?,
            to.to_owned(),
        ),
        None => {
            let parent = Path::new(&path).parent().ok_or(Error::InvalidName)?;
            let parent = parent.to_str().unwrap_or_default();
            // An item of a folder deleted since goes back where it was, its folders made again
            // (as Time Machine puts it back), inside the scope.
            std::fs::create_dir_all(scope.reach(parent, Access::Write)?)?;
            (
                scope.folder(parent, Access::Write)?,
                uri::parent(restore.uri),
            )
        }
    };
    let retrieval = backups.retrieve(restore.backup, &source)?;
    let staged = staging(&folder);
    let directory = retrieval.folder;
    let written = write(&staged, retrieval, &source, cancel, progress).and_then(|skipped| {
        let placed = place(scope, &staged, &folder, &name, directory, restore.replace)?;
        Ok((placed, skipped))
    });
    match written {
        Ok((placed, skipped)) => Ok(json!({
            "uri": uri::child(&folder_uri, &placed, directory),
            "skipped": skipped,
        })),
        Err(error) => {
            if std::fs::symlink_metadata(&staged).is_ok() {
                let _ = remove(&staged);
            }
            Err(error)
        }
    }
}

/// A hidden name of its own in `folder`, for what is written before it is put in place.
fn staging(folder: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    folder.join(format!(
        ".oxyos-restore-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn remove(item: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(item)?.is_dir() {
        // A folder restored read-only is made writable to be removed.
        make_removable(item);
        std::fs::remove_dir_all(item)
    } else {
        std::fs::remove_file(item)
    }
}

/// Gives every folder inside `item` its owner's write permission, so it can be removed.
fn make_removable(item: &Path) {
    let Ok(found) = std::fs::symlink_metadata(item) else {
        return;
    };
    if !found.is_dir() {
        return;
    }
    let _ = std::fs::set_permissions(
        item,
        std::fs::Permissions::from_mode(found.permissions().mode() | 0o700),
    );
    if let Ok(items) = std::fs::read_dir(item) {
        for inside in items.flatten() {
            make_removable(&inside.path());
        }
    }
}

/// Writes what comes through the retrieval's pipe at `staged`: the number of items left out.
fn write(
    staged: &Path,
    retrieval: Retrieval,
    source: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<u64, Error> {
    let mut told = Told::new(retrieval.size, progress);
    let mut pipe = File::from(retrieval.pipe.try_clone()?);
    let modified = retrieval.modified;
    let skipped = if retrieval.folder {
        let prefix = source
            .to_str()
            .ok_or_else(|| Error::Invalid("a path that is not UTF-8".into()))?
            .trim_start_matches('/')
            .to_owned();
        extract(&mut pipe, staged, Path::new(&prefix), cancel, &mut told)?
    } else {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o666)
            .open(staged)?;
        copy(&mut pipe, &mut file, cancel, &mut told)?;
        file.sync_all()?;
        0
    };
    drop(pipe);
    retrieval.finished()?;
    if let Some(modified) = modified {
        set_modified(staged, modified);
    }
    if std::fs::symlink_metadata(staged)?.is_dir() {
        // What was written is on the disk before it is put in place.
        if let Ok(folder) = File::open(staged) {
            let _ = rustix::fs::syncfs(&folder);
        }
    }
    told.end();
    Ok(skipped)
}

/// Progress, said at most every [`PROGRESS_EVERY`] and once at the end.
struct Told<'a> {
    written: u64,
    total: Option<u64>,
    last: Instant,
    progress: &'a dyn Fn(u64, Option<u64>),
}

impl<'a> Told<'a> {
    fn new(total: Option<u64>, progress: &'a dyn Fn(u64, Option<u64>)) -> Self {
        progress(0, total);
        Told {
            written: 0,
            total,
            last: Instant::now(),
            progress,
        }
    }

    fn add(&mut self, bytes: u64) {
        self.written += bytes;
        if self.last.elapsed() >= PROGRESS_EVERY {
            self.last = Instant::now();
            (self.progress)(self.written, self.total);
        }
    }

    fn end(&self) {
        (self.progress)(self.written, self.total.or(Some(self.written)));
    }
}

/// Copies `from` into `to`, stopping when cancelled.
fn copy(
    from: &mut impl Read,
    to: &mut impl Write,
    cancel: &AtomicBool,
    told: &mut Told,
) -> Result<(), Error> {
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let read = match from.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        to.write_all(&buffer[..read])?;
        told.add(read as u64);
    }
}

fn set_modified(item: &Path, millis: i64) {
    let Ok(millis) = u64::try_from(millis) else {
        return;
    };
    let time = UNIX_EPOCH + Duration::from_millis(millis);
    set_time(item, time);
}

fn set_time(item: &Path, time: SystemTime) {
    if let Ok(file) = File::open(item) {
        let _ = file.set_modified(time);
    }
}

/// An archive's item as a path inside what is restored: relative to `prefix`, every part a name.
fn inside(path: &Path, prefix: &Path) -> Result<PathBuf, Error> {
    let outside = || {
        Error::Invalid(format!(
            "the backup's archive names {} outside the folder asked for",
            path.display()
        ))
    };
    let relative = path.strip_prefix(prefix).map_err(|_| outside())?;
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(outside());
    }
    Ok(relative.to_path_buf())
}

/// Whether a link at `at` (inside what is restored) leading to `target` leads to something
/// inside it: relative, and never above its top on the way.
fn leads_inside(at: &Path, target: &Path) -> bool {
    if target.is_absolute() {
        return false;
    }
    let mut depth = at.components().count().saturating_sub(1) as i64;
    for part in target.components() {
        match part {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

/// Makes the folders down to `relative`'s inside `top`, never through a link; the folder.
fn folders(top: &Path, relative: &Path) -> Result<PathBuf, Error> {
    let mut at = top.to_path_buf();
    for part in relative.components() {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(found) if found.is_dir() => {}
            Ok(_) => {
                return Err(Error::Invalid(format!(
                    "the backup's archive writes through {}, which is not a folder",
                    at.display()
                )))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                std::fs::create_dir(&at)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(at)
}

/// Extracts a folder's archive at `staged`: the number of items left out.
fn extract(
    pipe: &mut impl Read,
    staged: &Path,
    prefix: &Path,
    cancel: &AtomicBool,
    told: &mut Told,
) -> Result<u64, Error> {
    use tar::EntryType;
    std::fs::create_dir(staged)?;
    let mut archive = tar::Archive::new(pipe);
    let mut skipped = 0;
    // Folders' modes and times, given once what they hold is written (a read-only folder would
    // refuse it), deepest first.
    let mut made_folders: Vec<(PathBuf, u32, Option<u64>)> = Vec::new();
    for entry in archive.entries()? {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let mut entry = entry?;
        let relative = inside(&entry.path()?, prefix)?;
        let header = entry.header();
        let kind = header.entry_type();
        let mode = header.mode().unwrap_or(0o644) & 0o777;
        let mtime = header.mtime().ok();
        let Some(name) = relative.file_name().map(|name| name.to_owned()) else {
            // The folder itself.
            continue;
        };
        let parent = folders(staged, relative.parent().unwrap_or(Path::new("")))?;
        let at = parent.join(&name);
        match kind {
            EntryType::Directory => {
                folders(staged, &relative)?;
                made_folders.push((at, mode, mtime));
            }
            EntryType::Regular | EntryType::Continuous => {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&at)?;
                copy(&mut entry, &mut file, cancel, told)?;
                file.set_permissions(std::fs::Permissions::from_mode(mode))?;
                if let Some(mtime) = mtime {
                    let _ = file.set_modified(UNIX_EPOCH + Duration::from_secs(mtime));
                }
            }
            EntryType::Symlink => match entry.link_name()? {
                Some(target) if leads_inside(&relative, &target) => {
                    std::os::unix::fs::symlink(&target, &at)?;
                }
                _ => skipped += 1,
            },
            EntryType::Link => {
                let target = entry
                    .link_name()?
                    .map(|target| inside(&target, prefix))
                    .transpose()?;
                match target.map(|target| staged.join(target)) {
                    Some(target)
                        if std::fs::symlink_metadata(&target)
                            .is_ok_and(|found| found.is_file()) =>
                    {
                        std::fs::hard_link(&target, &at)?;
                    }
                    _ => skipped += 1,
                }
            }
            // Devices, pipes, sockets and what else an archive may hold are no person's files.
            _ => skipped += 1,
        }
    }
    made_folders.sort_by_key(|(path, _, _)| std::cmp::Reverse(path.components().count()));
    for (folder, mode, mtime) in made_folders {
        if let Some(mtime) = mtime {
            set_time(&folder, UNIX_EPOCH + Duration::from_secs(mtime));
        }
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(mode))?;
    }
    Ok(skipped)
}

/// Puts what was staged in `folder` under `name`: as it, when nothing is there; in place of
/// what is, when asked to replace; else beside it ([`kept_name`]). The name it took.
fn place(
    scope: &Scope,
    staged: &Path,
    folder: &Path,
    name: &str,
    directory: bool,
    replace: bool,
) -> Result<String, Error> {
    let target = folder.join(name);
    if replace {
        // Never a standard folder, a root, or what holds one.
        let target = scope.entry(target.to_str().unwrap_or_default(), Access::Write)?;
        crate::access::replace(staged, &target)?;
        return Ok(name.to_owned());
    }
    match rename_new(staged, &target) {
        Ok(()) => return Ok(name.to_owned()),
        Err(Error::Exists) => {}
        Err(error) => return Err(error),
    }
    for n in 1..=1000 {
        let kept = kept_name(name, directory, n);
        match rename_new(staged, &folder.join(&kept)) {
            Ok(()) => return Ok(kept),
            Err(Error::Exists) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(Error::Exists)
}
