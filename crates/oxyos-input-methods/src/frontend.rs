//! The session as fcitx5's frontend (`org.fcitx.Fcitx.InputContext1`), on a thread of its own.
//!
//! Every message the bus delivers (replies and signals alike) and every command the compositor
//! sends go through one queue, in order, so a signal the engine sent before answering a call is
//! handled before the answer, and a key's answer before the next key. A call waits for its reply
//! there; commands that come meanwhile wait their turn.

use crate::engine::{Engine, Paths, Processes};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::num::NonZeroU32;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use zbus::blocking::Connection;
use zbus::message::Type;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Structure};
use zbus::Message;

const SERVICE: &str = "org.fcitx.Fcitx5";
const INPUT_METHOD_PATH: &str = "/org/freedesktop/portal/inputmethod";
const INPUT_METHOD: &str = "org.fcitx.Fcitx.InputMethod1";
const INPUT_CONTEXT: &str = "org.fcitx.Fcitx.InputContext1";
const CONTROLLER_PATH: &str = "/controller";
const CONTROLLER: &str = "org.fcitx.Fcitx.Controller1";
/// How long a call may take: past it, the engine is taken as stuck and started again.
const CALL_LIMIT: Duration = Duration::from_secs(5);
/// The least time between two starts of the engine.
const RESTART_EVERY: Duration = Duration::from_secs(3);

/// fcitx5's capability flags the frontend sets (`fcitx-utils/capabilityflags.h`).
pub mod capability {
    pub const PREEDIT: u64 = 1 << 1;
    pub const FORMATTED_PREEDIT: u64 = 1 << 4;
    pub const SURROUNDING_TEXT: u64 = 1 << 6;
    pub const EMAIL: u64 = 1 << 7;
    pub const DIGIT: u64 = 1 << 8;
    pub const URL: u64 = 1 << 12;
    pub const DIALABLE: u64 = 1 << 13;
    pub const NUMBER: u64 = 1 << 14;
    pub const NO_SPELL_CHECK: u64 = 1 << 17;
    pub const NAME: u64 = 1 << 22;
    pub const DATE: u64 = 1 << 33;
    pub const TIME: u64 = 1 << 34;
    pub const MULTILINE: u64 = 1 << 35;
    pub const KEY_EVENT_ORDER_FIX: u64 = 1 << 37;
    pub const CLIENT_SIDE_INPUT_PANEL: u64 = 1 << 39;
    /// What every field has: a formatted composition drawn by the field, the surrounding text,
    /// candidates drawn by the system, a key's effect in its reply.
    pub const ALWAYS: u64 = PREEDIT
        | FORMATTED_PREEDIT
        | SURROUNDING_TEXT
        | KEY_EVENT_ORDER_FIX
        | CLIENT_SIDE_INPUT_PANEL;
    /// Every flag the frontend may set (`SetSupportedCapability`).
    pub const SUPPORTED: u64 = ALWAYS
        | EMAIL
        | DIGIT
        | URL
        | DIALABLE
        | NUMBER
        | NO_SPELL_CHECK
        | NAME
        | DATE
        | TIME
        | MULTILINE;
}

/// fcitx5's text format flags for a part of the composition (`TextFormatFlag`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreeditFormat(pub i32);

impl PreeditFormat {
    pub const UNDERLINE: i32 = 1 << 3;
    pub const HIGHLIGHT: i32 = 1 << 4;

    /// The part being converted (Anthy's selected clause, a highlighted candidate's text).
    pub fn converting(self) -> bool {
        self.0 & Self::HIGHLIGHT != 0
    }
}

/// The engine's composition: its parts in order, each with its format, and the caret within it
/// in bytes of the whole (negative: none).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preedit {
    pub parts: Vec<(String, PreeditFormat)>,
    pub cursor: i32,
}

impl Preedit {
    pub fn text(&self) -> String {
        self.parts.iter().map(|(text, _)| text.as_str()).collect()
    }
}

/// What the engine did to the field, applied as one transaction: text around the caret deleted
/// (`offset` characters from it, `size` characters), then text committed, then the composition
/// replaced (`None`: as it was).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub delete: Option<(i32, u32)>,
    pub commit: String,
    pub preedit: Option<Preedit>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.delete.is_none() && self.commit.is_empty() && self.preedit.is_none()
    }

    fn add(&mut self, other: Change) {
        if other.delete.is_some() {
            self.delete = other.delete;
        }
        self.commit.push_str(&other.commit);
        if other.preedit.is_some() {
            self.preedit = other.preedit;
        }
    }
}

/// The candidates the engine shows now (`UpdateClientSideUI`), for the shell to draw.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Panel {
    pub candidates: Vec<String>,
    /// The highlighted one, if any.
    pub highlighted: Option<usize>,
    /// Above the candidates (a reading, QuickPhrase's prompt) and below them.
    pub aux_up: String,
    pub aux_down: String,
    pub previous: bool,
    pub next: bool,
    pub vertical: bool,
}

impl Panel {
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty() && self.aux_up.is_empty() && self.aux_down.is_empty()
    }
}

/// The person's input methods for fcitx5: its group (the layout's keyboard engine first, then
/// the input methods, by fcitx5's names) and the current one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    /// The XKB layout (`es`), the group's default.
    pub layout: String,
    pub group: Vec<String>,
    pub current: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// The group and the current input method.
    Sources(Sources),
    /// A field to serve, with its capability flags beyond [`capability::ALWAYS`].
    Activate {
        capability: u64,
    },
    /// The field served no more: the context focused out and reset.
    Deactivate,
    /// The field's text around the caret, and its selection, in characters.
    Surrounding {
        text: String,
        cursor: u32,
        anchor: u32,
    },
    /// A key, as the keymap made it (keysym, Linux input code, X11 modifiers).
    Key {
        serial: u64,
        keysym: u32,
        code: u32,
        modifiers: u32,
        pressed: bool,
        time_ms: u32,
    },
    /// The shell pressed a candidate, or turned the page.
    Choose(u8),
    Page {
        forward: bool,
    },
    /// Emoji in QuickPhrase, searched in `keyboard` (fcitx5's keyboard engine for the session's
    /// language), the key the person pressed for it.
    Emoji {
        keyboard: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The engine is running, with its input context.
    Ready,
    /// The answer to a key: whether the engine took it, and what it did meanwhile.
    Key {
        serial: u64,
        used: bool,
        change: Change,
    },
    /// What the engine did on its own (a candidate chosen, a page turned, a commit when focus
    /// left), as one transaction.
    Change(Change),
    /// The candidates to show (empty: none).
    Panel(Panel),
    /// The engine stopped; it starts again on its own.
    Stopped(String),
}

enum Input {
    Command(Command),
    Message(Message),
    Closed,
    /// The service is dropped: the engine stops and the thread ends.
    Shutdown,
}

/// What ends serving the engine: the service dropped, or the engine failing.
enum Stop {
    Shutdown,
    Failed(io::Error),
}

impl From<io::Error> for Stop {
    fn from(error: io::Error) -> Self {
        Stop::Failed(error)
    }
}

/// The engine running on a thread of its own, started again if it stops.
pub struct Service {
    inputs: Sender<Input>,
    outcomes: Receiver<Outcome>,
    /// Readable when outcomes are waiting.
    wake: UnixStream,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Service {
    /// The engine's processes stop before the service is gone.
    fn drop(&mut self) {
        let _ = self.inputs.send(Input::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Service {
    /// Starts `engine` with `paths`, serving `sources`.
    pub fn spawn(engine: Engine, paths: Paths, sources: Sources) -> io::Result<Self> {
        let (inputs, received) = channel::<Input>();
        let (sent, outcomes) = channel();
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let feed = inputs.clone();
        let thread = std::thread::Builder::new()
            .name("oxyos-input-methods".into())
            .spawn(move || {
                let mut worker = Worker {
                    engine,
                    paths,
                    sources,
                    inputs: received,
                    feed,
                    outcomes: sent,
                    woken,
                    deferred: VecDeque::new(),
                    active: None,
                    surrounding: None,
                };
                worker.run();
            })?;
        Ok(Self {
            inputs,
            outcomes,
            wake,
            thread: Some(thread),
        })
    }

    pub fn send(&self, command: Command) {
        // A worker that ended (its thread panicked) acts no more; the session goes on.
        let _ = self.inputs.send(Input::Command(command));
    }

    /// What the engine did since the last call, in order.
    pub fn outcomes(&mut self) -> Vec<Outcome> {
        let mut drained = [0; 64];
        while matches!(self.wake.read(&mut drained), Ok(n) if n > 0) {}
        self.outcomes.try_iter().collect()
    }

    /// What wakes the compositor's turn when outcomes are waiting.
    pub fn wake_fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }
}

/// A running engine: its processes, the bus and the input context.
struct Running {
    _processes: Processes,
    connection: Connection,
    context: OwnedObjectPath,
}

struct Worker {
    engine: Engine,
    paths: Paths,
    sources: Sources,
    inputs: Receiver<Input>,
    /// For the bus's reader thread.
    feed: Sender<Input>,
    outcomes: Sender<Outcome>,
    woken: UnixStream,
    /// Commands that came while a call waited for its reply.
    deferred: VecDeque<Command>,
    /// The field served (its capability flags), and its latest surrounding text, told again to
    /// an engine started anew.
    active: Option<u64>,
    surrounding: Option<(String, u32, u32)>,
}

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

/// The text of fcitx5's formatted text (`a(si)`), its parts joined.
fn joined(parts: &[(String, i32)]) -> String {
    parts.iter().map(|(text, _)| text.as_str()).collect()
}

fn preedit_of(parts: Vec<(String, i32)>, cursor: i32) -> Preedit {
    Preedit {
        parts: parts
            .into_iter()
            .map(|(text, format)| (text, PreeditFormat(format)))
            .collect(),
        cursor,
    }
}

/// A batched event of `ProcessKeyEventBatch`'s reply (`BATCHED_*`), as a change.
fn batched(kind: u32, value: OwnedValue) -> Option<Change> {
    match kind {
        0 => String::try_from(value).ok().map(|commit| Change {
            commit,
            ..Change::default()
        }),
        1 => {
            let structure = Structure::try_from(value).ok()?;
            let (parts, cursor) = <(Vec<(String, i32)>, i32)>::try_from(structure).ok()?;
            Some(Change {
                preedit: Some(preedit_of(parts, cursor)),
                ..Change::default()
            })
        }
        3 => {
            let structure = Structure::try_from(value).ok()?;
            let (offset, size) = <(i32, u32)>::try_from(structure).ok()?;
            Some(Change {
                delete: Some((offset, size)),
                ..Change::default()
            })
        }
        // A forwarded key (2) is the key itself given back; the caller sees it as not used.
        _ => None,
    }
}

impl Worker {
    fn tell(&mut self, outcome: Outcome) {
        if self.outcomes.send(outcome).is_ok() {
            let _ = self.woken.write(&[1]);
        }
    }

    /// Runs the engine, and starts it again whenever it stops, until the service is dropped.
    fn run(&mut self) {
        let mut last_start: Option<Instant> = None;
        loop {
            if let Some(started) = last_start {
                let wait = RESTART_EVERY.saturating_sub(started.elapsed());
                if !self.idle_for(wait) {
                    return;
                }
            }
            last_start = Some(Instant::now());
            let running = match self.start() {
                Ok(running) => running,
                Err(Stop::Shutdown) => return,
                Err(Stop::Failed(error)) => {
                    let why = self.engine.failure(&self.paths, &error);
                    self.tell(Outcome::Stopped(why));
                    continue;
                }
            };
            self.tell(Outcome::Ready);
            match self.serve(&running) {
                Stop::Shutdown => return,
                Stop::Failed(error) => {
                    let why = self.engine.failure(&self.paths, &error);
                    self.tell(Outcome::Stopped(why));
                }
            }
        }
    }

    /// While the engine is down: keys are given back at once, everything else is kept for when
    /// it is up (only the latest sources and field state matter). False once the compositor is
    /// gone.
    fn idle_for(&mut self, wait: Duration) -> bool {
        let until = Instant::now() + wait;
        loop {
            while let Some(command) = self.deferred.pop_front() {
                self.while_down(command);
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            match self.inputs.recv_timeout(left) {
                Ok(Input::Command(command)) => self.while_down(command),
                Ok(Input::Shutdown) | Err(RecvTimeoutError::Disconnected) => return false,
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => return true,
            }
        }
    }

    fn while_down(&mut self, command: Command) {
        match command {
            Command::Key { serial, .. } => self.tell(Outcome::Key {
                serial,
                used: false,
                change: Change::default(),
            }),
            Command::Sources(sources) => self.sources = sources,
            Command::Activate { capability } => self.active = Some(capability),
            Command::Deactivate => {
                self.active = None;
                self.surrounding = None;
            }
            Command::Surrounding {
                text,
                cursor,
                anchor,
            } => self.surrounding = Some((text, cursor, anchor)),
            Command::Choose(_) | Command::Page { .. } | Command::Emoji { .. } => {}
        }
    }

    fn start(&mut self) -> Result<Running, Stop> {
        let bus = self.engine.start_bus(&self.paths)?;
        self.engine.write_config(
            &self.paths,
            &self.sources.group,
            &self.sources.current,
            &self.sources.layout,
        )?;
        let mut processes = self.engine.start_fcitx(&self.paths, bus)?;
        let connection =
            zbus::blocking::connection::Builder::address(self.paths.address().as_str())
                .and_then(|builder| builder.build())
                .map_err(failed)?;
        // fcitx5 is up once it owns its name (pinyin's dictionaries load first).
        let dbus = zbus::blocking::fdo::DBusProxy::new(&connection).map_err(failed)?;
        let start = Instant::now();
        loop {
            let owned = dbus
                .name_has_owner(SERVICE.try_into().map_err(failed)?)
                .map_err(failed)?;
            if owned {
                break;
            }
            if !processes.running() {
                return Err(failed("fcitx5 exited as it started").into());
            }
            if start.elapsed() > Duration::from_secs(20) {
                return Err(failed("fcitx5 did not start").into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // Every message the bus delivers goes through the worker's queue, in order.
        let feed = self.feed.clone();
        let reading = connection.clone();
        std::thread::Builder::new()
            .name("oxyos-input-methods-bus".into())
            .spawn(move || {
                for message in zbus::blocking::MessageIterator::from(&reading) {
                    let Ok(message) = message else { break };
                    if feed.send(Input::Message(message)).is_err() {
                        return;
                    }
                }
                let _ = feed.send(Input::Closed);
            })?;
        let mut running = Running {
            _processes: processes,
            connection,
            context: OwnedObjectPath::default(),
        };
        let reply = self.call(
            &running,
            INPUT_METHOD_PATH,
            INPUT_METHOD,
            "CreateInputContext",
            &(vec![
                ("program".to_owned(), "oxyos".to_owned()),
                ("display".to_owned(), "oxyos".to_owned()),
            ],),
        )?;
        let (context, _uuid): (OwnedObjectPath, Vec<u8>) =
            reply.body().deserialize().map_err(failed)?;
        running.context = context;
        self.context_call(
            &running,
            "SetSupportedCapability",
            &(capability::SUPPORTED,),
        )?;
        // A field being served when the engine (re)started is served again.
        if let Some(extra) = self.active {
            self.activate(&running, extra)?;
            if let Some((text, cursor, anchor)) = self.surrounding.clone() {
                self.context_call(&running, "SetSurroundingText", &(text, cursor, anchor))?;
            }
        }
        Ok(running)
    }

    fn activate(&mut self, running: &Running, extra: u64) -> Result<(), Stop> {
        self.context_call(running, "SetCapability", &(capability::ALWAYS | extra,))?;
        self.context_call(running, "FocusIn", &())?;
        // A context starts on the group's first input method (the layout's): the current one
        // is the focused context's from now.
        let current = self.sources.current.clone();
        self.call(
            running,
            CONTROLLER_PATH,
            CONTROLLER,
            "SetCurrentIM",
            &(current.as_str(),),
        )?;
        Ok(())
    }

    /// Serves commands until the service is dropped or the engine stops.
    fn serve(&mut self, running: &Running) -> Stop {
        let mut pending = Change::default();
        loop {
            let command = match self.deferred.pop_front() {
                Some(command) => command,
                None => {
                    // Signals the engine sent on its own are one change, told once the queue
                    // holds nothing more just now.
                    let input = match self.inputs.try_recv() {
                        Ok(input) => input,
                        Err(_) => {
                            if !pending.is_empty() {
                                self.tell(Outcome::Change(std::mem::take(&mut pending)));
                            }
                            match self.inputs.recv() {
                                Ok(input) => input,
                                Err(_) => return Stop::Shutdown,
                            }
                        }
                    };
                    match input {
                        Input::Command(command) => command,
                        Input::Message(message) => {
                            self.signal(running, &message, &mut pending);
                            continue;
                        }
                        Input::Closed => {
                            return Stop::Failed(failed("the input methods' bus closed"))
                        }
                        Input::Shutdown => return Stop::Shutdown,
                    }
                }
            };
            if !pending.is_empty() {
                self.tell(Outcome::Change(std::mem::take(&mut pending)));
            }
            // A key the engine did not answer goes on to the field.
            let serial = match &command {
                Command::Key { serial, .. } => Some(*serial),
                _ => None,
            };
            if let Err(stop) = self.command(running, command) {
                if let Some(serial) = serial {
                    self.tell(Outcome::Key {
                        serial,
                        used: false,
                        change: Change::default(),
                    });
                }
                return stop;
            }
        }
    }

    fn command(&mut self, running: &Running, command: Command) -> Result<(), Stop> {
        match command {
            Command::Sources(sources) => {
                let entries: Vec<(String, String)> = sources
                    .group
                    .iter()
                    .map(|name| (name.clone(), String::new()))
                    .collect();
                self.call(
                    running,
                    CONTROLLER_PATH,
                    CONTROLLER,
                    "SetInputMethodGroupInfo",
                    &("OxyOS", sources.layout.as_str(), entries),
                )?;
                self.call(
                    running,
                    CONTROLLER_PATH,
                    CONTROLLER,
                    "SetCurrentIM",
                    &(sources.current.as_str(),),
                )?;
                self.sources = sources;
            }
            Command::Activate { capability } => {
                self.active = Some(capability);
                self.surrounding = None;
                self.activate(running, capability)?;
            }
            Command::Deactivate => {
                self.active = None;
                self.surrounding = None;
                self.context_call(running, "FocusOut", &())?;
                self.context_call(running, "Reset", &())?;
            }
            Command::Surrounding {
                text,
                cursor,
                anchor,
            } => {
                self.context_call(
                    running,
                    "SetSurroundingText",
                    &(text.as_str(), cursor, anchor),
                )?;
                self.surrounding = Some((text, cursor, anchor));
            }
            Command::Key {
                serial,
                keysym,
                code,
                modifiers,
                pressed,
                time_ms,
            } => {
                let (used, change) =
                    self.key(running, keysym, code, modifiers, pressed, time_ms)?;
                self.tell(Outcome::Key {
                    serial,
                    used,
                    change,
                });
            }
            Command::Choose(index) => {
                let change = self.collecting(running, "SelectCandidate", &(i32::from(index),))?;
                if !change.is_empty() {
                    self.tell(Outcome::Change(change));
                }
            }
            Command::Page { forward } => {
                let method = if forward { "NextPage" } else { "PrevPage" };
                let change = self.collecting(running, method, &())?;
                if !change.is_empty() {
                    self.tell(Outcome::Change(change));
                }
            }
            Command::Emoji { keyboard } => {
                self.call(
                    running,
                    CONTROLLER_PATH,
                    CONTROLLER,
                    "SetCurrentIM",
                    &(keyboard.as_str(),),
                )?;
                // QuickPhrase's trigger (engine.rs): Control+Super+Space, press and release.
                const SPACE_KEYSYM: u32 = 0x20;
                const SPACE_CODE: u32 = 57;
                const CONTROL_SUPER: u32 = 4 | 64;
                for pressed in [true, false] {
                    let (_, change) =
                        self.key(running, SPACE_KEYSYM, SPACE_CODE, CONTROL_SUPER, pressed, 0)?;
                    if !change.is_empty() {
                        self.tell(Outcome::Change(change));
                    }
                }
            }
        }
        Ok(())
    }

    fn key(
        &mut self,
        running: &Running,
        keysym: u32,
        code: u32,
        modifiers: u32,
        pressed: bool,
        time_ms: u32,
    ) -> Result<(bool, Change), Stop> {
        let mut during = Change::default();
        let reply = self.call_collecting(
            running,
            running.context.as_str(),
            INPUT_CONTEXT,
            "ProcessKeyEventBatch",
            &(keysym, code + 8, modifiers, !pressed, time_ms),
            &mut during,
        )?;
        let (events, used): (Vec<(u32, OwnedValue)>, bool) =
            reply.body().deserialize().map_err(failed)?;
        let mut forwarded = false;
        for (kind, value) in events {
            if kind == 2 {
                forwarded = true;
            }
            if let Some(change) = batched(kind, value) {
                during.add(change);
            }
        }
        // A key the engine forwarded is the key given back.
        Ok((used && !forwarded, during))
    }

    /// Calls the input context, gathering what its signals did meanwhile as one change.
    fn collecting<B>(&mut self, running: &Running, method: &str, body: &B) -> Result<Change, Stop>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let mut during = Change::default();
        self.call_collecting(
            running,
            running.context.as_str(),
            INPUT_CONTEXT,
            method,
            body,
            &mut during,
        )?;
        Ok(during)
    }

    fn context_call<B>(
        &mut self,
        running: &Running,
        method: &str,
        body: &B,
    ) -> Result<Message, Stop>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.call(
            running,
            running.context.as_str(),
            INPUT_CONTEXT,
            method,
            body,
        )
    }

    fn call<B>(
        &mut self,
        running: &Running,
        path: &str,
        interface: &str,
        method: &str,
        body: &B,
    ) -> Result<Message, Stop>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let mut during = Change::default();
        let reply = self.call_collecting(running, path, interface, method, body, &mut during)?;
        if !during.is_empty() {
            self.tell(Outcome::Change(during));
        }
        Ok(reply)
    }

    /// Sends a call and waits for its reply in the queue: signals before it are handled (their
    /// changes gathered in `during`), commands kept for after.
    fn call_collecting<B>(
        &mut self,
        running: &Running,
        path: &str,
        interface: &str,
        method: &str,
        body: &B,
        during: &mut Change,
    ) -> Result<Message, Stop>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let message = Message::method_call(path, method)
            .and_then(|builder| builder.destination(SERVICE))
            .and_then(|builder| builder.interface(interface))
            .and_then(|builder| builder.build(body))
            .map_err(failed)?;
        let serial: NonZeroU32 = message.primary_header().serial_num();
        running.connection.send(&message).map_err(failed)?;
        let deadline = Instant::now() + CALL_LIMIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let input = match self.inputs.recv_timeout(left) {
                Ok(input) => input,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(failed(format!("fcitx5 did not answer {method}")).into())
                }
                Err(RecvTimeoutError::Disconnected) => return Err(Stop::Shutdown),
            };
            match input {
                Input::Command(command) => self.deferred.push_back(command),
                Input::Shutdown => return Err(Stop::Shutdown),
                Input::Closed => return Err(failed("the input methods' bus closed").into()),
                Input::Message(reply) => {
                    let header = reply.header();
                    if header.reply_serial() != Some(serial) {
                        self.signal(running, &reply, during);
                        continue;
                    }
                    return match reply.message_type() {
                        Type::Error => Err(failed(format!(
                            "fcitx5 refused {method}: {}",
                            header
                                .error_name()
                                .map(|name| name.as_str())
                                .unwrap_or("an error")
                        ))
                        .into()),
                        _ => Ok(reply.clone()),
                    };
                }
            }
        }
    }

    /// A signal of the input context: what the engine did on its own, gathered into `change`,
    /// and the candidates told at once.
    fn signal(&mut self, running: &Running, message: &Message, change: &mut Change) {
        if message.message_type() != Type::Signal {
            return;
        }
        let header = message.header();
        if header.path().map(|path| path.as_str()) != Some(running.context.as_str()) {
            return;
        }
        let Some(member) = header.member().map(|member| member.as_str().to_owned()) else {
            return;
        };
        let body = message.body();
        match member.as_str() {
            "CommitString" => {
                if let Ok(text) = body.deserialize::<String>() {
                    change.commit.push_str(&text);
                }
            }
            "UpdateFormattedPreedit" => {
                if let Ok((parts, cursor)) = body.deserialize::<(Vec<(String, i32)>, i32)>() {
                    change.preedit = Some(preedit_of(parts, cursor));
                }
            }
            "DeleteSurroundingText" => {
                if let Ok((offset, size)) = body.deserialize::<(i32, u32)>() {
                    change.delete = Some((offset, size));
                }
            }
            "UpdateClientSideUI" => {
                type Ui = (
                    Vec<(String, i32)>,
                    i32,
                    Vec<(String, i32)>,
                    Vec<(String, i32)>,
                    Vec<(String, String)>,
                    i32,
                    i32,
                    bool,
                    bool,
                );
                if let Ok((_, _, up, down, candidates, index, layout, previous, next)) =
                    body.deserialize::<Ui>()
                {
                    let count = candidates.len();
                    self.tell(Outcome::Panel(Panel {
                        candidates: candidates.into_iter().map(|(_, text)| text).collect(),
                        highlighted: usize::try_from(index).ok().filter(|index| *index < count),
                        aux_up: joined(&up),
                        aux_down: joined(&down),
                        previous,
                        next,
                        // fcitx5's CandidateLayoutHint: 1 vertical, 2 horizontal.
                        vertical: layout == 1,
                    }));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_gather_in_order_the_latest_composition_winning() {
        let mut change = Change::default();
        change.add(Change {
            commit: "你".to_owned(),
            ..Change::default()
        });
        change.add(Change {
            commit: "好".to_owned(),
            preedit: Some(Preedit::default()),
            ..Change::default()
        });
        assert_eq!(change.commit, "你好");
        assert_eq!(change.preedit, Some(Preedit::default()));
        assert!(Change::default().is_empty());
    }

    #[test]
    fn a_highlighted_part_is_being_converted() {
        assert!(PreeditFormat(PreeditFormat::HIGHLIGHT | PreeditFormat::UNDERLINE).converting());
        assert!(!PreeditFormat(PreeditFormat::UNDERLINE).converting());
    }
}
