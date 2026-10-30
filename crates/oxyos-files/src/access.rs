//! `expo-file-system`'s operations on what the scope allows (docs/adr/0028-file-access.mdx), as
//! its Android implementation does them (`FileSystemFile`, `FileSystemDirectory`,
//! `CopyMoveStrategy`, `FileSystemFileHandle`), by `file://` URI.
//!
//! - **Reading** follows links, as Java's files do: what a link reaches must be in the scope.
//!   **Deleting, renaming and moving** take the item itself, a link and not where it leads.
//! - **Nothing is replaced unless asked:** creating, copying, moving and renaming refuse an item
//!   already there (`Exists`), atomically (renameat2's `RENAME_NOREPLACE`). With `overwrite`, what
//!   is there is replaced as one step: a copy is made beside it and exchanged with it
//!   (`RENAME_EXCHANGE`), so a copy that fails leaves the old item whole.
//! - **Deleting deletes**, for good, a folder with everything in it, as `expo-file-system` does;
//!   the Trash is the platform's own operation (`trash`), which a file manager uses instead.
//! - **A move between volumes** copies and then deletes what it moved, as Android does when a
//!   rename cannot.
//! - **Copies** keep links as links: a copied folder's link into somewhere else stays a link, and
//!   nothing is read through it.
//! - **Open files** (`FileHandle`) are numbered; each keeps its own offset.

use crate::scope::Access;
use crate::{entries, uri, Error, Scope};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

/// Base64 as Android's decoder takes it: the standard alphabet, its padding optional.
const LENIENT: base64::engine::GeneralPurpose = base64::engine::GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    base64::engine::GeneralPurposeConfig::new()
        .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
);

/// Milliseconds since 1970 (UTC), as JavaScript's dates count.
fn millis(time: std::io::Result<SystemTime>) -> Value {
    time.ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(Value::Null, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX).into()
        })
}

/// The path a URI names, reached for `access`, links followed.
fn reach(scope: &Scope, uri: &str, access: Access) -> Result<PathBuf, Error> {
    scope.reach(&uri::path_of(uri)?, access)
}

/// The item a URI names itself, for changing.
fn entry(scope: &Scope, uri: &str) -> Result<PathBuf, Error> {
    scope.entry(&uri::path_of(uri)?, Access::Write)
}

/// What is at `path`, links followed; none if nothing is.
fn metadata(path: &Path) -> Result<Option<Metadata>, Error> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// `expo-file-system`'s check that what is at a path is the kind its object says: a `File`
/// where a folder is, or a `Directory` where a file is, is refused.
fn kind(found: &Option<Metadata>, directory: bool) -> Result<(), Error> {
    match found {
        Some(metadata) if metadata.is_dir() && !directory => Err(Error::NotAFile),
        Some(metadata) if !metadata.is_dir() && directory => Err(Error::NotAFolder),
        _ => Ok(()),
    }
}

/// A child's name, as `expo-file-system` takes one: a single segment (no `/` nor `\`), not `.`
/// or `..`, that an item can have.
fn child_name(name: &str) -> Result<(), Error> {
    if name.contains('\\') {
        return Err(Error::InvalidName);
    }
    crate::valid_name(name)
}

/// The facts of what a URI reaches: `{exists, directory, size, modified, created}`; nothing is
/// `{exists: false}`. Times are milliseconds since 1970, or null where the file system keeps none.
pub fn stat(scope: &Scope, uri: &str) -> Result<Value, Error> {
    let path = reach(scope, uri, Access::Read)?;
    Ok(match metadata(&path)? {
        Some(found) => json!({
            "exists": true,
            "directory": found.is_dir(),
            "size": if found.is_dir() { 0 } else { found.len() },
            "modified": millis(found.modified()),
            "created": millis(found.created()),
        }),
        None => json!({ "exists": false, "directory": false, "size": 0,
                        "modified": null, "created": null }),
    })
}

/// The bytes of the files in a folder and every folder in it (links not followed), as
/// `Directory.size` counts them.
pub fn size(scope: &Scope, uri: &str) -> Result<u64, Error> {
    let path = reach(scope, uri, Access::Read)?;
    kind(&metadata(&path)?, true)?;
    fn sum(folder: &Path) -> Result<u64, Error> {
        let mut total = 0;
        for item in std::fs::read_dir(folder)? {
            let item = item?;
            let found = item.metadata()?;
            if found.is_dir() {
                total += sum(&item.path())?;
            } else if found.is_file() {
                total += found.len();
            }
        }
        Ok(total)
    }
    sum(&path)
}

/// How a file's content is read or written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// Text, UTF-8 (read: a byte that is not one is U+FFFD, as Java reads text).
    Text,
    /// Base64 (RFC 4648's standard alphabet; whitespace ignored when written).
    Base64,
    /// The bytes themselves.
    Bytes,
}

impl Encoding {
    pub fn parse(name: &str) -> Result<Encoding, Error> {
        match name {
            "utf8" | "text" => Ok(Encoding::Text),
            "base64" => Ok(Encoding::Base64),
            "bytes" => Ok(Encoding::Bytes),
            other => Err(Error::Invalid(format!("no encoding {other:?}"))),
        }
    }
}

/// A file's whole content: text as UTF-8, base64 as its text, bytes as they are.
pub fn read(scope: &Scope, uri: &str, encoding: Encoding) -> Result<Vec<u8>, Error> {
    let path = reach(scope, uri, Access::Read)?;
    kind(&metadata(&path)?, false)?;
    let bytes = std::fs::read(&path)?;
    Ok(match encoding {
        Encoding::Text => String::from_utf8_lossy(&bytes).into_owned().into_bytes(),
        Encoding::Base64 => base64::engine::general_purpose::STANDARD
            .encode(&bytes)
            .into_bytes(),
        Encoding::Bytes => bytes,
    })
}

/// Writes `content` (UTF-8 text, base64 text or bytes, as `encoding` says) into a file, made if
/// it is not there (its folder must be), its old content replaced or, with `append`, added to.
pub fn write(
    scope: &Scope,
    uri: &str,
    content: &[u8],
    encoding: Encoding,
    append: bool,
) -> Result<(), Error> {
    let path = reach(scope, uri, Access::Write)?;
    kind(&metadata(&path)?, false)?;
    let decoded;
    let bytes = match encoding {
        Encoding::Base64 => {
            let text: Vec<u8> = content
                .iter()
                .copied()
                .filter(|byte| !byte.is_ascii_whitespace())
                .collect();
            decoded = LENIENT
                .decode(&text)
                .map_err(|error| Error::Invalid(format!("the content is not base64: {error}")))?;
            &decoded
        }
        Encoding::Text | Encoding::Bytes => content,
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .append(append)
        .truncate(!append)
        .open(&path)?;
    file.write_all(bytes)?;
    Ok(())
}

/// Makes an empty file where a URI names, as `File.create` does: its folder made first with
/// `intermediates`; a file already there refused (`Exists`), or with `overwrite` emptied.
pub fn create_file(
    scope: &Scope,
    uri: &str,
    intermediates: bool,
    overwrite: bool,
) -> Result<(), Error> {
    let path = reach(scope, uri, Access::Write)?;
    let found = metadata(&path)?;
    kind(&found, false)?;
    if found.is_some() {
        if !overwrite {
            return Err(Error::Exists);
        }
        OpenOptions::new().write(true).truncate(true).open(&path)?;
        return Ok(());
    }
    if intermediates {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    Ok(())
}

/// Makes a folder where a URI names, as `Directory.create` does: with `intermediates` its
/// folders too; one already there is fine with `idempotent`, refused (`Exists`) without
/// `overwrite`, and with it deleted, with all it holds, and made again empty.
pub fn create_directory(
    scope: &Scope,
    uri: &str,
    intermediates: bool,
    overwrite: bool,
    idempotent: bool,
) -> Result<(), Error> {
    let path = reach(scope, uri, Access::Write)?;
    let found = metadata(&path)?;
    kind(&found, true)?;
    if found.is_some() {
        if idempotent {
            return Ok(());
        }
        if !overwrite {
            return Err(Error::Exists);
        }
        // What is replaced is an item the application may delete: never a root's folder.
        let item = entry(scope, uri)?;
        remove(&item)?;
    }
    if intermediates {
        std::fs::create_dir_all(&path)?;
    } else {
        std::fs::create_dir(&path)?;
    }
    Ok(())
}

/// Makes an empty file or folder named `name` in a folder, as `Directory.createFile` and
/// `createDirectory` do (one of that kind already there is it); its URI.
pub fn create_child(
    scope: &Scope,
    folder_uri: &str,
    name: &str,
    directory: bool,
) -> Result<String, Error> {
    child_name(name)?;
    let folder = reach(scope, folder_uri, Access::Write)?;
    match metadata(&folder)? {
        None => return Err(Error::NotFound),
        Some(found) if !found.is_dir() => return Err(Error::NotAFolder),
        Some(_) => {}
    }
    let path = folder.join(name);
    let found = metadata(&path)?;
    kind(&found, directory)?;
    if found.is_none() {
        if directory {
            std::fs::create_dir(&path)?;
        } else {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
        }
    }
    Ok(uri::child(folder_uri, name, directory))
}

/// Deletes an item for good: a file, a link (not what it leads to), or a folder with all it
/// holds.
pub fn delete(scope: &Scope, uri: &str) -> Result<(), Error> {
    let item = entry(scope, uri)?;
    std::fs::symlink_metadata(&item)?;
    remove(&item)
}

/// Removes an item that is there, a folder with what it holds (links inside not followed).
fn remove(item: &Path) -> Result<(), Error> {
    if std::fs::symlink_metadata(item)?.is_dir() {
        std::fs::remove_dir_all(item)?;
    } else {
        std::fs::remove_file(item)?;
    }
    Ok(())
}

/// A copy's or a move's two ends, as `expo-file-system` objects name them.
#[derive(Clone, Debug)]
pub struct Relocation<'a> {
    pub uri: &'a str,
    /// Whether the object moved or copied is a `Directory`.
    pub directory: bool,
    pub to: &'a str,
    /// Whether the destination is a `Directory`.
    pub to_directory: bool,
    pub overwrite: bool,
}

/// Where a copy or a move goes, as Android's `CopyMoveStrategy.LocalFile` decides it: into a
/// destination folder that exists, under the item's name; a folder to a folder that does not
/// exist yet, as it; a file to a file, as it. The destination's folder must exist
/// (`NoDestination`); a folder never goes to a file. The target's path (the item itself, for
/// the scope) and its URI.
fn target(
    scope: &Scope,
    relocation: &Relocation,
    source: &Path,
    source_is_dir: bool,
) -> Result<(PathBuf, String), Error> {
    if source_is_dir && !relocation.to_directory {
        return Err(Error::Invalid(
            "a folder cannot be copied or moved to a file".into(),
        ));
    }
    let to_path = uri::path_of(relocation.to)?;
    let to = scope.reach(&to_path, Access::Write)?;
    let found = metadata(&to)?;
    kind(&found, relocation.to_directory)?;
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Error::InvalidName)?;
    let parent_exists = |path: &Path| path.parent().is_some_and(Path::is_dir);
    let into = if relocation.to_directory {
        match found {
            Some(_) => true,
            None if source_is_dir && parent_exists(&to) => false,
            None => return Err(Error::NoDestination),
        }
    } else if parent_exists(&to) {
        false
    } else {
        return Err(Error::NoDestination);
    };
    if into {
        let item = scope.entry(&format!("{to_path}/{name}"), Access::Write)?;
        Ok((item, uri::child(relocation.to, name, source_is_dir)))
    } else {
        let item = scope.entry(&to_path, Access::Write)?;
        let mut target_uri = relocation.to.trim_end_matches('/').to_string();
        if source_is_dir {
            target_uri.push('/');
        }
        Ok((item, target_uri))
    }
}

/// The source of a copy or a move, of the kind its object says, which must be there.
fn source(path: PathBuf, directory: bool) -> Result<(PathBuf, bool), Error> {
    let found = metadata(&path)?;
    kind(&found, directory)?;
    let found = found.ok_or(Error::NotFound)?;
    Ok((path, found.is_dir()))
}

/// Copies an item as `File.copy` and `Directory.copy` do; the copy's URI.
pub fn copy(scope: &Scope, relocation: &Relocation) -> Result<String, Error> {
    let (from, is_dir) = source(
        reach(scope, relocation.uri, Access::Read)?,
        relocation.directory,
    )?;
    let (to, to_uri) = target(scope, relocation, &from, is_dir)?;
    if to == from {
        return if relocation.overwrite {
            Ok(to_uri)
        } else {
            Err(Error::Exists)
        };
    }
    if to.starts_with(&from) {
        return Err(Error::Refused("a folder cannot go inside itself".into()));
    }
    copy_into_place(&from, &to, relocation.overwrite)?;
    Ok(to_uri)
}

/// Moves an item as `File.move` and `Directory.move` do (the item itself: a link moves as a
/// link); its URI there.
pub fn relocate(scope: &Scope, relocation: &Relocation) -> Result<String, Error> {
    let item = entry(scope, relocation.uri)?;
    let own = std::fs::symlink_metadata(&item)?;
    let directory = own.is_dir();
    if !own.file_type().is_symlink() {
        kind(&Some(own), relocation.directory)?;
    }
    let (to, to_uri) = target(scope, relocation, &item, directory)?;
    if to == item {
        return if relocation.overwrite {
            Ok(to_uri)
        } else {
            Err(Error::Exists)
        };
    }
    if to.starts_with(&item) {
        return Err(Error::Refused("a folder cannot go inside itself".into()));
    }
    let moved = if relocation.overwrite {
        replace(&item, &to)
    } else {
        entries::rename_new(&item, &to)
    };
    match moved {
        Err(Error::CrossDevice) => {
            copy_into_place(&item, &to, relocation.overwrite)?;
            remove(&item)?;
        }
        other => other?,
    }
    Ok(to_uri)
}

/// Renames an item in its folder, never over another (`Exists`); its new URI.
pub fn rename(scope: &Scope, uri: &str, directory: bool, name: &str) -> Result<String, Error> {
    child_name(name)?;
    let item = entry(scope, uri)?;
    let own = std::fs::symlink_metadata(&item)?;
    if !own.file_type().is_symlink() {
        kind(&Some(own), directory)?;
    }
    let to = item.with_file_name(name);
    if to != item {
        entries::rename_new(&item, &to)?;
    }
    Ok(uri::child(&uri::parent(uri), name, directory))
}

/// What a folder holds: each item's URI (under the folder's URI as given) and whether it is a
/// folder (links followed), by name. Names that are not UTF-8 are left out.
pub fn list(scope: &Scope, uri: &str) -> Result<Value, Error> {
    let path = reach(scope, uri, Access::Read)?;
    match metadata(&path)? {
        None => return Err(Error::NotFound),
        Some(found) if !found.is_dir() => return Err(Error::NotAFolder),
        Some(_) => {}
    }
    let mut items = Vec::new();
    for item in std::fs::read_dir(&path)? {
        let item = item?;
        let Some(name) = item.file_name().to_str().map(String::from) else {
            continue;
        };
        let directory = std::fs::metadata(item.path()).is_ok_and(|found| found.is_dir());
        items.push((name, directory));
    }
    items.sort();
    Ok(items
        .into_iter()
        .map(|(name, directory)| {
            json!({ "uri": uri::child(uri, &name, directory), "isDirectory": directory })
        })
        .collect())
}

/// A file opened for reading as a request's body (a multipart form's file part): what a URI
/// reaches, a file.
pub fn open_read(scope: &Scope, uri: &str) -> Result<File, Error> {
    let path = reach(scope, uri, Access::Read)?;
    match metadata(&path)? {
        None => Err(Error::NotFound),
        Some(found) if found.is_dir() => Err(Error::NotAFile),
        Some(_) => Ok(File::open(&path)?),
    }
}

/// A file's content's digest, in lowercase hexadecimal.
pub fn digest(scope: &Scope, uri: &str, algorithm: &str) -> Result<String, Error> {
    use sha2::Digest;
    let path = reach(scope, uri, Access::Read)?;
    kind(&metadata(&path)?, false)?;
    let mut file = File::open(&path)?;
    fn hex(digest: &[u8]) -> String {
        use std::fmt::Write;
        digest.iter().fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
    }
    fn run<D: sha2::Digest>(file: &mut File, mut digest: D) -> Result<String, Error> {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                return Ok(hex(&digest.finalize()));
            }
            digest.update(&buffer[..read]);
        }
    }
    match algorithm {
        "MD5" => run(&mut file, md5::Md5::new()),
        "SHA-1" => run(&mut file, sha1::Sha1::new()),
        "SHA-256" => run(&mut file, sha2::Sha256::new()),
        "SHA-384" => run(&mut file, sha2::Sha384::new()),
        "SHA-512" => run(&mut file, sha2::Sha512::new()),
        other => Err(Error::Invalid(format!(
            "Unsupported digest algorithm: '{other}'."
        ))),
    }
}

/// The space of the file system the application's documents are on (the home's, without an
/// application): its size, and what the user may still use (`statvfs`'s `f_bavail`).
pub fn disk_space(scope: &Scope) -> Result<Value, Error> {
    let folder = scope
        .application()
        .map_or_else(|| scope.home().to_path_buf(), |app| app.documents.clone());
    let stats = rustix::fs::statvfs(&folder)?;
    Ok(json!({
        "total": stats.f_blocks.saturating_mul(stats.f_frsize),
        "available": stats.f_bavail.saturating_mul(stats.f_frsize),
    }))
}

/// The application's folders as `expo-file-system`'s constants name them: URIs ending in `/`, or
/// null without an application (or a bundle).
pub fn paths(scope: &Scope) -> Value {
    let application = scope.application();
    let folder =
        |path: Option<&PathBuf>| path.map_or(Value::Null, |path| uri::of(path, true).into());
    json!({
        "documentDirectory": folder(application.map(|app| &app.documents)),
        "cacheDirectory": folder(application.map(|app| &app.cache)),
        "bundleDirectory": folder(application.and_then(|app| app.bundle.as_ref())),
    })
}

/// A copy of `from` (a file, a link or a folder with what it holds) at `to`, made beside it under
/// a name of its own and then put in place: never over an item there unless `overwrite`, and
/// then exchanged with it, so a copy that fails leaves what was there whole.
fn copy_into_place(from: &Path, to: &Path, overwrite: bool) -> Result<(), Error> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = to.parent().ok_or(Error::NoDestination)?;
    let name = to
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Error::InvalidName)?;
    let staged = parent.join(format!(
        ".{name}.oxyos-copy-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let placed = copy_tree(from, &staged).and_then(|()| {
        if overwrite {
            replace(&staged, to)
        } else {
            entries::rename_new(&staged, to)
        }
    });
    if placed.is_err() && std::fs::symlink_metadata(&staged).is_ok() {
        let _ = remove(&staged);
    }
    placed
}

/// Copies an item to `to`, where nothing is: a file's content and mode, a link as a link, a
/// folder and everything in it.
fn copy_tree(from: &Path, to: &Path) -> Result<(), Error> {
    let own = std::fs::symlink_metadata(from)?;
    if own.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)?;
    } else if own.is_dir() {
        std::fs::create_dir(to)?;
        for item in std::fs::read_dir(from)? {
            let item = item?;
            copy_tree(&item.path(), &to.join(item.file_name()))?;
        }
        std::fs::set_permissions(to, own.permissions())?;
    } else {
        std::fs::copy(from, to)?;
    }
    Ok(())
}

/// Puts `from` at `to`, replacing what is there in one step: exchanged with it
/// (`RENAME_EXCHANGE`), which is then deleted. A file system that cannot exchange (`EINVAL`)
/// has what is there deleted first.
fn replace(from: &Path, to: &Path) -> Result<(), Error> {
    use rustix::fs::{renameat_with, RenameFlags, CWD};
    if std::fs::symlink_metadata(to).is_err() {
        return entries::rename_new(from, to);
    }
    match renameat_with(CWD, from, CWD, to, RenameFlags::EXCHANGE) {
        Ok(()) => remove(from),
        Err(rustix::io::Errno::INVAL) => {
            remove(to)?;
            entries::rename_new(from, to)
        }
        Err(errno) => Err(errno.into()),
    }
}

/// How an open file may be used, as `FileMode` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `r`: read; the file must be there.
    Read,
    /// `rw`: read and write, made if it is not there.
    ReadWrite,
    /// `w`: write from the start, made if it is not there.
    Write,
    /// `wa`: write, from its end.
    Append,
    /// `wt`: write, emptied first.
    Truncate,
}

impl Mode {
    pub fn parse(name: &str) -> Result<Mode, Error> {
        match name {
            "r" => Ok(Mode::Read),
            "rw" => Ok(Mode::ReadWrite),
            "w" => Ok(Mode::Write),
            "wa" => Ok(Mode::Append),
            "wt" => Ok(Mode::Truncate),
            other => Err(Error::Invalid(format!("no file mode {other:?}"))),
        }
    }

    fn reads(self) -> bool {
        matches!(self, Mode::Read | Mode::ReadWrite)
    }
}

struct Handle {
    file: File,
    mode: Mode,
    offset: u64,
}

/// The files an application has open, by number.
#[derive(Default)]
pub struct Handles {
    next: AtomicU64,
    open: Mutex<HashMap<u64, Mutex<Handle>>>,
}

impl Handles {
    /// Opens a file as `File.open(mode)` does; its handle's number.
    pub fn open(&self, scope: &Scope, uri: &str, mode: Mode) -> Result<u64, Error> {
        let access = if mode == Mode::Read {
            Access::Read
        } else {
            Access::Write
        };
        let path = reach(scope, uri, access)?;
        kind(&metadata(&path)?, false)?;
        let file = match mode {
            Mode::Read => File::open(&path)?,
            Mode::ReadWrite => OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)?,
            Mode::Write | Mode::Append => OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)?,
            Mode::Truncate => OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)?,
        };
        let offset = if mode == Mode::Append {
            file.metadata()?.len()
        } else {
            0
        };
        let number = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.open
            .lock()
            .expect("lock")
            .insert(number, Mutex::new(Handle { file, mode, offset }));
        Ok(number)
    }

    fn with<T>(
        &self,
        number: u64,
        then: impl FnOnce(&mut Handle) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let open = self.open.lock().expect("lock");
        let handle = open
            .get(&number)
            .ok_or_else(|| Error::Invalid("the file handle is closed".into()))?;
        let mut handle = handle.lock().expect("lock");
        then(&mut handle)
    }

    /// Up to `length` bytes from the handle's offset, which moves past them; fewer at the end.
    pub fn read(&self, number: u64, length: u64) -> Result<Vec<u8>, Error> {
        self.with(number, |handle| {
            if !handle.mode.reads() {
                return Err(Error::Invalid(
                    "Cannot read. File opened in write-only mode.".into(),
                ));
            }
            let size = handle.file.metadata()?.len();
            let wanted = length
                .min(size.saturating_sub(handle.offset))
                .min(i32::MAX as u64);
            let mut bytes = vec![0u8; usize::try_from(wanted).unwrap_or(0)];
            let mut read = 0;
            while read < bytes.len() {
                let got = handle
                    .file
                    .read_at(&mut bytes[read..], handle.offset + read as u64)?;
                if got == 0 {
                    break;
                }
                read += got;
            }
            bytes.truncate(read);
            handle.offset += read as u64;
            Ok(bytes)
        })
    }

    /// Writes `bytes` at the handle's offset, which moves past them.
    pub fn write(&self, number: u64, bytes: &[u8]) -> Result<(), Error> {
        self.with(number, |handle| {
            if handle.mode == Mode::Read {
                return Err(Error::Invalid(
                    "Cannot write. File opened in read-only mode.".into(),
                ));
            }
            handle.file.write_all_at(bytes, handle.offset)?;
            handle.offset += bytes.len() as u64;
            Ok(())
        })
    }

    pub fn offset(&self, number: u64) -> Result<u64, Error> {
        self.with(number, |handle| Ok(handle.offset))
    }

    /// Moves the handle's offset (past the end, the next write extends the file there).
    pub fn seek(&self, number: u64, offset: u64) -> Result<(), Error> {
        self.with(number, |handle| {
            handle.offset = offset;
            Ok(())
        })
    }

    pub fn size(&self, number: u64) -> Result<u64, Error> {
        self.with(number, |handle| Ok(handle.file.metadata()?.len()))
    }

    /// Closes a handle; one already closed stays so.
    pub fn close(&self, number: u64) {
        self.open.lock().expect("lock").remove(&number);
    }
}
