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
            bundle: None
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
            grants: Grants::default()
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
            grants: Grants { settings: true },
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
        Some(Grants { settings: true })
    );
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
    let (apps, skipped) = registry(&[dir.0.clone()], &[], &Policy::none(), &trusted);
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(
        apps.iter()
            .map(|a| (a.id.as_str(), a.system))
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
    let (apps, skipped) = registry(&[user.0.clone(), system.0.clone()], &[], &policy, &trusted);
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(
        apps.iter()
            .map(|a| (a.id.as_str(), a.name.as_str(), a.system))
            .collect::<Vec<_>>(),
        [("so.oxy.settings", "Settings", true)]
    );
    // The policy's entry the user's: Settings is left out, said, and never the user's instead.
    let (apps, skipped) = registry(&[user.0.clone(), system.0.clone()], &[], &policy, &|_| {
        false
    });
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
    let (apps, _) = registry(&[], &[], &through_link, &trusted);
    assert!(apps.is_empty(), "{apps:?}");
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
        Some(Grants { settings: true })
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
        },
        pinned: None,
        system: true,
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
        grants: Grants { settings: true },
    };
    let runtime = Runtime {
        socket: PathBuf::from("/run/oxyos-test"),
        bloom_host: Some(PathBuf::from("/home/someone/host")),
        bloom_bundle: Some(PathBuf::from("/home/someone/bundle.js")),
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
            bundle: Some(bundle.clone())
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
