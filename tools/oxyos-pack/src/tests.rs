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

/// A compiler standing in for `hermesc`: Hermes's magic, then the source.
struct Stand;

impl oxyos_install::compile::Compile for Stand {
    fn compile(
        &self,
        _hermesc: &Path,
        source: &std::fs::File,
        out: &std::fs::File,
    ) -> std::result::Result<(), String> {
        use std::io::{Read, Write};
        let (mut text, mut source) = (Vec::new(), source);
        source.read_to_end(&mut text).map_err(|e| e.to_string())?;
        let mut out = out;
        out.write_all(&HERMES_BYTECODE_MAGIC)
            .and_then(|()| out.write_all(&text))
            .map_err(|e| e.to_string())
    }
}

/// What `publish` publishes, the install service installs, and a later version is its update.
#[test]
fn the_install_service_installs_what_publish_published() {
    use oxyos_install::accounts::{Account, Fixed};
    use oxyos_install::fetch::{LocalRepository, ThreadFetcher};
    use oxyos_install::install::{Installer, Machine};
    use oxyos_install::state::Layout;
    use std::os::unix::fs::MetadataExt;
    let dir = Directory::new("publish");
    dir.pack().unwrap();
    let (keys, repo) = (dir.0.join("store-keys"), dir.0.join("repository"));
    store_keys(&keys).unwrap();
    repository(&repo, &keys).unwrap();
    // Not a listing's slug: refused, nothing published.
    let refused = publish(
        &dir.0.join("package"),
        &repo,
        &keys,
        &dir.0.join("key"),
        Some("Inbox"),
    )
    .unwrap_err();
    assert!(refused.contains("not a store listing"), "{refused}");
    publish(
        &dir.0.join("package"),
        &repo,
        &keys,
        &dir.0.join("key"),
        Some("inbox"),
    )
    .unwrap();

    // A machine: runtime 1 at api 1, this process's own account.
    let runtimes = dir.0.join("runtimes");
    std::fs::create_dir_all(runtimes.join("1")).unwrap();
    std::fs::write(runtimes.join("1/runtime.json"), r#"{"abi":1,"api":1}"#).unwrap();
    std::fs::write(runtimes.join("1/hermesc"), b"").unwrap();
    let me = std::fs::metadata(&dir.0).unwrap();
    let accounts = Fixed(vec![Account {
        uid: me.uid(),
        name: "me".into(),
        gid: me.gid(),
    }]);
    let fetcher = ThreadFetcher::start(&dir.0, LocalRepository::new(&repo)).unwrap();
    let installer = Installer::open(
        Layout::new(&dir.0.join("state")),
        &repo.join("metadata/1.root.json"),
        fetcher.client(),
        Stand,
        accounts,
        Machine::discover(&runtimes, oxyos_package::Architecture::X86_64),
        now,
    )
    .unwrap();
    // The listing it was published for names it on the machine.
    let resolved = installer
        .resolve(me.uid(), &["inbox".into(), "mention".into()])
        .unwrap();
    assert_eq!(resolved["inbox"].as_deref(), Some("so.oxy.inbox"));
    assert_eq!(resolved["mention"], None);
    let prepared = installer.prepare(me.uid(), "so.oxy.inbox").unwrap();
    assert_eq!(prepared.plan.version_code, 1);
    installer.commit(&prepared, &mut |_| {}).unwrap();
    let entry = std::fs::read_to_string(
        dir.0
            .join("state/users/me/applications/so.oxy.inbox.desktop"),
    )
    .unwrap();
    assert!(entry.contains("/so.oxy.inbox/1/index.hbc"), "{entry}");

    // Version 2, packed and published over the same repository: the next metadata, an update.
    let two = dir.0.join("package-2");
    std::fs::create_dir_all(two.join("bundle")).unwrap();
    std::fs::write(two.join("bundle/index.js"), "globalThis.x = 2;\n").unwrap();
    std::fs::copy(dir.0.join("package/icon.png"), two.join("icon.png")).unwrap();
    let description = std::fs::read_to_string(dir.0.join("description.json"))
        .unwrap()
        .replace("\"versionCode\": 1", "\"versionCode\": 2")
        .replace("\"0.1.0\"", "\"0.2.0\"");
    std::fs::write(dir.0.join("description-2.json"), description).unwrap();
    pack(
        &two,
        &dir.0.join("description-2.json"),
        &dir.0.join("runtime.json"),
        &dir.0.join("key"),
    )
    .unwrap();
    publish(&two, &repo, &keys, &dir.0.join("key"), None).unwrap();
    installer.check().unwrap();
    let state = installer.update_state(me.uid()).unwrap();
    assert_eq!(state.updates.len(), 1);
    assert_eq!(state.updates[0].to_version_code, 2);
}
