//! The session's side of the hardware keys and the Control Center, on a played machine
//! ([`oxyos_controls::Fake`]): what each key and request does to it, and what the shell is told.

use super::*;
use oxyos_compositor_core::{ClientId, Grants, Output};
use oxyos_controls::Fake;
use oxyos_protocol::{
    BatteryState, BluetoothState, Event, PanelEdge, Rect, Request, ShellKind, Size,
};

const APP: ClientId = 1;
const SHELL: ClientId = 2;
const STEP: u32 = FULL_LEVEL / 16;
const SHIFT: u32 = 42;
const ALT: u32 = 56;

const MACHINE: &str = "\
output_volume = 0.5
output_muted = true
input_volume = 1
display_brightness = 0.125
keyboard_brightness = 0.5
keyboard_levels = 3
battery = discharging
battery_percent = 64
battery_minutes = 200
on_ac = false
wifi = connected
wifi_signal = 73
ssid = LukeSkyRouter
bluetooth = on
bluetooth_connected = 1
";

struct Machine {
    scene: Scene,
    controls: Controls,
    fake: Fake,
    now: Instant,
}

impl Machine {
    fn new(described: &str) -> Self {
        let mut scene = Scene::new();
        scene
            .add_output(Output {
                id: 1,
                rect: Rect::new(0, 0, 1280, 800),
                scale_120: 120,
                refresh_mhz: 60_000,
            })
            .unwrap();
        scene.connect(APP, Grants::default()).unwrap();
        scene
            .connect(
                SHELL,
                Grants {
                    shell: true,
                    shortcuts: true,
                    lock: true,
                    ..Grants::default()
                },
            )
            .unwrap();
        // The shell's top bar: it shows the bezels of output 1.
        scene
            .handle(
                SHELL,
                Request::CreateShellSurface {
                    surface: 1,
                    kind: ShellKind::Panel,
                    edge: PanelEdge::Top,
                    output: 1,
                    size: Size::new(1, 28),
                    exclusive: 28,
                },
            )
            .unwrap();
        let fake = Fake::parse(described).unwrap();
        let now = Instant::now();
        let mut machine = Self {
            scene,
            controls: Controls::new(Worker::inline(Box::new(fake.clone())), now),
            fake,
            now,
        };
        machine.serve();
        machine
    }

    fn serve(&mut self) {
        self.controls.serve(&mut self.scene, self.now);
    }

    fn keys(&mut self, codes: &[u32]) {
        for code in codes {
            self.scene.key(*code, true);
        }
        for code in codes.iter().rev() {
            self.scene.key(*code, false);
        }
        self.serve();
    }

    fn shell(&mut self, request: Request) {
        self.scene.handle(SHELL, request).unwrap();
        self.serve();
    }

    /// What the shell and the application were told since the last call: the shell's control
    /// events, and how many events of any kind reached the application.
    fn told(&mut self) -> (Vec<Event>, usize) {
        let mut shell = Vec::new();
        let mut app = 0;
        for (to, event) in self.scene.drain_events() {
            match (to, event) {
                (APP, _) => app += 1,
                (
                    SHELL,
                    event @ (Event::ControlLevel { .. }
                    | Event::Power { .. }
                    | Event::Network { .. }
                    | Event::Bluetooth { .. }),
                ) => shell.push(event),
                _ => {}
            }
        }
        (shell, app)
    }

    /// The levels the shell was told since the last call: (control, level, muted, bezel).
    fn levels(&mut self) -> Vec<(Control, u32, bool, bool)> {
        self.told()
            .0
            .into_iter()
            .filter_map(|event| match event {
                Event::ControlLevel {
                    control,
                    level_65536,
                    muted,
                    bezel,
                    ..
                } => Some((control, level_65536, muted, bezel)),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn the_shell_is_told_the_machine_when_the_session_starts() {
    let mut machine = Machine::new(MACHINE);
    let (told, _) = machine.told();
    assert!(told.contains(&Event::ControlLevel {
        control: Control::OutputVolume,
        level_65536: FULL_LEVEL / 2,
        muted: true,
        available: true,
        bezel: false
    }));
    assert!(told.contains(&Event::Power {
        battery: BatteryState::Discharging,
        percent: 64,
        on_ac: false,
        minutes: 200
    }));
    assert!(told.contains(&Event::Network {
        wifi: WifiState::Connected,
        signal: 73,
        wired: false,
        ssid: PreeditText::new("LukeSkyRouter").unwrap()
    }));
    assert!(told.contains(&Event::Bluetooth {
        state: BluetoothState::On,
        connected: 1
    }));
    assert_eq!(
        told.len(),
        7,
        "four levels, the battery, the network and Bluetooth"
    );
    // A machine without them: the levels unavailable, no battery, no Wi-Fi.
    let mut bare = Machine::new("");
    let (told, _) = bare.told();
    assert!(told.contains(&Event::ControlLevel {
        control: Control::DisplayBrightness,
        level_65536: 0,
        muted: false,
        available: false,
        bezel: false
    }));
    assert!(told.contains(&Event::Power {
        battery: BatteryState::Absent,
        percent: 0,
        on_ac: true,
        minutes: 0
    }));
}

#[test]
fn volume_keys_step_the_default_output_and_unmute_it() {
    let mut machine = Machine::new(MACHINE);
    machine.told();
    machine.keys(&[KEY_VOLUMEUP]);
    // Told at once with its bezel, then the system's answer (the same: nothing more).
    assert_eq!(
        machine.levels(),
        [(Control::OutputVolume, FULL_LEVEL / 2 + STEP, false, true)]
    );
    assert_eq!(
        machine.fake.value(Control::OutputVolume),
        Some((FULL_LEVEL / 2 + STEP, false))
    );
    machine.keys(&[KEY_VOLUMEDOWN]);
    machine.keys(&[KEY_VOLUMEDOWN]);
    assert_eq!(
        machine.fake.value(Control::OutputVolume),
        Some((FULL_LEVEL / 2 - STEP, false))
    );
    // Shift+Alt: a quarter of a step.
    machine.keys(&[SHIFT, ALT, KEY_VOLUMEUP]);
    assert_eq!(
        machine.fake.value(Control::OutputVolume),
        Some((FULL_LEVEL / 2 - STEP + STEP / 4, false))
    );
    // Never past all: pressed at the end, the bezel shows it there again.
    machine.levels();
    for _ in 0..20 {
        machine.keys(&[KEY_VOLUMEUP]);
    }
    let told = machine.levels();
    assert_eq!(told.len(), 20, "a bezel for every press");
    assert_eq!(
        *told.last().unwrap(),
        (Control::OutputVolume, FULL_LEVEL, false, true)
    );
    // The application was told nothing of any of it.
    assert_eq!(machine.told().1, 0);
}

#[test]
fn mute_and_mic_mute_toggle() {
    let mut machine = Machine::new(MACHINE);
    machine.told();
    machine.keys(&[KEY_MUTE]);
    assert_eq!(
        machine.fake.value(Control::OutputVolume),
        Some((FULL_LEVEL / 2, false))
    );
    machine.keys(&[KEY_MUTE]);
    assert_eq!(
        machine.fake.value(Control::OutputVolume),
        Some((FULL_LEVEL / 2, true))
    );
    machine.keys(&[KEY_MICMUTE]);
    assert_eq!(
        machine.fake.value(Control::InputVolume),
        Some((FULL_LEVEL, true))
    );
    assert_eq!(
        machine.levels(),
        [
            (Control::OutputVolume, FULL_LEVEL / 2, false, true),
            (Control::OutputVolume, FULL_LEVEL / 2, true, true),
            (Control::InputVolume, FULL_LEVEL, true, true),
        ]
    );
    assert_eq!(
        machine.fake.calls(),
        [
            "mute output_volume false",
            "mute output_volume true",
            "mute input_volume true"
        ]
    );
}

#[test]
fn brightness_keys_step_the_display_down_to_off_and_up_to_all() {
    let mut machine = Machine::new(MACHINE);
    machine.keys(&[KEY_BRIGHTNESSDOWN]);
    machine.keys(&[KEY_BRIGHTNESSDOWN]);
    assert_eq!(
        machine.fake.value(Control::DisplayBrightness),
        Some((0, false))
    );
    machine.keys(&[KEY_BRIGHTNESSDOWN]);
    assert_eq!(
        machine.fake.value(Control::DisplayBrightness),
        Some((0, false))
    );
    for _ in 0..16 {
        machine.keys(&[KEY_BRIGHTNESSUP]);
    }
    assert_eq!(
        machine.fake.value(Control::DisplayBrightness),
        Some((FULL_LEVEL, false))
    );
}

#[test]
fn the_keyboard_light_steps_through_what_it_has_and_its_key_turns_it_off_and_back() {
    let mut machine = Machine::new(MACHINE);
    machine.keys(&[KEY_KBDILLUMUP]);
    assert_eq!(
        machine.fake.value(Control::KeyboardBrightness),
        Some((FULL_LEVEL, false))
    );
    machine.keys(&[KEY_KBDILLUMDOWN]);
    machine.keys(&[KEY_KBDILLUMTOGGLE]);
    assert_eq!(
        machine.fake.value(Control::KeyboardBrightness),
        Some((0, false))
    );
    machine.keys(&[KEY_KBDILLUMTOGGLE]);
    assert_eq!(
        machine.fake.value(Control::KeyboardBrightness),
        Some((FULL_LEVEL / 2, false))
    );
}

#[test]
fn a_keyboard_light_the_firmware_changes_is_read_never_set_by_its_key() {
    let mut machine = Machine::new(&format!("{MACHINE}keyboard_firmware = true\n"));
    machine.told();
    // A ThinkPad's Fn+Space: the firmware changed it to all already, and the key says so.
    machine.fake.change("keyboard_brightness = 1").unwrap();
    machine.keys(&[KEY_KBDILLUMTOGGLE]);
    assert_eq!(
        machine.levels(),
        [(Control::KeyboardBrightness, FULL_LEVEL, false, true)]
    );
    assert!(machine.fake.calls().is_empty(), "nothing set");
}

#[test]
fn a_key_with_nothing_to_act_on_shows_nothing() {
    let mut machine = Machine::new("");
    machine.told();
    machine.keys(&[KEY_KBDILLUMUP]);
    machine.keys(&[KEY_VOLUMEUP]);
    // Looked at again (it may have come since), still not there: no bezel.
    assert_eq!(machine.levels(), []);
    assert!(machine.fake.calls().is_empty());
}

#[test]
fn the_control_center_sets_levels_mutes_and_wifi() {
    let mut machine = Machine::new(MACHINE);
    machine.told();
    machine.shell(Request::SetControlLevel {
        control: Control::DisplayBrightness,
        level_65536: 3 * STEP,
    });
    machine.shell(Request::SetControlMuted {
        control: Control::OutputVolume,
        muted: false,
    });
    assert_eq!(
        machine.levels(),
        [
            (Control::DisplayBrightness, 3 * STEP, false, false),
            (Control::OutputVolume, FULL_LEVEL / 2, false, false),
        ]
    );
    assert_eq!(
        machine.fake.value(Control::DisplayBrightness),
        Some((3 * STEP, false))
    );
    // Wi-Fi off: told off, without the network it was on.
    machine.shell(Request::SetWifiEnabled { enabled: false });
    assert_eq!(
        machine.told().0,
        [Event::Network {
            wifi: WifiState::Off,
            signal: 0,
            wired: false,
            ssid: PreeditText::default()
        }]
    );
    machine.shell(Request::SetWifiEnabled { enabled: true });
    assert!(matches!(
        machine.told().0[..],
        [Event::Network {
            wifi: WifiState::Connected,
            ..
        }]
    ));
    // Opened, the Control Center sees what another program did.
    machine.fake.change("output_volume = 0.25").unwrap();
    machine.shell(Request::RefreshControls {});
    assert_eq!(
        machine.levels(),
        [(Control::OutputVolume, FULL_LEVEL / 4, false, false)]
    );
}

#[test]
fn the_battery_and_the_network_are_looked_at_every_few_seconds() {
    let mut machine = Machine::new(MACHINE);
    machine.told();
    machine
        .fake
        .change("battery = charging\nbattery_percent = 65\non_ac = true")
        .unwrap();
    machine.serve();
    assert!(machine.told().0.is_empty(), "not yet");
    assert!(machine.controls.next_wake(machine.now) <= STATUS_EVERY);
    machine.now += STATUS_EVERY;
    machine.serve();
    assert_eq!(
        machine.told().0,
        [Event::Power {
            battery: BatteryState::Charging,
            percent: 65,
            on_ac: true,
            minutes: 0
        }]
    );
    // The levels, less often.
    machine.fake.change("display_brightness = 1").unwrap();
    machine.now += LEVELS_EVERY;
    machine.serve();
    assert!(machine
        .levels()
        .contains(&(Control::DisplayBrightness, FULL_LEVEL, false, false)));
}

#[test]
fn locked_the_keys_still_act() {
    let mut machine = Machine::new(MACHINE);
    machine.scene.lock();
    machine.keys(&[KEY_VOLUMEUP]);
    assert_eq!(
        machine.fake.value(Control::OutputVolume),
        Some((FULL_LEVEL / 2 + STEP, false))
    );
    // The Control Center is not there to ask.
    machine.shell(Request::SetControlLevel {
        control: Control::OutputVolume,
        level_65536: 0,
    });
    assert_eq!(
        machine.fake.value(Control::OutputVolume),
        Some((FULL_LEVEL / 2 + STEP, false))
    );
}

#[test]
fn a_network_name_is_cut_to_one_chunk_at_a_whole_character() {
    let network = Network {
        wifi: WifiState::Connected,
        signal: 120,
        ssid: "ñ".repeat(20),
        wired: true,
    };
    let status = network_status(&network);
    assert_eq!(status.ssid.as_str(), "ñ".repeat(16));
    assert_eq!(status.signal, 100);
    let off = network_status(&Network {
        wifi: WifiState::Disconnected,
        ..network
    });
    assert_eq!(off.ssid.as_str(), "");
}

const KEY_MUTE: u32 = 113;
const KEY_VOLUMEDOWN: u32 = 114;
const KEY_VOLUMEUP: u32 = 115;
const KEY_BRIGHTNESSDOWN: u32 = 224;
const KEY_BRIGHTNESSUP: u32 = 225;
const KEY_KBDILLUMTOGGLE: u32 = 228;
const KEY_KBDILLUMDOWN: u32 = 229;
const KEY_KBDILLUMUP: u32 = 230;
const KEY_MICMUTE: u32 = 248;
