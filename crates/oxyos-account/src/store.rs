//! The enrolled accounts, one file each in the service's own directory
//! (`/var/lib/oxyos/accounts`): the account, its user and profile, and its device credential.
//!
//! - The directory is the service's alone: mode 0700, owned by the service's user, not a symbolic
//!   link. One that is anything else is refused, never repaired.
//! - Each file is mode 0600 and written whole: to a new file beside it, flushed to the disk,
//!   renamed over the old one, and the directory flushed, so a reader or a crash sees the old
//!   file or the new one, never a part. What is written is overwritten in memory afterwards.

use crate::api::{DeviceCredential, Profile};
use crate::id::{AccountId, UserName};
use crate::secret;
use serde::{Deserialize, Serialize};
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Where the service keeps enrolled accounts.
pub const STATE_DIRECTORY: &str = "/var/lib/oxyos/accounts";

/// The version of an account's file.
const VERSION: u32 = 1;
const SUFFIX: &str = ".json";
const NEW: &str = ".new";

/// An enrolled account.
#[derive(Debug, PartialEq, Eq)]
pub struct Enrollment {
    pub account: AccountId,
    pub user: UserName,
    pub uid: u32,
    pub profile: Profile,
    pub credential: DeviceCredential,
}

/// An account's file, as written.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Written<'a> {
    version: u32,
    account_id: &'a str,
    user: &'a str,
    uid: u32,
    profile: &'a Profile,
    credential: &'a DeviceCredential,
}

/// An account's file, as read.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct File1 {
    version: u32,
    account_id: String,
    user: String,
    uid: u32,
    profile: Profile,
    credential: DeviceCredential,
}

pub struct Store {
    directory: PathBuf,
}

fn invalid(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

impl Store {
    /// The store in `directory`, made (mode 0700) if it does not exist. Files a crash left half
    /// written are removed.
    pub fn open(directory: &Path) -> io::Result<Self> {
        match DirBuilder::new().mode(0o700).create(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs::symlink_metadata(directory)?;
        let owner = rustix::process::geteuid().as_raw();
        if !metadata.is_dir() || metadata.uid() != owner || metadata.mode() & 0o077 != 0 {
            return Err(invalid(format!(
                "{}: the accounts' directory must be a directory of the service's own, mode 0700",
                directory.display()
            )));
        }
        for entry in fs::read_dir(directory)? {
            let name = entry?.file_name();
            if name.to_string_lossy().ends_with(NEW) {
                fs::remove_file(directory.join(name))?;
            }
        }
        Ok(Self {
            directory: directory.to_owned(),
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    fn path(&self, user: &UserName) -> PathBuf {
        self.directory.join(format!("{user}{SUFFIX}"))
    }

    /// The account's enrollment, if it has one.
    pub fn get(&self, account: &AccountId) -> io::Result<Option<Enrollment>> {
        let path = self.path(&account.user_name());
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut bytes = Zeroizing::new(Vec::with_capacity(16 * 1024));
        file.read_to_end(&mut bytes)?;
        let stored: File1 = serde_json::from_slice(&bytes)
            .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
        if stored.version != VERSION
            || stored.account_id != account.as_str()
            || stored.user != account.user_name().as_str()
        {
            return Err(invalid(format!(
                "{}: not this account's file",
                path.display()
            )));
        }
        Ok(Some(Enrollment {
            account: account.clone(),
            user: account.user_name(),
            uid: stored.uid,
            profile: stored.profile,
            credential: stored.credential,
        }))
    }

    /// Every enrolled account's user, by its file's name.
    pub fn users(&self) -> io::Result<Vec<String>> {
        let mut users = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if let Some(user) = name.strip_suffix(SUFFIX) {
                users.push(user.to_owned());
            }
        }
        users.sort();
        Ok(users)
    }

    /// Writes the account's file whole, replacing the one it had.
    pub fn put(&self, enrollment: &Enrollment) -> io::Result<()> {
        let stored = Written {
            version: VERSION,
            account_id: enrollment.account.as_str(),
            user: enrollment.user.as_str(),
            uid: enrollment.uid,
            profile: &enrollment.profile,
            credential: &enrollment.credential,
        };
        let bytes = secret::json(&stored).map_err(|error| invalid(error.to_string()))?;
        let path = self.path(&enrollment.user);
        let mut temporary = path.clone().into_os_string();
        temporary.push(NEW);
        let temporary = PathBuf::from(temporary);
        let written = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                // Never through a link: `create_new` is O_EXCL, which follows none.
                .open(&temporary)?;
            // The mode asked for, whatever the umask took from it.
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)
        })();
        if written.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        written?;
        self.sync()
    }

    /// Forgets the account's file.
    pub fn remove(&self, user: &UserName) -> io::Result<()> {
        match fs::remove_file(self.path(user)) {
            Ok(()) => self.sync(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Flushes the directory, so a rename or a removal survives a crash.
    fn sync(&self) -> io::Result<()> {
        File::open(&self.directory)?.sync_all()
    }
}
