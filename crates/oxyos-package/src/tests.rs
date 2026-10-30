use super::*;
use crate::json;
use rustix::fd::AsFd;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const DEVELOPER: [u8; 32] = [7; 32];
const NEXT: [u8; 32] = [8; 32];
const THIRD: [u8; 32] = [9; 32];
const STORE: [u8; 32] = [42; 32];
const APPLICATION: &str = "0192f1e0-7c3a-7d2e-8a41-5b6c7d8e9f01";

/// A directory of the test's own, removed when it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("oxyos-package-{}-{name}", std::process::id()));
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

/// The package's files (path, contents), besides the manifest.
fn contents() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("bundle/drawable-mdpi/logo.png", b"\x89PNG logo".to_vec()),
        (
            "bundle/index.js",
            b"AppRegistry.registerComponent('Notes', () => App);".to_vec(),
        ),
        ("icon.svg", b"<svg/>".to_vec()),
    ]
}

fn text(s: &str) -> Text {
    let Ok(manifest) = Manifest::parse(&manifest_with(|_| {}, s)) else {
        unreachable!()
    };
    manifest.name
}

/// A valid manifest's canonical bytes, its name `name`, after `change` on its JSON value.
fn manifest_with(change: impl FnOnce(&mut Vec<(String, json::Value)>), name: &str) -> Vec<u8> {
    let files = contents()
        .into_iter()
        .map(|(path, bytes)| {
            json::object(vec![
                ("path", json::string(path)),
                ("size", json::Value::Integer(bytes.len() as u64)),
                ("sha256", json::string(&Sha256::of(&bytes).hex())),
            ])
        })
        .collect();
    let json::Value::Object(mut members) = json::object(vec![
        ("id", json::string("so.oxy.notes")),
        ("application", json::string(APPLICATION)),
        ("version", json::string("1.2.0")),
        ("versionCode", json::Value::Integer(12)),
        ("name", json::string(name)),
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
                ("abi", json::Value::Integer(1)),
                ("api", json::Value::Integer(3)),
            ]),
        ),
        (
            "architectures",
            json::Value::Array(vec![json::string("any")]),
        ),
        (
            "capabilities",
            json::Value::Array(vec![
                json::object(vec![
                    ("name", json::string("network")),
                    (
                        "purpose",
                        json::object(vec![("en", json::string("Syncs your notes."))]),
                    ),
                ]),
                json::object(vec![
                    ("name", json::string("notifications")),
                    (
                        "purpose",
                        json::object(vec![("en", json::string("Reminders."))]),
                    ),
                ]),
            ]),
        ),
        ("files", json::Value::Array(files)),
    ]) else {
        unreachable!()
    };
    change(&mut members);
    json::canonical(&json::Value::Object(members))
}

fn manifest_bytes() -> Vec<u8> {
    manifest_with(|_| {}, "Notes")
}

fn set(members: &mut Vec<(String, json::Value)>, key: &str, value: json::Value) {
    members.retain(|(k, _)| k != key);
    members.push((key.to_string(), value));
}

fn files_value(files: Vec<(&str, u64, String)>) -> json::Value {
    json::Value::Array(
        files
            .into_iter()
            .map(|(path, size, sha)| {
                json::object(vec![
                    ("path", json::string(path)),
                    ("size", json::Value::Integer(size)),
                    ("sha256", json::string(&sha)),
                ])
            })
            .collect(),
    )
}

/// Writes a package (its files, manifest and signature) into `dir`.
fn write_package(dir: &Path, manifest: &[u8], key: &SigningKey) {
    for (path, bytes) in contents() {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fs::write(dir.join("manifest.json"), manifest).unwrap();
    fs::write(
        dir.join("manifest.sig"),
        key.sign_manifest(manifest).to_canonical(),
    )
    .unwrap();
}

fn verify_dir(dir: &Path) -> Result<Package, Error> {
    let fd = open_directory(dir)?;
    let package = read_package(fd.as_fd())?;
    verify_tree(fd.as_fd(), &package.manifest, &[])?;
    Ok(package)
}

// The manifest.

#[test]
fn a_manifest_reads_back_to_its_bytes() {
    let bytes = manifest_bytes();
    let manifest = Manifest::parse(&bytes).unwrap();
    assert_eq!(manifest.id, "so.oxy.notes");
    assert_eq!(manifest.version_code, 12);
    assert_eq!(manifest.runtime, Runtime { abi: 1, api: 3 });
    assert_eq!(manifest.names["es"].as_str(), "Notas");
    assert_eq!(manifest.entry.module, "Notes");
    assert_eq!(manifest.capabilities[0].capability, Capability::Network);
    assert_eq!(manifest.files.len(), 3);
    assert_eq!(manifest.to_canonical(), bytes);
    assert!(manifest.runtime.runs_on(&Runtime { abi: 1, api: 4 }));
    assert!(!manifest.runtime.runs_on(&Runtime { abi: 1, api: 2 }));
    assert!(!manifest.runtime.runs_on(&Runtime { abi: 2, api: 9 }));
    assert!(Capability::Notifications.asked_at_first_use());
    assert!(!Capability::Network.asked_at_first_use());
}

#[test]
fn only_the_canonical_form_is_read() {
    let bytes = manifest_bytes();
    let text = String::from_utf8(bytes).unwrap();
    // Whitespace, members out of order, an escape JCS does not write.
    let spaced = text.replacen(",", ", ", 1);
    assert_eq!(Manifest::parse(spaced.as_bytes()), Err(Error::NotCanonical));
    let reordered = text.replacen("{\"application\"", "{\"zz\":1,\"application\"", 1);
    assert!(matches!(
        Manifest::parse(reordered.as_bytes()),
        Err(Error::NotCanonical)
    ));
    let escaped = text.replacen("Notes\"", "Note\\u0073\"", 1);
    assert_eq!(
        Manifest::parse(escaped.as_bytes()),
        Err(Error::NotCanonical)
    );
}

#[test]
fn a_key_written_twice_is_refused() {
    let text = String::from_utf8(manifest_bytes()).unwrap();
    let twice = text.replacen(
        "{\"application\"",
        "{\"application\":\"x\",\"application\"",
        1,
    );
    assert_eq!(
        Manifest::parse(twice.as_bytes()),
        Err(Error::DuplicateKey("application".into()))
    );
    let in_names = text.replacen("\"es\":\"Notas\"", "\"es\":\"Notas\",\"es\":\"Otra\"", 1);
    assert_eq!(
        Manifest::parse(in_names.as_bytes()),
        Err(Error::DuplicateKey("es".into()))
    );
}

#[test]
fn unknown_fields_are_refused() {
    let bytes = manifest_with(|m| set(m, "zIndex", json::string("x")), "Notes");
    assert_eq!(
        Manifest::parse(&bytes),
        Err(Error::UnknownField {
            object: "the manifest",
            field: "zIndex".into()
        })
    );
    let bytes = manifest_with(
        |m| {
            set(
                m,
                "entry",
                json::object(vec![
                    ("bundle", json::string("bundle/index.js")),
                    ("module", json::string("Notes")),
                    ("script", json::string("x")),
                ]),
            )
        },
        "Notes",
    );
    assert!(matches!(
        Manifest::parse(&bytes),
        Err(Error::UnknownField {
            object: "entry",
            ..
        })
    ));
}

#[test]
fn numbers_are_integers_in_range() {
    for (value, error) in [
        (b"12.0".as_slice(), Error::Json(0)),
        (b"1e3", Error::Json(0)),
        (b"-1", Error::Json(0)),
        (b"012", Error::Json(0)),
        (b"9007199254740992", Error::Json(0)),
    ] {
        let bytes = String::from_utf8(manifest_bytes()).unwrap().replacen(
            "\"versionCode\":12",
            &format!("\"versionCode\":{}", String::from_utf8_lossy(value)),
            1,
        );
        assert!(
            matches!(Manifest::parse(bytes.as_bytes()), Err(Error::Json(_))),
            "{value:?}: {error:?}"
        );
    }
    let zero = manifest_with(|m| set(m, "versionCode", json::Value::Integer(0)), "Notes");
    assert_eq!(
        Manifest::parse(&zero),
        Err(Error::OutOfRange("versionCode"))
    );
    let string = manifest_with(|m| set(m, "versionCode", json::string("12")), "Notes");
    assert_eq!(
        Manifest::parse(&string),
        Err(Error::WrongType("versionCode"))
    );
    let abi = manifest_with(
        |m| {
            set(
                m,
                "runtime",
                json::object(vec![
                    ("abi", json::Value::Integer(0)),
                    ("api", json::Value::Integer(0)),
                ]),
            )
        },
        "Notes",
    );
    assert_eq!(Manifest::parse(&abi), Err(Error::OutOfRange("runtime.abi")));
    let big = manifest_with(
        |m| {
            set(
                m,
                "runtime",
                json::object(vec![
                    ("abi", json::Value::Integer(1 << 40)),
                    ("api", json::Value::Integer(0)),
                ]),
            )
        },
        "Notes",
    );
    assert_eq!(Manifest::parse(&big), Err(Error::OutOfRange("runtime.abi")));
}

#[test]
fn ids_names_and_texts_are_checked() {
    for id in [
        "notes",
        "So.Oxy.Notes",
        "so..notes",
        ".so.notes",
        "so.oxy.notes.",
        "so.oxy/notes",
        "so.oxy.nötes",
    ] {
        let bytes = manifest_with(|m| set(m, "id", json::string(id)), "Notes");
        assert_eq!(
            Manifest::parse(&bytes),
            Err(Error::InvalidId(id.into())),
            "{id}"
        );
    }
    for app in [
        "",
        "abc",
        "0192F1E0-7C3A-7D2E-8A41-5B6C7D8E9F01",
        "0192f1e07c3a7d2e8a415b6c7d8e9f01",
    ] {
        let bytes = manifest_with(|m| set(m, "application", json::string(app)), "Notes");
        assert_eq!(
            Manifest::parse(&bytes),
            Err(Error::InvalidApplication(app.into())),
            "{app}"
        );
    }
    let legacy = manifest_with(
        |m| set(m, "application", json::string("64f0c0ffee0123456789abcd")),
        "Notes",
    );
    assert!(Manifest::parse(&legacy).is_ok());
    for name in ["", " Notes", "Notes\n", "Notes\u{202e}", &"x".repeat(129)] {
        assert_eq!(
            Manifest::parse(&manifest_with(|_| {}, name)),
            Err(Error::Invalid("name")),
            "{name:?}"
        );
    }
    assert_eq!(text("Notas rápidas").as_str(), "Notas rápidas");
    let module = manifest_with(
        |m| {
            set(
                m,
                "entry",
                json::object(vec![
                    ("bundle", json::string("bundle/index.js")),
                    ("module", json::string("9x")),
                ]),
            )
        },
        "Notes",
    );
    assert_eq!(
        Manifest::parse(&module),
        Err(Error::Invalid("entry.module"))
    );
    let language = manifest_with(
        |m| set(m, "names", json::object(vec![("e", json::string("x"))])),
        "Notes",
    );
    assert_eq!(
        Manifest::parse(&language),
        Err(Error::Invalid("a language tag"))
    );
    let empty = manifest_with(|m| set(m, "names", json::object(vec![])), "Notes");
    assert_eq!(Manifest::parse(&empty), Err(Error::EmptyOptional("names")));
}

#[test]
fn paths_cannot_leave_the_package_or_mean_two_files() {
    let digest = Sha256::of(b"x").hex();
    for path in [
        "/etc/passwd",
        "../x",
        "bundle/../x",
        "./x",
        "bundle//x",
        "bundle/",
        "a b",
        "a\\b",
        "ñ",
        "a\u{0}b",
    ] {
        let bytes = manifest_with(
            |m| set(m, "files", files_value(vec![(path, 1, digest.clone())])),
            "Notes",
        );
        assert_eq!(
            Manifest::parse(&bytes),
            Err(Error::InvalidPath(path.into())),
            "{path:?}"
        );
    }
    let long = "a/".repeat(20) + "x";
    let bytes = manifest_with(
        |m| set(m, "files", files_value(vec![(&long, 1, digest.clone())])),
        "Notes",
    );
    assert!(matches!(
        Manifest::parse(&bytes),
        Err(Error::InvalidPath(_))
    ));

    let with = |files: Vec<(&str, u64, String)>| {
        let mut all = vec![
            ("bundle/index.js", 1, digest.clone()),
            ("icon.svg", 1, digest.clone()),
        ];
        all.extend(files);
        all.sort_by(|a, b| a.0.cmp(b.0));
        Manifest::parse(&manifest_with(
            |m| set(m, "files", files_value(all)),
            "Notes",
        ))
    };
    assert_eq!(
        with(vec![("Icon.svg", 1, digest.clone())]),
        Err(Error::DuplicatePath("icon.svg".into()))
    );
    assert_eq!(
        with(vec![("bundle", 1, digest.clone())]),
        Err(Error::PathConflict("bundle".into()))
    );
    assert_eq!(
        with(vec![("manifest.json", 1, digest.clone())]),
        Err(Error::ReservedPath("manifest.json".into()))
    );
    assert_eq!(
        with(vec![("Index.hbc", 1, digest.clone())]),
        Err(Error::ReservedPath("Index.hbc".into()))
    );
    assert_eq!(
        with(vec![("native/x86_64/libnotes.so", 1, digest.clone())]),
        Err(Error::NativeCode)
    );
    assert_eq!(
        with(vec![("x", MAX_FILE_SIZE + 1, digest.clone())]),
        Err(Error::TooLarge("a file"))
    );
    // Out of order.
    let unsorted = files_value(vec![
        ("icon.svg", 1, digest.clone()),
        ("bundle/index.js", 1, digest.clone()),
    ]);
    assert_eq!(
        Manifest::parse(&manifest_with(|m| set(m, "files", unsorted), "Notes")),
        Err(Error::NotSorted("files"))
    );
    let bad_digest = files_value(vec![
        ("bundle/index.js", 1, digest.to_uppercase()),
        ("icon.svg", 1, digest.clone()),
    ]);
    assert_eq!(
        Manifest::parse(&manifest_with(|m| set(m, "files", bad_digest), "Notes")),
        Err(Error::Invalid("a file's sha256"))
    );
}

#[test]
fn the_entry_and_icon_are_listed_files() {
    let bytes = manifest_with(|m| set(m, "icon", json::string("other.png")), "Notes");
    assert_eq!(
        Manifest::parse(&bytes),
        Err(Error::NotListed {
            field: "icon",
            path: "other.png".into()
        })
    );
    let bytes = manifest_with(
        |m| {
            set(
                m,
                "entry",
                json::object(vec![
                    ("bundle", json::string("bundle/main.js")),
                    ("module", json::string("Notes")),
                ]),
            )
        },
        "Notes",
    );
    assert!(matches!(
        Manifest::parse(&bytes),
        Err(Error::NotListed {
            field: "entry.bundle",
            ..
        })
    ));
    let bytes = manifest_with(
        |m| {
            set(
                m,
                "entry",
                json::object(vec![
                    ("bundle", json::string("bundle/drawable-mdpi/logo.png")),
                    ("module", json::string("Notes")),
                ]),
            )
        },
        "Notes",
    );
    assert_eq!(Manifest::parse(&bytes), Err(Error::Invalid("entry.bundle")));
}

#[test]
fn native_code_is_not_accepted() {
    for architectures in [
        vec!["x86_64"],
        vec!["any", "aarch64"],
        vec!["aarch64", "x86_64"],
    ] {
        let value = json::Value::Array(architectures.iter().map(|a| json::string(a)).collect());
        let bytes = manifest_with(|m| set(m, "architectures", value), "Notes");
        assert_eq!(
            Manifest::parse(&bytes),
            Err(Error::NativeCode),
            "{architectures:?}"
        );
    }
    let bytes = manifest_with(
        |m| {
            set(
                m,
                "architectures",
                json::Value::Array(vec![json::string("riscv64")]),
            )
        },
        "Notes",
    );
    assert_eq!(
        Manifest::parse(&bytes),
        Err(Error::UnknownArchitecture("riscv64".into()))
    );
}

fn capability(name: &str) -> json::Value {
    json::object(vec![
        ("name", json::string(name)),
        (
            "purpose",
            json::object(vec![("en", json::string("Because."))]),
        ),
    ])
}

#[test]
fn system_grants_are_never_a_packages() {
    for grant in SYSTEM_GRANTS {
        let bytes = manifest_with(
            |m| {
                set(
                    m,
                    "capabilities",
                    json::Value::Array(vec![capability(grant)]),
                )
            },
            "Notes",
        );
        assert_eq!(
            Manifest::parse(&bytes),
            Err(Error::SystemGrant(grant.into())),
            "{grant}"
        );
    }
    let bytes = manifest_with(
        |m| {
            set(
                m,
                "capabilities",
                json::Value::Array(vec![capability("root")]),
            )
        },
        "Notes",
    );
    assert_eq!(
        Manifest::parse(&bytes),
        Err(Error::UnknownCapability("root".into()))
    );
}

#[test]
fn capabilities_are_sorted_with_a_purpose_and_agree_with_opens() {
    let twice = json::Value::Array(vec![capability("network"), capability("network")]);
    assert_eq!(
        Manifest::parse(&manifest_with(|m| set(m, "capabilities", twice), "Notes")),
        Err(Error::NotSorted("capabilities"))
    );
    let no_purpose = json::Value::Array(vec![json::object(vec![
        ("name", json::string("camera")),
        ("purpose", json::object(vec![])),
    ])]);
    assert!(matches!(
        Manifest::parse(&manifest_with(
            |m| set(m, "capabilities", no_purpose),
            "Notes"
        )),
        Err(Error::MissingField {
            field: "purpose",
            ..
        })
    ));
    let empty = json::Value::Array(vec![]);
    assert_eq!(
        Manifest::parse(&manifest_with(|m| set(m, "capabilities", empty), "Notes")),
        Err(Error::EmptyOptional("capabilities"))
    );
    // `opens` without its capability, the capability without `opens`, and both.
    let opens = json::object(vec![(
        "types",
        json::Value::Array(vec![json::string("text/plain")]),
    )]);
    assert_eq!(
        Manifest::parse(&manifest_with(|m| set(m, "opens", opens.clone()), "Notes")),
        Err(Error::OpensMismatch("opens.types"))
    );
    let open_files = json::Value::Array(vec![capability("network"), capability("open-files")]);
    assert_eq!(
        Manifest::parse(&manifest_with(
            |m| set(m, "capabilities", open_files.clone()),
            "Notes"
        )),
        Err(Error::OpensMismatch("opens.types"))
    );
    let both = manifest_with(
        |m| {
            set(m, "capabilities", open_files);
            set(m, "opens", opens);
        },
        "Notes",
    );
    let manifest = Manifest::parse(&both).unwrap();
    assert_eq!(manifest.opens.types, ["text/plain"]);
    assert_eq!(manifest.to_canonical(), both);
    let empty_types = json::object(vec![("types", json::Value::Array(vec![]))]);
    assert_eq!(
        Manifest::parse(&manifest_with(|m| set(m, "opens", empty_types), "Notes")),
        Err(Error::EmptyOptional("opens.types"))
    );
}

#[test]
fn json_limits_hold() {
    let deep = "[".repeat(40) + &"]".repeat(40);
    assert_eq!(Manifest::parse(deep.as_bytes()), Err(Error::TooDeep));
    let huge = vec![b' '; MAX_MANIFEST_SIZE + 1];
    assert_eq!(Manifest::parse(&huge), Err(Error::TooLarge("the manifest")));
    for bad in [
        &b"{\"a\":\"\xff\"}"[..],
        b"{\"a\":\"\\ud800\"}",
        b"{\"a\":null}",
        b"{\"a\":\"\x01\"}",
        b"{\"a\":1} x",
        b"",
    ] {
        assert!(
            matches!(Manifest::parse(bad), Err(Error::Json(_))),
            "{bad:?}"
        );
    }
    // A lone surrogate pair spelled right is a character; canonical writes it as UTF-8.
    assert_eq!(
        json::parse(b"\"\\ud83d\\ude00\"").unwrap(),
        json::Value::String("😀".into())
    );
    // JCS sorts keys by UTF-16 code units: U+1F600, a surrogate pair from 0xD83D, before U+FB33.
    let sorted = json::canonical(&json::object(vec![
        ("😀", json::Value::Integer(1)),
        ("\u{fb33}", json::Value::Integer(2)),
    ]));
    assert_eq!(
        String::from_utf8(sorted).unwrap(),
        "{\"😀\":1,\"\u{fb33}\":2}"
    );
    assert_eq!(
        json::canonical(&json::string("a\u{1f}\"\\\n")),
        b"\"a\\u001f\\\"\\\\\\n\""
    );
}

// Signatures.

#[test]
fn the_developers_signature_verifies_over_the_exact_bytes() {
    let key = SigningKey::from_seed(&DEVELOPER);
    let bytes = manifest_bytes();
    let signature = key.sign_manifest(&bytes);
    let parsed = DeveloperSignature::parse(&signature.to_canonical()).unwrap();
    assert_eq!(parsed, signature);
    parsed.verify(&bytes, &key.public_key()).unwrap();
    // Other bytes, another key, a key it does not name.
    let other = manifest_with(|_| {}, "Notas");
    assert_eq!(
        parsed.verify(&other, &key.public_key()),
        Err(Error::BadSignature)
    );
    let stranger = SigningKey::from_seed(&NEXT).public_key();
    assert_eq!(parsed.verify(&bytes, &stranger), Err(Error::KeyMismatch));
    let mut forged = parsed.clone();
    forged.key_id = stranger.key_id();
    assert_eq!(forged.verify(&bytes, &stranger), Err(Error::BadSignature));
    let mut flipped = parsed.clone();
    flipped.signature[10] ^= 1;
    assert_eq!(
        flipped.verify(&bytes, &key.public_key()),
        Err(Error::BadSignature)
    );
}

#[test]
fn a_signature_is_bound_to_its_kind_of_document() {
    // The developer's key signing a statement-shaped document does not make it a store
    // statement, nor a manifest signature a rotation: each is over its own context.
    let developer = SigningKey::from_seed(&DEVELOPER);
    let bytes = manifest_bytes();
    let manifest = Manifest::parse(&bytes).unwrap();
    let signature = developer.sign_manifest(&bytes);
    let statement = ReleaseStatement::of(&manifest, &bytes, Architecture::Any, &signature);
    let release = developer.sign_release(statement.clone());
    let as_manifest_sig = DeveloperSignature {
        key_id: release.key_id.clone(),
        signature: release.signature,
    };
    assert_eq!(
        as_manifest_sig.verify(
            &json::canonical(&statement.to_value()),
            &developer.public_key()
        ),
        Err(Error::BadSignature)
    );
}

#[test]
fn signature_files_are_strict() {
    let key = SigningKey::from_seed(&DEVELOPER);
    let good = String::from_utf8(key.sign_manifest(&manifest_bytes()).to_canonical()).unwrap();
    let rsa = good.replacen("ed25519", "rsa-v1_5-sha256", 1);
    assert_eq!(
        DeveloperSignature::parse(rsa.as_bytes()),
        Err(Error::UnsupportedAlgorithm("rsa-v1_5-sha256".into()))
    );
    let padded = good.replacen("\"}", "==\"}", 1);
    assert_eq!(
        DeveloperSignature::parse(padded.as_bytes()),
        Err(Error::InvalidKey)
    );
    let short = good.replacen("\"keyId\":\"", "\"keyId\":\"A", 1);
    assert_eq!(
        DeveloperSignature::parse(short.as_bytes()),
        Err(Error::InvalidKey)
    );
    // A signature whose last character carries bits past its 64 bytes: a second spelling.
    let signature_at = good.find("\"signature\":\"").unwrap() + 13;
    let last = good[signature_at..].find('"').unwrap() + signature_at - 1;
    let mut bytes = good.clone().into_bytes();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let position = alphabet.iter().position(|c| *c == bytes[last]).unwrap();
    bytes[last] = alphabet[position ^ 1];
    assert_eq!(DeveloperSignature::parse(&bytes), Err(Error::InvalidKey));
    let extra = good.replacen("{", "{\"aaa\":\"x\",", 1);
    assert!(matches!(
        DeveloperSignature::parse(extra.as_bytes()),
        Err(Error::UnknownField { .. })
    ));
}

#[test]
fn key_ids_and_keys_round_trip() {
    let key = SigningKey::from_seed(&DEVELOPER).public_key();
    assert_eq!(PublicKey::parse(&key.encode()).unwrap(), key);
    assert_eq!(KeyId::parse(key.key_id().as_str()).unwrap(), key.key_id());
    assert_eq!(key.key_id().as_str().len(), 43);
    assert_ne!(
        key.key_id(),
        SigningKey::from_seed(&NEXT).public_key().key_id()
    );
    assert_eq!(PublicKey::parse("AAAA"), Err(Error::InvalidKey));
}

// The lineage.

#[test]
fn a_lineage_hands_over_from_the_pinned_key() {
    let first = SigningKey::from_seed(&DEVELOPER);
    let second = SigningKey::from_seed(&NEXT);
    let third = SigningKey::from_seed(&THIRD);
    let lineage = Lineage(vec![
        first
            .rotate_to(&second.public_key(), Some("2026-10-30T12:00:00Z"))
            .unwrap(),
        second.rotate_to(&third.public_key(), None).unwrap(),
    ]);
    let lineage = Lineage::parse(&lineage.to_canonical()).unwrap();
    // From the first, the second or the third key pinned, the current key is the third.
    for pinned in [&first, &second, &third] {
        assert_eq!(
            lineage_current(&pinned.public_key(), &lineage),
            Ok(third.public_key())
        );
        assert_eq!(
            lineage_accepts(&pinned.public_key(), &lineage, &third.public_key().key_id()),
            Ok(third.public_key())
        );
    }
    // A key the lineage moved on from no longer signs.
    assert_eq!(
        lineage_accepts(&first.public_key(), &lineage, &second.public_key().key_id()),
        Err(Error::NotCurrentKey)
    );
    // No lineage: the pinned key alone.
    assert_eq!(
        lineage_accepts(
            &first.public_key(),
            &Lineage::default(),
            &first.public_key().key_id()
        ),
        Ok(first.public_key())
    );
    assert_eq!(
        lineage_accepts(
            &first.public_key(),
            &Lineage::default(),
            &second.public_key().key_id()
        ),
        Err(Error::NotCurrentKey)
    );
}

#[test]
fn a_lineage_is_refused_when_broken() {
    let first = SigningKey::from_seed(&DEVELOPER);
    let second = SigningKey::from_seed(&NEXT);
    let third = SigningKey::from_seed(&THIRD);
    let stranger = SigningKey::from_seed(&STORE);
    // The pinned key is not in it.
    let lineage = Lineage(vec![second.rotate_to(&third.public_key(), None).unwrap()]);
    assert_eq!(
        lineage_current(&stranger.public_key(), &lineage),
        Err(Error::NotInLineage)
    );
    // A gap: the second rotation is not from the key the first handed over to.
    let gap = Lineage(vec![
        first.rotate_to(&second.public_key(), None).unwrap(),
        stranger.rotate_to(&third.public_key(), None).unwrap(),
    ]);
    assert_eq!(
        lineage_current(&first.public_key(), &gap),
        Err(Error::LineageGap)
    );
    // A cycle: back to the first key.
    let cycle = Lineage(vec![
        first.rotate_to(&second.public_key(), None).unwrap(),
        second.rotate_to(&first.public_key(), None).unwrap(),
    ]);
    assert_eq!(
        lineage_current(&first.public_key(), &cycle),
        Err(Error::LineageCycle)
    );
    // A rotation that claims the pinned key's id but is signed by another key.
    let mut forged = stranger.rotate_to(&third.public_key(), None).unwrap();
    forged.previous_key_id = first.public_key().key_id();
    assert_eq!(
        lineage_current(&first.public_key(), &Lineage(vec![forged])),
        Err(Error::BadSignature)
    );
    // A rotation's next key changed after signing.
    let mut changed = first.rotate_to(&second.public_key(), None).unwrap();
    changed.next_key = stranger.public_key();
    assert_eq!(
        lineage_current(&first.public_key(), &Lineage(vec![changed])),
        Err(Error::BadSignature)
    );
    assert_eq!(
        first.rotate_to(&second.public_key(), Some("yesterday")),
        Err(Error::Invalid("signedAt"))
    );
}

// The store's statement.

fn signed_release(store: &SigningKey) -> (Vec<u8>, Manifest, DeveloperSignature, SignedRelease) {
    let bytes = manifest_bytes();
    let manifest = Manifest::parse(&bytes).unwrap();
    let signature = SigningKey::from_seed(&DEVELOPER).sign_manifest(&bytes);
    let statement = ReleaseStatement::of(&manifest, &bytes, Architecture::Any, &signature);
    (bytes, manifest, signature, store.sign_release(statement))
}

#[test]
fn the_stores_statement_verifies_and_matches() {
    let store = SigningKey::from_seed(&STORE);
    let (bytes, manifest, signature, release) = signed_release(&store);
    let statement = SignedRelease::verify(&release.to_canonical(), &[store.public_key()]).unwrap();
    assert_eq!(statement, release.statement);
    statement.matches(&manifest, &bytes, &signature).unwrap();
}

#[test]
fn the_stores_statement_is_refused_unless_trusted_and_matching() {
    let store = SigningKey::from_seed(&STORE);
    let (bytes, manifest, signature, release) = signed_release(&store);
    let other_store = SigningKey::from_seed(&THIRD).public_key();
    assert_eq!(
        SignedRelease::verify(&release.to_canonical(), &[other_store]),
        Err(Error::UntrustedStoreKey)
    );
    assert_eq!(
        SignedRelease::verify(&release.to_canonical(), &[]),
        Err(Error::UntrustedStoreKey)
    );
    // A statement changed after signing.
    let text = String::from_utf8(release.to_canonical()).unwrap();
    let changed = text.replacen("\"versionCode\":12", "\"versionCode\":13", 1);
    assert_eq!(
        SignedRelease::verify(changed.as_bytes(), &[store.public_key()]),
        Err(Error::BadSignature)
    );
    // Statements for another package, version, digest, length, runtime, architecture or key.
    let check = |change: &dyn Fn(&mut ReleaseStatement), field| {
        let mut statement = release.statement.clone();
        change(&mut statement);
        let resigned = store.sign_release(statement);
        let verified =
            SignedRelease::verify(&resigned.to_canonical(), &[store.public_key()]).unwrap();
        assert_eq!(
            verified.matches(&manifest, &bytes, &signature),
            Err(Error::ReleaseMismatch(field))
        );
    };
    check(&|s| s.package_id = "so.oxy.other".into(), "packageId");
    check(&|s| s.version_code = 11, "versionCode");
    check(
        &|s| s.manifest_sha256 = Sha256::of(b"other"),
        "manifestSha256",
    );
    check(&|s| s.manifest_length += 1, "manifestLength");
    check(&|s| s.runtime.api = 4, "runtime");
    check(&|s| s.architecture = Architecture::X86_64, "architecture");
    check(
        &|s| s.developer_key_id = SigningKey::from_seed(&NEXT).public_key().key_id(),
        "developerKeyId",
    );
}

// The tree on disk.

#[test]
fn a_package_written_whole_verifies() {
    let scratch = Scratch::new("whole");
    let key = SigningKey::from_seed(&DEVELOPER);
    write_package(&scratch.0, &manifest_bytes(), &key);
    let package = verify_dir(&scratch.0).unwrap();
    package
        .signature
        .verify(&package.manifest_bytes, &key.public_key())
        .unwrap();
    // The compiled bytecode beside the manifest, when asked for.
    fs::write(scratch.0.join("index.hbc"), HERMES_BYTECODE_MAGIC).unwrap();
    let fd = open_directory(&scratch.0).unwrap();
    assert_eq!(
        verify_tree(fd.as_fd(), &package.manifest, &[]),
        Err(Error::Unlisted("index.hbc".into()))
    );
    verify_tree(fd.as_fd(), &package.manifest, &["index.hbc"]).unwrap();
    assert_eq!(
        verify_tree(fd.as_fd(), &package.manifest, &["extra"]),
        Err(Error::ReservedPath("extra".into()))
    );
}

#[test]
fn a_tree_that_differs_from_its_manifest_is_refused() {
    let key = SigningKey::from_seed(&DEVELOPER);
    let case = |name: &str, change: &dyn Fn(&Path)| {
        let scratch = Scratch::new(name);
        write_package(&scratch.0, &manifest_bytes(), &key);
        change(&scratch.0);
        verify_dir(&scratch.0).err()
    };
    assert_eq!(
        case("changed", &|d| fs::write(d.join("icon.svg"), b"<svg!>")
            .unwrap()),
        Some(Error::DigestMismatch("icon.svg".into()))
    );
    assert_eq!(
        case("longer", &|d| fs::write(d.join("icon.svg"), b"<svg/> ")
            .unwrap()),
        Some(Error::SizeMismatch("icon.svg".into()))
    );
    assert_eq!(
        case("missing", &|d| fs::remove_file(
            d.join("bundle/drawable-mdpi/logo.png")
        )
        .unwrap()),
        Some(Error::Missing("bundle/drawable-mdpi/logo.png".into()))
    );
    assert_eq!(
        case("unlisted", &|d| fs::write(d.join("bundle/extra.js"), b"x")
            .unwrap()),
        Some(Error::Unlisted("bundle/extra.js".into()))
    );
    assert_eq!(
        case("empty-dir", &|d| fs::create_dir(d.join("assets")).unwrap()),
        Some(Error::Unlisted("assets".into()))
    );
    assert_eq!(
        case("linked-file", &|d| {
            fs::remove_file(d.join("icon.svg")).unwrap();
            std::os::unix::fs::symlink("/etc/hostname", d.join("icon.svg")).unwrap();
        }),
        Some(Error::Link("icon.svg".into()))
    );
    assert_eq!(
        case("linked-dir", &|d| {
            let real = d.join("real");
            fs::rename(d.join("bundle/drawable-mdpi"), &real).unwrap();
            std::os::unix::fs::symlink(&real, d.join("bundle/drawable-mdpi")).unwrap();
        }),
        Some(Error::Link("bundle/drawable-mdpi".into()))
    );
    assert_eq!(
        case("fifo", &|d| {
            rustix::fs::mknodat(
                rustix::fs::CWD,
                d.join("pipe"),
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
                0,
            )
            .unwrap();
        }),
        Some(Error::NotRegular("pipe".into()))
    );
    assert_eq!(
        case("unlisted-case", &|d| fs::rename(
            d.join("icon.svg"),
            d.join("Icon.svg")
        )
        .unwrap()),
        Some(Error::Unlisted("Icon.svg".into()))
    );
}

#[test]
fn a_linked_package_directory_is_refused() {
    let scratch = Scratch::new("linked-root");
    let real = scratch.0.join("real");
    fs::create_dir(&real).unwrap();
    write_package(&real, &manifest_bytes(), &SigningKey::from_seed(&DEVELOPER));
    std::os::unix::fs::symlink(&real, scratch.0.join("package")).unwrap();
    assert!(matches!(
        open_directory(&scratch.0.join("package")),
        Err(Error::Link(_))
    ));
    // A linked manifest is not read either.
    let other = scratch.0.join("other");
    fs::create_dir(&other).unwrap();
    std::os::unix::fs::symlink(real.join("manifest.json"), other.join("manifest.json")).unwrap();
    let fd = open_directory(&other).unwrap();
    assert!(matches!(read_package(fd.as_fd()), Err(Error::Link(_))));
}

#[test]
fn bytecode_as_the_entry_is_refused() {
    let scratch = Scratch::new("bytecode");
    let key = SigningKey::from_seed(&DEVELOPER);
    let mut bytecode = HERMES_BYTECODE_MAGIC.to_vec();
    bytecode.extend_from_slice(b"rest of a bytecode file");
    let manifest = manifest_with(
        |m| {
            let mut files: Vec<(&str, u64, String)> = contents()
                .into_iter()
                .filter(|(p, _)| *p != "bundle/index.js")
                .map(|(p, b)| (p, b.len() as u64, Sha256::of(&b).hex()))
                .collect();
            files.push((
                "bundle/index.js",
                bytecode.len() as u64,
                Sha256::of(&bytecode).hex(),
            ));
            files.sort_by(|a, b| a.0.cmp(b.0));
            set(m, "files", files_value(files));
        },
        "Notes",
    );
    write_package(&scratch.0, &manifest, &key);
    fs::write(scratch.0.join("bundle/index.js"), &bytecode).unwrap();
    assert_eq!(verify_dir(&scratch.0).err(), Some(Error::Bytecode));
}

#[test]
fn streams_are_checked_and_stop_when_too_long() {
    let bytes = b"0123456789".to_vec();
    let digest = Sha256::of(&bytes);
    let mut out = Vec::new();
    copy_verified(&mut bytes.as_slice(), &mut out, 10, &digest, "f").unwrap();
    assert_eq!(out, bytes);
    assert_eq!(
        verify_stream(&mut bytes.as_slice(), 9, &digest, "f"),
        Err(Error::SizeMismatch("f".into()))
    );
    assert_eq!(
        verify_stream(&mut bytes.as_slice(), 11, &digest, "f"),
        Err(Error::SizeMismatch("f".into()))
    );
    assert_eq!(
        verify_stream(&mut bytes.as_slice(), 10, &Sha256::of(b"x"), "f"),
        Err(Error::DigestMismatch("f".into()))
    );
    // An endless stream stops one chunk past the size, not at its end.
    struct Endless(u64);
    impl std::io::Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0 += buf.len() as u64;
            buf.fill(b'x');
            Ok(buf.len())
        }
    }
    let mut endless = Endless(0);
    assert_eq!(
        verify_stream(&mut endless, 100, &digest, "f"),
        Err(Error::SizeMismatch("f".into()))
    );
    assert!(endless.0 <= 101);
}

// Mutations: whatever is done to a valid manifest or signature, nothing panics, and a changed
// manifest never verifies against the signature over the original.

/// xorshift64*: deterministic, so a failure is reproduced by its iteration.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn mutate(random: &mut Random, bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for _ in 0..=random.below(3) {
        let at = random.below(out.len().max(1));
        match random.below(4) {
            0 if !out.is_empty() => out[at] ^= 1 << random.below(8),
            1 => out.insert(at.min(out.len()), b"\"{}[],:0aZ\\ \xff"[random.below(13)]),
            2 if !out.is_empty() => {
                out.remove(at);
            }
            _ if !out.is_empty() => out[at] = random.next() as u8,
            _ => out.push(b'{'),
        }
    }
    out
}

#[test]
fn mutated_manifests_never_panic_nor_verify() {
    let key = SigningKey::from_seed(&DEVELOPER);
    let original = manifest_bytes();
    let signature = key.sign_manifest(&original);
    let signature_bytes = signature.to_canonical();
    let mut random = Random(0x9e37_79b9_7f4a_7c15);
    let mut parsed = 0;
    for _ in 0..20_000 {
        let mutated = mutate(&mut random, &original);
        if let Ok(manifest) = Manifest::parse(&mutated) {
            parsed += 1;
            // What parses is its own canonical form.
            assert_eq!(manifest.to_canonical(), mutated);
        }
        if mutated != original {
            assert_eq!(
                signature.verify(&mutated, &key.public_key()),
                Err(Error::BadSignature)
            );
        }
        let mutated_signature = mutate(&mut random, &signature_bytes);
        if let Ok(other) = DeveloperSignature::parse(&mutated_signature) {
            if other != signature {
                assert!(other.verify(&original, &key.public_key()).is_err());
            }
        }
        let _ = Lineage::parse(&mutated_signature);
        let _ = SignedRelease::verify(&mutated, &[key.public_key()]);
    }
    // Some mutations stay manifests (a changed letter in a name); none verified.
    assert!(parsed > 0, "no mutation parsed");
}

#[test]
fn mutated_statements_never_verify() {
    let store = SigningKey::from_seed(&STORE);
    let (_, _, _, release) = signed_release(&store);
    let original = release.to_canonical();
    let mut random = Random(0x0123_4567_89ab_cdef);
    for _ in 0..10_000 {
        let mutated = mutate(&mut random, &original);
        if let Ok(statement) = SignedRelease::verify(&mutated, &[store.public_key()]) {
            // Only a change outside what is signed and checked could pass, and there is none.
            assert_eq!(
                statement,
                release.statement,
                "{}",
                String::from_utf8_lossy(&mutated)
            );
        }
    }
}

#[test]
fn texts_by_language_are_kept_sorted() {
    let manifest = Manifest::parse(&manifest_with(
        |m| {
            set(
                m,
                "names",
                json::object(vec![
                    ("pt-BR", json::string("Notas")),
                    ("es", json::string("Notas")),
                ]),
            )
        },
        "Notes",
    ))
    .unwrap();
    let languages: Vec<&String> = manifest.names.keys().collect();
    assert_eq!(languages, ["es", "pt-BR"]);
    let mut expected = BTreeMap::new();
    expected.insert("en".to_string(), "Syncs your notes.");
    assert_eq!(
        manifest.capabilities[0]
            .purpose
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str()))
            .collect::<BTreeMap<_, _>>(),
        expected
    );
}

// ---- a runtime's description (runtime.json; ADR 0030, decisions 2 and 17) ---------------------

#[test]
fn a_runtimes_description_is_read_strictly() {
    let digest = "0".repeat(64);
    assert_eq!(
        RuntimeDescription::parse(br#"{"abi":1,"api":3}"#),
        Ok(RuntimeDescription {
            runtime: Runtime { abi: 1, api: 3 },
            framework: None
        })
    );
    let with_framework = format!(
        r#"{{"abi":1,"api":3,"framework":{{"sha256":"{digest}","size":13849134,"manifest":"{digest}"}}}}"#
    );
    let read = RuntimeDescription::parse(with_framework.as_bytes()).unwrap();
    assert_eq!(read.runtime, Runtime { abi: 1, api: 3 });
    assert_eq!(read.framework.map(|f| f.size), Some(13849134));
    for refused in [
        r#"{"abi":0,"api":0}"#.to_string(),
        r#"{"abi":1}"#.to_string(),
        r#"{"abi":1,"api":1,"rn":"0.88"}"#.to_string(),
        r#"{"abi":1,"api":1,"api":2}"#.to_string(),
        r#"{"abi":"1","api":1}"#.to_string(),
        r#"{"abi":1.5,"api":1}"#.to_string(),
        r#"{"abi":-1,"api":1}"#.to_string(),
        r#"{"abi":4294967296,"api":1}"#.to_string(),
        r#"{"abi":1,"api":1,"framework":{"sha256":"x","size":1,"manifest":"x"}}"#.to_string(),
        format!(r#"{{"abi":1,"api":1,"framework":{{"sha256":"{digest}","size":1}}}}"#),
        format!(
            r#"{{"abi":1,"api":1,"framework":{{"sha256":"{digest}","size":1,"manifest":"{digest}","modules":3}}}}"#
        ),
        String::new(),
        " ".repeat(MAX_RUNTIME_DESCRIPTION + 1),
    ] {
        assert!(
            RuntimeDescription::parse(refused.as_bytes()).is_err(),
            "{refused} was accepted"
        );
    }
}
