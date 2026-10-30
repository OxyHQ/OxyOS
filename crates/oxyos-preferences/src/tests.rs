use super::*;

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("oxyos-preferences-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

#[test]
fn the_defaults_are_what_nothing_says() {
    let defaults = Preferences::default();
    assert_eq!(Preferences::parse(""), defaults);
    assert_eq!(
        Preferences::load(Path::new("/nonexistent/oxyos/preferences")),
        defaults
    );
    assert_eq!(defaults.appearance, Appearance::Auto);
    assert_eq!(defaults.accent, "oxy");
    assert_eq!(defaults.lock_after, 5);
    assert!(defaults.clock_24h && !defaults.reduce_motion && !defaults.screen_reader);
}

#[test]
fn the_file_is_read_line_by_line_and_bad_lines_keep_their_default() {
    let preferences = Preferences::parse(
        "# a comment\n\
         appearance = dark\n\
         accent=ocean-blue\n\
         reduce_motion = yes\n\
         lock_after = 99999\n\
         volume = 11\n\
         wallpaper = solar-gradients\n\
         no equals sign\n\
         screen_reader = true\n",
    );
    assert_eq!(preferences.appearance, Appearance::Dark);
    assert_eq!(preferences.accent, "ocean-blue");
    assert!(!preferences.reduce_motion, "`yes` is not a boolean here");
    assert_eq!(preferences.lock_after, 5, "more than a day is refused");
    assert_eq!(preferences.wallpaper.as_deref(), Some("solar-gradients"));
    assert!(preferences.screen_reader);
}

#[test]
fn a_change_is_validated_and_a_refused_one_changes_nothing() {
    let mut preferences = Preferences::default();
    assert_eq!(preferences.set("appearance", "light"), Ok(()));
    assert_eq!(
        preferences.set("appearance", "sepia"),
        Err(Refused::BadValue)
    );
    assert_eq!(preferences.appearance, Appearance::Light);
    assert_eq!(preferences.set("accent", "Ocean"), Err(Refused::BadValue));
    assert_eq!(preferences.set("accent", "../etc"), Err(Refused::BadValue));
    assert_eq!(
        preferences.set("accent", &"a".repeat(MAX_VALUE_BYTES + 1)),
        Err(Refused::BadValue)
    );
    assert_eq!(
        preferences.set("wallpaper", "../../secret"),
        Err(Refused::BadValue)
    );
    assert_eq!(preferences.set("wallpaper", ""), Ok(()));
    assert_eq!(preferences.wallpaper, None);
    assert_eq!(preferences.set("lock_after", "0"), Ok(()));
    assert_eq!(preferences.set("lock_after", "-1"), Err(Refused::BadValue));
    assert_eq!(preferences.set("brightness", "1"), Err(Refused::UnknownKey));
    assert_eq!(preferences.lock_after, 0);
}

#[test]
fn what_is_written_reads_back_the_same() {
    let mut preferences = Preferences::default();
    for (key, value) in [
        ("appearance", "dark"),
        ("accent", "jade"),
        ("reduce_motion", "true"),
        ("reduce_transparency", "true"),
        ("wallpaper", "solar-gradients"),
        ("lock_after", "15"),
        ("screen_reader", "true"),
        ("clock_24h", "false"),
    ] {
        preferences.set(key, value).unwrap();
        assert_eq!(preferences.get(key).as_deref(), Some(value));
    }
    assert_eq!(Preferences::parse(&preferences.to_text()), preferences);
    for key in KEYS {
        assert!(
            preferences.to_text().contains(&format!("\n{key} = ")),
            "{key}"
        );
    }

    let directory = scratch("roundtrip");
    let file = directory.join("oxyos").join("preferences");
    preferences.save(&file).unwrap();
    assert_eq!(Preferences::load(&file), preferences);
    // Written whole: nothing is left beside it.
    let left: Vec<_> = std::fs::read_dir(file.parent().unwrap()).unwrap().collect();
    assert_eq!(left.len(), 1);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_file_is_under_the_configuration_directory() {
    assert_eq!(
        path(Some(Path::new("/c")), Some(Path::new("/h"))),
        Some(PathBuf::from("/c/oxyos/preferences"))
    );
    assert_eq!(
        path(None, Some(Path::new("/h"))),
        Some(PathBuf::from("/h/.config/oxyos/preferences"))
    );
    // A relative XDG_CONFIG_HOME is ignored, as the base directory specification says.
    assert_eq!(
        path(Some(Path::new("c")), Some(Path::new("/h"))),
        Some(PathBuf::from("/h/.config/oxyos/preferences"))
    );
    assert_eq!(path(None, None), None);
}
