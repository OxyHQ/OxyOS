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
    let es = parse(
        "so.oxy.allo",
        ALLO,
        dir,
        &["es_ES".into(), "es".into()],
        false,
    )
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
    let en = parse(
        "so.oxy.allo",
        ALLO,
        dir,
        &["en_US".into(), "en".into()],
        false,
    )
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
            false,
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
    assert_eq!(parse("so.oxy.x", hidden, dir, &[], false), Ok(None));
    let link = "[Desktop Entry]\nType=Link\nName=X\nURL=https://oxy.so\n";
    assert_eq!(parse("so.oxy.x", link, dir, &[], false), Ok(None));
    assert!(parse("../etc/passwd", ALLO, dir, &[], false).is_err());
    assert!(parse(
        "so.oxy.allo",
        "[Desktop Entry]\nType=Application\nExec=x\n",
        dir,
        &[],
        false
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
    let (apps, skipped) = registry(&[user.0.clone(), system.0.clone()], &[], &|_| false);
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
        find(&[system.0.clone()], &[], &|_| false, "so.oxy.allo").map(|a| a.name),
        Some("Allo".into())
    );
    assert_eq!(
        find(&[system.0.clone()], &[], &|_| false, "so.oxy.none"),
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
            false,
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
    let bloom = parse("so.oxy.allo", ALLO, Path::new("/apps"), &[], false)
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

// ---- grants (docs/adr/0019-settings.mdx) ------------------------------------------------------

const SETTINGS: &str =
    "[Desktop Entry]\nType=Application\nName=Settings\nExec=/bin/true\nX-OxyOS-Grants=settings;\n";

#[test]
fn an_entry_has_the_grants_it_names_only_if_trusted() {
    let dir = Path::new("/");
    let trusted = parse("so.oxy.settings", SETTINGS, dir, &[], true)
        .unwrap()
        .unwrap();
    assert_eq!(trusted.grants, Grants { settings: true });
    let untrusted = parse("so.oxy.settings", SETTINGS, dir, &[], false)
        .unwrap()
        .unwrap();
    assert_eq!(untrusted.grants, Grants::default());
    // Names this system does not know grant nothing; the list may have spaces and no `;`.
    let entry = |list: &str| {
        parse(
            "so.oxy.x",
            &format!("[Desktop Entry]\nType=Application\nName=X\nExec=x\nX-OxyOS-Grants={list}\n"),
            dir,
            &[],
            true,
        )
        .unwrap()
        .unwrap()
        .grants
    };
    assert_eq!(entry("shell;lock;root"), Grants::default());
    assert_eq!(entry("shell; settings"), Grants { settings: true });
    assert_eq!(entry("Settings"), Grants::default());
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
fn a_user_entry_claiming_a_grant_gets_none() {
    let user = Directory::new(
        "grants-user",
        &[
            ("so.oxy.mine.desktop", SETTINGS),
            // The user's own Settings hides the system's, and is the user's.
            ("so.oxy.settings.desktop", SETTINGS),
            ("elsewhere.desktop.txt", SETTINGS),
        ],
    );
    let system = Directory::new(
        "grants-system",
        &[
            ("so.oxy.settings.desktop", SETTINGS),
            ("so.oxy.system.desktop", SETTINGS),
        ],
    );
    // A link in the system's directory to an entry the user can change.
    std::os::unix::fs::symlink(
        user.0.join("elsewhere.desktop.txt"),
        system.0.join("so.oxy.linked.desktop"),
    )
    .unwrap();
    let trusted = as_if_root(&system.0);
    let grants = |directories: &[PathBuf]| -> Vec<(String, bool)> {
        let (apps, skipped) = registry(directories, &[], &trusted);
        assert!(skipped.is_empty(), "{skipped:?}");
        apps.into_iter()
            .map(|app| (app.id, app.grants.settings))
            .collect()
    };
    let mut both = grants(&[user.0.clone(), system.0.clone()]);
    both.sort();
    assert_eq!(
        both,
        [
            ("so.oxy.linked".to_string(), false),
            ("so.oxy.mine".to_string(), false),
            ("so.oxy.settings".to_string(), false),
            ("so.oxy.system".to_string(), true),
        ]
    );
    // Without the user's directory, the system's Settings is granted.
    let mut system_only = grants(&[system.0.clone()]);
    system_only.sort();
    assert_eq!(
        system_only,
        [
            ("so.oxy.linked".to_string(), false),
            ("so.oxy.settings".to_string(), true),
            ("so.oxy.system".to_string(), true),
        ]
    );
}

#[test]
fn a_launch_keeps_its_grants_only_running_an_administrators_program() {
    let shell = Path::new("/bin/sh").canonicalize().unwrap();
    let dir = Directory::new(
        "grants-launch",
        &[
            (
                "so.oxy.settings.desktop",
                "[Desktop Entry]\nType=Application\nName=Settings\nExec=/bin/sh -c \"exit 0\"\nX-OxyOS-Grants=settings\n",
            ),
            (
                "so.oxy.searched.desktop",
                "[Desktop Entry]\nType=Application\nName=Searched\nExec=sh -c \"exit 0\"\nX-OxyOS-Grants=settings\n",
            ),
        ],
    );
    let users = Directory::new("grants-program", &[("program", "#!/bin/sh\nexit 0\n")]);
    let script = users.0.join("program");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(
        dir.0.join("so.oxy.users.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Users\nExec={}\nX-OxyOS-Grants=settings\n",
            script.display()
        ),
    )
    .unwrap();
    let runtime = Runtime {
        socket: PathBuf::from("/run/oxyos-test"),
        bloom_host: None,
        bloom_bundle: None,
        window: (960, 600),
        env: Vec::new(),
    };
    let mut launcher = Launcher::new(runtime.clone(), vec![dir.0.clone()], vec![]);
    let entries = as_if_root(&dir.0);
    let program = shell.clone();
    launcher.trusted = Box::new(move |path| entries(path) || path == program);
    let pid = launcher.launch("so.oxy.settings").unwrap();
    assert_eq!(
        launcher.application_of(pid).map(|l| l.grants),
        Some(Grants { settings: true })
    );
    // A program found on the PATH is whatever the PATH finds: no grant.
    let pid = launcher.launch("so.oxy.searched").unwrap();
    assert_eq!(
        launcher.application_of(pid).map(|l| l.grants),
        Some(Grants::default())
    );
    // Nor a program anyone but root could change: the user's own.
    let pid = launcher.launch("so.oxy.users").unwrap();
    assert_eq!(
        launcher.application_of(pid).map(|l| l.grants),
        Some(Grants::default())
    );
    launcher.end(std::time::Duration::from_secs(5));

    // A Bloom application's host and bundle are what run: both must be an administrator's, and
    // they run by their real paths.
    let bloom = Application {
        id: "so.oxy.settings".into(),
        name: "Settings".into(),
        icon: None,
        launch: Launch::Bloom {
            module: "Settings".into(),
            bundle: None,
        },
        pinned: None,
        grants: Grants { settings: true },
    };
    let host = dir.0.join("host");
    let bundle = dir.0.join("bundle.js");
    std::fs::write(&host, "").unwrap();
    std::fs::write(&bundle, "").unwrap();
    let runtime = Runtime {
        bloom_host: Some(host.clone()),
        bloom_bundle: Some(bundle.clone()),
        ..runtime
    };
    let trusted = as_if_root(&dir.0);
    let (_, real) = pinned(&bloom, &runtime, &trusted).unwrap();
    assert_eq!(real.bloom_host, host.canonicalize().ok());
    let only_host = |path: &Path| path == host.canonicalize().unwrap();
    assert!(pinned(&bloom, &runtime, &only_host).is_none());
    let no_host = Runtime {
        bloom_host: None,
        ..runtime
    };
    assert!(pinned(&bloom, &no_host, &trusted).is_none());
}
