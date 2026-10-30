//! The store's metadata as The Update Framework 1.0 gives it (docs/adr/0030-installing-
//! applications.mdx, decision 4), and the client that keeps it: the top-level roles alone.
//!
//! - **The documents** are TUF's: `{"signed": …, "signatures": [{"keyid", "sig"}]}`, the signature
//!   Ed25519 over the `signed` object's canonical JSON as TUF defines it (OLPC's: members sorted,
//!   no whitespace, only `"` and `\` escaped), which python-tuf and tuf-js write. They are read
//!   with the package's strict reader, so a key written twice is refused. Unknown fields are
//!   allowed, as the specification asks, and are covered by the signature like the rest.
//! - **Keys** are `ed25519` keys of the `ed25519` scheme; a key whose id is not the SHA-256 of its
//!   canonical form is not used, so one key cannot count twice towards a threshold under two ids.
//!   A key of another type is not used either.
//! - **The client** follows the specification's workflow (section 5): the root updated one
//!   version at a time, each new root signed by the threshold of the old and of itself; then the
//!   timestamp, the snapshot and the targets, each signed by its role's threshold, of the version
//!   the role before names, with the length and hash it names, never older than what the machine
//!   already has, and not expired. Keys of the timestamp or snapshot rotated by a new root drop the
//!   machine's copies of those roles (the specification's recovery from a fast-forward attack).
//! - **The repository's layout** is python-tuf's: the documents in `metadata/`, the targets in
//!   `targets/`. With consistent snapshots the snapshot and targets are `N.snapshot.json` and
//!   `N.targets.json`, a target `<directory>/<sha256>.<name>`.
//! - **Not here:** delegations (the hashed bins the store will use as it grows): only the
//!   top-level targets role is read, and targets a delegated role would sign are not found.

use crate::fetch::{Fetch, FetchError};
use crate::time;
use oxyos_package::json::{self, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

/// The most each document may be, in bytes, when the role before does not give its length.
pub const MAX_ROOT: u64 = 512 << 10;
pub const MAX_TIMESTAMP: u64 = 16 << 10;
pub const MAX_SNAPSHOT: u64 = 4 << 20;
pub const MAX_TARGETS: u64 = 32 << 20;
/// The most root versions one refresh walks through.
pub const MAX_ROOT_ROTATIONS: u64 = 32;

/// Why the store's metadata is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TufError {
    /// The repository could not be read.
    Fetch(String),
    /// A document longer than it may be.
    TooLarge(&'static str),
    /// Not a document of its role: what is wrong.
    Malformed(&'static str, String),
    /// Fewer valid signatures of the role's keys than its threshold.
    Threshold(&'static str),
    /// A version other than the one the role before names.
    Version {
        role: &'static str,
        expected: u64,
        found: u64,
    },
    /// Older than what the machine already has.
    Rollback(&'static str),
    /// Expired: a repository may be frozen at a moment, or replayed.
    Expired(&'static str),
    /// Not the length or hash the role before names.
    Mismatch(&'static str),
    /// No such target in the targets role.
    NoTarget(String),
    /// The machine's own copy of the metadata could not be written or read.
    Local(String),
}

impl fmt::Display for TufError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TufError::Fetch(e) => write!(f, "the store's repository: {e}"),
            TufError::TooLarge(role) => write!(f, "{role} is too large"),
            TufError::Malformed(role, what) => write!(f, "{role}: {what}"),
            TufError::Threshold(role) => write!(f, "{role} is not signed by enough of its keys"),
            TufError::Version {
                role,
                expected,
                found,
            } => write!(f, "{role} is version {found}, not {expected}"),
            TufError::Rollback(role) => write!(f, "{role} is older than the machine's"),
            TufError::Expired(role) => write!(f, "{role} has expired"),
            TufError::Mismatch(role) => write!(f, "{role} is not the length or hash it should be"),
            TufError::NoTarget(path) => write!(f, "no target {path:?}"),
            TufError::Local(e) => write!(f, "the machine's metadata: {e}"),
        }
    }
}

impl std::error::Error for TufError {}

/// TUF's canonical JSON (OLPC's), which its signatures are over.
pub fn canonical(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_canonical(&mut out, value);
    out
}

fn write_canonical(out: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Object(members) => {
            let mut sorted: Vec<&(String, Value)> = members.iter().collect();
            sorted.sort_by(|(a, _), (b, _)| a.as_bytes().cmp(b.as_bytes()));
            out.push(b'{');
            for (i, (key, value)) in sorted.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical_string(out, key);
                out.push(b':');
                write_canonical(out, value);
            }
            out.push(b'}');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(out, item);
            }
            out.push(b']');
        }
        Value::String(text) => write_canonical_string(out, text),
        Value::Integer(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
    }
}

fn write_canonical_string(out: &mut Vec<u8>, text: &str) {
    out.push(b'"');
    for byte in text.bytes() {
        if byte == b'"' || byte == b'\\' {
            out.push(b'\\');
        }
        out.push(byte);
    }
    out.push(b'"');
}

/// An object's member.
fn member<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

fn members(value: &Value) -> Option<&[(String, Value)]> {
    match value {
        Value::Object(members) => Some(members),
        _ => None,
    }
}

fn text<'a>(value: &'a Value, key: &str, role: &'static str) -> Result<&'a str, TufError> {
    match member(value, key) {
        Some(Value::String(s)) => Ok(s),
        _ => Err(TufError::Malformed(role, format!("{key} is not a string"))),
    }
}

fn integer(value: &Value, key: &str, role: &'static str) -> Result<u64, TufError> {
    match member(value, key) {
        Some(Value::Integer(n)) => Ok(*n),
        _ => Err(TufError::Malformed(
            role,
            format!("{key} is not an integer"),
        )),
    }
}

fn hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            let byte = text.get(i..i + 2)?;
            if !byte
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return None;
            }
            u8::from_str_radix(byte, 16).ok()
        })
        .collect()
}

/// Bytes in lowercase hex.
pub(crate) fn hex_of(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    out.copy_from_slice(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref());
    out
}

/// A signed document of a role: its `signed` object, the canonical bytes signed, and its
/// signatures by key id.
#[derive(Clone, Debug)]
pub struct Envelope {
    pub signed: Value,
    signed_bytes: Vec<u8>,
    signatures: Vec<(String, Vec<u8>)>,
}

impl Envelope {
    fn parse(bytes: &[u8], role: &'static str, kind: &str) -> Result<Self, TufError> {
        let value = json::parse(bytes).map_err(|e| TufError::Malformed(role, format!("{e:?}")))?;
        let signed = member(&value, "signed")
            .cloned()
            .ok_or_else(|| TufError::Malformed(role, "no signed object".into()))?;
        let mut signatures = Vec::new();
        match member(&value, "signatures") {
            Some(Value::Array(items)) => {
                for item in items {
                    let keyid = text(item, "keyid", role)?.to_string();
                    let sig = hex(text(item, "sig", role)?).ok_or_else(|| {
                        TufError::Malformed(role, "a signature is not hex".into())
                    })?;
                    signatures.push((keyid, sig));
                }
            }
            _ => return Err(TufError::Malformed(role, "no signatures".into())),
        }
        if text(&signed, "_type", role)? != kind {
            return Err(TufError::Malformed(role, format!("not a {kind} document")));
        }
        let spec = text(&signed, "spec_version", role)?;
        if spec.split('.').next() != Some("1") {
            return Err(TufError::Malformed(role, format!("specification {spec}")));
        }
        Ok(Envelope {
            signed_bytes: canonical(&signed),
            signed,
            signatures,
        })
    }

    fn version(&self, role: &'static str) -> Result<u64, TufError> {
        let version = integer(&self.signed, "version", role)?;
        if version == 0 {
            return Err(TufError::Malformed(role, "version 0".into()));
        }
        Ok(version)
    }

    fn expires(&self, role: &'static str) -> Result<i64, TufError> {
        time::parse_utc(text(&self.signed, "expires", role)?)
            .ok_or_else(|| TufError::Malformed(role, "expires is not a time".into()))
    }
}

/// A role as the root describes it: its keys' ids and how many must sign.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleKeys {
    pub keyids: Vec<String>,
    pub threshold: u64,
}

/// The root role: every role's keys.
#[derive(Clone, Debug)]
pub struct Root {
    pub version: u64,
    pub expires: i64,
    pub consistent_snapshot: bool,
    /// Usable keys by id: Ed25519, their id their canonical form's SHA-256.
    keys: BTreeMap<String, [u8; 32]>,
    roles: BTreeMap<String, RoleKeys>,
}

impl Root {
    fn from_envelope(envelope: &Envelope) -> Result<Self, TufError> {
        const ROLE: &str = "root";
        let signed = &envelope.signed;
        let mut keys = BTreeMap::new();
        for (keyid, key) in members(member(signed, "keys").unwrap_or(&Value::Bool(false)))
            .ok_or_else(|| TufError::Malformed(ROLE, "keys is not an object".into()))?
        {
            let usable = text(key, "keytype", ROLE).ok() == Some("ed25519")
                && text(key, "scheme", ROLE).ok() == Some("ed25519")
                && hex_of(&sha256(&canonical(key))) == *keyid;
            let public = member(key, "keyval")
                .and_then(|keyval| match member(keyval, "public") {
                    Some(Value::String(s)) => hex(s),
                    _ => None,
                })
                .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok());
            if let (true, Some(public)) = (usable, public) {
                keys.insert(keyid.clone(), public);
            }
        }
        let mut roles = BTreeMap::new();
        for (name, role) in members(member(signed, "roles").unwrap_or(&Value::Bool(false)))
            .ok_or_else(|| TufError::Malformed(ROLE, "roles is not an object".into()))?
        {
            let threshold = integer(role, "threshold", ROLE)?;
            let keyids = match member(role, "keyids") {
                Some(Value::Array(ids)) => ids
                    .iter()
                    .map(|id| match id {
                        Value::String(s) => Ok(s.clone()),
                        _ => Err(TufError::Malformed(ROLE, "a key id is not a string".into())),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                _ => return Err(TufError::Malformed(ROLE, "keyids is not a list".into())),
            };
            roles.insert(name.clone(), RoleKeys { keyids, threshold });
        }
        for required in ["root", "targets", "snapshot", "timestamp"] {
            if !roles.contains_key(required) {
                return Err(TufError::Malformed(ROLE, format!("no {required} role")));
            }
        }
        let consistent_snapshot = matches!(
            member(signed, "consistent_snapshot"),
            Some(Value::Bool(true))
        );
        Ok(Root {
            version: envelope.version(ROLE)?,
            expires: envelope.expires(ROLE)?,
            consistent_snapshot,
            keys,
            roles,
        })
    }

    /// The role's keys.
    pub fn role(&self, name: &str) -> Option<&RoleKeys> {
        self.roles.get(name)
    }

    /// The role's usable public keys.
    pub fn public_keys(&self, role: &str) -> Vec<[u8; 32]> {
        self.roles
            .get(role)
            .map(|r| {
                r.keyids
                    .iter()
                    .filter_map(|id| self.keys.get(id).copied())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether `envelope` is signed by at least `role`'s threshold of its keys, each counted
    /// once. A threshold of 0 is never met.
    fn verify(&self, role: &'static str, envelope: &Envelope) -> Result<(), TufError> {
        let keys = self.roles.get(role).ok_or(TufError::Threshold(role))?;
        let allowed: BTreeSet<&str> = keys.keyids.iter().map(String::as_str).collect();
        let mut valid = BTreeSet::new();
        for (keyid, sig) in &envelope.signatures {
            if !allowed.contains(keyid.as_str()) || valid.contains(keyid.as_str()) {
                continue;
            }
            let Some(public) = self.keys.get(keyid) else {
                continue;
            };
            let key = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public);
            if key.verify(&envelope.signed_bytes, sig).is_ok() {
                valid.insert(keyid.as_str());
            }
        }
        if keys.threshold == 0 || (valid.len() as u64) < keys.threshold {
            return Err(TufError::Threshold(role));
        }
        Ok(())
    }
}

/// What a role says of another's document: its version, and maybe its length and hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetaInfo {
    pub version: u64,
    pub length: Option<u64>,
    pub sha256: Option<[u8; 32]>,
}

impl MetaInfo {
    fn parse(value: &Value, role: &'static str) -> Result<Self, TufError> {
        let version = integer(value, "version", role)?;
        let length = match member(value, "length") {
            None => None,
            Some(Value::Integer(n)) => Some(*n),
            Some(_) => return Err(TufError::Malformed(role, "length".into())),
        };
        let sha256 = match member(value, "hashes") {
            None => None,
            Some(hashes) => Some(digest_of(hashes, role)?),
        };
        Ok(MetaInfo {
            version,
            length,
            sha256,
        })
    }

    fn check(&self, bytes: &[u8], role: &'static str) -> Result<(), TufError> {
        if self.length.is_some_and(|l| l != bytes.len() as u64)
            || self.sha256.is_some_and(|h| h != sha256(bytes))
        {
            return Err(TufError::Mismatch(role));
        }
        Ok(())
    }
}

/// A `hashes` object's SHA-256; one without it is refused (no other hash is checked).
fn digest_of(hashes: &Value, role: &'static str) -> Result<[u8; 32], TufError> {
    match member(hashes, "sha256") {
        Some(Value::String(s)) => hex(s)
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
            .ok_or_else(|| TufError::Malformed(role, "a sha256 is not 32 bytes of hex".into())),
        _ => Err(TufError::Malformed(role, "no sha256 hash".into())),
    }
}

/// The timestamp role: the snapshot's version.
#[derive(Clone, Debug)]
pub struct Timestamp {
    pub version: u64,
    pub expires: i64,
    pub snapshot: MetaInfo,
}

impl Timestamp {
    fn from_envelope(envelope: &Envelope) -> Result<Self, TufError> {
        const ROLE: &str = "timestamp";
        let snapshot = member(&envelope.signed, "meta")
            .and_then(|meta| member(meta, "snapshot.json"))
            .ok_or_else(|| TufError::Malformed(ROLE, "no snapshot.json".into()))?;
        Ok(Timestamp {
            version: envelope.version(ROLE)?,
            expires: envelope.expires(ROLE)?,
            snapshot: MetaInfo::parse(snapshot, ROLE)?,
        })
    }
}

/// The snapshot role: the version of every targets document.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub version: u64,
    pub expires: i64,
    pub meta: BTreeMap<String, MetaInfo>,
}

impl Snapshot {
    fn from_envelope(envelope: &Envelope) -> Result<Self, TufError> {
        const ROLE: &str = "snapshot";
        let mut meta = BTreeMap::new();
        for (name, info) in member(&envelope.signed, "meta")
            .and_then(members)
            .ok_or_else(|| TufError::Malformed(ROLE, "no meta".into()))?
        {
            meta.insert(name.clone(), MetaInfo::parse(info, ROLE)?);
        }
        if !meta.contains_key("targets.json") {
            return Err(TufError::Malformed(ROLE, "no targets.json".into()));
        }
        Ok(Snapshot {
            version: envelope.version(ROLE)?,
            expires: envelope.expires(ROLE)?,
            meta,
        })
    }
}

/// A target: its length, SHA-256 and custom data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub length: u64,
    pub sha256: [u8; 32],
    pub custom: Option<Value>,
}

/// The targets role.
#[derive(Clone, Debug)]
pub struct Targets {
    pub version: u64,
    pub expires: i64,
    pub targets: BTreeMap<String, Target>,
}

impl Targets {
    fn from_envelope(envelope: &Envelope) -> Result<Self, TufError> {
        const ROLE: &str = "targets";
        let mut targets = BTreeMap::new();
        for (path, target) in member(&envelope.signed, "targets")
            .and_then(members)
            .ok_or_else(|| TufError::Malformed(ROLE, "no targets".into()))?
        {
            let length = integer(target, "length", ROLE)?;
            let sha256 = digest_of(
                member(target, "hashes")
                    .ok_or_else(|| TufError::Malformed(ROLE, "a target has no hashes".into()))?,
                ROLE,
            )?;
            targets.insert(
                path.clone(),
                Target {
                    length,
                    sha256,
                    custom: member(target, "custom").cloned(),
                },
            );
        }
        Ok(Targets {
            version: envelope.version(ROLE)?,
            expires: envelope.expires(ROLE)?,
            targets,
        })
    }
}

/// Reads a file of the repository into memory, at most `limit` bytes.
fn fetch_bytes(fetch: &dyn Fetch, path: &str, limit: u64) -> Result<Option<Vec<u8>>, TufError> {
    match fetch.fetch_to_vec(path, limit) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(FetchError::NotFound) => Ok(None),
        Err(FetchError::TooLarge) => Err(TufError::TooLarge("a document")),
        Err(FetchError::Failed(e)) => Err(TufError::Fetch(e)),
    }
}

/// The machine's trusted metadata, kept in its own directory (`/var/lib/oxyos/apps/store`): the
/// newest root it trusts and the timestamp, snapshot and targets it last verified, each as the
/// bytes received.
pub struct Client {
    directory: PathBuf,
    root: Root,
    timestamp: Option<Timestamp>,
    snapshot: Option<Snapshot>,
    targets: Option<Targets>,
}

/// The metadata files the machine keeps.
const ROOT_FILE: &str = "root.json";
const TIMESTAMP_FILE: &str = "timestamp.json";
const SNAPSHOT_FILE: &str = "snapshot.json";
const TARGETS_FILE: &str = "targets.json";

impl Client {
    /// The client over `directory`, trusting the newer of the root kept there and `anchor`, the
    /// image's (`/usr/share/oxyos/store/root.json`): an OS update may bring a newer one. The
    /// timestamp, snapshot and targets kept are used, for rollback checks, only if the root
    /// still verifies them; their expiry is not checked here.
    pub fn open(directory: &Path, anchor: &Path) -> Result<Self, TufError> {
        let read = |path: &Path| -> Result<Option<Vec<u8>>, TufError> {
            match std::fs::read(path) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(TufError::Local(format!("{}: {e}", path.display()))),
            }
        };
        let parse_root = |bytes: &[u8]| -> Result<(Root, Envelope), TufError> {
            let envelope = Envelope::parse(bytes, "root", "root")?;
            let root = Root::from_envelope(&envelope)?;
            // A trusted root is the one its own keys sign.
            root.verify("root", &envelope)?;
            Ok((root, envelope))
        };
        let anchor_bytes = read(anchor)?
            .ok_or_else(|| TufError::Local(format!("{}: no trusted root", anchor.display())))?;
        let (mut root, _) = parse_root(&anchor_bytes)?;
        if let Some(bytes) = read(&directory.join(ROOT_FILE))? {
            if let Ok((kept, _)) = parse_root(&bytes) {
                if kept.version > root.version {
                    root = kept;
                }
            }
        }
        let mut client = Client {
            directory: directory.to_path_buf(),
            root,
            timestamp: None,
            snapshot: None,
            targets: None,
        };
        if let Some(bytes) = read(&directory.join(TIMESTAMP_FILE))? {
            client.timestamp = Envelope::parse(&bytes, "timestamp", "timestamp")
                .and_then(|e| {
                    client.root.verify("timestamp", &e)?;
                    Timestamp::from_envelope(&e)
                })
                .ok();
        }
        if let Some(bytes) = read(&directory.join(SNAPSHOT_FILE))? {
            client.snapshot = Envelope::parse(&bytes, "snapshot", "snapshot")
                .and_then(|e| {
                    client.root.verify("snapshot", &e)?;
                    Snapshot::from_envelope(&e)
                })
                .ok();
        }
        if let Some(bytes) = read(&directory.join(TARGETS_FILE))? {
            client.targets = Envelope::parse(&bytes, "targets", "targets")
                .and_then(|e| {
                    client.root.verify("targets", &e)?;
                    Targets::from_envelope(&e)
                })
                .ok();
        }
        Ok(client)
    }

    pub fn root(&self) -> &Root {
        &self.root
    }

    /// The targets last verified, if a refresh has succeeded.
    pub fn targets(&self) -> Option<&Targets> {
        self.targets.as_ref()
    }

    fn keep(&self, name: &str, bytes: &[u8]) -> Result<(), TufError> {
        crate::state::write_file(&self.directory, name, bytes, 0o644, None)
            .map_err(|e| TufError::Local(e.to_string()))
    }

    fn forget(&self, name: &str) -> Result<(), TufError> {
        match std::fs::remove_file(self.directory.join(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(TufError::Local(e.to_string()))
            }
            _ => Ok(()),
        }
    }

    /// Updates the metadata from the repository at `now`, as the specification's section 5
    /// says. On any error what the machine had stays; what was already verified and kept (a
    /// newer root) stays too, as the specification keeps it.
    pub fn refresh(&mut self, fetch: &dyn Fetch, now: i64) -> Result<(), TufError> {
        // 5.3: the root, one version at a time.
        let original = self.root.clone();
        for _ in 0..MAX_ROOT_ROTATIONS {
            let next = self.root.version + 1;
            let Some(bytes) = fetch_bytes(fetch, &format!("metadata/{next}.root.json"), MAX_ROOT)?
            else {
                break;
            };
            let envelope = Envelope::parse(&bytes, "root", "root")?;
            self.root.verify("root", &envelope)?;
            let root = Root::from_envelope(&envelope)?;
            root.verify("root", &envelope)?;
            if root.version != next {
                return Err(TufError::Version {
                    role: "root",
                    expected: next,
                    found: root.version,
                });
            }
            self.keep(ROOT_FILE, &bytes)?;
            self.root = root;
        }
        if self.root.expires <= now {
            return Err(TufError::Expired("root"));
        }
        for role in ["timestamp", "snapshot"] {
            if self.root.role(role) != original.role(role) {
                self.timestamp = None;
                self.snapshot = None;
                self.forget(TIMESTAMP_FILE)?;
                self.forget(SNAPSHOT_FILE)?;
            }
        }

        // 5.4: the timestamp.
        let bytes = fetch_bytes(fetch, &format!("metadata/{TIMESTAMP_FILE}"), MAX_TIMESTAMP)?
            .ok_or_else(|| TufError::Fetch("no timestamp.json".into()))?;
        let envelope = Envelope::parse(&bytes, "timestamp", "timestamp")?;
        self.root.verify("timestamp", &envelope)?;
        let timestamp = Timestamp::from_envelope(&envelope)?;
        if let Some(trusted) = &self.timestamp {
            if timestamp.version < trusted.version
                || timestamp.snapshot.version < trusted.snapshot.version
            {
                return Err(TufError::Rollback("timestamp"));
            }
        }
        if timestamp.expires <= now {
            return Err(TufError::Expired("timestamp"));
        }
        self.keep(TIMESTAMP_FILE, &bytes)?;
        let snapshot_info = timestamp.snapshot.clone();
        self.timestamp = Some(timestamp);

        // 5.5: the snapshot the timestamp names.
        let name = self.consistent("snapshot.json", snapshot_info.version);
        let bytes = fetch_bytes(fetch, &name, snapshot_info.length.unwrap_or(MAX_SNAPSHOT))?
            .ok_or_else(|| TufError::Fetch(format!("no {name}")))?;
        snapshot_info.check(&bytes, "snapshot")?;
        let envelope = Envelope::parse(&bytes, "snapshot", "snapshot")?;
        self.root.verify("snapshot", &envelope)?;
        let snapshot = Snapshot::from_envelope(&envelope)?;
        if snapshot.version != snapshot_info.version {
            return Err(TufError::Version {
                role: "snapshot",
                expected: snapshot_info.version,
                found: snapshot.version,
            });
        }
        if let Some(trusted) = &self.snapshot {
            for (name, old) in &trusted.meta {
                match snapshot.meta.get(name) {
                    Some(new) if new.version >= old.version => {}
                    _ => return Err(TufError::Rollback("snapshot")),
                }
            }
        }
        if snapshot.expires <= now {
            return Err(TufError::Expired("snapshot"));
        }
        self.keep(SNAPSHOT_FILE, &bytes)?;
        let targets_info = snapshot.meta["targets.json"].clone();
        self.snapshot = Some(snapshot);

        // 5.6: the targets the snapshot names.
        if let Some(trusted) = &self.targets {
            if targets_info.version < trusted.version {
                return Err(TufError::Rollback("targets"));
            }
        }
        let name = self.consistent("targets.json", targets_info.version);
        let bytes = fetch_bytes(fetch, &name, targets_info.length.unwrap_or(MAX_TARGETS))?
            .ok_or_else(|| TufError::Fetch(format!("no {name}")))?;
        targets_info.check(&bytes, "targets")?;
        let envelope = Envelope::parse(&bytes, "targets", "targets")?;
        self.root.verify("targets", &envelope)?;
        let targets = Targets::from_envelope(&envelope)?;
        if targets.version != targets_info.version {
            return Err(TufError::Version {
                role: "targets",
                expected: targets_info.version,
                found: targets.version,
            });
        }
        if targets.expires <= now {
            return Err(TufError::Expired("targets"));
        }
        self.keep(TARGETS_FILE, &bytes)?;
        self.targets = Some(targets);
        Ok(())
    }

    /// A role's document in the repository's `metadata/`, by version with consistent snapshots.
    fn consistent(&self, name: &str, version: u64) -> String {
        if self.root.consistent_snapshot {
            format!("metadata/{version}.{name}")
        } else {
            format!("metadata/{name}")
        }
    }

    /// Where the target `path` is in the repository: with a consistent snapshot, its directory,
    /// then its SHA-256 before its name.
    pub fn target_location(&self, path: &str, target: &Target) -> String {
        if !self.root.consistent_snapshot {
            return format!("targets/{path}");
        }
        let (directory, name) = match path.rsplit_once('/') {
            Some((directory, name)) => (format!("{directory}/"), name),
            None => (String::new(), path),
        };
        format!("targets/{directory}{}.{name}", hex_of(&target.sha256))
    }

    /// The target `path`'s bytes, of its length and hash.
    pub fn fetch_target(&self, fetch: &dyn Fetch, path: &str) -> Result<Vec<u8>, TufError> {
        let target = self
            .targets
            .as_ref()
            .and_then(|t| t.targets.get(path))
            .ok_or_else(|| TufError::NoTarget(path.to_string()))?;
        let bytes = fetch_bytes(fetch, &self.target_location(path, target), target.length)?
            .ok_or_else(|| TufError::NoTarget(path.to_string()))?;
        if bytes.len() as u64 != target.length || sha256(&bytes) != target.sha256 {
            return Err(TufError::Mismatch("a target"));
        }
        Ok(bytes)
    }
}

/// Writing TUF's documents, for the tests' repositories and development tools: the system only
/// reads them.
pub mod write {
    use super::*;
    use oxyos_package::SigningKey;

    /// A key's TUF form and its id.
    pub fn key(public: &[u8; 32]) -> (String, Value) {
        let value = json::object(vec![
            ("keytype", json::string("ed25519")),
            ("scheme", json::string("ed25519")),
            (
                "keyval",
                json::object(vec![("public", json::string(&hex_of(public)))]),
            ),
        ]);
        (hex_of(&sha256(&canonical(&value))), value)
    }

    /// A document: `signed`, signed by each of `keys`.
    pub fn sign(signed: Value, keys: &[&SigningKey]) -> Vec<u8> {
        let bytes = canonical(&signed);
        let signatures = keys
            .iter()
            .map(|k| {
                let (keyid, _) = key(&k.public_key().0);
                json::object(vec![
                    ("keyid", json::string(&keyid)),
                    ("sig", json::string(&hex_of(&k.sign_raw(&bytes)))),
                ])
            })
            .collect();
        canonical(&json::object(vec![
            ("signatures", Value::Array(signatures)),
            ("signed", signed),
        ]))
    }

    /// A `meta` entry: a version and, when given, the document's length and hash.
    pub fn meta(version: u64, document: Option<&[u8]>) -> Value {
        let mut members = vec![("version", Value::Integer(version))];
        if let Some(bytes) = document {
            members.push(("length", Value::Integer(bytes.len() as u64)));
            members.push((
                "hashes",
                json::object(vec![("sha256", json::string(&hex_of(&sha256(bytes))))]),
            ));
        }
        json::object(members)
    }

    /// A target entry.
    pub fn target(bytes: &[u8], custom: Option<Value>) -> Value {
        let mut members = vec![
            ("length", Value::Integer(bytes.len() as u64)),
            (
                "hashes",
                json::object(vec![("sha256", json::string(&hex_of(&sha256(bytes))))]),
            ),
        ];
        if let Some(custom) = custom {
            members.push(("custom", custom));
        }
        json::object(members)
    }

    /// Where a consistent snapshot keeps a target: its directory, then its SHA-256 before its
    /// name.
    pub fn target_location(path: &str, bytes: &[u8]) -> String {
        let (directory, name) = match path.rsplit_once('/') {
            Some((directory, name)) => (format!("{directory}/"), name),
            None => (String::new(), path),
        };
        format!("targets/{directory}{}.{name}", hex_of(&sha256(bytes)))
    }

    pub fn time(seconds: i64) -> Value {
        json::string(&crate::time::format_utc(seconds))
    }
}
