use super::*;

/// A directory of entries for one test, removed when dropped.
struct Directory(PathBuf);

impl Directory {
    fn new(name: &str, entries: &[(&str, &str)]) -> Self {
        let dir = std::env::temp_dir().join(format!("oxyos-apps-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in entries {
            std::fs::write(dir.join(file), text).unwrap();
        }
        Self(dir)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const ALLO: &str = "[Desktop Entry]\nType=Application\nName=Allo\nName[es]=Allo en español\nIcon=allo.png\nX-OxyOS-Bloom-Module=EditingProbe\nX-OxyOS-Pinned=2\n\n[Desktop Action new]\nName=Ignored\nExec=ignored\n";

#[test]
fn an_entry_is_an_application_named_in_the_sessions_language() {
    let dir = Path::new("/apps");
    let es = parse("so.oxy.allo", ALLO, dir, &["es_ES".into(), "es".into()])
        .unwrap()
        .unwrap();
    assert_eq!(es.name, "Allo en español");
    assert_eq!(es.icon, Some(PathBuf::from("/apps/allo.png")));
    assert_eq!(
        es.launch,
        Launch::Bloom {
            module: "EditingProbe".into(),
            bundle: None,
            runtime: None
        }
    );
    assert_eq!(es.pinned, Some(2));
    let en = parse("so.oxy.allo", ALLO, dir, &["en_US".into(), "en".into()])
        .unwrap()
        .unwrap();
    assert_eq!(en.name, "Allo");
}

#[test]
fn exec_is_split_as_the_specification_quotes_and_field_codes_are_refused() {
    let entry = |exec: &str| {
        parse(
            "so.oxy.tool",
            &format!("[Desktop Entry]\nType=Application\nName=Tool\nExec={exec}\n"),
            Path::new("/"),
            &[],
        )
    };
    assert_eq!(
        entry(r#"/usr/bin/tool --title "Hola mundo" 100%%"#)
            .unwrap()
            .unwrap()
            .launch,
        Launch::Exec(vec![
            "/usr/bin/tool".into(),
            "--title".into(),
            "Hola mundo".into(),
            "100%".into()
        ])
    );
    assert!(entry("/usr/bin/tool %f").is_err());
    assert!(entry(r#"/usr/bin/tool "open"#).is_err());
    assert!(entry("").is_err());
}

#[test]
fn entries_not_to_list_and_invalid_ids_are_not_applications() {
    let dir = Path::new("/");
    let hidden = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n";
    assert_eq!(parse("so.oxy.x", hidden, dir, &[]), Ok(None));
    let link = "[Desktop Entry]\nType=Link\nName=X\nURL=https://oxy.so\n";
    assert_eq!(parse("so.oxy.x", link, dir, &[]), Ok(None));
    assert!(parse("../etc/passwd", ALLO, dir, &[]).is_err());
    assert!(parse(
        "so.oxy.allo",
        "[Desktop Entry]\nType=Application\nExec=x\n",
        dir,
        &[]
    )
    .is_err());
    assert!(valid_id("so.oxy.allo") && !valid_id("") && !valid_id(".hidden") && !valid_id("a b"));
}

#[test]
fn the_registry_lists_by_name_and_the_first_directory_wins() {
    let user = Directory::new(
        "user",
        &[
            (
                "so.oxy.allo.desktop",
                "[Desktop Entry]\nType=Application\nName=Mi Allo\nExec=/bin/mine\n",
            ),
            (
                "so.oxy.gone.desktop",
                "[Desktop Entry]\nType=Application\nName=Gone\nExec=x\nHidden=true\n",
            ),
        ],
    );
    let system = Directory::new(
        "system",
        &[
            ("so.oxy.allo.desktop", ALLO),
            (
                "so.oxy.gone.desktop",
                "[Desktop Entry]\nType=Application\nName=Gone\nExec=x\n",
            ),
            (
                "so.oxy.astro.desktop",
                "[Desktop Entry]\nType=Application\nName=Astro\nExec=/bin/astro\n",
            ),
            (
                "so.oxy.broken.desktop",
                "[Desktop Entry]\nType=Application\nName=Broken\n",
            ),
            ("README", "not an entry"),
        ],
    );
    let (apps, skipped) = registry(
        &[user.0.clone(), system.0.clone()],
        &[],
        &Policy::none(),
        None,
        &|_| false,
    );
    let names: Vec<(&str, &str)> = apps
        .iter()
        .map(|a| (a.id.as_str(), a.name.as_str()))
        .collect();
    // The user's Allo replaces the system's; the user hid Gone; Broken is said and left out.
    assert_eq!(
        names,
        [("so.oxy.astro", "Astro"), ("so.oxy.allo", "Mi Allo")]
    );
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert!(skipped[0].contains("so.oxy.broken"));
    assert_eq!(
        find(
            &[system.0.clone()],
            &[],
            &Policy::none(),
            None,
            &|_| false,
            "so.oxy.allo"
        )
        .map(|a| a.name),
        Some("Allo".into())
    );
    assert_eq!(
        find(
            &[system.0.clone()],
            &[],
            &Policy::none(),
            None,
            &|_| false,
            "so.oxy.none"
        ),
        None
    );
}

#[test]
fn a_locales_languages_are_its_region_then_its_language() {
    assert_eq!(languages_of("es_ES.UTF-8"), ["es_ES", "es"]);
    assert_eq!(languages_of("sr_RS@latin"), ["sr_RS", "sr"]);
    assert_eq!(languages_of("fi"), ["fi"]);
    assert!(languages_of("C.UTF-8").is_empty() && languages_of("POSIX").is_empty());
    assert!(languages_of("").is_empty());
}

#[test]
fn a_dock_place_is_a_number_from_one() {
    let entry = |pinned: &str| {
        parse(
            "so.oxy.x",
            &format!(
                "[Desktop Entry]\nType=Application\nName=X\nExec=x\nX-OxyOS-Pinned={pinned}\n"
            ),
            Path::new("/"),
            &[],
        )
    };
    assert_eq!(entry("3").unwrap().unwrap().pinned, Some(3));
    assert!(entry("0").is_err() && entry("true").is_err());
}

#[test]
fn a_file_opens_in_the_application_declaring_its_type_with_its_url() {
    let out = std::env::temp_dir().join(format!("oxyos-apps-opened-{}", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let writer = |name: &str, types: &str| {
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nMimeType={types}\nExec=/bin/sh -c \"echo $OXYOS_APP_ID $OXYOS_LAUNCH_URL > {}\"\n",
            out.display()
        )
    };
    let viewer = writer("Viewer", "image/jpeg; image/png;");
    let files = writer("Files", "");
    let dir = Directory::new(
        "open",
        &[
            ("so.oxy.viewer.desktop", viewer.as_str()),
            ("so.oxy.files.desktop", files.as_str()),
        ],
    );
    let runtime = Runtime {
        socket: PathBuf::from("/run/oxyos-test"),
        bloom_host: None,
        bloom_bundle: None,
        runtimes: PathBuf::from("/nonexistent-runtimes"),
        window: (960, 600),
        env: Vec::new(),
    };
    let mut launcher = Launcher::new(runtime, vec![dir.0.clone()], vec![]);
    let file = Path::new("/home/ada/Escritorio/Captura de pantalla 2026-10-31 a las 10.04.05.png");
    let (id, pid) = launcher.open(file, "image/png", "so.oxy.files").unwrap();
    assert_eq!(id, "so.oxy.viewer");
    assert_eq!(
        launcher
            .application_of(pid)
            .and_then(|l| l.opened.as_deref()),
        Some(file)
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while launcher.reap().is_empty() {
        assert!(std::time::Instant::now() < deadline, "it did not exit");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(&out).unwrap().trim(),
        "so.oxy.viewer file:///home/ada/Escritorio/Captura%20de%20pantalla%202026-10-31%20a%20las%2010.04.05.png"
    );
    // A type nobody declares: the fallback, the file manager.
    let (id, _) = launcher
        .open(file, "application/pdf", "so.oxy.files")
        .unwrap();
    assert_eq!(id, "so.oxy.files");
    launcher.end(std::time::Duration::from_secs(2));
    let _ = std::fs::remove_file(&out);
    assert_eq!(file_url(Path::new("/a/ñ b")), "file:///a/%C3%B1%20b");
}

#[test]
fn a_launch_is_told_its_grants_and_a_development_entry_none() {
    let out = std::env::temp_dir().join(format!("oxyos-apps-grants-{}", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let dir = Directory::new(
        "grants",
        &[
            (
                "so.oxy.atlas.desktop",
                &format!(
                    "[Desktop Entry]\nType=Application\nName=Atlas\nExec=/bin/sh -c \"echo atlas=$OXYOS_APP_GRANTS >> {}\"\n",
                    out.display()
                ),
            ),
            (
                "so.oxy.writer.desktop",
                &format!(
                    "[Desktop Entry]\nType=Application\nName=Writer\nExec=/bin/sh -c \"echo writer=$OXYOS_APP_GRANTS >> {}\"\n",
                    out.display()
                ),
            ),
        ],
    );
    let runtime = Runtime {
        socket: PathBuf::from("/run/oxyos-test"),
        bloom_host: None,
        bloom_bundle: None,
        runtimes: PathBuf::from("/nonexistent-runtimes"),
        window: (960, 600),
        env: Vec::new(),
    };
    // The development compositor's grants, as a scenario gives them in the policy's place.
    let store = Grants::named(&["store".into(), "updates".into()]).unwrap();
    let mut launcher = Launcher::new(runtime, vec![dir.0.clone()], vec![])
        .with_fixture_grants(vec![("so.oxy.atlas".into(), store)]);
    for id in ["so.oxy.atlas", "so.oxy.writer"] {
        launcher.launch(id).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while launcher.reap().is_empty() {
            assert!(std::time::Instant::now() < deadline, "it did not exit");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "atlas=store,updates\nwriter=\n"
    );
    assert_eq!(store.names(), "store,updates");
    assert_eq!(Grants::default().names(), "");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn a_url_opens_in_the_application_declaring_its_scheme_and_a_force_quit_ends_only_ours() {
    let out = std::env::temp_dir().join(format!("oxyos-apps-url-{}", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let stash = format!(
        "[Desktop Entry]\nType=Application\nName=Stash\nMimeType=inode/directory;x-scheme-handler/stash;\nExec=/bin/sh -c \"echo $OXYOS_APP_ID $OXYOS_LAUNCH_URL > {}\"\n",
        out.display()
    );
    let sleeper = "[Desktop Entry]\nType=Application\nName=Sleeper\nExec=/bin/sleep 30\n";
    let dir = Directory::new(
        "url",
        &[
            ("so.oxy.stash.desktop", stash.as_str()),
            ("so.oxy.sleeper.desktop", sleeper),
        ],
    );
    let runtime = Runtime {
        socket: PathBuf::from("/run/oxyos-test"),
        bloom_host: None,
        bloom_bundle: None,
        runtimes: PathBuf::from("/nonexistent-runtimes"),
        window: (960, 600),
        env: Vec::new(),
    };
    let mut launcher = Launcher::new(runtime, vec![dir.0.clone()], vec![]);
    let (id, pid) = launcher.open_url("Stash://trash").unwrap();
    assert_eq!(id, "so.oxy.stash");
    assert_eq!(
        launcher.application_of(pid).and_then(|l| l.url.as_deref()),
        Some("Stash://trash")
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while launcher.reap().is_empty() {
        assert!(std::time::Instant::now() < deadline, "it did not exit");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(&out).unwrap().trim(),
        "so.oxy.stash Stash://trash"
    );
    // No scheme, a file, a scheme nobody opens: refused, nothing started.
    for url in ["trash", "1x://a", "file:///home", "mailto:ana@oxy.so"] {
        assert!(launcher.open_url(url).is_err(), "{url}");
    }
    // A force quit ends a process it started, at once, and nothing it did not.
    let sleeping = launcher.launch("so.oxy.sleeper").unwrap();
    assert!(!launcher.force_quit(std::process::id()));
    assert!(launcher.force_quit(sleeping));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let exited = launcher.reap();
        if exited.iter().any(|(pid, _)| *pid == sleeping) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "it was not ended");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(url_scheme("stash://trash").as_deref(), Some("stash"));
    assert_eq!(url_scheme("://x"), None);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn a_launch_runs_what_the_registry_lists_and_nothing_else() {
    let out = std::env::temp_dir().join(format!("oxyos-apps-launched-{}", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let dir = Directory::new(
        "launch",
        &[(
            "so.oxy.writer.desktop",
            &format!(
                "[Desktop Entry]\nType=Application\nName=Writer\nExec=/bin/sh -c \"echo $OXYOS_APP_ID $OXYOS_COMPOSITOR_SOCKET ${{DISPLAY:-none}} > {}\"\n",
                out.display()
            ),
        )],
    );
    let runtime = Runtime {
        socket: PathBuf::from("/run/oxyos-test"),
        bloom_host: None,
        bloom_bundle: None,
        runtimes: PathBuf::from("/nonexistent-runtimes"),
        window: (960, 600),
        env: Vec::new(),
    };
    let mut launcher = Launcher::new(runtime, vec![dir.0.clone()], vec![]);
    assert!(launcher.launch("so.oxy.none").is_err());
    let pid = launcher.launch("so.oxy.writer").unwrap();
    assert_eq!(
        launcher.application_of(pid),
        Some(&Launched {
            id: "so.oxy.writer".into(),
            name: "Writer".into(),
            grants: Grants::default(),
            origin: Origin::Development,
            capabilities: BTreeMap::new(),
            opened: None,
            chooser: false,
            url: None,
        })
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while launcher.reap().is_empty() {
        assert!(std::time::Instant::now() < deadline, "it did not exit");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(launcher.application_of(pid), None);
    // It was told its id and the compositor, and no other display.
    assert_eq!(
        std::fs::read_to_string(&out).unwrap().trim(),
        "so.oxy.writer /run/oxyos-test none"
    );
    let _ = std::fs::remove_file(&out);
    // A Bloom application needs the session's host and a bundle.
    let bloom = parse("so.oxy.allo", ALLO, Path::new("/apps"), &[])
        .unwrap()
        .unwrap();
    assert!(command(&bloom, &launcher.runtime).is_err());
    let runtime = Runtime {
        bloom_host: Some("/opt/oxyos-bloom-host".into()),
        bloom_bundle: Some("/opt/bundle/index.js".into()),
        runtimes: PathBuf::from("/nonexistent-runtimes"),
        ..launcher.runtime.clone()
    };
    let command = command(&bloom, &runtime).unwrap();
    assert_eq!(command.get_program(), "/opt/oxyos-bloom-host");
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [
            "/opt/bundle/index.js",
            "EditingProbe",
            "--window",
            "960",
            "600"
        ]
    );
}

// ---- system grants (docs/adr/0019-settings.mdx, docs/adr/0030-installing-applications.mdx) -----

const SETTINGS: &str =
    "[Desktop Entry]\nType=Application\nName=Settings\nExec=/bin/sh -c \"exit 0\"\nX-OxyOS-Grants=settings;\n";

/// A policy naming `id` as a native system application: its entry in `directory`, `program`
/// what it runs.
fn policy_of(id: &str, directory: &Path, program: &Path, grants: &str) -> String {
    format!(
        r#"{{"applications":{{"{id}":{{"entry":"{}","exec":"{}","grants":[{grants}]}}}}}}"#,
        directory.join(format!("{id}.desktop")).display(),
        program.display()
    )
}

#[test]
fn the_policy_is_read_strictly() {
    let good = r#"{"applications":{
        "so.oxy.settings":{"entry":"/usr/share/oxyos/applications/so.oxy.settings.desktop",
            "bloom":{"host":"/usr/lib/oxyos/oxyos-bloom-host","bundle":"/usr/lib/oxyos/bundle/index.js"},
            "grants":["settings"]},
        "so.oxy.stash":{"entry":"/usr/share/oxyos/applications/so.oxy.stash.desktop",
            "exec":"/usr/libexec/stash","grants":[]}}}"#;
    let policy = Policy::parse(good).unwrap();
    assert_eq!(
        policy.get("so.oxy.settings"),
        Some(&SystemApplication {
            entry: "/usr/share/oxyos/applications/so.oxy.settings.desktop".into(),
            runs: Runs::Bloom {
                host: "/usr/lib/oxyos/oxyos-bloom-host".into(),
                bundle: "/usr/lib/oxyos/bundle/index.js".into(),
            },
            grants: Grants {
                settings: true,
                ..Grants::default()
            },
        })
    );
    assert_eq!(
        policy.get("so.oxy.stash").map(|app| app.grants),
        Some(Grants::default())
    );
    assert_eq!(policy.get("so.oxy.mention"), None);
    let one = |fields: &str| {
        Policy::parse(&format!(
            r#"{{"applications":{{"so.oxy.x":{{{fields}}}}}}}"#
        ))
    };
    let entry = r#""entry":"/usr/share/oxyos/applications/so.oxy.x.desktop""#;
    assert!(one(&format!(
        r#"{entry},"exec":"/usr/bin/x","grants":["settings"]"#
    ))
    .is_ok());
    // Each way a policy could mean something it does not say refuses it whole.
    let refused = [
        (
            "an unknown key",
            format!(r#"{entry},"exec":"/x","grants":[],"pinned":1"#),
        ),
        (
            "a key twice",
            format!(r#"{entry},"exec":"/x","exec":"/y","grants":[]"#),
        ),
        (
            "an unknown grant",
            format!(r#"{entry},"exec":"/x","grants":["shell"]"#),
        ),
        (
            "a grant's other spelling",
            format!(r#"{entry},"exec":"/x","grants":["Settings"]"#),
        ),
        (
            "a grant twice",
            format!(r#"{entry},"exec":"/x","grants":["settings","settings"]"#),
        ),
        ("no grants named", format!(r#"{entry},"exec":"/x""#)),
        (
            "a relative program",
            format!(r#"{entry},"exec":"x","grants":[]"#),
        ),
        (
            "a program through ..",
            format!(r#"{entry},"exec":"/usr/../x","grants":[]"#),
        ),
        (
            "both exec and bloom",
            format!(r#"{entry},"exec":"/x","bloom":{{"host":"/h","bundle":"/b"}},"grants":[]"#),
        ),
        ("neither exec nor bloom", format!(r#"{entry},"grants":[]"#)),
        (
            "a relative bundle",
            format!(r#"{entry},"bloom":{{"host":"/h","bundle":"b"}},"grants":[]"#),
        ),
        (
            "a bloom key unknown",
            format!(r#"{entry},"bloom":{{"host":"/h","bundle":"/b","module":"M"}},"grants":[]"#),
        ),
        (
            "an entry of another id",
            r#""entry":"/usr/share/oxyos/applications/so.oxy.y.desktop","exec":"/x","grants":[]"#
                .to_string(),
        ),
        (
            "a relative entry",
            r#""entry":"so.oxy.x.desktop","exec":"/x","grants":[]"#.to_string(),
        ),
    ];
    for (what, fields) in refused {
        assert!(one(&fields).is_err(), "{what} was accepted");
    }
    let app = format!(r#"{{{entry},"exec":"/x","grants":[]}}"#);
    assert!(
        Policy::parse(&format!(
            r#"{{"applications":{{"so.oxy.x":{app},"so.oxy.x":{app}}}}}"#
        ))
        .is_err(),
        "an id twice"
    );
    assert!(
        Policy::parse(&format!(r#"{{"applications":{{"../x":{app}}}}}"#)).is_err(),
        "not an id"
    );
    assert!(
        Policy::parse(r#"{"applications":{},"version":1}"#).is_err(),
        "an unknown key above"
    );
    assert!(Policy::parse(r#"{}"#).is_err(), "no applications");
    assert!(
        Policy::parse("").is_err() && Policy::parse("[").is_err(),
        "not JSON"
    );
    assert_eq!(Policy::parse(r#"{"applications":{}}"#), Ok(Policy::none()));
    // A development machine's, with Settings' host and bundle in /opt/oxyos and its entry in
    // /usr/local, laid out as a person writes it.
    let development = r#"{
  "applications": {
    "so.oxy.settings": {
      "entry": "/usr/local/share/oxyos/applications/so.oxy.settings.desktop",
      "bloom": { "host": "/opt/oxyos/oxyos-bloom-host", "bundle": "/opt/oxyos/bundle/index.js" },
      "grants": ["settings"]
    }
  }
}
"#;
    assert_eq!(
        Policy::parse(development)
            .unwrap()
            .get("so.oxy.settings")
            .map(|app| app.grants),
        Some(Grants {
            settings: true,
            ..Grants::default()
        })
    );
}

#[test]
fn the_policy_names_the_store_s_updates_and_privacy_grants() {
    let entry = r#""entry":"/usr/share/oxyos/applications/so.oxy.x.desktop","exec":"/x""#;
    let policy = |grants: &str| {
        Policy::parse(&format!(
            r#"{{"applications":{{"so.oxy.x":{{{entry},"grants":[{grants}]}}}}}}"#
        ))
    };
    let grants = |list: &str| policy(list).unwrap().get("so.oxy.x").unwrap().grants;
    assert_eq!(
        grants(r#""store","updates""#),
        Grants {
            store: true,
            updates: true,
            ..Grants::default()
        }
    );
    assert_eq!(
        grants(r#""settings","updates","privacy""#),
        Grants {
            settings: true,
            updates: true,
            privacy: true,
            ..Grants::default()
        }
    );
    assert!(policy(r#""store","store""#).is_err(), "a grant twice");
    assert!(policy(r#""Store""#).is_err(), "another spelling");
    assert!(policy(r#""install""#).is_err(), "not a system grant");
}

#[test]
fn a_policy_is_an_administrators_or_nothing() {
    let dir = Directory::new("policy-read", &[]);
    let path = dir.0.join("system-apps.json");
    let trusted = as_if_root(&dir.0);
    // No policy: no system application.
    assert_eq!(Policy::read(&path, &trusted), Ok(Policy::none()));
    std::fs::write(
        &path,
        policy_of(
            "so.oxy.settings",
            Path::new("/e"),
            Path::new("/p"),
            r#""settings""#,
        ),
    )
    .unwrap();
    assert!(Policy::read(&path, &trusted)
        .unwrap()
        .get("so.oxy.settings")
        .is_some());
    // The same file, the user's: nothing, and said.
    assert!(Policy::read(&path, &|_| false).is_err());
    // A link to it, even in the administrator's directory: nothing.
    let link = dir.0.join("linked.json");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(Policy::read(&link, &trusted).is_err());
    // Larger than a policy may be: nothing.
    std::fs::write(&path, " ".repeat(POLICY_LIMIT as usize + 1)).unwrap();
    assert!(Policy::read(&path, &trusted).is_err());
    // A user's file is never an administrator's: the session's own rule refuses it.
    std::fs::write(&path, r#"{"applications":{}}"#).unwrap();
    assert!(Policy::read(&path, &administrator_written).is_err());
}

#[test]
fn an_entry_grants_nothing_whoever_wrote_it() {
    // Root's entry asking for `settings`, which the policy does not name: the hole ADR 0030's
    // decision 7 closes (an installer writes root's files from what it downloads).
    let dir = Directory::new("grants-entry", &[("so.oxy.settings.desktop", SETTINGS)]);
    let trusted = as_if_root(&dir.0);
    let (apps, skipped) = registry(&[dir.0.clone()], &[], &Policy::none(), None, &trusted);
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(
        apps.iter()
            .map(|a| (a.id.as_str(), a.origin == Origin::System))
            .collect::<Vec<_>>(),
        [("so.oxy.settings", false)]
    );
    let mut launcher = test_launcher(&dir, None);
    launcher.trusted =
        Box::new(move |path| trusted(path) || path == Path::new("/bin/sh").canonicalize().unwrap());
    let pid = launcher.launch("so.oxy.settings").unwrap();
    assert_eq!(
        launcher.application_of(pid).map(|l| l.grants),
        Some(Grants::default())
    );
    launcher.end(std::time::Duration::from_secs(5));
}

/// A launcher over `dir`'s entries with the policy at `policy` (none if absent).
fn test_launcher(dir: &Directory, policy: Option<&Path>) -> Launcher {
    let mut launcher = Launcher::new(
        Runtime {
            socket: PathBuf::from("/run/oxyos-test"),
            bloom_host: None,
            bloom_bundle: None,
            runtimes: PathBuf::from("/nonexistent-runtimes"),
            window: (960, 600),
            env: Vec::new(),
        },
        vec![dir.0.clone()],
        vec![],
    );
    launcher.policy = policy.map_or_else(|| dir.0.join("no-policy.json"), Path::to_path_buf);
    launcher
}

#[test]
fn a_system_application_is_the_policys_entry_and_no_one_elses() {
    let user = Directory::new(
        "system-user",
        &[(
            "so.oxy.settings.desktop",
            "[Desktop Entry]\nType=Application\nName=Mine\nExec=/bin/true\n",
        )],
    );
    let system = Directory::new("system-entries", &[("so.oxy.settings.desktop", SETTINGS)]);
    let policy = Policy::parse(&policy_of(
        "so.oxy.settings",
        &system.0.canonicalize().unwrap(),
        Path::new("/bin/sh"),
        r#""settings""#,
    ))
    .unwrap();
    let trusted = as_if_root(&system.0);
    // The user's directory first, as `$XDG_DATA_HOME` is: the system's Settings still wins.
    let (apps, skipped) = registry(
        &[user.0.clone(), system.0.clone()],
        &[],
        &policy,
        None,
        &trusted,
    );
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(
        apps.iter()
            .map(|a| (a.id.as_str(), a.name.as_str(), a.origin == Origin::System))
            .collect::<Vec<_>>(),
        [("so.oxy.settings", "Settings", true)]
    );
    // The policy's entry the user's: Settings is left out, said, and never the user's instead.
    let (apps, skipped) = registry(
        &[user.0.clone(), system.0.clone()],
        &[],
        &policy,
        None,
        &|_| false,
    );
    assert!(apps.is_empty(), "{apps:?}");
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    // Nor through a link on the way to it: the path the policy names must be the real one.
    let linked = Directory::new("system-linked", &[]);
    std::os::unix::fs::symlink(&system.0, linked.0.join("entries")).unwrap();
    let through_link = Policy::parse(&policy_of(
        "so.oxy.settings",
        &linked.0.canonicalize().unwrap().join("entries"),
        Path::new("/bin/sh"),
        r#""settings""#,
    ))
    .unwrap();
    let (apps, _) = registry(&[], &[], &through_link, None, &trusted);
    assert!(apps.is_empty(), "{apps:?}");
}

#[test]
fn the_file_chooser_is_the_policy_s_file_manager_told_what_to_show() {
    let shell = Path::new("/bin/sh").canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("oxyos-apps-chooser-{}", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Stash\nExec=/bin/sh -c \"echo $OXYOS_APP_ID $OXYOS_FILE_CHOOSER > {}\"\n",
        out.display()
    );
    let dir = Directory::new("chooser", &[("so.oxy.stash.desktop", &entry)]);
    let policy_file = Directory::new("chooser-policy", &[]);
    let path = policy_file.0.join("system-apps.json");
    let mut launcher = test_launcher(&dir, Some(&path));
    let shown = r#"{"mode":"open","multiple":false}"#;
    // Not a system application: no chooser, unless the development compositor's fixture.
    assert!(launcher.choose("so.oxy.stash", shown, false).is_err());
    assert!(launcher.choose("so.oxy.none", shown, true).is_err());
    let pid = launcher.choose("so.oxy.stash", shown, true).unwrap();
    assert!(launcher.application_of(pid).is_some_and(|l| l.chooser));
    launcher.end(std::time::Duration::from_secs(5));
    // The policy's: started, told what to show.
    std::fs::write(
        &path,
        policy_of("so.oxy.stash", &dir.0.canonicalize().unwrap(), &shell, ""),
    )
    .unwrap();
    let root = {
        let (entries, policies) = (as_if_root(&dir.0), as_if_root(&policy_file.0));
        move |path: &Path| entries(path) || policies(path) || path == shell
    };
    launcher.trusted = Box::new(root);
    let pid = launcher.choose("so.oxy.stash", shown, false).unwrap();
    let launched = launcher.application_of(pid).unwrap();
    assert!(launched.chooser && launched.origin == Origin::System);
    assert_eq!(launched.name, "Stash");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while launcher.reap().is_empty() {
        assert!(std::time::Instant::now() < deadline, "it did not exit");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(&out).unwrap().trim(),
        r#"so.oxy.stash {"mode":"open","multiple":false}"#
    );
    // A process it did not start is never signalled.
    launcher.terminate(1);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn a_system_launch_runs_what_the_policy_names() {
    let shell = Path::new("/bin/sh").canonicalize().unwrap();
    let dir = Directory::new(
        "system-launch",
        &[
            ("so.oxy.settings.desktop", SETTINGS),
            (
                "so.oxy.searched.desktop",
                "[Desktop Entry]\nType=Application\nName=Searched\nExec=sh -c \"exit 0\"\n",
            ),
            (
                "so.oxy.other.desktop",
                "[Desktop Entry]\nType=Application\nName=Other\nExec=/bin/true\n",
            ),
        ],
    );
    let entries = dir.0.canonicalize().unwrap();
    let policy_file = Directory::new("system-launch-policy", &[]);
    let path = policy_file.0.join("system-apps.json");
    let write = |id: &str, program: &Path| {
        std::fs::write(&path, policy_of(id, &entries, program, r#""settings""#)).unwrap();
    };
    let root = {
        let (entries, policies) = (as_if_root(&dir.0), as_if_root(&policy_file.0));
        let shell = shell.clone();
        move |path: &Path| entries(path) || policies(path) || path == shell
    };
    let mut launcher = test_launcher(&dir, Some(&path));
    launcher.trusted = Box::new(root);
    // The policy's program, an administrator's, by its real path: granted.
    write("so.oxy.settings", &shell);
    let pid = launcher.launch("so.oxy.settings").unwrap();
    assert_eq!(
        launcher.application_of(pid).map(|l| l.grants),
        Some(Grants {
            settings: true,
            ..Grants::default()
        })
    );
    // A program the PATH finds is whatever the PATH finds: not started.
    write("so.oxy.searched", &shell);
    assert!(launcher.launch("so.oxy.searched").is_err());
    // A program other than the policy's: not started.
    write("so.oxy.other", &shell);
    assert!(launcher.launch("so.oxy.other").is_err());
    // The policy's program, but anyone's (the user's own script): not started.
    let users = Directory::new(
        "system-launch-program",
        &[("program", "#!/bin/sh\nexit 0\n")],
    );
    let script = users.0.canonicalize().unwrap().join("program");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(
        dir.0.join("so.oxy.users.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Users\nExec={}\n",
            script.display()
        ),
    )
    .unwrap();
    write("so.oxy.users", &script);
    assert!(launcher.launch("so.oxy.users").is_err());
    write("so.oxy.settings", &shell);
    // A policy anyone could have written gives nothing: Settings runs, as an application.
    launcher.trusted = Box::new({
        let entries = as_if_root(&dir.0);
        let shell = shell.clone();
        move |path: &Path| entries(path) || path == shell
    });
    let pid = launcher.launch("so.oxy.settings").unwrap();
    assert_eq!(
        launcher.application_of(pid).map(|l| l.grants),
        Some(Grants::default())
    );
    launcher.end(std::time::Duration::from_secs(5));

    // A Bloom application runs the policy's host and bundle, whatever the session's are, by
    // their real paths, and only if they are an administrator's.
    let bloom = Application {
        id: "so.oxy.settings".into(),
        name: "Settings".into(),
        icon: None,
        launch: Launch::Bloom {
            module: "Settings".into(),
            bundle: Some(dir.0.join("elsewhere.js")),
            runtime: None,
        },
        pinned: None,
        origin: Origin::System,
        opens: Vec::new(),
    };
    let host = entries.join("host");
    let bundle = entries.join("bundle.js");
    std::fs::write(&host, "").unwrap();
    std::fs::write(&bundle, "").unwrap();
    let system = SystemApplication {
        entry: entries.join("so.oxy.settings.desktop"),
        runs: Runs::Bloom {
            host: host.clone(),
            bundle: bundle.clone(),
        },
        grants: Grants {
            settings: true,
            ..Grants::default()
        },
    };
    let runtime = Runtime {
        socket: PathBuf::from("/run/oxyos-test"),
        bloom_host: Some(PathBuf::from("/home/someone/host")),
        bloom_bundle: Some(PathBuf::from("/home/someone/bundle.js")),
        runtimes: PathBuf::from("/nonexistent-runtimes"),
        window: (960, 600),
        env: Vec::new(),
    };
    let trusted = as_if_root(&dir.0);
    let (application, real) = pinned(&bloom, &system, &runtime, &trusted).unwrap();
    assert_eq!(real.bloom_host, Some(host.clone()));
    assert_eq!(
        application.launch,
        Launch::Bloom {
            module: "Settings".into(),
            bundle: Some(bundle.clone()),
            runtime: None
        }
    );
    let only_host = |path: &Path| path == host;
    assert!(pinned(&bloom, &system, &runtime, &only_host).is_none());
    // What it runs must be what the policy says it is: a native client is not a Bloom one.
    let native = SystemApplication {
        runs: Runs::Exec(shell.clone()),
        ..system.clone()
    };
    assert!(pinned(&bloom, &native, &runtime, &trusted).is_none());
}

/// A filesystem as `lstat` would describe it, for the trust rule alone.
fn filesystem(entries: &[(&str, u32, u32, Kind)]) -> impl Fn(&Path) -> Option<Status> {
    let map: BTreeMap<PathBuf, Status> = entries
        .iter()
        .map(|&(path, uid, mode, kind)| (PathBuf::from(path), Status { uid, mode, kind }))
        .collect();
    move |path| map.get(path).copied()
}

#[test]
fn only_root_owned_entries_in_root_only_directories_are_trusted() {
    use Kind::*;
    let system = [
        ("/", 0, 0o755, Directory),
        ("/usr", 0, 0o755, Directory),
        ("/usr/share", 0, 0o755, Directory),
        ("/usr/share/oxyos", 0, 0o755, Directory),
        ("/usr/share/oxyos/applications", 0, 0o755, Directory),
        (
            "/usr/share/oxyos/applications/so.oxy.settings.desktop",
            0,
            0o644,
            File,
        ),
    ];
    let entry = Path::new("/usr/share/oxyos/applications/so.oxy.settings.desktop");
    assert!(trusted_by(entry, &filesystem(&system)));
    // Each way anyone but root could change what the path holds.
    let changed = |index: usize, uid: u32, mode: u32, kind: Kind| {
        let mut entries = system;
        entries[index] = (entries[index].0, uid, mode, kind);
        trusted_by(entry, &filesystem(&entries))
    };
    assert!(!changed(5, 1000, 0o644, File), "the user's entry");
    assert!(!changed(5, 0, 0o664, File), "an entry its group may write");
    assert!(!changed(5, 0, 0o646, File), "an entry anyone may write");
    assert!(!changed(5, 0, 0o777, Link), "an entry that is a link");
    assert!(
        !changed(3, 1000, 0o755, Directory),
        "a directory of the user's"
    );
    assert!(
        !changed(2, 0, 0o775, Directory),
        "a directory its group may write"
    );
    assert!(
        !changed(0, 0, 0o1777, Directory),
        "a directory anyone may write"
    );
    assert!(!changed(1, 0, 0o777, Link), "a link on the way");
    // Not there, or a path that is not plain.
    assert!(!trusted_by(
        Path::new("/usr/share/oxyos/applications/none.desktop"),
        &filesystem(&system)
    ));
    assert!(!trusted_by(
        Path::new("/usr/share/oxyos/applications/../applications/so.oxy.settings.desktop"),
        &filesystem(&system)
    ));
    assert!(!trusted_by(
        Path::new("usr/share/oxyos/applications/so.oxy.settings.desktop"),
        &filesystem(&system)
    ));
}

#[test]
fn the_filesystem_rule_sees_files_directories_and_links_as_they_are() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Directory::new("lstat", &[("so.oxy.settings.desktop", SETTINGS)]);
    let entry = dir.0.join("so.oxy.settings.desktop");
    std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o640)).unwrap();
    let link = dir.0.join("so.oxy.link.desktop");
    std::os::unix::fs::symlink(&entry, &link).unwrap();
    let me = lstat(&dir.0).unwrap().uid;
    assert_eq!(
        lstat(&entry),
        Some(Status {
            uid: me,
            mode: 0o640,
            kind: Kind::File
        })
    );
    assert_eq!(lstat(&link).map(|s| s.kind), Some(Kind::Link));
    assert_eq!(lstat(&dir.0).map(|s| s.kind), Some(Kind::Directory));
    // A user's files are never an administrator's (and a temporary directory anyone may write).
    assert!(!administrator_written(&entry));
    assert!(!administrator_written(&link));
    // Directories' links are resolved; the entry's own name is kept.
    assert_eq!(
        real_path(&link),
        Some(dir.0.canonicalize().unwrap().join("so.oxy.link.desktop"))
    );
}

/// The trust rule over the real filesystem, with `system` (and the directories above it) taken
/// as root's: what the tests cannot make root's is played, everything else is as it is.
fn as_if_root(system: &Path) -> impl Fn(&Path) -> bool {
    let system = system.canonicalize().unwrap();
    move |path| {
        trusted_by(path, &|p: &Path| {
            lstat(p).map(|status| {
                if p.starts_with(&system) || system.starts_with(p) {
                    Status {
                        uid: 0,
                        mode: status.mode & !0o022,
                        ..status
                    }
                } else {
                    status
                }
            })
        })
    }
}

#[test]
fn an_applications_directories_are_its_own_under_the_base_directories() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| std::ffi::OsString::from(value))
        }
    };
    let data = |pairs, id| base_directory(&env(pairs), "XDG_DATA_HOME", ".local/share", id);
    assert_eq!(
        data(&[("XDG_DATA_HOME", "/d"), ("HOME", "/h")], "so.oxy.stash"),
        Some(PathBuf::from("/d/oxyos/apps/so.oxy.stash"))
    );
    // Without it, or with a relative one (ignored, as the base directory specification says): home's.
    assert_eq!(
        data(&[("HOME", "/h")], "so.oxy.stash"),
        Some(PathBuf::from("/h/.local/share/oxyos/apps/so.oxy.stash"))
    );
    assert_eq!(
        data(&[("XDG_DATA_HOME", "rel"), ("HOME", "/h")], "so.oxy.stash"),
        Some(PathBuf::from("/h/.local/share/oxyos/apps/so.oxy.stash"))
    );
    assert_eq!(data(&[], "so.oxy.stash"), None);
    // An id that is not one never names a directory, least of all one outside.
    for id in ["", "..", "../x", "a/b", ".hidden"] {
        assert_eq!(data(&[("HOME", "/h")], id), None, "{id}");
    }
    assert_eq!(
        base_directory(
            &env(&[("HOME", "/h")]),
            "XDG_CACHE_HOME",
            ".cache",
            "so.oxy.stash"
        ),
        Some(PathBuf::from("/h/.cache/oxyos/apps/so.oxy.stash"))
    );
}

// ---- runtimes and packages (docs/adr/0030-installing-applications.mdx, decisions 1 and 2) -----

/// A runtime of `abi` in `runtimes`, as the image lays one out: its description, its host and its
/// compiler (here, programs that do nothing).
fn lay_out_runtime(runtimes: &Path, abi: u32, description: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = runtimes.join(abi.to_string());
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("runtime.json"), description).unwrap();
    for program in ["oxyos-bloom-host", "hermesc"] {
        std::fs::write(dir.join(program), "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(dir.join(program), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    dir
}

#[test]
fn a_runtime_is_its_directory_its_description_its_host_and_compiler() {
    let dir = Directory::new("runtimes", &[]);
    let one = lay_out_runtime(&dir.0, 1, r#"{"abi":1,"api":2}"#);
    let found = runtime(&dir.0, 1).unwrap();
    assert_eq!(found.level, oxyos_package::Runtime { abi: 1, api: 2 });
    assert_eq!(found.host, one.join("oxyos-bloom-host"));
    assert_eq!(found.hermesc, one.join("hermesc"));
    // A runtime the machine lacks says which.
    let lacking = runtime(&dir.0, 2).unwrap_err();
    assert!(
        lacking.0.contains("runtime 2 is not on this machine"),
        "{lacking}"
    );
    // A directory whose description says another abi is not that runtime.
    lay_out_runtime(&dir.0, 3, r#"{"abi":4,"api":0}"#);
    assert!(runtime(&dir.0, 3).is_err());
    // Nor one without its host or its compiler.
    let five = lay_out_runtime(&dir.0, 5, r#"{"abi":5,"api":0}"#);
    std::fs::remove_file(five.join("hermesc")).unwrap();
    assert!(runtime(&dir.0, 5).is_err());
    std::fs::create_dir_all(dir.0.join("01")).unwrap();
    std::fs::create_dir_all(dir.0.join("+6")).unwrap();
    let (runtimes, errors) = installed_runtimes(&dir.0);
    assert_eq!(
        runtimes.iter().map(|r| r.level).collect::<Vec<_>>(),
        [oxyos_package::Runtime { abi: 1, api: 2 }]
    );
    assert_eq!(errors.len(), 4, "{errors:?}");
}

#[test]
fn an_entry_names_the_runtime_it_was_built_for() {
    let entry = |runtime: &str| {
        parse(
            "so.oxy.inbox",
            &format!("[Desktop Entry]\nType=Application\nName=Inbox\nX-OxyOS-Bloom-Module=Inbox\nX-OxyOS-Runtime={runtime}\n"),
            Path::new("/apps"),
            &[],
        )
    };
    assert_eq!(
        entry("1").unwrap().unwrap().launch,
        Launch::Bloom {
            module: "Inbox".into(),
            bundle: None,
            runtime: Some(1)
        }
    );
    for refused in ["0", "01", "+1", "-1", "x", "", "1.0", "4294967296"] {
        assert!(
            entry(refused).is_err(),
            "X-OxyOS-Runtime={refused} was accepted"
        );
    }
}

#[test]
fn an_application_runs_on_its_runtimes_host() {
    let dir = Directory::new("runtime-launch", &[]);
    let runtimes = dir.0.join("runtimes");
    lay_out_runtime(&runtimes, 1, r#"{"abi":1,"api":0}"#);
    let application = |runtime| Application {
        id: "so.oxy.inbox".into(),
        name: "Inbox".into(),
        icon: None,
        launch: Launch::Bloom {
            module: "Inbox".into(),
            bundle: Some(dir.0.join("bundle.js")),
            runtime,
        },
        pinned: None,
        origin: Origin::Development,
        opens: Vec::new(),
    };
    let session = Runtime {
        socket: PathBuf::from("/run/oxyos-test"),
        bloom_host: Some(PathBuf::from("/session/host")),
        bloom_bundle: None,
        runtimes: runtimes.clone(),
        window: (960, 600),
        env: Vec::new(),
    };
    let program = |runtime| {
        command(&application(runtime), &session).map(|command| PathBuf::from(command.get_program()))
    };
    assert_eq!(program(Some(1)), Ok(runtimes.join("1/oxyos-bloom-host")));
    // A development entry naming no runtime runs on the session's host.
    assert_eq!(program(None), Ok(PathBuf::from("/session/host")));
    // A runtime the machine lacks: not started, and said why.
    let lacking = program(Some(2)).unwrap_err();
    assert!(
        lacking
            .0
            .starts_with("so.oxy.inbox: runtime 2 is not on this machine"),
        "{lacking}"
    );
}

/// A package's canonical manifest (keys sorted, no space), naming its source bundle.
fn manifest_of(id: &str, module: &str) -> String {
    let hash = "0".repeat(64);
    format!(
        r#"{{"application":"00000000-0000-0000-0000-000000000000","architectures":["any"],"entry":{{"bundle":"bundle/index.js","module":"{module}"}},"files":[{{"path":"bundle/index.js","sha256":"{hash}","size":1}},{{"path":"icon.png","sha256":"{hash}","size":1}}],"icon":"icon.png","id":"{id}","name":"Inbox","runtime":{{"abi":1,"api":0}},"version":"1.0","versionCode":1}}"#
    )
}

#[test]
fn a_packages_bytecode_is_known_by_its_source_bundles_url() {
    let dir = Directory::new(
        "script-url",
        &[("manifest.json", &manifest_of("so.oxy.inbox", "Inbox"))],
    );
    assert!(
        oxyos_package::Manifest::parse(manifest_of("so.oxy.inbox", "Inbox").as_bytes()).is_ok()
    );
    std::fs::write(dir.0.join("index.hbc"), "").unwrap();
    assert_eq!(
        script_url(&dir.0.join("index.hbc")),
        dir.0.join("bundle/index.js")
    );
    // Any other bundle is itself: one not named index.hbc, or one with no manifest that reads.
    assert_eq!(script_url(&dir.0.join("index.js")), dir.0.join("index.js"));
    std::fs::write(dir.0.join("manifest.json"), "{}").unwrap();
    assert_eq!(
        script_url(&dir.0.join("index.hbc")),
        dir.0.join("index.hbc")
    );
}

#[test]
fn an_application_whose_package_is_not_there_is_not_installed() {
    let first = Directory::new(
        "not-installed",
        &[(
            "so.oxy.stash.desktop",
            "[Desktop Entry]\nType=Application\nName=Stash\nIcon=icon.png\nX-OxyOS-Bloom-Module=Stash\nX-OxyOS-Bloom-Bundle=packages/so.oxy.stash/index.hbc\nX-OxyOS-Runtime=1\n",
        )],
    );
    let later = Directory::new(
        "not-installed-later",
        &[(
            "so.oxy.stash.desktop",
            "[Desktop Entry]\nType=Application\nName=Not Stash\nExec=/bin/true\n",
        )],
    );
    let (apps, skipped) = registry(
        &[first.0.clone(), later.0.clone()],
        &[],
        &Policy::none(),
        None,
        &|_| false,
    );
    // Said, not listed, and its id still the first directory's.
    assert!(apps.is_empty(), "{apps:?}");
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert!(skipped[0].contains("not installed"), "{skipped:?}");
    // Once its package is there, it is.
    let package = first.0.join("packages/so.oxy.stash");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("index.hbc"), "").unwrap();
    let (apps, skipped) = registry(&[first.0.clone()], &[], &Policy::none(), None, &|_| false);
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(
        apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["Stash"]
    );
}

#[test]
fn a_system_application_naming_a_runtime_runs_that_runtimes_host_only() {
    let dir = Directory::new("system-runtime", &[]);
    let root = dir.0.canonicalize().unwrap();
    let runtimes = root.join("runtimes");
    let one = lay_out_runtime(&runtimes, 1, r#"{"abi":1,"api":0}"#);
    let bundle = root.join("index.hbc");
    std::fs::write(&bundle, "").unwrap();
    let other_host = root.join("other-host");
    std::fs::write(&other_host, "").unwrap();
    let application = Application {
        id: "so.oxy.settings".into(),
        name: "Settings".into(),
        icon: None,
        launch: Launch::Bloom {
            module: "OxyOSSettings".into(),
            bundle: None,
            runtime: Some(1),
        },
        pinned: None,
        origin: Origin::System,
        opens: Vec::new(),
    };
    let system = |host: &Path| SystemApplication {
        entry: root.join("so.oxy.settings.desktop"),
        runs: Runs::Bloom {
            host: host.to_path_buf(),
            bundle: bundle.clone(),
        },
        grants: Grants {
            settings: true,
            ..Grants::default()
        },
    };
    let session = Runtime {
        socket: PathBuf::from("/run/oxyos-test"),
        bloom_host: None,
        bloom_bundle: None,
        runtimes,
        window: (960, 600),
        env: Vec::new(),
    };
    let trusted = as_if_root(&dir.0);
    let (pinned_app, pinned_runtime) = pinned(
        &application,
        &system(&one.join("oxyos-bloom-host")),
        &session,
        &trusted,
    )
    .unwrap();
    assert_eq!(
        pinned_runtime.bloom_host,
        Some(one.join("oxyos-bloom-host"))
    );
    assert_eq!(
        pinned_app.launch,
        Launch::Bloom {
            module: "OxyOSSettings".into(),
            bundle: Some(bundle.clone()),
            runtime: None
        }
    );
    // A policy naming another host than the entry's runtime's: not started.
    assert!(pinned(&application, &system(&other_host), &session, &trusted).is_none());
}

// ---- the install service's applications (docs/adr/0030-installing-applications.mdx, decision 7)

/// An entry as the install service writes one for an installed version (`entry_text`), its bundle
/// in `version`, with what the test adds.
fn installed_text(name: &str, version: &Path, more: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nIcon={}\nX-OxyOS-Bloom-Module=Inbox\nX-OxyOS-Bloom-Bundle={}\nX-OxyOS-Runtime=1\n{more}",
        version.join("icon.png").display(),
        version.join("index.hbc").display()
    )
}

#[test]
fn the_installed_applications_are_where_the_install_service_writes_them() {
    let layout = oxyos_install::state::Layout::new(Path::new(INSTALLED));
    assert_eq!(
        installed_applications(Path::new(INSTALLED), "oxy-abc"),
        layout.user_applications("oxy-abc")
    );
    assert_eq!(INSTALLED, oxyos_install::state::STATE_DIRECTORY);
    // This process's user, by the account database.
    let user = session_user().expect("this process's user is in /etc/passwd");
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap();
    assert!(passwd
        .lines()
        .any(|line| line.starts_with(&format!("{user}:"))));
}

#[test]
fn an_installed_application_is_the_install_services_and_comes_between() {
    let state = Directory::new("installed-state", &[]);
    let version = state.0.join("packages/so.oxy.inbox/12");
    std::fs::create_dir_all(&version).unwrap();
    std::fs::write(version.join("index.hbc"), "").unwrap();
    let installed = installed_applications(&state.0, "oxy-test");
    std::fs::create_dir_all(&installed).unwrap();
    let write = |id: &str, text: &str| {
        std::fs::write(installed.join(format!("{id}.desktop")), text).unwrap()
    };
    write(
        "so.oxy.inbox",
        &installed_text(
            "Inbox",
            &version,
            "X-OxyOS-Package-Version=12\nX-OxyOS-Grants=settings\n",
        ),
    );
    write(
        "so.oxy.settings",
        &installed_text("Not Settings", &version, "X-OxyOS-Package-Version=1\n"),
    );
    write(
        "so.oxy.unversioned",
        &installed_text("Unversioned", &version, ""),
    );
    write("so.oxy.native", "[Desktop Entry]\nType=Application\nName=Native\nExec=/bin/true\nX-OxyOS-Package-Version=1\n");
    std::fs::write(
        installed.parent().unwrap().join("so.oxy.inbox.json"),
        r#"{"format":1,"id":"so.oxy.inbox","capabilities":{"network":"granted","files":"undecided","camera":"denied"}}"#,
    )
    .unwrap();
    let development = Directory::new(
        "installed-development",
        &[
            (
                "so.oxy.inbox.desktop",
                "[Desktop Entry]\nType=Application\nName=Mine\nExec=/bin/true\n",
            ),
            (
                "so.oxy.other.desktop",
                "[Desktop Entry]\nType=Application\nName=Other\nExec=/bin/true\n",
            ),
        ],
    );
    let system = Directory::new("installed-system", &[("so.oxy.settings.desktop", SETTINGS)]);
    let policy = Policy::parse(&policy_of(
        "so.oxy.settings",
        &system.0.canonicalize().unwrap(),
        Path::new("/bin/sh"),
        r#""settings""#,
    ))
    .unwrap();
    let state_root = state.0.canonicalize().unwrap();
    let root = {
        let system = as_if_root(&system.0);
        move |path: &Path| path.starts_with(&state_root) || system(path)
    };
    let installed = installed.canonicalize().unwrap();
    let (apps, skipped) = registry(
        &[development.0.clone()],
        &[],
        &policy,
        Some(&installed),
        &root,
    );
    let listed: Vec<(&str, &str, &Origin)> = apps
        .iter()
        .map(|a| (a.id.as_str(), a.name.as_str(), &a.origin))
        .collect();
    // The system's Settings over the installed one; the installed Inbox over the development one.
    assert_eq!(
        listed,
        [
            (
                "so.oxy.inbox",
                "Inbox",
                &Origin::Installed { version_code: 12 }
            ),
            ("so.oxy.other", "Other", &Origin::Development),
            ("so.oxy.settings", "Settings", &Origin::System),
        ]
    );
    assert!(apps
        .iter()
        .all(|a| a.origin.verified() != (a.id == "so.oxy.other")));
    // Said: the installed Settings, the one naming no version, the native one.
    assert_eq!(skipped.len(), 3, "{skipped:?}");
    // The same entries, not root's: none of them, and never the development Inbox instead.
    let (apps, _) = registry(
        &[development.0.clone()],
        &[],
        &Policy::none(),
        Some(&installed),
        &|_| false,
    );
    assert_eq!(
        apps.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        ["so.oxy.other"]
    );

    // Launched: installed, its version, its account's decisions, and no grant whatever its entry
    // says.
    let runtimes = state.0.join("runtimes");
    lay_out_runtime(&runtimes, 1, r#"{"abi":1,"api":0}"#);
    let mut launcher = Launcher::new(
        Runtime {
            socket: PathBuf::from("/run/oxyos-test"),
            bloom_host: None,
            bloom_bundle: None,
            runtimes,
            window: (960, 600),
            env: Vec::new(),
        },
        vec![development.0.clone()],
        vec![],
    )
    .with_installed_fixture(installed.clone());
    launcher.policy = state.0.join("no-policy.json");
    let pid = launcher.launch("so.oxy.inbox").unwrap();
    let launched = launcher.application_of(pid).unwrap().clone();
    assert_eq!(launched.origin, Origin::Installed { version_code: 12 });
    assert_eq!(launched.grants, Grants::default());
    assert_eq!(
        launched
            .capabilities
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>(),
        [
            ("camera", "denied"),
            ("files", "undecided"),
            ("network", "granted")
        ]
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut reaped = Vec::new();
    while reaped.is_empty() && std::time::Instant::now() < deadline {
        reaped = launcher.reap();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        reaped
            .iter()
            .map(|(p, l)| (*p, l.id.as_str()))
            .collect::<Vec<_>>(),
        [(pid, "so.oxy.inbox")]
    );
}

#[test]
fn the_registrys_sources_are_followed_even_before_they_are_there() {
    let dir = Directory::new("watch", &[]);
    let installed = installed_applications(&dir.0, "oxy-test");
    let development = dir.0.join("development");
    std::fs::create_dir_all(&development).unwrap();
    let mut watch = Watch::new(sources(
        &[development.clone()],
        &Policy::none(),
        Some(&installed),
    ))
    .unwrap();
    assert!(!watch.changed());
    // A development entry written.
    std::fs::write(development.join("so.oxy.x.desktop"), "[Desktop Entry]\n").unwrap();
    assert!(watch.changed());
    assert!(!watch.changed());
    // The account's directory, not there when the watch began: its parents made, then an entry.
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    assert!(watch.changed());
    std::fs::create_dir(&installed).unwrap();
    assert!(watch.changed());
    std::fs::write(installed.join("so.oxy.inbox.desktop"), "[Desktop Entry]\n").unwrap();
    assert!(watch.changed());
    // Removed.
    std::fs::remove_file(installed.join("so.oxy.inbox.desktop")).unwrap();
    assert!(watch.changed());
    assert!(!watch.changed());
}

#[test]
fn the_host_is_told_what_an_installed_version_was_granted() {
    let decisions: BTreeMap<String, String> = [
        ("network", "granted"),
        ("notifications", "undecided"),
        ("files", "denied"),
        ("oxy-account", "granted"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    assert_eq!(
        launch_capabilities(&Origin::Installed { version_code: 3 }, &decisions),
        ("installed", "network,oxy-account".to_string())
    );
    assert_eq!(
        launch_capabilities(&Origin::System, &decisions),
        ("system", String::new())
    );
    assert_eq!(
        launch_capabilities(&Origin::Development, &BTreeMap::new()),
        ("development", String::new())
    );
}
