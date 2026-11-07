//! The sound's outputs and inputs (docs/adr/0056-sound-devices.mdx): what a person chooses to hear
//! through and speak into, as a Mac's Control Center and Sound settings show them, and the
//! volume, mute and balance of the one chosen. PipeWire is the session's one audio stack: its
//! devices are its `Audio/Sink` and `Audio/Source` nodes, each named by its device and by the
//! route it plays through (speakers, headphones, HDMI); the one chosen is WirePlumber's default
//! node, chosen as `wpctl set-default` chooses it, by its `default.configured.audio.sink` (or
//! `.source`) in the `default` metadata, which WirePlumber applies, remembers per device and
//! follows with every stream that follows the default.
//!
//! [`Audio`] is what the session asks; [`PipeWire`] is a session's, [`Played`] a machine played
//! for tests and the development compositor. What either has is a [`State`], told as it changes
//! ([`Event`]); [`takeover`] is the session's rule for a device plugged in or connected (a Mac's:
//! headphones take over).

use serde::{Deserialize, Serialize};
use std::os::fd::BorrowedFd;

pub mod graph;
mod pipewire_client;
mod played;
#[cfg(test)]
mod tests;

pub use pipewire_client::{pipewire_socket, PipeWire};
pub use played::Played;

/// Where sound goes: out of the machine (a sink) or into it (a source).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Output,
    Input,
}

impl Direction {
    /// WirePlumber's suffix for its keys: `default.audio.sink`, `default.configured.audio.source`.
    pub fn metadata_suffix(self) -> &'static str {
        match self {
            Direction::Output => "audio.sink",
            Direction::Input => "audio.source",
        }
    }
}

/// What a device is to the person, as its icon and a built-in one's name say it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Speakers,
    Headphones,
    /// Headphones with a microphone (a headset's, through the jack, USB or Bluetooth).
    Headset,
    /// A display's sound, through HDMI or DisplayPort.
    Hdmi,
    Line,
    Microphone,
    Other,
}

impl Kind {
    /// Worn on the head: what takes over when it comes, as a Mac's headphones do.
    pub fn worn(self) -> bool {
        matches!(self, Kind::Headphones | Kind::Headset)
    }
}

/// How it is connected: a Mac's Sound settings' *Type* column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Transport {
    /// The machine's own (its speakers, its microphone, its headphone jack).
    BuiltIn,
    /// A display's, through HDMI or DisplayPort.
    Hdmi,
    Usb,
    Bluetooth,
    /// No hardware: a program's (a filter, a sink of its own).
    Virtual,
    Other,
}

/// One output or input as a person sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Device {
    /// PipeWire's node name, which WirePlumber remembers it by: stable across restarts.
    pub id: String,
    /// What the system calls it: a display's or a headset's own name, a route's ("Speakers").
    /// The machine's own are named by their kind where they are shown.
    pub name: String,
    pub kind: Kind,
    pub transport: Transport,
    /// The one sound goes through (or comes from): WirePlumber's default.
    pub selected: bool,
}

/// One direction's devices, and the volume of the one chosen.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Side {
    /// Built-in first, then displays, USB and Bluetooth, each by name.
    pub devices: Vec<Device>,
    /// The chosen one's volume, 0 to 1 on WirePlumber's cubic scale (what `wpctl` and GNOME's
    /// slider show); `None` without one.
    pub volume: Option<f64>,
    pub muted: bool,
    /// Left (-1) to right (1), 0 centred; `None` where it has no left and right (a mono device,
    /// an input).
    pub balance: Option<f64>,
}

impl Side {
    pub fn selected(&self) -> Option<&Device> {
        self.devices.iter().find(|device| device.selected)
    }
}

/// What the session tells of the sound: whether PipeWire answers, and each direction.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub available: bool,
    pub output: Side,
    pub input: Side,
    /// Which connection to PipeWire it was read on (one more each time it is made): a device
    /// is only new to the person between two states of the same one.
    #[serde(skip)]
    pub generation: u64,
}

impl State {
    /// No PipeWire.
    pub fn unavailable() -> Self {
        Self::default()
    }

    pub fn side(&self, direction: Direction) -> &Side {
        match direction {
            Direction::Output => &self.output,
            Direction::Input => &self.input,
        }
    }

    pub fn side_mut(&mut self, direction: Direction) -> &mut Side {
        match direction {
            Direction::Output => &mut self.output,
            Direction::Input => &mut self.input,
        }
    }
}

/// What came of the sound since it was last looked at.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The state changed (told only when it did).
    State(State),
    /// The chosen input's level, 0 to 1, while it is metered ([`Audio::meter`]): told as it
    /// moves by a step of [`METER_STEPS`].
    InputLevel(f64),
}

/// The steps an input's level is told in: a Mac's level meter lights its segments one by one.
pub const METER_STEPS: u32 = 32;
/// The quietest sound a meter shows, in dB below full scale: below it, nothing is lit.
pub const METER_FLOOR_DB: f64 = 60.0;

/// What the session asks of the sound. Every call returns at once: a change is applied by the
/// system, and the next [`Event::State`] says what it then has.
pub trait Audio: Send + Sync {
    /// The state as last told.
    fn state(&self) -> State;
    /// Chooses a device (by its [`Device::id`]) for a direction.
    fn set_default(&self, direction: Direction, id: &str);
    /// The chosen device's volume (0 to 1), its balance kept.
    fn set_volume(&self, direction: Direction, volume: f64);
    fn set_muted(&self, direction: Direction, muted: bool);
    /// The chosen output's balance, -1 (left) to 1 (right).
    fn set_balance(&self, balance: f64);
    /// Meters the chosen input or stops: its level is told while on, and nothing listens
    /// to it while off.
    fn meter(&self, on: bool);
    /// What came since the last call, in order.
    fn take_events(&self) -> Vec<Event>;
    /// What wakes a waiting turn when events come; `None` where they come only as it is
    /// asked (a played machine's).
    fn wake_fd(&self) -> Option<BorrowedFd<'_>>;
}

/// A device plugged in or connected that takes over, as a Mac's does: the choices `after` asks
/// for, given the state `before` it on the same connection to PipeWire. An output takes over
/// when headphones or a headset come (a USB or Bluetooth one, or the machine's headphone jack
/// becoming its route) or a USB or Bluetooth output is connected; an input when a headset's
/// microphone comes, or a USB microphone, never a Bluetooth one (choosing it would turn the
/// headset's sound into a call's). A display's sound never does. The machine's state at the
/// session's start, or after PipeWire restarted, takes nothing over.
pub fn takeover(before: &State, after: &State) -> Vec<(Direction, String)> {
    if !before.available || !after.available || before.generation != after.generation {
        return Vec::new();
    }
    let mut chosen = Vec::new();
    for direction in [Direction::Output, Direction::Input] {
        let was = before.side(direction);
        let taking = after.side(direction).devices.iter().rev().find(|device| {
            if device.selected || !takes_over(direction, device) {
                return false;
            }
            match was.devices.iter().find(|old| old.id == device.id) {
                // Newly there.
                None => true,
                // The same device now worn: the jack's headphones became its route.
                Some(old) => !old.kind.worn() && device.kind.worn(),
            }
        });
        if let Some(device) = taking {
            chosen.push((direction, device.id.clone()));
        }
    }
    chosen
}

fn takes_over(direction: Direction, device: &Device) -> bool {
    match direction {
        Direction::Output => {
            device.transport != Transport::Hdmi
                && (device.kind.worn()
                    || matches!(device.transport, Transport::Usb | Transport::Bluetooth))
        }
        Direction::Input => {
            device.transport != Transport::Bluetooth
                && (device.kind == Kind::Headset
                    || (device.transport == Transport::Usb
                        && matches!(device.kind, Kind::Microphone | Kind::Headset)))
        }
    }
}

/// A volume on WirePlumber's cubic scale from PipeWire's linear one (what a channel's volume
/// is), rounded to 1/65536 so that what is told does not change by a float's noise.
pub fn cubic(linear: f32) -> f64 {
    if linear <= 0.0 || !linear.is_finite() {
        return 0.0;
    }
    rounded(f64::from(linear).cbrt(), 65536.0)
}

/// PipeWire's linear volume for a volume on the cubic scale.
pub fn linear(cubic: f64) -> f32 {
    let cubic = cubic.clamp(0.0, 1.0);
    (cubic * cubic * cubic) as f32
}

fn rounded(value: f64, steps: f64) -> f64 {
    (value * steps).round() / steps
}

/// Where each channel is: on the left, on the right, or neither (a centre, a subwoofer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Left,
    Right,
    Middle,
}

/// The volume and balance of channels' volumes (on the cubic scale), as PulseAudio and GNOME
/// read them: the loudest channel is the volume; the balance compares the left's average with
/// the right's. `None` balance where there is no left and right.
pub fn volume_and_balance(levels: &[f64], places: &[Place]) -> (f64, Option<f64>) {
    let volume = levels.iter().copied().fold(0.0, f64::max);
    let side = |place: Place| {
        let chosen: Vec<f64> = levels
            .iter()
            .zip(places)
            .filter(|(_, at)| **at == place)
            .map(|(level, _)| *level)
            .collect();
        (!chosen.is_empty()).then(|| chosen.iter().sum::<f64>() / chosen.len() as f64)
    };
    let (Some(left), Some(right)) = (side(Place::Left), side(Place::Right)) else {
        return (volume, None);
    };
    let balance = if left == right {
        0.0
    } else if right > left {
        1.0 - left / right
    } else {
        right / left - 1.0
    };
    (volume, Some(rounded(balance, 1000.0)))
}

/// Each channel's volume (on the cubic scale) for a volume and a balance: the side the balance
/// leans away from is lowered, the other and the middle channels are at the volume.
pub fn levels_for(volume: f64, balance: f64, places: &[Place]) -> Vec<f64> {
    let volume = volume.clamp(0.0, 1.0);
    let balance = balance.clamp(-1.0, 1.0);
    places
        .iter()
        .map(|place| match place {
            Place::Left => volume * (1.0 - balance.max(0.0)),
            Place::Right => volume * (1.0 + balance.min(0.0)),
            Place::Middle => volume,
        })
        .collect()
}

/// A meter's level for a peak (0 to 1 of full scale): its loudness in dB from
/// [`METER_FLOOR_DB`] below full scale to full scale, in [`METER_STEPS`].
pub fn meter_level(peak: f32) -> f64 {
    if peak <= 0.0 || !peak.is_finite() {
        return 0.0;
    }
    let db = 20.0 * f64::from(peak.min(1.0)).log10();
    let level = ((db + METER_FLOOR_DB) / METER_FLOOR_DB).clamp(0.0, 1.0);
    rounded(level, f64::from(METER_STEPS))
}
