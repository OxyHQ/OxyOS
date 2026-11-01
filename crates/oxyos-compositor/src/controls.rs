//! Hardware keys and the Control Center for the program running the compositor
//! (docs/adr/0025-hardware-keys-and-control-center.mdx), the session authority: it takes the keys
//! the scene took and the shell's requests, decides what each does to the system, has it done by
//! a worker ([`oxyos_controls::Worker`], off the compositor's turn) and tells the scene what the
//! system has then, which tells the shell.
//!
//! A key moves its level a step from what the system last said (sixteen, or a quarter of one
//! with Shift and Alt) and the shell is told at once, so the bezel follows the keys as fast as
//! they are pressed; the system's answer follows, and corrects it if the system did otherwise.
//! A volume key unmutes, as a Mac's does. The keyboard backlight's key turns it off and back on to
//! where it was, unless the firmware changes it itself (a ThinkPad's Fn+Space): then its key
//! is only told, and the level is read. The battery, the network and Bluetooth are looked at
//! every few seconds and when the Control Center opens; the levels, which other programs change
//! too, less often.

use oxyos_compositor_core::{
    BluetoothStatus, ControlLevel, ControlRequest, HardwareKey, NetworkStatus, OutputId,
    PowerStatus, Scene,
};
use oxyos_controls::{
    stepped, Battery, Bluetooth, Command, Control, Level, Network, Outcome, Worker, FULL_LEVEL,
};
use oxyos_protocol::{PreeditText, WifiState, TEXT_CHUNK_BYTES};
use std::collections::BTreeMap;
use std::os::fd::BorrowedFd;
use std::time::{Duration, Instant};

/// How often the battery, the network and Bluetooth are looked at.
pub const STATUS_EVERY: Duration = Duration::from_secs(5);
/// How often the levels are looked at (another program may change them).
pub const LEVELS_EVERY: Duration = Duration::from_secs(30);

pub struct Controls {
    worker: Worker,
    /// Each level as the system last said it, or as last set; `None`: the machine has none.
    levels: BTreeMap<Control, Option<Level>>,
    keyboard_by_firmware: bool,
    /// The keyboard's backlight before its key turned it off.
    keyboard_before: u32,
    next_status: Instant,
    next_levels: Instant,
}

/// A level as the scene keeps it.
fn scene_level(level: Option<Level>) -> ControlLevel {
    match level {
        Some(level) => ControlLevel {
            level: level.value,
            muted: level.muted,
            available: true,
        },
        None => ControlLevel {
            level: 0,
            muted: false,
            available: false,
        },
    }
}

/// An access point's name cut to one chunk, at a whole character.
fn chunk_of(text: &str) -> PreeditText {
    let mut end = text.len().min(TEXT_CHUNK_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    PreeditText::new(&text[..end]).unwrap_or_default()
}

pub fn network_status(network: &Network) -> NetworkStatus {
    let named = matches!(network.wifi, WifiState::Connected | WifiState::Connecting);
    NetworkStatus {
        wifi: network.wifi,
        signal: if named { network.signal.min(100) } else { 0 },
        wired: network.wired,
        ssid: if named {
            chunk_of(&network.ssid)
        } else {
            PreeditText::default()
        },
    }
}

pub fn power_status(battery: Battery) -> PowerStatus {
    PowerStatus {
        battery: battery.state,
        percent: battery.percent.min(100),
        on_ac: battery.on_ac,
        minutes: battery.minutes,
    }
}

pub fn bluetooth_status(bluetooth: Bluetooth) -> BluetoothStatus {
    BluetoothStatus {
        state: bluetooth.state,
        connected: bluetooth.connected,
    }
}

impl Controls {
    /// Controls through `worker`, which is asked at once what the system has.
    pub fn new(mut worker: Worker, now: Instant) -> Self {
        worker.send(Command::ReadLevels);
        worker.send(Command::ReadStatus);
        Self {
            worker,
            levels: BTreeMap::new(),
            keyboard_by_firmware: false,
            keyboard_before: FULL_LEVEL,
            next_status: now + STATUS_EVERY,
            next_levels: now + LEVELS_EVERY,
        }
    }

    /// What wakes the compositor's turn when the worker has answers.
    pub fn wake_fd(&self) -> Option<BorrowedFd<'_>> {
        self.worker.wake_fd()
    }

    /// How long the turn may wait before the system is looked at again.
    pub fn next_wake(&self, now: Instant) -> Duration {
        self.next_status
            .min(self.next_levels)
            .saturating_duration_since(now)
    }

    /// The battery, the network and Bluetooth looked at now: the network's daemon said it changed
    /// (docs/adr/0038-networks.mdx), so the top bar follows a cable or a network at once.
    pub fn refresh_status(&mut self) {
        self.worker.send(Command::ReadStatus);
    }

    /// One turn: the keys and requests the scene took, the system looked at when it is time,
    /// and the worker's answers told to the scene.
    pub fn serve(&mut self, scene: &mut Scene, now: Instant) {
        for request in scene.take_control_requests() {
            self.request(scene, request);
        }
        if now >= self.next_status {
            self.next_status = now + STATUS_EVERY;
            self.worker.send(Command::ReadStatus);
        }
        if now >= self.next_levels {
            self.next_levels = now + LEVELS_EVERY;
            self.worker.send(Command::ReadLevels);
        }
        for outcome in self.worker.outcomes() {
            self.outcome(scene, outcome);
        }
    }

    fn request(&mut self, scene: &mut Scene, request: ControlRequest) {
        match request {
            ControlRequest::Key { key, fine, output } => self.key(scene, key, fine, output),
            ControlRequest::SetLevel { control, level } => {
                if let Some(Some(known)) = self.levels.get(&control).copied() {
                    self.set(scene, control, known, level, None);
                } else {
                    self.worker.send(Command::SetLevel(control, level));
                }
            }
            ControlRequest::SetMuted { control, muted } => {
                if let Some(Some(known)) = self.levels.get(&control).copied() {
                    self.shown(scene, control, Level { muted, ..known }, None);
                }
                self.worker.send(Command::SetMuted(control, muted));
            }
            ControlRequest::SetWifi(enabled) => self.worker.send(Command::SetWifi(enabled)),
            ControlRequest::Refresh => {
                self.worker.send(Command::ReadLevels);
                self.worker.send(Command::ReadStatus);
            }
        }
    }

    fn key(&mut self, scene: &mut Scene, key: HardwareKey, fine: bool, output: OutputId) {
        let control = key.control();
        let known = self.levels.get(&control).copied().flatten();
        // The firmware's own: it changed already, and its key says so.
        let firmware = key == HardwareKey::KeyboardToggle && self.keyboard_by_firmware;
        let Some(known) = known.filter(|_| !firmware) else {
            // Not known yet (or the firmware's): looked at, and shown when the answer comes.
            self.worker.send(Command::ReadLevel(control, Some(output)));
            return;
        };
        let steps = if fine { known.fine_steps } else { known.steps };
        match key {
            HardwareKey::Mute | HardwareKey::MicMute => {
                let muted = !known.muted;
                self.shown(scene, control, Level { muted, ..known }, Some(output));
                self.worker.send(Command::SetMuted(control, muted));
            }
            HardwareKey::KeyboardToggle => {
                let value = if known.value > 0 {
                    self.keyboard_before = known.value;
                    0
                } else {
                    self.keyboard_before
                };
                self.set(scene, control, known, value, Some(output));
            }
            _ => {
                let up = matches!(
                    key,
                    HardwareKey::VolumeUp | HardwareKey::BrightnessUp | HardwareKey::KeyboardUp
                );
                let value = stepped(known.value, steps, up, known.resolution);
                self.set(scene, control, known, value, Some(output));
            }
        }
    }

    /// Sets a level: the scene told at once, the system asked. A volume set is unmuted.
    fn set(
        &mut self,
        scene: &mut Scene,
        control: Control,
        known: Level,
        value: u32,
        bezel: Option<OutputId>,
    ) {
        let level = Level {
            value: value.min(FULL_LEVEL),
            muted: false,
            ..known
        };
        self.shown(scene, control, level, bezel);
        self.worker.send(Command::SetLevel(control, level.value));
    }

    fn shown(
        &mut self,
        scene: &mut Scene,
        control: Control,
        level: Level,
        bezel: Option<OutputId>,
    ) {
        self.levels.insert(control, Some(level));
        scene.set_control_level(control, scene_level(Some(level)), bezel);
    }

    fn outcome(&mut self, scene: &mut Scene, outcome: Outcome) {
        match outcome {
            Outcome::Level {
                control,
                level,
                tag,
            } => {
                self.levels.insert(control, level);
                // A key with nothing to act on shows nothing.
                let bezel = tag.filter(|_| level.is_some());
                scene.set_control_level(control, scene_level(level), bezel);
            }
            Outcome::Battery(battery) => scene.set_power(power_status(battery)),
            Outcome::Network(network) => scene.set_network(network_status(&network)),
            Outcome::Bluetooth(bluetooth) => scene.set_bluetooth(bluetooth_status(bluetooth)),
            Outcome::KeyboardByFirmware(firmware) => self.keyboard_by_firmware = firmware,
        }
    }
}

#[cfg(test)]
mod tests;
