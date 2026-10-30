//! `manifest.json`: what a package is, and every file it has.

use crate::json::{self, Value};
use crate::{canonical_document, Error, Fields};
use std::collections::{BTreeMap, HashSet};
use std::fmt;

/// The largest manifest: room for [`MAX_FILES`] files with long paths, and no more.
pub const MAX_MANIFEST_SIZE: usize = 4 << 20;
/// The most files a package lists.
pub const MAX_FILES: usize = 16_384;
/// The largest file.
pub const MAX_FILE_SIZE: u64 = 1 << 30;
/// The longest path, in bytes.
pub const MAX_PATH_LENGTH: usize = 255;
const MAX_COMPONENT_LENGTH: usize = 128;
const MAX_COMPONENTS: usize = 16;
const MAX_LANGUAGES: usize = 64;
const MAX_NAME_LENGTH: usize = 128;
const MAX_PURPOSE_LENGTH: usize = 512;
const MAX_OPENS: usize = 64;

/// Paths at a package's top that it never lists: the manifest, its signature, and the bytecode
/// `oxyos-installd` compiles beside them (ADR 0030, decisions 1 and 6). Compared ignoring case.
pub const RESERVED: [&str; 3] = ["manifest.json", "manifest.sig", "index.hbc"];

/// Grants that are the system's alone (ADR 0030, decision 7): a package naming one as a
/// capability is refused, whatever signed it.
pub const SYSTEM_GRANTS: [&str; 6] = [
    "shell",
    "lock",
    "shortcuts",
    "settings",
    "store",
    "accounts",
];

/// A SHA-256 digest, written as 64 lowercase hexadecimal digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sha256(pub [u8; 32]);

impl Sha256 {
    pub fn of(bytes: &[u8]) -> Self {
        let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
        let mut out = [0; 32];
        out.copy_from_slice(digest.as_ref());
        Sha256(out)
    }

    /// The digest's lowercase hexadecimal spelling, its only one.
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 64 {
            return None;
        }
        let digit = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        let mut out = [0; 32];
        for (i, pair) in bytes.chunks(2).enumerate() {
            out[i] = (digit(pair[0])? << 4) | digit(pair[1])?;
        }
        Some(Sha256(out))
    }

    pub fn hex(&self) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(64);
        for b in self.0 {
            out.push(DIGITS[usize::from(b >> 4)] as char);
            out.push(DIGITS[usize::from(b & 0xf)] as char);
        }
        out
    }
}

impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256({})", self.hex())
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

/// A path inside a package: relative, `/`-separated, each part of ASCII letters, digits and
/// `._-@+`, never `.` or `..`. With nothing else, a path means one file on every file system and
/// to every reader, and cannot leave the package.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackagePath(String);

impl PackagePath {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let invalid = || Error::InvalidPath(text.to_string());
        if text.is_empty() || text.len() > MAX_PATH_LENGTH {
            return Err(invalid());
        }
        let components: Vec<&str> = text.split('/').collect();
        if components.len() > MAX_COMPONENTS {
            return Err(invalid());
        }
        for component in &components {
            if component.is_empty()
                || *component == "."
                || *component == ".."
                || component.len() > MAX_COMPONENT_LENGTH
                || !component.bytes().all(|b| {
                    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'@' | b'+')
                })
            {
                return Err(invalid());
            }
        }
        Ok(PackagePath(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Its parts, from the package's top.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    fn folded(&self) -> String {
        self.0.to_ascii_lowercase()
    }
}

impl fmt::Display for PackagePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A text people read (a name, a purpose): not empty, no surrounding space, no control
/// characters and no bidirectional overrides, which could make the install sheet say what the
/// package does not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text(String);

impl Text {
    fn parse(text: String, limit: usize, field: &'static str) -> Result<Self, Error> {
        let bidi = |c: char| matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{61c}');
        if text.is_empty()
            || text.len() > limit
            || text.trim() != text
            || text.chars().any(|c| c.is_control() || bidi(c))
        {
            return Err(Error::Invalid(field));
        }
        Ok(Text(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A listed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PackagePath,
    pub size: u64,
    pub sha256: Sha256,
}

/// What the runtime starts: the bundle (JavaScript) and the module it registers with
/// `AppRegistry`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub bundle: PackagePath,
    pub module: String,
}

/// The runtime a package is built for (ADR 0030, decision 2): it runs on a runtime of the same
/// `abi` whose `api` is at least its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Runtime {
    pub abi: u32,
    pub api: u32,
}

impl Runtime {
    pub fn runs_on(&self, runtime: &Runtime) -> bool {
        self.abi == runtime.abi && self.api <= runtime.api
    }
}

/// An architecture a package's files are for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Architecture {
    /// No native code: every architecture.
    Any,
    X86_64,
    Aarch64,
}

impl Architecture {
    pub fn name(&self) -> &'static str {
        match self {
            Architecture::Any => "any",
            Architecture::X86_64 => "x86_64",
            Architecture::Aarch64 => "aarch64",
        }
    }

    pub fn parse(name: &str) -> Result<Self, Error> {
        match name {
            "any" => Ok(Architecture::Any),
            "x86_64" => Ok(Architecture::X86_64),
            "aarch64" => Ok(Architecture::Aarch64),
            other => Err(Error::UnknownArchitecture(other.to_string())),
        }
    }
}

/// A capability a package may ask for (ADR 0030, decision 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Capability {
    Network,
    Notifications,
    Files,
    Camera,
    Microphone,
    Location,
    OxyAccount,
    OpenFiles,
    UrlSchemes,
    Background,
    LoginItem,
}

impl Capability {
    pub const ALL: [Capability; 11] = [
        Capability::Network,
        Capability::Notifications,
        Capability::Files,
        Capability::Camera,
        Capability::Microphone,
        Capability::Location,
        Capability::OxyAccount,
        Capability::OpenFiles,
        Capability::UrlSchemes,
        Capability::Background,
        Capability::LoginItem,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Capability::Network => "network",
            Capability::Notifications => "notifications",
            Capability::Files => "files",
            Capability::Camera => "camera",
            Capability::Microphone => "microphone",
            Capability::Location => "location",
            Capability::OxyAccount => "oxy-account",
            Capability::OpenFiles => "open-files",
            Capability::UrlSchemes => "url-schemes",
            Capability::Background => "background",
            Capability::LoginItem => "login-item",
        }
    }

    pub fn parse(name: &str) -> Result<Self, Error> {
        if SYSTEM_GRANTS.contains(&name) {
            return Err(Error::SystemGrant(name.to_string()));
        }
        Capability::ALL
            .into_iter()
            .find(|c| c.name() == name)
            .ok_or_else(|| Error::UnknownCapability(name.to_string()))
    }

    /// Asked of the person when the application first uses it rather than only shown at install
    /// (decision 8's table).
    pub fn asked_at_first_use(&self) -> bool {
        matches!(
            self,
            Capability::Notifications
                | Capability::Files
                | Capability::Camera
                | Capability::Microphone
                | Capability::Location
                | Capability::Background
                | Capability::LoginItem
        )
    }
}

/// A capability with its purpose, in the person's languages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityRequest {
    pub capability: Capability,
    /// The purpose by language tag (`en`, `es`, `pt-BR`); at least one.
    pub purpose: BTreeMap<String, Text>,
}

/// The kinds of file and the URL schemes a package opens (ADR 0027's opening of files).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Opens {
    /// Media types (`text/plain`), sorted and unique.
    pub types: Vec<String>,
    /// URL schemes (`mailto`), sorted and unique.
    pub schemes: Vec<String>,
}

/// A package's manifest, read from its canonical bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// Reverse-DNS, the registry's id (`so.oxy.mention`).
    pub id: String,
    /// The Oxy application it belongs to (oxy-api's application id).
    pub application: String,
    /// The version people read (`1.4.0`).
    pub version: String,
    /// The version machines compare; it only grows.
    pub version_code: u64,
    pub name: Text,
    /// Localized names by language tag.
    pub names: BTreeMap<String, Text>,
    pub icon: PackagePath,
    pub entry: Entry,
    pub runtime: Runtime,
    /// `[Any]`: nothing else is accepted yet (decision 1).
    pub architectures: Vec<Architecture>,
    /// Sorted by capability, each once.
    pub capabilities: Vec<CapabilityRequest>,
    pub opens: Opens,
    /// Every other file, sorted by path.
    pub files: Vec<FileEntry>,
}

/// A package id: a reverse domain name of lowercase ASCII letters, digits, `-` and `_`, at least
/// two labels, at most 128 bytes. Every such id is one `oxyos_apps::valid_id` accepts (this is a
/// stricter rule, copied rather than depended on: the registry's rule is for Desktop Entries,
/// whose ids may be any case).
pub(crate) fn valid_package_id(id: &str) -> bool {
    id.len() <= 128
        && id.split('.').count() >= 2
        && id.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        })
}

/// An Oxy id: a legacy 24-character hexadecimal id or a UUID, lowercase (the SDK's `ids.ts`).
fn valid_application(id: &str) -> bool {
    let hex = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    if id.len() == 24 {
        return hex(id);
    }
    let parts: Vec<&str> = id.split('-').collect();
    parts.len() == 5
        && parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12])
        && parts.iter().all(|p| hex(p))
}

fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
}

fn valid_module(module: &str) -> bool {
    let mut bytes = module.bytes();
    module.len() <= 128
        && matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic() || b == b'_' || b == b'$')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
}

/// A BCP 47 tag's shape: subtags of 1 to 8 ASCII letters or digits, the first of 2 or 3
/// letters, at most 35 bytes.
fn valid_language(tag: &str) -> bool {
    let mut subtags = tag.split('-');
    tag.len() <= 35
        && matches!(subtags.next(), Some(first) if (2..=3).contains(&first.len()) && first.bytes().all(|b| b.is_ascii_alphabetic()))
        && subtags
            .all(|s| (1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric()))
}

fn valid_media_type(text: &str) -> bool {
    let token = |s: &str| {
        !s.is_empty()
            && s.len() <= 64
            && s.bytes().all(|b| {
                b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || matches!(
                        b,
                        b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                    )
            })
    };
    matches!(text.split_once('/'), Some((t, s)) if token(t) && token(s))
}

fn valid_scheme(text: &str) -> bool {
    let mut bytes = text.bytes();
    text.len() <= 32
        && matches!(bytes.next(), Some(b) if b.is_ascii_lowercase())
        && bytes.all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'+' | b'.' | b'-')
        })
}

fn u32_in(value: u64, min: u32, field: &'static str) -> Result<u32, Error> {
    u32::try_from(value)
        .ok()
        .filter(|v| *v >= min)
        .ok_or(Error::OutOfRange(field))
}

fn array(value: Value, field: &'static str, limit: usize) -> Result<Vec<Value>, Error> {
    match value {
        Value::Array(items) if items.len() > limit => Err(Error::TooMany(field)),
        Value::Array(items) => Ok(items),
        _ => Err(Error::WrongType(field)),
    }
}

fn strings(value: Value, field: &'static str, limit: usize) -> Result<Vec<String>, Error> {
    array(value, field, limit)?
        .into_iter()
        .map(|item| match item {
            Value::String(s) => Ok(s),
            _ => Err(Error::WrongType(field)),
        })
        .collect()
}

fn sorted_unique<T: Ord>(items: &[T], field: &'static str) -> Result<(), Error> {
    if items.windows(2).all(|w| w[0] < w[1]) {
        Ok(())
    } else {
        Err(Error::NotSorted(field))
    }
}

fn localized(
    value: Value,
    field: &'static str,
    limit: usize,
) -> Result<BTreeMap<String, Text>, Error> {
    let Value::Object(members) = value else {
        return Err(Error::WrongType(field));
    };
    if members.len() > MAX_LANGUAGES {
        return Err(Error::TooMany(field));
    }
    let mut out = BTreeMap::new();
    let mut folded = HashSet::new();
    for (tag, text) in members {
        if !valid_language(&tag) || !folded.insert(tag.to_ascii_lowercase()) {
            return Err(Error::Invalid("a language tag"));
        }
        let Value::String(text) = text else {
            return Err(Error::WrongType(field));
        };
        out.insert(tag, Text::parse(text, limit, field)?);
    }
    Ok(out)
}

impl Manifest {
    /// Reads a manifest from its bytes, which must be exactly its canonical form.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let value = canonical_document(bytes, MAX_MANIFEST_SIZE, "the manifest")?;
        Manifest::from_value(value)
    }

    fn from_value(value: Value) -> Result<Self, Error> {
        let mut fields = Fields::of("the manifest", value)?;

        let id = fields.string("id")?;
        if !valid_package_id(&id) {
            return Err(Error::InvalidId(id));
        }
        let application = fields.string("application")?;
        if !valid_application(&application) {
            return Err(Error::InvalidApplication(application));
        }
        let version = fields.string("version")?;
        if !valid_version(&version) {
            return Err(Error::Invalid("version"));
        }
        let version_code = fields.integer("versionCode")?;
        if version_code == 0 {
            return Err(Error::OutOfRange("versionCode"));
        }
        let name = Text::parse(fields.string("name")?, MAX_NAME_LENGTH, "name")?;
        let names = match fields.optional("names") {
            Some(value) => {
                let names = localized(value, "names", MAX_NAME_LENGTH)?;
                if names.is_empty() {
                    return Err(Error::EmptyOptional("names"));
                }
                names
            }
            None => BTreeMap::new(),
        };
        let icon = PackagePath::parse(&fields.string("icon")?)?;

        let mut entry_fields = Fields::of("entry", fields.required("entry")?)?;
        let bundle = PackagePath::parse(&entry_fields.string("bundle")?)?;
        let module = entry_fields.string("module")?;
        entry_fields.finish()?;
        if !valid_module(&module) {
            return Err(Error::Invalid("entry.module"));
        }
        let entry = Entry { bundle, module };

        let mut runtime_fields = Fields::of("runtime", fields.required("runtime")?)?;
        let runtime = Runtime {
            abi: u32_in(runtime_fields.integer("abi")?, 1, "runtime.abi")?,
            api: u32_in(runtime_fields.integer("api")?, 0, "runtime.api")?,
        };
        runtime_fields.finish()?;

        let architectures = strings(fields.required("architectures")?, "architectures", 3)?
            .iter()
            .map(|a| Architecture::parse(a))
            .collect::<Result<Vec<_>, _>>()?;
        if architectures.is_empty() {
            return Err(Error::MissingField {
                object: "the manifest",
                field: "architectures",
            });
        }
        // Anything but `any` alone is native code (decision 1).
        if architectures != [Architecture::Any] {
            return Err(Error::NativeCode);
        }

        let capabilities = match fields.optional("capabilities") {
            Some(value) => {
                let items = array(value, "capabilities", Capability::ALL.len())?;
                if items.is_empty() {
                    return Err(Error::EmptyOptional("capabilities"));
                }
                let mut out = Vec::new();
                for item in items {
                    let mut c = Fields::of("a capability", item)?;
                    let capability = Capability::parse(&c.string("name")?)?;
                    let purpose = localized(
                        c.required("purpose")?,
                        "a capability's purpose",
                        MAX_PURPOSE_LENGTH,
                    )?;
                    c.finish()?;
                    if purpose.is_empty() {
                        return Err(Error::MissingField {
                            object: "a capability",
                            field: "purpose",
                        });
                    }
                    out.push(CapabilityRequest {
                        capability,
                        purpose,
                    });
                }
                sorted_unique(
                    &out.iter().map(|c| c.capability.name()).collect::<Vec<_>>(),
                    "capabilities",
                )?;
                out
            }
            None => Vec::new(),
        };

        let opens = match fields.optional("opens") {
            Some(value) => {
                let mut o = Fields::of("opens", value)?;
                let mut list = |field: &'static str, name: &'static str| match o.optional(name) {
                    Some(v) => {
                        let items = strings(v, field, MAX_OPENS)?;
                        if items.is_empty() {
                            return Err(Error::EmptyOptional(field));
                        }
                        Ok(items)
                    }
                    None => Ok(Vec::new()),
                };
                let types = list("opens.types", "types")?;
                let schemes = list("opens.schemes", "schemes")?;
                o.finish()?;
                if types.is_empty() && schemes.is_empty() {
                    return Err(Error::EmptyOptional("opens"));
                }
                if !types.iter().all(|t| valid_media_type(t)) {
                    return Err(Error::Invalid("opens.types"));
                }
                if !schemes.iter().all(|s| valid_scheme(s)) {
                    return Err(Error::Invalid("opens.schemes"));
                }
                sorted_unique(&types, "opens.types")?;
                sorted_unique(&schemes, "opens.schemes")?;
                Opens { types, schemes }
            }
            None => Opens::default(),
        };
        let declares = |c: Capability| capabilities.iter().any(|r| r.capability == c);
        if opens.types.is_empty() == declares(Capability::OpenFiles) {
            return Err(Error::OpensMismatch("opens.types"));
        }
        if opens.schemes.is_empty() == declares(Capability::UrlSchemes) {
            return Err(Error::OpensMismatch("opens.schemes"));
        }

        let mut files = Vec::new();
        for item in array(fields.required("files")?, "files", MAX_FILES)? {
            let mut f = Fields::of("a file", item)?;
            let path = PackagePath::parse(&f.string("path")?)?;
            let size = f.integer("size")?;
            let sha256 =
                Sha256::parse(&f.string("sha256")?).ok_or(Error::Invalid("a file's sha256"))?;
            f.finish()?;
            if size > MAX_FILE_SIZE {
                return Err(Error::TooLarge("a file"));
            }
            files.push(FileEntry { path, size, sha256 });
        }
        fields.finish()?;
        check_files(&files)?;

        let listed = |path: &PackagePath| files.iter().any(|f| f.path == *path);
        if !listed(&entry.bundle) {
            return Err(Error::NotListed {
                field: "entry.bundle",
                path: entry.bundle.to_string(),
            });
        }
        if !entry.bundle.as_str().ends_with(".js") {
            return Err(Error::Invalid("entry.bundle"));
        }
        if !listed(&icon) {
            return Err(Error::NotListed {
                field: "icon",
                path: icon.to_string(),
            });
        }
        if !(icon.as_str().ends_with(".png") || icon.as_str().ends_with(".svg")) {
            return Err(Error::Invalid("icon"));
        }

        Ok(Manifest {
            id,
            application,
            version,
            version_code,
            name,
            names,
            icon,
            entry,
            runtime,
            architectures,
            capabilities,
            opens,
            files,
        })
    }

    /// The listed file at `path`.
    pub fn file(&self, path: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.path.as_str() == path)
    }

    /// The manifest's canonical bytes: what [`Manifest::parse`] reads back to this manifest,
    /// and what the developer signs.
    pub fn to_canonical(&self) -> Vec<u8> {
        json::canonical(&self.to_value())
    }

    fn to_value(&self) -> Value {
        let texts = |map: &BTreeMap<String, Text>| {
            Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), json::string(v.as_str())))
                    .collect(),
            )
        };
        let strings =
            |items: &[String]| Value::Array(items.iter().map(|s| json::string(s)).collect());
        let mut members = vec![
            ("id", json::string(&self.id)),
            ("application", json::string(&self.application)),
            ("version", json::string(&self.version)),
            ("versionCode", Value::Integer(self.version_code)),
            ("name", json::string(self.name.as_str())),
            ("icon", json::string(self.icon.as_str())),
            (
                "entry",
                json::object(vec![
                    ("bundle", json::string(self.entry.bundle.as_str())),
                    ("module", json::string(&self.entry.module)),
                ]),
            ),
            (
                "runtime",
                json::object(vec![
                    ("abi", Value::Integer(self.runtime.abi.into())),
                    ("api", Value::Integer(self.runtime.api.into())),
                ]),
            ),
            (
                "architectures",
                Value::Array(
                    self.architectures
                        .iter()
                        .map(|a| json::string(a.name()))
                        .collect(),
                ),
            ),
            (
                "files",
                Value::Array(
                    self.files
                        .iter()
                        .map(|f| {
                            json::object(vec![
                                ("path", json::string(f.path.as_str())),
                                ("size", Value::Integer(f.size)),
                                ("sha256", json::string(&f.sha256.hex())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ];
        if !self.names.is_empty() {
            members.push(("names", texts(&self.names)));
        }
        if !self.capabilities.is_empty() {
            members.push((
                "capabilities",
                Value::Array(
                    self.capabilities
                        .iter()
                        .map(|c| {
                            json::object(vec![
                                ("name", json::string(c.capability.name())),
                                ("purpose", texts(&c.purpose)),
                            ])
                        })
                        .collect(),
                ),
            ));
        }
        if !self.opens.types.is_empty() || !self.opens.schemes.is_empty() {
            let mut opens = Vec::new();
            if !self.opens.types.is_empty() {
                opens.push(("types", strings(&self.opens.types)));
            }
            if !self.opens.schemes.is_empty() {
                opens.push(("schemes", strings(&self.opens.schemes)));
            }
            members.push(("opens", json::object(opens)));
        }
        json::object(members)
    }
}

/// The files list: sorted by path, each path once even ignoring case, none reserved, none under
/// `native/`, none a file with files below it.
fn check_files(files: &[FileEntry]) -> Result<(), Error> {
    let paths: Vec<&PackagePath> = files.iter().map(|f| &f.path).collect();
    sorted_unique(&paths, "files")?;
    let mut folded = HashSet::new();
    for path in &paths {
        let lower = path.folded();
        if !folded.insert(lower.clone()) {
            return Err(Error::DuplicatePath(path.to_string()));
        }
        if RESERVED.contains(&lower.as_str()) {
            return Err(Error::ReservedPath(path.to_string()));
        }
        if lower == "native" || lower.starts_with("native/") {
            return Err(Error::NativeCode);
        }
    }
    for path in &paths {
        let mut ancestor = String::new();
        let components: Vec<&str> = path.components().collect();
        for component in &components[..components.len() - 1] {
            if !ancestor.is_empty() {
                ancestor.push('/');
            }
            ancestor.push_str(&component.to_ascii_lowercase());
            if folded.contains(&ancestor) {
                return Err(Error::PathConflict(ancestor));
            }
        }
    }
    Ok(())
}
