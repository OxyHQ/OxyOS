//! Hardware keys and the Control Center (ADR 0025): the keys are the system's, handed to the
//! program; the shell is told what the program says, and asks for changes.

use super::*;
use crate::scene::controls::*;
use oxyos_protocol::{BatteryState, BluetoothState, Control, WifiState, FULL_LEVEL};

const SHIFT: u32 = 42;
const SHELL_2: ClientId = 9;
/// Every key the system takes, by its code.
const HARDWARE: [u32; 9] = [
    KEY_MUTE,
    KEY_VOLUMEDOWN,
    KEY_VOLUMEUP,
    KEY_BRIGHTNESSDOWN,
    KEY_BRIGHTNESSUP,
    KEY_KBDILLUMTOGGLE,
    KEY_KBDILLUMDOWN,
    KEY_KBDILLUMUP,
    KEY_MICMUTE,
];

fn dock(h: &mut Harness, client: ClientId, output: u32) {
    h.request(
        client,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Bottom,
            output,
            size: Size::new(1, 48),
            exclusive: 48,
        },
    )
    .unwrap();
    h.present(key(client, 1));
}

/// Everything sent to `client` that is input.
fn input_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<Event> {
    to(events, client)
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                Event::Key { .. }
                    | Event::Text { .. }
                    | Event::Preedit { .. }
                    | Event::InputMethodKey { .. }
            )
        })
        .collect()
}

fn levels_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<(Control, u32, bool)> {
    to(events, client)
        .iter()
        .filter_map(|event| match event {
            Event::ControlLevel {
                control,
                level_65536,
                bezel,
                ..
            } => Some((*control, *level_65536, *bezel)),
            _ => None,
        })
        .collect()
}

/// What `client` was told of the controls alone.
fn controls_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<Event> {
    to(events, client)
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                Event::ControlLevel { .. }
                    | Event::Power { .. }
                    | Event::Network { .. }
                    | Event::Bluetooth { .. }
            )
        })
        .collect()
}

fn level(level: u32) -> ControlLevel {
    ControlLevel {
        level,
        muted: false,
        available: true,
    }
}

fn press(h: &mut Harness, codes: &[u32], text: Option<&str>) {
    for code in codes {
        h.scene.keyboard(*code, true, text);
    }
    for code in codes.iter().rev() {
        h.scene.keyboard(*code, false, None);
    }
}

#[test]
fn hardware_keys_never_reach_an_application_and_each_press_goes_to_the_program() {
    let mut h = Harness::new();
    dock(&mut h, SHELL, 1);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    for code in HARDWARE {
        press(&mut h, &[code], None);
    }
    // With modifiers held, and with Super (whose chords are the system's too): still the keys'.
    press(&mut h, &[CTRL, KEY_VOLUMEUP], None);
    press(&mut h, &[SUPER, KEY_BRIGHTNESSUP], None);
    // Through the path without a layout too.
    h.scene.key(KEY_MUTE, true);
    h.scene.key(KEY_MUTE, false);
    let events = logged(&mut h);
    // Ctrl's press and release are the application's; nothing of a hardware key is.
    assert_eq!(keys_to(&events, APP), [(CTRL, true), (CTRL, false)]);
    assert_eq!(input_to(&events, APP).len(), 2);
    let pressed: Vec<HardwareKey> = h
        .scene
        .take_control_requests()
        .into_iter()
        .map(|request| match request {
            ControlRequest::Key { key, fine, output } => {
                assert!(!fine);
                assert_eq!(output, 1);
                key
            }
            other => panic!("not a key: {other:?}"),
        })
        .collect();
    use HardwareKey::*;
    assert_eq!(
        pressed,
        [
            Mute,
            VolumeDown,
            VolumeUp,
            BrightnessDown,
            BrightnessUp,
            KeyboardToggle,
            KeyboardDown,
            KeyboardUp,
            MicMute,
            VolumeUp,
            BrightnessUp,
            Mute
        ]
    );
    assert!(h.scene.take_control_requests().is_empty(), "taken once");
    // The control: an ordinary key is the application's, press, release and text.
    press(&mut h, &[KEY_A], Some("a"));
    let events = logged(&mut h);
    assert_eq!(keys_to(&events, APP), [(KEY_A, true), (KEY_A, false)]);
    assert_eq!(input_to(&events, APP).len(), 3);
    assert!(h.scene.take_control_requests().is_empty());
}

#[test]
fn shift_and_alt_make_a_fine_step() {
    let mut h = Harness::new();
    let fine = |h: &mut Harness| match h.scene.take_control_requests()[..] {
        [ControlRequest::Key { fine, .. }] => fine,
        ref other => panic!("{other:?}"),
    };
    press(&mut h, &[SHIFT, ALT, KEY_VOLUMEUP], None);
    assert!(fine(&mut h));
    press(&mut h, &[SHIFT, KEY_VOLUMEUP], None);
    assert!(!fine(&mut h));
    press(&mut h, &[ALT, KEY_VOLUMEUP], None);
    assert!(!fine(&mut h));
}

#[test]
fn the_input_method_never_sees_a_hardware_key() {
    let (mut h, window) = served("");
    press(&mut h, &[KEY_VOLUMEUP, KEY_MICMUTE], None);
    let events = logged(&mut h);
    assert!(to(&events, IME)
        .iter()
        .all(|event| !matches!(event, Event::InputMethodKey { .. })));
    assert_eq!(input_to(&events, window.client), []);
    assert_eq!(h.scene.take_control_requests().len(), 2);
}

#[test]
fn locked_the_keys_still_act_and_the_lock_screen_receives_none() {
    let mut h = Harness::new();
    h.scene.lock();
    h.request(
        LOCKER,
        Request::CreateLockSurface {
            surface: 1,
            output: 1,
        },
    )
    .unwrap();
    h.present(key(LOCKER, 1));
    assert_eq!(h.scene.focus(), Some(key(LOCKER, 1)));
    logged(&mut h);
    press(&mut h, &[KEY_VOLUMEDOWN], None);
    press(&mut h, &[KEY_BRIGHTNESSUP], None);
    assert_eq!(input_to(&logged(&mut h), LOCKER), []);
    assert_eq!(h.scene.take_control_requests().len(), 2);
    // The password's keys are still the lock screen's.
    press(&mut h, &[KEY_A], Some("a"));
    assert_eq!(input_to(&logged(&mut h), LOCKER).len(), 3);
}

#[test]
fn the_key_names_the_output_in_front() {
    let mut h = Harness::new();
    dock(&mut h, SHELL, 1);
    h.scene
        .add_output(Output {
            id: 2,
            rect: Rect::new(1920, 0, 1280, 720),
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    h.scene
        .connect(
            SHELL_2,
            Grants {
                shell: true,
                shortcuts: true,
                ..Grants::default()
            },
        )
        .unwrap();
    dock(&mut h, SHELL_2, 2);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    h.scene.move_window_to(window, (2000, 100));
    h.present(window);
    logged(&mut h);
    press(&mut h, &[KEY_VOLUMEUP], None);
    assert_eq!(
        h.scene.take_control_requests(),
        [ControlRequest::Key {
            key: HardwareKey::VolumeUp,
            fine: false,
            output: 2
        }]
    );
    // The program's answer: every shell is told the level, the second shows it.
    h.scene
        .set_control_level(Control::OutputVolume, level(32_768), Some(2));
    let events = logged(&mut h);
    assert_eq!(
        levels_to(&events, SHELL),
        [(Control::OutputVolume, 32_768, false)]
    );
    assert_eq!(
        levels_to(&events, SHELL_2),
        [(Control::OutputVolume, 32_768, true)]
    );
    // Pressed again at the same level (the end of the range): only the bezel's shell is told.
    h.scene
        .set_control_level(Control::OutputVolume, level(32_768), Some(2));
    let events = logged(&mut h);
    assert_eq!(levels_to(&events, SHELL), []);
    assert_eq!(
        levels_to(&events, SHELL_2),
        [(Control::OutputVolume, 32_768, true)]
    );
    // Unchanged and no key: nobody.
    h.scene
        .set_control_level(Control::OutputVolume, level(32_768), None);
    assert!(levels_to(&logged(&mut h), SHELL_2).is_empty());
}

#[test]
fn shells_are_told_what_changed_and_a_late_one_everything() {
    let mut h = Harness::new();
    let power = PowerStatus {
        battery: BatteryState::Charging,
        percent: 78,
        on_ac: true,
        minutes: 42,
    };
    let network = NetworkStatus {
        wifi: WifiState::Connected,
        signal: 73,
        wired: false,
        ssid: PreeditText::new("LukeSkyRouter").unwrap(),
    };
    let bluetooth = BluetoothStatus {
        state: BluetoothState::On,
        connected: 1,
    };
    h.scene.set_power(power);
    h.scene.set_network(network);
    h.scene.set_bluetooth(bluetooth);
    h.scene
        .set_control_level(Control::DisplayBrightness, level(FULL_LEVEL + 9), None);
    let events = logged(&mut h);
    // Applications are told nothing; the shell all of it, the level kept within its range.
    assert!(to(&events, APP).is_empty());
    let shell = controls_to(&events, SHELL);
    assert_eq!(shell.len(), 4);
    assert!(shell.contains(&Event::Power {
        battery: BatteryState::Charging,
        percent: 78,
        on_ac: true,
        minutes: 42
    }));
    assert!(shell.contains(&Event::ControlLevel {
        control: Control::DisplayBrightness,
        level_65536: FULL_LEVEL,
        muted: false,
        available: true,
        bezel: false
    }));
    // The same again: nothing.
    h.scene.set_power(power);
    h.scene.set_network(network);
    h.scene.set_bluetooth(bluetooth);
    assert!(controls_to(&logged(&mut h), SHELL).is_empty());
    // A change: that alone.
    h.scene.set_power(PowerStatus {
        percent: 79,
        ..power
    });
    assert_eq!(controls_to(&logged(&mut h), SHELL).len(), 1);
    // A shell that connects now is told everything known, at once.
    h.scene
        .connect(
            SHELL_2,
            Grants {
                shell: true,
                ..Grants::default()
            },
        )
        .unwrap();
    assert_eq!(controls_to(&logged(&mut h), SHELL_2).len(), 4);
    assert_eq!(
        h.scene.control_level(Control::DisplayBrightness),
        Some(level(FULL_LEVEL))
    );
}

#[test]
fn the_control_center_asks_through_the_shell_grant_alone() {
    let mut h = Harness::new();
    for request in [
        Request::SetControlLevel {
            control: Control::OutputVolume,
            level_65536: 4096,
        },
        Request::SetControlMuted {
            control: Control::InputVolume,
            muted: true,
        },
        Request::SetWifiEnabled { enabled: false },
        Request::RefreshControls {},
    ] {
        h.request(SHELL, request).unwrap();
        // An application asking is refused, and the connection closed.
        assert_eq!(h.request(APP, request), Err(ErrorCode::Unauthorized));
    }
    assert_eq!(
        h.scene.take_control_requests(),
        [
            ControlRequest::SetLevel {
                control: Control::OutputVolume,
                level: 4096
            },
            ControlRequest::SetMuted {
                control: Control::InputVolume,
                muted: true
            },
            ControlRequest::SetWifi(false),
            ControlRequest::Refresh,
        ]
    );
    // Past all of it: not a level.
    assert_eq!(
        h.request(
            SHELL,
            Request::SetControlLevel {
                control: Control::DisplayBrightness,
                level_65536: FULL_LEVEL + 1
            }
        ),
        Err(ErrorCode::InvalidMessage)
    );
    // Locked: nothing is taken but looking again.
    h.scene.lock();
    for request in [
        Request::SetControlLevel {
            control: Control::OutputVolume,
            level_65536: 0,
        },
        Request::SetWifiEnabled { enabled: true },
        Request::RefreshControls {},
    ] {
        h.request(SHELL, request).unwrap();
    }
    assert_eq!(h.scene.take_control_requests(), [ControlRequest::Refresh]);
}

#[test]
fn the_appearance_goes_the_preferences_way() {
    let mut h = Harness::new();
    h.request(
        SHELL,
        Request::SetAppearance {
            serial: 7,
            dark: true,
        },
    )
    .unwrap();
    assert_eq!(
        h.request(
            APP,
            Request::SetAppearance {
                serial: 1,
                dark: true
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
    assert_eq!(
        h.scene.take_preference_changes(),
        [PreferenceChange {
            client: SHELL,
            serial: 7,
            key: "appearance".to_string(),
            value: "dark".to_string(),
        }]
    );
    // Locked, it is refused at once and never handed over.
    h.scene.lock();
    logged(&mut h);
    h.request(
        SHELL,
        Request::SetAppearance {
            serial: 8,
            dark: false,
        },
    )
    .unwrap();
    assert!(h.scene.take_preference_changes().is_empty());
    assert!(
        to(&logged(&mut h), SHELL).contains(&Event::PreferenceResult {
            serial: 8,
            accepted: false
        })
    );
}
