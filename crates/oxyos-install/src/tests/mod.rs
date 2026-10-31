//! The install service's logic over directories of the tests' own: a repository laid out as the
//! store serves it (TUF metadata signed with keys of fixed seeds, targets, objects), read through
//! the real fetcher over a socket, a machine's runtimes, and accounts given by the tests.

use crate::accounts::{Account, Fixed};
use crate::compile::Compile;
use crate::fetch::{Counting, Fetch, FetcherClient, LocalRepository, ThreadFetcher};
use crate::install::{InstallError, Installer, Machine};
use crate::state::{read_record, Decision, Installed, Layout, UserInstall};
use crate::tuf::{self, write, TufError};
use oxyos_package::json::{self, Value};
use oxyos_package::{
    Architecture, DeveloperSignature, Lineage, Manifest, ReleaseStatement, Sha256, SigningKey,
    HERMES_BYTECODE_MAGIC,
};
use std::fs::{self, File};
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

mod flows;
mod ipc;
mod listings;
mod refusals;
mod sandbox;
mod tuf_client;

/// 2026-10-30T00:00:00Z, the tests' now.
pub const NOW: i64 = 1_793_318_400;
/// A year on, when what expires within the year has.
pub const LATER: i64 = NOW + 400 * 86_400;
pub fn now() -> i64 {
    NOW
}

/// The tests' accounts: uids no real user has (a CI runner's own uid is 1001), so a request
/// "for another account" is never the test process's own.
pub const ALICE: u32 = 2_000_001;
pub const BOB: u32 = 2_000_002;
pub const APPLICATION: &str = "0192f1e0-7c3a-7d2e-8a41-5b6c7d8e9f01";

pub fn key(seed: u8) -> SigningKey {
    SigningKey::from_seed(&[seed; 32])
}

/// The store's keys: root (three, two to sign), targets, snapshot, timestamp.
pub const ROOT_KEYS: [u8; 3] = [1, 2, 3];
pub const TARGETS_KEY: u8 = 4;
pub const SNAPSHOT_KEY: u8 = 5;
pub const TIMESTAMP_KEY: u8 = 6;
/// The developer's keys.
pub const DEVELOPER: u8 = 7;
pub const DEVELOPER_NEXT: u8 = 8;
/// A key nobody trusts.
pub const STRANGER: u8 = 9;

/// A directory of the test's own, removed when it ends.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("oxyos-install-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A package the tests publish.
#[derive(Clone)]
pub struct Pkg {
    pub id: String,
    pub code: u64,
    pub files: Vec<(String, Vec<u8>)>,
    /// Capabilities by name, each with an English purpose.
    pub capabilities: Vec<&'static str>,
    pub runtime: (u32, u32),
    /// Who signs the manifest.
    pub signer: u8,
    /// The key the store names as the developer's.
    pub published_key: u8,
    /// Who signs the store's statement.
    pub store: u8,
    pub lineage: Lineage,
    /// Release notes by language, the store's.
    pub notes: Vec<(&'static str, &'static str)>,
    /// The store listing it belongs to, the store's.
    pub listing: Option<&'static str>,
}

impl Pkg {
    pub fn notes(code: u64) -> Self {
        Pkg {
            id: "so.oxy.notes".into(),
            code,
            files: vec![
                (
                    "bundle/drawable-mdpi/logo.png".into(),
                    b"\x89PNG logo".to_vec(),
                ),
                (
                    "bundle/index.js".into(),
                    format!("AppRegistry.registerComponent('Notes', () => App); // {code}")
                        .into_bytes(),
                ),
                ("icon.svg".into(), b"<svg/>".to_vec()),
            ],
            capabilities: vec!["network", "notifications"],
            runtime: (1, 3),
            signer: DEVELOPER,
            published_key: DEVELOPER,
            store: TARGETS_KEY,
            lineage: Lineage::default(),
            notes: vec![("en", "Faster."), ("es", "Más rápida.")],
            listing: Some("notes"),
        }
    }

    pub fn file(mut self, path: &str, bytes: &[u8]) -> Self {
        self.files.retain(|(p, _)| p != path);
        self.files.push((path.into(), bytes.to_vec()));
        self
    }

    pub fn manifest(&self) -> Vec<u8> {
        let mut files = self.files.clone();
        files.sort();
        let files = files
            .iter()
            .map(|(path, bytes)| {
                json::object(vec![
                    ("path", json::string(path)),
                    ("size", Value::Integer(bytes.len() as u64)),
                    ("sha256", json::string(&Sha256::of(bytes).hex())),
                ])
            })
            .collect();
        let mut capabilities = self.capabilities.clone();
        capabilities.sort();
        let capabilities = capabilities
            .iter()
            .map(|name| {
                json::object(vec![
                    ("name", json::string(name)),
                    (
                        "purpose",
                        json::object(vec![("en", json::string("To work."))]),
                    ),
                ])
            })
            .collect();
        json::canonical(&json::object(vec![
            ("id", json::string(&self.id)),
            ("application", json::string(APPLICATION)),
            ("version", json::string(&format!("1.{}.0", self.code))),
            ("versionCode", Value::Integer(self.code)),
            ("name", json::string("Notes")),
            ("names", json::object(vec![("es", json::string("Notas"))])),
            ("icon", json::string("icon.svg")),
            (
                "entry",
                json::object(vec![
                    ("bundle", json::string("bundle/index.js")),
                    ("module", json::string("Notes")),
                ]),
            ),
            (
                "runtime",
                json::object(vec![
                    ("abi", Value::Integer(self.runtime.0.into())),
                    ("api", Value::Integer(self.runtime.1.into())),
                ]),
            ),
            ("architectures", Value::Array(vec![json::string("any")])),
            ("capabilities", Value::Array(capabilities)),
            ("files", Value::Array(files)),
        ]))
    }

    pub fn target_path(&self) -> String {
        format!("{}/{}/any/manifest.json", self.id, self.code)
    }

    /// The target's custom data: the store's statement, the developer's signature, key and
    /// lineage, each as the text signed.
    pub fn custom(&self, manifest: &[u8]) -> Value {
        let signature: DeveloperSignature = key(self.signer).sign_manifest(manifest);
        let statement = match Manifest::parse(manifest) {
            Ok(parsed) => ReleaseStatement::of(&parsed, manifest, Architecture::Any, &signature),
            // A manifest the package format refuses still gets a statement, as a store that
            // erred would sign one: the install service must refuse it itself.
            Err(_) => ReleaseStatement {
                package_id: self.id.clone(),
                version_code: self.code,
                runtime: oxyos_package::Runtime {
                    abi: self.runtime.0,
                    api: self.runtime.1,
                },
                architecture: Architecture::Any,
                manifest_sha256: Sha256::of(manifest),
                manifest_length: manifest.len() as u64,
                developer_key_id: signature.key_id.clone(),
            },
        };
        crate::repository::release_custom(
            statement,
            &signature,
            &key(self.published_key).public_key(),
            &key(self.store),
            &self.lineage,
            &self.notes,
            self.listing,
        )
    }
}

/// A repository as the store serves it: `crate::repository`'s, with the tests' keys of fixed seeds.
pub type Repo = crate::repository::Repository;

pub use crate::repository::{TARGETS, TIMESTAMP};

/// The tests' ways with a repository, over `crate::repository`, keys by their seeds.
pub trait TestRepo {
    fn new(dir: &Path) -> Self;
    fn root_value(version: u64, root_keys: &[u8], timestamp_key: u8) -> Value;
    fn write_root(&self, version: u64, root_keys: &[u8], signers: &[u8], timestamp_key: u8);
    fn add(&mut self, pkg: &Pkg);
    fn remove(&mut self, pkg: &Pkg);
    fn publish(&mut self);
    fn publish_with(&mut self, timestamp_key: u8);
    fn fetcher(&self, scratch: &Path) -> ThreadFetcher;
}

impl TestRepo for Repo {
    fn new(dir: &Path) -> Self {
        for sub in ["metadata", "targets", "objects"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let repo = Repo {
            dir: dir.to_path_buf(),
            targets: Vec::new(),
            versions: [0; 3],
            expires: [NOW + 300 * 86_400; 3],
        };
        repo.write_root(1, &ROOT_KEYS, &[1, 2], TIMESTAMP_KEY);
        repo
    }

    fn root_value(version: u64, root_keys: &[u8], timestamp_key: u8) -> Value {
        let root: Vec<oxyos_package::PublicKey> =
            root_keys.iter().map(|s| key(*s).public_key()).collect();
        crate::repository::root_document(
            version,
            &root,
            2,
            &key(TARGETS_KEY).public_key(),
            &key(SNAPSHOT_KEY).public_key(),
            &key(timestamp_key).public_key(),
            NOW + 3 * 365 * 86_400,
        )
    }

    /// Writes root `version`, signed by `signers`.
    fn write_root(&self, version: u64, root_keys: &[u8], signers: &[u8], timestamp_key: u8) {
        let keys: Vec<SigningKey> = signers.iter().map(|s| key(*s)).collect();
        let refs: Vec<&SigningKey> = keys.iter().collect();
        crate::repository::write_root(
            &self.dir,
            version,
            Self::root_value(version, root_keys, timestamp_key),
            &refs,
        )
        .unwrap();
    }

    /// Adds a package's release: its objects, its manifest as a target, its custom data.
    fn add(&mut self, pkg: &Pkg) {
        let manifest = pkg.manifest();
        let files: Vec<&[u8]> = pkg
            .files
            .iter()
            .map(|(_, bytes)| bytes.as_slice())
            .collect();
        self.add_release(&pkg.target_path(), &manifest, &files, pkg.custom(&manifest))
            .unwrap();
    }

    fn remove(&mut self, pkg: &Pkg) {
        self.remove_release(&pkg.target_path());
    }

    /// Publishes the targets: a new targets, snapshot and timestamp, each its next version.
    fn publish(&mut self) {
        self.publish_with(TIMESTAMP_KEY);
    }

    fn publish_with(&mut self, timestamp_key: u8) {
        self.publish_metadata(&key(TARGETS_KEY), &key(SNAPSHOT_KEY), &key(timestamp_key))
            .unwrap();
    }

    /// The fetcher over this repository as the store lays it out: TUF's documents are asked for
    /// by name, so the fetcher's root is the repository and metadata paths are under `metadata/`.
    fn fetcher(&self, scratch: &Path) -> ThreadFetcher {
        ThreadFetcher::start(scratch, LocalRepository::new(&self.dir)).unwrap()
    }
}

/// A compiler that writes Hermes's magic and the source after it: the tests' stand-in for
/// `hermesc`, whose real run is its own test.
#[derive(Clone, Copy)]
pub enum FakeCompiler {
    Works,
    Fails,
    WritesText,
}

impl Compile for FakeCompiler {
    fn compile(&self, _hermesc: &Path, source: &File, out: &File) -> Result<(), String> {
        let mut text = Vec::new();
        let mut source = source;
        source.read_to_end(&mut text).map_err(|e| e.to_string())?;
        let mut out = out;
        match self {
            FakeCompiler::Works => {
                out.write_all(&HERMES_BYTECODE_MAGIC).unwrap();
                out.write_all(&text).unwrap();
                Ok(())
            }
            FakeCompiler::Fails => Err("syntax error".into()),
            FakeCompiler::WritesText => {
                out.write_all(b"not bytecode").unwrap();
                Ok(())
            }
        }
    }
}

/// A machine with runtime 1.3 (and its compiler at `hermesc`).
pub fn machine(dir: &Path, hermesc: Option<&Path>) -> Machine {
    let runtime = dir.join("runtimes/1");
    fs::create_dir_all(&runtime).unwrap();
    fs::write(runtime.join("runtime.json"), br#"{"abi":1,"api":3}"#).unwrap();
    match hermesc {
        Some(real) => {
            let _ = fs::remove_file(runtime.join("hermesc"));
            std::os::unix::fs::symlink(real, runtime.join("hermesc")).unwrap();
        }
        None => fs::write(runtime.join("hermesc"), b"").unwrap(),
    }
    Machine::discover(&dir.join("runtimes"), Architecture::X86_64)
}

pub fn accounts() -> Fixed {
    let me = rustix::process::getuid().as_raw();
    assert!(
        me != ALICE && me != BOB,
        "the tests run as one of their own accounts' uids"
    );
    Fixed(vec![
        Account {
            uid: ALICE,
            name: "alice".into(),
            gid: ALICE,
        },
        Account {
            uid: BOB,
            name: "bob".into(),
            gid: BOB,
        },
        Account {
            uid: rustix::process::getuid().as_raw(),
            name: "me".into(),
            gid: rustix::process::getgid().as_raw(),
        },
    ])
}

/// Everything a test needs: a repository, its fetcher, and an installer over a state directory.
pub struct World {
    pub scratch: Scratch,
    pub repo: Repo,
    pub fetcher: ThreadFetcher,
    pub state: PathBuf,
}

pub type TestInstaller<F> = Installer<F, FakeCompiler, Fixed>;

impl World {
    pub fn new(name: &str) -> Self {
        let scratch = Scratch::new(name);
        let repo = Repo::new(&scratch.0.join("repository"));
        let fetcher = repo.fetcher(&scratch.0);
        let state = scratch.0.join("state");
        World {
            scratch,
            repo,
            fetcher,
            state,
        }
    }

    pub fn installer_with<F: Fetch>(
        &self,
        fetch: F,
        compiler: FakeCompiler,
        now: fn() -> i64,
    ) -> Result<TestInstaller<F>, InstallError> {
        Installer::open(
            Layout::new(&self.state),
            &self.repo.anchor(),
            fetch,
            compiler,
            accounts(),
            machine(&self.scratch.0, None),
            now,
        )
    }

    pub fn installer(&self) -> TestInstaller<FetcherClient> {
        self.installer_with(self.fetcher.client(), FakeCompiler::Works, now)
            .unwrap()
    }

    pub fn counting(&self) -> TestInstaller<Counting<FetcherClient>> {
        self.installer_with(
            Counting::new(self.fetcher.client()),
            FakeCompiler::Works,
            now,
        )
        .unwrap()
    }

    pub fn layout(&self) -> Layout {
        Layout::new(&self.state)
    }

    pub fn installed(&self, id: &str) -> Option<Installed> {
        read_record(&self.layout().installed().join(format!("{id}.json"))).unwrap()
    }

    pub fn user(&self, user: &str, id: &str) -> Option<UserInstall> {
        read_record(&self.layout().user(user).join(format!("{id}.json"))).unwrap()
    }

    /// The version the account `user` runs.
    pub fn current(&self, user: &str, id: &str) -> Option<u64> {
        self.user(user, id).map(|u| u.current)
    }

    /// The account's entry.
    pub fn entry(&self, user: &str, id: &str) -> String {
        read(
            &self
                .layout()
                .user_applications(user)
                .join(format!("{id}.desktop")),
        )
    }

    /// The files under the state directory but the store's metadata, sorted: what a refusal
    /// must leave as it found.
    pub fn files(&self) -> Vec<String> {
        let mut out = Vec::new();
        walk(&self.state, &self.state, &mut out);
        out.retain(|p| !p.starts_with("store/"));
        out.sort();
        out
    }

    /// The objects on the machine.
    pub fn objects(&self) -> Vec<String> {
        self.files()
            .into_iter()
            .filter(|p| p.starts_with("objects/"))
            .collect()
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            walk(root, &path, out);
        } else {
            out.push(path.strip_prefix(root).unwrap().display().to_string());
        }
    }
}

/// Installs `id` for `uid`, the person confirming the sheet.
pub fn install<F: Fetch>(
    installer: &TestInstaller<F>,
    uid: u32,
    id: &str,
) -> Result<crate::install::Committed, InstallError> {
    let prepared = installer.prepare(uid, id)?;
    installer.commit(&prepared, &mut |_| {})
}

pub fn decision(world: &World, user: &str, capability: &str) -> Option<Decision> {
    world
        .user(user, "so.oxy.notes")
        .and_then(|u| u.capabilities.get(capability).copied())
}

pub fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

pub use tuf::Client;
