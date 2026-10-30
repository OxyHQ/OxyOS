use super::*;
use crate::audio::{parse_volume, volume_text, Wpctl};
use crate::backlight::{level_of, raw_of, Setter};
use oxyos_protocol::{BatteryState, BluetoothState, WifiState};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const STEP: u32 = FULL_LEVEL / STEPS;

/// A volume as read from `wpctl`, which prints two decimals: within half a hundredth.
fn read_volume(value: u32, muted: bool) -> Level {
    Level {
        resolution: FULL_LEVEL.div_ceil(200),
        ..Level::new(value, muted)
    }
}

/// A directory of the test's own, removed first if a previous run left it.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oxyos-controls-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A script standing in for `wpctl`: it appends its arguments to `log` and runs `body`.
fn script(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("wpctl");
    let log = dir.join("log");
    std::fs::write(
        &path,
        format!("#!/bin/sh\necho \"$@\" >> '{}'\n{body}\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn logged(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn keys_move_in_sixteen_steps_to_the_next_level_beyond() {
    // From nothing to all in sixteen presses, and back.
    let mut level = 0;
    for step in 1..=STEPS {
        level = stepped(level, STEPS, true, 0);
        assert_eq!(level, step * STEP);
    }
    assert_eq!(stepped(level, STEPS, true, 0), FULL_LEVEL, "never past all");
    for step in (0..STEPS).rev() {
        level = stepped(level, STEPS, false, 0);
        assert_eq!(level, step * STEP);
    }
    assert_eq!(stepped(0, STEPS, false, 0), 0, "never below nothing");
    // Between two steps (another program set it): to the one beyond, either way.
    assert_eq!(stepped(5 * STEP + 100, STEPS, true, 0), 6 * STEP);
    assert_eq!(stepped(5 * STEP + 100, STEPS, false, 0), 5 * STEP);
    // A fine step is a quarter of one.
    assert_eq!(stepped(5 * STEP, STEPS * 4, true, 0), 5 * STEP + STEP / 4);
    // A keyboard of three brightnesses: two steps.
    assert_eq!(stepped(0, 2, true, 0), FULL_LEVEL / 2);
    assert_eq!(stepped(FULL_LEVEL / 2, 2, true, 0), FULL_LEVEL);
    assert_eq!(steps_of(FULL_LEVEL / 2, STEPS), 8);
    assert_eq!(steps_of(5 * STEP + 100, STEPS), 5);
    assert_eq!(steps_of(FULL_LEVEL, STEPS), 16);
}

#[test]
fn wpctl_says_the_volume_and_its_mute() {
    assert_eq!(parse_volume("Volume: 0.41\n"), Some((26_870, false)));
    assert_eq!(
        parse_volume("Volume: 1.00 [MUTED]\n"),
        Some((FULL_LEVEL, true))
    );
    // Above 100% (another program boosted it): all of it.
    assert_eq!(parse_volume("Volume: 1.50"), Some((FULL_LEVEL, false)));
    assert_eq!(parse_volume("Volume: 0.50 [LOUD]"), None);
    assert_eq!(parse_volume("Error"), None);
    assert_eq!(parse_volume("Volume: -1"), None);
    assert_eq!(volume_text(FULL_LEVEL / 2), "0.5000");
    assert_eq!(volume_text(FULL_LEVEL + 5), "1.0000");
}

#[test]
fn wpctl_is_run_for_the_default_devices_with_a_deadline() {
    let dir = scratch("wpctl");
    let wpctl = Wpctl::new(script(&dir, "echo 'Volume: 0.25 [MUTED]'"));
    assert_eq!(
        wpctl.level(Control::OutputVolume).unwrap(),
        Some(read_volume(FULL_LEVEL / 4, true))
    );
    wpctl
        .set_level(Control::InputVolume, FULL_LEVEL / 2)
        .unwrap();
    wpctl.set_muted(Control::OutputVolume, true).unwrap();
    assert_eq!(
        logged(&dir),
        [
            "get-volume @DEFAULT_AUDIO_SINK@",
            "set-volume @DEFAULT_AUDIO_SOURCE@ 0.5000",
            "set-mute @DEFAULT_AUDIO_SOURCE@ 0",
            "set-mute @DEFAULT_AUDIO_SINK@ 1",
        ]
    );
    // No default device (wpctl fails): none, not an error.
    let dir = scratch("wpctl-none");
    let wpctl = Wpctl::new(script(&dir, "exit 1"));
    assert_eq!(wpctl.level(Control::OutputVolume).unwrap(), None);
    assert!(wpctl.set_level(Control::OutputVolume, 0).is_err());
    // A backlight is no volume.
    assert_eq!(wpctl.level(Control::DisplayBrightness).unwrap(), None);
    // No wpctl at all: none either.
    let missing = Wpctl::new(dir.join("missing"));
    assert_eq!(missing.level(Control::InputVolume).unwrap(), None);
    // A PipeWire that does not answer: an error at the deadline, never a wait beyond it.
    let dir = scratch("wpctl-stuck");
    let wpctl = Wpctl::new(script(&dir, "sleep 10"));
    let started = Instant::now();
    let error = wpctl.level(Control::OutputVolume).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// A backlight device in a described sysfs.
fn device(sysfs: &Path, class: &str, name: &str, files: &[(&str, &str)]) {
    let dir = sysfs.join("class").join(class).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    for (file, text) in files {
        std::fs::write(dir.join(file), text).unwrap();
    }
}

#[derive(Clone, Default)]
struct Recorded(Arc<Mutex<Vec<(String, String, u32)>>>);

impl Setter for Recorded {
    fn set(&self, subsystem: &str, name: &str, value: u32) -> std::io::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push((subsystem.to_string(), name.to_string(), value));
        Ok(())
    }
}

#[test]
fn the_display_backlight_is_the_firmwares_first_and_the_keyboards_is_an_led() {
    let sysfs = scratch("sysfs");
    assert_eq!(backlight::display(&sysfs), None);
    assert_eq!(backlight::keyboard(&sysfs), None);
    device(
        &sysfs,
        "backlight",
        "intel_backlight",
        &[
            ("type", "raw\n"),
            ("max_brightness", "19393\n"),
            ("brightness", "16897\n"),
        ],
    );
    let display = backlight::display(&sysfs).unwrap();
    assert_eq!(
        (display.subsystem, display.name.as_str()),
        ("backlight", "intel_backlight")
    );
    let level = display.level().unwrap();
    assert_eq!(level.value, level_of(16_897, 19_393));
    assert_eq!((level.steps, level.fine_steps), (16, 64));
    // The firmware's interface goes before the GPU's.
    device(
        &sysfs,
        "backlight",
        "acpi_video0",
        &[
            ("type", "firmware\n"),
            ("max_brightness", "100\n"),
            ("brightness", "50\n"),
        ],
    );
    assert_eq!(backlight::display(&sysfs).unwrap().name, "acpi_video0");
    // A ThinkPad's keyboard: three brightnesses, changed by its firmware.
    device(
        &sysfs,
        "leds",
        "input0::capslock",
        &[("max_brightness", "1\n"), ("brightness", "0\n")],
    );
    device(
        &sysfs,
        "leds",
        "tpacpi::kbd_backlight",
        &[
            ("max_brightness", "2\n"),
            ("brightness", "1\n"),
            ("brightness_hw_changed", "1\n"),
        ],
    );
    let keyboard = backlight::keyboard(&sysfs).unwrap();
    assert_eq!(
        (keyboard.subsystem, keyboard.name.as_str()),
        ("leds", "tpacpi::kbd_backlight")
    );
    assert!(keyboard.changed_by_firmware());
    let level = keyboard.level().unwrap();
    assert_eq!(level.value, FULL_LEVEL / 2);
    assert_eq!((level.steps, level.fine_steps), (2, 2));
    // Set through the setter (logind in a session), in the device's own units.
    let setter = Recorded::default();
    keyboard.set(&setter, FULL_LEVEL).unwrap();
    display.set(&setter, FULL_LEVEL / 2).unwrap();
    assert_eq!(
        *setter.0.lock().unwrap(),
        [
            ("leds".to_string(), "tpacpi::kbd_backlight".to_string(), 2),
            ("backlight".to_string(), "intel_backlight".to_string(), 9697),
        ]
    );
    assert_eq!(raw_of(0, 19_393), 0);
    assert_eq!(raw_of(FULL_LEVEL, 19_393), 19_393);
    assert!(level_of(raw_of(12_345, 19_393), 19_393).abs_diff(12_345) <= 2);
}

#[test]
fn a_session_acts_through_wpctl_sysfs_and_its_setter() {
    let dir = scratch("session");
    let sysfs = dir.join("sys");
    device(
        &sysfs,
        "backlight",
        "intel_backlight",
        &[
            ("type", "raw\n"),
            ("max_brightness", "100\n"),
            ("brightness", "75\n"),
        ],
    );
    let setter = Recorded::default();
    let made = setter.clone();
    let mut session = Session::with(
        Wpctl::new(script(&dir, "echo 'Volume: 0.50'")),
        sysfs,
        Box::new(move || Ok(Box::new(made.clone()) as Box<dyn Setter>)),
    );
    assert_eq!(
        session
            .level(Control::DisplayBrightness)
            .unwrap()
            .unwrap()
            .value,
        level_of(75, 100)
    );
    assert_eq!(
        session.level(Control::OutputVolume).unwrap(),
        Some(read_volume(FULL_LEVEL / 2, false))
    );
    // No keyboard backlight on this machine.
    assert_eq!(session.level(Control::KeyboardBrightness).unwrap(), None);
    assert!(!session.keyboard_changed_by_firmware());
    session
        .set_level(Control::DisplayBrightness, FULL_LEVEL)
        .unwrap();
    assert_eq!(
        *setter.0.lock().unwrap(),
        [("backlight".to_string(), "intel_backlight".to_string(), 100)]
    );
    assert!(session
        .set_level(Control::KeyboardBrightness, FULL_LEVEL)
        .is_err());
    session.set_muted(Control::InputVolume, true).unwrap();
    assert_eq!(
        logged(&dir).last().map(String::as_str),
        Some("set-mute @DEFAULT_AUDIO_SOURCE@ 1")
    );
}

#[test]
fn upower_says_the_battery() {
    use crate::power::{battery, Reading};
    let reading = Reading {
        present: true,
        kind: 2,
        percentage: 78.6,
        state: 2,
        on_battery: true,
        time_to_empty: 7_260,
        time_to_full: 0,
    };
    assert_eq!(
        battery(reading),
        Battery {
            state: BatteryState::Discharging,
            percent: 79,
            on_ac: false,
            minutes: 121
        }
    );
    // The ThinkPad's: charging on AC, its time to full.
    let charging = battery(Reading {
        state: 1,
        on_battery: false,
        time_to_full: 10_906,
        ..reading
    });
    assert_eq!(
        (charging.state, charging.on_ac, charging.minutes),
        (BatteryState::Charging, true, 181)
    );
    assert_eq!(
        battery(Reading {
            state: 4,
            on_battery: false,
            ..reading
        })
        .state,
        BatteryState::Full
    );
    // Plugged in, held below full (a charge threshold).
    assert_eq!(
        battery(Reading {
            state: 5,
            on_battery: false,
            ..reading
        })
        .state,
        BatteryState::NotCharging
    );
    // Unknown between readings: the power source says.
    assert_eq!(
        battery(Reading {
            state: 0,
            on_battery: false,
            ..reading
        })
        .state,
        BatteryState::NotCharging
    );
    // A desktop: UPower's display device is no battery.
    assert_eq!(
        battery(Reading {
            present: false,
            on_battery: false,
            ..reading
        })
        .state,
        BatteryState::Absent
    );
    assert_eq!(
        battery(Reading { kind: 1, ..reading }).state,
        BatteryState::Absent
    );
}

#[test]
fn networkmanager_says_wifi() {
    use crate::network::{ssid_text, wifi_state};
    assert_eq!(wifi_state(true, true, None), WifiState::Unavailable);
    assert_eq!(wifi_state(false, true, Some(30)), WifiState::Off);
    // The hardware's switch off.
    assert_eq!(wifi_state(true, false, Some(20)), WifiState::Off);
    assert_eq!(wifi_state(true, true, Some(30)), WifiState::Disconnected);
    assert_eq!(wifi_state(true, true, Some(20)), WifiState::Disconnected);
    for connecting in [40, 50, 60, 70, 80, 90] {
        assert_eq!(
            wifi_state(true, true, Some(connecting)),
            WifiState::Connecting
        );
    }
    assert_eq!(wifi_state(true, true, Some(100)), WifiState::Connected);
    assert_eq!(wifi_state(true, true, Some(110)), WifiState::Disconnected);
    assert_eq!(ssid_text(b"LukeSkyRouter"), "LukeSkyRouter");
    assert_eq!(ssid_text(b"caf\xe9"), "caf\u{fffd}");
}

#[test]
fn bluez_says_bluetooth() {
    use crate::bluetooth::status;
    assert_eq!(status(&[], 3), Bluetooth::UNAVAILABLE);
    assert_eq!(
        status(&[false], 2),
        Bluetooth {
            state: BluetoothState::Off,
            connected: 0
        }
    );
    assert_eq!(
        status(&[false, true], 2),
        Bluetooth {
            state: BluetoothState::On,
            connected: 2
        }
    );
}

const MACHINE: &str = "\
output_volume = 0.5
output_muted = true
input_volume = 1
display_brightness = 0.75
keyboard_brightness = 0.5
keyboard_levels = 3
keyboard_firmware = true
battery = charging
battery_percent = 78
battery_minutes = 181
on_ac = true
wifi = connected
wifi_signal = 73
ssid = LukeSkyRouter
wired = false
bluetooth = on
bluetooth_connected = 1
";

#[test]
fn a_fake_machine_is_what_it_is_told_and_keeps_what_is_done_to_it() {
    let dir = scratch("fake");
    let file = dir.join("machine");
    std::fs::write(&file, MACHINE).unwrap();
    let mut fake = Fake::from_file(file.clone()).unwrap();
    let watch = fake.clone();
    assert_eq!(
        fake.level(Control::OutputVolume).unwrap(),
        Some(Level::new(FULL_LEVEL / 2, true))
    );
    let keyboard = fake.level(Control::KeyboardBrightness).unwrap().unwrap();
    assert_eq!(
        (keyboard.value, keyboard.steps, keyboard.fine_steps),
        (FULL_LEVEL / 2, 2, 2)
    );
    assert!(fake.keyboard_changed_by_firmware());
    assert_eq!(
        fake.battery().unwrap(),
        Battery {
            state: BatteryState::Charging,
            percent: 78,
            on_ac: true,
            minutes: 181
        }
    );
    assert_eq!(fake.network().unwrap().ssid, "LukeSkyRouter");
    // A level set unmutes, a keyboard of few brightnesses takes the nearest it has, and Wi-Fi
    // goes off; each written back.
    fake.set_level(Control::OutputVolume, 9 * STEP).unwrap();
    fake.set_level(Control::KeyboardBrightness, 40_000).unwrap();
    fake.set_wifi(false).unwrap();
    assert_eq!(watch.value(Control::OutputVolume), Some((9 * STEP, false)));
    assert_eq!(
        watch.value(Control::KeyboardBrightness),
        Some((FULL_LEVEL / 2, false))
    );
    assert_eq!(
        watch.calls(),
        [
            "set output_volume 0.562500",
            "set keyboard_brightness 0.610352",
            "wifi false"
        ]
    );
    let written = Fake::from_file(file).unwrap();
    assert_eq!(written.text(), watch.text());
    assert!(watch
        .text()
        .contains("output_volume = 0.562500\noutput_muted = false\n"));
    assert!(watch.text().contains("wifi = off\n"));
    // What it does not have: none, and setting it fails.
    let mut bare = Fake::parse("").unwrap();
    assert_eq!(bare.level(Control::DisplayBrightness).unwrap(), None);
    assert!(bare.set_level(Control::DisplayBrightness, 0).is_err());
    assert!(bare.set_wifi(true).is_err());
    assert_eq!(bare.battery().unwrap(), Battery::ABSENT);
    // A line it does not know is refused, with its place.
    assert_eq!(
        Fake::parse("volume = 1").unwrap_err(),
        "line 1: volume: unknown"
    );
    assert!(Fake::parse("output_volume = 2").is_err());
}

#[test]
fn commands_are_answered_with_what_the_system_has_then() {
    let fake = Fake::parse(MACHINE).unwrap();
    let mut worker = Worker::inline(Box::new(fake.clone()));
    assert!(worker.wake_fd().is_none());
    worker.send(Command::SetLevel(Control::DisplayBrightness, FULL_LEVEL));
    worker.send(Command::ReadLevel(Control::KeyboardBrightness, Some(2)));
    assert_eq!(
        worker.outcomes(),
        [
            Outcome::Level {
                control: Control::DisplayBrightness,
                level: Some(Level::new(FULL_LEVEL, false)),
                tag: None
            },
            Outcome::Level {
                control: Control::KeyboardBrightness,
                level: Some(Level {
                    value: FULL_LEVEL / 2,
                    muted: false,
                    steps: 2,
                    fine_steps: 2,
                    resolution: 0
                }),
                tag: Some(2)
            }
        ]
    );
    assert!(worker.outcomes().is_empty());
    worker.send(Command::ReadLevels);
    let levels = worker.outcomes();
    assert_eq!(levels.len(), 5);
    assert_eq!(levels[0], Outcome::KeyboardByFirmware(true));
    worker.send(Command::ReadStatus);
    let status = worker.outcomes();
    assert!(matches!(status[0], Outcome::Battery(_)));
    assert!(
        matches!(&status[1], Outcome::Network(network) if network.wifi == WifiState::Connected)
    );
    // A failure: said, and the level unavailable.
    let mut bare = Worker::inline(Box::new(Fake::parse("").unwrap()));
    bare.send(Command::SetMuted(Control::OutputVolume, true));
    assert_eq!(
        bare.outcomes(),
        [Outcome::Level {
            control: Control::OutputVolume,
            level: None,
            tag: None
        }]
    );
}

#[test]
fn a_worker_thread_answers_and_wakes_the_turn() {
    let fake = Fake::parse(MACHINE).unwrap();
    let mut worker = Worker::spawn(Box::new(fake.clone())).unwrap();
    assert!(worker.wake_fd().is_some());
    worker.send(Command::SetMuted(Control::InputVolume, true));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut outcomes = Vec::new();
    while outcomes.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
        outcomes = worker.outcomes();
    }
    assert_eq!(
        outcomes,
        [Outcome::Level {
            control: Control::InputVolume,
            level: Some(Level::new(FULL_LEVEL, true)),
            tag: None
        }]
    );
    assert_eq!(fake.value(Control::InputVolume), Some((FULL_LEVEL, true)));
}

#[test]
fn a_level_set_again_before_the_system_follows_is_set_once() {
    let commands = vec![
        Command::SetLevel(Control::OutputVolume, 1),
        Command::SetLevel(Control::DisplayBrightness, 2),
        Command::SetMuted(Control::OutputVolume, true),
        Command::SetLevel(Control::OutputVolume, 3),
        Command::ReadStatus,
    ];
    assert_eq!(
        coalesce(commands),
        [
            Command::SetLevel(Control::DisplayBrightness, 2),
            Command::SetMuted(Control::OutputVolume, true),
            Command::SetLevel(Control::OutputVolume, 3),
            Command::ReadStatus,
        ]
    );
}

#[test]
fn a_level_read_rounded_still_steps_away_from_where_it_is() {
    // The session set 7/16 (0.4375); `wpctl` reads it back as 0.44.
    let (read, _) = crate::audio::parse_volume("Volume: 0.44").unwrap();
    let resolution = FULL_LEVEL.div_ceil(200);
    let down = stepped(read, STEPS, false, resolution);
    assert_eq!(down, 6 * STEP, "one step down, never the same step");
    // Read exactly, the same value steps up to the next step as before.
    assert_eq!(stepped(read, STEPS, true, resolution), 8 * STEP);
    // At a step read rounded down (0.375 as 0.38 is above; 0.3125 as 0.31 is below).
    let (read, _) = crate::audio::parse_volume("Volume: 0.31").unwrap();
    assert_eq!(stepped(read, STEPS, true, resolution), 6 * STEP);
    assert_eq!(stepped(read, STEPS, false, resolution), 4 * STEP);
}
