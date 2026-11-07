//! Bluetooth as a session sees and uses it (docs/adr/0050-bluetooth.mdx), through BlueZ, the
//! system's Bluetooth daemon, on the system bus as the session's user.
//!
//! [`run`] is the session's one connection to BlueZ, on a thread of the caller's: it follows
//! BlueZ's objects (the adapter, its devices, their batteries) by the signals that can change what
//! a person is shown, and nothing else; it serves the machine's pairing agent (`org.bluez.Agent1`,
//! registered as BlueZ's default), whose every question is handed to the caller
//! ([`Event::Agent`]) and answered by the person; it holds discovery while the caller wants it
//! (BlueZ ends a discovery when the connection that started it goes, so a session that ends leaves
//! none running); and it makes the calls a person asks for (on and off, connecting, which pairs
//! first, disconnecting, forgetting, renaming). Everything waits on the bus, the caller's commands
//! and its order to stop: no clock.
//!
//! What a person is shown is [`State`]: built from BlueZ's objects ([`model::Objects`]), told
//! again only when it changes.

mod agent;
mod bluez;
pub mod model;

pub use bluez::{run, AGENT_PATH};

use serde::Serialize;

/// BlueZ's name on the bus.
pub const BLUEZ: &str = "org.bluez";

/// Bluetooth as the session tells the shell and Settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// BlueZ answers.
    pub available: bool,
    /// The adapter used (the first BlueZ has); none without Bluetooth hardware.
    pub adapter: Option<Adapter>,
    /// The adapter's devices a person is shown: every paired one, and the others that have a name
    /// of their own. By name, then address.
    pub devices: Vec<Device>,
}

impl State {
    /// No BlueZ: nothing to show.
    pub fn unavailable() -> Self {
        Self::default()
    }

    /// Whether Bluetooth is on: an adapter, powered.
    pub fn powered(&self) -> bool {
        self.adapter.as_ref().is_some_and(|adapter| adapter.powered)
    }

    /// The paired devices connected now.
    pub fn connected(&self) -> usize {
        self.devices
            .iter()
            .filter(|device| device.connected)
            .count()
    }

    pub fn device(&self, id: &str) -> Option<&Device> {
        self.devices.iter().find(|device| device.id == id)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Adapter {
    /// What other devices see this computer as.
    pub name: String,
    pub address: String,
    pub powered: bool,
    /// Off by rfkill (a hardware switch, airplane mode): it cannot be turned on.
    pub blocked: bool,
    pub discovering: bool,
    pub discoverable: bool,
}

/// What a device is, as its glyph shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Headphones,
    Speaker,
    Keyboard,
    Mouse,
    Trackpad,
    Phone,
    Gamepad,
    Computer,
    #[default]
    Other,
}

impl Kind {
    /// Its name in the vocabulary.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Headphones => "headphones",
            Kind::Speaker => "speaker",
            Kind::Keyboard => "keyboard",
            Kind::Mouse => "mouse",
            Kind::Trackpad => "trackpad",
            Kind::Phone => "phone",
            Kind::Gamepad => "gamepad",
            Kind::Computer => "computer",
            Kind::Other => "other",
        }
    }
}

/// A call of the session's under way on a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Busy {
    /// Connecting, pairing first if it is not paired.
    Connecting,
    Disconnecting,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// Its address, what every request names.
    pub id: String,
    pub address: String,
    /// Its alias (a name the person gave it) or its own name; its address when it has neither.
    pub name: String,
    pub kind: Kind,
    pub paired: bool,
    pub connected: bool,
    pub busy: Option<Busy>,
    /// 0 to 100, when it reports its battery (BlueZ's `Battery1`).
    pub battery: Option<u8>,
    /// It plays or records sound: PipeWire's BlueZ plugin makes it an output or input as it
    /// connects.
    pub audio: bool,
    /// A keyboard, a pointer or a game controller: the kernel makes it an input device as it
    /// connects, and the session's libinput takes it.
    pub input: bool,
}

/// A device's kind from what BlueZ says of it: its icon name (BlueZ's, from its class of device or
/// its appearance), else its services.
pub fn kind(icon: Option<&str>, uuids: &[String]) -> Kind {
    match icon.unwrap_or("") {
        "audio-headphones" | "audio-headset" => return Kind::Headphones,
        "audio-card" | "audio-speakers" => return Kind::Speaker,
        "input-keyboard" => return Kind::Keyboard,
        "input-mouse" => return Kind::Mouse,
        "input-tablet" => return Kind::Trackpad,
        "input-gaming" => return Kind::Gamepad,
        "phone" => return Kind::Phone,
        "computer" => return Kind::Computer,
        _ => {}
    }
    if uuids.iter().any(|uuid| has_short(uuid, AUDIO_SINKS)) {
        Kind::Headphones
    } else {
        Kind::Other
    }
}

/// The 16-bit number of a Bluetooth base UUID (`0000XXXX-0000-1000-8000-00805f9b34fb`).
fn short(uuid: &str) -> Option<u16> {
    let uuid = uuid.to_ascii_lowercase();
    let rest = uuid.strip_prefix("0000")?;
    let (number, base) = rest.split_at_checked(4)?;
    (base == "-0000-1000-8000-00805f9b34fb")
        .then(|| u16::from_str_radix(number, 16).ok())
        .flatten()
}

fn has_short(uuid: &str, among: &[u16]) -> bool {
    short(uuid).is_some_and(|number| among.contains(&number))
}

/// Where a device's sound goes: A2DP's sink, Hands-Free, Headset, LE Audio's stream control.
const AUDIO_SINKS: &[u16] = &[0x110b, 0x111e, 0x1108, 0x184e];
/// Every audio service, either way: the sinks, A2DP's source, the gateways, LE Audio's capabilities.
const AUDIO: &[u16] = &[
    0x110a, 0x110b, 0x110d, 0x1108, 0x1112, 0x111e, 0x111f, 0x184e, 0x1850,
];
/// Human interface devices: classic HID and HID over GATT.
const INPUT: &[u16] = &[0x1124, 0x1812];

/// Whether a device plays or records sound, by its services or its kind.
pub fn is_audio(kind: Kind, uuids: &[String]) -> bool {
    matches!(kind, Kind::Headphones | Kind::Speaker) || uuids.iter().any(|u| has_short(u, AUDIO))
}

/// Whether a device is an input device, by its services or its kind.
pub fn is_input(kind: Kind, uuids: &[String]) -> bool {
    matches!(
        kind,
        Kind::Keyboard | Kind::Mouse | Kind::Trackpad | Kind::Gamepad
    ) || uuids.iter().any(|u| has_short(u, INPUT))
}

/// What a service a device asks to use is, as the person is asked about it.
pub fn service(uuid: &str) -> &'static str {
    match short(uuid) {
        Some(n) if AUDIO.contains(&n) || n == 0x110c || n == 0x110e => "audio",
        Some(n) if INPUT.contains(&n) => "input",
        // Phone book, messages, SIM access.
        Some(0x112f | 0x1130 | 0x1132 | 0x1133 | 0x1134 | 0x112d) => "phone",
        // Object push and file transfer.
        Some(0x1105 | 0x1106) => "files",
        // PAN: networking.
        Some(0x1115..=0x1117) => "network",
        _ => "other",
    }
}

/// The longest name Bluetooth carries, in bytes.
pub const NAME_LIMIT: usize = 248;

/// A name as given to a device: without the spaces around it, cut at a whole character to what
/// Bluetooth carries. Empty, the device's own name comes back.
pub fn name_given(name: &str) -> String {
    let name = name.trim();
    let mut end = name.len().min(NAME_LIMIT);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_string()
}

/// Why a call did not do what it was asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    NotFound,
    /// No BlueZ, no adapter, or Bluetooth off.
    Unavailable,
    /// Off by rfkill.
    Blocked,
    /// The person refused or cancelled what the pairing asked.
    Cancelled,
    /// The other device refused, or the codes did not match.
    Rejected,
    Timeout,
    /// Another call on it is under way.
    Busy,
    Failed,
}

impl Failure {
    /// Its name in the vocabulary.
    pub fn reason(self) -> &'static str {
        match self {
            Failure::NotFound => "notFound",
            Failure::Unavailable => "unavailable",
            Failure::Blocked => "blocked",
            Failure::Cancelled => "cancelled",
            Failure::Rejected => "rejected",
            Failure::Timeout => "timeout",
            Failure::Busy => "busy",
            Failure::Failed => "failed",
        }
    }

    /// What a BlueZ error means, by its name and message.
    pub fn of(name: &str, message: &str) -> Failure {
        let error = name.rsplit('.').next().unwrap_or(name);
        let lower = message.to_ascii_lowercase();
        match error {
            "AuthenticationCanceled" => Failure::Cancelled,
            "AuthenticationRejected" | "AuthenticationFailed" => Failure::Rejected,
            "AuthenticationTimeout" | "ConnectionAttemptFailed" => Failure::Timeout,
            "InProgress" | "Busy" => Failure::Busy,
            "NotReady" => Failure::Unavailable,
            "DoesNotExist" | "UnknownObject" | "UnknownMethod" => Failure::NotFound,
            "ServiceUnknown" | "NameHasNoOwner" => Failure::Unavailable,
            _ if lower.contains("rfkill") || lower.contains("blocked") => Failure::Blocked,
            _ if lower.contains("timeout") || lower.contains("timed out") => Failure::Timeout,
            _ => Failure::Failed,
        }
    }
}

/// What a call did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Done {
    Powered(bool),
    Connected,
    /// Paired, with nothing of it to connect (a phone, a computer).
    Paired,
    Disconnected,
    Forgotten,
    Renamed(String),
}

/// What the caller asks of BlueZ. `call` names the answer ([`Event::Done`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Power {
        on: bool,
        call: u64,
    },
    /// Discovery wanted or not: while wanted and the adapter is on, it discovers and is
    /// discoverable.
    Discover(bool),
    /// Connects, pairing and trusting it first if it is not paired.
    Connect {
        id: String,
        call: u64,
    },
    Disconnect {
        id: String,
        call: u64,
    },
    Forget {
        id: String,
        call: u64,
    },
    Rename {
        id: String,
        name: String,
        call: u64,
    },
    /// Ends a pairing under way (its code shown is cancelled by the person).
    CancelPairing {
        id: String,
    },
}

impl Command {
    /// The device it acts on.
    pub fn device(&self) -> Option<&str> {
        match self {
            Command::Connect { id, .. }
            | Command::Disconnect { id, .. }
            | Command::Forget { id, .. }
            | Command::Rename { id, .. }
            | Command::CancelPairing { id } => Some(id),
            _ => None,
        }
    }
}

/// What the pairing agent is asked, as BlueZ asks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// A legacy PIN, typed by the person (`RequestPinCode`).
    PinCode,
    /// The six-digit passkey the other device shows, typed by the person (`RequestPasskey`).
    Passkey,
    /// A legacy PIN to type on the other device (a keyboard) (`DisplayPinCode`).
    ShowPinCode(String),
    /// A passkey to type on the other device (`DisplayPasskey`).
    ShowPasskey(u32),
    /// Whether the other device shows this passkey too (`RequestConfirmation`).
    Confirm(u32),
    /// Whether to pair with a device that asked, with nothing to compare (`RequestAuthorization`).
    Authorize,
    /// Whether a device may use a service (`AuthorizeService`): its UUID.
    Service(String),
}

impl Ask {
    /// Whether it only shows a code: answered at once, it ends with the pairing.
    pub fn shows(&self) -> bool {
        matches!(self, Ask::ShowPinCode(_) | Ask::ShowPasskey(_))
    }
}

/// The person's answer to an [`Ask`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Accept,
    Refuse,
    /// The PIN or passkey typed.
    Code(String),
}

/// A question of the pairing agent's: what is asked, of which device, and where its answer goes.
/// Dropped unanswered, it is refused.
#[derive(Debug)]
pub struct Question {
    /// Its number, which [`Event::QuestionEnded`] names.
    pub id: u64,
    pub ask: Ask,
    /// The device's address, name and kind, as BlueZ has them.
    pub address: String,
    pub name: String,
    pub kind: Kind,
    reply: Option<async_channel::Sender<Answer>>,
}

impl Question {
    /// A question, and where its answer arrives: what the agent asks (and a BlueZ played in
    /// memory, for the session's tests).
    pub fn new(
        id: u64,
        ask: Ask,
        address: String,
        name: String,
        kind: Kind,
    ) -> (Question, async_channel::Receiver<Answer>) {
        let (reply, answered) = async_channel::bounded(1);
        (
            Question {
                id,
                ask,
                address,
                name,
                kind,
                reply: Some(reply),
            },
            answered,
        )
    }

    /// Answers it (once).
    pub fn answer(mut self, answer: Answer) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.try_send(answer);
        }
    }
}

impl Drop for Question {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.try_send(Answer::Refuse);
        }
    }
}

/// What [`run`] tells its caller.
#[derive(Debug)]
pub enum Event {
    /// Bluetooth as a person is shown it: once at start, then as it changes.
    State(State),
    /// A command's outcome.
    Done {
        call: u64,
        outcome: Result<Done, Failure>,
    },
    /// The pairing agent asks the person.
    Question(Question),
    /// A question no longer stands (BlueZ cancelled it, it timed out, or the pairing it was of
    /// ended): what shows it goes.
    QuestionEnded(u64),
}

/// A PIN or passkey as typed, checked for what BlueZ takes: a legacy PIN is 1 to 16 characters
/// (`RequestPinCode`), a passkey 1 to 6 digits (`RequestPasskey`, 0 to 999999).
pub fn code_valid(ask: &Ask, code: &str) -> bool {
    match ask {
        Ask::PinCode => (1..=16).contains(&code.len()) && !code.chars().any(char::is_control),
        Ask::Passkey => (1..=6).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_digit()),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
