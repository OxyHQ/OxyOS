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
    let (apps, skipped) = registry(&[user.0.clone(), system.0.clone()], &[]);
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
        find(&[system.0.clone()], &[], "so.oxy.allo").map(|a| a.name),
        Some("Allo".into())
    );
    assert_eq!(find(&[system.0.clone()], &[], "so.oxy.none"), None);
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
    assert_eq!(launcher.application_of(pid), Some("so.oxy.writer"));
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
