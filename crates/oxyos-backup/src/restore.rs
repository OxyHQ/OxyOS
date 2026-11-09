//! Restoring a whole computer from its backups ([ADR 0059](../../docs/adr/0059-backups.mdx),
//! decision 7): from the recovery, into the root it has just made again (*Restore from Backup*),
//! and in setup, into the running system of a computer nobody has set up yet (*Migrate from a
//! Backup*). Both go through [`Restorer`]:
//!
//! 1. **Where the backups are** ([`Restorer::open`]): a disk, mounted while it is read, or a share
//!    reached through rclone; its `OxyOS Backups` folder holds one repository per computer.
//! 2. **Whose they are** ([`Restorer::computers`]): each repository the person's backup password
//!    opens (restic's second key, decision 5), its computer and its backups by date, newest first,
//!    and the accounts its newest backup holds (its manifest, read with `restic dump`). A
//!    repository the password does not open is another person's, and is counted, never shown.
//! 3. **The restore** ([`Restorer::restore`]), into a target root that has no account yet:
//!    - the manifest's accounts are checked: each user is the one its account names (`oxy-…`,
//!      derived from the account's id, so a backup cannot make `root`), each uid and gid an
//!      ordinary user's of the target (its `login.defs`), none twice; a user, a uid, a group or a
//!      gid the target has already is a clash, refused before anything is written, unless it is
//!      the very user and group an interrupted restore made (the same name, uid and gid, and no
//!      account enrolled);
//!    - their groups and users are made again with the numbers they had (`groupadd --gid`,
//!      `useradd --uid --gid`, their passwords locked), the administrators in `sudo`;
//!    - restic restores their homes, their pictures and the computer's Wi-Fi networks, owners and
//!      modes as they were (as root); a home the backup does not hold is made empty;
//!    - their accounts are written to the target's account service's directory with no
//!      credential: each is `signInAgain` (ADR 0020), listed by the login screen and set up again
//!      online, which finds its user enrolled and keeps it. No credential, verifier or keychain is
//!      in a backup (decision 6); the keychain comes back from Oxy (ADR 0050).
//!
//! The account's file is written last: a restore cut short leaves users the next restore finds
//! and keeps, and no account the login screen lists half restored.

use crate::disks::{Disk, Disks};
use crate::engine::{self, Engine, Location, Progress, Repository, Stop};
use crate::machine::{self, Manifest};
use crate::policy::{escape, MANIFEST_DIRECTORY};
use crate::share::{self, Share};
use crate::store::{valid_id, BACKUPS_FOLDER};
use oxyos_account::api::Profile;
use oxyos_account::store::{Enrollment, Store as Accounts, ADMINISTRATOR_MADE};
use oxyos_account::users::{SystemUsers, Users, UsersError};
use oxyos_account::{AccountId, UserName};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use zeroize::Zeroizing;

/// The homes' folder, each account's home its user's folder in it.
const HOMES: &str = "/home";
/// The accounts' pictures, each a file named by its user.
const PICTURES: &str = "/var/lib/oxyos/pictures";
/// The computer's Wi-Fi networks, NetworkManager's.
pub const NETWORKS: &str = "/etc/NetworkManager/system-connections";
/// The account service's directory in a root.
const ACCOUNTS: &str = "var/lib/oxyos/accounts";

/// Where a computer's backups are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Source {
    /// A disk, by its file system's UUID.
    Disk { uuid: String },
    /// A share on a NAS or another computer.
    Share { share: Share },
}

/// Why a restore, or what comes before it, could not be done: its reason is the socket's word.
#[derive(Debug)]
pub enum Refusal {
    /// The disk is not connected, or cannot be mounted.
    NotConnected,
    /// No backup of a computer there.
    NoBackup,
    /// The password opens none of the backups there.
    WrongPassword,
    /// The share refused its user and password.
    ShareRefused,
    /// The share's computer cannot be reached.
    Unreachable,
    /// Another program is using the backup.
    InUse,
    /// The target is full.
    Full,
    /// The backup holds no manifest of the computer, or one that cannot be restored.
    InvalidBackup(String),
    /// The target has an account already.
    AlreadySetUp,
    /// A user or group of the backup's is the target's already.
    Clash(String),
    Stopped,
    Failed(String),
}

impl Refusal {
    /// The word the socket and the recovery say it with.
    pub fn reason(&self) -> &'static str {
        match self {
            Refusal::NotConnected => "notConnected",
            Refusal::NoBackup => "noBackup",
            Refusal::WrongPassword => "wrongPassword",
            Refusal::ShareRefused => "shareRefused",
            Refusal::Unreachable => "unreachable",
            Refusal::InUse => "inUse",
            Refusal::Full => "full",
            Refusal::InvalidBackup(_) => "invalidBackup",
            Refusal::AlreadySetUp => "alreadySetUp",
            Refusal::Clash(_) => "clash",
            Refusal::Stopped => "stopped",
            Refusal::Failed(_) => "failed",
        }
    }

    fn of(error: engine::Error, share: bool) -> Self {
        match error {
            engine::Error::NoRepository => Refusal::NoBackup,
            engine::Error::WrongPassword => Refusal::WrongPassword,
            engine::Error::Locked => Refusal::InUse,
            engine::Error::NoSpace => Refusal::Full,
            engine::Error::Refused => Refusal::ShareRefused,
            engine::Error::Unreachable => Refusal::Unreachable,
            engine::Error::Stopped => Refusal::Stopped,
            // A share that failed for a reason restic does not name: its computer, as a person
            // sees it.
            engine::Error::Failed(_) if share => Refusal::Unreachable,
            error => Refusal::Failed(error.to_string()),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::InvalidBackup(why) => write!(f, "the backup cannot be restored: {why}"),
            Refusal::Clash(why) | Refusal::Failed(why) => f.write_str(why),
            Refusal::AlreadySetUp => f.write_str("an account is set up on this computer already"),
            other => f.write_str(other.reason()),
        }
    }
}

impl std::error::Error for Refusal {}

/// A computer whose backups the password opens.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Computer {
    /// Its repository's folder in `OxyOS Backups`.
    pub repository: String,
    /// Its name, as its newest backup has it.
    pub name: String,
    /// Its backups, newest first.
    pub backups: Vec<Backup>,
    /// The accounts its newest backup holds.
    pub accounts: Vec<Person>,
}

/// A backup of a computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub snapshot: String,
    /// When it was made, in seconds since the epoch.
    pub time: i64,
}

/// An account a backup holds, as the person is shown it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub user: String,
    pub name: String,
    pub administrator: bool,
}

/// The computers of a source's `OxyOS Backups`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    /// Those the password opens, the one backed up most recently first.
    pub computers: Vec<Computer>,
    /// How many it does not open: other people's.
    pub unopened: usize,
}

/// Where a restore is.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// The backup's accounts, checked and made again.
    Accounts,
    /// Their files, as restic restores them.
    Files(Progress),
    /// Their accounts written for the login screen.
    Finishing,
}

/// What a restore brought back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Restored {
    pub computer: String,
    pub accounts: Vec<machine::Account>,
}

/// A source's `OxyOS Backups` folder, reachable: a disk mounted while this lives, or a share.
pub struct Folder {
    place: Place,
    _mounted: Option<Mounted>,
}

enum Place {
    Directory(PathBuf),
    Share {
        share: Share,
        obscured: Zeroizing<String>,
    },
}

struct Mounted {
    disks: Arc<dyn Disks>,
    uuid: String,
}

impl Drop for Mounted {
    fn drop(&mut self) {
        self.disks.unmount(&self.uuid);
    }
}

impl Folder {
    fn share(&self) -> bool {
        matches!(self.place, Place::Share { .. })
    }

    /// The repository of `folder`, opened by `password`; none for a name that is not one a
    /// backup service makes.
    pub fn repository(&self, folder: &str, password: &str) -> Option<Repository> {
        if !valid_id(folder) {
            return None;
        }
        let location = match &self.place {
            Place::Directory(path) => Location::directory(&path.join(folder)),
            Place::Share { share, obscured } => {
                share.location(&format!("{BACKUPS_FOLDER}/{folder}"), obscured)
            }
        };
        Some(Repository {
            location,
            password: Zeroizing::new(password.to_owned()),
        })
    }
}

/// Restores computers from their backups, through restic.
pub struct Restorer {
    engine: Engine,
    disks: Arc<dyn Disks>,
    root: PathBuf,
}

impl Restorer {
    /// Backups of machines, read from `disks` with `engine`.
    pub fn new(engine: Engine, disks: Arc<dyn Disks>) -> Self {
        Self {
            engine,
            disks,
            root: "/".into(),
        }
    }

    /// Backups whose machine's root was `root` when they were made (tests', and a developer's
    /// service backing up folders of its own): its paths are read below it.
    pub fn with_root(mut self, root: &Path) -> Self {
        self.root = root.to_owned();
        self
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The disks connected now that may hold backups.
    pub fn disks(&self) -> Vec<Disk> {
        self.disks.list()
    }

    /// A machine path as the backups hold it.
    fn backed_up(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    /// The `OxyOS Backups` folder of `source`: a disk is mounted while the folder lives; a share
    /// is reached with `share_password`.
    pub fn open(&self, source: &Source, share_password: Option<&str>) -> Result<Folder, Refusal> {
        match source {
            Source::Disk { uuid } => {
                let disk = self
                    .disks
                    .list()
                    .into_iter()
                    .find(|d| &d.uuid == uuid)
                    .ok_or(Refusal::NotConnected)?;
                let path = self.disks.mount(&disk).map_err(|e| {
                    eprintln!("oxyos-backup: mounting {uuid}: {e}");
                    Refusal::NotConnected
                })?;
                Ok(Folder {
                    place: Place::Directory(path.join(BACKUPS_FOLDER)),
                    _mounted: Some(Mounted {
                        disks: self.disks.clone(),
                        uuid: uuid.clone(),
                    }),
                })
            }
            Source::Share { share } => {
                share
                    .validate()
                    .map_err(|why| Refusal::Failed(why.to_owned()))?;
                let obscured = share::obscure(self.engine.rclone(), share_password.unwrap_or(""))
                    .map_err(|e| Refusal::Failed(e.to_string()))?;
                Ok(Folder {
                    place: Place::Share {
                        share: share.clone(),
                        obscured,
                    },
                    _mounted: None,
                })
            }
        }
    }

    /// The repositories of the folder, by their folders' names.
    fn repositories(&self, folder: &Folder) -> Result<Vec<String>, Refusal> {
        let mut names = match &folder.place {
            Place::Directory(path) => match fs::read_dir(path) {
                Ok(entries) => entries
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .filter_map(|e| e.file_name().to_str().map(str::to_owned))
                    .collect(),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(Refusal::Failed(format!("{}: {e}", path.display()))),
            },
            Place::Share { share, obscured } => {
                share_folders(self.engine.rclone(), share, obscured)?
            }
        };
        names.retain(|name| valid_id(name));
        names.sort();
        Ok(names)
    }

    /// The computers whose backups `password` opens, in the folder.
    pub fn computers(&self, folder: &Folder, password: &str) -> Result<Listing, Refusal> {
        let mut listing = Listing::default();
        let manifests = self.backed_up(MANIFEST_DIRECTORY);
        for name in self.repositories(folder)? {
            let Some(repository) = folder.repository(&name, password) else {
                continue;
            };
            let snapshots = match self.engine.snapshots(&repository, None) {
                Ok(snapshots) => snapshots,
                Err(engine::Error::WrongPassword) => {
                    listing.unopened += 1;
                    continue;
                }
                // A folder that is no repository (made by hand, or a repository never finished).
                Err(engine::Error::NoRepository) => continue,
                Err(error) => return Err(Refusal::of(error, folder.share())),
            };
            // A machine's backups: those that read its manifest.
            let mut backups: Vec<(Backup, String)> = snapshots
                .into_iter()
                .filter(|s| s.paths.iter().any(|p| Path::new(p) == manifests))
                .filter_map(|s| {
                    let time = s.seconds()?;
                    Some((
                        Backup {
                            snapshot: s.id,
                            time,
                        },
                        s.hostname,
                    ))
                })
                .collect();
            backups.sort_by(|a, b| b.0.time.cmp(&a.0.time));
            let Some((newest, host)) = backups.first().cloned() else {
                continue;
            };
            let accounts = match self.manifest(&repository, &newest.snapshot) {
                Ok(manifest) => manifest
                    .accounts
                    .into_iter()
                    .map(|a| Person {
                        user: a.user,
                        name: a.name,
                        administrator: a.administrator,
                    })
                    .collect(),
                Err(error) => {
                    eprintln!("oxyos-backup: {name}'s newest backup: {error}");
                    Vec::new()
                }
            };
            listing.computers.push(Computer {
                repository: name,
                name: host,
                backups: backups.into_iter().map(|(b, _)| b).collect(),
                accounts,
            });
        }
        listing.computers.sort_by(|a, b| {
            let newest = |c: &Computer| c.backups.first().map_or(0, |b| b.time);
            newest(b).cmp(&newest(a))
        });
        Ok(listing)
    }

    /// The machine's manifest as `snapshot` holds it.
    pub fn manifest(&self, repository: &Repository, snapshot: &str) -> Result<Manifest, Refusal> {
        let share = is_share(repository);
        let path = self.backed_up(MANIFEST_DIRECTORY).join(machine::MANIFEST);
        let mut dump = self
            .engine
            .dump(repository, snapshot, &path.to_string_lossy(), false)
            .map_err(|e| Refusal::of(e, share))?;
        let mut bytes = Vec::new();
        let read = dump.output().take(1 << 20).read_to_end(&mut bytes);
        let finished = dump.finish();
        match (read, finished) {
            (Ok(_), Ok(())) => {}
            (_, Err(engine::Error::Failed(why))) => {
                return Err(Refusal::InvalidBackup(format!("no manifest: {why}")))
            }
            (_, Err(error)) => return Err(Refusal::of(error, share)),
            (Err(error), Ok(())) => return Err(Refusal::Failed(error.to_string())),
        }
        Manifest::read(&bytes).map_err(|e| Refusal::InvalidBackup(e.to_string()))
    }

    /// Restores `snapshot` of `repository` into the root `target` (`/` for the running system),
    /// which must have no account yet: its accounts' users, homes and pictures, the computer's
    /// Wi-Fi networks, and the accounts as `signInAgain`. Stopped by `stop`; `progress` is told
    /// each step.
    pub fn restore(
        &self,
        repository: &Repository,
        snapshot: &str,
        target: &Path,
        stop: &Stop,
        mut progress: impl FnMut(Step),
    ) -> Result<Restored, Refusal> {
        let manifest = self.manifest(repository, snapshot)?;
        progress(Step::Accounts);
        let defs = LoginDefs::read(&target.join("etc/login.defs"));
        let accounts = planned(&manifest, &defs)?;

        // The target's account service's directory: no account may be enrolled there yet.
        let accounts_directory = target.join(ACCOUNTS);
        if let Some(parent) = accounts_directory.parent() {
            fs::create_dir_all(parent).map_err(|e| Refusal::Failed(e.to_string()))?;
        }
        let store = Accounts::open(&accounts_directory)
            .map_err(|e| Refusal::Failed(format!("{}: {e}", accounts_directory.display())))?;
        if !store
            .users()
            .map_err(|e| Refusal::Failed(e.to_string()))?
            .is_empty()
        {
            return Err(Refusal::AlreadySetUp);
        }

        // Every clash refused before anything is made.
        let passwd = entries(&target.join("etc/passwd"))?;
        let groups = entries(&target.join("etc/group"))?;
        let mut work = Vec::new();
        for account in &accounts {
            work.push(account.against(&passwd, &groups)?);
        }

        let users = if target == Path::new("/") {
            SystemUsers::new()
        } else {
            SystemUsers::with_prefix(target)
        };
        for (account, (make_group, make_user)) in accounts.iter().zip(&work) {
            if *make_group {
                users
                    .make_group(&account.user, account.gid)
                    .map_err(|e| account.failed("its group", e))?;
            }
            if *make_user {
                users
                    .make_user(&account.user, account.uid, account.gid)
                    .map_err(|e| account.failed("its user", e))?;
            }
            if account.manifest.administrator {
                users
                    .make_administrator(&account.user)
                    .map_err(|e| account.failed("an administrator", e))?;
            }
        }

        let mut includes = Vec::new();
        for account in &accounts {
            includes.push(escape(&Path::new(HOMES).join(account.user.as_str())));
            includes.push(escape(&Path::new(PICTURES).join(account.user.as_str())));
        }
        includes.push(escape(Path::new(NETWORKS)));
        // The machine's root in the snapshot is restored as the target's root.
        let restored = if self.root == Path::new("/") {
            snapshot.to_owned()
        } else {
            format!("{snapshot}:{}", self.root.display())
        };
        self.engine
            .restore(repository, &restored, &includes, target, stop, |p| {
                progress(Step::Files(p.clone()))
            })
            .map_err(|e| Refusal::of(e, is_share(repository)))?;

        progress(Step::Finishing);
        for account in &accounts {
            let home = target
                .join(HOMES.trim_start_matches('/'))
                .join(account.user.as_str());
            if fs::symlink_metadata(&home).is_err() {
                make_home(&home, account.uid, account.gid)
                    .map_err(|e| account.failed("its home", UsersError::Failed(e.to_string())))?;
            }
        }
        for account in &accounts {
            store
                .put(&Enrollment {
                    account: account.id.clone(),
                    user: account.user.clone(),
                    uid: account.uid,
                    profile: Profile {
                        id: account.id.to_string(),
                        username: None,
                        display_name: Some(account.manifest.name.clone())
                            .filter(|n| !n.trim().is_empty()),
                        ..Profile::default()
                    },
                    credential: None,
                    password_changed: false,
                })
                .map_err(|e| Refusal::Failed(format!("{}'s account: {e}", account.user)))?;
        }
        // The machine has its administrator: a setup on it makes no other by itself (ADR 0054).
        if accounts.iter().any(|a| a.manifest.administrator) {
            store
                .record_fact(ADMINISTRATOR_MADE)
                .map_err(|e| Refusal::Failed(e.to_string()))?;
        }
        Ok(Restored {
            computer: manifest.computer,
            accounts: accounts.into_iter().map(|a| a.manifest).collect(),
        })
    }
}

/// The ordinary users' numbers, as a root's `login.defs` says them (Debian's defaults otherwise).
struct LoginDefs {
    uid: (u32, u32),
    gid: (u32, u32),
}

impl LoginDefs {
    fn read(path: &Path) -> Self {
        let text = fs::read_to_string(path).unwrap_or_default();
        let value = |name: &str, default: u32| {
            text.lines()
                .filter_map(|line| {
                    let mut words = line.split_whitespace();
                    (words.next() == Some(name)).then(|| words.next()?.parse().ok())?
                })
                .next_back()
                .unwrap_or(default)
        };
        Self {
            uid: (value("UID_MIN", 1000), value("UID_MAX", 60000)),
            gid: (value("GID_MIN", 1000), value("GID_MAX", 60000)),
        }
    }
}

/// An account of the manifest, checked.
struct Planned {
    manifest: machine::Account,
    id: AccountId,
    user: UserName,
    uid: u32,
    gid: u32,
}

impl Planned {
    /// Whether its group and its user are to be made (true) or are the target's already as an
    /// interrupted restore made them (false); a clash otherwise.
    fn against(&self, passwd: &[Entry], groups: &[Entry]) -> Result<(bool, bool), Refusal> {
        let user = self.user.as_str();
        let clash = |what: String| {
            Refusal::Clash(format!(
                "{} ({user}, uid {}): {what} on this computer already",
                self.manifest.name, self.uid
            ))
        };
        let make_user = match passwd.iter().find(|e| e.name == user) {
            Some(e) if e.id == self.uid && e.gid == Some(self.gid) => false,
            Some(_) => return Err(clash(format!("a user {user} is"))),
            None => match passwd.iter().find(|e| e.id == self.uid) {
                Some(e) => return Err(clash(format!("the uid {} is {}'s", self.uid, e.name))),
                None => true,
            },
        };
        let make_group = match groups.iter().find(|e| e.name == user) {
            Some(e) if e.id == self.gid => false,
            Some(_) => return Err(clash(format!("a group {user} is"))),
            None => match groups.iter().find(|e| e.id == self.gid) {
                Some(e) => return Err(clash(format!("the gid {} is {}'s", self.gid, e.name))),
                None => true,
            },
        };
        Ok((make_group, make_user))
    }

    fn failed(&self, what: &str, error: UsersError) -> Refusal {
        match error {
            UsersError::Exists => Refusal::Clash(format!(
                "{} ({}): {what} is on this computer already",
                self.manifest.name, self.user
            )),
            error => Refusal::Failed(format!("{}: {what}: {error}", self.user)),
        }
    }
}

/// The manifest's accounts, each checked: its user the one its account names, its numbers an
/// ordinary user's of the target, none twice.
fn planned(manifest: &Manifest, defs: &LoginDefs) -> Result<Vec<Planned>, Refusal> {
    let invalid = |why: String| Refusal::InvalidBackup(why);
    let (mut users, mut uids, mut gids) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    let mut planned = Vec::new();
    for account in &manifest.accounts {
        let id = AccountId::parse(&account.account_id)
            .map_err(|_| invalid(format!("{} is not an account's id", account.account_id)))?;
        let user = id.user_name();
        if user.as_str() != account.user {
            return Err(invalid(format!(
                "{} is not {}'s user",
                account.user, account.account_id
            )));
        }
        let gid = account.gid.unwrap_or(account.uid);
        let ordinary = |n: u32, (min, max): (u32, u32)| (min..=max).contains(&n);
        if !ordinary(account.uid, defs.uid) || !ordinary(gid, defs.gid) {
            return Err(invalid(format!(
                "{} has the uid {} and gid {}, not an ordinary user's here",
                account.user, account.uid, gid
            )));
        }
        if !users.insert(account.user.clone()) || !uids.insert(account.uid) || !gids.insert(gid) {
            return Err(invalid(format!("{} is in it twice", account.user)));
        }
        planned.push(Planned {
            manifest: account.clone(),
            id,
            user,
            uid: account.uid,
            gid,
        });
    }
    Ok(planned)
}

/// A line of a passwd(5) or group(5) file: its name, its number and (passwd's) its group.
struct Entry {
    name: String,
    id: u32,
    gid: Option<u32>,
}

fn entries(path: &Path) -> Result<Vec<Entry>, Refusal> {
    let text = fs::read_to_string(path)
        .map_err(|e| Refusal::Failed(format!("{}: {e}", path.display())))?;
    Ok(text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            Some(Entry {
                name: (*fields.first()?).to_owned(),
                id: fields.get(2)?.parse().ok()?,
                gid: fields.get(3).and_then(|g| g.parse().ok()),
            })
        })
        .collect())
}

/// An empty home for an account whose backup held none, as useradd makes one (mode 0700).
fn make_home(home: &Path, uid: u32, gid: u32) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::DirBuilder::new().mode(0o700).create(home)?;
    fs::set_permissions(home, fs::Permissions::from_mode(0o700))?;
    std::os::unix::fs::chown(home, Some(uid), Some(gid))
}

fn is_share(repository: &Repository) -> bool {
    repository
        .location
        .repository
        .to_str()
        .is_some_and(|r| r.starts_with("rclone:"))
}

/// The folders of a share's `OxyOS Backups` (`rclone lsjson --dirs-only`), its configuration from
/// the environment alone, as restic's backend has it.
fn share_folders(rclone: &Path, share: &Share, obscured: &str) -> Result<Vec<String>, Refusal> {
    let location = share.location(BACKUPS_FOLDER, obscured);
    let remote = location
        .repository
        .to_str()
        .and_then(|r| r.strip_prefix("rclone:"))
        .ok_or_else(|| Refusal::Failed("the share's location".into()))?
        .to_owned();
    let mut command = Command::new(rclone);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .args(["lsjson", "--dirs-only", "--", &remote])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in &location.environment {
        command.env(name, value.as_str());
    }
    let output = command
        .output()
        .map_err(|e| Refusal::Failed(format!("rclone cannot be run: {e}")))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        // rclone's exit status 3: the folder is not there, so no computer was backed up there.
        if output.status.code() == Some(3) || stderr.contains("directory not found") {
            return Ok(Vec::new());
        }
        return Err(Refusal::of(
            engine::classify(output.status.code(), &stderr),
            true,
        ));
    }
    #[derive(Deserialize)]
    struct Item {
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "IsDir", default)]
        is_dir: bool,
    }
    let items: Vec<Item> = serde_json::from_slice(&output.stdout)
        .map_err(|e| Refusal::Failed(format!("rclone's listing: {e}")))?;
    Ok(items
        .into_iter()
        .filter(|i| i.is_dir)
        .map(|i| i.name)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(user: &str, id: &str, uid: u32) -> machine::Account {
        machine::Account {
            account_id: id.into(),
            user: user.into(),
            uid,
            gid: None,
            name: "Ada".into(),
            administrator: false,
        }
    }

    fn defs() -> LoginDefs {
        LoginDefs {
            uid: (1000, 60000),
            gid: (1000, 60000),
        }
    }

    fn manifest(accounts: Vec<machine::Account>) -> Manifest {
        Manifest {
            version: 1,
            computer: "ada-laptop".into(),
            written: 0,
            accounts,
        }
    }

    const ID: &str = "65a1b2c3d4e5f60718293a4b";

    #[test]
    fn a_manifests_accounts_are_checked_before_anything_is_made() {
        let user = AccountId::parse(ID).unwrap().user_name();
        let good = account(user.as_str(), ID, 1001);
        assert_eq!(
            planned(&manifest(vec![good.clone()]), &defs()).unwrap()[0].gid,
            1001
        );
        // A user that is not its account's (a backup cannot make root, or anyone else).
        let mut root = good.clone();
        root.user = "root".into();
        assert!(matches!(
            planned(&manifest(vec![root]), &defs()),
            Err(Refusal::InvalidBackup(_))
        ));
        // A system user's number.
        let mut system = good.clone();
        system.uid = 27;
        assert!(matches!(
            planned(&manifest(vec![system]), &defs()),
            Err(Refusal::InvalidBackup(_))
        ));
        // Twice.
        assert!(matches!(
            planned(&manifest(vec![good.clone(), good]), &defs()),
            Err(Refusal::InvalidBackup(_))
        ));
    }

    #[test]
    fn a_clash_is_refused_and_an_interrupted_restores_user_kept() {
        let user = AccountId::parse(ID).unwrap().user_name();
        let plan = planned(&manifest(vec![account(user.as_str(), ID, 1001)]), &defs()).unwrap();
        let entry = |name: &str, id: u32, gid: Option<u32>| Entry {
            name: name.into(),
            id,
            gid,
        };
        let groups = [entry("root", 0, None), entry("sudo", 27, None)];
        let passwd = [entry("root", 0, Some(0))];
        assert!(matches!(
            plan[0].against(&passwd, &groups),
            Ok((true, true))
        ));
        // The developer's user has the uid.
        let developer = [entry("nate", 1001, Some(1001))];
        match plan[0].against(&developer, &groups) {
            Err(Refusal::Clash(why)) => assert!(why.contains("uid 1001 is nate's"), "{why}"),
            other => panic!("{other:?}"),
        }
        // The same user and group, as an interrupted restore made them.
        let made = [entry(user.as_str(), 1001, Some(1001))];
        let made_group = [entry(user.as_str(), 1001, None)];
        assert!(matches!(
            plan[0].against(&made, &made_group),
            Ok((false, false))
        ));
        // The name with other numbers.
        let other = [entry(user.as_str(), 1002, Some(1002))];
        assert!(matches!(
            plan[0].against(&other, &groups),
            Err(Refusal::Clash(_))
        ));
    }

    #[test]
    fn login_defs_are_read_as_useradd_reads_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.defs");
        fs::write(&path, "# UID_MIN 1\nUID_MIN\t\t 2000\nUID_MAX 3000\n").unwrap();
        let defs = LoginDefs::read(&path);
        assert_eq!(defs.uid, (2000, 3000));
        assert_eq!(defs.gid, (1000, 60000));
    }
}
