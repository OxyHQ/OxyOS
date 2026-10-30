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
    // Notifications show as banners, and the lock screen says only whose they are.
    assert!(!defaults.do_not_disturb);
    assert_eq!(defaults.notification_previews, Previews::Unlocked);
}

#[test]
fn notifications_previews_are_always_or_once_unlocked() {
    let mut preferences = Preferences::default();
    assert_eq!(
        preferences.set("notification_previews", "never"),
        Err(Refused::BadValue)
    );
    assert_eq!(
        preferences.set("do_not_disturb", "on"),
        Err(Refused::BadValue)
    );
    assert_eq!(preferences, Preferences::default());
    assert_eq!(preferences.set("notification_previews", "always"), Ok(()));
    assert_eq!(preferences.notification_previews, Previews::Always);
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
        preferences.set("accent", &"a".repeat(MAX_ACCENT_BYTES + 1)),
        Err(Refused::BadValue)
    );
    assert_eq!(
        preferences.set("accent", &"a".repeat(MAX_ACCENT_BYTES)),
        Ok(())
    );
    // A wallpaper's name may be longer than an accent's.
    assert_eq!(
        preferences.set("wallpaper", &"a".repeat(MAX_VALUE_BYTES)),
        Ok(())
    );
    assert_eq!(
        preferences.set("wallpaper", &"a".repeat(MAX_VALUE_BYTES + 1)),
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
        ("do_not_disturb", "true"),
        ("notification_previews", "always"),
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

const PANEL: &str = "BOE-0A4B-00000001";
const MONITOR: &str = "DEL-A0B4-7Y9K2N3";

fn settings(width: u32, height: u32, refresh_mhz: u32, scale_120: u32) -> DisplaySettings {
    DisplaySettings {
        mode: DisplayMode {
            width,
            height,
            refresh_mhz,
        },
        scale_120,
    }
}

#[test]
fn each_display_keeps_its_mode_and_scale_and_each_set_its_arrangement() {
    let mut preferences = Preferences::default();
    preferences.keep_display(PANEL, settings(1920, 1200, 60_000, 150));
    preferences.keep_display(MONITOR, settings(2560, 1440, 59_951, 120));
    preferences.keep_arrangement(Arrangement {
        displays: vec![(MONITOR.into(), (0, 0)), (PANEL.into(), (2560, 480))],
    });
    let text = preferences.to_text();
    assert!(text.contains("\ndisplay.BOE-0A4B-00000001 = 1920x1200@60000 1.25\n"));
    assert!(text.contains("\ndisplay.DEL-A0B4-7Y9K2N3 = 2560x1440@59951 1\n"));
    assert!(text.contains(
        "\narrangement.BOE-0A4B-00000001+DEL-A0B4-7Y9K2N3 = DEL-A0B4-7Y9K2N3 0,0 BOE-0A4B-00000001 2560,480\n"
    ));
    let read = Preferences::parse(&text);
    assert_eq!(read, preferences);
    assert_eq!(read.display(PANEL), Some(settings(1920, 1200, 60_000, 150)));
    assert_eq!(read.display("nobody"), None);
    // The set's arrangement, whatever order its displays are named in; the primary first.
    let arrangement = read.arrangement([PANEL, MONITOR]).unwrap();
    assert_eq!(arrangement.displays[0].0, MONITOR);
    assert!(read.arrangement([PANEL]).is_none());
    // Settings cannot write them as a preference: they are the session's displays' to change.
    assert_eq!(
        preferences.set("display.BOE-0A4B-00000001", "640x480@60000 1"),
        Err(Refused::UnknownKey)
    );
}

#[test]
fn bad_display_lines_are_nothing() {
    let preferences = Preferences::parse(
        "display.BOE-0A4B-00000001 = 1920x1200@60000 1.25\n\
         display.../../etc = 1920x1200@60000 1\n\
         display.A = 0x1200@60000 1\n\
         display.B = 1920x1200@60000 9\n\
         display.C = 1920x1200 1\n\
         arrangement.A+B = A 0,0\n\
         arrangement.A+B = A 0,0 C 10,0\n\
         arrangement.A+B = A 0,0 A 10,0\n\
         arrangement.A+B = A 0,0 B 2000000,0\n\
         arrangement.A+C = C 0,0 A -1920,0\n\
         appearance = dark\n",
    );
    assert_eq!(preferences.displays.len(), 1);
    assert_eq!(preferences.arrangements.len(), 1);
    assert_eq!(
        preferences.arrangement(["A", "C"]).unwrap().displays,
        [("C".to_string(), (0, 0)), ("A".to_string(), (-1920, 0))]
    );
    assert_eq!(preferences.appearance, Appearance::Dark);
}

#[test]
fn the_displays_changed_longest_ago_go_first() {
    let mut preferences = Preferences::default();
    for n in 0..displays::MAX_DISPLAYS + 2 {
        preferences.keep_display(&format!("D-{n}"), settings(1920, 1080, 60_000, 120));
    }
    // Changing one makes it the latest.
    preferences.keep_display("D-2", settings(1280, 720, 60_000, 120));
    assert_eq!(preferences.displays.len(), displays::MAX_DISPLAYS);
    assert!(preferences.display("D-0").is_none() && preferences.display("D-1").is_none());
    assert_eq!(preferences.displays.last().unwrap().0, "D-2");
    assert_eq!(preferences.display("D-2").unwrap().mode.width, 1280);
}

#[test]
fn a_scale_is_written_as_people_read_it() {
    assert_eq!(displays::scale_text(120), "1");
    assert_eq!(displays::scale_text(150), "1.25");
    assert_eq!(displays::scale_text(180), "1.5");
    assert_eq!(displays::scale_text(210), "1.75");
    assert_eq!(displays::scale_text(240), "2");
    assert_eq!(
        DisplaySettings::parse("1920x1080@60000 1.75")
            .unwrap()
            .scale_120,
        210
    );
}
