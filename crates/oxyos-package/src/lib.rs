//! An OxyOS application package (docs/adr/0030-installing-applications.mdx, decisions 1 and 3).
//!
//! A package is a tree of files described by `manifest.json`: its id, its Oxy application, its
//! versions, names, icon, entry, runtime, architectures, capabilities, what it opens, and every
//! other file with its size and SHA-256. The manifest is written in RFC 8785's canonical form
//! (JCS) and its bytes are what the developer signs (`manifest.sig`, Ed25519), so its digest
//! covers the whole package. The store's statement of a release (decision 3), the custom data its
//! TUF targets carry (decision 4), names the manifest's digest and the developer's key.
//!
//! Everything here refuses rather than repairs: a document that is not exactly its canonical
//! form, a key written twice, a field this version does not know, a path that could mean two
//! files, a capability that is the system's. No input makes it panic.
//!
//! What it does not do: fetch, install, compile, keep the TUF repository's metadata or decide
//! which key is pinned. Those are `oxyos-installd`'s (decision 5); this crate gives it the checks.

mod base64url;
mod json;
mod manifest;
mod signature;
mod tree;

pub use manifest::{
    Architecture, Capability, CapabilityRequest, Entry, FileEntry, Manifest, Opens, PackagePath,
    Runtime, Sha256, Text, MAX_FILES, MAX_FILE_SIZE, MAX_MANIFEST_SIZE, MAX_PATH_LENGTH, RESERVED,
    SYSTEM_GRANTS,
};
pub use signature::{
    lineage_accepts, lineage_current, DeveloperSignature, KeyId, Lineage, PublicKey,
    ReleaseStatement, Rotation, SignedRelease, SigningKey,
};
pub use tree::{
    copy_verified, open_directory, read_package, verify_stream, verify_tree, Package,
    HERMES_BYTECODE_MAGIC,
};

use std::fmt;

/// Why a package, a signature or a statement is refused. One variant per refusal, so a caller
/// (and a test) tells them apart without reading a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The document is larger than its limit.
    TooLarge(&'static str),
    /// Not JSON, or JSON outside the documents' value space, at this byte offset.
    Json(usize),
    /// A key written twice in one object.
    DuplicateKey(String),
    /// Nested deeper than any document is.
    TooDeep,
    /// JSON, but not its RFC 8785 canonical form: the signed bytes would not be the only ones.
    NotCanonical,
    /// A field this version does not know.
    UnknownField { object: &'static str, field: String },
    /// A required field is absent.
    MissingField {
        object: &'static str,
        field: &'static str,
    },
    /// A field of the wrong JSON type.
    WrongType(&'static str),
    /// An optional field written empty: absent is its only spelling.
    EmptyOptional(&'static str),
    /// A list whose items are not sorted and unique.
    NotSorted(&'static str),
    /// More items than the limit.
    TooMany(&'static str),
    /// The package id is not a reverse domain name of this crate's alphabet.
    InvalidId(String),
    /// The Oxy application id is neither a legacy 24-character id nor a UUID.
    InvalidApplication(String),
    /// A field's value is malformed (the version, the module, a language, a text, a digest…).
    Invalid(&'static str),
    /// `versionCode` or the runtime is zero or out of range.
    OutOfRange(&'static str),
    /// A path that is not relative, plain and of the package's alphabet.
    InvalidPath(String),
    /// Two paths that a case-insensitive file system would take for one.
    DuplicatePath(String),
    /// A path that names a file and, below it, another.
    PathConflict(String),
    /// A path the package may not list (`manifest.json`, `manifest.sig`, `index.hbc`).
    ReservedPath(String),
    /// The entry's bundle or the icon is not one of the listed files.
    NotListed { field: &'static str, path: String },
    /// An architecture this version does not know.
    UnknownArchitecture(String),
    /// Native code: an architecture other than `any`, or a file under `native/`. Not accepted
    /// until a native-extension ABI and confinement exist (decision 1).
    NativeCode,
    /// A capability outside decision 8's vocabulary.
    UnknownCapability(String),
    /// A system grant (`shell`, `settings`, `store`…) asked for as a capability: never a
    /// package's (decisions 7 and 8).
    SystemGrant(String),
    /// `open-files` without file types, file types without `open-files`, and the same for
    /// `url-schemes`.
    OpensMismatch(&'static str),
    /// The entry's bundle is Hermes bytecode; a package carries JavaScript (decision 1).
    Bytecode,
    /// A signature's algorithm other than Ed25519.
    UnsupportedAlgorithm(String),
    /// A public key or signature of the wrong length or encoding.
    InvalidKey,
    /// The signature names another key than the one it is checked against.
    KeyMismatch,
    /// The signature does not verify.
    BadSignature,
    /// The pinned key is not in the lineage.
    NotInLineage,
    /// A rotation that does not follow from the key before it.
    LineageGap,
    /// A key that comes back in the lineage.
    LineageCycle,
    /// Signed by a key of the lineage that is not its current one.
    NotCurrentKey,
    /// The store's statement is signed by no key the machine trusts.
    UntrustedStoreKey,
    /// The store's statement does not describe this manifest or its signature.
    ReleaseMismatch(&'static str),
    /// Reading the package's tree failed: the path and the system's error.
    Io { path: String, error: String },
    /// A link in the package's tree.
    Link(String),
    /// Neither a regular file nor a directory.
    NotRegular(String),
    /// A file or directory the manifest does not list.
    Unlisted(String),
    /// A listed file that is absent.
    Missing(String),
    /// A file whose size is not the listed one (or a stream longer than it).
    SizeMismatch(String),
    /// A file whose SHA-256 is not the listed one.
    DigestMismatch(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooLarge(what) => write!(f, "{what} is too large"),
            Error::Json(at) => write!(f, "not a document of this format (byte {at})"),
            Error::DuplicateKey(key) => write!(f, "{key:?} is written twice"),
            Error::TooDeep => f.write_str("nested too deep"),
            Error::NotCanonical => f.write_str("not in its canonical form (RFC 8785)"),
            Error::UnknownField { object, field } => write!(f, "{object} has no field {field:?}"),
            Error::MissingField { object, field } => write!(f, "{object} lacks {field:?}"),
            Error::WrongType(field) => write!(f, "{field} has the wrong type"),
            Error::EmptyOptional(field) => write!(f, "{field} is empty; leave it out"),
            Error::NotSorted(field) => write!(f, "{field} is not sorted and unique"),
            Error::TooMany(field) => write!(f, "{field} has too many items"),
            Error::InvalidId(id) => write!(f, "{id:?} is not a package id"),
            Error::InvalidApplication(id) => write!(f, "{id:?} is not an Oxy application id"),
            Error::Invalid(field) => write!(f, "{field} is malformed"),
            Error::OutOfRange(field) => write!(f, "{field} is out of range"),
            Error::InvalidPath(path) => write!(f, "{path:?} is not a package path"),
            Error::DuplicatePath(path) => write!(f, "{path:?} is listed twice"),
            Error::PathConflict(path) => write!(f, "{path:?} is a file with files below it"),
            Error::ReservedPath(path) => write!(f, "{path:?} is reserved"),
            Error::NotListed { field, path } => write!(f, "{field} {path:?} is not listed"),
            Error::UnknownArchitecture(arch) => write!(f, "unknown architecture {arch:?}"),
            Error::NativeCode => f.write_str("native code is not accepted"),
            Error::UnknownCapability(name) => write!(f, "unknown capability {name:?}"),
            Error::SystemGrant(name) => write!(f, "{name:?} is the system's, never a package's"),
            Error::OpensMismatch(what) => write!(f, "{what} and its capability disagree"),
            Error::Bytecode => f.write_str("the entry is bytecode; a package carries JavaScript"),
            Error::UnsupportedAlgorithm(alg) => write!(f, "unsupported algorithm {alg:?}"),
            Error::InvalidKey => f.write_str("malformed key or signature"),
            Error::KeyMismatch => f.write_str("signed by another key"),
            Error::BadSignature => f.write_str("the signature does not verify"),
            Error::NotInLineage => f.write_str("the pinned key is not in the lineage"),
            Error::LineageGap => f.write_str("the lineage is broken"),
            Error::LineageCycle => f.write_str("a key comes back in the lineage"),
            Error::NotCurrentKey => f.write_str("not signed by the lineage's current key"),
            Error::UntrustedStoreKey => f.write_str("not signed by a trusted store key"),
            Error::ReleaseMismatch(field) => write!(f, "the store's statement differs: {field}"),
            Error::Io { path, error } => write!(f, "{path}: {error}"),
            Error::Link(path) => write!(f, "{path:?} is a link"),
            Error::NotRegular(path) => write!(f, "{path:?} is not a regular file"),
            Error::Unlisted(path) => write!(f, "{path:?} is not listed"),
            Error::Missing(path) => write!(f, "{path:?} is missing"),
            Error::SizeMismatch(path) => write!(f, "{path:?} has the wrong size"),
            Error::DigestMismatch(path) => write!(f, "{path:?} has the wrong digest"),
        }
    }
}

impl std::error::Error for Error {}

impl From<json::JsonError> for Error {
    fn from(error: json::JsonError) -> Self {
        match error {
            json::JsonError::Syntax(at) => Error::Json(at),
            json::JsonError::DuplicateKey(key) => Error::DuplicateKey(key),
            json::JsonError::TooDeep => Error::TooDeep,
        }
    }
}

/// Parses a document that must be exactly its canonical form.
fn canonical_document(
    bytes: &[u8],
    limit: usize,
    what: &'static str,
) -> Result<json::Value, Error> {
    if bytes.len() > limit {
        return Err(Error::TooLarge(what));
    }
    let value = json::parse(bytes)?;
    if json::canonical(&value) != bytes {
        return Err(Error::NotCanonical);
    }
    Ok(value)
}

/// An object's members, taken one by one; what is left is unknown.
struct Fields {
    object: &'static str,
    members: Vec<(String, json::Value)>,
}

impl Fields {
    fn of(object: &'static str, value: json::Value) -> Result<Self, Error> {
        match value {
            json::Value::Object(members) => Ok(Fields { object, members }),
            _ => Err(Error::WrongType(object)),
        }
    }

    fn optional(&mut self, field: &'static str) -> Option<json::Value> {
        let at = self.members.iter().position(|(k, _)| k == field)?;
        Some(self.members.remove(at).1)
    }

    fn required(&mut self, field: &'static str) -> Result<json::Value, Error> {
        self.optional(field).ok_or(Error::MissingField {
            object: self.object,
            field,
        })
    }

    fn string(&mut self, field: &'static str) -> Result<String, Error> {
        match self.required(field)? {
            json::Value::String(s) => Ok(s),
            _ => Err(Error::WrongType(field)),
        }
    }

    fn integer(&mut self, field: &'static str) -> Result<u64, Error> {
        match self.required(field)? {
            json::Value::Integer(n) => Ok(n),
            _ => Err(Error::WrongType(field)),
        }
    }

    fn finish(self) -> Result<(), Error> {
        match self.members.into_iter().next() {
            Some((field, _)) => Err(Error::UnknownField {
                object: self.object,
                field,
            }),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
