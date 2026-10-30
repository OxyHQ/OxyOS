//! Installing applications for the session (docs/adr/0030-installing-applications.mdx,
//! decisions 8, 9 and 16): the program running the compositor's side of the store's, Settings'
//! and applications' requests, over the install service.
//!
//! A client with the `store`, `updates` or `privacy` grant asks in the session's vocabulary, one
//! JSON object naming what it asks ([`Ask`]), read strictly; each is checked against the grant it
//! needs and, if it changes anything, against an input the client received. Each runs on a thread
//! of its own with a connection of its own to the install service (a plan belongs to its
//! connection), so the compositor's turn never waits for the service, a download or the person.
//! What needs the person's confirmation (installing, removing, an update's new capabilities, a
//! capability asked at first use) is a system sheet the shell alone is sent; nothing proceeds
//! until the shell answers it citing an input it received, and no password is asked (the
//! platform's author's decision). The account's update state is watched on one connection and told
//! to every client with `store` or `updates` as it changes: Atlas's Updates and Settings' Software
//! Update show one state and never check on their own.

use oxyos_compositor_core::{ClientId, Scene};
use oxyos_install::ipc::{Answer, Request};
use oxyos_install::socket::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// How long the service may take to answer a request that fetches or compiles.
const LONG: Duration = Duration::from_secs(15 * 60);
/// How long a quick request may take.
const SHORT: Duration = Duration::from_secs(30);

/// What a client asks, by the grant it needs (`store`, `updates` or `privacy`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Ask {
    /// `store`: what the account has of each application.
    Status { ids: Vec<String> },
    /// `store`, with an input: install, after the person confirms in the sheet.
    Install { id: String },
    /// `store`, with an input: remove, and the account's data, after the person confirms.
    Uninstall { id: String },
    /// `store`, with an input: open an installed application.
    Open { id: String },
    /// `store` or `updates`: the account's update state.
    State {},
    /// `store` or `updates`, with an input: check the store now.
    Check {},
    /// `store` or `updates`, with an input: apply these updates; new capabilities are asked.
    Update { ids: Vec<String> },
    /// `store` or `updates`, with an input: apply every update; new capabilities are asked.
    UpdateAll {},
    /// `privacy`: the account's applications and their capabilities' decisions.
    Privacy {},
    /// `privacy`, with an input: a decision on a capability an application declares.
    Decide {
        id: String,
        capability: String,
        granted: bool,
    },
}

impl Ask {
    /// Whether `grants` may ask it.
    fn allowed(&self, grants: &oxyos_compositor_core::Grants) -> bool {
        match self {
            Ask::Status { .. } | Ask::Install { .. } | Ask::Uninstall { .. } | Ask::Open { .. } => {
                grants.store
            }
            Ask::State {} | Ask::Check {} | Ask::Update { .. } | Ask::UpdateAll {} => {
                grants.store || grants.updates
            }
            Ask::Privacy {} | Ask::Decide { .. } => grants.privacy,
        }
    }

    /// Whether it changes anything, so needs an input the client received.
    fn changes(&self) -> bool {
        !matches!(self, Ask::Status { .. } | Ask::State {} | Ask::Privacy {})
    }
}

/// What a client is, for a capability it asks for: what the program running the compositor
/// knows from its launch record (ADR 0030, decision 7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asker {
    /// Not a process the session launched: it is no application, and gets nothing.
    Unknown,
    /// A development entry: unverified, it keeps what development applications had before
    /// capabilities (everything it asks for), as the registry says of it.
    Development,
    /// A system application of the image's policy: what the image ships is trusted.
    System,
    /// An installed version: the account's decisions, by capability.
    Installed {
        id: String,
        decisions: BTreeMap<String, String>,
    },
}

/// The capabilities asked at first use, by their protocol numbers.
pub fn capability_name(capability: u8) -> Option<&'static str> {
    Some(match capability {
        1 => "notifications",
        2 => "files",
        3 => "camera",
        4 => "microphone",
        5 => "location",
        _ => return None,
    })
}

/// An installed application to open, asked for by the store: the program running the compositor
/// launches it from its registry and answers with [`Installs::opened`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Open {
    pub client: ClientId,
    pub serial: u32,
    pub id: String,
}

enum Done {
    Answer {
        client: ClientId,
        serial: u32,
        text: String,
    },
    Sheet {
        text: String,
        reply: mpsc::Sender<bool>,
    },
    State(String),
    Open(Open),
    Capability {
        client: ClientId,
        serial: u32,
        capability: u8,
        granted: bool,
    },
}

/// A thread's way back: what it did, and a byte to wake the compositor's turn.
#[derive(Clone)]
struct Back {
    done: mpsc::Sender<Done>,
    wake: std::sync::Arc<UnixStream>,
}

impl Back {
    fn send(&self, done: Done) {
        if self.done.send(done).is_ok() {
            let _ = (&*self.wake).write(&[1]);
        }
    }

    fn answer(&self, client: ClientId, serial: u32, text: Value) {
        self.send(Done::Answer {
            client,
            serial,
            text: text.to_string(),
        });
    }

    /// Shows a sheet and waits for the person's answer; false if it could not be shown.
    fn sheet(&self, text: Value) -> bool {
        let (reply, answer) = mpsc::channel();
        self.send(Done::Sheet {
            text: text.to_string(),
            reply,
        });
        answer.recv().unwrap_or(false)
    }
}

pub struct Installs {
    socket: PathBuf,
    /// Where the account's data directories are (its home).
    home: Option<PathBuf>,
    back: Back,
    done: mpsc::Receiver<Done>,
    woken: UnixStream,
    /// Sheets shown, by number: the flow waiting for each one's answer.
    sheets: BTreeMap<u32, mpsc::Sender<bool>>,
    /// Capabilities decided at first use by this session, by client: they hold for that process.
    decided: BTreeMap<(ClientId, u8), bool>,
    /// The update state last told, for a client that asks before it changes.
    state: Option<String>,
    watching: bool,
}

impl Installs {
    /// The session's side of the install service at `socket`, removing removed applications'
    /// data under `home`.
    pub fn new(socket: PathBuf, home: Option<PathBuf>) -> std::io::Result<Self> {
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let (done, received) = mpsc::channel();
        Ok(Self {
            socket,
            home,
            back: Back {
                done,
                wake: std::sync::Arc::new(wake),
            },
            done: received,
            woken,
            sheets: BTreeMap::new(),
            decided: BTreeMap::new(),
            state: None,
            watching: false,
        })
    }

    /// What wakes the compositor's turn when a thread is done (`Compositor::wakers`).
    pub fn wake_fd(&self) -> std::io::Result<OwnedFd> {
        Ok(self.woken.try_clone()?.into())
    }

    /// A turn: the scene's requests, sheets answered and capability requests, and what the
    /// threads did. `asker` says what a client is, for its capability requests. Returns the
    /// applications the store asked to open.
    pub fn serve(&mut self, scene: &mut Scene, asker: &dyn Fn(ClientId) -> Asker) -> Vec<Open> {
        let mut buffer = [0u8; 64];
        while matches!((&self.woken).read(&mut buffer), Ok(read) if read > 0) {}
        if !self.watching {
            self.watching = true;
            let (socket, back) = (self.socket.clone(), self.back.clone());
            let _ = std::thread::Builder::new()
                .name("installs-watch".into())
                .spawn(move || watch(socket, back));
        }
        for request in scene.take_system_requests() {
            let refuse = |scene: &mut Scene, reason: &str| {
                let text = json!({"refused": {"reason": reason}}).to_string();
                scene.system_text(request.client, request.serial, &text);
            };
            let Ok(ask) = serde_json::from_str::<Ask>(&request.text) else {
                refuse(scene, "invalidRequest");
                continue;
            };
            let Some(grants) = scene.grants_of(request.client) else {
                continue;
            };
            if !ask.allowed(&grants) {
                refuse(scene, "notPermitted");
                continue;
            }
            if ask.changes() && !request.with_input {
                refuse(scene, "needsInput");
                continue;
            }
            if let (Ask::State {}, Some(state)) = (&ask, &self.state) {
                scene.system_text(request.client, request.serial, state);
                continue;
            }
            let (socket, home, back) = (self.socket.clone(), self.home.clone(), self.back.clone());
            let (client, serial) = (request.client, request.serial);
            let _ = std::thread::Builder::new()
                .name("installs".into())
                .spawn(move || flow(&socket, home.as_deref(), &back, client, serial, ask));
        }
        for answered in scene.take_sheet_answers() {
            if let Some(reply) = self.sheets.remove(&answered.sheet) {
                let _ = reply.send(answered.accepted);
            }
        }
        for request in scene.take_capability_requests() {
            let (client, serial, capability) = (request.client, request.serial, request.capability);
            if let Some(&granted) = self.decided.get(&(client, capability)) {
                scene.capability_result(client, serial, granted);
                continue;
            }
            let name = capability_name(capability).expect("the scene checks the number");
            match asker(client) {
                Asker::Unknown => scene.capability_result(client, serial, false),
                Asker::Development | Asker::System => scene.capability_result(client, serial, true),
                Asker::Installed { id, decisions } => {
                    match decisions.get(name).map(String::as_str) {
                        Some("granted") => scene.capability_result(client, serial, true),
                        // Asked only once the person acted in the application, as Expo's own
                        // permission requests come from a press.
                        Some("undecided") if request.with_input => {
                            let (socket, back) = (self.socket.clone(), self.back.clone());
                            let _ = std::thread::Builder::new().name("installs".into()).spawn(
                                move || permission(&socket, &back, client, serial, capability, &id),
                            );
                        }
                        _ => scene.capability_result(client, serial, false),
                    }
                }
            }
        }
        let mut opens = Vec::new();
        for done in self.done.try_iter().collect::<Vec<_>>() {
            match done {
                Done::Answer {
                    client,
                    serial,
                    text,
                } => scene.system_text(client, serial, &text),
                Done::Sheet { text, reply } => match scene.show_sheet(&text) {
                    Some(sheet) => {
                        self.sheets.insert(sheet, reply);
                    }
                    None => {
                        let _ = reply.send(false);
                    }
                },
                Done::State(text) => {
                    for client in scene.update_watchers() {
                        scene.system_text(client, 0, &text);
                    }
                    self.state = Some(text);
                }
                Done::Open(open) => opens.push(open),
                Done::Capability {
                    client,
                    serial,
                    capability,
                    granted,
                } => {
                    self.decided.insert((client, capability), granted);
                    scene.capability_result(client, serial, granted);
                }
            }
        }
        opens
    }

    /// Answers the store's request to open an application: whether it was started.
    pub fn opened(&self, scene: &mut Scene, open: &Open, launched: bool) {
        let text = if launched {
            json!({"opened": {"id": open.id}})
        } else {
            json!({"failed": {"reason": "notInstalled"}})
        };
        scene.system_text(open.client, open.serial, &text.to_string());
    }

    /// A client went: what it decided at first use goes with its process.
    pub fn forget(&mut self, client: ClientId) {
        self.decided.retain(|(asker, _), _| *asker != client);
    }
}

fn connect(socket: &Path, timeout: Duration) -> std::io::Result<Connection> {
    let connection = Connection::connect(socket)?;
    connection.set_timeout(timeout)?;
    Ok(connection)
}

/// One request and its answer as JSON; the service's refusal is an answer like any other.
fn ask(connection: &Connection, request: &Request) -> std::io::Result<Answer> {
    connection.send(request)?;
    match connection.receive::<Answer>()? {
        Some(Some(answer)) => Ok(answer),
        Some(None) => Err(std::io::Error::other("an answer the session does not read")),
        None => Err(std::io::Error::other(
            "the install service closed the connection",
        )),
    }
}

fn to_json(answer: &Answer) -> Value {
    serde_json::to_value(answer).unwrap_or(Value::Null)
}

fn failed(error: impl std::fmt::Display) -> Value {
    json!({"failed": {"reason": "unavailable", "detail": error.to_string()}})
}

/// An application's name in its account's entry (`Name=`), else its id.
fn name_of(entry: &str, id: &str) -> String {
    std::fs::read_to_string(entry)
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("Name=").map(str::to_string))
        })
        .unwrap_or_else(|| id.to_string())
}

/// `relative` under `home`, if it is plain, relative and names something in it (never the home
/// itself): the service names data directories relative to the home, and nothing else is removed.
fn under(home: &Path, relative: &str) -> Option<PathBuf> {
    let path = Path::new(relative);
    let mut parts = path.components().peekable();
    (parts.peek().is_some() && parts.all(|part| matches!(part, Component::Normal(_))))
        .then(|| home.join(path))
}

fn flow(
    socket: &Path,
    home: Option<&Path>,
    back: &Back,
    client: ClientId,
    serial: u32,
    request: Ask,
) {
    let answer = |text: Value| back.answer(client, serial, text);
    let timeout = match request {
        Ask::Install { .. } | Ask::Update { .. } | Ask::UpdateAll {} | Ask::Check {} => LONG,
        _ => SHORT,
    };
    let connection = match connect(socket, timeout) {
        Ok(connection) => connection,
        Err(error) => return answer(failed(error)),
    };
    let result: std::io::Result<()> = (|| {
        match request {
            Ask::Status { ids } => {
                let listed = ask(&connection, &Request::List { uid: None })?;
                let state = ask(&connection, &Request::UpdateState { uid: None })?;
                let listed = match listed {
                    Answer::Applications(listed) => listed,
                    other => {
                        answer(to_json(&other));
                        return Ok(());
                    }
                };
                let updates = match state {
                    Answer::UpdateState(state) => state.updates,
                    _ => Vec::new(),
                };
                let status: Vec<Value> = ids
                    .iter()
                    .map(|id| {
                        let installed = listed.iter().find(|l| &l.id == id);
                        let update = updates.iter().find(|u| &u.id == id);
                        json!({
                            "id": id,
                            "installed": installed.is_some(),
                            "version": installed.map(|l| l.version.clone()),
                            "versionCode": installed.map(|l| l.version_code),
                            "update": update.map(|u| json!({
                                "toVersion": u.to_version, "toVersionCode": u.to_version_code,
                                "status": u.status,
                            })),
                        })
                    })
                    .collect();
                answer(json!({ "status": status }));
            }
            Ask::Install { id } => {
                let plan = match ask(&connection, &Request::Prepare { id, uid: None })? {
                    Answer::Plan(plan) => plan,
                    other => {
                        answer(to_json(&other));
                        return Ok(());
                    }
                };
                let sheet = json!({"kind": if plan.plan.update { "update" } else { "install" },
                                   "plan": plan.plan});
                if !back.sheet(sheet) {
                    {
                        answer(json!({"cancelled": {}}));
                        return Ok(());
                    }
                }
                connection.send(&Request::Commit {
                    plan_id: plan.plan_id,
                })?;
                loop {
                    match connection.receive::<Answer>()? {
                        Some(Some(Answer::Progress(progress))) => {
                            answer(json!({ "progress": progress }))
                        }
                        Some(Some(Answer::Committed(committed))) => {
                            break answer(json!({ "installed": committed }))
                        }
                        Some(Some(other)) => break answer(to_json(&other)),
                        _ => break answer(failed("the install service closed the connection")),
                    }
                }
            }
            Ask::Uninstall { id } => {
                let listed = match ask(&connection, &Request::List { uid: None })? {
                    Answer::Applications(listed) => listed,
                    other => {
                        answer(to_json(&other));
                        return Ok(());
                    }
                };
                let Some(installed) = listed.iter().find(|l| l.id == id) else {
                    {
                        answer(json!({"refused": {"reason": "notInstalled"}}));
                        return Ok(());
                    }
                };
                let sheet = json!({"kind": "uninstall", "id": id,
                                   "name": name_of(&installed.entry, &id),
                                   "version": installed.version});
                if !back.sheet(sheet) {
                    {
                        answer(json!({"cancelled": {}}));
                        return Ok(());
                    }
                }
                match ask(&connection, &Request::Uninstall { id, uid: None })? {
                    Answer::Uninstalled {
                        id,
                        data_directories,
                    } => {
                        // The account's data, removed as the account: the service never touches
                        // a home.
                        if let Some(home) = home {
                            for directory in &data_directories {
                                if let Some(path) = under(home, directory) {
                                    let _ = std::fs::remove_dir_all(path);
                                }
                            }
                        }
                        answer(json!({"uninstalled": {"id": id}}));
                    }
                    other => answer(to_json(&other)),
                }
            }
            Ask::Open { id } => back.send(Done::Open(Open { client, serial, id })),
            Ask::State {} => answer(to_json(&ask(
                &connection,
                &Request::UpdateState { uid: None },
            )?)),
            Ask::Check {} => answer(to_json(&ask(
                &connection,
                &Request::CheckUpdates { uid: None },
            )?)),
            Ask::Update { .. } | Ask::UpdateAll {} => {
                let request = match &request {
                    Ask::Update { ids } => Request::Update {
                        ids: ids.clone(),
                        uid: None,
                    },
                    _ => Request::UpdateAll { uid: None },
                };
                let mut updated = match ask(&connection, &request)? {
                    Answer::Updated(updated) => updated,
                    other => {
                        answer(to_json(&other));
                        return Ok(());
                    }
                };
                // Each update waiting for this account's consent is asked in the sheet, one by
                // one, after the rest were applied.
                let waiting: Vec<usize> = (0..updated.len())
                    .filter(|&at| updated[at].outcome == "awaitingConsent")
                    .collect();
                if !waiting.is_empty() {
                    let state = match ask(&connection, &Request::UpdateState { uid: None })? {
                        Answer::UpdateState(state) => state,
                        _ => {
                            answer(json!({ "updated": updated }));
                            return Ok(());
                        }
                    };
                    for at in waiting {
                        let Some(available) = state.updates.iter().find(|u| u.id == updated[at].id)
                        else {
                            continue;
                        };
                        let sheet = json!({"kind": "consent", "update": available});
                        if !back.sheet(sheet) {
                            continue;
                        }
                        if let Answer::Updated(mut given) = ask(
                            &connection,
                            &Request::Consent {
                                id: available.id.clone(),
                                version_code: available.to_version_code,
                                uid: None,
                            },
                        )? {
                            if let Some(given) = given.pop() {
                                updated[at] = given;
                            }
                        }
                    }
                }
                answer(json!({ "updated": updated }));
            }
            Ask::Privacy {} => match ask(&connection, &Request::List { uid: None })? {
                Answer::Applications(listed) => {
                    let applications: Vec<Value> = listed
                        .iter()
                        .map(|l| {
                            json!({"id": l.id, "name": name_of(&l.entry, &l.id),
                                   "capabilities": l.capabilities})
                        })
                        .collect();
                    answer(json!({ "privacy": applications }));
                }
                other => answer(to_json(&other)),
            },
            Ask::Decide {
                id,
                capability,
                granted,
            } => match ask(
                &connection,
                &Request::Decide {
                    id: id.clone(),
                    capability: capability.clone(),
                    granted,
                    uid: None,
                },
            )? {
                Answer::Decided => answer(json!({"decided": {
                    "id": id, "capability": capability, "granted": granted}})),
                other => answer(to_json(&other)),
            },
        }
        Ok(())
    })();
    if let Err(error) = result {
        answer(failed(error));
    }
}

/// A capability asked at first use by an installed application: the sheet, then the account's
/// decision kept by the service, then the answer.
fn permission(socket: &Path, back: &Back, client: ClientId, serial: u32, capability: u8, id: &str) {
    let name = capability_name(capability).expect("checked");
    let granted = back.sheet(json!({"kind": "permission", "id": id, "capability": name}));
    let kept = connect(socket, SHORT).and_then(|connection| {
        ask(
            &connection,
            &Request::Decide {
                id: id.to_string(),
                capability: name.to_string(),
                granted,
                uid: None,
            },
        )
    });
    // What the service did not keep is not granted: the person is asked again next time.
    let granted = granted && matches!(kept, Ok(Answer::Decided));
    back.send(Done::Capability {
        client,
        serial,
        capability,
        granted,
    });
}

/// The account's update state, watched on one connection and sent to the compositor's turn as it
/// changes; a lost connection is made again.
fn watch(socket: PathBuf, back: Back) {
    let mut said = false;
    loop {
        let watched = (|| -> std::io::Result<()> {
            let connection = Connection::connect(&socket)?;
            connection.send(&Request::WatchUpdates { uid: None })?;
            loop {
                match connection.receive::<Answer>()? {
                    Some(Some(Answer::Watching)) => {}
                    Some(Some(state @ Answer::UpdateState(_))) => {
                        back.send(Done::State(to_json(&state).to_string()));
                    }
                    Some(_) => {}
                    None => return Err(std::io::Error::other("closed")),
                }
            }
        })();
        if let Err(error) = watched {
            if !said {
                eprintln!(
                    "[installs] the install service ({}): {error}; its update state is not watched",
                    socket.display()
                );
                said = true;
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests;
