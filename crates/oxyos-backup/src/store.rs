//! What the service keeps, in a directory of its own (`/var/lib/oxyos/backup`, root's, mode 0700),
//! as the account service keeps its accounts ([ADR 0059](../../docs/adr/0059-backups.mdx),
//! decision 5):
//!
//! - `config.json`: the destinations, the excluded items, whether backups are automatic, and the
//!   computer's name in its snapshots;
//! - `state.json`: what each destination has (its last backup, its oldest, the space it takes,
//!   its last failure, when it was last thinned and checked);
//! - `keys/<destination>`: each destination's secrets, mode 0600: the machine's key to its
//!   repository (random, never shown) and a share's password.
//!
//! The directory must be the service's own, mode 0700, not a link; one that is anything else is
//! refused, never repaired. Each file is written whole: to a new file beside it, flushed, renamed
//! over the old one, the directory flushed, so a crash leaves the old file or the new one. Secrets
//! are overwritten in memory once used.

use crate::share::Share;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Where the service keeps what it keeps.
pub const STATE_DIRECTORY: &str = "/var/lib/oxyos/backup";
/// The folder of a destination that holds OxyOS's backups, one repository per computer in it.
pub const BACKUPS_FOLDER: &str = "OxyOS Backups";

const CONFIG: &str = "config.json";
const STATE: &str = "state.json";
const KEYS: &str = "keys";
const NEW: &str = ".new";

/// A destination's id: 16 lowercase hexadecimal digits, random.
pub fn new_id() -> io::Result<String> {
    let mut bytes = [0u8; 8];
    rustix::rand::getrandom(&mut bytes, rustix::rand::GetRandomFlags::empty())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether `id` is one [`new_id`] makes (it names files).
pub fn valid_id(id: &str) -> bool {
    id.len() == 16
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A destination's kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Kind {
    /// A disk, known by its file system's UUID, mounted by the service when it is connected.
    Disk {
        uuid: String,
        #[serde(rename = "fsType")]
        fs_type: String,
    },
    /// A share on a NAS or another computer.
    Share(Share),
}

/// Where backups go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Destination {
    pub id: String,
    /// What the person sees: the disk's label, or the share's URL.
    pub name: String,
    pub kind: Kind,
    /// The repository's folder in the destination's [`BACKUPS_FOLDER`].
    pub folder: String,
}

impl Destination {
    /// The repository's path within the destination.
    pub fn repository_path(&self) -> String {
        format!("{BACKUPS_FOLDER}/{}", self.folder)
    }
}

/// What the service is set to do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub destinations: Vec<Destination>,
    /// Absolute paths the person excluded.
    #[serde(default)]
    pub excluded: Vec<PathBuf>,
    /// Backing up every hour, on its own (Time Machine's *Back Up Automatically*).
    #[serde(default = "yes")]
    pub automatic: bool,
    /// The computer's name in its snapshots (restic's host), chosen once.
    #[serde(default)]
    pub computer: String,
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            destinations: Vec::new(),
            excluded: Vec::new(),
            automatic: true,
            computer: String::new(),
        }
    }
}

/// Why a backup failed, as the person is told it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub reason: String,
    pub at: i64,
}

/// What a destination has.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// The last backup that finished, and its snapshot.
    #[serde(default)]
    pub last_backup: Option<i64>,
    #[serde(default)]
    pub last_snapshot: Option<String>,
    /// The oldest backup kept.
    #[serde(default)]
    pub oldest_backup: Option<i64>,
    #[serde(default)]
    pub backups: u64,
    /// The bytes the repository takes, after its last backup.
    #[serde(default)]
    pub used_bytes: Option<u64>,
    /// The bytes free on the destination, when it says (a disk).
    #[serde(default)]
    pub free_bytes: Option<u64>,
    #[serde(default)]
    pub last_failure: Option<Failure>,
    /// After a failure or a skipped backup, not tried again before this (unless its disk is
    /// connected again).
    #[serde(default)]
    pub retry_at: Option<i64>,
    #[serde(default)]
    pub last_prune: Option<i64>,
    #[serde(default)]
    pub last_check: Option<i64>,
}

/// A destination's secrets.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Keys {
    /// The machine's key to the repository: 32 random bytes, hexadecimal.
    pub machine: Zeroizing<String>,
    /// A share's password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_password: Option<Zeroizing<String>>,
}

impl Keys {
    /// A new machine key.
    pub fn generate(share_password: Option<Zeroizing<String>>) -> io::Result<Self> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        rustix::rand::getrandom(&mut bytes[..], rustix::rand::GetRandomFlags::empty())?;
        let mut machine = Zeroizing::new(String::with_capacity(64));
        for byte in bytes.iter() {
            machine.push_str(&format!("{byte:02x}"));
        }
        Ok(Self {
            machine,
            share_password,
        })
    }
}

/// The service's directory.
pub struct Store {
    directory: PathBuf,
}

fn invalid(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

impl Store {
    /// The store in `directory`, made (mode 0700) if it is not there. Files a crash left half
    /// written are removed.
    pub fn open(directory: &Path) -> io::Result<Self> {
        let store = Self {
            directory: directory.to_owned(),
        };
        private_directory(directory)?;
        private_directory(&directory.join(KEYS))?;
        for folder in [directory.to_owned(), directory.join(KEYS)] {
            for entry in fs::read_dir(&folder)? {
                let name = entry?.file_name();
                if name.to_string_lossy().ends_with(NEW) {
                    fs::remove_file(folder.join(name))?;
                }
            }
        }
        Ok(store)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The configuration, the default one if none was written.
    pub fn config(&self) -> io::Result<Config> {
        read_json(&self.directory.join(CONFIG)).map(Option::unwrap_or_default)
    }

    pub fn put_config(&self, config: &Config) -> io::Result<()> {
        write_whole(&self.directory.join(CONFIG), &json(config)?)
    }

    /// Each destination's record, by its id.
    pub fn records(&self) -> io::Result<BTreeMap<String, Record>> {
        read_json(&self.directory.join(STATE)).map(Option::unwrap_or_default)
    }

    pub fn put_records(&self, records: &BTreeMap<String, Record>) -> io::Result<()> {
        write_whole(&self.directory.join(STATE), &json(records)?)
    }

    fn keys_path(&self, id: &str) -> io::Result<PathBuf> {
        if !valid_id(id) {
            return Err(invalid(format!("{id}: not a destination's id")));
        }
        Ok(self.directory.join(KEYS).join(id))
    }

    /// A destination's secrets.
    pub fn keys(&self, id: &str) -> io::Result<Option<Keys>> {
        let path = self.keys_path(id)?;
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.mode() & 0o077 != 0 || metadata.nlink() != 1 {
            return Err(invalid(format!(
                "{}: a destination's keys must be a file of the service's own, mode 0600",
                path.display()
            )));
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(512));
        file.read_to_end(&mut bytes)?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| invalid(format!("{}: {e}", path.display())))
    }

    pub fn put_keys(&self, id: &str, keys: &Keys) -> io::Result<()> {
        let path = self.keys_path(id)?;
        let bytes = Zeroizing::new(serde_json::to_vec(keys).map_err(io::Error::other)?);
        write_whole(&path, &bytes)
    }

    pub fn remove_keys(&self, id: &str) -> io::Result<()> {
        match fs::remove_file(self.keys_path(id)?) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => sync_directory(&self.directory.join(KEYS)),
        }
    }
}

fn private_directory(directory: &Path) -> io::Result<()> {
    match DirBuilder::new().mode(0o700).create(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(directory)?;
    let owner = rustix::process::geteuid().as_raw();
    if !metadata.is_dir() || metadata.uid() != owner || metadata.mode() & 0o077 != 0 {
        return Err(invalid(format!(
            "{}: the backups' directory must be a directory of the service's own, mode 0700",
            directory.display()
        )));
    }
    Ok(())
}

fn json(value: &impl Serialize) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| invalid(format!("{}: {e}", path.display()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes `bytes` to `path` whole, mode 0600: a new file beside it, flushed, renamed over it, the
/// directory flushed.
pub(crate) fn write_whole(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| invalid(format!("{}: no name", path.display())))?;
    let new = path.with_file_name(format!("{}{NEW}", name.to_string_lossy()));
    let _ = fs::remove_file(&new);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&new)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&new, path)?;
    sync_directory(path.parent().unwrap_or(Path::new(".")))
}

fn sync_directory(directory: &Path) -> io::Result<()> {
    File::open(directory)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_store_keeps_its_files_whole_and_its_secrets_private() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("backup");
        let store = Store::open(&root).unwrap();
        assert_eq!(store.config().unwrap(), Config::default());
        let id = new_id().unwrap();
        assert!(valid_id(&id));
        let config = Config {
            destinations: vec![Destination {
                id: id.clone(),
                name: "Backup".into(),
                kind: Kind::Disk {
                    uuid: "1234-ABCD".into(),
                    fs_type: "exfat".into(),
                },
                folder: "f".into(),
            }],
            excluded: vec!["/home/ada/Movies".into()],
            automatic: false,
            computer: "ada-laptop".into(),
        };
        store.put_config(&config).unwrap();
        assert_eq!(Store::open(&root).unwrap().config().unwrap(), config);

        let keys = Keys::generate(Some(Zeroizing::new("share".into()))).unwrap();
        assert_eq!(keys.machine.len(), 64);
        store.put_keys(&id, &keys).unwrap();
        let mode = fs::metadata(root.join("keys").join(&id))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let read = store.keys(&id).unwrap().unwrap();
        assert_eq!(read.machine, keys.machine);
        assert_eq!(
            read.share_password.as_deref().map(String::as_str),
            Some("share")
        );
        assert!(store.keys("../config.json").is_err());
        store.remove_keys(&id).unwrap();
        assert!(store.keys(&id).unwrap().is_none());

        // A file a crash left half written goes; a directory others may read is refused.
        fs::write(root.join("state.json.new"), b"{").unwrap();
        Store::open(&root).unwrap();
        assert!(!root.join("state.json.new").exists());
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Store::open(&root).is_err());
    }
}
