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
//! - **Switching** is the installed record renamed over the old one, then the entry. The previous
//!   version is kept until the new one has shown a window; one that exits before its first
//!   window twice is rolled back and never tried again. A version a session holds is never
//!   removed, and the current one is not switched away from while held.
//! - **Consent:** an update that declares a capability shown at install that the current version
//!   does not, or that is signed by a rotated key, is staged until every account that has the
//!   application approves it.
//! - **Accounts:** the bytes are the machine's, each account's install its own record. Removing
//!   the last account's install removes the application; objects nothing names are collected.

use crate::accounts::{Account, Accounts};
use crate::compile::Compile;
use crate::fetch::{Fetch, FetchError};
use crate::state::{
    self, read_record, write_record, Consent, Decision, Installed, Layout, Staged, UserInstall,
    Version, FORMAT,
};
use crate::tuf::{self, TufError};
use oxyos_package::json::Value;
use oxyos_package::{
    lineage_accepts, Architecture, Capability, DeveloperSignature, Lineage, Manifest, PublicKey,
    ReleaseStatement, Runtime, Sha256, SignedRelease, HERMES_BYTECODE_MAGIC,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
    /// The runtimes in `directory`: each `<abi>/runtime.json` (`{"abi": N, "api": M}`, its `abi`
    /// its directory's name) beside its `hermesc`. One that cannot be read is left out.
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
                let Ok(value) = oxyos_package::json::parse(&bytes) else {
                    continue;
                };
                let number = |key| match member(&value, key) {
                    Some(Value::Integer(n)) => u32::try_from(*n).ok(),
                    _ => None,
                };
                if let (Some(a), Some(api)) = (number("abi"), number("api")) {
                    if a == abi && dir.join("hermesc").is_file() {
                        runtimes.push((Runtime { abi, api }, dir.join("hermesc")));
                    }
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
        });
    }
    found.sort_by(|a, b| b.statement.version_code.cmp(&a.statement.version_code));
    found
}

/// A target's `custom.oxyos`: the store's signed statement (verified with the targets role's
/// keys), the developer's signature, the developer's key and its lineage.
fn parse_custom(
    custom: &Value,
    store_keys: &[PublicKey],
) -> Result<(ReleaseStatement, DeveloperSignature, PublicKey, Lineage), InstallError> {
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
    Ok((statement, signature, developer_key, lineage))
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
    /// Capabilities shown at install the current version does not declare.
    pub new_capabilities: Vec<String>,
    /// Signed by a rotated key of the developer's lineage.
    pub key_rotation: bool,
    /// The machine already has a version of it.
    pub update: bool,
}

/// A plan with what it was made from, kept by the conversation until it is committed.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub plan: Plan,
    uid: u32,
    release: Release,
    manifest_bytes: Vec<u8>,
    manifest: Manifest,
    /// The key the machine pins once it is installed.
    pinned: PublicKey,
}

/// How a commit is going.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Progress {
    Fetching { done: u64, total: u64 },
    Compiling,
}

/// What a commit came to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Committed {
    pub id: String,
    pub version_code: u64,
    /// It is the current version now.
    pub switched: bool,
    /// What it waits for, if staged: other accounts' approval, or the current version to stop
    /// running (`None` with `switched` false).
    pub awaiting_consent: Option<Consent>,
}

/// An account's application, as `list` gives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Listed {
    pub id: String,
    pub version: String,
    pub version_code: u64,
    pub entry: String,
    pub capabilities: BTreeMap<String, Decision>,
    /// A newer version staged, and what it waits for.
    pub staged: Option<u64>,
    pub awaiting_consent: Option<Consent>,
}

/// What a check for updates did for an application.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    pub id: String,
    pub version_code: Option<u64>,
    /// `upToDate`, `switched`, `staged`, `awaitingConsent` or `failed`.
    pub outcome: String,
    pub reason: Option<String>,
}

/// What the service keeps in memory: the versions sessions hold.
#[derive(Default)]
struct Memory {
    holds: BTreeMap<(String, u64), usize>,
}

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
}

/// An account's data directories, relative to its home: what the session removes when the
/// account removes the application (the service never touches a home).
pub fn data_directories(id: &str) -> Vec<String> {
    vec![
        format!(".local/share/oxyos/apps/{id}"),
        format!(".cache/oxyos/apps/{id}"),
    ]
}

impl<F: Fetch, C: Compile, A: Accounts> Installer<F, C, A> {
    /// The installer over `layout`'s directory, trusting the store root at `anchor`. Recovers
    /// from a crash: what was being fetched or assembled is removed, entries are written again
    /// from the records, and what nothing names is collected.
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
        let path = self
            .layout
            .users()
            .join(&account.name)
            .join(format!("{id}.json"));
        read_record(&path).map_err(io_at(&path))
    }

    /// The accounts (uids) that have `id`.
    fn holders(&self, id: &str) -> Result<Vec<u32>, InstallError> {
        let mut uids = Vec::new();
        let users = self.layout.users();
        for entry in fs::read_dir(&users).map_err(io_at(&users))?.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let path = entry.path().join(format!("{id}.json"));
            if let Some(record) = read_record::<UserInstall>(&path).map_err(io_at(&path))? {
                uids.push(record.uid);
            }
        }
        Ok(uids)
    }

    fn client(&self) -> Result<tuf::Client, InstallError> {
        Ok(tuf::Client::open(&self.layout.store(), &self.anchor)?)
    }

    /// What installing (or updating) `id` for the account `uid` would do: the store's metadata
    /// refreshed, the newest release for this machine found, and its manifest fetched and
    /// verified. Nothing changes but the store's metadata.
    pub fn prepare(&self, uid: u32, id: &str) -> Result<Prepared, InstallError> {
        let account = self.account(uid)?;
        let _metadata = self.metadata.lock().unwrap_or_else(|p| p.into_inner());
        let installed = self.installed(id)?;
        let mut client = self.client()?;
        client.refresh(&self.fetch, (self.now)())?;
        let bad = installed
            .as_ref()
            .map(|i| i.bad.clone())
            .unwrap_or_default();
        let release = releases(&client, id, &self.machine, &bad)
            .into_iter()
            .next()
            .ok_or(InstallError::NoRelease)?;
        if let Some(installed) = &installed {
            if release.statement.version_code < installed.current.version_code {
                return Err(InstallError::Downgrade);
            }
        }
        let manifest_bytes = client.fetch_target(&self.fetch, &release.path)?;
        let (manifest, pinned) = self.verify(&release, &manifest_bytes, installed.as_ref())?;
        let key_rotation = installed
            .as_ref()
            .is_some_and(|i| i.pinned_key != pinned.encode());
        let current_capabilities: BTreeSet<String> = installed
            .as_ref()
            .map(|i| i.current.capabilities.iter().cloned().collect())
            .unwrap_or_default();
        let has_it = self.user_install(&account, id)?.is_some();
        let update = installed
            .as_ref()
            .is_some_and(|i| i.current.version_code != manifest.version_code);
        let new_capabilities = if installed.is_some() && has_it {
            manifest
                .capabilities
                .iter()
                .filter(|c| !c.capability.asked_at_first_use())
                .map(|c| c.capability.name().to_string())
                .filter(|name| !current_capabilities.contains(name))
                .collect()
        } else {
            Vec::new()
        };
        let mut download = 0;
        let mut seen = BTreeSet::new();
        for file in &manifest.files {
            if seen.insert(file.sha256.0) && !self.layout.object(&file.sha256).exists() {
                download += file.size;
            }
        }
        let plan = Plan {
            id: manifest.id.clone(),
            name: manifest.name.as_str().to_string(),
            names: manifest
                .names
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().to_string()))
                .collect(),
            version: manifest.version.clone(),
            version_code: manifest.version_code,
            runtime_abi: manifest.runtime.abi,
            download,
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
            key_rotation: key_rotation && has_it,
            update,
        };
        Ok(Prepared {
            plan,
            uid,
            release,
            manifest_bytes,
            manifest,
            pinned,
        })
    }

    /// The manifest of `release`, verified, and the key the machine pins with it.
    fn verify(
        &self,
        release: &Release,
        manifest_bytes: &[u8],
        installed: Option<&Installed>,
    ) -> Result<(Manifest, PublicKey), InstallError> {
        let manifest = Manifest::parse(manifest_bytes)?;
        release
            .statement
            .matches(&manifest, manifest_bytes, &release.signature)?;
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
        release.signature.verify(manifest_bytes, &pinned)?;
        if self.machine.runtime_for(&manifest.runtime).is_none() {
            return Err(InstallError::NoRelease);
        }
        Ok((manifest, pinned))
    }

    /// Installs or updates as `prepared` says, for its account: the person confirmed it in the
    /// system's sheet. `progress` is told how it goes.
    pub fn commit(
        &self,
        prepared: &Prepared,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Committed, InstallError> {
        let account = self.account(prepared.uid)?;
        let id = prepared.manifest.id.as_str();
        let code = prepared.manifest.version_code;
        // Is the version here already, or does it need assembling?
        let needs_version = {
            let _memory = self.memory();
            let installed = self.installed(id)?;
            if let Some(installed) = &installed {
                if code < installed.current.version_code {
                    return Err(InstallError::Downgrade);
                }
                if installed.bad.contains(&code) {
                    return Err(InstallError::RolledBack);
                }
            }
            !self.layout.version(id, code).is_dir()
        };
        let assembled = if needs_version {
            Some(self.assemble(prepared, progress)?)
        } else {
            None
        };
        let mut memory = self.memory();
        if let Some(assembled) = assembled {
            self.place(&assembled, id, code)?;
        }
        let installed = self.installed(id)?;
        let version = version_of(
            &prepared.manifest,
            &prepared.manifest_bytes,
            &prepared.release,
        );
        let committed = match installed {
            None => {
                let record = Installed {
                    format: FORMAT,
                    id: id.to_string(),
                    pinned_key: prepared.pinned.encode(),
                    current: version,
                    previous: None,
                    staged: None,
                    bad: Vec::new(),
                };
                self.write_installed(&record)?;
                self.write_entry(&record)?;
                Committed {
                    id: id.to_string(),
                    version_code: code,
                    switched: true,
                    awaiting_consent: None,
                }
            }
            Some(installed) if installed.current.version_code == code => Committed {
                id: id.to_string(),
                version_code: code,
                switched: true,
                awaiting_consent: None,
            },
            Some(mut installed) => {
                if code < installed.current.version_code {
                    return Err(InstallError::Downgrade);
                }
                let staged = match installed.staged.take() {
                    Some(staged) if staged.version.version_code == code => staged,
                    _ => Staged {
                        consent: consent_needed(&installed, &version, &prepared.pinned),
                        version,
                        pinned_key: prepared.pinned.encode(),
                    },
                };
                installed.staged = Some(staged);
                self.approve(&mut installed, Some(prepared.uid))?;
                let switched = self.try_switch(&mut installed, &memory)?;
                let awaiting_consent = installed.staged.as_ref().and_then(|s| s.consent.clone());
                self.write_installed(&installed)?;
                if switched {
                    self.write_entry(&installed)?;
                }
                Committed {
                    id: id.to_string(),
                    version_code: code,
                    switched,
                    awaiting_consent,
                }
            }
        };
        // The account's install, and its decisions: what is shown at install is granted by
        // the sheet, what is asked at first use waits for its first use.
        let mut user = self.user_install(&account, id)?.unwrap_or(UserInstall {
            format: FORMAT,
            id: id.to_string(),
            uid: account.uid,
            user: account.name.clone(),
            capabilities: BTreeMap::new(),
        });
        for c in &prepared.manifest.capabilities {
            let decision = if c.capability.asked_at_first_use() {
                Decision::Undecided
            } else {
                Decision::Granted
            };
            user.capabilities
                .entry(c.capability.name().to_string())
                .or_insert(decision);
        }
        self.write_user(&account, &user)?;
        self.collect(&mut memory)?;
        Ok(committed)
    }

    /// Records the account `uid`'s approval of the staged version, if one; it no longer waits
    /// for consent once every account that has the application approved.
    fn approve(&self, installed: &mut Installed, uid: Option<u32>) -> Result<(), InstallError> {
        let holders = self.holders(&installed.id)?;
        if let Some(staged) = installed.staged.as_mut() {
            if let Some(consent) = staged.consent.as_mut() {
                if let Some(uid) = uid.filter(|uid| !consent.approved.contains(uid)) {
                    consent.approved.push(uid);
                    consent.approved.sort_unstable();
                }
                if holders.iter().all(|h| consent.approved.contains(h)) {
                    staged.consent = None;
                }
            }
        }
        Ok(())
    }

    /// Switches to the staged version if it waits for no consent and no session holds the
    /// current one. The previous version is the current one until the new one shows a window.
    fn try_switch(&self, installed: &mut Installed, memory: &Memory) -> Result<bool, InstallError> {
        let ready = installed
            .staged
            .as_ref()
            .is_some_and(|s| s.consent.is_none());
        let held = memory
            .holds
            .get(&(installed.id.clone(), installed.current.version_code))
            .is_some_and(|n| *n > 0);
        if !ready || held {
            return Ok(false);
        }
        let Some(staged) = installed.staged.take() else {
            return Ok(false);
        };
        let new_capabilities: Vec<String> = staged
            .version
            .capabilities
            .iter()
            .filter(|c| !installed.current.capabilities.contains(c))
            .cloned()
            .collect();
        installed.previous = Some(installed.current.clone());
        installed.current = staged.version;
        installed.pinned_key = staged.pinned_key;
        // Every account's decisions gain the new capabilities: granted if shown at install
        // (every account approved them), undecided if asked at first use.
        for uid in self.holders(&installed.id)? {
            let Some(account) = self.accounts.account(uid) else {
                continue;
            };
            if let Some(mut user) = self.user_install(&account, &installed.id)? {
                for name in &new_capabilities {
                    let first_use = Capability::parse(name)
                        .map(|c| c.asked_at_first_use())
                        .unwrap_or(true);
                    user.capabilities
                        .entry(name.clone())
                        .or_insert(if first_use {
                            Decision::Undecided
                        } else {
                            Decision::Granted
                        });
                }
                self.write_user(&account, &user)?;
            }
        }
        Ok(true)
    }

    /// Fetches what the machine lacks, assembles and compiles the version in a hidden directory
    /// of its own and checks its tree: that directory. Without the state's lock: objects are
    /// linked into the version before they are named in `objects/`, so a collection never takes
    /// one it needs, and the collection never touches a hidden directory. [`Self::place`] renames
    /// it into place under the lock, with the record that names it.
    fn assemble(
        &self,
        prepared: &Prepared,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<PathBuf, InstallError> {
        let manifest = &prepared.manifest;
        let package_dir = self.layout.packages().join(&manifest.id);
        state::ensure_directory(&package_dir, 0o755).map_err(io_at(&package_dir))?;
        let assembling =
            package_dir.join(format!(".{}.new-{}", manifest.version_code, random_token()));
        state::ensure_directory(&assembling, 0o755).map_err(io_at(&assembling))?;
        let result = self.assemble_into(prepared, &assembling, progress);
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
        prepared: &Prepared,
        assembling: &Path,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<(), InstallError> {
        let manifest = &prepared.manifest;
        let total: u64 = prepared.plan.download;
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
            &prepared.manifest_bytes,
            0o444,
            None,
        )?;
        state::write_file(
            assembling,
            "manifest.sig",
            &prepared.release.signature.to_canonical(),
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

    fn write_user(&self, account: &Account, user: &UserInstall) -> Result<(), InstallError> {
        let dir = self.layout.users().join(&account.name);
        state::ensure_directory(&dir, 0o750).map_err(io_at(&dir))?;
        let owner = self.chown.then_some((0, account.gid));
        if let Some((uid, gid)) = owner {
            rustix::fs::chown(
                &dir,
                Some(rustix::fs::Uid::from_raw(uid)),
                Some(rustix::fs::Gid::from_raw(gid)),
            )
            .map_err(|e| InstallError::Io(format!("{}: {e}", dir.display())))?;
        }
        write_record(&dir, &format!("{}.json", user.id), user, 0o640, owner).map_err(io_at(&dir))
    }

    /// The entry of the current version (`applications/<id>.desktop`): what the registry lists
    /// for the accounts that have the application. It asks for no grant: no entry can.
    fn write_entry(&self, installed: &Installed) -> Result<(), InstallError> {
        let version_dir = self
            .layout
            .version(&installed.id, installed.current.version_code);
        let manifest_path = version_dir.join("manifest.json");
        let bytes = fs::read(&manifest_path).map_err(io_at(&manifest_path))?;
        let manifest = Manifest::parse(&bytes)?;
        let text = entry_text(&manifest, &version_dir, installed.current.runtime_abi);
        let dir = self.layout.applications();
        state::write_file(
            &dir,
            &format!("{}.desktop", installed.id),
            text.as_bytes(),
            0o644,
            None,
        )
        .map_err(io_at(&dir))
    }

    /// Removes the account's install of `id`: the data directories the session should remove
    /// (relative to the home). The application goes once no account has it.
    pub fn uninstall(&self, uid: u32, id: &str) -> Result<Vec<String>, InstallError> {
        let account = self.account(uid)?;
        valid_id(id)?;
        let mut memory = self.memory();
        let dir = self.layout.users().join(&account.name);
        let path = dir.join(format!("{id}.json"));
        match fs::remove_file(&path) {
            Ok(()) => state::sync_directory(&dir).map_err(io_at(&dir))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(InstallError::NotInstalled)
            }
            Err(e) => return Err(io_at(&path)(e)),
        }
        // A staged version waiting for this account's approval waits for one account fewer.
        if let Some(mut installed) = self.installed(id)? {
            if installed.staged.is_some() && !self.holders(id)?.is_empty() {
                self.approve(&mut installed, None)?;
                let switched = self.try_switch(&mut installed, &memory)?;
                self.write_installed(&installed)?;
                if switched {
                    self.write_entry(&installed)?;
                }
            }
        }
        self.collect(&mut memory)?;
        Ok(data_directories(id))
    }

    /// The account's applications.
    pub fn list(&self, uid: u32) -> Result<Vec<Listed>, InstallError> {
        let account = self.account(uid)?;
        let _memory = self.memory();
        let dir = self.layout.users().join(&account.name);
        let mut listed = Vec::new();
        let Ok(entries) = fs::read_dir(&dir) else {
            return Ok(listed);
        };
        let mut names: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        names.sort();
        for path in names {
            let Some(id) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".json"))
            else {
                continue;
            };
            if id.starts_with('.') {
                continue;
            }
            let (Some(user), Some(installed)) = (
                read_record::<UserInstall>(&path).map_err(io_at(&path))?,
                self.installed(id)?,
            ) else {
                continue;
            };
            listed.push(Listed {
                id: id.to_string(),
                version: installed.current.version.clone(),
                version_code: installed.current.version_code,
                entry: self
                    .layout
                    .applications()
                    .join(format!("{id}.desktop"))
                    .display()
                    .to_string(),
                capabilities: user.capabilities,
                staged: installed.staged.as_ref().map(|s| s.version.version_code),
                awaiting_consent: installed.staged.and_then(|s| s.consent),
            });
        }
        Ok(listed)
    }

    /// Whether the account `uid` has `id` (root has every application).
    pub fn has(&self, uid: u32, id: &str) -> bool {
        if uid == 0 {
            return true;
        }
        self.accounts
            .account(uid)
            .and_then(|account| self.user_install(&account, id).ok().flatten())
            .is_some()
    }

    /// A session runs version `code` of `id`: it is never removed, and the current version is not
    /// switched away from, until it is released.
    pub fn hold(&self, id: &str, code: u64) -> Result<(), InstallError> {
        valid_id(id)?;
        let mut memory = self.memory();
        if !self.layout.version(id, code).is_dir() {
            return Err(InstallError::NotInstalled);
        }
        *memory.holds.entry((id.to_string(), code)).or_insert(0) += 1;
        Ok(())
    }

    /// A session no longer runs version `code` of `id`. A staged version waiting only for it
    /// is switched to.
    pub fn release(&self, id: &str, code: u64) -> Result<(), InstallError> {
        valid_id(id)?;
        let mut memory = self.memory();
        let key = (id.to_string(), code);
        match memory.holds.get_mut(&key) {
            Some(n) if *n > 1 => *n -= 1,
            Some(_) => {
                memory.holds.remove(&key);
            }
            None => return Err(InstallError::NotInstalled),
        }
        if let Some(mut installed) = self.installed(id)? {
            if self.try_switch(&mut installed, &memory)? {
                self.write_installed(&installed)?;
                self.write_entry(&installed)?;
            }
        }
        self.collect(&mut memory)
    }

    /// A session showed a window of version `code` of `id`: the version before it is no longer
    /// kept for a rollback.
    pub fn shown(&self, id: &str, code: u64) -> Result<(), InstallError> {
        let mut memory = self.memory();
        let Some(mut installed) = self.installed(id)? else {
            return Err(InstallError::NotInstalled);
        };
        if installed.current.version_code == code && !installed.current.shown {
            installed.current.shown = true;
            installed.previous = None;
            self.write_installed(&installed)?;
            self.collect(&mut memory)?;
        }
        Ok(())
    }

    /// A launch of version `code` of `id` exited, having shown a window or not. A new version
    /// that exits before its first window [`UNSHOWN_EXITS`] times is rolled back to the version
    /// before and never tried again here: `true` if it was.
    pub fn exited(&self, id: &str, code: u64, shown: bool) -> Result<bool, InstallError> {
        if shown {
            self.shown(id, code)?;
            return Ok(false);
        }
        let mut memory = self.memory();
        let Some(mut installed) = self.installed(id)? else {
            return Err(InstallError::NotInstalled);
        };
        if installed.current.version_code != code || installed.current.shown {
            return Ok(false);
        }
        installed.current.unshown_exits += 1;
        let rolled_back =
            installed.current.unshown_exits >= UNSHOWN_EXITS && installed.previous.is_some();
        if rolled_back {
            if let Some(previous) = installed.previous.take() {
                installed.bad.push(code);
                installed.current = previous;
            }
        }
        self.write_installed(&installed)?;
        if rolled_back {
            self.write_entry(&installed)?;
            self.collect(&mut memory)?;
        }
        Ok(rolled_back)
    }

    /// Looks for a newer release of every application on the machine: one that needs no consent
    /// is assembled and switched to (or staged while its current version runs), one that needs
    /// it is assembled and staged until every account approves.
    pub fn check_updates(&self) -> Result<Vec<Update>, InstallError> {
        let ids: Vec<String> = {
            let _memory = self.memory();
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
            ids
        };
        let mut updates = Vec::new();
        for id in ids {
            updates.push(match self.update_one(&id) {
                Ok(update) => update,
                Err(e) => Update {
                    id: id.clone(),
                    version_code: None,
                    outcome: "failed".into(),
                    reason: Some(e.to_string()),
                },
            });
        }
        Ok(updates)
    }

    fn update_one(&self, id: &str) -> Result<Update, InstallError> {
        let Some(installed) = self.installed(id)? else {
            return Err(InstallError::NotInstalled);
        };
        let Some(&uid) = self.holders(id)?.first() else {
            return Err(InstallError::NotInstalled);
        };
        let up_to_date = Update {
            id: id.to_string(),
            version_code: Some(installed.current.version_code),
            outcome: "upToDate".into(),
            reason: None,
        };
        let prepared = match self.prepare(uid, id) {
            Err(InstallError::NoRelease) => return Ok(up_to_date),
            other => other?,
        };
        let code = prepared.plan.version_code;
        let already_staged = installed
            .staged
            .as_ref()
            .is_some_and(|s| s.version.version_code == code);
        if code == installed.current.version_code || already_staged {
            return Ok(Update {
                outcome: if already_staged { "staged" } else { "upToDate" }.into(),
                version_code: Some(code),
                ..up_to_date
            });
        }
        let assembled = if self.layout.version(id, code).is_dir() {
            None
        } else {
            Some(self.assemble(&prepared, &mut |_| {})?)
        };
        let mut memory = self.memory();
        if let Some(assembled) = assembled {
            self.place(&assembled, id, code)?;
        }
        let Some(mut installed) = self.installed(id)? else {
            return Err(InstallError::Stale);
        };
        if code <= installed.current.version_code {
            return Err(InstallError::Stale);
        }
        let version = version_of(
            &prepared.manifest,
            &prepared.manifest_bytes,
            &prepared.release,
        );
        let consent = consent_needed(&installed, &version, &prepared.pinned);
        installed.staged = Some(Staged {
            consent,
            version,
            pinned_key: prepared.pinned.encode(),
        });
        let switched = self.try_switch(&mut installed, &memory)?;
        let awaiting = installed.staged.as_ref().and_then(|s| s.consent.clone());
        self.write_installed(&installed)?;
        if switched {
            self.write_entry(&installed)?;
        }
        self.collect(&mut memory)?;
        Ok(Update {
            id: id.to_string(),
            version_code: Some(code),
            outcome: if switched {
                "switched"
            } else if awaiting.is_some() {
                "awaitingConsent"
            } else {
                "staged"
            }
            .into(),
            reason: None,
        })
    }

    /// After a crash: what was being fetched or assembled goes, entries are written again from
    /// the records, and what nothing names is collected.
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
        for dir in [self.layout.installed(), self.layout.applications()] {
            for entry in fs::read_dir(&dir).map_err(io_at(&dir))?.flatten() {
                if entry.file_name().to_string_lossy().starts_with('.') {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        let installed_dir = self.layout.installed();
        for entry in fs::read_dir(&installed_dir)
            .map_err(io_at(&installed_dir))?
            .flatten()
        {
            let path = entry.path();
            if let Some(installed) = read_record::<Installed>(&path).map_err(io_at(&path))? {
                // An entry is listed only for a version that is there to run.
                if self.write_entry(&installed).is_err() {
                    let _ = fs::remove_file(
                        self.layout
                            .applications()
                            .join(format!("{}.desktop", installed.id)),
                    );
                }
            }
        }
        self.collect(&mut memory)
    }

    /// Removes what nothing names: an application no account has (its record, entry and the
    /// versions no session holds), versions neither current, previous nor staged that no session
    /// holds, and objects no version links (their link count is one).
    fn collect(&self, memory: &mut Memory) -> Result<(), InstallError> {
        let packages = self.layout.packages();
        for package in fs::read_dir(&packages).map_err(io_at(&packages))?.flatten() {
            let Some(id) = package.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let installed = self.installed(&id).ok().flatten();
            let wanted = !self.holders(&id)?.is_empty();
            let keep: BTreeSet<u64> = match (&installed, wanted) {
                (Some(i), true) => std::iter::once(i.current.version_code)
                    .chain(i.previous.as_ref().map(|p| p.version_code))
                    .chain(i.staged.as_ref().map(|s| s.version.version_code))
                    .collect(),
                _ => BTreeSet::new(),
            };
            if !wanted && installed.is_some() {
                for (dir, name) in [
                    (self.layout.installed(), format!("{id}.json")),
                    (self.layout.applications(), format!("{id}.desktop")),
                ] {
                    match fs::remove_file(dir.join(&name)) {
                        Err(e) if e.kind() != io::ErrorKind::NotFound => {
                            return Err(io_at(&dir)(e))
                        }
                        _ => state::sync_directory(&dir).map_err(io_at(&dir))?,
                    }
                }
            }
            let mut left = 0;
            for version in fs::read_dir(package.path()).into_iter().flatten().flatten() {
                let name = version.file_name();
                let Some(code) = name.to_str().and_then(|n| n.parse::<u64>().ok()) else {
                    left += 1;
                    continue;
                };
                let held = memory.holds.contains_key(&(id.clone(), code));
                if keep.contains(&code) || held {
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

/// What consent an update needs from the accounts that have the application.
fn consent_needed(installed: &Installed, version: &Version, pinned: &PublicKey) -> Option<Consent> {
    let capabilities: Vec<String> = version
        .capabilities
        .iter()
        .filter(|name| {
            !installed.current.capabilities.contains(name)
                && Capability::parse(name).is_ok_and(|c| !c.asked_at_first_use())
        })
        .cloned()
        .collect();
    let key_rotation = installed.pinned_key != pinned.encode();
    (key_rotation || !capabilities.is_empty()).then_some(Consent {
        capabilities,
        key_rotation,
        approved: Vec::new(),
    })
}

fn version_of(manifest: &Manifest, bytes: &[u8], release: &Release) -> Version {
    Version {
        version_code: manifest.version_code,
        version: manifest.version.clone(),
        runtime_abi: manifest.runtime.abi,
        runtime_api: manifest.runtime.api,
        manifest_sha256: Sha256::of(bytes).hex(),
        developer_key_id: release.signature.key_id.as_str().to_string(),
        capabilities: manifest
            .capabilities
            .iter()
            .map(|c| c.capability.name().to_string())
            .collect(),
        shown: false,
        unshown_exits: 0,
    }
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
