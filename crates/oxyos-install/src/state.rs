//! What the install service keeps, in its own directory (`/var/lib/oxyos/apps`,
//! docs/adr/0030-installing-applications.mdx, decision 6):
//!
//! - `objects/<aa>/<sha256>`: the content-addressed files, read-only (0444), shared by every
//!   version and account that has them;
//! - `packages/<id>/<versionCode>/`: a version's tree, hard links into `objects/`, its manifest,
//!   its developer's signature and its compiled `index.hbc`, assembled whole in a hidden
//!   directory beside it before anything names it, once for the machine;
//! - `installed/<id>.json`: what is the machine's about an application: the developer's key it
//!   pinned, which its lineage moves, so an account installing later is never handed another;
//!   a version's own facts are its manifest's, read from its tree;
//! - `users/<user>/<id>.json`: an account's install: its current version, the one before (kept
//!   until the current one shows a window), one staged for it, the releases it rolled back, and
//!   its decisions on capabilities;
//! - `users/<user>/applications/<id>.desktop`: the entry of that account's current version, for
//!   the registry;
//!
//!   both owned by root with the account's group, 0640, in directories of 0750, so no other
//!   account reads them. An account's cost is a record and an entry, never another copy of the
//!   files or the bytecode;
//! - `store/`: the store's TUF metadata as last verified;
//! - `staging/`: files being fetched and versions being compiled, the service's alone (0700).
//!
//! Every file is written whole: to a new file beside it, flushed, renamed over the old one, its
//! directory flushed, so a reader or a crash sees the old file or the new one.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Where the install service keeps applications.
pub const STATE_DIRECTORY: &str = "/var/lib/oxyos/apps";

/// Flushes a directory, so a rename or a new name in it is on the disk.
pub fn sync_directory(directory: &Path) -> io::Result<()> {
    File::open(directory)?.sync_all()
}

/// Writes `bytes` to `directory/name` whole, with `mode` and, when given, its owner and group.
pub fn write_file(
    directory: &Path,
    name: &str,
    bytes: &[u8],
    mode: u32,
    owner: Option<(u32, u32)>,
) -> io::Result<()> {
    let new = directory.join(format!(".{name}.new"));
    let _ = fs::remove_file(&new);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(&new)?;
    file.write_all(bytes)?;
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    if let Some((uid, gid)) = owner {
        rustix::fs::fchown(
            &file,
            Some(rustix::fs::Uid::from_raw(uid)),
            Some(rustix::fs::Gid::from_raw(gid)),
        )?;
    }
    file.sync_all()?;
    fs::rename(&new, directory.join(name))?;
    sync_directory(directory)
}

/// Makes `path` a directory of `mode` if it is not one; refuses a link or a file there.
pub fn ensure_directory(path: &Path, mode: u32) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_dir() => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} is not a directory", path.display()),
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .mode(mode)
                .create(path)
                .or_else(|e| match e.kind() {
                    io::ErrorKind::AlreadyExists => Ok(()),
                    _ => Err(e),
                })?;
            fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// The service's directory and the paths in it.
#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    pub fn objects(&self) -> PathBuf {
        self.root.join("objects")
    }
    pub fn packages(&self) -> PathBuf {
        self.root.join("packages")
    }
    pub fn installed(&self) -> PathBuf {
        self.root.join("installed")
    }
    pub fn users(&self) -> PathBuf {
        self.root.join("users")
    }
    /// An account's directory: its records.
    pub fn user(&self, user: &str) -> PathBuf {
        self.users().join(user)
    }
    /// An account's entries.
    pub fn user_applications(&self, user: &str) -> PathBuf {
        self.user(user).join("applications")
    }
    pub fn store(&self) -> PathBuf {
        self.root.join("store")
    }
    pub fn staging(&self) -> PathBuf {
        self.root.join("staging")
    }

    /// An object's path: `objects/<first two hex digits>/<sha256 in hex>`.
    pub fn object(&self, sha256: &oxyos_package::Sha256) -> PathBuf {
        let hex = sha256.hex();
        self.objects().join(&hex[..2]).join(hex)
    }

    /// A version's directory.
    pub fn version(&self, id: &str, version_code: u64) -> PathBuf {
        self.packages().join(id).join(version_code.to_string())
    }

    /// The service's directories, made if they are not there.
    pub fn ensure(&self) -> io::Result<()> {
        ensure_directory(&self.root, 0o755)?;
        for (directory, mode) in [
            (self.objects(), 0o755),
            (self.packages(), 0o755),
            (self.installed(), 0o755),
            (self.users(), 0o755),
            (self.store(), 0o755),
            (self.staging(), 0o700),
        ] {
            ensure_directory(&directory, mode)?;
        }
        Ok(())
    }
}

/// Why a version staged for an account waits for that account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Consent {
    /// Capabilities shown at install that the account's current version does not declare.
    pub capabilities: Vec<String>,
    /// It is signed by another key of the developer's lineage than the account's current one.
    pub key_rotation: bool,
}

/// A version assembled for an account but not yet its current one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Staged {
    pub version_code: u64,
    /// What it waits for: the account's consent; none, only for the account's running copy of
    /// its current version to stop.
    pub consent: Option<Consent>,
}

/// `installed/<id>.json`: what is the machine's about an application.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Installed {
    pub format: u32,
    pub id: String,
    /// The developer's key the machine pinned (base64url), as the lineage moved it.
    pub pinned_key: String,
}

/// What an account decided of a capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Granted,
    Denied,
    /// Asked at first use, not yet asked.
    Undecided,
}

/// `users/<user>/<id>.json`: an account's install.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserInstall {
    pub format: u32,
    pub id: String,
    pub uid: u32,
    pub user: String,
    /// The version the account runs.
    pub current: u64,
    /// Whether a session of the account has shown a window of it.
    pub shown: bool,
    /// The account's launches of it that exited before a window, while it never showed one.
    pub unshown_exits: u32,
    /// The version before, kept until the current one has shown a window.
    pub previous: Option<u64>,
    pub staged: Option<Staged>,
    /// Releases (by version code) the account rolled back: never offered to it again.
    pub bad: Vec<u64>,
    pub capabilities: BTreeMap<String, Decision>,
}

pub const FORMAT: u32 = 1;

/// Reads a record, or `None` if there is none.
pub fn read_record<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {e}", path.display()),
            )
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Writes a record whole.
pub fn write_record<T: Serialize>(
    directory: &Path,
    name: &str,
    record: &T,
    mode: u32,
    owner: Option<(u32, u32)>,
) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
    bytes.push(b'\n');
    write_file(directory, name, &bytes, mode, owner)
}
