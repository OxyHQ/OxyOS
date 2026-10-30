//! Installing, updating and removing applications (docs/adr/0030-installing-applications.mdx,
//! decisions 3, 5, 6, 10 and 11).
//!
//! - **A release** is a target of the store's TUF metadata named `<id>/<versionCode>/<arch>/
//!   manifest.json`, whose custom data (`custom.oxyos`) carries, each as the exact text that was
//!   signed: the store's statement of the release (signed by a key of the root's targets role),
//!   the developer's signature over the manifest, the developer's current public key and its
//!   lineage. The manifest's own bytes are the target's, of the length and hash TUF verified; the
//!   files it lists are fetched as `objects/<aa>/<sha256>` and each checked against the manifest.
//! - **Verified before anything is used:** the store's statement against the manifest and the
//!   developer's signature; the signature against the key pinned at first install or one its
//!   lineage hands over to; the manifest (no native code, no system grant); a runtime of the
//!   machine that runs it; the version never lower than the one installed, and never one rolled
//!   back here.
//! - **A version** is assembled in a hidden directory of its own (hard links to the objects, the
//!   manifest, its signature, and `index.hbc` compiled from the entry's JavaScript), its tree
//!   checked whole, then renamed into place. Fetching and compiling run without the state's lock,
//!   so a session's `hold` or `shown` never waits for a download.
//! - **Each account its own** (decisions 6 and 10): the objects, the version trees with their
//!   bytecode, and the pinned key are the machine's, once; which version an account runs, the one
//!   before, one staged for it, its consents, its rollbacks and its entry are the account's
//!   (`users/<user>/`). No account holds another's update back.
//! - **Switching** is the account's record renamed over the old one, then its entry. The version
//!   before is kept until the new one has shown a window in one of the account's sessions; one
//!   that exits before its first window twice is rolled back for that account and never offered
//!   to it again. A version a session holds is never removed, and that account's current version
//!   is not switched away from while it holds it.
//! - **Consent:** an update that declares a capability shown at install that the account's
//!   current version does not, or that is signed by a rotated key, is staged for the account
//!   until the account consents.
//! - **One update state** (decision 16): a check reads the store's metadata and verifies each
//!   update's manifest once for the machine; each account's state (what it can update, the size
//!   to fetch, release notes, what needs its consent) is computed from it and the account's
//!   records. A check asked for while one runs joins it. Watchers are told each change.
//! - **Removal and collection:** a version tree goes once no account's current, previous or staged
//!   version names it and no session holds it; objects once no tree links them.

use crate::accounts::{Account, Accounts};
use crate::compile::Compile;
use crate::fetch::{Fetch, FetchError};
use crate::state::{
    self, read_record, write_record, Consent, Decision, Installed, Layout, Staged, UserInstall,
    FORMAT,
};
use crate::tuf::{self, TufError};
use oxyos_package::json::Value;
use oxyos_package::{
    lineage_accepts, Architecture, DeveloperSignature, Lineage, Manifest, PublicKey,
    ReleaseStatement, Runtime, Sha256, SignedRelease, HERMES_BYTECODE_MAGIC,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

/// The image's trusted root of the store's metadata.
pub const ANCHOR: &str = "/usr/share/oxyos/store/root.json";
/// Where the image's runtimes are: `<abi>/runtime.json` and `<abi>/hermesc`.
pub const RUNTIMES: &str = "/usr/lib/oxyos/runtimes";
/// How many launches may exit before a first window before a new version is rolled back.
pub const UNSHOWN_EXITS: u32 = 2;

/// Why an install, an update or a removal did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// The peer may not do this (another account's, or not an account).
    NotPermitted,
    /// No release of the application for this machine (its runtimes and architecture).
    NoRelease,
    /// The account does not have the application.
    NotInstalled,
    /// Lower than the version installed.
    Downgrade,
    /// A release this machine rolled back.
    RolledBack,
    /// The plan no longer applies: the state changed since it was prepared.
    Stale,
    /// The store's metadata was refused.
    Store(TufError),
    /// The package was refused.
    Package(oxyos_package::Error),
    /// The store's custom data for the release is not what it must be.
    Release(String),
    Fetch(String),
    Compile(String),
    /// The machine's own files.
    Io(String),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallError::NotPermitted => f.write_str("not permitted"),
            InstallError::NoRelease => f.write_str("no release for this computer"),
            InstallError::NotInstalled => f.write_str("not installed for this account"),
            InstallError::Downgrade => f.write_str("older than the version installed"),
            InstallError::RolledBack => f.write_str("a release this computer rolled back"),
            InstallError::Stale => f.write_str("the plan no longer applies"),
            InstallError::Store(e) => write!(f, "{e}"),
            InstallError::Package(e) => write!(f, "{e}"),
            InstallError::Release(e) => write!(f, "the store's release: {e}"),
            InstallError::Fetch(e) => write!(f, "fetching: {e}"),
            InstallError::Compile(e) => write!(f, "compiling: {e}"),
            InstallError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<TufError> for InstallError {
    fn from(e: TufError) -> Self {
        InstallError::Store(e)
    }
}

impl From<oxyos_package::Error> for InstallError {
    fn from(e: oxyos_package::Error) -> Self {
        InstallError::Package(e)
    }
}

impl From<io::Error> for InstallError {
    fn from(e: io::Error) -> Self {
        InstallError::Io(e.to_string())
    }
}

fn io_at(path: &Path) -> impl Fn(io::Error) -> InstallError + '_ {
    move |e| InstallError::Io(format!("{}: {e}", path.display()))
}

/// The machine: its runtimes, each with its compiler, and its architecture.
#[derive(Clone, Debug)]
pub struct Machine {
    pub runtimes: Vec<(Runtime, PathBuf)>,
    pub architecture: Architecture,
}

impl Machine {
    /// The runtimes in `directory`: each `<abi>/runtime.json` (`oxyos_package::RuntimeDescription`:
    /// `abi`, `api` and its framework's digests, its `abi` its directory's name) beside its
    /// `hermesc`. One that cannot be read is left out.
    pub fn discover(directory: &Path, architecture: Architecture) -> Self {
        let mut runtimes = Vec::new();
        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten() {
                let dir = entry.path();
                let Some(abi) = entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<u32>().ok())
                else {
                    continue;
                };
                let Ok(bytes) = fs::read(dir.join("runtime.json")) else {
                    continue;
                };
                // The one reading of a runtime's description, the session's launcher's too.
                let Ok(description) = oxyos_package::RuntimeDescription::parse(&bytes) else {
                    continue;
                };
                if description.runtime.abi == abi && dir.join("hermesc").is_file() {
                    runtimes.push((description.runtime, dir.join("hermesc")));
                }
            }
        }
        runtimes.sort_by_key(|(r, _)| (r.abi, r.api));
        Self {
            runtimes,
            architecture,
        }
    }

    /// The runtime that runs a package of `runtime`, and its compiler.
    pub fn runtime_for(&self, runtime: &Runtime) -> Option<&(Runtime, PathBuf)> {
        self.runtimes.iter().rev().find(|(r, _)| runtime.runs_on(r))
    }
}

/// This machine's architecture.
pub fn machine_architecture() -> Architecture {
    match std::env::consts::ARCH {
        "aarch64" => Architecture::Aarch64,
        _ => Architecture::X86_64,
    }
}

fn member<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

/// A release the store's metadata offers, verified as far as the metadata alone allows.
#[derive(Clone, Debug)]
struct Release {
    path: String,
    statement: ReleaseStatement,
    signature: DeveloperSignature,
    developer_key: PublicKey,
    lineage: Lineage,
    /// What is new, by language: the store's text, signed with the targets by TUF, not by the
    /// developer.
    notes: BTreeMap<String, String>,
}

/// The releases of `id` in `targets` this machine can run, the highest first.
fn releases(client: &tuf::Client, id: &str, machine: &Machine, bad: &[u64]) -> Vec<Release> {
    let Some(targets) = client.targets() else {
        return Vec::new();
    };
    let store_keys: Vec<PublicKey> = client
        .root()
        .public_keys("targets")
        .into_iter()
        .map(PublicKey)
        .collect();
    let mut found = Vec::new();
    for (path, target) in &targets.targets {
        let parts: Vec<&str> = path.split('/').collect();
        let [package, code, arch, "manifest.json"] = parts.as_slice() else {
            continue;
        };
        if *package != id {
            continue;
        }
        let (Ok(version_code), Ok(architecture)) = (code.parse::<u64>(), Architecture::parse(arch))
        else {
            continue;
        };
        if architecture != Architecture::Any && architecture != machine.architecture {
            continue;
        }
        let Some(release) = target
            .custom
            .as_ref()
            .and_then(|custom| parse_custom(custom, &store_keys).ok())
        else {
            continue;
        };
        let statement = &release.0;
        if statement.package_id != id
            || statement.version_code != version_code
            || statement.architecture != architecture
            || statement.manifest_length != target.length
            || statement.manifest_sha256.0 != target.sha256
            || machine.runtime_for(&statement.runtime).is_none()
            || bad.contains(&version_code)
        {
            continue;
        }
        found.push(Release {
            path: path.clone(),
            statement: release.0,
            signature: release.1,
            developer_key: release.2,
            lineage: release.3,
            notes: release.4,
        });
    }
    found.sort_by(|a, b| b.statement.version_code.cmp(&a.statement.version_code));
    found
}

/// Release notes' bounds: languages, and bytes in each.
const MAX_NOTE_LANGUAGES: usize = 32;
const MAX_NOTE: usize = 4096;

/// The pieces of a target's `custom.oxyos`.
type Custom = (
    ReleaseStatement,
    DeveloperSignature,
    PublicKey,
    Lineage,
    BTreeMap<String, String>,
);

/// A target's `custom.oxyos`: the store's signed statement (verified with the targets role's
/// keys), the developer's signature, the developer's key and its lineage, and the release notes
/// by language (`releaseNotes`, the store's: TUF signs them with the targets, the developer does
/// not).
fn parse_custom(custom: &Value, store_keys: &[PublicKey]) -> Result<Custom, InstallError> {
    let oxyos = member(custom, "oxyos").ok_or_else(|| InstallError::Release("no oxyos".into()))?;
    let text = |key| match member(oxyos, key) {
        Some(Value::String(s)) => Ok(s.as_str()),
        _ => Err(InstallError::Release(format!("no {key}"))),
    };
    let statement = SignedRelease::verify(text("release")?.as_bytes(), store_keys)?;
    let signature = DeveloperSignature::parse(text("signature")?.as_bytes())?;
    let developer_key = PublicKey::parse(text("developerKey")?)?;
    let lineage = match member(oxyos, "lineage") {
        Some(Value::String(s)) => Lineage::parse(s.as_bytes())?,
        None => Lineage::default(),
        Some(_) => return Err(InstallError::Release("lineage is not text".into())),
    };
    let mut notes = BTreeMap::new();
    match member(oxyos, "releaseNotes") {
        None => {}
        Some(Value::Object(members)) if members.len() <= MAX_NOTE_LANGUAGES => {
            for (language, text) in members {
                let valid_language = !language.is_empty()
                    && language.len() <= 35
                    && language
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-');
                match text {
                    Value::String(text) if valid_language && text.len() <= MAX_NOTE => {
                        notes.insert(language.clone(), text.clone());
                    }
                    _ => return Err(InstallError::Release("a release note".into())),
                }
            }
        }
        Some(_) => return Err(InstallError::Release("releaseNotes".into())),
    }
    Ok((statement, signature, developer_key, lineage, notes))
}

/// A capability as an install sheet shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannedCapability {
    pub name: String,
    pub purpose: BTreeMap<String, String>,
    pub asked_at_first_use: bool,
}

/// What an install or update would do, for the system's sheet (decision 9).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Plan {
    pub id: String,
    pub name: String,
    pub names: BTreeMap<String, String>,
    pub version: String,
    pub version_code: u64,
    pub runtime_abi: u32,
    /// Bytes to fetch: the files the machine does not have.
    pub download: u64,
    pub capabilities: Vec<PlannedCapability>,
    /// Capabilities shown at install the account's current version does not declare.
    pub new_capabilities: Vec<String>,
    /// Signed by another key of the developer's lineage than the account's current version.
    pub key_rotation: bool,
    /// The account has a version of it.
    pub update: bool,
}

/// A release verified: its manifest, and the developer key the machine pins with it.
#[derive(Clone, Debug)]
struct Verified {
    release: Release,
    manifest_bytes: Vec<u8>,
    manifest: Manifest,
    pinned: PublicKey,
}

/// A plan with what it was made from, kept by the conversation until it is committed.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub plan: Plan,
    uid: u32,
    verified: Verified,
}

/// How a commit is going.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Progress {
    Fetching { done: u64, total: u64 },
    Compiling,
}

/// What a commit came to, for its account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Committed {
    pub id: String,
    pub version_code: u64,
    /// It is the account's current version now.
    pub switched: bool,
    /// It waits for the account's running copy to stop: its next launch runs it.
    pub at_next_launch: bool,
}

/// An account's application, as `list` gives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Listed {
    pub id: String,
    pub version: String,
    pub version_code: u64,
    /// The account's entry for it.
    pub entry: String,
    pub capabilities: BTreeMap<String, Decision>,
    /// A version staged for the account, and what it waits for.
    pub staged: Option<Staged>,
}

/// What an update came to for an account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Updated {
    pub id: String,
    pub version_code: Option<u64>,
    /// `switched`, `atNextLaunch`, `awaitingConsent`, `upToDate` or `failed`.
    pub outcome: String,
    pub reason: Option<String>,
}

/// What an update of an application is, for an account (decision 16).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AvailableUpdate {
    pub id: String,
    pub name: String,
    pub names: BTreeMap<String, String>,
    pub from_version: String,
    pub from_version_code: u64,
    pub to_version: String,
    pub to_version_code: u64,
    /// Bytes to fetch: only the objects the machine lacks.
    pub download: u64,
    /// What is new, by language: the store's text (TUF signs it with the targets; the developer
    /// does not).
    pub release_notes: BTreeMap<String, String>,
    /// `available`, `awaitingConsent` (the account's consent, `consent` says for what) or
    /// `atNextLaunch` (applied once the account's running copy stops).
    pub status: String,
    pub consent: Option<Consent>,
}

/// An account's update state (decision 16): what Settings' Software Update and Atlas's Updates
/// both show.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateState {
    /// A check is running.
    pub checking: bool,
    /// When the store was last read, as TUF writes a time; none since the service started.
    pub checked_at: Option<String>,
    /// The last check's failure, if it failed.
    pub check_failed: Option<String>,
    /// The account's automatic updates of its applications (on by default).
    pub automatic: bool,
    pub updates: Vec<AvailableUpdate>,
}

/// `users/<user>/settings.json`: the account's own options.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UserSettings {
    format: u32,
    automatic: bool,
}

/// What the service keeps in memory: the versions sessions hold, by account.
#[derive(Default)]
struct Memory {
    holds: BTreeMap<(u32, String, u64), usize>,
}

/// The last check, and whether one runs.
#[derive(Default)]
struct Checks {
    running: bool,
    /// Bumped as each check ends, so a joiner knows its check ended.
    generation: u64,
    checked_at: Option<i64>,
    failed: Option<String>,
    /// For each application on the machine, its releases newer than some account's current
    /// version, verified, newest first.
    candidates: BTreeMap<String, Vec<Verified>>,
}

/// A watcher of an account's update state: told the state at each change; `false` from it ends
/// the watch.
pub type Watcher = Box<dyn Fn(&UpdateState) -> bool + Send>;

/// The install service's logic.
pub struct Installer<F, C, A> {
    layout: Layout,
    anchor: PathBuf,
    fetch: F,
    compile: C,
    accounts: A,
    machine: Machine,
    now: fn() -> i64,
    /// Whether records are given the account's group (only root can).
    chown: bool,
    /// Every change of the records and the holds.
    state: Mutex<Memory>,
    /// The store's metadata, refreshed by one caller at a time, apart from the records' lock so
    /// a session's `hold` or `shown` never waits for the network.
    metadata: Mutex<()>,
    checks: Mutex<Checks>,
    checked: Condvar,
    watchers: Mutex<Vec<(u64, u32, Watcher)>>,
}

/// An account's data directories, relative to its home: what the session removes when the
/// account removes the application (the service never touches a home).
pub fn data_directories(id: &str) -> Vec<String> {
    vec![
        format!(".local/share/oxyos/apps/{id}"),
        format!(".cache/oxyos/apps/{id}"),
    ]
}

/// What an account's update to `to` needs its consent for, from its current version `from`.
fn consent_needed(from: &Facts, to: &Manifest, to_key: &str) -> Option<Consent> {
    let declared: BTreeSet<&str> = from
        .manifest
        .capabilities
        .iter()
        .map(|c| c.capability.name())
        .collect();
    let capabilities: Vec<String> = to
        .capabilities
        .iter()
        .filter(|c| !c.capability.asked_at_first_use() && !declared.contains(c.capability.name()))
        .map(|c| c.capability.name().to_string())
        .collect();
    let key_rotation = from.key_id != to_key;
    (key_rotation || !capabilities.is_empty()).then_some(Consent {
        capabilities,
        key_rotation,
    })
}

/// A version's facts, from its tree: its manifest and the key that signed it.
#[derive(Clone, Debug)]
struct Facts {
    manifest: Manifest,
    key_id: String,
}

impl<F: Fetch, C: Compile, A: Accounts> Installer<F, C, A> {
    /// The installer over `layout`'s directory, trusting the store root at `anchor`. Recovers
    /// from a crash: what was being fetched or assembled is removed, every account's entry is
    /// written again from its record, and what nothing names is collected.
    pub fn open(
        layout: Layout,
        anchor: &Path,
        fetch: F,
        compile: C,
        accounts: A,
        machine: Machine,
        now: fn() -> i64,
    ) -> Result<Self, InstallError> {
        layout.ensure().map_err(io_at(&layout.root))?;
        let installer = Self {
            layout,
            anchor: anchor.to_path_buf(),
            fetch,
            compile,
            accounts,
            machine,
            now,
            chown: rustix::process::geteuid().is_root(),
            state: Mutex::new(Memory::default()),
            metadata: Mutex::new(()),
            checks: Mutex::new(Checks::default()),
            checked: Condvar::new(),
            watchers: Mutex::new(Vec::new()),
        };
        installer.recover()?;
        Ok(installer)
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    #[cfg(test)]
    pub(crate) fn fetch_for_tests(&self) -> &F {
        &self.fetch
    }

    fn memory(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn checks(&self) -> std::sync::MutexGuard<'_, Checks> {
        self.checks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn account(&self, uid: u32) -> Result<Account, InstallError> {
        self.accounts.account(uid).ok_or(InstallError::NotPermitted)
    }

    fn installed(&self, id: &str) -> Result<Option<Installed>, InstallError> {
        valid_id(id)?;
        let path = self.layout.installed().join(format!("{id}.json"));
        read_record(&path).map_err(io_at(&path))
    }

    fn user_install(
        &self,
        account: &Account,
        id: &str,
    ) -> Result<Option<UserInstall>, InstallError> {
        valid_id(id)?;
        let path = self.layout.user(&account.name).join(format!("{id}.json"));
        read_record(&path).map_err(io_at(&path))
    }

    /// Every account's install of `id`.
    fn installs(&self, id: &str) -> Result<Vec<UserInstall>, InstallError> {
        let mut installs = Vec::new();
        let users = self.layout.users();
        for entry in fs::read_dir(&users).map_err(io_at(&users))?.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let path = entry.path().join(format!("{id}.json"));
            if let Some(record) = read_record::<UserInstall>(&path).map_err(io_at(&path))? {
                installs.push(record);
            }
        }
        Ok(installs)
    }

    /// The account's applications' ids.
    fn ids_of(&self, account: &Account) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(self.layout.user(&account.name))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".json"))
                    .filter(|n| !n.starts_with('.') && valid_id(n).is_ok())
                    .map(str::to_string)
            })
            .collect();
        ids.sort();
        ids
    }

    /// The applications on the machine.
    fn machine_ids(&self) -> Result<Vec<String>, InstallError> {
        let dir = self.layout.installed();
        let mut ids: Vec<String> = fs::read_dir(&dir)
            .map_err(io_at(&dir))?
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".json"))
                    .filter(|n| !n.starts_with('.'))
                    .map(str::to_string)
            })
            .collect();
        ids.sort();
        Ok(ids)
    }

    fn facts(&self, id: &str, code: u64) -> Result<Facts, InstallError> {
        let dir = self.layout.version(id, code);
        let bytes = fs::read(dir.join("manifest.json")).map_err(io_at(&dir))?;
        let signature = fs::read(dir.join("manifest.sig")).map_err(io_at(&dir))?;
        Ok(Facts {
            manifest: Manifest::parse(&bytes)?,
            key_id: DeveloperSignature::parse(&signature)?
                .key_id
                .as_str()
                .to_string(),
        })
    }

    fn client(&self) -> Result<tuf::Client, InstallError> {
        Ok(tuf::Client::open(&self.layout.store(), &self.anchor)?)
    }

    /// The bytes the machine would fetch for `manifest`: the objects it lacks, each once.
    fn download(&self, manifest: &Manifest) -> u64 {
        let mut seen = BTreeSet::new();
        manifest
            .files
            .iter()
            .filter(|f| seen.insert(f.sha256.0) && !self.layout.object(&f.sha256).exists())
            .map(|f| f.size)
            .sum()
    }

    /// Verifies `release`'s manifest, fetched from the store: the key the machine pins with it.
    fn verify(
        &self,
        client: &tuf::Client,
        release: Release,
        installed: Option<&Installed>,
    ) -> Result<Verified, InstallError> {
        let manifest_bytes = client.fetch_target(&self.fetch, &release.path)?;
        let manifest = Manifest::parse(&manifest_bytes)?;
        release
            .statement
            .matches(&manifest, &manifest_bytes, &release.signature)?;
        // Key continuity is the application's on this machine: an account installing later is
        // never handed another key than the one pinned, or the one its lineage hands over to.
        let pinned = match installed {
            None => release.developer_key,
            Some(installed) => {
                let pinned = PublicKey::parse(&installed.pinned_key)?;
                let current =
                    lineage_accepts(&pinned, &release.lineage, &release.signature.key_id)?;
                if current != release.developer_key {
                    return Err(InstallError::Release(
                        "the developer's key is not the lineage's".into(),
                    ));
                }
                current
            }
        };
        if pinned.key_id() != release.statement.developer_key_id {
            return Err(InstallError::Release(
                "the statement names another developer key".into(),
            ));
        }
        release.signature.verify(&manifest_bytes, &pinned)?;
        if self.machine.runtime_for(&manifest.runtime).is_none() {
            return Err(InstallError::NoRelease);
        }
        Ok(Verified {
            release,
            manifest_bytes,
            manifest,
            pinned,
        })
    }

    /// What installing (or updating) `id` for the account `uid` would do: the store's metadata
    /// refreshed, the newest release for this machine the account has not rolled back found, and
    /// its manifest fetched and verified. Nothing changes but the store's metadata.
    pub fn prepare(&self, uid: u32, id: &str) -> Result<Prepared, InstallError> {
        let account = self.account(uid)?;
        valid_id(id)?;
        let _metadata = self
            .metadata
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let installed = self.installed(id)?;
        let mine = self.user_install(&account, id)?;
        let mut client = self.client()?;
        client.refresh(&self.fetch, (self.now)())?;
        let bad = mine.as_ref().map(|u| u.bad.clone()).unwrap_or_default();
        let release = releases(&client, id, &self.machine, &bad)
            .into_iter()
            .next()
            .ok_or(InstallError::NoRelease)?;
        if let Some(mine) = &mine {
            if release.statement.version_code < mine.current {
                return Err(InstallError::Downgrade);
            }
        }
        let verified = self.verify(&client, release, installed.as_ref())?;
        let manifest = &verified.manifest;
        let (new_capabilities, key_rotation, update) = match &mine {
            Some(mine) if mine.current != manifest.version_code => {
                let from = self.facts(id, mine.current)?;
                let consent =
                    consent_needed(&from, manifest, verified.release.signature.key_id.as_str());
                (
                    consent
                        .as_ref()
                        .map(|c| c.capabilities.clone())
                        .unwrap_or_default(),
                    consent.is_some_and(|c| c.key_rotation),
                    true,
                )
            }
            _ => (Vec::new(), false, false),
        };
        let plan = Plan {
            id: manifest.id.clone(),
            name: manifest.name.as_str().to_string(),
            names: names_of(manifest),
            version: manifest.version.clone(),
            version_code: manifest.version_code,
            runtime_abi: manifest.runtime.abi,
            download: self.download(manifest),
            capabilities: manifest
                .capabilities
                .iter()
                .map(|c| PlannedCapability {
                    name: c.capability.name().to_string(),
                    purpose: c
                        .purpose
                        .iter()
                        .map(|(k, v)| (k.clone(), v.as_str().to_string()))
                        .collect(),
                    asked_at_first_use: c.capability.asked_at_first_use(),
                })
                .collect(),
            new_capabilities,
            key_rotation,
            update,
        };
        Ok(Prepared {
            plan,
            uid,
            verified,
        })
    }

    /// Installs or updates as `prepared` says, for its account: the person confirmed it in the
    /// system's sheet, which is the account's consent. `progress` is told how it goes.
    pub fn commit(
        &self,
        prepared: &Prepared,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Committed, InstallError> {
        let account = self.account(prepared.uid)?;
        let result = self.apply(&account, &prepared.verified, true, progress);
        self.notify(&[account.uid]);
        let (switched, at_next_launch) = match result? {
            Applied::Switched => (true, false),
            Applied::AtNextLaunch => (false, true),
            Applied::AwaitingConsent => (false, false),
        };
        Ok(Committed {
            id: prepared.verified.manifest.id.clone(),
            version_code: prepared.verified.manifest.version_code,
            switched,
            at_next_launch,
        })
    }

    /// Makes `verified` the account's version: assembled once for the machine if it is not
    /// there, then switched to for the account, or staged for it (its consent, unless
    /// `consented`; or its running copy).
    fn apply(
        &self,
        account: &Account,
        verified: &Verified,
        consented: bool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Applied, InstallError> {
        let id = verified.manifest.id.as_str();
        let code = verified.manifest.version_code;
        {
            let _memory = self.memory();
            if let Some(mine) = self.user_install(account, id)? {
                if code < mine.current {
                    return Err(InstallError::Downgrade);
                }
                if mine.bad.contains(&code) {
                    return Err(InstallError::RolledBack);
                }
            }
        }
        let assembled = if self.layout.version(id, code).is_dir() {
            None
        } else {
            Some(self.assemble(verified, progress)?)
        };
        let mut memory = self.memory();
        if let Some(assembled) = assembled {
            self.place(&assembled, id, code)?;
        }
        // The machine's pinned key follows the lineage the release was verified through.
        let pinned = verified.pinned.encode();
        if self.installed(id)?.is_none_or(|i| i.pinned_key != pinned) {
            self.write_installed(&Installed {
                format: FORMAT,
                id: id.to_string(),
                pinned_key: pinned,
            })?;
        }
        let applied = match self.user_install(account, id)? {
            None => {
                let mut mine = UserInstall {
                    format: FORMAT,
                    id: id.to_string(),
                    uid: account.uid,
                    user: account.name.clone(),
                    current: code,
                    shown: false,
                    unshown_exits: 0,
                    previous: None,
                    staged: None,
                    bad: Vec::new(),
                    capabilities: BTreeMap::new(),
                };
                decide_capabilities(&mut mine, &verified.manifest);
                self.write_account(account, &mine)?;
                Applied::Switched
            }
            Some(mut mine) if mine.current == code => {
                decide_capabilities(&mut mine, &verified.manifest);
                self.write_account(account, &mine)?;
                Applied::Switched
            }
            Some(mut mine) => {
                let from = self.facts(id, mine.current)?;
                let consent = if consented {
                    None
                } else {
                    consent_needed(
                        &from,
                        &verified.manifest,
                        verified.release.signature.key_id.as_str(),
                    )
                };
                mine.staged = Some(Staged {
                    version_code: code,
                    consent,
                });
                let applied = self.try_switch(account, &mut mine, &memory)?;
                self.write_account(account, &mine)?;
                applied
            }
        };
        self.collect(&mut memory)?;
        Ok(applied)
    }

    /// Switches the account to its staged version if it waits for no consent and none of the
    /// account's sessions holds its current version.
    fn try_switch(
        &self,
        account: &Account,
        mine: &mut UserInstall,
        memory: &Memory,
    ) -> Result<Applied, InstallError> {
        let Some(staged) = mine.staged.clone() else {
            return Ok(Applied::Switched);
        };
        if staged.consent.is_some() {
            return Ok(Applied::AwaitingConsent);
        }
        let held = memory
            .holds
            .get(&(account.uid, mine.id.clone(), mine.current))
            .is_some_and(|n| *n > 0);
        if held {
            return Ok(Applied::AtNextLaunch);
        }
        let to = self.facts(&mine.id, staged.version_code)?;
        mine.previous = Some(mine.current);
        mine.current = staged.version_code;
        mine.shown = false;
        mine.unshown_exits = 0;
        mine.staged = None;
        decide_capabilities(mine, &to.manifest);
        Ok(Applied::Switched)
    }

    fn write_installed(&self, installed: &Installed) -> Result<(), InstallError> {
        let dir = self.layout.installed();
        write_record(
            &dir,
            &format!("{}.json", installed.id),
            installed,
            0o644,
            None,
        )
        .map_err(io_at(&dir))
    }

    /// An account's directory (or its entries'), made root's with the account's group, 0750.
    fn account_directory(&self, dir: &Path, account: &Account) -> Result<(), InstallError> {
        state::ensure_directory(dir, 0o750).map_err(io_at(dir))?;
        if self.chown {
            rustix::fs::chown(
                dir,
                Some(rustix::fs::Uid::from_raw(0)),
                Some(rustix::fs::Gid::from_raw(account.gid)),
            )
            .map_err(|e| InstallError::Io(format!("{}: {e}", dir.display())))?;
        }
        Ok(())
    }

    /// Writes an account's record, then its entry for its current version: each renamed whole
    /// over the old one, root's with the account's group, 0640.
    fn write_account(&self, account: &Account, mine: &UserInstall) -> Result<(), InstallError> {
        let dir = self.layout.user(&account.name);
        self.account_directory(&dir, account)?;
        let owner = self.chown.then_some((0, account.gid));
        write_record(&dir, &format!("{}.json", mine.id), mine, 0o640, owner)
            .map_err(io_at(&dir))?;
        self.write_entry(account, mine)
    }

    /// The account's entry of its current version (`users/<user>/applications/<id>.desktop`):
    /// what the registry lists for it. It asks for no grant: no entry can.
    fn write_entry(&self, account: &Account, mine: &UserInstall) -> Result<(), InstallError> {
        let version_dir = self.layout.version(&mine.id, mine.current);
        let facts = self.facts(&mine.id, mine.current)?;
        let text = entry_text(&facts.manifest, &version_dir, facts.manifest.runtime.abi);
        let dir = self.layout.user_applications(&account.name);
        self.account_directory(&dir, account)?;
        let owner = self.chown.then_some((0, account.gid));
        state::write_file(
            &dir,
            &format!("{}.desktop", mine.id),
            text.as_bytes(),
            0o640,
            owner,
        )
        .map_err(io_at(&dir))
    }

    fn remove_account_files(&self, account: &Account, id: &str) -> Result<bool, InstallError> {
        let mut removed = false;
        for (dir, name) in [
            (
                self.layout.user_applications(&account.name),
                format!("{id}.desktop"),
            ),
            (self.layout.user(&account.name), format!("{id}.json")),
        ] {
            match fs::remove_file(dir.join(&name)) {
                Ok(()) => {
                    removed = true;
                    state::sync_directory(&dir).map_err(io_at(&dir))?;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_at(&dir)(e)),
            }
        }
        Ok(removed)
    }

    /// Removes the account's install of `id`: the data directories the session should remove
    /// (relative to the home). The versions it named go once no other account names them.
    pub fn uninstall(&self, uid: u32, id: &str) -> Result<Vec<String>, InstallError> {
        let account = self.account(uid)?;
        valid_id(id)?;
        {
            let mut memory = self.memory();
            if self.user_install(&account, id)?.is_none() {
                return Err(InstallError::NotInstalled);
            }
            self.remove_account_files(&account, id)?;
            self.collect(&mut memory)?;
        }
        self.notify(&[uid]);
        Ok(data_directories(id))
    }

    /// The account's applications.
    pub fn list(&self, uid: u32) -> Result<Vec<Listed>, InstallError> {
        let account = self.account(uid)?;
        let _memory = self.memory();
        let mut listed = Vec::new();
        for id in self.ids_of(&account) {
            let Some(mine) = self.user_install(&account, &id)? else {
                continue;
            };
            let facts = self.facts(&id, mine.current)?;
            listed.push(Listed {
                version: facts.manifest.version,
                version_code: mine.current,
                entry: self
                    .layout
                    .user_applications(&account.name)
                    .join(format!("{id}.desktop"))
                    .display()
                    .to_string(),
                capabilities: mine.capabilities,
                staged: mine.staged,
                id,
            });
        }
        Ok(listed)
    }

    /// Whether the account `uid` has `id`.
    pub fn has(&self, uid: u32, id: &str) -> bool {
        self.accounts
            .account(uid)
            .and_then(|account| self.user_install(&account, id).ok().flatten())
            .is_some()
    }

    /// A session of the account `uid` runs version `code` of `id`: it is never removed, and the
    /// account's current version is not switched away from, until it is released.
    pub fn hold(&self, uid: u32, id: &str, code: u64) -> Result<(), InstallError> {
        let account = self.account(uid)?;
        let mut memory = self.memory();
        let mine = self
            .user_install(&account, id)?
            .ok_or(InstallError::NotInstalled)?;
        let names = [
            Some(mine.current),
            mine.previous,
            mine.staged.map(|s| s.version_code),
        ];
        if !names.contains(&Some(code)) || !self.layout.version(id, code).is_dir() {
            return Err(InstallError::NotInstalled);
        }
        *memory.holds.entry((uid, id.to_string(), code)).or_insert(0) += 1;
        Ok(())
    }

    /// A session of the account `uid` no longer runs version `code` of `id`. The account's staged
    /// version waiting only for it is switched to.
    pub fn release(&self, uid: u32, id: &str, code: u64) -> Result<(), InstallError> {
        let account = self.account(uid)?;
        valid_id(id)?;
        {
            let mut memory = self.memory();
            let key = (uid, id.to_string(), code);
            match memory.holds.get_mut(&key) {
                Some(n) if *n > 1 => *n -= 1,
                Some(_) => {
                    memory.holds.remove(&key);
                }
                None => return Err(InstallError::NotInstalled),
            }
            if let Some(mut mine) = self.user_install(&account, id)? {
                if mine.staged.is_some()
                    && self.try_switch(&account, &mut mine, &memory)? == Applied::Switched
                {
                    self.write_account(&account, &mine)?;
                }
            }
            self.collect(&mut memory)?;
        }
        self.notify(&[uid]);
        Ok(())
    }

    /// A session of the account `uid` showed a window of version `code` of `id`: the account's
    /// version before it is no longer kept for a rollback.
    pub fn shown(&self, uid: u32, id: &str, code: u64) -> Result<(), InstallError> {
        let account = self.account(uid)?;
        let mut memory = self.memory();
        let mut mine = self
            .user_install(&account, id)?
            .ok_or(InstallError::NotInstalled)?;
        if mine.current == code && !mine.shown {
            mine.shown = true;
            mine.previous = None;
            self.write_account(&account, &mine)?;
            self.collect(&mut memory)?;
        }
        Ok(())
    }

    /// A launch of version `code` of `id` by the account `uid` exited, having shown a window or
    /// not. A new version that exits before its first window [`UNSHOWN_EXITS`] times is rolled
    /// back for the account to its version before and never offered to it again: `true` if so.
    pub fn exited(&self, uid: u32, id: &str, code: u64, shown: bool) -> Result<bool, InstallError> {
        if shown {
            self.shown(uid, id, code)?;
            return Ok(false);
        }
        let account = self.account(uid)?;
        let rolled_back = {
            let mut memory = self.memory();
            let mut mine = self
                .user_install(&account, id)?
                .ok_or(InstallError::NotInstalled)?;
            if mine.current != code || mine.shown {
                return Ok(false);
            }
            mine.unshown_exits += 1;
            let rolled_back = mine.unshown_exits >= UNSHOWN_EXITS && mine.previous.is_some();
            if let (true, Some(previous)) = (rolled_back, mine.previous) {
                mine.bad.push(code);
                mine.current = previous;
                mine.previous = None;
                mine.shown = true;
                mine.unshown_exits = 0;
            }
            self.write_account(&account, &mine)?;
            if rolled_back {
                self.collect(&mut memory)?;
            }
            rolled_back
        };
        if rolled_back {
            self.notify(&[uid]);
        }
        Ok(rolled_back)
    }

    /// Checks the store for updates of every application on the machine, once: a check asked for
    /// while one runs joins it and is answered when it ends. Watchers are told it start and end.
    pub fn check(&self) -> Result<(), InstallError> {
        {
            let mut checks = self.checks();
            if checks.running {
                let generation = checks.generation;
                while checks.running && checks.generation == generation {
                    checks = self
                        .checked
                        .wait(checks)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                return match &checks.failed {
                    Some(reason) => Err(InstallError::Fetch(reason.clone())),
                    None => Ok(()),
                };
            }
            checks.running = true;
        }
        self.notify_all();
        let result = self.run_check();
        {
            let mut checks = self.checks();
            checks.running = false;
            checks.generation += 1;
            match &result {
                Ok(candidates) => {
                    checks.candidates = candidates.clone();
                    checks.checked_at = Some((self.now)());
                    checks.failed = None;
                }
                Err(e) => checks.failed = Some(e.to_string()),
            }
        }
        self.checked.notify_all();
        self.notify_all();
        result.map(|_| ())
    }

    /// The store's metadata refreshed, and for each application on the machine its releases newer
    /// than some account's current version (and not every such account's rolled back),
    /// verified, newest first.
    fn run_check(&self) -> Result<BTreeMap<String, Vec<Verified>>, InstallError> {
        let _metadata = self
            .metadata
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut client = self.client()?;
        client.refresh(&self.fetch, (self.now)())?;
        let mut candidates = BTreeMap::new();
        for id in self.machine_ids()? {
            let installed = self.installed(&id)?;
            let installs = self.installs(&id)?;
            let mut verified = Vec::new();
            for release in releases(&client, &id, &self.machine, &[]) {
                let code = release.statement.version_code;
                let wanted = installs
                    .iter()
                    .any(|mine| code > mine.current && !mine.bad.contains(&code));
                if !wanted {
                    continue;
                }
                // A release that does not verify is left out; the store's next one may.
                if let Ok(v) = self.verify(&client, release, installed.as_ref()) {
                    verified.push(v);
                }
                if verified.len() >= 3 {
                    break;
                }
            }
            if !verified.is_empty() {
                candidates.insert(id, verified);
            }
        }
        Ok(candidates)
    }

    /// The release the account `uid` would update `id` to, from the last check.
    fn candidate_for(&self, mine: &UserInstall) -> Option<Verified> {
        self.checks()
            .candidates
            .get(&mine.id)?
            .iter()
            .find(|v| {
                let code = v.manifest.version_code;
                code > mine.current && !mine.bad.contains(&code)
            })
            .cloned()
    }

    /// The account's update state (decision 16), from the last check and its records.
    pub fn update_state(&self, uid: u32) -> Result<UpdateState, InstallError> {
        let account = self.account(uid)?;
        let (checking, checked_at, check_failed) = {
            let checks = self.checks();
            (
                checks.running,
                checks.checked_at.map(crate::time::format_utc),
                checks.failed.clone(),
            )
        };
        let mut updates = Vec::new();
        let memory = self.memory();
        for id in self.ids_of(&account) {
            let Some(mine) = self.user_install(&account, &id)? else {
                continue;
            };
            let from = self.facts(&id, mine.current)?;
            let available =
                |to: &Manifest, notes: BTreeMap<String, String>, status: &str, consent| {
                    AvailableUpdate {
                        id: id.clone(),
                        name: to.name.as_str().to_string(),
                        names: names_of(to),
                        from_version: from.manifest.version.clone(),
                        from_version_code: mine.current,
                        to_version: to.version.clone(),
                        to_version_code: to.version_code,
                        download: self.download(to),
                        release_notes: notes,
                        status: status.to_string(),
                        consent,
                    }
                };
            if let Some(staged) = &mine.staged {
                let to = self.facts(&id, staged.version_code)?;
                let notes = self
                    .checks()
                    .candidates
                    .get(&id)
                    .and_then(|c| {
                        c.iter()
                            .find(|v| v.manifest.version_code == staged.version_code)
                    })
                    .map(|v| v.release.notes.clone())
                    .unwrap_or_default();
                let status = if staged.consent.is_some() {
                    "awaitingConsent"
                } else {
                    "atNextLaunch"
                };
                updates.push(available(
                    &to.manifest,
                    notes,
                    status,
                    staged.consent.clone(),
                ));
                continue;
            }
            if let Some(v) = self.candidate_for(&mine) {
                let consent =
                    consent_needed(&from, &v.manifest, v.release.signature.key_id.as_str());
                let status = if consent.is_some() {
                    "awaitingConsent"
                } else if memory
                    .holds
                    .get(&(uid, id.clone(), mine.current))
                    .is_some_and(|n| *n > 0)
                {
                    "atNextLaunch"
                } else {
                    "available"
                };
                updates.push(available(
                    &v.manifest,
                    v.release.notes.clone(),
                    status,
                    consent,
                ));
            }
        }
        drop(memory);
        Ok(UpdateState {
            checking,
            checked_at,
            check_failed,
            automatic: self.settings(&account)?.automatic,
            updates,
        })
    }

    /// Updates the account's applications `ids` (all with an update, for `None`) to the last
    /// check's releases, checking first if there has been no check: those needing the account's
    /// consent are staged for it and answered `awaitingConsent`; a running one is updated at its
    /// next launch.
    pub fn update(&self, uid: u32, ids: Option<&[String]>) -> Result<Vec<Updated>, InstallError> {
        let account = self.account(uid)?;
        if self.checks().checked_at.is_none() {
            self.check()?;
        }
        let ids: Vec<String> = match ids {
            Some(ids) => ids.to_vec(),
            None => self.ids_of(&account),
        };
        let mut outcomes = Vec::new();
        for id in ids {
            let outcome = |code, outcome: &str, reason: Option<String>| Updated {
                id: id.clone(),
                version_code: code,
                outcome: outcome.into(),
                reason,
            };
            let mine = match self.user_install(&account, &id) {
                Ok(Some(mine)) => mine,
                Ok(None) => {
                    outcomes.push(outcome(None, "failed", Some("not installed".into())));
                    continue;
                }
                Err(e) => {
                    outcomes.push(outcome(None, "failed", Some(e.to_string())));
                    continue;
                }
            };
            let Some(verified) = self.candidate_for(&mine) else {
                outcomes.push(outcome(Some(mine.current), "upToDate", None));
                continue;
            };
            let code = verified.manifest.version_code;
            outcomes.push(match self.apply(&account, &verified, false, &mut |_| {}) {
                Ok(Applied::Switched) => outcome(Some(code), "switched", None),
                Ok(Applied::AtNextLaunch) => outcome(Some(code), "atNextLaunch", None),
                Ok(Applied::AwaitingConsent) => outcome(Some(code), "awaitingConsent", None),
                Err(e) => outcome(Some(code), "failed", Some(e.to_string())),
            });
        }
        self.notify(&[uid]);
        Ok(outcomes)
    }

    /// The account consents to its staged version `code` of `id` (it confirmed the system's
    /// sheet): switched to for the account, or at its next launch while it runs.
    pub fn consent(&self, uid: u32, id: &str, code: u64) -> Result<Updated, InstallError> {
        let account = self.account(uid)?;
        let applied = {
            let mut memory = self.memory();
            let mut mine = self
                .user_install(&account, id)?
                .ok_or(InstallError::NotInstalled)?;
            match mine.staged.as_mut() {
                Some(staged) if staged.version_code == code => staged.consent = None,
                _ => return Err(InstallError::Stale),
            }
            let applied = self.try_switch(&account, &mut mine, &memory)?;
            self.write_account(&account, &mine)?;
            self.collect(&mut memory)?;
            applied
        };
        self.notify(&[uid]);
        Ok(Updated {
            id: id.to_string(),
            version_code: Some(code),
            outcome: match applied {
                Applied::Switched => "switched",
                Applied::AtNextLaunch => "atNextLaunch",
                Applied::AwaitingConsent => "awaitingConsent",
            }
            .into(),
            reason: None,
        })
    }

    fn settings(&self, account: &Account) -> Result<UserSettings, InstallError> {
        let path = self.layout.user(&account.name).join("settings.json");
        Ok(read_record(&path)
            .map_err(io_at(&path))?
            .unwrap_or(UserSettings {
                format: FORMAT,
                automatic: true,
            }))
    }

    /// The account's automatic updates of its applications, on or off (decision 16).
    pub fn set_automatic(&self, uid: u32, automatic: bool) -> Result<(), InstallError> {
        let account = self.account(uid)?;
        {
            let _memory = self.memory();
            let dir = self.layout.user(&account.name);
            self.account_directory(&dir, &account)?;
            write_record(
                &dir,
                "settings.json",
                &UserSettings {
                    format: FORMAT,
                    automatic,
                },
                0o640,
                self.chown.then_some((0, account.gid)),
            )
            .map_err(io_at(&dir))?;
        }
        self.notify(&[uid]);
        Ok(())
    }

    /// What the update timer runs: a check, then each account with automatic updates on updated
    /// as `update` would (what needs consent waits for it). The accounts updated, by uid.
    pub fn automatic(&self) -> Result<Vec<(u32, Vec<Updated>)>, InstallError> {
        self.check()?;
        let mut done = Vec::new();
        let users = self.layout.users();
        let mut uids: Vec<u32> = Vec::new();
        for entry in fs::read_dir(&users).map_err(io_at(&users))?.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            for id in fs::read_dir(entry.path()).into_iter().flatten().flatten() {
                let path = id.path();
                if path.extension().is_some_and(|e| e == "json") && !path.ends_with("settings.json")
                {
                    if let Ok(Some(mine)) = read_record::<UserInstall>(&path) {
                        if mine.user == name && !uids.contains(&mine.uid) {
                            uids.push(mine.uid);
                        }
                    }
                }
            }
        }
        for uid in uids {
            let Ok(account) = self.account(uid) else {
                continue;
            };
            if self.settings(&account)?.automatic {
                done.push((uid, self.update(uid, None)?));
            }
        }
        Ok(done)
    }

    /// Watches the account `uid`'s update state: `watcher` is told it now and at each change. Its
    /// id, for [`Self::unwatch`].
    pub fn watch(&self, uid: u32, watcher: Watcher) -> Result<u64, InstallError> {
        let state = self.update_state(uid)?;
        if !watcher(&state) {
            return Err(InstallError::Io("the watcher ended".into()));
        }
        let mut watchers = self
            .watchers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let id = watchers.iter().map(|(id, _, _)| id + 1).max().unwrap_or(1);
        watchers.push((id, uid, watcher));
        Ok(id)
    }

    pub fn unwatch(&self, id: u64) {
        self.watchers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|(w, _, _)| *w != id);
    }

    /// Tells the watchers of these accounts their state. Called with no lock of the records held.
    fn notify(&self, uids: &[u32]) {
        let mut watchers = self
            .watchers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut states: BTreeMap<u32, Option<UpdateState>> = BTreeMap::new();
        watchers.retain(|(_, uid, watcher)| {
            if !uids.contains(uid) {
                return true;
            }
            let state = states
                .entry(*uid)
                .or_insert_with(|| self.update_state(*uid).ok());
            match state {
                Some(state) => watcher(state),
                None => true,
            }
        });
    }

    fn notify_all(&self) {
        let uids: Vec<u32> = self
            .watchers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(_, uid, _)| *uid)
            .collect();
        self.notify(&uids);
    }

    /// Fetches what the machine lacks, assembles and compiles the version in a hidden directory
    /// of its own and checks its tree: that directory. Without the state's lock: objects are
    /// linked into the version before they are named in `objects/`, so a collection never takes
    /// one it needs, and the collection never touches a hidden directory. [`Self::place`] renames
    /// it into place under the lock, with the record that names it.
    fn assemble(
        &self,
        verified: &Verified,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<PathBuf, InstallError> {
        let manifest = &verified.manifest;
        let package_dir = self.layout.packages().join(&manifest.id);
        state::ensure_directory(&package_dir, 0o755).map_err(io_at(&package_dir))?;
        let assembling =
            package_dir.join(format!(".{}.new-{}", manifest.version_code, random_token()));
        state::ensure_directory(&assembling, 0o755).map_err(io_at(&assembling))?;
        let result = self.assemble_into(verified, &assembling, progress);
        if let Err(e) = result {
            // What it fetched goes with it: the objects it named are linked by nothing now.
            let _ = fs::remove_dir_all(&assembling);
            let _ = self.collect(&mut self.memory());
            return Err(e);
        }
        Ok(assembling)
    }

    /// Renames an assembled version into place; one assembled meanwhile by another commit is the
    /// same bytes, and ours goes. Called with the state's lock held.
    fn place(&self, assembled: &Path, id: &str, code: u64) -> Result<(), InstallError> {
        let final_dir = self.layout.version(id, code);
        if final_dir.is_dir() {
            let _ = fs::remove_dir_all(assembled);
            return Ok(());
        }
        let package_dir = self.layout.packages().join(id);
        // The package's directory may have gone with a collection meanwhile.
        state::ensure_directory(&package_dir, 0o755).map_err(io_at(&package_dir))?;
        fs::rename(assembled, &final_dir).map_err(io_at(&final_dir))?;
        state::sync_directory(&package_dir).map_err(io_at(&package_dir))
    }

    fn assemble_into(
        &self,
        verified: &Verified,
        assembling: &Path,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<(), InstallError> {
        let manifest = &verified.manifest;
        let total: u64 = self.download(manifest);
        let mut done = 0;
        progress(Progress::Fetching { done, total });
        for file in &manifest.files {
            let destination = assembling.join(file.path.as_str());
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(io_at(parent))?;
            }
            let object = self.layout.object(&file.sha256);
            match fs::hard_link(&object, &destination) {
                Ok(()) if fs::metadata(&destination).map(|m| m.len()).ok() == Some(file.size) => {
                    continue;
                }
                Ok(()) => {
                    // An object of the wrong size is no object of this digest: fetched again.
                    let _ = fs::remove_file(&destination);
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io_at(&destination)(e)),
            }
            let staged = self.fetch_object(&file.sha256, file.size)?;
            fs::hard_link(&staged, &destination).map_err(io_at(&destination))?;
            let shard = object.parent().unwrap_or(&object).to_path_buf();
            state::ensure_directory(&shard, 0o755).map_err(io_at(&shard))?;
            fs::rename(&staged, &object).map_err(io_at(&object))?;
            state::sync_directory(&shard).map_err(io_at(&shard))?;
            done += file.size;
            progress(Progress::Fetching { done, total });
        }
        state::write_file(
            assembling,
            "manifest.json",
            &verified.manifest_bytes,
            0o444,
            None,
        )?;
        state::write_file(
            assembling,
            "manifest.sig",
            &verified.release.signature.to_canonical(),
            0o444,
            None,
        )?;
        // The whole tree, as the manifest lists it and nothing else, before the compiler reads
        // any of it (the entry not bytecode among the checks).
        let dir = oxyos_package::open_directory(assembling)?;
        oxyos_package::verify_tree(rustix::fd::AsFd::as_fd(&dir), manifest, &[])?;
        progress(Progress::Compiling);
        let (_, hermesc) = self
            .machine
            .runtime_for(&manifest.runtime)
            .ok_or(InstallError::NoRelease)?;
        let source_path = assembling.join(manifest.entry.bundle.as_str());
        let source = File::open(&source_path).map_err(io_at(&source_path))?;
        let out_path = self
            .layout
            .staging()
            .join(format!("compiled-{}", random_token()));
        let out = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&out_path)
            .map_err(io_at(&out_path))?;
        let compiled = self
            .compile
            .compile(hermesc, &source, &out)
            .map_err(InstallError::Compile)
            .and_then(|()| {
                let mut head = [0; 8];
                let mut reader = &out;
                reader.rewind()?;
                reader
                    .read_exact(&mut head)
                    .map_err(|_| InstallError::Compile("the compiler wrote no bytecode".into()))?;
                if head != HERMES_BYTECODE_MAGIC {
                    return Err(InstallError::Compile(
                        "the compiler wrote something other than Hermes bytecode".into(),
                    ));
                }
                out.set_permissions(fs::Permissions::from_mode(0o444))?;
                out.sync_all()?;
                Ok(())
            });
        if let Err(e) = compiled {
            let _ = fs::remove_file(&out_path);
            return Err(e);
        }
        let index = assembling.join("index.hbc");
        fs::rename(&out_path, &index).map_err(io_at(&index))?;
        sync_tree(assembling)?;
        Ok(())
    }

    /// Fetches an object into the staging directory and checks its size and digest: its path.
    fn fetch_object(&self, sha256: &Sha256, size: u64) -> Result<PathBuf, InstallError> {
        let hex = sha256.hex();
        let path = self
            .layout
            .staging()
            .join(format!("object-{hex}-{}", random_token()));
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(io_at(&path))?;
        let result = (|| {
            let location = format!("objects/{}/{hex}", &hex[..2]);
            self.fetch
                .fetch(&location, size, rustix::fd::AsFd::as_fd(&file))
                .map_err(|e| match e {
                    FetchError::NotFound => InstallError::Fetch(format!("{location} is missing")),
                    e => InstallError::Fetch(format!("{location}: {e}")),
                })?;
            file.rewind()?;
            oxyos_package::verify_stream(&mut file, size, sha256, &hex)?;
            file.set_permissions(fs::Permissions::from_mode(0o444))?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&path);
            return Err(e);
        }
        Ok(path)
    }

    /// After a crash: what was being fetched or assembled goes, every account's entry is
    /// written again from its record, and what nothing names is collected.
    fn recover(&self) -> Result<(), InstallError> {
        let mut memory = self.memory();
        let staging = self.layout.staging();
        for entry in fs::read_dir(&staging).map_err(io_at(&staging))?.flatten() {
            let _ = fs::remove_file(entry.path());
        }
        let packages = self.layout.packages();
        for package in fs::read_dir(&packages).map_err(io_at(&packages))?.flatten() {
            for version in fs::read_dir(package.path()).into_iter().flatten().flatten() {
                if version.file_name().to_string_lossy().starts_with('.') {
                    let _ = fs::remove_dir_all(version.path());
                }
            }
        }
        let remove_new = |dir: &Path| {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                if entry.file_name().to_string_lossy().starts_with('.') {
                    let _ = fs::remove_file(entry.path());
                }
            }
        };
        remove_new(&self.layout.installed());
        let users = self.layout.users();
        for user in fs::read_dir(&users).map_err(io_at(&users))?.flatten() {
            let Some(name) = user.file_name().to_str().map(str::to_string) else {
                continue;
            };
            remove_new(&user.path());
            remove_new(&self.layout.user_applications(&name));
            for record in fs::read_dir(user.path()).into_iter().flatten().flatten() {
                let path = record.path();
                let Some(id) = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix(".json"))
                    .filter(|n| valid_id(n).is_ok())
                else {
                    continue;
                };
                let Some(mine) = read_record::<UserInstall>(&path).map_err(io_at(&path))? else {
                    continue;
                };
                let account = self.accounts.account(mine.uid).filter(|a| a.name == name);
                // An entry is listed only for a version that is there to run, of an account that
                // is there.
                let written = account.is_some_and(|a| self.write_entry(&a, &mine).is_ok());
                if !written {
                    let _ = fs::remove_file(
                        self.layout
                            .user_applications(&name)
                            .join(format!("{id}.desktop")),
                    );
                }
            }
        }
        self.collect(&mut memory)
    }

    /// Removes what nothing names: a version tree no account's current, previous or staged
    /// version names and no session holds, an application's record once no account has it, and
    /// objects no tree links (their link count is one).
    fn collect(&self, memory: &mut Memory) -> Result<(), InstallError> {
        let packages = self.layout.packages();
        for package in fs::read_dir(&packages).map_err(io_at(&packages))?.flatten() {
            let Some(id) = package
                .file_name()
                .to_str()
                .filter(|n| valid_id(n).is_ok())
                .map(str::to_string)
            else {
                continue;
            };
            let installs = self.installs(&id)?;
            let mut keep: BTreeSet<u64> = BTreeSet::new();
            for mine in &installs {
                keep.insert(mine.current);
                keep.extend(mine.previous);
                keep.extend(mine.staged.as_ref().map(|s| s.version_code));
            }
            keep.extend(
                memory
                    .holds
                    .keys()
                    .filter(|(_, held, _)| *held == id)
                    .map(|(_, _, code)| *code),
            );
            if installs.is_empty() {
                let dir = self.layout.installed();
                match fs::remove_file(dir.join(format!("{id}.json"))) {
                    Ok(()) => state::sync_directory(&dir).map_err(io_at(&dir))?,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io_at(&dir)(e)),
                }
            }
            let mut left = 0;
            for version in fs::read_dir(package.path()).into_iter().flatten().flatten() {
                let name = version.file_name();
                let Some(code) = name.to_str().and_then(|n| n.parse::<u64>().ok()) else {
                    left += 1;
                    continue;
                };
                if keep.contains(&code) {
                    left += 1;
                    continue;
                }
                let gone = package.path().join(format!(".{code}.gone"));
                fs::rename(version.path(), &gone).map_err(io_at(&gone))?;
                fs::remove_dir_all(&gone).map_err(io_at(&gone))?;
            }
            if left == 0 {
                let _ = fs::remove_dir(package.path());
            }
        }
        let objects = self.layout.objects();
        for shard in fs::read_dir(&objects).map_err(io_at(&objects))?.flatten() {
            for object in fs::read_dir(shard.path()).into_iter().flatten().flatten() {
                if object
                    .metadata()
                    .is_ok_and(|m| m.nlink() == 1 && m.is_file())
                {
                    let _ = fs::remove_file(object.path());
                }
            }
        }
        Ok(())
    }
}

/// What applying a version did for an account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Applied {
    Switched,
    AtNextLaunch,
    AwaitingConsent,
}

/// The account's decisions gain what `manifest` declares and they lack: shown at install,
/// granted (the person confirmed it, or consented to it); asked at first use, undecided.
fn decide_capabilities(mine: &mut UserInstall, manifest: &Manifest) {
    for c in &manifest.capabilities {
        let decision = if c.capability.asked_at_first_use() {
            Decision::Undecided
        } else {
            Decision::Granted
        };
        mine.capabilities
            .entry(c.capability.name().to_string())
            .or_insert(decision);
    }
}

fn names_of(manifest: &Manifest) -> BTreeMap<String, String> {
    manifest
        .names
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().to_string()))
        .collect()
}

/// A package id this service keeps files under: the package format's own rule.
fn valid_id(id: &str) -> Result<(), InstallError> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id.split('.').count() >= 2
        && id.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
        });
    if ok {
        Ok(())
    } else {
        Err(InstallError::Package(oxyos_package::Error::InvalidId(
            id.to_string(),
        )))
    }
}

/// A Desktop Entry value: `\` escaped, as the specification asks.
fn entry_value(text: &str) -> String {
    text.replace('\\', "\\\\")
}

/// The entry of a version: its names, its icon and bundle in its directory, its module and its
/// runtime. Never `X-OxyOS-Grants`: system grants are the image's policy's alone.
pub fn entry_text(manifest: &Manifest, version_dir: &Path, runtime_abi: u32) -> String {
    let mut text = format!(
        "# Written by oxyos-installd for {} {} ({}); not to be edited.\n[Desktop Entry]\nType=Application\nName={}\n",
        manifest.id,
        manifest.version_code,
        entry_value(&manifest.version),
        entry_value(manifest.name.as_str())
    );
    for (language, name) in &manifest.names {
        text.push_str(&format!(
            "Name[{}]={}\n",
            language.replace('-', "_"),
            entry_value(name.as_str())
        ));
    }
    text.push_str(&format!(
        "Icon={}\nX-OxyOS-Bloom-Module={}\nX-OxyOS-Bloom-Bundle={}\nX-OxyOS-Runtime={runtime_abi}\nX-OxyOS-Package-Version={}\n",
        version_dir.join(manifest.icon.as_str()).display(),
        manifest.entry.module,
        version_dir.join("index.hbc").display(),
        manifest.version_code
    ));
    text
}

/// Flushes every directory of a tree, deepest first, and every file.
fn sync_tree(root: &Path) -> Result<(), InstallError> {
    for entry in fs::read_dir(root).map_err(io_at(root))?.flatten() {
        let path = entry.path();
        let kind = entry.file_type().map_err(io_at(&path))?;
        if kind.is_dir() {
            sync_tree(&path)?;
        } else if kind.is_file() {
            File::open(&path)
                .and_then(|f| f.sync_all())
                .map_err(io_at(&path))?;
        }
    }
    state::sync_directory(root).map_err(io_at(root))
}

/// A random hex token, for names no other commit takes.
fn random_token() -> String {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 8];
    let _ = ring::rand::SystemRandom::new().fill(&mut bytes);
    crate::tuf::hex_of(&bytes)
}
