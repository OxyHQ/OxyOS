//! Bluetooth for the program running the compositor (docs/adr/0050-bluetooth.mdx): the session's
//! side of the shell's and Settings' requests, over BlueZ, and the machine's pairing agent.
//!
//! The shell (its `shell` grant) and a client with `bluetooth` ask in the session's vocabulary
//! (`SystemRequest`, one JSON object read strictly, [`Ask`]); what changes the machine must cite an
//! input the client received. One thread holds the session's connection to BlueZ
//! (`oxyos_bluetooth::run`): it follows BlueZ by the signals that can change what is shown, makes
//! the calls asked, holds discovery while a client's Bluetooth page holds it and serves the pairing
//! agent; the compositor's turn never waits for it. Bluetooth's state is told to every client
//! holding `bluetooth` or `shell` as it changes, and the top bar's status from it.
//!
//! The agent's questions are the shell's system sheets: a code to compare (`bluetooth-confirm`),
//! one to type on the other device (`bluetooth-display`), one to type here (`bluetooth-code`,
//! answered by the shell alone with `{"bluetoothCode":…}`), a device asking to pair
//! (`bluetooth-authorize`) or to use a service (`bluetooth-service`). While locked, none is asked:
//! BlueZ is answered no. A connection that failed, but for the person's own cancel, is said in a
//! `bluetooth-failed` sheet. Discovery runs only while a client holds it (Settings' Bluetooth page
//! open), the session unlocked; it ends when the client lets it go or goes.

use crate::stop::{identity, Stop};
use oxyos_bluetooth::{Answer, Command, Done, Event, Failure, Question, State};
use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::{ClientId, Scene};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};

/// How the session reaches BlueZ.
pub trait Reach: Send + Sync {
    /// Runs the session's connection to BlueZ ([`oxyos_bluetooth::run`]): its events to `emit`,
    /// until `stop` is given, `commands` closes or BlueZ's bus closes.
    fn run(
        &self,
        stop: &Stop,
        commands: &async_channel::Receiver<Command>,
        emit: &mut dyn FnMut(Event),
    ) -> io::Result<()>;
    /// What to wait for to be made, when it cannot be reached, before it is tried again: the bus's
    /// socket. `None`: nothing to wait for; it is not tried again.
    fn socket(&self) -> Option<PathBuf>;
}

/// BlueZ on the system bus, or on the bus at `address` (the development compositor's played one).
pub struct Bus {
    pub address: Option<String>,
}

impl Reach for Bus {
    fn run(
        &self,
        stop: &Stop,
        commands: &async_channel::Receiver<Command>,
        emit: &mut dyn FnMut(Event),
    ) -> io::Result<()> {
        oxyos_bluetooth::run(self.address.as_deref(), stop.fd(), commands, emit)
    }

    fn socket(&self) -> Option<PathBuf> {
        oxyos_networks::bus_socket(self.address.as_deref())
    }
}

/// What a client asks, read strictly.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Ask {
    /// Bluetooth's state.
    Bluetooth {},
    /// With an input: Bluetooth on or off.
    BluetoothPower { on: bool },
    /// `bluetooth` alone: discovery held (its Bluetooth page open) or let go.
    BluetoothDiscover { on: bool },
    /// With an input: a device connected, paired first.
    BluetoothConnect { id: String },
    /// With an input.
    BluetoothDisconnect { id: String },
    /// `bluetooth` alone, with an input.
    BluetoothForget { id: String },
    /// `bluetooth` alone, with an input: a device's name; empty, its own.
    BluetoothRename { id: String, name: String },
    /// The shell alone, with an input: the PIN or passkey typed into a code sheet.
    BluetoothCode { sheet: u32, code: String },
}

/// The vocabulary's names: a request naming one of them is this part's.
const ASKS: [&str; 8] = [
    "bluetooth",
    "bluetoothPower",
    "bluetoothDiscover",
    "bluetoothConnect",
    "bluetoothDisconnect",
    "bluetoothForget",
    "bluetoothRename",
    "bluetoothCode",
];

fn ours(text: &str) -> bool {
    serde_json::from_str::<HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.keys().all(|key| ASKS.contains(&key.as_str())))
}

/// A client's call waiting for its outcome.
struct Pending {
    client: ClientId,
    serial: u32,
    /// The device it acts on, and whether it is a connection (whose failure is said in a sheet).
    id: String,
    connecting: bool,
}

/// A sheet shown: an agent's question (its number), or one that only tells.
enum Sheet {
    Question(u64),
    Note,
}

/// An agent's question standing, shown in `sheet`.
struct Standing {
    question: Question,
    sheet: u32,
}

#[derive(Clone)]
struct Back {
    done: mpsc::Sender<Event>,
    wake: Arc<UnixStream>,
}

impl Back {
    fn send(&self, event: Event) {
        if self.done.send(event).is_ok() {
            let _ = (&*self.wake).write(&[1]);
        }
    }
}

pub struct Bluetooth {
    reach: Arc<dyn Reach>,
    commands: async_channel::Sender<Command>,
    receiving: async_channel::Receiver<Command>,
    back: Back,
    done: mpsc::Receiver<Event>,
    woken: UnixStream,
    state: Option<State>,
    told: Option<String>,
    changed: bool,
    next_call: u64,
    pending: HashMap<u64, Pending>,
    /// Asked for the state before BlueZ was first read: answered as it is.
    waiting: Vec<(ClientId, u32)>,
    /// The clients holding discovery, and whether it was last asked on.
    holders: BTreeSet<ClientId>,
    discovering: bool,
    /// Who asked to connect each device: its sheets are asked for it.
    connecting: HashMap<String, ClientId>,
    standing: HashMap<u64, Standing>,
    sheets: BTreeMap<u32, Sheet>,
    stop: Arc<Stop>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Bluetooth {
    pub fn new(reach: Arc<dyn Reach>) -> io::Result<Self> {
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let (done, received) = mpsc::channel();
        let (commands, receiving) = async_channel::unbounded();
        Ok(Self {
            reach,
            commands,
            receiving,
            back: Back {
                done,
                wake: Arc::new(wake),
            },
            done: received,
            woken,
            state: None,
            told: None,
            changed: false,
            next_call: 0,
            pending: HashMap::new(),
            waiting: Vec::new(),
            holders: BTreeSet::new(),
            discovering: false,
            connecting: HashMap::new(),
            standing: HashMap::new(),
            sheets: BTreeMap::new(),
            stop: Arc::new(Stop::new()?),
            thread: None,
        })
    }

    /// BlueZ on the system bus, or at `address`.
    pub fn on_bus(address: Option<String>) -> io::Result<Self> {
        Self::new(Arc::new(Bus { address }))
    }

    /// What wakes the compositor's turn when the thread has something (`Compositor::wakers`).
    pub fn wake_fd(&self) -> io::Result<OwnedFd> {
        Ok(self.woken.try_clone()?.into())
    }

    /// Whether Bluetooth's state changed since the last call: the hardware controls are told (the
    /// Control Center's module, [`Self::status`]).
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    pub fn state(&self) -> Option<&State> {
        self.state.as_ref()
    }

    /// What the Control Center's module and the top bar say of Bluetooth (ADR 0025's status),
    /// from the state as last told.
    pub fn status(&self) -> Option<oxyos_controls::Bluetooth> {
        let state = self.state.as_ref()?;
        let adapters: Vec<bool> = state.adapter.iter().map(|a| a.powered).collect();
        Some(oxyos_controls::bluetooth::status(
            &adapters,
            state.connected(),
        ))
    }

    fn start(&mut self) {
        if self.thread.is_some() {
            return;
        }
        let (reach, back, stop, commands) = (
            self.reach.clone(),
            self.back.clone(),
            self.stop.clone(),
            self.receiving.clone(),
        );
        self.thread = std::thread::Builder::new()
            .name("bluetooth".into())
            .spawn(move || follow(reach.as_ref(), &back, &stop, &commands))
            .ok();
    }

    fn command(&self, command: Command) {
        let _ = self.commands.try_send(command);
    }

    /// A turn: the scene's requests that are this part's, discovery as its holders want it, the
    /// sheets answered, and what BlueZ said.
    pub fn serve(&mut self, scene: &mut Scene) {
        let mut buffer = [0u8; 64];
        while matches!((&self.woken).read(&mut buffer), Ok(read) if read > 0) {}
        self.start();
        for request in scene.take_system_requests_where(|request| ours(&request.text)) {
            self.handle(scene, request);
        }
        // Discovery: held by clients still here, never while locked.
        self.holders
            .retain(|client| scene.grants_of(*client).is_some());
        let wanted = !self.holders.is_empty() && !scene.is_locked();
        if wanted != self.discovering {
            self.discovering = wanted;
            self.command(Command::Discover(wanted));
        }
        self.connecting
            .retain(|_, client| scene.grants_of(*client).is_some());
        // The sheets' answers.
        let sheets: Vec<u32> = self.sheets.keys().copied().collect();
        for sheet in sheets {
            let Some(accepted) = scene.take_sheet_answer(sheet) else {
                continue;
            };
            if let Some(Sheet::Question(id)) = self.sheets.remove(&sheet) {
                if let Some(standing) = self.standing.remove(&id) {
                    self.answer(standing.question, accepted);
                }
            }
        }
        for event in self.done.try_iter().collect::<Vec<_>>() {
            match event {
                Event::State(state) => {
                    let was = self.state.as_ref().is_some_and(|state| state.available);
                    if !state.available {
                        self.gone(scene);
                    } else if !was && self.discovering {
                        // BlueZ reached again, on a connection of its own: discovery asked anew.
                        self.command(Command::Discover(true));
                    }
                    self.state = Some(state);
                    self.tell(scene);
                }
                Event::Done { call, outcome } => self.done(scene, call, outcome),
                Event::Question(question) => self.ask(scene, question),
                Event::QuestionEnded(id) => {
                    if let Some(standing) = self.standing.remove(&id) {
                        self.sheets.remove(&standing.sheet);
                        scene.dismiss_sheet(standing.sheet);
                    }
                }
            }
        }
        // Questions asked or answered change what is told of their devices.
        self.tell(scene);
    }

    /// BlueZ went (stopped, or its bus): every call under way is answered unavailable, and its
    /// questions are withdrawn.
    fn gone(&mut self, scene: &mut Scene) {
        for (_, pending) in self.pending.drain() {
            scene.system_text(
                pending.client,
                pending.serial,
                &failed(Failure::Unavailable.reason()).to_string(),
            );
        }
        self.connecting.clear();
        for (_, standing) in self.standing.drain() {
            self.sheets.remove(&standing.sheet);
            scene.dismiss_sheet(standing.sheet);
        }
    }

    /// The person's answer to a question's sheet: yes or no; a code shown, refused, ends its
    /// pairing (BlueZ was answered as it was shown).
    fn answer(&self, question: Question, accepted: bool) {
        if question.ask.shows() {
            if !accepted {
                self.command(Command::CancelPairing {
                    id: question.address.clone(),
                });
            }
            question.answer(Answer::Refuse);
            return;
        }
        question.answer(if accepted {
            Answer::Accept
        } else {
            Answer::Refuse
        });
    }

    /// An agent's question, as a sheet the shell shows; while locked, or with no sheet to ask
    /// it in, refused.
    fn ask(&mut self, scene: &mut Scene, question: Question) {
        if scene.is_locked() {
            question.answer(Answer::Refuse);
            return;
        }
        let mut sheet = json!({"name": question.name, "device": question.kind.name()});
        match &question.ask {
            oxyos_bluetooth::Ask::Confirm(passkey) => {
                sheet["kind"] = json!("bluetooth-confirm");
                sheet["code"] = json!(format!("{passkey:06}"));
            }
            oxyos_bluetooth::Ask::ShowPasskey(passkey) => {
                sheet["kind"] = json!("bluetooth-display");
                sheet["code"] = json!(format!("{passkey:06}"));
            }
            oxyos_bluetooth::Ask::ShowPinCode(pin) => {
                sheet["kind"] = json!("bluetooth-display");
                sheet["code"] = json!(pin);
            }
            oxyos_bluetooth::Ask::PinCode => {
                sheet["kind"] = json!("bluetooth-code");
                sheet["code"] = json!("pin");
            }
            oxyos_bluetooth::Ask::Passkey => {
                sheet["kind"] = json!("bluetooth-code");
                sheet["code"] = json!("passkey");
            }
            oxyos_bluetooth::Ask::Authorize => sheet["kind"] = json!("bluetooth-authorize"),
            oxyos_bluetooth::Ask::Service(uuid) => {
                sheet["kind"] = json!("bluetooth-service");
                sheet["service"] = json!(oxyos_bluetooth::service(uuid));
            }
        }
        let asker = self.connecting.get(&question.address).copied();
        match scene.show_sheet(asker, &sheet.to_string()) {
            Some(shown) => {
                self.sheets.insert(shown, Sheet::Question(question.id));
                self.standing.insert(
                    question.id,
                    Standing {
                        question,
                        sheet: shown,
                    },
                );
            }
            None => question.answer(Answer::Refuse),
        }
    }

    fn done(&mut self, scene: &mut Scene, call: u64, outcome: Result<Done, Failure>) {
        let Some(pending) = self.pending.remove(&call) else {
            return;
        };
        if pending.connecting {
            self.connecting.remove(&pending.id);
        }
        let text = match outcome {
            Ok(Done::Powered(on)) => json!({"powered": {"on": on}}),
            Ok(Done::Connected) => json!({"connected": {"id": pending.id}}),
            Ok(Done::Paired) => json!({"paired": {"id": pending.id}}),
            Ok(Done::Disconnected) => json!({"disconnected": {"id": pending.id}}),
            Ok(Done::Forgotten) => json!({"forgotten": {"id": pending.id}}),
            Ok(Done::Renamed(name)) => json!({"renamed": {"id": pending.id, "name": name}}),
            Err(failure) => {
                // A connection that failed is said, but for the person's own cancel.
                if pending.connecting && failure != Failure::Cancelled {
                    let device = self.state.as_ref().and_then(|s| s.device(&pending.id));
                    let note = json!({
                        "kind": "bluetooth-failed",
                        "name": device.map_or(pending.id.as_str(), |d| d.name.as_str()),
                        "device": device.map_or("other", |d| d.kind.name()),
                        "reason": failure.reason(),
                    });
                    if let Some(sheet) = scene.show_sheet(None, &note.to_string()) {
                        self.sheets.insert(sheet, Sheet::Note);
                    }
                }
                failed(failure.reason())
            }
        };
        scene.system_text(pending.client, pending.serial, &text.to_string());
    }

    /// One request of this part's vocabulary: read strictly, checked against the grant it needs
    /// and, if it changes anything, an input the client received.
    pub fn handle(&mut self, scene: &mut Scene, request: SystemRequest) {
        let (client, serial) = (request.client, request.serial);
        let refuse = |scene: &mut Scene, reason: &str| {
            scene.system_text(client, serial, &refused(reason).to_string());
        };
        let Ok(ask) = serde_json::from_str::<Ask>(&request.text) else {
            refuse(scene, "invalidRequest");
            return;
        };
        let Some(grants) = scene.grants_of(client) else {
            return;
        };
        let allowed = match ask {
            Ask::BluetoothCode { .. } => grants.shell,
            Ask::BluetoothDiscover { .. }
            | Ask::BluetoothForget { .. }
            | Ask::BluetoothRename { .. } => grants.bluetooth,
            _ => grants.bluetooth || grants.shell,
        };
        if !allowed {
            refuse(scene, "notPermitted");
            return;
        }
        if matches!(ask, Ask::Bluetooth {}) {
            match self.shown() {
                Some(state) => {
                    let text = json!({ "bluetooth": state }).to_string();
                    scene.system_text(client, serial, &text);
                }
                None => self.waiting.push((client, serial)),
            }
            return;
        }
        if scene.is_locked() {
            refuse(scene, "locked");
            return;
        }
        if let Ask::BluetoothDiscover { on } = ask {
            if on {
                self.holders.insert(client);
            } else {
                self.holders.remove(&client);
            }
            scene.system_text(
                client,
                serial,
                &json!({"discovering": {"on": on}}).to_string(),
            );
            return;
        }
        if !request.with_input {
            refuse(scene, "needsInput");
            return;
        }
        if let Ask::BluetoothCode { sheet, code } = ask {
            self.code(scene, client, serial, sheet, code);
            return;
        }
        self.next_call += 1;
        let call = self.next_call;
        let (command, id, connecting) = match ask {
            Ask::BluetoothPower { on } => (Command::Power { on, call }, String::new(), false),
            Ask::BluetoothConnect { id } => {
                self.connecting.insert(id.clone(), client);
                (
                    Command::Connect {
                        id: id.clone(),
                        call,
                    },
                    id,
                    true,
                )
            }
            Ask::BluetoothDisconnect { id } => (
                Command::Disconnect {
                    id: id.clone(),
                    call,
                },
                id,
                false,
            ),
            Ask::BluetoothForget { id } => (
                Command::Forget {
                    id: id.clone(),
                    call,
                },
                id,
                false,
            ),
            Ask::BluetoothRename { id, name } => (
                Command::Rename {
                    id: id.clone(),
                    name,
                    call,
                },
                id,
                false,
            ),
            Ask::Bluetooth {} | Ask::BluetoothDiscover { .. } | Ask::BluetoothCode { .. } => return,
        };
        self.pending.insert(
            call,
            Pending {
                client,
                serial,
                id,
                connecting,
            },
        );
        self.command(command);
    }

    /// A code typed into a code sheet: the shell's, for a sheet showing a question that asks one,
    /// and one BlueZ takes.
    fn code(&mut self, scene: &mut Scene, client: ClientId, serial: u32, sheet: u32, code: String) {
        let id = match self.sheets.get(&sheet) {
            Some(Sheet::Question(id)) => *id,
            _ => {
                scene.system_text(client, serial, &refused("noSuchSheet").to_string());
                return;
            }
        };
        let valid = self
            .standing
            .get(&id)
            .is_some_and(|standing| oxyos_bluetooth::code_valid(&standing.question.ask, &code));
        if !valid {
            scene.system_text(client, serial, &refused("invalidRequest").to_string());
            return;
        }
        self.sheets.remove(&sheet);
        scene.dismiss_sheet(sheet);
        if let Some(standing) = self.standing.remove(&id) {
            standing.question.answer(Answer::Code(code));
        }
    }

    /// The state as told: a device whose pairing asks the person something is not told busy
    /// meanwhile, since it waits on the person, not on the radio (as a Wi-Fi join is not told
    /// joining while its password is asked, ADR 0038).
    fn shown(&self) -> Option<State> {
        let mut state = self.state.clone()?;
        for device in &mut state.devices {
            if self
                .standing
                .values()
                .any(|standing| standing.question.address == device.address)
            {
                device.busy = None;
            }
        }
        Some(state)
    }

    /// Tells every client that follows Bluetooth its state, if it changed.
    fn tell(&mut self, scene: &mut Scene) {
        let Some(state) = self.shown() else {
            return;
        };
        let text = json!({ "bluetooth": state }).to_string();
        for (client, serial) in std::mem::take(&mut self.waiting) {
            scene.system_text(client, serial, &text);
        }
        if self.told.as_deref() == Some(text.as_str()) {
            return;
        }
        self.changed = true;
        for client in scene.bluetooth_watchers() {
            scene.system_text(client, 0, &text);
        }
        self.told = Some(text);
    }
}

impl Drop for Bluetooth {
    fn drop(&mut self) {
        self.stop.give();
        // Questions standing are refused as they go.
        self.standing.clear();
        self.commands.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn failed(reason: &str) -> Value {
    json!({"failed": {"reason": reason}})
}

fn refused(reason: &str) -> Value {
    json!({"refused": {"reason": reason}})
}

/// Holds the session's connection to BlueZ until the session ends. When it cannot be held (no bus
/// yet, or the bus restarted), Bluetooth is unavailable until the bus's socket is made (anew), and
/// it is held again: the thread waits on the bus or that socket, never a clock.
fn follow(
    reach: &dyn Reach,
    back: &Back,
    stop: &Stop,
    commands: &async_channel::Receiver<Command>,
) {
    let mut failed_on = None;
    loop {
        if let Some(socket) = reach.socket() {
            if !stop.wait_for_path(&socket, failed_on) {
                return;
            }
        }
        let tried = reach.socket().and_then(|socket| identity(&socket));
        let result = reach.run(stop, commands, &mut |event| back.send(event));
        if stop.given() || commands.is_closed() {
            return;
        }
        if let Err(error) = result {
            eprintln!("[bluetooth] BlueZ's bus: {error}");
        }
        back.send(Event::State(State::unavailable()));
        if reach.socket().is_none() {
            stop.wait();
            return;
        }
        failed_on = tried;
    }
}

#[cfg(test)]
mod tests;
