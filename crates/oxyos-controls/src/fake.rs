//! A machine played for tests and the development compositor: its levels, battery, network and
//! Bluetooth are what it is told, as `key = value` lines (the preferences' format), and every
//! change the session makes is kept there, written back to its file when it has one, so a
//! scenario checks what the session did to the system.

use crate::{Battery, Bluetooth, Control, Level, Network, System, FULL_LEVEL, STEPS};
use oxyos_protocol::{BatteryState, BluetoothState, WifiState};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
struct State {
    /// Each control's level and mute; `None`: the machine has none.
    levels: [Option<(u32, bool)>; 4],
    /// How many brightnesses the keyboard's backlight has (a ThinkPad's: three).
    keyboard_levels: u32,
    keyboard_firmware: bool,
    battery: Battery,
    network: Network,
    bluetooth: Bluetooth,
    /// Every call that changed something, in order: `set output_volume 0.5625`.
    calls: Vec<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            levels: [None; 4],
            keyboard_levels: 0,
            keyboard_firmware: false,
            battery: Battery::ABSENT,
            network: Network::UNAVAILABLE,
            bluetooth: Bluetooth::UNAVAILABLE,
            calls: Vec::new(),
        }
    }
}

/// A [`System`] that is what it is told. Clones share it: a test keeps one to look at what the
/// session did.
#[derive(Clone, Debug, Default)]
pub struct Fake {
    state: Arc<Mutex<State>>,
    file: Option<PathBuf>,
    /// The network from a NetworkManager on this bus instead (a played one, docs/adr/0038-
    /// networks.mdx), so the top bar and Control Center's switch follow the daemon the session's
    /// network requests change.
    /// Clones share it, so it is closed once for all of them.
    network_bus: Arc<Mutex<Option<zbus::blocking::Connection>>>,
}

fn index(control: Control) -> usize {
    match control {
        Control::OutputVolume => 0,
        Control::InputVolume => 1,
        Control::DisplayBrightness => 2,
        Control::KeyboardBrightness => 3,
    }
}

fn name(control: Control) -> &'static str {
    match control {
        Control::OutputVolume => "output_volume",
        Control::InputVolume => "input_volume",
        Control::DisplayBrightness => "display_brightness",
        Control::KeyboardBrightness => "keyboard_brightness",
    }
}

fn fraction(text: &str) -> Result<u32, String> {
    let value: f64 = text.parse().map_err(|_| format!("not a level: {text:?}"))?;
    if !(0.0..=1.0).contains(&value) {
        return Err(format!("not between 0 and 1: {text:?}"));
    }
    Ok((value * f64::from(FULL_LEVEL)).round() as u32)
}

fn flag(text: &str) -> Result<bool, String> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("not true or false: {text:?}")),
    }
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("not a number: {text:?}"))
}

fn battery_state(text: &str) -> Result<BatteryState, String> {
    Ok(match text {
        "absent" => BatteryState::Absent,
        "discharging" => BatteryState::Discharging,
        "charging" => BatteryState::Charging,
        "full" => BatteryState::Full,
        "not_charging" => BatteryState::NotCharging,
        _ => return Err(format!("not a battery state: {text:?}")),
    })
}

fn battery_name(state: BatteryState) -> &'static str {
    match state {
        BatteryState::Absent => "absent",
        BatteryState::Discharging => "discharging",
        BatteryState::Charging => "charging",
        BatteryState::Full => "full",
        BatteryState::NotCharging => "not_charging",
    }
}

fn wifi_state(text: &str) -> Result<WifiState, String> {
    Ok(match text {
        "unavailable" => WifiState::Unavailable,
        "off" => WifiState::Off,
        "disconnected" => WifiState::Disconnected,
        "connecting" => WifiState::Connecting,
        "connected" => WifiState::Connected,
        _ => return Err(format!("not a Wi-Fi state: {text:?}")),
    })
}

fn wifi_name(state: WifiState) -> &'static str {
    match state {
        WifiState::Unavailable => "unavailable",
        WifiState::Off => "off",
        WifiState::Disconnected => "disconnected",
        WifiState::Connecting => "connecting",
        WifiState::Connected => "connected",
    }
}

fn bluetooth_state(text: &str) -> Result<BluetoothState, String> {
    Ok(match text {
        "unavailable" => BluetoothState::Unavailable,
        "off" => BluetoothState::Off,
        "on" => BluetoothState::On,
        _ => return Err(format!("not a Bluetooth state: {text:?}")),
    })
}

fn bluetooth_name(state: BluetoothState) -> &'static str {
    match state {
        BluetoothState::Unavailable => "unavailable",
        BluetoothState::Off => "off",
        BluetoothState::On => "on",
    }
}

impl Fake {
    /// Its network read from, and Wi-Fi turned on and off on, the NetworkManager on the bus at
    /// `address`, as a session's is on the system bus.
    pub fn with_network_bus(self, address: &str) -> io::Result<Self> {
        let bus = zbus::blocking::connection::Builder::address(address)
            .map_err(crate::bus::failed)?
            .build()
            .map_err(crate::bus::failed)?;
        *self.network_bus.lock().expect("the fake's bus") = Some(bus);
        Ok(self)
    }

    /// Its connection to the network's bus closed (the development compositor finishing): the
    /// network is then the one last read.
    pub fn close_network_bus(&self) {
        if let Some(bus) = self.network_bus.lock().expect("the fake's bus").take() {
            let _ = bus.close();
        }
    }

    /// A machine described by `key = value` lines; what they leave out it does not have.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut state = State::default();
        for (number_of, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .map(|(key, value)| (key.trim(), value.trim()))
                .ok_or_else(|| format!("line {}: no `=`", number_of + 1))?;
            let at = |error: String| format!("line {}: {key}: {error}", number_of + 1);
            let control = [
                Control::OutputVolume,
                Control::InputVolume,
                Control::DisplayBrightness,
                Control::KeyboardBrightness,
            ]
            .into_iter()
            .find(|control| name(*control) == key);
            if let Some(control) = control {
                let muted = state.levels[index(control)].is_some_and(|(_, muted)| muted);
                state.levels[index(control)] = Some((fraction(value).map_err(at)?, muted));
                continue;
            }
            match key {
                "output_muted" | "input_muted" => {
                    let control = if key == "output_muted" {
                        Control::OutputVolume
                    } else {
                        Control::InputVolume
                    };
                    let level = state.levels[index(control)].get_or_insert((0, false));
                    level.1 = flag(value).map_err(at)?;
                }
                "keyboard_levels" => state.keyboard_levels = number(value).map_err(at)?,
                "keyboard_firmware" => state.keyboard_firmware = flag(value).map_err(at)?,
                "battery" => state.battery.state = battery_state(value).map_err(at)?,
                "battery_percent" => state.battery.percent = number(value).map_err(at)?,
                "battery_minutes" => state.battery.minutes = number(value).map_err(at)?,
                "on_ac" => state.battery.on_ac = flag(value).map_err(at)?,
                "wifi" => state.network.wifi = wifi_state(value).map_err(at)?,
                "wifi_signal" => state.network.signal = number(value).map_err(at)?,
                "ssid" => state.network.ssid = value.to_string(),
                "wired" => state.network.wired = flag(value).map_err(at)?,
                "bluetooth" => state.bluetooth.state = bluetooth_state(value).map_err(at)?,
                "bluetooth_connected" => state.bluetooth.connected = number(value).map_err(at)?,
                _ => return Err(at("unknown".to_string())),
            }
        }
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            file: None,
            network_bus: Arc::default(),
        })
    }

    /// The machine `file` describes, which it writes back after every change.
    pub fn from_file(file: PathBuf) -> Result<Self, String> {
        let text = std::fs::read_to_string(&file)
            .map_err(|error| format!("{}: {error}", file.display()))?;
        let mut fake =
            Self::parse(&text).map_err(|error| format!("{}: {error}", file.display()))?;
        fake.file = Some(file);
        Ok(fake)
    }

    /// The machine as it is now, in the lines it is described by.
    pub fn text(&self) -> String {
        let state = self.state.lock().expect("the fake's state");
        let mut lines = Vec::new();
        for control in crate::CONTROLS {
            if let Some((value, muted)) = state.levels[index(control)] {
                lines.push(format!(
                    "{} = {:.6}",
                    name(control),
                    f64::from(value) / f64::from(FULL_LEVEL)
                ));
                match control {
                    Control::OutputVolume => lines.push(format!("output_muted = {muted}")),
                    Control::InputVolume => lines.push(format!("input_muted = {muted}")),
                    _ => {}
                }
            }
        }
        if state.keyboard_levels > 0 {
            lines.push(format!("keyboard_levels = {}", state.keyboard_levels));
        }
        lines.push(format!("keyboard_firmware = {}", state.keyboard_firmware));
        let battery = state.battery;
        lines.push(format!("battery = {}", battery_name(battery.state)));
        lines.push(format!("battery_percent = {}", battery.percent));
        lines.push(format!("battery_minutes = {}", battery.minutes));
        lines.push(format!("on_ac = {}", battery.on_ac));
        let network = &state.network;
        lines.push(format!("wifi = {}", wifi_name(network.wifi)));
        lines.push(format!("wifi_signal = {}", network.signal));
        lines.push(format!("ssid = {}", network.ssid));
        lines.push(format!("wired = {}", network.wired));
        lines.push(format!(
            "bluetooth = {}",
            bluetooth_name(state.bluetooth.state)
        ));
        lines.push(format!(
            "bluetooth_connected = {}",
            state.bluetooth.connected
        ));
        lines.join("\n") + "\n"
    }

    /// The changes the session made, in order.
    pub fn calls(&self) -> Vec<String> {
        self.state.lock().expect("the fake's state").calls.clone()
    }

    /// A level as the machine has it, `None` where it has none.
    pub fn value(&self, control: Control) -> Option<(u32, bool)> {
        self.state.lock().expect("the fake's state").levels[index(control)]
    }

    /// Changes the machine behind the session's back (another program, the firmware).
    pub fn change(&self, describe: &str) -> Result<(), String> {
        let changed = Self::parse(describe)?;
        let changed = changed.state.lock().expect("the fake's state").clone();
        let mut state = self.state.lock().expect("the fake's state");
        for (level, new) in state.levels.iter_mut().zip(changed.levels) {
            if new.is_some() {
                *level = new;
            }
        }
        if describe.contains("battery") || describe.contains("on_ac") {
            state.battery = changed.battery;
        }
        if describe.contains("wifi") || describe.contains("ssid") || describe.contains("wired") {
            state.network = changed.network;
        }
        if describe.contains("bluetooth") {
            state.bluetooth = changed.bluetooth;
        }
        Ok(())
    }

    fn changed(&self, call: String) -> io::Result<()> {
        self.state
            .lock()
            .expect("the fake's state")
            .calls
            .push(call);
        if let Some(file) = &self.file {
            std::fs::write(file, self.text())?;
        }
        Ok(())
    }

    /// The keyboard's steps: one per brightness it has beyond off.
    fn keyboard_steps(state: &State) -> u32 {
        state.keyboard_levels.saturating_sub(1).max(1)
    }
}

impl System for Fake {
    fn level(&mut self, control: Control) -> io::Result<Option<Level>> {
        let state = self.state.lock().expect("the fake's state");
        Ok(state.levels[index(control)].map(|(value, muted)| {
            let mut level = Level::new(value, muted);
            if control == Control::KeyboardBrightness && state.keyboard_levels > 0 {
                level.steps = Self::keyboard_steps(&state).min(STEPS);
                level.fine_steps = level.steps;
            }
            level
        }))
    }

    fn set_level(&mut self, control: Control, value: u32) -> io::Result<()> {
        {
            let mut state = self.state.lock().expect("the fake's state");
            let steps = Self::keyboard_steps(&state);
            let raw = control == Control::KeyboardBrightness && state.keyboard_levels > 0;
            let level = state.levels[index(control)]
                .as_mut()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such control"))?;
            // A device of few brightnesses takes the nearest it has.
            level.0 = if raw {
                let raw = (u64::from(value) * u64::from(steps) + u64::from(FULL_LEVEL) / 2)
                    / u64::from(FULL_LEVEL);
                (raw * u64::from(FULL_LEVEL) / u64::from(steps)) as u32
            } else {
                value.min(FULL_LEVEL)
            };
            level.1 = false;
        }
        self.changed(format!(
            "set {} {:.6}",
            name(control),
            f64::from(value) / f64::from(FULL_LEVEL)
        ))
    }

    fn set_muted(&mut self, control: Control, muted: bool) -> io::Result<()> {
        {
            let mut state = self.state.lock().expect("the fake's state");
            let level = state.levels[index(control)]
                .as_mut()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such control"))?;
            level.1 = muted;
        }
        self.changed(format!("mute {} {muted}", name(control)))
    }

    fn keyboard_changed_by_firmware(&mut self) -> bool {
        self.state
            .lock()
            .expect("the fake's state")
            .keyboard_firmware
    }

    fn battery(&mut self) -> io::Result<Battery> {
        Ok(self.state.lock().expect("the fake's state").battery)
    }

    fn network(&mut self) -> io::Result<Network> {
        let bus = self.network_bus.lock().expect("the fake's bus").clone();
        if let Some(bus) = &bus {
            let network = crate::network::read(bus)?;
            self.state.lock().expect("the fake's state").network = network.clone();
            return Ok(network);
        }
        Ok(self.state.lock().expect("the fake's state").network.clone())
    }

    fn set_wifi(&mut self, enabled: bool) -> io::Result<()> {
        let bus = self.network_bus.lock().expect("the fake's bus").clone();
        if let Some(bus) = &bus {
            return crate::network::set_wifi(bus, enabled);
        }
        {
            let mut state = self.state.lock().expect("the fake's state");
            let network = &mut state.network;
            if network.wifi == WifiState::Unavailable {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no Wi-Fi"));
            }
            // Turned on, it connects to the network it knows, as NetworkManager does.
            network.wifi = if enabled {
                if network.ssid.is_empty() {
                    WifiState::Disconnected
                } else {
                    WifiState::Connected
                }
            } else {
                WifiState::Off
            };
        }
        self.changed(format!("wifi {enabled}"))
    }

    fn bluetooth(&mut self) -> io::Result<Bluetooth> {
        Ok(self.state.lock().expect("the fake's state").bluetooth)
    }
}
