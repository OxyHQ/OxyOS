//! An application's key-value storage on OxyOS: what `@react-native-async-storage/async-storage`
//! keeps (the Bloom host's `RNCAsyncStorage`, docs/react-native-android-adaptation.mdx), in the
//! application's own data directory (`oxyos_apps::data_directory`), as Android keeps it in the
//! application's `databases/RKStorage`.
//!
//! - **One file**, `async-storage.json` (a JSON object of string keys to string values), replaced
//!   whole on every change: written beside it, flushed, renamed over it and the directory flushed,
//!   so a crash or power loss leaves the old contents or the new, never a mix.
//! - **More than one process** (two windows of an application, each its own host): every
//!   operation holds `async-storage.lock` (`flock`: shared to read, exclusive to change) and reads
//!   the file again when another process changed it since, so no change is lost.
//! - **What AsyncStorage asks**, with its semantics: `get`, `set`, `remove`, `merge` (a JSON object
//!   deep-merged into the one kept, as Android's `mergeJsonObjects`), `keys`, `clear`.
//!
//! Nothing is kept for a process that has no application id: there is no directory to keep it in.

use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The file the entries are kept in, in the application's data directory.
pub const FILE: &str = "async-storage.json";
/// The lock every operation holds, beside it.
pub const LOCK: &str = "async-storage.lock";

#[derive(Debug)]
pub enum Error {
    /// The file or its directory could not be read, written or locked.
    Io(io::Error),
    /// The file is not a JSON object of strings: it is left as it is, never overwritten.
    Corrupt(String),
    /// A merge of a value that is not a JSON object, or into one that is not.
    Merge(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(error) => write!(f, "the application's storage: {error}"),
            Error::Corrupt(why) => write!(f, "the application's storage file is not valid: {why}"),
            Error::Merge(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Io(error)
    }
}

/// What identifies the file's contents as last read: a change by any process changes one of them.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Version {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
}

/// What this process last read of a storage: the file's version then, and its entries.
type Read = (Option<Version>, Map<String, Value>);

/// What this process last read of each storage, by its directory.
static READ: Mutex<Option<HashMap<PathBuf, Read>>> = Mutex::new(None);

/// An application's storage, in `directory` (its data directory).
pub struct Storage {
    directory: PathBuf,
}

impl Storage {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    /// The values of `keys`, in order; None for a key not kept.
    pub fn get(&self, keys: &[String]) -> Result<Vec<Option<String>>, Error> {
        self.with(false, |entries| {
            Ok(keys
                .iter()
                .map(|key| entries.get(key).and_then(Value::as_str).map(str::to_owned))
                .collect())
        })
    }

    /// Keeps each `(key, value)`, in order (a key given twice keeps the last).
    pub fn set(&self, pairs: &[(String, String)]) -> Result<(), Error> {
        self.with(true, |entries| {
            for (key, value) in pairs {
                entries.insert(key.clone(), Value::String(value.clone()));
            }
            Ok(())
        })
    }

    pub fn remove(&self, keys: &[String]) -> Result<(), Error> {
        self.with(true, |entries| {
            for key in keys {
                entries.remove(key);
            }
            Ok(())
        })
    }

    /// Deep-merges each value, a JSON object, into the one kept under its key (kept as it is when
    /// there is none), as Android's AsyncStorage does. Nothing is changed if any fails.
    pub fn merge(&self, pairs: &[(String, String)]) -> Result<(), Error> {
        self.with(true, |entries| {
            let mut merged = Vec::with_capacity(pairs.len());
            for (key, value) in pairs {
                let object = |text: &str, what: &str| match serde_json::from_str::<Value>(text) {
                    Ok(Value::Object(object)) => Ok(object),
                    _ => Err(Error::Merge(format!(
                        "{what} for key {key} is not a JSON object"
                    ))),
                };
                let new = object(value, "the value to merge")?;
                let kept = merged
                    .iter()
                    .rev()
                    .find(|(k, _)| k == key)
                    .map(|(_, v): &(String, String)| v.clone())
                    .or_else(|| entries.get(key).and_then(Value::as_str).map(str::to_owned));
                let result = match kept {
                    Some(kept) => {
                        let mut old = object(&kept, "the value kept")?;
                        deep_merge(&mut old, new);
                        old
                    }
                    None => new,
                };
                merged.push((key.clone(), Value::Object(result).to_string()));
            }
            for (key, value) in merged {
                entries.insert(key, Value::String(value));
            }
            Ok(())
        })
    }

    /// Every key kept, sorted.
    pub fn keys(&self) -> Result<Vec<String>, Error> {
        self.with(false, |entries| Ok(entries.keys().cloned().collect()))
    }

    pub fn clear(&self) -> Result<(), Error> {
        self.with(true, |entries| {
            entries.clear();
            Ok(())
        })
    }

    /// Runs `operation` on the entries under the lock (exclusive if it `changes` them, and then
    /// writes them back), with the file read again if it changed since this process read it.
    fn with<T>(
        &self,
        changes: bool,
        operation: impl FnOnce(&mut Map<String, Value>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let file = self.directory.join(FILE);
        if !changes && !self.directory.exists() {
            // Nothing was ever kept: a read creates nothing.
            return operation(&mut Map::new());
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.directory)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.directory.join(LOCK))?;
        use rustix::fs::{flock, FlockOperation};
        flock(
            &lock,
            if changes {
                FlockOperation::LockExclusive
            } else {
                FlockOperation::LockShared
            },
        )
        .map_err(io::Error::from)?;
        let mut read = READ.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let cache = read.get_or_insert_with(HashMap::new);
        let current = version(&file)?;
        let entries = match cache.get_mut(&self.directory) {
            Some((kept, entries)) if *kept == current => entries,
            _ => {
                let entries = load(&file)?;
                cache.insert(self.directory.clone(), (current, entries));
                &mut cache.get_mut(&self.directory).expect("just inserted").1
            }
        };
        if !changes {
            return operation(entries);
        }
        let mut changed = entries.clone();
        let result = operation(&mut changed)?;
        if changed != *entries {
            write(&self.directory, &file, &changed)?;
            *entries = changed;
            let written = version(&file)?;
            cache.get_mut(&self.directory).expect("read above").0 = written;
        }
        Ok(result)
    }
}

/// The file's version, None while there is no file.
fn version(file: &Path) -> Result<Option<Version>, Error> {
    match fs::metadata(file) {
        Ok(metadata) => Ok(Some(Version {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.size(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
        })),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn load(file: &Path) -> Result<Map<String, Value>, Error> {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(error.into()),
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(entries)) if entries.values().all(Value::is_string) => Ok(entries),
        Ok(_) => Err(Error::Corrupt("not an object of strings".into())),
        Err(error) => Err(Error::Corrupt(error.to_string())),
    }
}

/// Replaces `file` with `entries`: written beside it, flushed, renamed over it, the directory
/// flushed.
fn write(directory: &Path, file: &Path, entries: &Map<String, Value>) -> Result<(), Error> {
    let staged = directory.join(format!("{FILE}.{}", std::process::id()));
    let mut out = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staged)?;
    let written = out
        .write_all(Value::Object(entries.clone()).to_string().as_bytes())
        .and_then(|()| out.sync_all())
        .and_then(|()| fs::rename(&staged, file))
        .and_then(|()| File::open(directory)?.sync_all());
    if written.is_err() {
        let _ = fs::remove_file(&staged);
    }
    Ok(written?)
}

/// `new` merged into `old`: an object into an object key by key, anything else replacing.
fn deep_merge(old: &mut Map<String, Value>, new: Map<String, Value>) {
    for (key, value) in new {
        match (old.get_mut(&key), value) {
            (Some(Value::Object(kept)), Value::Object(value)) => deep_merge(kept, value),
            (_, value) => {
                old.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests;
