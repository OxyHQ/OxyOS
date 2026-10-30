//! `oxyos-pack`: makes an OxyOS application package (docs/adr/0030-installing-applications.mdx,
//! decision 1) from a directory of its files, for developers and the development registry.
//!
//! ```text
//! oxyos-pack key KEY                        a new development key (its 32-byte seed, in hex)
//! oxyos-pack pack DIR DESCRIPTION RUNTIME KEY
//! oxyos-pack compile DIR HERMESC
//! oxyos-pack entry DIR
//! oxyos-pack store-keys DIR                 a development store's keys (root ×3, targets, snapshot, timestamp)
//! oxyos-pack repository DIR STORE_KEYS      a new store repository, root 1 for those keys
//! oxyos-pack publish PACKAGE DIR STORE_KEYS DEVELOPER_KEY
//! ```
//!
//! `publish` publishes a packed package as a release in the repository at DIR, as the store
//! would once it approved it (docs/adr/0030-installing-applications.mdx, decisions 3 and 4): its
//! files as objects, its manifest as a target with the store's statement (signed by STORE_KEYS'
//! targets key), the developer's signature and DEVELOPER_KEY's public key, then the next
//! targets, snapshot and timestamp (`oxyos_install::repository`, what the install service's tests
//! publish with). For development and scenarios: the store's own keys are never on a machine.
//!
//! `pack` lists every file under DIR (but `manifest.json`, `manifest.sig` and `index.hbc` at its
//! top) with its size and SHA-256, adds them and the runtime's numbers (RUNTIME, a runtime's
//! `runtime.json`) to DESCRIPTION (the manifest's other fields, as a person writes them), writes
//! the manifest in its canonical form (`manifest.json`, what `oxyos_package::Manifest::parse`
//! reads back) and signs it with KEY (`manifest.sig`). A link, a device or a file that is not
//! readable refuses the package, as does a description `Manifest::parse` would refuse.
//!
//! `compile` does what `oxyos-installd` does on the machine, with its code (`oxyos_install::compile`):
//! checks the tree against its manifest and compiles the source bundle it names with the runtime's
//! compiler into `index.hbc` at the top (Hermes bytecode, which a package never carries: Hermes runs
//! bytecode as trusted input). `entry` prints the Desktop Entry `oxyos-installd` writes for the
//! package installed at DIR (`oxyos_install::install::entry_text`), for a development registry.

use oxyos_package::{
    open_directory, read_package, verify_tree, Manifest, SigningKey, HERMES_BYTECODE_MAGIC,
    RESERVED,
};
use serde_json::{Map, Number, Value};
use std::fmt::Write as _;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: oxyos-pack key KEY | pack DIR DESCRIPTION RUNTIME KEY | compile DIR HERMESC | entry DIR | store-keys DIR | repository DIR STORE_KEYS | publish PACKAGE DIR STORE_KEYS DEVELOPER_KEY";

/// The store's keys in a directory of seeds, as `store-keys` writes them.
const STORE_KEY_FILES: [&str; 6] = [
    "root-1",
    "root-2",
    "root-3",
    "targets",
    "snapshot",
    "timestamp",
];
/// How long a published document is valid, in days.
const VALID_DAYS: i64 = 365;

type Result<T> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["key", key] => new_key(Path::new(key)),
        ["pack", dir, description, runtime, key] => pack(
            Path::new(dir),
            Path::new(description),
            Path::new(runtime),
            Path::new(key),
        ),
        ["compile", dir, hermesc] => compile(Path::new(dir), Path::new(hermesc)),
        ["entry", dir] => entry(Path::new(dir)),
        ["store-keys", dir] => store_keys(Path::new(dir)),
        ["repository", dir, keys] => repository(Path::new(dir), Path::new(keys)),
        ["publish", package, dir, keys, developer] => publish(
            Path::new(package),
            Path::new(dir),
            Path::new(keys),
            Path::new(developer),
        ),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("oxyos-pack: {error}");
            ExitCode::FAILURE
        }
    }
}

/// A new key's seed, from the system's random source, written to a new file only its owner reads.
fn new_key(path: &Path) -> Result<()> {
    use ring::rand::SecureRandom;
    let mut seed = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut seed)
        .map_err(|_| "the system's random source failed".to_string())?;
    let mut text = String::new();
    for byte in seed {
        let _ = write!(text, "{byte:02x}");
    }
    text.push('\n');
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// The key whose seed `path` holds: 64 hex digits, a line's end allowed.
fn read_key(path: &Path) -> Result<SigningKey> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let hex = text.strip_suffix('\n').unwrap_or(&text);
    let bad = || format!("{}: not a key's seed (64 hex digits)", path.display());
    if hex.len() != 64 {
        return Err(bad());
    }
    let mut seed = [0u8; 32];
    for (i, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2).ok_or_else(bad)?, 16)
            .map_err(|_| bad())?;
    }
    Ok(SigningKey::from_seed(&seed))
}

/// A development store's keys: a seed for each, in a new directory only its owner reads.
fn store_keys(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(dir)
        .map_err(|error| format!("{}: {error}", dir.display()))?;
    for name in STORE_KEY_FILES {
        new_key(&dir.join(name))?;
    }
    Ok(())
}

fn read_store_keys(dir: &Path) -> Result<oxyos_install::repository::StoreKeys> {
    let key = |name: &str| read_key(&dir.join(name));
    Ok(oxyos_install::repository::StoreKeys {
        root: vec![key("root-1")?, key("root-2")?, key("root-3")?],
        root_threshold: 2,
        targets: key("targets")?,
        snapshot: key("snapshot")?,
        timestamp: key("timestamp")?,
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

/// A new repository at `dir`, root 1 for the store's keys in `keys`.
fn repository(dir: &Path, keys: &Path) -> Result<()> {
    let keys = read_store_keys(keys)?;
    let repository = oxyos_install::repository::Repository::create(dir, &keys, now(), VALID_DAYS)
        .map_err(|error| format!("{}: {error}", dir.display()))?;
    println!(
        "{}: the anchor is {}",
        dir.display(),
        repository.anchor().display()
    );
    Ok(())
}

/// Publishes the packed `package` as a release of the repository at `dir`.
fn publish(package: &Path, dir: &Path, keys: &Path, developer: &Path) -> Result<()> {
    let keys = read_store_keys(keys)?;
    let developer = read_key(developer)?.public_key();
    let mut repository = oxyos_install::repository::Repository::open(dir, now(), VALID_DAYS)
        .map_err(|error| format!("{}: {error}", dir.display()))?;
    let (id, version_code) = repository
        .publish_package(package, &keys, &developer, &[])
        .map_err(|error| format!("{}: {error}", package.display()))?;
    println!(
        "{id} {version_code} published: targets {}",
        repository.versions[oxyos_install::repository::TARGETS]
    );
    Ok(())
}

/// A JSON document, read refusing a key written twice in any object: JSON allows it, and which
/// one a reader keeps is the reader's choice.
fn read_json(path: &Path) -> Result<Value> {
    struct Strict;
    impl<'de> serde::de::DeserializeSeed<'de> for Strict {
        type Value = Value;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Value, D::Error> {
            deserializer.deserialize_any(self)
        }
    }
    impl<'de> serde::de::Visitor<'de> for Strict {
        type Value = Value;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("JSON")
        }
        fn visit_bool<E>(self, v: bool) -> std::result::Result<Value, E> {
            Ok(Value::Bool(v))
        }
        fn visit_u64<E>(self, v: u64) -> std::result::Result<Value, E> {
            Ok(Value::Number(v.into()))
        }
        fn visit_i64<E: serde::de::Error>(self, _: i64) -> std::result::Result<Value, E> {
            Err(E::custom("a manifest has no negative numbers"))
        }
        fn visit_f64<E: serde::de::Error>(self, _: f64) -> std::result::Result<Value, E> {
            Err(E::custom("a manifest has no fractions"))
        }
        fn visit_str<E>(self, v: &str) -> std::result::Result<Value, E> {
            Ok(Value::String(v.to_string()))
        }
        fn visit_unit<E>(self) -> std::result::Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Value, A::Error> {
            let mut out = Vec::new();
            while let Some(value) = seq.next_element_seed(Strict)? {
                out.push(value);
            }
            Ok(Value::Array(out))
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Value, A::Error> {
            let mut out = Map::new();
            while let Some(key) = map.next_key::<String>()? {
                if out.contains_key(&key) {
                    return Err(serde::de::Error::custom(format!(
                        "{key:?} is written twice"
                    )));
                }
                let value = map.next_value_seed(Strict)?;
                out.insert(key, value);
            }
            Ok(Value::Object(out))
        }
    }
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    let value = serde::de::DeserializeSeed::deserialize(Strict, &mut deserializer)
        .and_then(|value| deserializer.end().map(|()| value))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(value)
}

/// `value` in RFC 8785's canonical form, for the values a manifest has: members sorted by key
/// (UTF-16 code units, as JCS sorts), no space, strings escaped as JCS escapes them, integers.
fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(members) => {
            let mut keys: Vec<&String> = members.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                string(key, out);
                out.push(':');
                canonical(&members[key], out);
            }
            out.push('}');
        }
    }
}

fn string(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Every file under `dir`, by its path from `dir` (`/`-separated), sorted, but the reserved names
/// at its top; a link or anything but a file or a directory refuses the package.
fn files(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let mut pending = vec![(String::new(), dir.to_path_buf())];
    while let Some((prefix, directory)) = pending.pop() {
        let entries = std::fs::read_dir(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("{}: {error}", directory.display()))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|name| format!("{name:?} is not UTF-8"))?;
            let path = format!("{prefix}{name}");
            if prefix.is_empty() && RESERVED.contains(&name.as_str()) {
                continue;
            }
            let kind = entry
                .file_type()
                .map_err(|error| format!("{path}: {error}"))?;
            if kind.is_dir() {
                pending.push((format!("{path}/"), entry.path()));
            } else if kind.is_file() {
                out.push((path, entry.path()));
            } else {
                return Err(format!("{path}: not a file or a directory (a link?)"));
            }
        }
    }
    out.sort();
    Ok(out)
}

fn pack(dir: &Path, description: &Path, runtime: &Path, key: &Path) -> Result<()> {
    let Value::Object(mut manifest) = read_json(description)? else {
        return Err(format!("{}: not an object", description.display()));
    };
    for field in ["files", "runtime"] {
        if manifest.contains_key(field) {
            return Err(format!(
                "{}: `{field}` is the tool's to write",
                description.display()
            ));
        }
    }
    let Value::Object(level) = read_json(runtime)? else {
        return Err(format!(
            "{}: not a runtime's description",
            runtime.display()
        ));
    };
    let number = |name: &str| {
        level
            .get(name)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{}: no {name}", runtime.display()))
    };
    let mut runtime_members = Map::new();
    runtime_members.insert("abi".into(), Value::Number(number("abi")?.into()));
    runtime_members.insert("api".into(), Value::Number(number("api")?.into()));
    manifest.insert("runtime".into(), Value::Object(runtime_members));
    let mut listed = Vec::new();
    for (path, file) in files(dir)? {
        let bytes = std::fs::read(&file).map_err(|error| format!("{path}: {error}"))?;
        let digest = ring::digest::digest(&ring::digest::SHA256, &bytes);
        let mut hex = String::new();
        for byte in digest.as_ref() {
            let _ = write!(hex, "{byte:02x}");
        }
        let mut entry = Map::new();
        entry.insert("path".into(), Value::String(path));
        entry.insert(
            "size".into(),
            Value::Number(Number::from(bytes.len() as u64)),
        );
        entry.insert("sha256".into(), Value::String(hex));
        listed.push(Value::Object(entry));
    }
    manifest.insert("files".into(), Value::Array(listed));
    let mut text = String::new();
    canonical(&Value::Object(manifest), &mut text);
    // What the system will read: refused here as it would be there.
    let parsed =
        Manifest::parse(text.as_bytes()).map_err(|error| format!("the manifest: {error}"))?;
    if parsed.to_canonical() != text.as_bytes() {
        return Err("the manifest is not read back as written".into());
    }
    let key = read_key(key)?;
    let signature = key.sign_manifest(text.as_bytes());
    write_new(&dir.join("manifest.json"), text.as_bytes())?;
    write_new(&dir.join("manifest.sig"), &signature.to_canonical())?;
    // The package as the system reads it: its signature and its tree.
    let fd = open_directory(dir).map_err(|error| error.to_string())?;
    let package = read_package(fd.as_fd()).map_err(|error| error.to_string())?;
    package
        .signature
        .verify(&package.manifest_bytes, &key.public_key())
        .map_err(|error| error.to_string())?;
    verify_tree(fd.as_fd(), &package.manifest, &["index.hbc"])
        .map_err(|error| error.to_string())?;
    println!(
        "{} {} ({}): {} files, key {}",
        parsed.id,
        parsed.version,
        parsed.version_code,
        parsed.files.len(),
        key.public_key().key_id().as_str()
    );
    Ok(())
}

/// `bytes` into a new file at `path`, then renamed over it: a reader sees the old or the new.
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let new = path.with_extension("new");
    std::fs::write(&new, bytes)
        .and_then(|()| std::fs::rename(&new, path))
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn compile(dir: &Path, hermesc: &Path) -> Result<()> {
    use oxyos_install::compile::{Compile, Hermesc};
    let fd = open_directory(dir).map_err(|error| error.to_string())?;
    let package = read_package(fd.as_fd()).map_err(|error| error.to_string())?;
    verify_tree(fd.as_fd(), &package.manifest, &["index.hbc"])
        .map_err(|error| error.to_string())?;
    let source_path = dir.join(package.manifest.entry.bundle.as_str());
    let source = std::fs::File::open(&source_path)
        .map_err(|error| format!("{}: {error}", source_path.display()))?;
    let new = dir.join("index.hbc.new");
    let out = std::fs::File::create(&new).map_err(|error| format!("{}: {error}", new.display()))?;
    // As the install service compiles when it is not root: the compiler run directly.
    if let Err(error) = (Hermesc { confined: false }).compile(hermesc, &source, &out) {
        let _ = std::fs::remove_file(&new);
        return Err(error);
    }
    let bytecode = std::fs::read(&new).map_err(|error| format!("{}: {error}", new.display()))?;
    if !bytecode.starts_with(&HERMES_BYTECODE_MAGIC) {
        let _ = std::fs::remove_file(&new);
        return Err("the compiler's output is not Hermes bytecode".into());
    }
    std::fs::rename(&new, dir.join("index.hbc")).map_err(|error| error.to_string())?;
    println!(
        "{}: index.hbc, {} bytes",
        package.manifest.id,
        bytecode.len()
    );
    Ok(())
}

/// The entry `oxyos-installd` writes for the package installed at `dir` (its version's
/// directory), on the runtime it was built for.
fn entry(dir: &Path) -> Result<()> {
    let fd = open_directory(dir).map_err(|error| error.to_string())?;
    let package = read_package(fd.as_fd()).map_err(|error| error.to_string())?;
    let dir = dir
        .canonicalize()
        .map_err(|error| format!("{}: {error}", dir.display()))?;
    print!(
        "{}",
        oxyos_install::install::entry_text(&package.manifest, &dir, package.manifest.runtime.abi)
    );
    Ok(())
}

#[cfg(test)]
mod tests;
