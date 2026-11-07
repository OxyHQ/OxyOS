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
//! is only told, and the level is read. The battery, the network, Bluetooth and the levels, which
//! other programs and the firmware change too, are read when the Control Center opens and, in a
//! session, as they change ([`Following`], and the volumes as the session's sound says them,
//! [`Controls::audio_is`]: never looked at on a clock, so a session doing nothing wakes for
//! nothing).

use crate::stop::{identity, Stop};
use oxyos_compositor_core::{
    BluetoothStatus, ControlLevel, ControlRequest, HardwareKey, NetworkStatus, OutputId,
    PowerStatus, Scene,
};
use oxyos_controls::follow::{self, Change};
use oxyos_controls::{
    stepped, Battery, Bluetooth, Command, Control, Level, Network, Outcome, Worker, FULL_LEVEL,
};
use oxyos_protocol::{PreeditText, WifiState, TEXT_CHUNK_BYTES};
use std::collections::BTreeMap;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;

pub struct Controls {
    worker: Worker,
    /// Each level as the system last said it, or as last set; `None`: the machine has none.
    levels: BTreeMap<Control, Option<Level>>,
    keyboard_by_firmware: bool,
    /// The keyboard's backlight before its key turned it off.
    keyboard_before: u32,
    /// What follows the system's changes (a session's), stopped when the controls go.
    following: Option<Following>,
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
    pub fn new(mut worker: Worker) -> Self {
        worker.send(Command::ReadLevels);
        worker.send(Command::ReadStatus);
        Self {
            worker,
            levels: BTreeMap::new(),
            keyboard_by_firmware: false,
            keyboard_before: FULL_LEVEL,
            following: None,
        }
    }

    /// Follows the system's changes (a session's: its worker on a thread of its own), each read
    /// again as it changes: the bus's services (NetworkManager too where `network`, when no other
    /// part follows it) and the backlights under `sysfs`; the volumes are told by the sound's part
    /// ([`Self::audio_is`]). Nothing for an inline worker.
    pub fn follow(&mut self, network: bool, sysfs: PathBuf) -> std::io::Result<()> {
        if let Some(commands) = self.worker.commands() {
            self.following = Some(Following::start(commands, network, sysfs)?);
        }
        Ok(())
    }

    /// What wakes the compositor's turn when the worker has answers.
    pub fn wake_fd(&self) -> Option<BorrowedFd<'_>> {
        self.worker.wake_fd()
    }

    /// Whether the output is muted or at no volume, as the system last said it (or as last set):
    /// no system sound plays then (docs/adr/0044-system-sounds.mdx). A machine whose output was
    /// never told is not silent.
    pub fn output_silent(&self) -> bool {
        matches!(
            self.levels.get(&Control::OutputVolume),
            Some(Some(level)) if level.muted || level.value == 0
        )
    }

    /// The network as the networks' part read it (docs/adr/0038-networks.mdx), which follows the
    /// daemon: told as if read here, so the top bar follows a cable or a network at once with no
    /// reading of the daemon again.
    pub fn network_is(&mut self, scene: &mut Scene, network: Network) {
        self.outcome(scene, Outcome::Network(network));
    }

    /// A request as the Control Center's (Settings' Sound page sets a volume this way, so the
    /// keys, the bezel and the slider follow it).
    pub fn ask(&mut self, scene: &mut Scene, request: ControlRequest) {
        self.request(scene, request);
    }

    /// The volumes as the session's sound says them (docs/adr/0056-sound-devices.mdx), which
    /// follows PipeWire: told as if read here, each only when it changed.
    pub fn audio_is(&mut self, scene: &mut Scene, state: &oxyos_audio::State) {
        for control in [Control::OutputVolume, Control::InputVolume] {
            let level = oxyos_controls::audio_level(state, control);
            if self.levels.get(&control) != Some(&level) {
                self.outcome(
                    scene,
                    Outcome::Level {
                        control,
                        level,
                        tag: None,
                    },
                );
            }
        }
    }

    /// One turn: the keys and requests the scene took, and the worker's answers told to the
    /// scene.
    pub fn serve(&mut self, scene: &mut Scene) {
        for request in scene.take_control_requests() {
            self.request(scene, request);
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

/// The threads that follow the system's changes for a session's controls (`oxyos_controls::follow`),
/// each sending the worker the read of what changed. Each waits on its source and the order to
/// stop, never a clock; a source that goes (the bus restarted) is followed again once its socket
/// is made anew. They stop and are waited for when it goes.
pub struct Following {
    stop: Arc<Stop>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

/// The read a change asks for.
fn read(change: Change) -> Command {
    match change {
        Change::Battery => Command::ReadBattery,
        Change::Network => Command::ReadNetwork,
        Change::Bluetooth => Command::ReadBluetooth,
        Change::Level(control) => Command::ReadLevel(control, None),
    }
}

/// Follows a source with `follow` until `stop` is given: again whenever it goes, once `socket` is
/// made anew (or, with none, never again).
pub(crate) fn again(
    what: &str,
    stop: &Stop,
    socket: Option<PathBuf>,
    mut follow: impl FnMut(&Stop) -> std::io::Result<()>,
) {
    let mut failed_on = None;
    let mut said = false;
    loop {
        if let Some(socket) = &socket {
            if !stop.wait_for_path(socket, failed_on) {
                return;
            }
        }
        let tried = socket.as_deref().and_then(identity);
        let result = follow(stop);
        if stop.given() {
            return;
        }
        if let Err(error) = result {
            if !said {
                eprintln!("[controls] following {what}: {error}; again once it is there");
                said = true;
            }
        }
        if socket.is_none() {
            stop.wait();
            return;
        }
        failed_on = tried;
    }
}

impl Following {
    fn start(commands: Sender<Command>, network: bool, sysfs: PathBuf) -> std::io::Result<Self> {
        let stop = Arc::new(Stop::new()?);
        let mut threads = Vec::new();
        let spawn = |name: &str, run: Box<dyn FnOnce() + Send>| {
            std::thread::Builder::new().name(name.into()).spawn(run)
        };
        {
            let (stop, commands) = (stop.clone(), commands.clone());
            threads.push(spawn(
                "controls-bus",
                Box::new(move || {
                    let socket = oxyos_networks::bus_socket(None);
                    again("the system bus", &stop, socket, |stop| {
                        follow::bus(network, stop.fd(), |change| {
                            commands.send(read(change)).is_ok()
                        })
                    });
                }),
            )?);
        }
        {
            let (stop, commands) = (stop.clone(), commands.clone());
            threads.push(spawn(
                "controls-backlights",
                Box::new(move || {
                    again("the backlights", &stop, None, |stop| {
                        follow::backlights(Path::new(&sysfs), stop.fd(), |change| {
                            commands.send(read(change)).is_ok()
                        })
                    });
                }),
            )?);
        }
        Ok(Self { stop, threads })
    }
}

impl Drop for Following {
    fn drop(&mut self) {
        self.stop.give();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
