//! Ed25519 keys and the package's three signed documents (ADR 0030, decision 3): the
//! developer's signature over the manifest, a developer key's rotation to the next, and the
//! store's statement of an approved release.
//!
//! Each signature is over a context string, a zero byte and the document's canonical bytes, so
//! bytes signed as one kind of document are never valid as another, whoever holds the key.
//! Verification is `ring`'s Ed25519, which refuses a signature whose scalar is not reduced
//! (the malleable spelling).

use crate::json::{self, Value};
use crate::manifest::{valid_package_id, Architecture, Manifest, Runtime, Sha256};
use crate::{base64url, canonical_document, Error, Fields};
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use std::collections::HashSet;
use std::fmt;

const MANIFEST_CONTEXT: &[u8] = b"oxyos-package manifest v1";
const ROTATION_CONTEXT: &[u8] = b"oxyos-package rotation v1";
const RELEASE_CONTEXT: &[u8] = b"oxyos-package release v1";
const MAX_SIGNED_DOCUMENT: usize = 64 << 10;
const MAX_ROTATIONS: usize = 64;
const ALGORITHM: &str = "ed25519";

fn signed_message(context: &[u8], document: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(context.len() + 1 + document.len());
    message.extend_from_slice(context);
    message.push(0);
    message.extend_from_slice(document);
    message
}

/// An Ed25519 public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(pub [u8; 32]);

impl PublicKey {
    /// Its id: the unpadded base64url of the SHA-256 of its 32 bytes. One key has one id, and
    /// no id names two keys.
    pub fn key_id(&self) -> KeyId {
        KeyId(base64url::encode(&Sha256::of(&self.0).0))
    }

    /// The key from its unpadded base64url spelling.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let bytes = base64url::decode(text).ok_or(Error::InvalidKey)?;
        Ok(PublicKey(bytes.try_into().map_err(|_| Error::InvalidKey)?))
    }

    pub fn encode(&self) -> String {
        base64url::encode(&self.0)
    }

    fn verify(&self, context: &[u8], document: &[u8], signature: &[u8; 64]) -> Result<(), Error> {
        UnparsedPublicKey::new(&ED25519, self.0)
            .verify(&signed_message(context, document), signature)
            .map_err(|_| Error::BadSignature)
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.encode())
    }
}

/// A key's id ([`PublicKey::key_id`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyId(String);

impl KeyId {
    pub fn parse(text: &str) -> Result<Self, Error> {
        match base64url::decode(text) {
            Some(bytes) if bytes.len() == 32 => Ok(KeyId(text.to_string())),
            _ => Err(Error::InvalidKey),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An Ed25519 private key, from its 32-byte seed. Signing is for developers' tools and tests;
/// the system only verifies.
pub struct SigningKey(Ed25519KeyPair);

impl SigningKey {
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        // A 32-byte seed is always a key; `from_seed_unchecked` fails only on other lengths.
        SigningKey(Ed25519KeyPair::from_seed_unchecked(seed).expect("a 32-byte seed is a key"))
    }

    pub fn public_key(&self) -> PublicKey {
        let mut out = [0; 32];
        out.copy_from_slice(self.0.public_key().as_ref());
        PublicKey(out)
    }

    fn sign(&self, context: &[u8], document: &[u8]) -> [u8; 64] {
        let mut out = [0; 64];
        out.copy_from_slice(self.0.sign(&signed_message(context, document)).as_ref());
        out
    }

    /// The developer's signature over a manifest's canonical bytes.
    pub fn sign_manifest(&self, manifest: &[u8]) -> DeveloperSignature {
        DeveloperSignature {
            key_id: self.public_key().key_id(),
            signature: self.sign(MANIFEST_CONTEXT, manifest),
        }
    }

    /// This key handing over to `next`, at `signed_at` (an RFC 3339 time, said, not checked).
    pub fn rotate_to(&self, next: &PublicKey, signed_at: Option<&str>) -> Result<Rotation, Error> {
        let mut rotation = Rotation {
            previous_key_id: self.public_key().key_id(),
            next_key: *next,
            signed_at: signed_at.map(str::to_string),
            signature: [0; 64],
        };
        if let Some(time) = &rotation.signed_at {
            check_time(time)?;
        }
        rotation.signature = self.sign(ROTATION_CONTEXT, &rotation.signed_bytes());
        Ok(rotation)
    }

    /// The store's statement of a release, signed.
    pub fn sign_release(&self, statement: ReleaseStatement) -> SignedRelease {
        let signature = self.sign(RELEASE_CONTEXT, &json::canonical(&statement.to_value()));
        SignedRelease {
            statement,
            key_id: self.public_key().key_id(),
            signature,
        }
    }
}

fn signature_bytes(text: &str) -> Result<[u8; 64], Error> {
    base64url::decode(text)
        .and_then(|b| b.try_into().ok())
        .ok_or(Error::InvalidKey)
}

fn algorithm(fields: &mut Fields) -> Result<(), Error> {
    let algorithm = fields.string("algorithm")?;
    if algorithm != ALGORITHM {
        return Err(Error::UnsupportedAlgorithm(algorithm));
    }
    Ok(())
}

/// A time as RFC 3339 writes it in UTC (`2026-10-30T12:00:00Z`): said in a rotation, never
/// compared with a clock.
fn check_time(text: &str) -> Result<(), Error> {
    let b = text.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if b.len() == 20
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'T'
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19)
        && b[19] == b'Z'
    {
        Ok(())
    } else {
        Err(Error::Invalid("signedAt"))
    }
}

/// `manifest.sig`: `{"algorithm":"ed25519","keyId":…,"signature":…}`, canonical.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeveloperSignature {
    pub key_id: KeyId,
    pub signature: [u8; 64],
}

impl DeveloperSignature {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let mut fields = Fields::of(
            "manifest.sig",
            canonical_document(bytes, MAX_SIGNED_DOCUMENT, "manifest.sig")?,
        )?;
        algorithm(&mut fields)?;
        let key_id = KeyId::parse(&fields.string("keyId")?)?;
        let signature = signature_bytes(&fields.string("signature")?)?;
        fields.finish()?;
        Ok(DeveloperSignature { key_id, signature })
    }

    pub fn to_canonical(&self) -> Vec<u8> {
        json::canonical(&json::object(vec![
            ("algorithm", json::string(ALGORITHM)),
            ("keyId", json::string(self.key_id.as_str())),
            (
                "signature",
                json::string(&base64url::encode(&self.signature)),
            ),
        ]))
    }

    /// Verifies the signature over the manifest's exact bytes with `key`, which must be the
    /// key it names.
    pub fn verify(&self, manifest: &[u8], key: &PublicKey) -> Result<(), Error> {
        if key.key_id() != self.key_id {
            return Err(Error::KeyMismatch);
        }
        key.verify(MANIFEST_CONTEXT, manifest, &self.signature)
    }
}

/// A developer key handing over to the next, signed by the key it replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rotation {
    pub previous_key_id: KeyId,
    pub next_key: PublicKey,
    pub signed_at: Option<String>,
    pub signature: [u8; 64],
}

impl Rotation {
    fn signed_value(&self) -> Value {
        let mut members = vec![
            ("nextPublicKey", json::string(&self.next_key.encode())),
            ("previousKeyId", json::string(self.previous_key_id.as_str())),
        ];
        if let Some(time) = &self.signed_at {
            members.push(("signedAt", json::string(time)));
        }
        json::object(members)
    }

    fn signed_bytes(&self) -> Vec<u8> {
        json::canonical(&self.signed_value())
    }

    fn to_value(&self) -> Value {
        let Value::Object(mut members) = self.signed_value() else {
            unreachable!("a rotation is an object")
        };
        members.push(("algorithm".into(), json::string(ALGORITHM)));
        members.push((
            "signature".into(),
            json::string(&base64url::encode(&self.signature)),
        ));
        Value::Object(members)
    }

    fn from_value(value: Value) -> Result<Self, Error> {
        let mut fields = Fields::of("a rotation", value)?;
        algorithm(&mut fields)?;
        let previous_key_id = KeyId::parse(&fields.string("previousKeyId")?)?;
        let next_key = PublicKey::parse(&fields.string("nextPublicKey")?)?;
        let signed_at = match fields.optional("signedAt") {
            Some(Value::String(time)) => {
                check_time(&time)?;
                Some(time)
            }
            Some(_) => return Err(Error::WrongType("signedAt")),
            None => None,
        };
        let signature = signature_bytes(&fields.string("signature")?)?;
        fields.finish()?;
        Ok(Rotation {
            previous_key_id,
            next_key,
            signed_at,
            signature,
        })
    }

    fn verify(&self, previous: &PublicKey) -> Result<(), Error> {
        if previous.key_id() != self.previous_key_id {
            return Err(Error::LineageGap);
        }
        previous.verify(ROTATION_CONTEXT, &self.signed_bytes(), &self.signature)
    }
}

/// A developer key's lineage: its rotations, oldest first, as `{"rotations":[…]}`, canonical.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lineage(pub Vec<Rotation>);

impl Lineage {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let mut fields = Fields::of(
            "the lineage",
            canonical_document(bytes, MAX_SIGNED_DOCUMENT, "the lineage")?,
        )?;
        let rotations = match fields.required("rotations")? {
            Value::Array(items) if items.len() > MAX_ROTATIONS => {
                return Err(Error::TooMany("rotations"))
            }
            Value::Array(items) => items
                .into_iter()
                .map(Rotation::from_value)
                .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(Error::WrongType("rotations")),
        };
        fields.finish()?;
        Ok(Lineage(rotations))
    }

    pub fn to_canonical(&self) -> Vec<u8> {
        json::canonical(&json::object(vec![(
            "rotations",
            Value::Array(self.0.iter().map(Rotation::to_value).collect()),
        )]))
    }
}

/// The lineage's current key as seen from `pinned`, the key the machine pinned at first install.
///
/// The lineage must be one chain: each rotation's previous key is the one the rotation before it
/// handed over to, and signed it. The pinned key must be in it (the first rotation's previous
/// key, or a key a rotation handed over to); the rotations after it are verified from it. No key
/// comes back. An empty lineage leaves the pinned key current.
pub fn lineage_current(pinned: &PublicKey, lineage: &Lineage) -> Result<PublicKey, Error> {
    let rotations = &lineage.0;
    if rotations.is_empty() {
        return Ok(*pinned);
    }
    // One chain, no key twice.
    let mut seen = HashSet::new();
    seen.insert(rotations[0].previous_key_id.clone());
    for (i, rotation) in rotations.iter().enumerate() {
        if i > 0 && rotation.previous_key_id != rotations[i - 1].next_key.key_id() {
            return Err(Error::LineageGap);
        }
        if !seen.insert(rotation.next_key.key_id()) {
            return Err(Error::LineageCycle);
        }
    }
    let pinned_id = pinned.key_id();
    // Where the pinned key is: before every rotation, or handed over to by rotation `i`.
    let start = if rotations[0].previous_key_id == pinned_id {
        0
    } else {
        rotations
            .iter()
            .position(|r| r.next_key.key_id() == pinned_id)
            .map(|i| i + 1)
            .ok_or(Error::NotInLineage)?
    };
    let mut current = *pinned;
    for rotation in &rotations[start..] {
        rotation.verify(&current)?;
        current = rotation.next_key;
    }
    Ok(current)
}

/// Whether a release signed by `signing` may update an application whose pinned key is
/// `pinned`: `signing` must be the lineage's current key. Answers that key, which the machine
/// pins from then on.
pub fn lineage_accepts(
    pinned: &PublicKey,
    lineage: &Lineage,
    signing: &KeyId,
) -> Result<PublicKey, Error> {
    let current = lineage_current(pinned, lineage)?;
    if current.key_id() != *signing {
        return Err(Error::NotCurrentKey);
    }
    Ok(current)
}

/// The store's statement of an approved release (decision 3): the custom data the store's TUF
/// targets carry for it (decision 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseStatement {
    pub package_id: String,
    pub version_code: u64,
    pub runtime: Runtime,
    pub architecture: Architecture,
    pub manifest_sha256: Sha256,
    pub manifest_length: u64,
    pub developer_key_id: KeyId,
}

impl ReleaseStatement {
    /// The statement of this manifest's release for `architecture`, signed by the developer as
    /// `signature` says.
    pub fn of(
        manifest: &Manifest,
        manifest_bytes: &[u8],
        architecture: Architecture,
        signature: &DeveloperSignature,
    ) -> Self {
        ReleaseStatement {
            package_id: manifest.id.clone(),
            version_code: manifest.version_code,
            runtime: manifest.runtime,
            architecture,
            manifest_sha256: Sha256::of(manifest_bytes),
            manifest_length: manifest_bytes.len() as u64,
            developer_key_id: signature.key_id.clone(),
        }
    }

    pub(crate) fn to_value(&self) -> Value {
        json::object(vec![
            ("packageId", json::string(&self.package_id)),
            ("versionCode", Value::Integer(self.version_code)),
            (
                "runtime",
                json::object(vec![
                    ("abi", Value::Integer(self.runtime.abi.into())),
                    ("api", Value::Integer(self.runtime.api.into())),
                ]),
            ),
            ("architecture", json::string(self.architecture.name())),
            ("manifestSha256", json::string(&self.manifest_sha256.hex())),
            ("manifestLength", Value::Integer(self.manifest_length)),
            (
                "developerKeyId",
                json::string(self.developer_key_id.as_str()),
            ),
        ])
    }

    fn from_value(value: Value) -> Result<Self, Error> {
        let mut fields = Fields::of("the statement", value)?;
        let package_id = fields.string("packageId")?;
        if !valid_package_id(&package_id) {
            return Err(Error::InvalidId(package_id));
        }
        let version_code = fields.integer("versionCode")?;
        if version_code == 0 {
            return Err(Error::OutOfRange("versionCode"));
        }
        let mut r = Fields::of("runtime", fields.required("runtime")?)?;
        let abi = u32::try_from(r.integer("abi")?).ok().filter(|v| *v >= 1);
        let api = u32::try_from(r.integer("api")?).ok();
        r.finish()?;
        let (Some(abi), Some(api)) = (abi, api) else {
            return Err(Error::OutOfRange("runtime"));
        };
        let architecture = Architecture::parse(&fields.string("architecture")?)?;
        let manifest_sha256 = Sha256::parse(&fields.string("manifestSha256")?)
            .ok_or(Error::Invalid("manifestSha256"))?;
        let manifest_length = fields.integer("manifestLength")?;
        let developer_key_id = KeyId::parse(&fields.string("developerKeyId")?)?;
        fields.finish()?;
        Ok(ReleaseStatement {
            package_id,
            version_code,
            runtime: Runtime { abi, api },
            architecture,
            manifest_sha256,
            manifest_length,
            developer_key_id,
        })
    }

    /// Whether this statement describes this manifest, these bytes and this signature's key.
    /// The developer's signature itself is verified apart ([`DeveloperSignature::verify`]).
    pub fn matches(
        &self,
        manifest: &Manifest,
        manifest_bytes: &[u8],
        signature: &DeveloperSignature,
    ) -> Result<(), Error> {
        let check = |ok: bool, field| {
            if ok {
                Ok(())
            } else {
                Err(Error::ReleaseMismatch(field))
            }
        };
        check(
            self.manifest_length == manifest_bytes.len() as u64,
            "manifestLength",
        )?;
        check(
            self.manifest_sha256 == Sha256::of(manifest_bytes),
            "manifestSha256",
        )?;
        check(self.package_id == manifest.id, "packageId")?;
        check(self.version_code == manifest.version_code, "versionCode")?;
        check(self.runtime == manifest.runtime, "runtime")?;
        check(
            manifest.architectures.contains(&self.architecture),
            "architecture",
        )?;
        check(self.developer_key_id == signature.key_id, "developerKeyId")
    }
}

/// A signed statement: `{"algorithm":"ed25519","keyId":…,"signature":…,"statement":{…}}`,
/// canonical, the signature over the statement's canonical bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedRelease {
    pub statement: ReleaseStatement,
    pub key_id: KeyId,
    pub signature: [u8; 64],
}

impl SignedRelease {
    /// Reads a signed statement and verifies it with the store key it names, which must be one
    /// of `trusted`.
    pub fn verify(bytes: &[u8], trusted: &[PublicKey]) -> Result<ReleaseStatement, Error> {
        let mut fields = Fields::of(
            "the release",
            canonical_document(bytes, MAX_SIGNED_DOCUMENT, "the release")?,
        )?;
        algorithm(&mut fields)?;
        let key_id = KeyId::parse(&fields.string("keyId")?)?;
        let signature = signature_bytes(&fields.string("signature")?)?;
        let statement = fields.required("statement")?;
        fields.finish()?;
        let key = trusted
            .iter()
            .find(|k| k.key_id() == key_id)
            .ok_or(Error::UntrustedStoreKey)?;
        key.verify(RELEASE_CONTEXT, &json::canonical(&statement), &signature)?;
        ReleaseStatement::from_value(statement)
    }

    pub fn to_canonical(&self) -> Vec<u8> {
        json::canonical(&json::object(vec![
            ("algorithm", json::string(ALGORITHM)),
            ("keyId", json::string(self.key_id.as_str())),
            (
                "signature",
                json::string(&base64url::encode(&self.signature)),
            ),
            ("statement", self.statement.to_value()),
        ]))
    }
}
