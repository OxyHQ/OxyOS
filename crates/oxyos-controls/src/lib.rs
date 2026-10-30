//! The session's hardware controls (docs/adr/0025-hardware-keys-and-control-center.mdx): what the
//! volume, brightness and keyboard backlight keys and the shell's Control Center act on, and what
//! the top bar shows of the battery, the network and Bluetooth. The session (the authority) acts
//! on the system through [`System`]; a session's is [`Session`], which speaks to what the system
//! already runs:
//! - the volume of PipeWire's default output and input, through WirePlumber's own command,
//!   `wpctl` ([`audio`]);
//! - the built-in display's and the keyboard's backlights, found in sysfs and set through
//!   logind's `SetBrightness` on the session ([`backlight`]);
//! - the battery, from UPower ([`power`]);
//! - Wi-Fi and the wired network, from NetworkManager, which turns Wi-Fi on and off
//!   ([`network`]);
//! - Bluetooth's adapter, from BlueZ ([`bluetooth`]).
//!
//! Each may be missing on a machine: what is not there is said to be unavailable, never an
//! error. [`Fake`] plays a machine for tests and the development compositor. [`Worker`] runs a
//! [`System`] off the compositor's thread, as every call may wait on another process.
//!
//! Levels are 0 to [`FULL_LEVEL`]; the keys move them in sixteen steps, as a Mac's do, or a
//! quarter of one with Shift and Alt ([`stepped`]).

use oxyos_protocol::{BatteryState, BluetoothState, WifiState};
pub use oxyos_protocol::{Control, FULL_LEVEL};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{channel, Receiver, Sender};

pub mod audio;
pub mod backlight;
pub mod bluetooth;
mod bus;
mod fake;
pub mod network;
pub mod power;
mod session;
#[cfg(test)]
mod tests;

pub use fake::Fake;
pub use session::Session;

/// The steps a key moves a level in, as a Mac's: sixteen.
pub const STEPS: u32 = 16;
/// Every control, in the protocol's order.
pub const CONTROLS: [Control; 4] = [
    Control::OutputVolume,
    Control::InputVolume,
    Control::DisplayBrightness,
    Control::KeyboardBrightness,
];

/// A level as the system has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    /// 0 to [`FULL_LEVEL`].
    pub value: u32,
    /// A volume's mute; a backlight's is always false.
    pub muted: bool,
    /// The steps a key moves it in: [`STEPS`], or fewer where the device has fewer (a keyboard
    /// backlight of three brightnesses has two).
    pub steps: u32,
    /// The steps with Shift and Alt: four times as many, as many as the device has at most.
    pub fine_steps: u32,
    /// How far the system's own level may be from `value`, as read: 0 for a level read exactly
    /// (a backlight's), more for one the system reports rounded (`wpctl` prints a volume to two
    /// decimals). A key steps from the level the system may really have.
    pub resolution: u32,
}

impl Level {
    /// A level of sixteen steps, sixty-four fine ones.
    pub fn new(value: u32, muted: bool) -> Self {
        Self {
            value: value.min(FULL_LEVEL),
            muted,
            steps: STEPS,
            fine_steps: STEPS * 4,
            resolution: 0,
        }
    }
}

/// The battery as the system has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub state: BatteryState,
    /// 0 to 100.
    pub percent: u8,
    pub on_ac: bool,
    /// Until empty (discharging) or full (charging); 0 unknown.
    pub minutes: u32,
}

impl Battery {
    /// No battery, and nothing said of the power.
    pub const ABSENT: Self = Self {
        state: BatteryState::Absent,
        percent: 0,
        on_ac: true,
        minutes: 0,
    };
}

/// The network as the system has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    pub wifi: WifiState,
    /// The access point's signal, 0 to 100.
    pub signal: u8,
    /// The access point's name, as text (bytes that are not UTF-8 replaced).
    pub ssid: String,
    /// A wired connection is up.
    pub wired: bool,
}

impl Network {
    pub const UNAVAILABLE: Self = Self {
        wifi: WifiState::Unavailable,
        signal: 0,
        ssid: String::new(),
        wired: false,
    };
}

/// Bluetooth as the system has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bluetooth {
    pub state: BluetoothState,
    /// Devices connected.
    pub connected: u8,
}

impl Bluetooth {
    pub const UNAVAILABLE: Self = Self {
        state: BluetoothState::Unavailable,
        connected: 0,
    };
}

/// What the session acts on. Every call may wait (on another process, on the system bus), so a
/// session runs it on a [`Worker`]. What a machine does not have is `None` or unavailable, never
/// an error; an error is a call that failed.
pub trait System: Send {
    /// A level now; `None` if the machine has none (no default output, no backlight).
    fn level(&mut self, control: Control) -> io::Result<Option<Level>>;
    /// Sets a level (0 to [`FULL_LEVEL`]); a volume is unmuted by it.
    fn set_level(&mut self, control: Control, value: u32) -> io::Result<()>;
    /// Mutes a volume or unmutes it.
    fn set_muted(&mut self, control: Control, muted: bool) -> io::Result<()>;
    /// Whether the machine's firmware changes the keyboard's backlight itself when its key is
    /// pressed, the key only telling it did (a ThinkPad's Fn+Space): then the session reads it
    /// and never sets it from that key.
    fn keyboard_changed_by_firmware(&mut self) -> bool;
    fn battery(&mut self) -> io::Result<Battery>;
    fn network(&mut self) -> io::Result<Network>;
    fn set_wifi(&mut self, enabled: bool) -> io::Result<()>;
    fn bluetooth(&mut self) -> io::Result<Bluetooth>;
}

/// Where a key moves a level from `value`: to the next of `steps` evenly spaced levels above
/// it (`up`) or below it, so a level between two goes to the one beyond, as a Mac's does; never
/// past either end. `resolution` is how far the real level may be from `value` (a level read
/// rounded): a value within it of a step is that step, so a key never lands where it started.
pub fn stepped(value: u32, steps: u32, up: bool, resolution: u32) -> u32 {
    let steps = u64::from(steps.max(1));
    let full = u64::from(FULL_LEVEL);
    let value = if up {
        value.saturating_add(resolution)
    } else {
        value.saturating_sub(resolution)
    };
    let value = u64::from(value.min(FULL_LEVEL));
    // Where the value is, in steps: below (floor) and above (ceiling).
    let below = value * steps / full;
    let above = (value * steps).div_ceil(full);
    let step = if up {
        (below + 1).min(steps)
    } else {
        above.saturating_sub(1)
    };
    (step * full / steps) as u32
}

/// How many steps of `steps` a level is, rounded: the shell's bezel draws that many.
pub fn steps_of(value: u32, steps: u32) -> u32 {
    let steps = u64::from(steps.max(1));
    ((u64::from(value.min(FULL_LEVEL)) * steps + u64::from(FULL_LEVEL) / 2) / u64::from(FULL_LEVEL))
        as u32
}

/// What the session asks of the system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    SetLevel(Control, u32),
    SetMuted(Control, bool),
    SetWifi(bool),
    /// A level looked at, its answer carrying `tag` back (the output a key's bezel shows on).
    ReadLevel(Control, Option<u32>),
    /// The battery, the network and Bluetooth looked at.
    ReadStatus,
    /// The four levels looked at.
    ReadLevels,
}

/// What came of a command: what the system has now. A level set or a mute is answered with the
/// level read again, so what the session says is what the system did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Level {
        control: Control,
        level: Option<Level>,
        tag: Option<u32>,
    },
    Battery(Battery),
    Network(Network),
    Bluetooth(Bluetooth),
    /// The keyboard's backlight is the firmware's ([`System::keyboard_changed_by_firmware`]).
    KeyboardByFirmware(bool),
}

/// One command run on `system`, and what it found. A call that fails is said once each time it
/// fails, and what it would have found is unavailable.
pub fn run(system: &mut dyn System, command: Command) -> Vec<Outcome> {
    let level = |system: &mut dyn System, control, tag| {
        let level = system.level(control).unwrap_or_else(|error| {
            eprintln!("[controls] {control:?}: {error}");
            None
        });
        Outcome::Level {
            control,
            level,
            tag,
        }
    };
    match command {
        Command::SetLevel(control, value) => {
            if let Err(error) = system.set_level(control, value.min(FULL_LEVEL)) {
                eprintln!("[controls] setting {control:?}: {error}");
            }
            vec![level(system, control, None)]
        }
        Command::SetMuted(control, muted) => {
            if let Err(error) = system.set_muted(control, muted) {
                eprintln!("[controls] muting {control:?}: {error}");
            }
            vec![level(system, control, None)]
        }
        Command::SetWifi(enabled) => {
            if let Err(error) = system.set_wifi(enabled) {
                eprintln!("[controls] turning Wi-Fi on ({enabled}): {error}");
            }
            vec![Outcome::Network(network(system))]
        }
        Command::ReadLevel(control, tag) => vec![level(system, control, tag)],
        Command::ReadLevels => {
            let mut outcomes = vec![Outcome::KeyboardByFirmware(
                system.keyboard_changed_by_firmware(),
            )];
            outcomes.extend(CONTROLS.iter().map(|control| level(system, *control, None)));
            outcomes
        }
        Command::ReadStatus => {
            let battery = system.battery().unwrap_or_else(|error| {
                eprintln!("[controls] the battery: {error}");
                Battery::ABSENT
            });
            let bluetooth = system.bluetooth().unwrap_or_else(|error| {
                eprintln!("[controls] Bluetooth: {error}");
                Bluetooth::UNAVAILABLE
            });
            vec![
                Outcome::Battery(battery),
                Outcome::Network(network(system)),
                Outcome::Bluetooth(bluetooth),
            ]
        }
    }
}

fn network(system: &mut dyn System) -> Network {
    system.network().unwrap_or_else(|error| {
        eprintln!("[controls] the network: {error}");
        Network::UNAVAILABLE
    })
}

/// Commands waiting, with every level set that a later one of the same control replaces left
/// out: a slider dragged faster than the system follows sets where it is, not where it was.
pub fn coalesce(commands: Vec<Command>) -> Vec<Command> {
    let mut kept: Vec<Command> = Vec::with_capacity(commands.len());
    for (index, command) in commands.iter().enumerate() {
        let replaced = matches!(command, Command::SetLevel(control, _)
        if commands[index + 1..].iter().any(
            |later| matches!(later, Command::SetLevel(other, _) if other == control)
        ));
        if !replaced {
            kept.push(command.clone());
        }
    }
    kept
}

/// Runs a [`System`]'s commands: on a thread of its own (a session's, whose calls wait on other
/// processes), or in the caller's turn (a [`Fake`]'s, so a scenario is the same every run).
pub enum Worker {
    Inline {
        system: Box<dyn System>,
        outcomes: Vec<Outcome>,
    },
    Thread {
        commands: Sender<Command>,
        outcomes: Receiver<Outcome>,
        /// Readable when outcomes are waiting (`AsFd`).
        wake: UnixStream,
    },
}

impl Worker {
    pub fn inline(system: Box<dyn System>) -> Self {
        Self::Inline {
            system,
            outcomes: Vec::new(),
        }
    }

    /// `system` on a thread of its own, which ends when the worker is dropped.
    pub fn spawn(mut system: Box<dyn System>) -> io::Result<Self> {
        let (commands, received) = channel::<Command>();
        let (sent, outcomes) = channel();
        let (wake, mut woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        std::thread::Builder::new()
            .name("oxyos-controls".into())
            .spawn(move || {
                let mut pending: Vec<Command> = Vec::new();
                loop {
                    if pending.is_empty() {
                        match received.recv() {
                            Ok(command) => pending.push(command),
                            Err(_) => return,
                        }
                    }
                    pending.extend(received.try_iter());
                    pending = coalesce(pending);
                    let command = pending.remove(0);
                    for outcome in run(system.as_mut(), command) {
                        if sent.send(outcome).is_err() {
                            return;
                        }
                    }
                    // The compositor's turn wakes to take them; a full socket already wakes it.
                    let _ = woken.write(&[1]);
                }
            })?;
        Ok(Self::Thread {
            commands,
            outcomes,
            wake,
        })
    }

    pub fn send(&mut self, command: Command) {
        match self {
            Self::Inline { system, outcomes } => outcomes.extend(run(system.as_mut(), command)),
            Self::Thread { commands, .. } => {
                // A worker that ended (its thread panicked) acts no more; the session goes on.
                let _ = commands.send(command);
            }
        }
    }

    /// What came of the commands since the last call, in order.
    pub fn outcomes(&mut self) -> Vec<Outcome> {
        match self {
            Self::Inline { outcomes, .. } => std::mem::take(outcomes),
            Self::Thread { outcomes, wake, .. } => {
                let mut drained = [0; 64];
                while matches!(wake.read(&mut drained), Ok(n) if n > 0) {}
                outcomes.try_iter().collect()
            }
        }
    }

    /// What wakes the compositor's turn when outcomes are waiting: none for an inline worker,
    /// whose outcomes are there as soon as it is sent a command.
    pub fn wake_fd(&self) -> Option<BorrowedFd<'_>> {
        match self {
            Self::Inline { .. } => None,
            Self::Thread { wake, .. } => Some(wake.as_fd()),
        }
    }
}
