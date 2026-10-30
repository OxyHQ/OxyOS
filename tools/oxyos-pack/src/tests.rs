use super::*;

/// A directory for one test, removed when dropped.
struct Directory(PathBuf);

impl Directory {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("oxyos-pack-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("package/bundle/drawable-mdpi")).unwrap();
        std::fs::write(dir.join("package/bundle/index.js"), "globalThis.x = 1;\n").unwrap();
        std::fs::write(dir.join("package/bundle/drawable-mdpi/a.png"), [0x89, b'P']).unwrap();
        std::fs::write(dir.join("package/icon.png"), [0x89, b'P', b'N', b'G']).unwrap();
        std::fs::write(
            dir.join("description.json"),
            r#"{
  "id": "so.oxy.inbox",
  "application": "00000000-0000-0000-0000-000000000000",
  "version": "0.1.0",
  "versionCode": 1,
  "name": "Inbox",
  "names": { "es": "Bandeja de entrada" },
  "icon": "icon.png",
  "entry": { "bundle": "bundle/index.js", "module": "OxyBloom" },
  "architectures": ["any"]
}
"#,
        )
        .unwrap();
        std::fs::write(dir.join("runtime.json"), r#"{"abi":1,"api":1}"#).unwrap();
        std::fs::write(dir.join("key"), format!("{}\n", "07".repeat(32))).unwrap();
        Self(dir)
    }

    fn pack(&self) -> Result<()> {
        pack(
            &self.0.join("package"),
            &self.0.join("description.json"),
            &self.0.join("runtime.json"),
            &self.0.join("key"),
        )
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_package_is_its_files_listed_and_signed() {
    let dir = Directory::new("pack");
    dir.pack().unwrap();
    let bytes = std::fs::read(dir.0.join("package/manifest.json")).unwrap();
    let manifest = Manifest::parse(&bytes).unwrap();
    assert_eq!(manifest.id, "so.oxy.inbox");
    assert_eq!(manifest.runtime, oxyos_package::Runtime { abi: 1, api: 1 });
    assert_eq!(
        manifest
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["bundle/drawable-mdpi/a.png", "bundle/index.js", "icon.png"]
    );
    assert_eq!(manifest.names["es"].as_str(), "Bandeja de entrada");
    // Packing again (its own manifest and signature now there) gives the same manifest.
    dir.pack().unwrap();
    assert_eq!(
        std::fs::read(dir.0.join("package/manifest.json")).unwrap(),
        bytes
    );
}

#[test]
fn what_the_system_would_refuse_is_refused_here() {
    let dir = Directory::new("refused");
    let description = |text: &str| {
        std::fs::write(dir.0.join("description.json"), text).unwrap();
        dir.pack()
    };
    let good = std::fs::read_to_string(dir.0.join("description.json")).unwrap();
    assert!(
        description(&good.replace(
            r#""versionCode": 1,"#,
            r#""versionCode": 1, "versionCode": 2,"#
        ))
        .is_err(),
        "a key twice"
    );
    assert!(
        description(&good.replace(r#""versionCode": 1"#, r#""versionCode": 1.5"#)).is_err(),
        "a fraction"
    );
    assert!(
        description(&good.replace(
            r#""architectures": ["any"]"#,
            r#""architectures": ["x86_64"]"#
        ))
        .is_err(),
        "native code"
    );
    assert!(
        description(&good.replace(r#""icon": "icon.png","#, r#""icon": "missing.png","#)).is_err(),
        "an icon not there"
    );
    assert!(
        description(&good.replace(
            r#""id": "so.oxy.inbox","#,
            r#""id": "so.oxy.inbox", "runtime": {"abi":1,"api":1},"#
        ))
        .is_err(),
        "the tool's field"
    );
    assert!(
        description(&good.replace(
            r#""architectures": ["any"]"#,
            r#""architectures": ["any"], "grants": ["settings"]"#
        ))
        .is_err(),
        "an unknown field"
    );
    std::fs::write(dir.0.join("description.json"), &good).unwrap();
    // A link in the package.
    std::os::unix::fs::symlink("/etc/passwd", dir.0.join("package/bundle/passwd")).unwrap();
    assert!(dir.pack().is_err());
    std::fs::remove_file(dir.0.join("package/bundle/passwd")).unwrap();
    // Bytecode as the entry: a package carries its source.
    std::fs::write(dir.0.join("package/bundle/index.js"), HERMES_BYTECODE_MAGIC).unwrap();
    assert!(dir.pack().is_err());
    std::fs::write(dir.0.join("package/bundle/index.js"), "x;\n").unwrap();
    // A key that is not one.
    std::fs::write(dir.0.join("key"), "07").unwrap();
    assert!(dir.pack().is_err());
}

/// A program that does what a compiler would, as a shell script: reads the source on its standard
/// input and writes `bytes` to its standard output, as the runtime's `hermesc -emit-binary -O -`.
fn compiler(dir: &Path, bytes: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(format!("hermesc-{}", bytes.len()));
    std::fs::write(
        &path,
        format!("#!/bin/sh\ncat > /dev/null\nprintf '{bytes}'\n"),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn compiling_checks_the_tree_and_keeps_only_bytecode() {
    let dir = Directory::new("compile");
    dir.pack().unwrap();
    let package = dir.0.join("package");
    let mut magic = String::new();
    for byte in HERMES_BYTECODE_MAGIC {
        let _ = write!(magic, "\\{byte:03o}");
    }
    compile(&package, &compiler(&dir.0, &magic)).unwrap();
    assert!(std::fs::read(package.join("index.hbc"))
        .unwrap()
        .starts_with(&HERMES_BYTECODE_MAGIC));
    // Output that is not bytecode is not kept.
    std::fs::remove_file(package.join("index.hbc")).unwrap();
    assert!(compile(&package, &compiler(&dir.0, "nonsense")).is_err());
    assert!(!package.join("index.hbc").exists() && !package.join("index.hbc.new").exists());
    // A file changed since the package was made: not compiled.
    std::fs::write(package.join("bundle/index.js"), "globalThis.x = 2;\n").unwrap();
    assert!(compile(&package, &compiler(&dir.0, &magic)).is_err());
    // The real compiler, when the machine has one (the runtime's own, OXYOS_TEST_HERMESC).
    if let Some(hermesc) = std::env::var_os("OXYOS_TEST_HERMESC") {
        dir.pack().unwrap();
        compile(&package, Path::new(&hermesc)).unwrap();
        assert!(std::fs::read(package.join("index.hbc"))
            .unwrap()
            .starts_with(&HERMES_BYTECODE_MAGIC));
    } else {
        eprintln!("OXYOS_TEST_HERMESC is not set: the real compiler is not run");
    }
}

#[test]
fn the_canonical_form_is_jcs() {
    let mut out = String::new();
    canonical(
        &serde_json::json!({"b": [1, "é\n\u{1}\"\\"], "a": {"z": true, "A": null}, "€": 0, "\u{10000}": 1}),
        &mut out,
    );
    // Keys by UTF-16 code units: U+20AC before U+10000 (D800 DC00), which by code points is after.
    assert_eq!(
        out,
        "{\"a\":{\"A\":null,\"z\":true},\"b\":[1,\"é\\n\\u0001\\\"\\\\\"],\"€\":0,\"\u{10000}\":1}"
    );
}
