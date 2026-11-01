//! Wi-Fi and networks for the program running the compositor (docs/adr/0038-networks.mdx): the
//! session's side of the shell's and Settings' requests, over NetworkManager.
//!
//! The shell (its `shell` grant) and a client with `network` ask in the session's vocabulary
//! (`SystemRequest`, one JSON object read strictly, [`Ask`]); what changes the machine must cite an
//! input the client received. Each call to the daemon runs on a thread of its own with a connection
//! of its own, so the compositor's turn never waits for it. One thread follows the daemon and the
//! network's state is told to every client holding `network` or `shell` as it changes.
//!
//! Joining a network that needs a password asks for it in a system sheet the shell alone is sent
//! (`wifi-password`); the shell answers with the password (`{"password":…}`, citing the input that
//! submitted it), which the session takes only from the shell, only for a password sheet it is
//! showing and never while locked, and hands to the daemon. A wrong one asks again, saying so;
//! cancelling ends the join. Whoever asked to join learns only whether it joined.

use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::{ClientId, Scene};
use oxyos_networks::{Failure, Plan, Secret, State};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// What the session asks of the network's daemon: NetworkManager's ([`Bus`]), or a played one.
pub trait Daemon {
    fn state(&self) -> io::Result<State>;
    fn scan(&self) -> io::Result<()>;
    fn set_wifi(&self, enabled: bool) -> Result<(), Failure>;
    fn forget(&self, id: &str) -> Result<(), Failure>;
    fn plan(&self, ssid: &str) -> Result<Plan, Failure>;
    /// Joins and waits until it is joined or given up.
    fn join(&self, ssid: &str, password: Option<&Secret>) -> Result<(), Failure>;
}

/// How the session reaches the daemon: a connection for each call, and the daemon followed.
pub trait Reach: Send + Sync {
    fn connect(&self) -> io::Result<Box<dyn Daemon>>;
    /// Calls `changed` once, then after each change of the daemon's, until it returns false,
    /// `stop` returns true (looked at often), or the daemon cannot be followed.
    fn watch(&self, stop: &dyn Fn() -> bool, changed: &mut dyn FnMut() -> bool) -> io::Result<()>;
}

impl Daemon for oxyos_networks::NetworkManager {
    fn state(&self) -> io::Result<State> {
        oxyos_networks::NetworkManager::state(self)
    }
    fn scan(&self) -> io::Result<()> {
        oxyos_networks::NetworkManager::scan(self)
    }
    fn set_wifi(&self, enabled: bool) -> Result<(), Failure> {
        oxyos_networks::NetworkManager::set_wifi(self, enabled)
    }
    fn forget(&self, id: &str) -> Result<(), Failure> {
        oxyos_networks::NetworkManager::forget(self, id)
    }
    fn plan(&self, ssid: &str) -> Result<Plan, Failure> {
        oxyos_networks::NetworkManager::plan(self, ssid)
    }
    fn join(&self, ssid: &str, password: Option<&Secret>) -> Result<(), Failure> {
        oxyos_networks::NetworkManager::join(self, ssid, password)
    }
}

/// NetworkManager on the system bus, or on the bus at `address` (the development compositor's
/// played daemon).
pub struct Bus {
    pub address: Option<String>,
}

impl Reach for Bus {
    fn connect(&self) -> io::Result<Box<dyn Daemon>> {
        Ok(Box::new(oxyos_networks::NetworkManager::connect(
            self.address.as_deref(),
        )?))
    }

    fn watch(&self, stop: &dyn Fn() -> bool, changed: &mut dyn FnMut() -> bool) -> io::Result<()> {
        oxyos_networks::watch(
            self.address.as_deref(),
            Duration::from_millis(100),
            stop,
            changed,
        )
    }
}

/// What a client asks, read strictly.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Ask {
    /// The network's state.
    Networks {},
    /// Look for networks.
    Scan {},
    /// With an input: join a network by its name.
    Join { ssid: String },
    /// With an input: forget a saved network by its id.
    Forget { id: String },
    /// With an input: Wi-Fi on or off.
    Wifi { enabled: bool },
    /// The shell alone, with an input: the password typed into a password sheet.
    Password { sheet: u32, password: String },
}

/// The vocabulary's names: a request naming one of them is this part's.
const ASKS: [&str; 6] = ["networks", "scan", "join", "forget", "wifi", "password"];

fn ours(text: &str) -> bool {
    serde_json::from_str::<HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.keys().all(|key| ASKS.contains(&key.as_str())))
}

/// What a thread did, for the compositor's turn.
enum Done {
    Answer {
        client: ClientId,
        serial: u32,
        text: String,
    },
    State(Box<State>),
    /// A join needs the person's password: a sheet, and where its answer goes.
    AskPassword {
        client: ClientId,
        ssid: String,
        security: &'static str,
        incorrect: bool,
        reply: mpsc::Sender<Option<Secret>>,
    },
    /// A sheet that only tells (a network not supported, a join that failed).
    Note(String),
    Joining(Option<String>),
}

#[derive(Clone)]
struct Back {
    done: mpsc::Sender<Done>,
    wake: Arc<UnixStream>,
}

impl Back {
    fn send(&self, done: Done) {
        if self.done.send(done).is_ok() {
            let _ = (&*self.wake).write(&[1]);
        }
    }
}

enum Sheet {
    Password(mpsc::Sender<Option<Secret>>),
    Note,
}

pub struct Networks {
    reach: Arc<dyn Reach>,
    back: Back,
    done: mpsc::Receiver<Done>,
    woken: UnixStream,
    /// The state as last read, and as last told (its JSON).
    state: Option<State>,
    told: Option<String>,
    joining: Option<String>,
    /// A join's password is being asked: it is not told joining meanwhile (nothing connects until
    /// the person answers), though another join still waits for it.
    asking: bool,
    sheets: BTreeMap<u32, Sheet>,
    watching: bool,
    changed: bool,
    stop: Arc<AtomicBool>,
    /// The threads running, waited for when it goes, so their connections to the daemon go too.
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Networks {
    pub fn new(reach: Arc<dyn Reach>) -> io::Result<Self> {
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let (done, received) = mpsc::channel();
        Ok(Self {
            reach,
            back: Back {
                done,
                wake: Arc::new(wake),
            },
            done: received,
            woken,
            state: None,
            told: None,
            joining: None,
            asking: false,
            sheets: BTreeMap::new(),
            watching: false,
            changed: false,
            stop: Arc::new(AtomicBool::new(false)),
            threads: Vec::new(),
        })
    }

    /// NetworkManager on the system bus, or at `address`.
    pub fn on_bus(address: Option<String>) -> io::Result<Self> {
        Self::new(Arc::new(Bus { address }))
    }

    /// What wakes the compositor's turn when a thread is done (`Compositor::wakers`).
    pub fn wake_fd(&self) -> io::Result<OwnedFd> {
        Ok(self.woken.try_clone()?.into())
    }

    /// Whether the network's state changed since the last call: the hardware controls look again
    /// (the top bar's Wi-Fi and Ethernet).
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// The state as last read.
    pub fn state(&self) -> Option<&State> {
        self.state.as_ref()
    }

    fn spawn(&mut self, work: impl FnOnce(&dyn Daemon, &Back) + Send + 'static) {
        let (reach, back) = (self.reach.clone(), self.back.clone());
        if let Ok(thread) = std::thread::Builder::new()
            .name("networks".into())
            .spawn(move || match reach.connect() {
                Ok(daemon) => work(daemon.as_ref(), &back),
                Err(error) => {
                    eprintln!("[networks] the daemon: {error}");
                    work(&Absent, &back)
                }
            })
        {
            self.threads.push(thread);
        }
    }

    /// A turn: the scene's requests that are this part's, the sheets answered, and what the
    /// threads did.
    pub fn serve(&mut self, scene: &mut Scene) {
        let mut buffer = [0u8; 64];
        while matches!((&self.woken).read(&mut buffer), Ok(read) if read > 0) {}
        self.threads.retain(|thread| !thread.is_finished());
        if !self.watching {
            self.watching = true;
            let (reach, back, stop) = (self.reach.clone(), self.back.clone(), self.stop.clone());
            if let Ok(thread) = std::thread::Builder::new()
                .name("networks-watch".into())
                .spawn(move || watch(reach.as_ref(), &back, &stop))
            {
                self.threads.push(thread);
            }
        }
        for request in scene.take_system_requests_where(|request| ours(&request.text)) {
            self.handle(scene, request);
        }
        // Its sheets' answers: a password sheet refused is a join cancelled; a note is read.
        let sheets: Vec<u32> = self.sheets.keys().copied().collect();
        for sheet in sheets {
            // A password sheet is answered with its password, never accepted bare.
            if scene.take_sheet_answer(sheet).is_some() {
                if let Some(Sheet::Password(reply)) = self.sheets.remove(&sheet) {
                    let _ = reply.send(None);
                    self.asking = false;
                }
            }
        }
        for done in self.done.try_iter().collect::<Vec<_>>() {
            match done {
                Done::Answer {
                    client,
                    serial,
                    text,
                } => scene.system_text(client, serial, &text),
                Done::State(state) => {
                    self.state = Some(*state);
                    self.tell(scene);
                }
                Done::Joining(joining) => {
                    self.joining = joining;
                    self.asking = false;
                    self.tell(scene);
                }
                Done::AskPassword {
                    client,
                    ssid,
                    security,
                    incorrect,
                    reply,
                } => {
                    let text = json!({
                        "kind": "wifi-password", "ssid": ssid, "security": security,
                        "incorrect": incorrect,
                    })
                    .to_string();
                    match scene.show_sheet(Some(client), &text) {
                        Some(sheet) => {
                            self.sheets.insert(sheet, Sheet::Password(reply));
                            self.asking = true;
                            self.tell(scene);
                        }
                        None => {
                            let _ = reply.send(None);
                        }
                    }
                }
                Done::Note(text) => {
                    if let Some(sheet) = scene.show_sheet(None, &text) {
                        self.sheets.insert(sheet, Sheet::Note);
                    }
                }
            }
        }
    }

    /// One request of this part's vocabulary: read strictly, checked against the grant it needs
    /// and, if it changes anything, an input the client received.
    pub fn handle(&mut self, scene: &mut Scene, mut request: SystemRequest) {
        let parsed = serde_json::from_str::<Ask>(&request.text);
        // A password's text goes with the request: cleared, it is held only as a Secret.
        oxyos_networks::clear(&mut request.text);
        let (client, serial) = (request.client, request.serial);
        let refuse = |scene: &mut Scene, reason: &str| {
            scene.system_text(client, serial, &refused_text(reason));
        };
        let Ok(ask) = parsed else {
            refuse(scene, "invalidRequest");
            return;
        };
        let Some(grants) = scene.grants_of(client) else {
            return;
        };
        let allowed = match ask {
            Ask::Password { .. } => grants.shell,
            _ => grants.network || grants.shell,
        };
        if !allowed {
            refuse(scene, "notPermitted");
            return;
        }
        let changes = !matches!(ask, Ask::Networks {} | Ask::Scan {});
        if changes && !request.with_input {
            refuse(scene, "needsInput");
            return;
        }
        self.ask(scene, client, serial, ask);
    }

    /// Tells every client that follows the network its state, if it changed.
    fn tell(&mut self, scene: &mut Scene) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        let mut state = state.clone();
        state.joining = self.told_joining();
        let text = json!({ "network": state }).to_string();
        if self.told.as_deref() == Some(text.as_str()) {
            return;
        }
        self.changed = true;
        for client in scene.network_watchers() {
            scene.system_text(client, 0, &text);
        }
        self.told = Some(text);
    }

    fn told_joining(&self) -> Option<String> {
        self.joining.clone().filter(|_| !self.asking)
    }

    fn current_text(&self) -> Option<String> {
        let mut state = self.state.clone()?;
        state.joining = self.told_joining();
        Some(json!({ "network": state }).to_string())
    }

    fn ask(&mut self, scene: &mut Scene, client: ClientId, serial: u32, ask: Ask) {
        let answer = move |back: &Back, text: String| {
            back.send(Done::Answer {
                client,
                serial,
                text,
            })
        };
        match ask {
            Ask::Networks {} => match self.current_text() {
                Some(text) => scene.system_text(client, serial, &text),
                None => self.spawn(move |daemon, back| {
                    let state = daemon.state().unwrap_or_else(|_| State::unavailable());
                    answer(back, json!({ "network": state }).to_string());
                    back.send(Done::State(Box::new(state)));
                }),
            },
            Ask::Scan {} => self.spawn(move |daemon, back| {
                let text = match daemon.scan() {
                    Ok(()) => json!({"scanned": {}}),
                    Err(error) => {
                        eprintln!("[networks] scanning: {error}");
                        failed(Failure::Failed)
                    }
                };
                answer(back, text.to_string());
            }),
            Ask::Wifi { enabled } => self.spawn(move |daemon, back| {
                let text = match daemon.set_wifi(enabled) {
                    Ok(()) => {
                        let state = daemon.state().unwrap_or_else(|_| State::unavailable());
                        back.send(Done::State(Box::new(state.clone())));
                        json!({ "network": state })
                    }
                    Err(failure) => failed(failure),
                };
                answer(back, text.to_string());
            }),
            Ask::Forget { id } => self.spawn(move |daemon, back| {
                let text = match daemon.forget(&id) {
                    Ok(()) => json!({"forgotten": {}}),
                    Err(failure) => failed(failure),
                };
                if let Ok(state) = daemon.state() {
                    back.send(Done::State(Box::new(state)));
                }
                answer(back, text.to_string());
            }),
            Ask::Join { ssid } => {
                if self.joining.is_some() {
                    scene.system_text(client, serial, &failed_text("busy"));
                    return;
                }
                self.joining = Some(ssid.clone());
                self.tell(scene);
                self.spawn(move |daemon, back| {
                    let text = match join(daemon, back, client, &ssid) {
                        Ok(()) => json!({"joined": {"ssid": ssid}}),
                        Err(failure) => failed(failure),
                    };
                    if let Ok(state) = daemon.state() {
                        back.send(Done::State(Box::new(state)));
                    }
                    back.send(Done::Joining(None));
                    answer(back, text.to_string());
                });
            }
            Ask::Password { sheet, password } => {
                let password = Secret::new(password);
                if scene.is_locked() {
                    scene.system_text(client, serial, &refused_text("locked"));
                    return;
                }
                match self.sheets.remove(&sheet) {
                    Some(Sheet::Password(reply)) => {
                        scene.dismiss_sheet(sheet);
                        let _ = reply.send(Some(password));
                        self.asking = false;
                        self.tell(scene);
                    }
                    other => {
                        if let Some(other) = other {
                            self.sheets.insert(sheet, other);
                        }
                        scene.system_text(client, serial, &refused_text("noSuchSheet"));
                    }
                }
            }
        }
    }
}

impl Drop for Networks {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // A join waiting for its password is cancelled.
        self.sheets.clear();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn failed(failure: Failure) -> Value {
    json!({"failed": {"reason": failure.reason()}})
}

fn failed_text(reason: &str) -> String {
    json!({"failed": {"reason": reason}}).to_string()
}

fn refused_text(reason: &str) -> String {
    json!({"refused": {"reason": reason}}).to_string()
}

/// No daemon to reach: everything is unavailable.
struct Absent;

impl Daemon for Absent {
    fn state(&self) -> io::Result<State> {
        Ok(State::unavailable())
    }
    fn scan(&self) -> io::Result<()> {
        Ok(())
    }
    fn set_wifi(&self, _: bool) -> Result<(), Failure> {
        Err(Failure::Unavailable)
    }
    fn forget(&self, _: &str) -> Result<(), Failure> {
        Err(Failure::Unavailable)
    }
    fn plan(&self, _: &str) -> Result<Plan, Failure> {
        Err(Failure::Unavailable)
    }
    fn join(&self, _: &str, _: Option<&Secret>) -> Result<(), Failure> {
        Err(Failure::Unavailable)
    }
}

/// Follows the daemon, telling its state as it changes, until the session ends; again after a
/// pause when the daemon cannot be followed (not started yet, or restarted).
fn watch(reach: &dyn Reach, back: &Back, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        let mut daemon: Option<Box<dyn Daemon>> = None;
        let stopped = || stop.load(Ordering::Relaxed);
        let result = reach.watch(&stopped, &mut || {
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            if daemon.is_none() {
                daemon = reach.connect().ok();
            }
            let state = daemon
                .as_ref()
                .and_then(|d| d.state().ok())
                .unwrap_or_else(State::unavailable);
            back.send(Done::State(Box::new(state)));
            true
        });
        if let Err(error) = result {
            eprintln!("[networks] following the daemon: {error}");
            back.send(Done::State(Box::new(State::unavailable())));
        }
        for _ in 0..20 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// Joins `ssid` for `client`: what it needs decided, the password asked in the shell's sheet
/// (again, saying so, while it is wrong), a failure or a network not supported said in a sheet.
fn join(daemon: &dyn Daemon, back: &Back, client: ClientId, ssid: &str) -> Result<(), Failure> {
    let note = |kind: &str, security: Option<&str>| {
        let mut sheet = json!({"kind": kind, "ssid": ssid});
        if let Some(security) = security {
            sheet["security"] = json!(security);
        }
        back.send(Done::Note(sheet.to_string()));
    };
    let ask = |security: &'static str, incorrect: bool| -> Option<Secret> {
        let (reply, answered) = mpsc::channel();
        back.send(Done::AskPassword {
            client,
            ssid: ssid.to_string(),
            security,
            incorrect,
            reply,
        });
        answered.recv().ok().flatten()
    };
    let (security, mut incorrect) = match daemon.plan(ssid)? {
        Plan::Unsupported { security } => {
            note("wifi-unsupported", Some(security.name()));
            return Err(Failure::Unsupported);
        }
        Plan::Open => (None, false),
        Plan::Known { security } => match daemon.join(ssid, None) {
            // The saved password no longer works: asked for the new one.
            Err(Failure::WrongPassword) if security.needs_password() => (Some(security), true),
            outcome => return said(outcome, &note),
        },
        Plan::Password { security } => (Some(security), false),
    };
    let Some(security) = security else {
        return said(daemon.join(ssid, None), &note);
    };
    loop {
        let password = ask(security.name(), incorrect).ok_or(Failure::Cancelled)?;
        if !password.valid_wpa() {
            incorrect = true;
            continue;
        }
        match daemon.join(ssid, Some(&password)) {
            Err(Failure::WrongPassword) => incorrect = true,
            outcome => return said(outcome, &note),
        }
    }
}

/// A join's outcome, a failure said in a sheet.
fn said(outcome: Result<(), Failure>, note: &dyn Fn(&str, Option<&str>)) -> Result<(), Failure> {
    if let Err(Failure::Failed | Failure::Timeout | Failure::NotPermitted) = outcome {
        note("wifi-failed", None);
    }
    outcome
}

#[cfg(test)]
mod tests;
