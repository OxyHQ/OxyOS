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
    assert!(!defaults.reduce_motion && !defaults.screen_reader);
    // The system's language and region, the region's hour cycle (ADR 0040).
    assert_eq!(
        (defaults.language.as_deref(), defaults.region.as_deref()),
        (None, None)
    );
    assert_eq!(defaults.hour_cycle, HourCycle::Auto);
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
         wallpaper = horizon\n\
         wallpaper.BOE-0A4B-00000001.2 = color:1f4fd8\n\
         no equals sign\n\
         screen_reader = true\n",
    );
    assert_eq!(preferences.appearance, Appearance::Dark);
    assert_eq!(preferences.accent, "ocean-blue");
    assert!(!preferences.reduce_motion, "`yes` is not a boolean here");
    assert_eq!(preferences.lock_after, 5, "more than a day is refused");
    assert_eq!(preferences.wallpapers.default.text(), "horizon");
    assert_eq!(
        preferences.wallpapers.shown("BOE-0A4B-00000001", 2).text(),
        "color:1f4fd8"
    );
    assert!(preferences.screen_reader);
}

#[test]
fn the_retired_clock_24h_is_an_unknown_key_dropped_on_the_next_write() {
    let preferences = Preferences::parse("clock_24h = false\nlock_after = 10\n");
    assert_eq!(preferences.lock_after, 10);
    assert!(!preferences.to_text().contains("clock_24h"));
    assert_eq!(
        Preferences::default().set("clock_24h", "true"),
        Err(Refused::UnknownKey)
    );
}

/// `minimize_effect` (4 November 2026, ADR 0043) chose the genie or a scale; since 5 November the
/// genie is the only one. A file written with it, either value, still reads whole, the line
/// dropped on the next write.
#[test]
fn the_retired_minimize_effect_is_an_unknown_key_dropped_on_the_next_write() {
    let directory = scratch("retired-minimize-effect");
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("preferences");
    for effect in ["scale", "genie"] {
        std::fs::write(
            &file,
            format!(
                "# OxyOS preferences, written by the session.\n\
                 appearance = dark\n\
                 reduce_motion = true\n\
                 minimize_effect = {effect}\n\
                 reduce_transparency = true\n\
                 lock_after = 10\n"
            ),
        )
        .unwrap();
        let preferences = Preferences::load(&file);
        assert_eq!(preferences.appearance, Appearance::Dark);
        assert!(preferences.reduce_motion && preferences.reduce_transparency);
        assert_eq!(preferences.lock_after, 10, "the lines after it are read");
        preferences.save(&file).unwrap();
        let written = std::fs::read_to_string(&file).unwrap();
        assert!(!written.contains("minimize_effect"), "{written}");
        assert_eq!(Preferences::load(&file), preferences);
    }
    assert!(!KEYS.contains(&"minimize_effect"));
    assert_eq!(
        Preferences::default().set("minimize_effect", "genie"),
        Err(Refused::UnknownKey)
    );
    assert_eq!(Preferences::default().get("minimize_effect"), None);
    let _ = std::fs::remove_dir_all(&directory);
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
    // A wallpaper is a choice (docs/adr/0048-wallpapers.mdx), a path of the person's included.
    assert_eq!(
        preferences.set("wallpaper", "photo:fill:000000:/home/ana/Pictures/a.jpg"),
        Ok(())
    );
    assert_eq!(
        preferences.set("wallpaper", "../../secret"),
        Err(Refused::BadValue)
    );
    assert_eq!(preferences.set("wallpaper", ""), Ok(()));
    assert_eq!(preferences.wallpapers.default, wallpaper::Choice::Default);
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
        ("caret_blink", "false"),
        ("ui_sounds", "false"),
        ("trackpad_natural_scrolling", "false"),
        ("mouse_natural_scrolling", "true"),
        ("mission_control_swipe", "four"),
        ("app_expose_swipe", "off"),
        ("desktops_swipe", "off"),
        ("launchpad_pinch", "false"),
        ("wallpaper", "horizon/night"),
        ("wallpaper_all_desktops", "false"),
        ("lock_after", "15"),
        ("screen_reader", "true"),
        ("language", "es"),
        ("region", "MX"),
        ("hour_cycle", "h12"),
        ("do_not_disturb", "true"),
        ("notification_previews", "always"),
        ("empty_trash_automatically", "true"),
    ] {
        preferences.set(key, value).unwrap();
        assert_eq!(preferences.get(key).as_deref(), Some(value));
    }
    // And a display's and a desktop's wallpapers, the session's lines.
    preferences.wallpapers.choose(
        &wallpaper::Place {
            display: "DEL-A0B4-7Y9K2N3",
            primary: false,
            desktop: 3,
        },
        &[],
        wallpaper::Choice::parse("folder:tile:202020:15:/home/ana/Fotos").unwrap(),
    );
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
        mode: Some(DisplayMode {
            width,
            height,
            refresh_mhz,
        }),
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
fn a_display_left_at_its_own_mode_keeps_its_scale_alone() {
    // Only the scale chosen: the mode stays the display's own (its native resolution at its fastest
    // refresh), so a monitor whose card drives it faster later is shown faster.
    let mut preferences = Preferences::default();
    preferences.keep_display(PANEL, DisplaySettings::default_mode(150));
    let text = preferences.to_text();
    assert!(text.contains("\ndisplay.BOE-0A4B-00000001 = default 1.25\n"));
    let read = Preferences::parse(&text);
    assert_eq!(
        read.display(PANEL),
        Some(DisplaySettings::default_mode(150))
    );
    // A mode chosen is kept as chosen, whatever the default is.
    assert_eq!(
        DisplaySettings::parse("2560x1440@59951 1").unwrap().mode,
        Some(DisplayMode {
            width: 2560,
            height: 1440,
            refresh_mhz: 59_951
        })
    );
    assert_eq!(DisplaySettings::parse("defaults 1"), None);
    assert_eq!(DisplaySettings::parse("default"), None);
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
    assert_eq!(
        preferences.display("D-2").unwrap().mode.unwrap().width,
        1280
    );
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

#[test]
fn the_persons_lists_of_applications_are_kept_whole_and_read_back() {
    let mut preferences = Preferences::default();
    // Until the person changes it, the dock is the registry's: no list.
    assert_eq!(preferences.dock, None);
    assert!(!preferences.to_text().contains("dock"));
    let ids = |list: &[&str]| list.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    preferences
        .set_list(
            ApplicationList::Dock,
            &ids(&["so.oxy.dropsea", "so.oxy.allo"]),
        )
        .unwrap();
    preferences
        .set_list(ApplicationList::OpenAtLogin, &ids(&["so.oxy.inbox"]))
        .unwrap();
    let text = preferences.to_text();
    assert!(text.contains("dock = so.oxy.dropsea,so.oxy.allo\n"));
    assert!(text.contains("open_at_login = so.oxy.inbox\n"));
    let back = Preferences::parse(&text);
    assert_eq!(back.dock, Some(ids(&["so.oxy.dropsea", "so.oxy.allo"])));
    assert_eq!(back.open_at_login, ids(&["so.oxy.inbox"]));
    // An empty dock is the person's choice too, and stays one.
    preferences.set_list(ApplicationList::Dock, &[]).unwrap();
    assert_eq!(
        Preferences::parse(&preferences.to_text()).dock,
        Some(vec![])
    );
    // A bad id, one twice or too many: refused, nothing changed.
    for bad in [
        ids(&["so.oxy.dropsea", "so.oxy.dropsea"]),
        ids(&["not an id"]),
        ids(&[".hidden"]),
        (0..=MAX_APPLICATIONS)
            .map(|n| format!("so.oxy.app{n}"))
            .collect(),
    ] {
        assert_eq!(
            preferences.set_list(ApplicationList::OpenAtLogin, &bad),
            Err(Refused::BadValue)
        );
    }
    assert_eq!(preferences.open_at_login, ids(&["so.oxy.inbox"]));
    // Whose notifications play no sound (docs/adr/0044-system-sounds.mdx): none until the person
    // turns one off, then kept and read back.
    assert!(preferences.notification_sounds_off.is_empty());
    assert!(!preferences.to_text().contains("notification_sounds_off"));
    preferences
        .set_list(
            ApplicationList::NotificationSoundsOff,
            &ids(&["so.oxy.allo"]),
        )
        .unwrap();
    assert!(preferences
        .to_text()
        .contains("notification_sounds_off = so.oxy.allo\n"));
    assert_eq!(
        Preferences::parse(&preferences.to_text()).notification_sounds_off,
        ids(&["so.oxy.allo"])
    );
    // A bad line in the file keeps the default.
    assert_eq!(
        Preferences::parse("dock = so.oxy.dropsea,bad id\n").dock,
        None
    );
}

#[test]
fn input_sources_are_the_input_methods_added_and_the_current_is_one_of_them_or_the_layout() {
    let mut preferences = Preferences::default();
    assert_eq!(preferences.get("input_sources").unwrap(), "");
    assert_eq!(preferences.get("input_source").unwrap(), "layout");
    // Only the input methods OxyOS has, each once.
    assert_eq!(
        preferences.set("input_sources", "pinyin,klingon"),
        Err(Refused::BadValue)
    );
    assert_eq!(
        preferences.set("input_sources", "pinyin,pinyin"),
        Err(Refused::BadValue)
    );
    preferences.set("input_sources", "pinyin,korean").unwrap();
    // The current one is one of them, or the layout.
    assert_eq!(
        preferences.set("input_source", "japanese"),
        Err(Refused::BadValue)
    );
    preferences.set("input_source", "korean").unwrap();
    assert_eq!(preferences.input_source.as_deref(), Some("korean"));
    // Written and read back, in that order.
    let written = Preferences::parse(&preferences.to_text());
    assert_eq!(written.input_sources, ["pinyin", "korean"]);
    assert_eq!(written.input_source.as_deref(), Some("korean"));
    // The current one removed: the layout is current.
    preferences.set("input_sources", "pinyin").unwrap();
    assert_eq!(preferences.input_source, None);
    preferences.set("input_source", "layout").unwrap();
    assert_eq!(preferences.get("input_source").unwrap(), "layout");
}

#[test]
fn interface_sound_effects_are_on_by_default_as_on_a_mac() {
    let mut preferences = Preferences::default();
    assert!(preferences.ui_sounds);
    assert_eq!(preferences.get("ui_sounds").as_deref(), Some("true"));
    assert_eq!(preferences.set("ui_sounds", "off"), Err(Refused::BadValue));
    assert!(preferences.ui_sounds);
    preferences.set("ui_sounds", "false").unwrap();
    assert!(!Preferences::parse(&preferences.to_text()).ui_sounds);
}

#[test]
fn the_trash_empties_itself_only_once_the_person_turns_it_on_as_on_a_mac() {
    let mut preferences = Preferences::default();
    assert!(!preferences.empty_trash_automatically);
    assert_eq!(
        preferences.get("empty_trash_automatically").as_deref(),
        Some("false")
    );
    assert_eq!(
        preferences.set("empty_trash_automatically", "30"),
        Err(Refused::BadValue)
    );
    preferences
        .set("empty_trash_automatically", "true")
        .unwrap();
    assert!(Preferences::parse(&preferences.to_text()).empty_trash_automatically);
}

#[test]
fn natural_scrolling_is_on_for_a_trackpad_and_off_for_a_mouse_by_default() {
    let mut preferences = Preferences::default();
    // A Mac's trackpad ships with it; a wheel turned toward the person scrolls down.
    assert!(preferences.trackpad_natural_scrolling);
    assert!(!preferences.mouse_natural_scrolling);
    assert_eq!(
        preferences.get("trackpad_natural_scrolling").as_deref(),
        Some("true")
    );
    assert_eq!(
        preferences.get("mouse_natural_scrolling").as_deref(),
        Some("false")
    );
    // Each is a boolean, refused otherwise with nothing changed.
    for key in ["trackpad_natural_scrolling", "mouse_natural_scrolling"] {
        for bad in ["on", "1", "", "True"] {
            assert_eq!(preferences.set(key, bad), Err(Refused::BadValue), "{key}");
        }
    }
    assert_eq!(preferences, Preferences::default());
    // Each is its own: one changed leaves the other.
    preferences
        .set("trackpad_natural_scrolling", "false")
        .unwrap();
    assert!(!preferences.trackpad_natural_scrolling && !preferences.mouse_natural_scrolling);
    preferences.set("mouse_natural_scrolling", "true").unwrap();
    assert!(!preferences.trackpad_natural_scrolling && preferences.mouse_natural_scrolling);
    // Kept in the file and read back; a bad line keeps the default.
    let written = Preferences::parse(&preferences.to_text());
    assert!(!written.trackpad_natural_scrolling && written.mouse_natural_scrolling);
    let bad =
        Preferences::parse("trackpad_natural_scrolling = no\nmouse_natural_scrolling = yes\n");
    assert!(bad.trackpad_natural_scrolling && !bad.mouse_natural_scrolling);
}

#[test]
fn the_trackpads_gestures_are_a_macs_and_mission_control_and_app_expose_share_their_fingers() {
    let mut preferences = Preferences::default();
    // Three fingers up, down and across, and the pinch, as a Mac's trackpad ships them.
    for (key, value) in [
        ("mission_control_swipe", "three"),
        ("app_expose_swipe", "three"),
        ("desktops_swipe", "three"),
        ("launchpad_pinch", "true"),
    ] {
        assert_eq!(preferences.get(key).as_deref(), Some(value), "{key}");
    }
    // Three, four or off; anything else refused with nothing changed.
    for key in [
        "mission_control_swipe",
        "app_expose_swipe",
        "desktops_swipe",
    ] {
        for bad in ["3", "two", "", "Four", "true"] {
            assert_eq!(preferences.set(key, bad), Err(Refused::BadValue), "{key}");
        }
    }
    assert_eq!(
        preferences.set("launchpad_pinch", "on"),
        Err(Refused::BadValue)
    );
    assert_eq!(preferences, Preferences::default());
    // Four for Mission Control is four for App Exposé too, as a Mac's popups go together; the
    // desktops' fingers are their own.
    preferences.set("mission_control_swipe", "four").unwrap();
    assert_eq!(preferences.get("app_expose_swipe").as_deref(), Some("four"));
    assert_eq!(preferences.get("desktops_swipe").as_deref(), Some("three"));
    // App Exposé off leaves Mission Control; turned on again with three, both are three.
    preferences.set("app_expose_swipe", "off").unwrap();
    assert_eq!(
        preferences.get("mission_control_swipe").as_deref(),
        Some("four")
    );
    assert!(!preferences.trackpad_gestures.app_expose);
    // Mission Control's fingers changed while App Exposé is off leave it off.
    preferences.set("mission_control_swipe", "three").unwrap();
    assert_eq!(preferences.get("app_expose_swipe").as_deref(), Some("off"));
    preferences.set("app_expose_swipe", "four").unwrap();
    assert_eq!(
        preferences.get("mission_control_swipe").as_deref(),
        Some("four")
    );
    preferences.set("desktops_swipe", "off").unwrap();
    preferences.set("launchpad_pinch", "false").unwrap();
    assert_eq!(
        preferences.trackpad_gestures,
        TrackpadGestures {
            vertical: SwipeFingers::Four,
            mission_control: true,
            app_expose: true,
            desktops: None,
            launchpad: false,
        }
    );
    // Kept in the file and read back; a bad line keeps the default.
    assert_eq!(Preferences::parse(&preferences.to_text()), preferences);
    let bad = Preferences::parse("desktops_swipe = 4\nlaunchpad_pinch = no\n");
    assert_eq!(bad.trackpad_gestures, TrackpadGestures::default());
    assert_eq!(SwipeFingers::Three.fewest(), 3);
    assert_eq!(SwipeFingers::Four.fewest(), 4);
}

#[test]
fn energy_is_a_macs_by_default_and_each_timer_at_most_three_hours() {
    let defaults = Preferences::default();
    assert!(!defaults.battery_percentage);
    let energy = defaults.energy;
    // The display off after 2 minutes on battery and 10 on the adapter; the machine asleep after
    // 10 on battery and never on the adapter; dimmer on battery; Low Power Mode never.
    assert_eq!(
        (energy.display_off(false), energy.display_off(true)),
        (2, 10)
    );
    assert_eq!((energy.sleep(false), energy.sleep(true)), (10, 0));
    assert!(energy.dim_on_battery);
    assert_eq!(energy.low_power_mode, LowPowerMode::Never);

    let mut preferences = Preferences::default();
    for (key, value) in [
        ("battery_percentage", "true"),
        ("low_power_mode", "battery"),
        ("display_off_battery", "5"),
        ("display_off_adapter", "0"),
        ("sleep_battery", "180"),
        ("sleep_adapter", "60"),
        ("dim_on_battery", "false"),
    ] {
        preferences.set(key, value).unwrap();
        assert_eq!(preferences.get(key).as_deref(), Some(value), "{key}");
    }
    for (key, value) in [
        ("low_power_mode", "sometimes"),
        ("display_off_battery", "181"),
        ("sleep_adapter", "-1"),
        ("dim_on_battery", "1"),
    ] {
        let before = preferences.clone();
        assert_eq!(preferences.set(key, value), Err(Refused::BadValue), "{key}");
        assert_eq!(preferences, before);
    }
    assert_eq!(Preferences::parse(&preferences.to_text()), preferences);
}

#[test]
fn low_power_mode_follows_the_power_source() {
    use LowPowerMode::*;
    assert_eq!(
        [Never, Always, OnBattery, OnAdapter].map(|mode| (mode.on(false), mode.on(true))),
        [(false, false), (true, true), (true, false), (false, true)]
    );
}
