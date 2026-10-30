//! Hardware keys and the Control Center (docs/adr/0025-hardware-keys-and-control-center.mdx).
//!
//! The volume, mute, microphone, display brightness and keyboard backlight keys are the
//! system's, as a Mac's are: the compositor takes them from the keyboard before the system
//! shortcuts, the input method and every application, press and release, whatever modifiers are
//! held and whether or not the session is locked, and hands each press to the program running
//! the compositor (the session authority), which acts on the system and says what came of it.
//! Keys are matched by their Linux key codes, which XKB's evdev keymaps name XF86AudioRaiseVolume,
//! XF86MonBrightnessUp and the rest on every layout, as the system shortcuts are by place.
//!
//! The shell (its `shell` grant) is told each level, the battery, the network and Bluetooth as
//! the program says them, when it connects and whenever they change; the shell of the output in
//! front (its `shortcuts` grant, as for the switcher) is told a key's level with `bezel`, and shows
//! it. Its Control Center asks for changes (`SetControlLevel` and the rest), which the program
//! validates and applies; the appearance goes the preferences' way (ADR 0019). The scene keeps no
//! level of its own and talks to nothing outside: what it is told is all it tells.

use super::preferences::PreferenceChange;
use super::{ClientId, OutputId, Scene};
use oxyos_protocol::{
    BatteryState, BluetoothState, Control, ErrorCode, Event, PreeditText, Request, WifiState,
    FULL_LEVEL,
};
use std::collections::BTreeMap;

/// Requests and key presses waiting for the program: more are dropped (a stuck program never
/// grows the scene).
const MAX_PENDING: usize = 64;

/// The kernel's codes of the keys the system takes (`linux/input-event-codes.h`).
pub const KEY_MUTE: u32 = 113;
pub const KEY_VOLUMEDOWN: u32 = 114;
pub const KEY_VOLUMEUP: u32 = 115;
pub const KEY_BRIGHTNESSDOWN: u32 = 224;
pub const KEY_BRIGHTNESSUP: u32 = 225;
pub const KEY_KBDILLUMTOGGLE: u32 = 228;
pub const KEY_KBDILLUMDOWN: u32 = 229;
pub const KEY_KBDILLUMUP: u32 = 230;
pub const KEY_MICMUTE: u32 = 248;

/// A key of the system's: what it acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HardwareKey {
    /// XF86AudioRaiseVolume, XF86AudioLowerVolume, XF86AudioMute: the default output.
    VolumeUp,
    VolumeDown,
    Mute,
    /// XF86AudioMicMute: the default input.
    MicMute,
    /// XF86MonBrightnessUp, XF86MonBrightnessDown: the built-in display's backlight.
    BrightnessUp,
    BrightnessDown,
    /// XF86KbdBrightnessUp, XF86KbdBrightnessDown, XF86KbdLightOnOff: the keyboard's backlight.
    KeyboardUp,
    KeyboardDown,
    KeyboardToggle,
}

impl HardwareKey {
    /// The key a Linux key code is, if it is one of the system's.
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            KEY_VOLUMEUP => Self::VolumeUp,
            KEY_VOLUMEDOWN => Self::VolumeDown,
            KEY_MUTE => Self::Mute,
            KEY_MICMUTE => Self::MicMute,
            KEY_BRIGHTNESSUP => Self::BrightnessUp,
            KEY_BRIGHTNESSDOWN => Self::BrightnessDown,
            KEY_KBDILLUMUP => Self::KeyboardUp,
            KEY_KBDILLUMDOWN => Self::KeyboardDown,
            KEY_KBDILLUMTOGGLE => Self::KeyboardToggle,
            _ => return None,
        })
    }

    /// The level it acts on.
    pub fn control(self) -> Control {
        match self {
            Self::VolumeUp | Self::VolumeDown | Self::Mute => Control::OutputVolume,
            Self::MicMute => Control::InputVolume,
            Self::BrightnessUp | Self::BrightnessDown => Control::DisplayBrightness,
            Self::KeyboardUp | Self::KeyboardDown | Self::KeyboardToggle => {
                Control::KeyboardBrightness
            }
        }
    }
}

/// What the program running the compositor is asked to do, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlRequest {
    /// A hardware key was pressed: `fine` with Shift and Alt held (a quarter of a step, as a
    /// Mac's Shift+Option), `output` the one in front (the active window's, else the pointer's),
    /// whose shell shows the level.
    Key {
        key: HardwareKey,
        fine: bool,
        output: OutputId,
    },
    /// The Control Center sets a level (0 to [`FULL_LEVEL`]).
    SetLevel { control: Control, level: u32 },
    /// The Control Center mutes a volume or unmutes it.
    SetMuted { control: Control, muted: bool },
    /// The Control Center turns Wi-Fi on or off.
    SetWifi(bool),
    /// The Control Center opened: the system's state is looked at again now.
    Refresh,
}

/// A level as the program says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlLevel {
    /// 0 to [`FULL_LEVEL`].
    pub level: u32,
    /// A volume's mute.
    pub muted: bool,
    /// Whether this machine has it at all.
    pub available: bool,
}

/// The battery as the program says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerStatus {
    pub battery: BatteryState,
    pub percent: u8,
    pub on_ac: bool,
    /// Until empty (discharging) or full (charging); 0 unknown.
    pub minutes: u32,
}

/// The network as the program says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkStatus {
    pub wifi: WifiState,
    pub signal: u8,
    pub wired: bool,
    /// The access point's name, cut to one chunk.
    pub ssid: PreeditText,
}

/// Bluetooth as the program says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BluetoothStatus {
    pub state: BluetoothState,
    pub connected: u8,
}

#[derive(Debug, Default)]
pub(super) struct Controls {
    asked: Vec<ControlRequest>,
    levels: BTreeMap<Control, ControlLevel>,
    power: Option<PowerStatus>,
    network: Option<NetworkStatus>,
    bluetooth: Option<BluetoothStatus>,
}

impl Scene {
    /// Whether a key from the keyboard is one of the system's hardware keys: then nobody else
    /// sees it, press or release, and a press is handed to the program.
    pub(super) fn hardware_key(&mut self, code: u32, pressed: bool) -> bool {
        let Some(key) = HardwareKey::from_code(code) else {
            return false;
        };
        if pressed {
            let request = ControlRequest::Key {
                key,
                fine: self.fine_step(),
                output: self.shortcut_output(),
            };
            self.ask_control(request);
        }
        true
    }

    fn ask_control(&mut self, request: ControlRequest) {
        if self.controls.asked.len() < MAX_PENDING {
            self.controls.asked.push(request);
        }
    }

    /// A request of the shell's Control Center, which needs the `shell` grant. While the session
    /// is locked none is taken, but looking at the system again.
    pub(super) fn control_request(
        &mut self,
        client: ClientId,
        request: Request,
    ) -> Result<(), ErrorCode> {
        if !self.clients.get(&client).is_some_and(|grants| grants.shell) {
            return Err(ErrorCode::Unauthorized);
        }
        let request = match request {
            Request::SetControlLevel {
                control,
                level_65536,
            } => {
                if level_65536 > FULL_LEVEL {
                    return Err(ErrorCode::InvalidMessage);
                }
                ControlRequest::SetLevel {
                    control,
                    level: level_65536,
                }
            }
            Request::SetControlMuted { control, muted } => {
                ControlRequest::SetMuted { control, muted }
            }
            Request::SetWifiEnabled { enabled } => ControlRequest::SetWifi(enabled),
            Request::SetAppearance { serial, dark } => {
                if self.locked {
                    self.preference_set(client, serial, false);
                    return Ok(());
                }
                // The appearance is a preference like any other, kept and told by the session
                // (ADR 0019); the shell may change this one alone.
                let value = if dark { "dark" } else { "light" };
                return self.ask_preference(PreferenceChange {
                    client,
                    serial,
                    key: "appearance".to_string(),
                    value: value.to_string(),
                });
            }
            Request::RefreshControls {} => ControlRequest::Refresh,
            _ => unreachable!("only the Control Center's requests come here"),
        };
        if !self.locked || request == ControlRequest::Refresh {
            self.ask_control(request);
        }
        Ok(())
    }

    /// The key presses and the Control Center's requests since the last call, in order: the
    /// program running the compositor acts on each and says what came of it
    /// ([`Scene::set_control_level`] and the rest).
    pub fn take_control_requests(&mut self) -> Vec<ControlRequest> {
        std::mem::take(&mut self.controls.asked)
    }

    /// What a level is now, from the program. Each shell is told when it changed, and the shell
    /// showing `bezel`, the output a hardware key was pressed with in front, is told even if it
    /// did not, with `bezel` set, to show it.
    pub fn set_control_level(
        &mut self,
        control: Control,
        level: ControlLevel,
        bezel: Option<OutputId>,
    ) {
        let level = ControlLevel {
            level: level.level.min(FULL_LEVEL),
            ..level
        };
        let changed = self.controls.levels.insert(control, level) != Some(level);
        let showing = bezel.and_then(|output| self.shortcut_shell(output));
        for shell in self.shell_clients() {
            let bezel = showing == Some(shell);
            if changed || bezel {
                self.tell_level(shell, control, level, bezel);
            }
        }
    }

    /// The battery now, from the program; each shell is told when it changed.
    pub fn set_power(&mut self, power: PowerStatus) {
        if self.controls.power.replace(power) != Some(power) {
            for shell in self.shell_clients() {
                self.tell_power(shell, power);
            }
        }
    }

    /// The network now, from the program; each shell is told when it changed.
    pub fn set_network(&mut self, network: NetworkStatus) {
        if self.controls.network.replace(network) != Some(network) {
            for shell in self.shell_clients() {
                self.tell_network(shell, network);
            }
        }
    }

    /// Bluetooth now, from the program; each shell is told when it changed.
    pub fn set_bluetooth(&mut self, bluetooth: BluetoothStatus) {
        if self.controls.bluetooth.replace(bluetooth) != Some(bluetooth) {
            for shell in self.shell_clients() {
                self.tell_bluetooth(shell, bluetooth);
            }
        }
    }

    /// A level as the program last said it.
    pub fn control_level(&self, control: Control) -> Option<ControlLevel> {
        self.controls.levels.get(&control).copied()
    }

    /// Everything known, to a client given the `shell` grant.
    pub(super) fn tell_controls(&mut self, client: ClientId) {
        let levels: Vec<(Control, ControlLevel)> = self
            .controls
            .levels
            .iter()
            .map(|(control, level)| (*control, *level))
            .collect();
        for (control, level) in levels {
            self.tell_level(client, control, level, false);
        }
        if let Some(power) = self.controls.power {
            self.tell_power(client, power);
        }
        if let Some(network) = self.controls.network {
            self.tell_network(client, network);
        }
        if let Some(bluetooth) = self.controls.bluetooth {
            self.tell_bluetooth(client, bluetooth);
        }
    }

    fn shell_clients(&self) -> Vec<ClientId> {
        self.clients
            .iter()
            .filter(|(_, grants)| grants.shell)
            .map(|(client, _)| *client)
            .collect()
    }

    fn tell_level(&mut self, client: ClientId, control: Control, level: ControlLevel, bezel: bool) {
        self.emit(
            client,
            Event::ControlLevel {
                control,
                level_65536: level.level,
                muted: level.muted,
                available: level.available,
                bezel,
            },
        );
    }

    fn tell_power(&mut self, client: ClientId, power: PowerStatus) {
        self.emit(
            client,
            Event::Power {
                battery: power.battery,
                percent: power.percent.min(100),
                on_ac: power.on_ac,
                minutes: power.minutes,
            },
        );
    }

    fn tell_network(&mut self, client: ClientId, network: NetworkStatus) {
        self.emit(
            client,
            Event::Network {
                wifi: network.wifi,
                signal: network.signal.min(100),
                wired: network.wired,
                ssid: network.ssid,
            },
        );
    }

    fn tell_bluetooth(&mut self, client: ClientId, bluetooth: BluetoothStatus) {
        self.emit(
            client,
            Event::Bluetooth {
                state: bluetooth.state,
                connected: bluetooth.connected,
            },
        );
    }
}
