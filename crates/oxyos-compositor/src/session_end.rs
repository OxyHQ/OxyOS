//! Sleep, restart, shut down and log out for the program running the compositor
//! (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx), the session's one authority over them.
//!
//! - **What it takes.** The shell's requests of the session's vocabulary (`endSession`, `sleep`,
//!   `reopenWindows`, `powerKeyChoice`; the `shell` grant, citing the person's input), the scene's
//!   keys (the power key; Super+Shift+Q and Super+Alt+Shift+Q) and the answers to its sheets.
//! - **The question** is a system sheet with a 60-second deadline the session keeps: once it
//!   passes, the sheet is withdrawn and the end goes on as if accepted.
//! - **Every application is asked to quit** (`Scene::ask_to_quit`) and the end waits until each
//!   asked has answered `quit` and gone, or one cancels, at most [`QUIT_DEADLINE`]. One that
//!   cancels, or does not answer in time, is named to the person in a sheet, and nothing more ends.
//! - **logind**, on a worker of its own (`oxyos_power::ending`, `oxyos_power::sleep`): `Can…` first,
//!   anything but `yes` refused before an application is asked; then `Reboot`, `PowerOff` or
//!   `Suspend`, never interactive. The `handle-power-key` inhibitor is held for the part's life, so
//!   logind never turns the machine off at a press; what a press does follows `HandlePowerKey`.
//! - **Log out** ends the session ([`Served::log_out`]), as Settings' Sign Out.
//! - **Reopen windows when logging back in**: the applications running as the end was asked are
//!   written to the person's state folder, and the next session opens them ([`take_reopen`]).
//!
//! The login screen's (`greeter`) asks no question and has no application: its Sleep, Restart and
//! Shut Down act at once, and a press of the power key is told to it to ask itself.

use crate::settings::{self, Settings};
use crate::Compositor;
use oxyos_compositor_core::scene::{SessionKey, SystemRequest};
use oxyos_compositor_core::ClientId;
use oxyos_power::ending::{End, PowerKey};
use oxyos_power::Failure;
use oxyos_protocol::SessionEnd as Reason;
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// How long the question waits before the end goes on by itself, as a Mac's.
pub const CONFIRM_SECONDS: u64 = 60;
/// How long the applications have, from the asking, to answer and quit.
pub const QUIT_DEADLINE: Duration = Duration::from_secs(30);

/// What the person asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    LogOut,
    Restart,
    ShutDown,
}

impl Action {
    fn name(self) -> &'static str {
        match self {
            Action::LogOut => "logOut",
            Action::Restart => "restart",
            Action::ShutDown => "shutDown",
        }
    }

    fn reason(self) -> Reason {
        match self {
            Action::LogOut => Reason::LogOut,
            Action::Restart => Reason::Restart,
            Action::ShutDown => Reason::ShutDown,
        }
    }

    fn end(self) -> Option<End> {
        match self {
            Action::LogOut => None,
            Action::Restart => Some(End::Restart),
            Action::ShutDown => Some(End::ShutDown),
        }
    }
}

/// The requests, read strictly.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Ask {
    EndSession { action: AskedAction, confirm: bool },
    Sleep {},
    ReopenWindows { enabled: bool },
    PowerKeyChoice { sheet: u32, choice: Choice },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum AskedAction {
    LogOut,
    Restart,
    ShutDown,
}

impl From<AskedAction> for Action {
    fn from(asked: AskedAction) -> Self {
        match asked {
            AskedAction::LogOut => Action::LogOut,
            AskedAction::Restart => Action::Restart,
            AskedAction::ShutDown => Action::ShutDown,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Choice {
    Restart,
    Sleep,
}

const ASKS: [&str; 4] = ["endSession", "sleep", "reopenWindows", "powerKeyChoice"];

fn ours(text: &str) -> bool {
    serde_json::from_str::<std::collections::HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.keys().all(|key| ASKS.contains(&key.as_str())))
}

/// What the worker is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    /// Hold the power key's inhibitor.
    Hold,
    /// Let it go: the person logged out, and the session is no longer the seat's.
    Release,
    /// Whether the machine may end so.
    Can(End),
    /// End it so (asked again first).
    End(End),
    Sleep,
    /// What a press of the power key does now.
    PowerKey,
}

/// What it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Done {
    Can(End, Result<(), Failure>),
    Ended(End, Result<(), Failure>),
    Slept(Result<(), Failure>),
    PowerKey(PowerKey),
}

/// Where logind is: the system's, a played one's bus, or none (every call unavailable).
#[derive(Clone, Debug)]
pub enum Logind {
    System,
    Bus(String),
    None,
}

struct Calls {
    logind: Logind,
    bus: Option<oxyos_power::Bus>,
    /// The power key's inhibitor, held while the part is.
    held: Option<OwnedFd>,
}

impl Calls {
    fn bus(&mut self) -> Option<&oxyos_power::Bus> {
        if self.bus.is_none() {
            let address = match &self.logind {
                Logind::None => return None,
                Logind::System => None,
                Logind::Bus(address) => Some(address.as_str()),
            };
            match oxyos_power::connect(address) {
                Ok(bus) => self.bus = Some(bus),
                Err(error) => eprintln!("[session end] the system bus: {error}"),
            }
        }
        self.bus.as_ref()
    }

    fn run(&mut self, job: Job) -> Option<Done> {
        let unavailable = Err(Failure::Unavailable);
        match job {
            Job::Hold => {
                if self.held.is_none() {
                    match self.bus().map(oxyos_power::ending::hold_power_key) {
                        Some(Ok(fd)) => self.held = Some(fd),
                        Some(Err(error)) => eprintln!(
                            "[session end] the power key's inhibitor: {error}; logind handles it"
                        ),
                        None => {}
                    }
                }
                None
            }
            Job::Release => {
                self.held = None;
                None
            }
            Job::Can(end) => Some(Done::Can(
                end,
                self.bus()
                    .map_or(unavailable, |bus| oxyos_power::ending::can(bus, end)),
            )),
            Job::End(end) => Some(Done::Ended(
                end,
                self.bus()
                    .map_or(unavailable, |bus| oxyos_power::ending::end(bus, end)),
            )),
            Job::Sleep => Some(Done::Slept(
                self.bus().map_or(unavailable, oxyos_power::sleep::suspend),
            )),
            Job::PowerKey => Some(Done::PowerKey(
                // Unreadable, the safe choice: ask.
                self.bus()
                    .and_then(|bus| oxyos_power::ending::handle_power_key(bus).ok())
                    .map_or(PowerKey::Ask, |policy| PowerKey::of(&policy)),
            )),
        }
    }
}

/// The calls, in the turn (the development compositor's played logind, tests) or on a thread.
enum Worker {
    Inline(Calls, VecDeque<Done>),
    Thread {
        jobs: mpsc::Sender<Job>,
        done: mpsc::Receiver<Done>,
        woken: UnixStream,
    },
}

impl Worker {
    fn send(&mut self, job: Job) {
        match self {
            Worker::Inline(calls, done) => done.extend(calls.run(job)),
            Worker::Thread { jobs, .. } => {
                let _ = jobs.send(job);
            }
        }
    }

    fn take(&mut self) -> Vec<Done> {
        match self {
            Worker::Inline(_, done) => done.drain(..).collect(),
            Worker::Thread { done, woken, .. } => {
                let mut buffer = [0u8; 64];
                while matches!(woken.read(&mut buffer), Ok(read) if read > 0) {}
                done.try_iter().collect()
            }
        }
    }
}

/// A request waiting for logind's `Can…`.
#[derive(Clone, Copy, Debug)]
struct Checking {
    action: Action,
    confirm: bool,
    /// Whose request it answers (none: a key's, or the power key's sheet's).
    asker: Option<(ClientId, u32)>,
}

#[derive(Debug)]
enum State {
    Idle,
    /// logind is asked whether the machine may end so.
    Checking(Checking),
    /// The question is shown; it goes on by itself at `deadline`.
    Confirming {
        action: Action,
        sheet: u32,
        deadline: Duration,
    },
    /// The power key's choice is shown.
    PowerKeySheet {
        sheet: u32,
    },
    /// What a press of the power key does is asked of logind.
    PowerKeyPolicy,
    /// The applications are asked to quit.
    Quitting {
        action: Action,
        serial: u32,
        /// Asked and not yet answered.
        waiting: BTreeSet<ClientId>,
        /// Answered `quit` and not yet gone.
        going: BTreeSet<ClientId>,
        deadline: Duration,
        names: BTreeMap<ClientId, String>,
    },
    /// logind is asked to restart or shut down.
    Ending(Action),
    /// Logged out, or logind restarts or shuts the machine down: nothing more is taken.
    Ended,
}

/// What a turn did that the program carries out.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Served {
    /// The person logged out: the session ends.
    pub log_out: bool,
}

/// The part.
pub struct SessionEnding {
    greeter: bool,
    worker: Worker,
    state: State,
    /// Time the development compositor said passed (`pass`).
    passed: Duration,
    /// The person's state folder's reopen file (none: nothing is reopened).
    reopen_file: Option<PathBuf>,
    next_serial: u32,
    /// Whose request the sleep asked of logind answers (none: the power key's).
    sleep_asker: Option<(ClientId, u32)>,
}

/// The file the applications to reopen at the next login are kept in, from the environment.
pub fn reopen_path() -> Option<PathBuf> {
    let state = match std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        Some(state) => state,
        None => PathBuf::from(std::env::var_os("HOME")?)
            .join(".local")
            .join("state"),
    };
    Some(state.join("oxyos").join("reopen-at-login"))
}

/// The applications to open at this login, read once: the file is removed.
pub fn take_reopen(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(path);
    let mut ids: Vec<String> = Vec::new();
    for id in text.lines().map(str::trim).filter(|id| !id.is_empty()) {
        if !ids.iter().any(|kept| kept == id) {
            ids.push(id.to_owned());
        }
    }
    ids
}

fn write_reopen(path: &Path, ids: &[String]) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let mut kept: Vec<&str> = Vec::new();
    for id in ids {
        if !kept.contains(&id.as_str()) {
            kept.push(id);
        }
    }
    let mut text = kept.join("\n");
    text.push('\n');
    // Written whole: a session that ends half way leaves the last one, never a part.
    let partial = path.with_extension("new");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path)
}

impl SessionEnding {
    /// The part, logind's calls on a thread of their own (a session).
    pub fn new(
        logind: Logind,
        greeter: bool,
        reopen_file: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let wake = Arc::new(wake);
        let (jobs, taken) = mpsc::channel::<Job>();
        let (done, received) = mpsc::channel::<Done>();
        std::thread::Builder::new()
            .name("session-end".into())
            .spawn(move || {
                let mut calls = Calls {
                    logind,
                    bus: None,
                    held: None,
                };
                // Waits on its jobs alone; it ends, the inhibitor let go, when the part goes.
                while let Ok(job) = taken.recv() {
                    if let Some(answer) = calls.run(job) {
                        if done.send(answer).is_ok() {
                            let _ = (&*wake).write(&[1]);
                        }
                    }
                }
            })?;
        Ok(Self::with_worker(
            Worker::Thread {
                jobs,
                done: received,
                woken,
            },
            greeter,
            reopen_file,
        ))
    }

    /// The part, logind's calls made in the turn: the development compositor's played logind,
    /// which answers at once, and tests.
    pub fn inline(logind: Logind, greeter: bool, reopen_file: Option<PathBuf>) -> Self {
        Self::with_worker(
            Worker::Inline(
                Calls {
                    logind,
                    bus: None,
                    held: None,
                },
                VecDeque::new(),
            ),
            greeter,
            reopen_file,
        )
    }

    fn with_worker(worker: Worker, greeter: bool, reopen_file: Option<PathBuf>) -> Self {
        let mut part = Self {
            greeter,
            worker,
            state: State::Idle,
            passed: Duration::ZERO,
            reopen_file: reopen_file.filter(|_| !greeter),
            next_serial: 0,
            sleep_asker: None,
        };
        // Held from the start: logind never turns the machine off at a press while OxyOS runs.
        part.worker.send(Job::Hold);
        part
    }

    /// What wakes the compositor's turn when the worker is done (`Compositor::wakers`), for a
    /// part whose calls run on a thread.
    pub fn wake_fd(&self) -> Option<std::io::Result<OwnedFd>> {
        match &self.worker {
            Worker::Thread { woken, .. } => Some(woken.try_clone().map(OwnedFd::from)),
            Worker::Inline(..) => None,
        }
    }

    /// Ends the part now: the power key's inhibitor let go and logind's connection closed (the
    /// development compositor checks it leaves no descriptor). A part on a thread ends as it is
    /// dropped, its thread letting both go.
    pub fn end(&mut self) {
        if let Worker::Inline(calls, done) = &mut self.worker {
            calls.held = None;
            done.clear();
            if let Some(bus) = calls.bus.take() {
                let _ = bus.close();
            }
        }
    }

    /// Time the development compositor says passed with no input (an `idle` step).
    pub fn pass(&mut self, time: Duration) {
        self.passed += time;
    }

    fn now(&self) -> Duration {
        crate::monotonic() + self.passed
    }

    /// How long the turn may wait before a deadline of its own is due.
    pub fn wait(&self) -> Option<Duration> {
        let deadline = match self.state {
            State::Confirming { deadline, .. } | State::Quitting { deadline, .. } => deadline,
            _ => return None,
        };
        Some(deadline.saturating_sub(self.now()))
    }

    /// A short name of the state, for the development compositor's results.
    pub fn state_name(&self) -> &'static str {
        match self.state {
            State::Idle => "idle",
            State::Checking(_) | State::PowerKeyPolicy => "checking",
            State::Confirming { .. } => "confirming",
            State::PowerKeySheet { .. } => "powerKey",
            State::Quitting { .. } => "quitting",
            State::Ending(_) => "ending",
            State::Ended => "ended",
        }
    }

    /// A turn: the requests, keys and sheet answers that are this part's, the worker's answers,
    /// the applications' and the deadlines. `name_of` names a client's application (its launch
    /// record's registry name, else its id); `running` are the registry ids of the applications
    /// running, in the order they were launched.
    pub fn serve(
        &mut self,
        compositor: &mut Compositor,
        settings: &mut Settings,
        name_of: &dyn Fn(ClientId) -> String,
        running: &[String],
    ) -> Served {
        let mut served = Served::default();
        for request in compositor
            .scene
            .take_system_requests_where(|request| ours(&request.text))
        {
            self.request(compositor, settings, request, name_of, running, &mut served);
        }
        for key in compositor.scene.take_session_keys() {
            self.key(compositor, settings, key, name_of, running, &mut served);
        }
        self.sheets(compositor, settings, name_of, running, &mut served);
        // What the worker did, and what that asked of it in turn (an inline worker's at once).
        loop {
            let answers = self.worker.take();
            if answers.is_empty() {
                break;
            }
            for done in answers {
                self.done(compositor, settings, done, name_of, running, &mut served);
            }
        }
        self.quitting(compositor, &mut served);
        served
    }

    fn answer(
        compositor: &mut Compositor,
        asker: Option<(ClientId, u32)>,
        text: serde_json::Value,
    ) {
        if let Some((client, serial)) = asker {
            compositor
                .scene
                .system_text(client, serial, &text.to_string());
        }
    }

    fn refused(compositor: &mut Compositor, asker: Option<(ClientId, u32)>, reason: &str) {
        Self::answer(
            compositor,
            asker,
            json!({ "refused": { "reason": reason } }),
        );
    }

    fn busy(&self) -> bool {
        matches!(
            self.state,
            State::Checking(_)
                | State::PowerKeyPolicy
                | State::Quitting { .. }
                | State::Ending(_)
                | State::Ended
        )
    }

    fn reopen(settings: &Settings) -> bool {
        settings.preferences().reopen_windows
    }

    #[allow(clippy::too_many_arguments)]
    fn request(
        &mut self,
        compositor: &mut Compositor,
        settings: &mut Settings,
        request: SystemRequest,
        name_of: &dyn Fn(ClientId) -> String,
        running: &[String],
        served: &mut Served,
    ) {
        let asker = Some((request.client, request.serial));
        let Ok(ask) = serde_json::from_str::<Ask>(&request.text) else {
            return Self::refused(compositor, asker, "invalidRequest");
        };
        let shell = compositor
            .scene
            .grants_of(request.client)
            .is_some_and(|grants| grants.shell);
        if !shell {
            return Self::refused(compositor, asker, "notPermitted");
        }
        if !request.with_input {
            return Self::refused(compositor, asker, "needsInput");
        }
        // The lock screen has none of these; the login screen is locked, and its own.
        if compositor.scene.is_locked() && !self.greeter {
            return Self::refused(compositor, asker, "locked");
        }
        match ask {
            Ask::EndSession { action, confirm } => {
                let action = Action::from(action);
                self.end_session(
                    compositor, settings, action, confirm, asker, name_of, running, served,
                );
            }
            Ask::Sleep {} => {
                if self.busy() {
                    return Self::refused(compositor, asker, "busy");
                }
                self.withdraw_question(compositor);
                self.sleep(asker);
            }
            Ask::ReopenWindows { enabled } => {
                if self.greeter {
                    return Self::refused(compositor, asker, "notPermitted");
                }
                let value = if enabled { "true" } else { "false" };
                if let Err(error) =
                    settings.set_preference(compositor, "reopen_windows", value, settings::now())
                {
                    eprintln!("[session end] reopen windows not kept: {error}");
                    return Self::refused(compositor, asker, "failed");
                }
                Self::answer(
                    compositor,
                    asker,
                    json!({ "reopenWindows": { "enabled": enabled } }),
                );
            }
            Ask::PowerKeyChoice { sheet, choice } => {
                if !matches!(self.state, State::PowerKeySheet { sheet: shown } if shown == sheet) {
                    return Self::refused(compositor, asker, "invalidRequest");
                }
                compositor.scene.dismiss_sheet(sheet);
                self.state = State::Idle;
                match choice {
                    Choice::Restart => self.end_session(
                        compositor,
                        settings,
                        Action::Restart,
                        false,
                        asker,
                        name_of,
                        running,
                        served,
                    ),
                    Choice::Sleep => self.sleep(asker),
                }
            }
        }
    }

    /// The question or the power key's sheet withdrawn: a new choice replaces it.
    fn withdraw_question(&mut self, compositor: &mut Compositor) {
        if let State::Confirming { sheet, .. } | State::PowerKeySheet { sheet } = self.state {
            compositor.scene.dismiss_sheet(sheet);
            self.state = State::Idle;
        }
    }

    fn sleep(&mut self, asker: Option<(ClientId, u32)>) {
        self.sleep_asker = asker;
        self.worker.send(Job::Sleep);
    }

    #[allow(clippy::too_many_arguments)]
    fn end_session(
        &mut self,
        compositor: &mut Compositor,
        settings: &mut Settings,
        action: Action,
        confirm: bool,
        asker: Option<(ClientId, u32)>,
        name_of: &dyn Fn(ClientId) -> String,
        running: &[String],
        served: &mut Served,
    ) {
        if self.busy() {
            return Self::refused(compositor, asker, "busy");
        }
        if self.greeter && action == Action::LogOut {
            return Self::refused(compositor, asker, "notPermitted");
        }
        self.withdraw_question(compositor);
        // Nobody is signed in at the login screen: nothing to confirm.
        let checking = Checking {
            action,
            confirm: confirm && !self.greeter,
            asker,
        };
        match action.end() {
            Some(end) => {
                self.state = State::Checking(checking);
                self.worker.send(Job::Can(end));
                // An inline worker answered already.
                for done in self.worker.take() {
                    self.done(compositor, settings, done, name_of, running, served);
                }
            }
            None => self.checked(compositor, settings, checking, name_of, running),
        }
    }

    /// logind said the machine may end so (or it is a log out): the question, or the round.
    fn checked(
        &mut self,
        compositor: &mut Compositor,
        settings: &Settings,
        checking: Checking,
        name_of: &dyn Fn(ClientId) -> String,
        running: &[String],
    ) {
        let Checking {
            action,
            confirm,
            asker,
        } = checking;
        if confirm {
            let text = json!({
                "kind": "end-session",
                "action": action.name(),
                "seconds": CONFIRM_SECONDS,
                "reopenWindows": Self::reopen(settings),
            });
            match compositor.scene.show_sheet(None, &text.to_string()) {
                Some(sheet) => {
                    self.state = State::Confirming {
                        action,
                        sheet,
                        deadline: self.now() + Duration::from_secs(CONFIRM_SECONDS),
                    };
                    Self::answer(compositor, asker, json!({ "confirming": {} }));
                }
                None => {
                    self.state = State::Idle;
                    Self::refused(compositor, asker, "busy");
                }
            }
            return;
        }
        Self::answer(
            compositor,
            asker,
            json!({ "ending": { "action": action.name() } }),
        );
        self.ask_applications(compositor, settings, action, name_of, running);
    }

    /// Every application asked to quit; the reopen file written first.
    fn ask_applications(
        &mut self,
        compositor: &mut Compositor,
        settings: &Settings,
        action: Action,
        name_of: &dyn Fn(ClientId) -> String,
        running: &[String],
    ) {
        if let Some(path) = self.reopen_file.as_ref() {
            let wanted = Self::reopen(settings) && !running.is_empty();
            let result = if wanted {
                write_reopen(path, running)
            } else {
                match std::fs::remove_file(path) {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                    _ => Ok(()),
                }
            };
            if let Err(error) = result {
                eprintln!("[session end] {}: {error}", path.display());
            }
        }
        self.next_serial = self.next_serial.wrapping_add(1).max(1);
        let serial = self.next_serial;
        let mut waiting = BTreeSet::new();
        let mut names = BTreeMap::new();
        for client in compositor.scene.applications() {
            names.insert(client, name_of(client));
            if compositor
                .scene
                .ask_to_quit(client, serial, action.reason())
            {
                waiting.insert(client);
            }
        }
        eprintln!(
            "[session end] {}: {} applications asked to quit",
            action.name(),
            names.len()
        );
        self.state = State::Quitting {
            action,
            serial,
            waiting,
            going: BTreeSet::new(),
            deadline: self.now() + QUIT_DEADLINE,
            names,
        };
    }

    #[allow(clippy::too_many_arguments)]
    fn key(
        &mut self,
        compositor: &mut Compositor,
        settings: &mut Settings,
        key: SessionKey,
        name_of: &dyn Fn(ClientId) -> String,
        running: &[String],
        served: &mut Served,
    ) {
        match key {
            SessionKey::LogOut { confirm } => {
                if !self.greeter && !compositor.scene.is_locked() {
                    self.end_session(
                        compositor,
                        settings,
                        Action::LogOut,
                        confirm,
                        None,
                        name_of,
                        running,
                        served,
                    );
                }
            }
            SessionKey::Power => {
                // A press while an end is questioned or under way does nothing more.
                if matches!(self.state, State::Idle) {
                    self.state = State::PowerKeyPolicy;
                    self.worker.send(Job::PowerKey);
                }
            }
        }
    }

    fn sheets(
        &mut self,
        compositor: &mut Compositor,
        settings: &Settings,
        name_of: &dyn Fn(ClientId) -> String,
        running: &[String],
        _served: &mut Served,
    ) {
        match self.state {
            State::Confirming {
                action,
                sheet,
                deadline,
            } => {
                let answer = compositor.scene.take_sheet_answer(sheet);
                if answer == Some(false) {
                    self.state = State::Idle;
                    return;
                }
                // Accepted, or left until its deadline: the session goes on by itself.
                let due = self.now() >= deadline;
                if answer == Some(true) || due {
                    if due {
                        compositor.scene.dismiss_sheet(sheet);
                    }
                    self.ask_applications(compositor, settings, action, name_of, running);
                }
            }
            State::PowerKeySheet { sheet } => match compositor.scene.take_sheet_answer(sheet) {
                Some(true) => {
                    // Shut Down: the sheet was the question.
                    self.state = State::Checking(Checking {
                        action: Action::ShutDown,
                        confirm: false,
                        asker: None,
                    });
                    self.worker.send(Job::Can(End::ShutDown));
                }
                Some(false) => self.state = State::Idle,
                None => {}
            },
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn done(
        &mut self,
        compositor: &mut Compositor,
        settings: &mut Settings,
        done: Done,
        name_of: &dyn Fn(ClientId) -> String,
        running: &[String],
        served: &mut Served,
    ) {
        match done {
            Done::Can(end, result) => {
                let State::Checking(checking) = self.state else {
                    return;
                };
                if checking.action.end() != Some(end) {
                    return;
                }
                match result {
                    Ok(()) => self.checked(compositor, settings, checking, name_of, running),
                    Err(failure) => {
                        self.state = State::Idle;
                        eprintln!(
                            "[session end] {} refused: {}",
                            checking.action.name(),
                            failure.name()
                        );
                        Self::refused(compositor, checking.asker, failure.name());
                        self.failed(compositor, checking.action.name(), failure);
                    }
                }
            }
            Done::Ended(..) => self.finish(compositor, done),
            Done::Slept(result) => {
                let asker = self.sleep_asker.take();
                match result {
                    Ok(()) => Self::answer(compositor, asker, json!({ "sleeping": {} })),
                    Err(failure) => {
                        eprintln!("[session end] sleep refused: {}", failure.name());
                        Self::refused(compositor, asker, failure.name());
                        self.failed(compositor, "sleep", failure);
                    }
                }
            }
            Done::PowerKey(policy) => {
                if !matches!(self.state, State::PowerKeyPolicy) {
                    return;
                }
                self.state = State::Idle;
                let locked = compositor.scene.is_locked() && !self.greeter;
                match policy {
                    PowerKey::Ignore => {}
                    PowerKey::Lock => {
                        if !self.greeter {
                            compositor.scene.lock();
                        }
                    }
                    // Locked, the lock is already on screen: the machine sleeps, as a locked Mac.
                    PowerKey::Sleep => self.sleep(None),
                    PowerKey::Ask if locked => self.sleep(None),
                    PowerKey::Ask if self.greeter => {
                        for shell in compositor.scene.shells() {
                            compositor.scene.system_text(
                                shell,
                                0,
                                &json!({ "powerKey": {} }).to_string(),
                            );
                        }
                    }
                    PowerKey::Ask => {
                        let text = json!({
                            "kind": "power-key",
                            "reopenWindows": Self::reopen(settings),
                        });
                        if let Some(sheet) = compositor.scene.show_sheet(None, &text.to_string()) {
                            self.state = State::PowerKeySheet { sheet };
                        }
                    }
                }
                let _ = served;
            }
        }
    }

    /// The person told what failed, in a sheet (the login screen shows its own refusal).
    fn failed(&self, compositor: &mut Compositor, action: &str, failure: Failure) {
        if self.greeter {
            return;
        }
        let text =
            json!({ "kind": "end-session-failed", "action": action, "reason": failure.name() });
        compositor.scene.show_sheet(None, &text.to_string());
    }

    /// The round: answers taken, the gone counted, and the end going on, cancelled or timed out.
    fn quitting(&mut self, compositor: &mut Compositor, served: &mut Served) {
        let State::Quitting {
            action,
            serial,
            ref mut waiting,
            ref mut going,
            deadline,
            ref names,
        } = self.state
        else {
            // Answers to an end that was cancelled are nobody's.
            compositor.scene.take_quit_replies();
            return;
        };
        let mut cancelled = None;
        for reply in compositor.scene.take_quit_replies() {
            if reply.serial != serial || !waiting.remove(&reply.client) {
                continue;
            }
            if reply.quit {
                going.insert(reply.client);
            } else if cancelled.is_none() {
                cancelled = Some(reply.client);
            }
        }
        // Gone: its connection closed (a cancel it gave before it went was taken above).
        waiting.retain(|client| compositor.scene.connected(*client));
        going.retain(|client| compositor.scene.connected(*client));
        let name = |client: ClientId| names.get(&client).cloned().unwrap_or_default();
        let outcome = if let Some(client) = cancelled {
            Some(("end-session-cancelled", Some(name(client))))
        } else if waiting.is_empty() && going.is_empty() {
            None
        } else if self.passed + crate::monotonic() >= deadline {
            let late = waiting.iter().chain(going.iter()).next().copied();
            Some(("end-session-timed-out", late.map(name)))
        } else {
            return;
        };
        compositor.scene.forget_quit_requests();
        match outcome {
            Some((kind, application)) => {
                eprintln!(
                    "[session end] {}: {kind} ({})",
                    action.name(),
                    application.as_deref().unwrap_or("")
                );
                self.state = State::Idle;
                // Nothing reopens of an end that did not happen.
                if let Some(path) = self.reopen_file.as_ref() {
                    let _ = std::fs::remove_file(path);
                }
                let text = json!({
                    "kind": kind,
                    "action": action.name(),
                    "application": application.unwrap_or_default(),
                });
                compositor.scene.show_sheet(None, &text.to_string());
            }
            None => match action.end() {
                None => {
                    eprintln!("[session end] logging out");
                    self.state = State::Ended;
                    served.log_out = true;
                    // The session is no longer the seat's: logind sees its hold of the power key
                    // go as it ends, and the login screen takes it.
                    self.worker.send(Job::Release);
                }
                Some(end) => {
                    self.state = State::Ending(action);
                    self.worker.send(Job::End(end));
                    for done in self.worker.take() {
                        if let Done::Ended(..) = done {
                            self.finish(compositor, done);
                        }
                    }
                }
            },
        }
    }

    fn finish(&mut self, compositor: &mut Compositor, done: Done) {
        let Done::Ended(_, result) = done else {
            return;
        };
        let State::Ending(action) = self.state else {
            return;
        };
        match result {
            Ok(()) => {
                eprintln!(
                    "[session end] logind is asked to {} the machine",
                    action.name()
                );
                self.state = State::Ended;
            }
            Err(failure) => {
                eprintln!("[session end] {} failed: {}", action.name(), failure.name());
                self.state = State::Idle;
                self.failed(compositor, action.name(), failure);
            }
        }
    }
}

#[cfg(test)]
mod tests;
