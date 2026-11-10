//! The enrolled accounts, one file each in the service's own directory
//! (`/var/lib/oxyos/accounts`): the account, its user and profile (as last read online), its
//! platform credential, and whether Oxy refused a password the machine accepts.
//!
//! - **An account signed out by Oxy** (its device session gone, ADR 0020 decision 2) keeps its
//!   file with no credential: it lists as `signInAgain`, its user and home stay, and setting it up
//!   again online gives it a credential anew.
//! - Version 1 files (step 1's, a device credential and nothing else) are read as ready accounts.
//!
//! - **Two facts of the machine** are files of the same directory, beside the accounts'
//!   ([`Store::fact`]): [`ADMINISTRATOR_MADE`], the machine's first administrator was made (once,
//!   never again: a machine whose administrator was removed gets no new one by itself), and
//!   [`ERASE_ARMED`], the erase is armed and the recovery erases the machine at its next boot
//!   ([ADR 0054](../../docs/adr/0054-recovery-and-erasing.mdx)), which a service started again
//!   before that boot keeps to.
//!
//! - The directory is the service's alone: mode 0700, owned by the service's user, not a symbolic
//!   link. One that is anything else is refused, never repaired.
//! - Each file is mode 0600 and written whole: to a new file beside it, flushed to the disk,
//!   renamed over the old one, and the directory flushed, so a reader or a crash sees the old
//!   file or the new one, never a part. What is written is overwritten in memory afterwards.
//! - What the login screen shows of each (its user, name and picture, [`Store::list`]) is read
//!   from the same files; the credential in them is dropped unread, overwritten.
//! - An account's picture (its Oxy avatar's image) is a file of the pictures' directory,
//!   `/var/lib/oxyos/pictures/<user>-<avatar>`, as AccountsService keeps its icons: the directory
//!   root's, mode 0755 (the unit's tmpfiles), each picture root's, mode 0644, so the login screen
//!   (the greeter's user) and the account's session read it, and no one else writes it. Named by
//!   the avatar it shows, so a new one is a new file (an image is never told apart from an older
//!   one by its name alone) and the one shown until a new one is read (offline) is the last read.
//!   Written whole, as the account's file is; the others of the user removed once it is.

use crate::api::{DeviceCredential, Profile};
use crate::id::{AccountId, UserName};
use crate::pin::{LocalSignIn, PinState};
use crate::secret;
use serde::{Deserialize, Serialize};
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;
// The messages, the protocol's own crate's (`oxyos-account-proto`): clients link them alone.
pub use oxyos_account_proto::store::State;

/// Where the service keeps enrolled accounts.
pub const STATE_DIRECTORY: &str = "/var/lib/oxyos/accounts";
/// Where accounts' pictures are: a file named by the account's user and avatar, for the login
/// screen and the account's session.
pub const PICTURES_DIRECTORY: &str = "/var/lib/oxyos/pictures";

/// The machine's first administrator was made (by its first setup, or by the service's start on a
/// machine set up before administrators existed).
pub const ADMINISTRATOR_MADE: &str = "administrator-made";
/// The erase is armed: until the machine restarts into its recovery, nothing more is asked of Oxy.
pub const ERASE_ARMED: &str = "erase-armed";

/// The version of an account's file written; version 1 is still read.
const VERSION: u32 = 2;
const SUFFIX: &str = ".json";
/// An account's local sign-in, its PIN and whether its password signs in (`crate::pin`).
const LOCAL_SUFFIX: &str = ".local";
const NEW: &str = ".new";

/// An enrolled account.
#[derive(Debug, PartialEq, Eq)]
pub struct Enrollment {
    pub account: AccountId,
    pub user: UserName,
    pub uid: u32,
    pub profile: Profile,
    /// Its platform credential; none once Oxy signed it out (it must be set up again online).
    pub credential: Option<DeviceCredential>,
    /// Oxy refused a password the machine accepts: the password was changed elsewhere, and while
    /// Oxy can be asked the old one no longer opens the account.
    pub password_changed: bool,
}

impl Enrollment {
    pub fn state(&self) -> State {
        match self.credential {
            Some(_) => State::Ready,
            None => State::SignInAgain,
        }
    }
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
    credential: Option<&'a DeviceCredential>,
    password_changed: bool,
}

/// An account's file, as read: version 2, or version 1 (a credential always, no flag).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Stored {
    version: u32,
    account_id: String,
    user: String,
    uid: u32,
    profile: Profile,
    credential: Option<DeviceCredential>,
    #[serde(default)]
    password_changed: bool,
}

impl Stored {
    fn read(bytes: &[u8]) -> Result<Self, String> {
        let stored: Stored = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        match (stored.version, &stored.credential, stored.password_changed) {
            (2, _, _) | (1, Some(_), false) => Ok(stored),
            _ => Err(format!("not an account's file of version 1 or {VERSION}")),
        }
    }
}

/// What the login screen shows of an enrolled account: never its credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub account: AccountId,
    pub user: UserName,
    pub uid: u32,
    /// What the person is called ([`Profile::name`]): the account's display name, else its
    /// username; empty when it has neither, never its user's.
    pub name: String,
    /// The account's picture, a file of the pictures' directory, when it has one.
    pub picture: Option<PathBuf>,
    pub state: State,
    /// Its PIN: none, set, or locked out (`crate::pin`); never its verifier.
    pub pin: PinState,
    /// Whether its password signs in on this computer.
    pub password: bool,
}

pub struct Store {
    directory: PathBuf,
    pictures: Option<PathBuf>,
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
            pictures: None,
        })
    }

    /// Accounts' pictures are files of `directory`, named by their users.
    pub fn with_pictures(mut self, directory: &Path) -> Self {
        self.pictures = Some(directory.to_owned());
        self
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// `user`'s picture: the one of `avatar` (its account's avatar now), else the last one read
    /// (the avatar changed while Oxy's cloud could not be reached); none when it has none, or
    /// accounts have no pictures. Only a regular file, never a link.
    pub fn picture(&self, user: &UserName, avatar: Option<&str>) -> Option<PathBuf> {
        let directory = self.pictures.as_ref()?;
        let is_file =
            |path: &Path| fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file());
        if let Some(current) = avatar
            .and_then(crate::api::avatar_id)
            .map(|avatar| directory.join(format!("{user}-{avatar}")))
            .filter(|path| is_file(path))
        {
            return Some(current);
        }
        self.pictures_of(user)
            .into_iter()
            .filter(|path| is_file(path))
            .max_by_key(|path| fs::symlink_metadata(path).and_then(|m| m.modified()).ok())
    }

    /// Whether `user` has the picture of `avatar` already.
    pub fn has_picture(&self, user: &UserName, avatar: &str) -> bool {
        match (&self.pictures, crate::api::avatar_id(avatar)) {
            (Some(directory), Some(avatar)) => {
                fs::symlink_metadata(directory.join(format!("{user}-{avatar}")))
                    .is_ok_and(|metadata| metadata.is_file())
            }
            _ => false,
        }
    }

    /// Every picture file of `user`'s (any avatar's), by its name.
    fn pictures_of(&self, user: &UserName) -> Vec<PathBuf> {
        let Some(directory) = &self.pictures else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(directory) else {
            return Vec::new();
        };
        let prefix = format!("{user}-");
        entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                // A user's name has no `-` past `oxy-`: `<user>-…` is no other user's.
                name.strip_prefix(&prefix)
                    .is_some_and(|avatar| crate::api::avatar_id(avatar).is_some())
            })
            .map(|entry| entry.path())
            .collect()
    }

    /// `bytes` made `user`'s picture of `avatar` (root's, mode 0644, written whole), its others
    /// removed; where it is.
    pub fn put_picture(&self, user: &UserName, avatar: &str, bytes: &[u8]) -> io::Result<PathBuf> {
        let directory = self
            .pictures
            .as_ref()
            .ok_or_else(|| invalid("accounts have no pictures here".into()))?;
        let avatar = crate::api::avatar_id(avatar)
            .ok_or_else(|| invalid("not an avatar's file id".into()))?;
        let path = directory.join(format!("{user}-{avatar}"));
        let new = directory.join(format!("{user}-{avatar}{NEW}"));
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
                .mode(0o644)
                .open(&new)?;
            // The service's umask (0077) is not the picture's: the login screen reads it.
            file.set_permissions(fs::Permissions::from_mode(0o644))?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        fs::rename(&new, &path)?;
        for old in self.pictures_of(user) {
            if old != path {
                let _ = fs::remove_file(old);
            }
        }
        File::open(directory)?.sync_all()?;
        Ok(path)
    }

    /// Every picture of `user`'s removed (the account removed, or its avatar removed on Oxy).
    pub fn remove_pictures(&self, user: &UserName) -> io::Result<()> {
        let pictures = self.pictures_of(user);
        if pictures.is_empty() {
            return Ok(());
        }
        for picture in pictures {
            match fs::remove_file(&picture) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        match &self.pictures {
            Some(directory) => File::open(directory)?.sync_all(),
            None => Ok(()),
        }
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
        let stored = Stored::read(&bytes)
            .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
        if stored.account_id != account.as_str() || stored.user != account.user_name().as_str() {
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
            password_changed: stored.password_changed,
        }))
    }

    /// The enrollment whose user is `user` (an enrolled account's `oxy-…`), if there is one.
    pub fn get_user(&self, user: &str) -> io::Result<Option<Enrollment>> {
        let Some(account) = self.account_of(user)? else {
            return Ok(None);
        };
        self.get(&account)
    }

    /// The enrollment whose user has `uid`, if there is one: the account a peer of the socket
    /// runs as. A file that cannot be read is passed over (it is said when listed).
    pub fn get_uid(&self, uid: u32) -> io::Result<Option<Enrollment>> {
        for user in self.users()? {
            if let Ok(Some(enrollment)) = self.get_user(&user) {
                if enrollment.uid == uid {
                    return Ok(Some(enrollment));
                }
            }
        }
        Ok(None)
    }

    /// The account whose file is `user`'s, read from that file.
    fn account_of(&self, user: &str) -> io::Result<Option<AccountId>> {
        if user.contains('/') || user.starts_with('.') || !self.users()?.iter().any(|u| u == user) {
            return Ok(None);
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(16 * 1024));
        File::open(self.directory.join(format!("{user}{SUFFIX}")))?.read_to_end(&mut bytes)?;
        let stored = Stored::read(&bytes).map_err(invalid)?;
        let account = AccountId::parse(&stored.account_id)
            .map_err(|_| invalid("not an account's id".into()))?;
        Ok((account.user_name().as_str() == user).then_some(account))
    }

    /// Every enrolled account as the login screen shows it, by user. A file that is not an
    /// account's is left out and said in the returned errors: one broken file hides one account,
    /// never the others.
    pub fn list(&self) -> io::Result<(Vec<Listed>, Vec<String>)> {
        let (mut listed, mut broken) = (Vec::new(), Vec::new());
        for user in self.users()? {
            match self.listed(&user) {
                Ok(account) => listed.push(account),
                Err(error) => broken.push(format!("{user}: {error}")),
            }
        }
        Ok((listed, broken))
    }

    fn listed(&self, user: &str) -> io::Result<Listed> {
        let path = self.directory.join(format!("{user}{SUFFIX}"));
        let mut bytes = Zeroizing::new(Vec::with_capacity(16 * 1024));
        File::open(&path)?.read_to_end(&mut bytes)?;
        let stored = Stored::read(&bytes).map_err(invalid)?;
        let account = AccountId::parse(&stored.account_id)
            .map_err(|_| invalid("not an account's id".into()))?;
        if stored.user != user || account.user_name().as_str() != user {
            return Err(invalid("not this user's account".into()));
        }
        let picture = self.picture(&account.user_name(), stored.profile.avatar.as_deref());
        // Which field the screen shows; PAM decides what signs in, so an unreadable one shows the
        // password's and hides no account.
        let local = self.local(&account.user_name()).unwrap_or_default();
        let name = stored.profile.name().unwrap_or_default().to_owned();
        Ok(Listed {
            user: account.user_name(),
            account,
            uid: stored.uid,
            name,
            picture,
            state: if stored.credential.is_some() {
                State::Ready
            } else {
                State::SignInAgain
            },
            pin: local.pin_state(),
            password: local.password,
        })
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

    /// Every enrolled account whose file can be read, by its user. A file that cannot be read is
    /// no account, and is left out, as [`Store::list`] leaves it out.
    pub fn enrollments(&self) -> io::Result<Vec<Enrollment>> {
        Ok(self
            .users()?
            .iter()
            .filter_map(|user| self.get_user(user).ok().flatten())
            .collect())
    }

    /// Writes the account's file whole, replacing the one it had.
    pub fn put(&self, enrollment: &Enrollment) -> io::Result<()> {
        let stored = Written {
            version: VERSION,
            account_id: enrollment.account.as_str(),
            user: enrollment.user.as_str(),
            uid: enrollment.uid,
            profile: &enrollment.profile,
            credential: enrollment.credential.as_ref(),
            password_changed: enrollment.password_changed,
        };
        let bytes = secret::json(&stored).map_err(|error| invalid(error.to_string()))?;
        self.write_whole(&self.path(&enrollment.user), &bytes)
    }

    fn local_path(&self, user: &UserName) -> PathBuf {
        self.directory.join(format!("{user}{LOCAL_SUFFIX}"))
    }

    /// `user`'s local sign-in: its PIN and whether its password signs in. An account without the
    /// file (set up before PINs) has no PIN and its password.
    pub fn local(&self, user: &UserName) -> io::Result<LocalSignIn> {
        let path = self.local_path(user);
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(LocalSignIn::default())
            }
            Err(error) => return Err(error),
        };
        let mut bytes = Zeroizing::new(Vec::with_capacity(4 * 1024));
        file.read_to_end(&mut bytes)?;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Stored {
            version: u32,
            #[serde(flatten)]
            local: LocalSignIn,
        }
        let stored: Stored = serde_json::from_slice(&bytes)
            .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
        if stored.version != 1 {
            return Err(invalid(format!(
                "{}: not a local sign-in of version 1",
                path.display()
            )));
        }
        Ok(stored.local)
    }

    /// Writes `user`'s local sign-in whole, replacing the one it had.
    pub fn put_local(&self, user: &UserName, local: &LocalSignIn) -> io::Result<()> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Written<'a> {
            version: u32,
            #[serde(flatten)]
            local: &'a LocalSignIn,
        }
        let bytes = secret::json(&Written { version: 1, local })
            .map_err(|error| invalid(error.to_string()))?;
        self.write_whole(&self.local_path(user), &bytes)
    }

    /// Writes `bytes` to `path` whole: a new file beside it, mode 0600, flushed, renamed over.
    fn write_whole(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let path = path.to_owned();
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
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)
        })();
        if written.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        written?;
        self.sync()
    }

    /// Forgets the account's file and its local sign-in.
    pub fn remove(&self, user: &UserName) -> io::Result<()> {
        match fs::remove_file(self.local_path(user)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        match fs::remove_file(self.path(user)) {
            Ok(()) => self.sync(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.sync(),
            Err(error) => Err(error),
        }
    }

    /// Whether the machine's fact `name` ([`ADMINISTRATOR_MADE`], [`ERASE_ARMED`]) holds. One that
    /// cannot be read holds: each is the safer of the two answers.
    pub fn fact(&self, name: &str) -> bool {
        !matches!(fs::symlink_metadata(self.directory.join(name)), Err(error) if error.kind() == io::ErrorKind::NotFound)
    }

    /// Records the machine's fact `name`: an empty file, mode 0600, flushed with its directory.
    pub fn record_fact(&self, name: &str) -> io::Result<()> {
        let file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.directory.join(name))
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
            Err(error) => return Err(error),
        };
        file.sync_all()?;
        self.sync()
    }

    /// Flushes the directory, so a rename or a removal survives a crash.
    fn sync(&self) -> io::Result<()> {
        File::open(&self.directory)?.sync_all()
    }
}
