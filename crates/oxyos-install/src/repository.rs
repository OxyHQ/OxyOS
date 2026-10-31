//! Writing a store repository (docs/adr/0030-installing-applications.mdx, decisions 3 and 4): what
//! the store publishes and the install service reads, laid out as TUF's consistent snapshots and
//! the store's objects. For development and tests (`oxyos-pack publish`, the install service's
//! tests): the store's own publishing is oxy-api's (Oxy ADR 0053), and its keys are never on a
//! developer's machine.
//!
//! A repository directory holds `metadata/` (`N.root.json`, `N.targets.json`, `N.snapshot.json`,
//! `timestamp.json`), `targets/` (each release's manifest, by its hash) and `objects/` (every
//! file, by its SHA-256). A release is a target whose custom data carries the store's statement,
//! the developer's signature, key and lineage, and the store's release notes.

use crate::tuf::write;
use oxyos_package::json::{self, Value};
use oxyos_package::{
    Architecture, DeveloperSignature, Lineage, Manifest, PublicKey, ReleaseStatement, Sha256,
    SigningKey,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const TARGETS: usize = 0;
pub const SNAPSHOT: usize = 1;
pub const TIMESTAMP: usize = 2;

/// The specification the documents follow.
const SPEC_VERSION: &str = "1.0.31";

/// The store's keys: the root's (a threshold of them signs a root) and each online role's.
pub struct StoreKeys {
    pub root: Vec<SigningKey>,
    pub root_threshold: u64,
    pub targets: SigningKey,
    pub snapshot: SigningKey,
    pub timestamp: SigningKey,
}

/// The root document `version`: the roles' public keys and thresholds, expiring at `expires`.
pub fn root_document(
    version: u64,
    root: &[PublicKey],
    root_threshold: u64,
    targets: &PublicKey,
    snapshot: &PublicKey,
    timestamp: &PublicKey,
    expires: i64,
) -> Value {
    let mut keys = Vec::new();
    let mut ids = |publics: &[&PublicKey]| -> Vec<Value> {
        publics
            .iter()
            .map(|public| {
                let (id, value) = write::key(&public.0);
                keys.push((id.clone(), value));
                json::string(&id)
            })
            .collect()
    };
    let role = |ids: Vec<Value>, threshold: u64| {
        json::object(vec![
            ("keyids", Value::Array(ids)),
            ("threshold", Value::Integer(threshold)),
        ])
    };
    let root_refs: Vec<&PublicKey> = root.iter().collect();
    let root = role(ids(&root_refs), root_threshold);
    let targets = role(ids(&[targets]), 1);
    let snapshot = role(ids(&[snapshot]), 1);
    let timestamp = role(ids(&[timestamp]), 1);
    keys.sort_by(|a, b| a.0.cmp(&b.0));
    keys.dedup_by(|a, b| a.0 == b.0);
    Value::Object(vec![
        ("_type".into(), json::string("root")),
        ("spec_version".into(), json::string(SPEC_VERSION)),
        ("version".into(), Value::Integer(version)),
        ("expires".into(), write::time(expires)),
        ("consistent_snapshot".into(), Value::Bool(true)),
        ("keys".into(), Value::Object(keys)),
        (
            "roles".into(),
            json::object(vec![
                ("root", root),
                ("targets", targets),
                ("snapshot", snapshot),
                ("timestamp", timestamp),
            ]),
        ),
    ])
}

/// `dir/metadata/N.root.json`: the root `document`, signed by `signers`.
pub fn write_root(
    dir: &Path,
    version: u64,
    document: Value,
    signers: &[&SigningKey],
) -> io::Result<()> {
    fs::write(
        dir.join(format!("metadata/{version}.root.json")),
        write::sign(document, signers),
    )
}

/// Where a release's manifest is a target: `ID/VERSION_CODE/ARCHITECTURE/manifest.json`.
pub fn target_path(id: &str, version_code: u64, architecture: &str) -> String {
    format!("{id}/{version_code}/{architecture}/manifest.json")
}

/// A release's custom data: the store's statement signed by `store`, the developer's
/// `signature`, the developer's key the machine pins (`published_key`), its `lineage` if any, and
/// the store's release notes by language, and the store listing the release belongs to (its slug,
/// which the machine resolves an application named by its listing with). `statement` is the
/// store's; built by [`ReleaseStatement::of`] for a manifest the format reads.
pub fn release_custom(
    statement: ReleaseStatement,
    signature: &DeveloperSignature,
    published_key: &PublicKey,
    store: &SigningKey,
    lineage: &Lineage,
    notes: &[(&str, &str)],
    listing: Option<&str>,
) -> Value {
    let release = store.sign_release(statement).to_canonical();
    let text =
        |bytes: Vec<u8>| json::string(&String::from_utf8(bytes).expect("canonical JSON is UTF-8"));
    let mut members = vec![
        ("release", text(release)),
        ("signature", text(signature.to_canonical())),
        ("developerKey", json::string(&published_key.encode())),
    ];
    if !lineage.0.is_empty() {
        members.push(("lineage", text(lineage.to_canonical())));
    }
    if !notes.is_empty() {
        members.push((
            "releaseNotes",
            json::object(
                notes
                    .iter()
                    .map(|(language, text)| (*language, json::string(text)))
                    .collect(),
            ),
        ));
    }
    if let Some(listing) = listing {
        members.push(("listing", json::string(listing)));
    }
    json::object(vec![("oxyos", json::object(members))])
}

/// `value`'s member `name`, if it is an object that has one.
fn member<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Object(members) => members.iter().find(|(key, _)| key == name).map(|(_, v)| v),
        _ => None,
    }
}

/// A repository being written: its directory, its targets, each role's current version and when
/// each role's next document expires.
pub struct Repository {
    pub dir: PathBuf,
    pub targets: Vec<(String, Value)>,
    pub versions: [u64; 3],
    pub expires: [i64; 3],
}

impl Repository {
    /// A new repository at `dir` with root 1 for `keys`, its documents expiring `days` after
    /// `now` (the root three years after).
    pub fn create(dir: &Path, keys: &StoreKeys, now: i64, days: i64) -> io::Result<Self> {
        for sub in ["metadata", "targets", "objects"] {
            fs::create_dir_all(dir.join(sub))?;
        }
        let root: Vec<PublicKey> = keys.root.iter().map(SigningKey::public_key).collect();
        let document = root_document(
            1,
            &root,
            keys.root_threshold,
            &keys.targets.public_key(),
            &keys.snapshot.public_key(),
            &keys.timestamp.public_key(),
            now + 3 * 365 * 86_400,
        );
        let signers: Vec<&SigningKey> = keys.root.iter().collect();
        write_root(dir, 1, document, &signers)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            targets: Vec::new(),
            versions: [0; 3],
            expires: [now + days * 86_400; 3],
        })
    }

    /// The repository at `dir` as its newest documents say, to publish its next versions; its
    /// next documents expire `days` after `now`.
    pub fn open(dir: &Path, now: i64, days: i64) -> io::Result<Self> {
        let newest = |kind: &str| -> io::Result<u64> {
            let mut newest = 0;
            for entry in fs::read_dir(dir.join("metadata"))? {
                let name = entry?.file_name().to_string_lossy().into_owned();
                if let Some(version) = name
                    .strip_suffix(&format!(".{kind}.json"))
                    .and_then(|v| v.parse::<u64>().ok())
                {
                    newest = newest.max(version);
                }
            }
            Ok(newest)
        };
        let (targets_version, snapshot_version) = (newest("targets")?, newest("snapshot")?);
        let mut targets = Vec::new();
        if targets_version > 0 {
            let bytes = fs::read(dir.join(format!("metadata/{targets_version}.targets.json")))?;
            let document = json::parse(&bytes)
                .map_err(|error| io::Error::other(format!("the targets: {error:?}")))?;
            if let Some(Value::Object(listed)) =
                member(&document, "signed").and_then(|signed| member(signed, "targets"))
            {
                targets = listed.clone();
            }
        }
        let timestamp_version = match fs::read(dir.join("metadata/timestamp.json")) {
            Ok(bytes) => json::parse(&bytes)
                .ok()
                .and_then(|d| {
                    member(&d, "signed")
                        .and_then(|s| member(s, "version"))
                        .cloned()
                })
                .and_then(|v| match v {
                    Value::Integer(n) => Some(n),
                    _ => None,
                })
                .unwrap_or(0),
            Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error),
        };
        Ok(Self {
            dir: dir.to_path_buf(),
            targets,
            versions: [targets_version, snapshot_version, timestamp_version],
            expires: [now + days * 86_400; 3],
        })
    }

    /// The root the machine trusts: the image's anchor.
    pub fn anchor(&self) -> PathBuf {
        self.dir.join("metadata/1.root.json")
    }

    /// Adds a release: each of its `files` as an object, its `manifest` as a target at `path`
    /// with its `custom` data. One at that path already is replaced.
    pub fn add_release(
        &mut self,
        path: &str,
        manifest: &[u8],
        files: &[&[u8]],
        custom: Value,
    ) -> io::Result<()> {
        for bytes in files {
            let hex = Sha256::of(bytes).hex();
            let dir = self.dir.join("objects").join(&hex[..2]);
            fs::create_dir_all(&dir)?;
            fs::write(dir.join(&hex), bytes)?;
        }
        let location = self.dir.join(write::target_location(path, manifest));
        fs::create_dir_all(location.parent().expect("a target has a directory"))?;
        fs::write(location, manifest)?;
        self.targets.retain(|(p, _)| p != path);
        self.targets
            .push((path.to_string(), write::target(manifest, Some(custom))));
        Ok(())
    }

    /// Withdraws the release at `path`.
    pub fn remove_release(&mut self, path: &str) {
        self.targets.retain(|(p, _)| p != path);
    }

    fn signed(kind: &str, version: u64, expires: i64, rest: Vec<(&str, Value)>) -> Value {
        let mut members = vec![
            ("_type", json::string(kind)),
            ("spec_version", json::string(SPEC_VERSION)),
            ("version", Value::Integer(version)),
            ("expires", write::time(expires)),
        ];
        members.extend(rest);
        json::object(members)
    }

    /// Publishes the targets: a new targets, snapshot and timestamp, each its next version,
    /// signed by each role's key.
    pub fn publish_metadata(
        &mut self,
        targets_key: &SigningKey,
        snapshot_key: &SigningKey,
        timestamp_key: &SigningKey,
    ) -> io::Result<()> {
        for v in &mut self.versions {
            *v += 1;
        }
        let targets = write::sign(
            Self::signed(
                "targets",
                self.versions[TARGETS],
                self.expires[TARGETS],
                vec![("targets", Value::Object(self.targets.clone()))],
            ),
            &[targets_key],
        );
        fs::write(
            self.dir
                .join(format!("metadata/{}.targets.json", self.versions[TARGETS])),
            &targets,
        )?;
        let snapshot = write::sign(
            Self::signed(
                "snapshot",
                self.versions[SNAPSHOT],
                self.expires[SNAPSHOT],
                vec![(
                    "meta",
                    json::object(vec![(
                        "targets.json",
                        write::meta(self.versions[TARGETS], Some(&targets)),
                    )]),
                )],
            ),
            &[snapshot_key],
        );
        fs::write(
            self.dir.join(format!(
                "metadata/{}.snapshot.json",
                self.versions[SNAPSHOT]
            )),
            &snapshot,
        )?;
        let timestamp = write::sign(
            Self::signed(
                "timestamp",
                self.versions[TIMESTAMP],
                self.expires[TIMESTAMP],
                vec![(
                    "meta",
                    json::object(vec![(
                        "snapshot.json",
                        write::meta(self.versions[SNAPSHOT], Some(&snapshot)),
                    )]),
                )],
            ),
            &[timestamp_key],
        );
        fs::write(self.dir.join("metadata/timestamp.json"), &timestamp)
    }

    /// Publishes a packed package (a directory with `manifest.json`, `manifest.sig` and its
    /// files): its files and manifest, the store's statement signed by the targets key, the
    /// developer's signature and `developer`'s public key, the store `listing` it belongs to if
    /// any, then the next metadata.
    pub fn publish_package(
        &mut self,
        package: &Path,
        keys: &StoreKeys,
        developer: &PublicKey,
        notes: &[(&str, &str)],
        listing: Option<&str>,
    ) -> io::Result<(String, u64)> {
        let invalid = |what: String| io::Error::new(io::ErrorKind::InvalidData, what);
        if let Some(listing) = listing.filter(|l| !crate::install::valid_listing(l)) {
            return Err(invalid(format!(
                "{listing:?} is not a store listing (lowercase letters, digits and single hyphens)"
            )));
        }
        let manifest_bytes = fs::read(package.join("manifest.json"))?;
        let manifest =
            Manifest::parse(&manifest_bytes).map_err(|e| invalid(format!("the manifest: {e}")))?;
        let signature = DeveloperSignature::parse(&fs::read(package.join("manifest.sig"))?)
            .map_err(|e| invalid(format!("the signature: {e}")))?;
        signature
            .verify(&manifest_bytes, developer)
            .map_err(|e| invalid(format!("the signature: {e}")))?;
        let mut files = Vec::new();
        for file in &manifest.files {
            files.push(fs::read(package.join(file.path.as_str()))?);
        }
        let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        let statement =
            ReleaseStatement::of(&manifest, &manifest_bytes, Architecture::Any, &signature);
        let custom = release_custom(
            statement,
            &signature,
            developer,
            &keys.targets,
            &Lineage(Vec::new()),
            notes,
            listing,
        );
        let path = target_path(&manifest.id, manifest.version_code, "any");
        self.add_release(&path, &manifest_bytes, &refs, custom)?;
        self.publish_metadata(&keys.targets, &keys.snapshot, &keys.timestamp)?;
        Ok((manifest.id.clone(), manifest.version_code))
    }
}
