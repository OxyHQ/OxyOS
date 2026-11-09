//! Backups for the program running the compositor (docs/adr/0059-backups.mdx, step 2), the
//! session authority's side of the backup service (`oxyos-backupd`): what the menu bar's backup
//! item and Settings › Backups are told and may ask, and Low Power Mode's hold.
//!
//! - **One connection, followed.** A thread keeps one connection to the service's socket, asks it
//!   to `watch` and hands each state it says over to the compositor's turn, which tells the shell
//!   and Settings (serial 0) when it changed. Nothing wakes on a clock: the thread waits on the
//!   connection, on Low Power Mode's descriptor and on the order to stop; while the service is not
//!   there, on its socket being made (`Stop::wait_for_path`, as the install service's is waited
//!   for), so a service started or restarted later is followed at once, and until then the
//!   clients are told there is none (`{"backup": null}`).
//! - **Low Power Mode holds scheduled backups back** (ADR 0057, decision 4; ADR 0059, decision 4):
//!   while `LowPower` is on, the connection holds them (`{"hold":{"reason":"lowPower"}}`), and
//!   releases them when it goes off. The hold is the connection's: it goes with it, and is asked
//!   again on the next one.
//! - **Who may ask** (`SystemRequest`, the session's vocabulary, read strictly): the shell
//!   (`shell`) and Settings (`settings`) the state, *Back Up Now* and *Skip This Backup* (those
//!   two citing an input); Settings alone the disks that can be added, adding a disk or a share,
//!   removing a destination, the excluded items and *Back Up Automatically*, each but the disks
//!   citing an input. The service decides what an administrator may change (its state's
//!   `administrator`) by the session's credentials; its refusals are passed on as failures.
//! - **Each request runs off the compositor's turn**, on a thread of its own with a connection of
//!   its own (adding a destination sets up an encrypted repository, seconds), and is answered once
//!   the service answered. Passwords pass through to the service: never kept, never logged.

use crate::stop::{identity, Stop};
use oxyos_backup::share::Share;
use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::{ClientId, Scene};
use oxyos_power::LowPower;
use oxyos_preferences::Preferences;
use oxyos_service_socket::Connection;
use rustix::event::{poll, PollFd, PollFlags, Timespec};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use zeroize::Zeroizing;

/// How long a service that took the connection may take to say its state: one that does not is
/// taken as not there until it makes its socket again.
const FIRST_STATE: Duration = Duration::from_secs(5);

/// What a client asks, read strictly.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Ask {
    /// The state.
    Backup {},
    /// *Back Up Now*, with an input.
    BackupNow {},
    /// *Skip This Backup*, with an input.
    BackupStop {},
    /// Settings: the disks connected that can hold backups.
    BackupDisks {},
    /// Settings, with an input: a disk added, with the person's backup password.
    BackupAddDisk {
        uuid: String,
        password: Zeroizing<String>,
    },
    /// Settings, with an input: a share added, with its user's password and the backup password.
    #[serde(rename_all = "camelCase")]
    BackupAddShare {
        share: Share,
        share_password: Zeroizing<String>,
        password: Zeroizing<String>,
    },
    /// Settings, with an input: a destination removed (its backups stay where they are).
    BackupRemove { id: String },
    /// Settings, with an input: the items left out of backups, as absolute paths.
    BackupExcluded { paths: Vec<String> },
    /// Settings, with an input: *Back Up Automatically*.
    BackupAutomatic { enabled: bool },
}

const ASKS: [&str; 9] = [
    "backup",
    "backupNow",
    "backupStop",
    "backupDisks",
    "backupAddDisk",
    "backupAddShare",
    "backupRemove",
    "backupExcluded",
    "backupAutomatic",
];

fn ours(text: &str) -> bool {
    serde_json::from_str::<HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.keys().all(|key| ASKS.contains(&key.as_str())))
}

impl Ask {
    fn settings_only(&self) -> bool {
        !matches!(
            self,
            Ask::Backup {} | Ask::BackupNow {} | Ask::BackupStop {}
        )
    }

    fn changes(&self) -> bool {
        !matches!(self, Ask::Backup {} | Ask::BackupDisks {})
    }

    /// What it asks of the service, and the name its answer is given back by.
    fn request(&self) -> (Value, &'static str) {
        let (request, name) = match self {
            Ask::Backup {} => (json!("status"), "backup"),
            Ask::BackupNow {} => (json!({ "backUpNow": {} }), "backupNow"),
            Ask::BackupStop {} => (json!({ "stop": {} }), "backupStop"),
            Ask::BackupDisks {} => (json!({ "disks": {} }), "backupDisks"),
            Ask::BackupAddDisk { uuid, password } => (
                json!({ "addDisk": { "uuid": uuid, "password": password.as_str() } }),
                "backupAddDisk",
            ),
            Ask::BackupAddShare {
                share,
                share_password,
                password,
            } => (
                json!({ "addShare": {
                    "share": share,
                    "sharePassword": share_password.as_str(),
                    "password": password.as_str(),
                } }),
                "backupAddShare",
            ),
            Ask::BackupRemove { id } => (json!({ "remove": { "id": id } }), "backupRemove"),
            Ask::BackupExcluded { paths } => (
                json!({ "setExcluded": { "paths": paths } }),
                "backupExcluded",
            ),
            Ask::BackupAutomatic { enabled } => (
                json!({ "setAutomatic": { "enabled": enabled } }),
                "backupAutomatic",
            ),
        };
        (request, name)
    }
}

/// The service's answer to a request named `name`, in the session's vocabulary: the request's own
/// name for what it did, `{"failed":{"reason"}}` with the service's reason for what it refused.
fn answered(name: &str, answer: &Value) -> Value {
    if let Some(reason) = answer
        .get("refused")
        .and_then(|refused| refused.get("reason"))
        .and_then(Value::as_str)
    {
        return json!({ "failed": { "reason": reason } });
    }
    let done = match (name, answer) {
        ("backupNow", Value::String(said)) if said == "started" => Some(json!({})),
        ("backupStop", Value::String(said)) if said == "stopping" => Some(json!({})),
        ("backupRemove", Value::String(said)) if said == "removed" => Some(json!({})),
        ("backupExcluded" | "backupAutomatic", Value::String(said)) if said == "set" => {
            Some(json!({}))
        }
        ("backupDisks", Value::Object(object)) => object
            .get("disks")
            .filter(|disks| disks.is_array())
            .cloned(),
        ("backupAddDisk" | "backupAddShare", Value::Object(object)) => object
            .get("added")
            .and_then(|added| added.get("id"))
            .and_then(Value::as_str)
            .map(|id| json!({ "id": id })),
        _ => None,
    };
    match done {
        Some(done) => json!({ name: done }),
        None => json!({ "failed": { "reason": "failed" } }),
    }
}

fn refused(reason: &str) -> String {
    json!({ "refused": { "reason": reason } }).to_string()
}

/// What a thread did.
enum Done {
    /// The service's state, or that it is not there.
    State(Option<Value>),
    Answer {
        client: ClientId,
        serial: u32,
        text: String,
    },
}

/// A thread's way back: what it did, and a byte to wake the compositor's turn.
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

pub struct Backups {
    socket: PathBuf,
    back: Back,
    done: mpsc::Receiver<Done>,
    woken: UnixStream,
    /// The service's state as it last said it: none until the first word of the follower; `null`
    /// while the service is not there.
    state: Option<Value>,
    in_menu_bar: bool,
    /// Who asked for the state before the follower first said it.
    waiting: Vec<(ClientId, u32)>,
    told: Option<String>,
    /// Its threads stop when it goes, and are waited for, so nothing of it outlives it.
    stop: Arc<Stop>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Backups {
    /// The session's side of the backup service at `socket` (`oxyos_backup::ipc::SOCKET`, or the
    /// development service's), holding scheduled backups back while `low_power` is on.
    pub fn new(socket: PathBuf, low_power: LowPower) -> std::io::Result<Self> {
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let (done, received) = mpsc::channel();
        let back = Back {
            done,
            wake: Arc::new(wake),
        };
        let stop = Arc::new(Stop::new()?);
        let changes = low_power.changes()?;
        let follower = {
            let (socket, back, stop) = (socket.clone(), back.clone(), stop.clone());
            std::thread::Builder::new()
                .name("backups-follow".into())
                .spawn(move || follow(&socket, &low_power, &changes, &back, &stop))?
        };
        Ok(Self {
            socket,
            back,
            done: received,
            woken,
            state: None,
            in_menu_bar: Preferences::default().backup_in_menu_bar,
            waiting: Vec::new(),
            told: None,
            stop,
            threads: vec![follower],
        })
    }

    /// What wakes the compositor's turn when a thread is done (`Compositor::wakers`).
    pub fn wake_fd(&self) -> std::io::Result<OwnedFd> {
        Ok(self.woken.try_clone()?.into())
    }

    /// A turn: the preference, the requests that are this part's, what the threads did, and the
    /// state told if it changed.
    pub fn serve(&mut self, scene: &mut Scene, preferences: &Preferences) {
        let mut buffer = [0u8; 64];
        while matches!((&self.woken).read(&mut buffer), Ok(read) if read > 0) {}
        self.threads.retain(|thread| !thread.is_finished());
        self.in_menu_bar = preferences.backup_in_menu_bar;
        for request in scene.take_system_requests_where(|request| ours(&request.text)) {
            self.handle(scene, request);
        }
        for done in self.done.try_iter().collect::<Vec<_>>() {
            match done {
                Done::State(state) => self.state = Some(state.unwrap_or(Value::Null)),
                Done::Answer {
                    client,
                    serial,
                    text,
                } => scene.system_text(client, serial, &text),
            }
        }
        if let Some(text) = self.text() {
            for (client, serial) in std::mem::take(&mut self.waiting) {
                scene.system_text(client, serial, &text);
            }
            if self.told.as_deref() != Some(text.as_str()) {
                for client in scene.backup_watchers() {
                    scene.system_text(client, 0, &text);
                }
                self.told = Some(text);
            }
        }
    }

    /// The state as the shell and Settings are told it: the service's, with whether the person
    /// wants the menu bar's item; `null` while the service is not there. None before it is known.
    fn text(&self) -> Option<String> {
        let state = match self.state.as_ref()? {
            Value::Object(object) => {
                let mut object = object.clone();
                object.insert("inMenuBar".into(), Value::Bool(self.in_menu_bar));
                Value::Object(object)
            }
            _ => Value::Null,
        };
        Some(json!({ "backup": state }).to_string())
    }

    fn handle(&mut self, scene: &mut Scene, request: SystemRequest) {
        let (client, serial) = (request.client, request.serial);
        let Ok(ask) = serde_json::from_str::<Ask>(&request.text) else {
            scene.system_text(client, serial, &refused("invalidRequest"));
            return;
        };
        let Some(grants) = scene.grants_of(client) else {
            return;
        };
        let allowed = if ask.settings_only() {
            grants.settings
        } else {
            grants.settings || grants.shell
        };
        if !allowed {
            scene.system_text(client, serial, &refused("notPermitted"));
            return;
        }
        if ask.changes() && !request.with_input {
            scene.system_text(client, serial, &refused("needsInput"));
            return;
        }
        if let Ask::Backup {} = ask {
            // The state as followed: the service is never asked again for what it tells.
            match self.text() {
                Some(text) => scene.system_text(client, serial, &text),
                None => self.waiting.push((client, serial)),
            }
            return;
        }
        let (socket, back, stop) = (self.socket.clone(), self.back.clone(), self.stop.clone());
        let spawned = std::thread::Builder::new()
            .name("backups".into())
            .spawn(move || {
                let (request, name) = ask.request();
                drop(ask);
                let text = match ask_service(&socket, &request, &stop) {
                    Ok(answer) => answered(name, &answer),
                    Err(error) => {
                        if stop.given() {
                            return;
                        }
                        eprintln!(
                            "[backups] the backup service ({}): {error}",
                            socket.display()
                        );
                        json!({ "failed": { "reason": "unavailable" } })
                    }
                };
                back.send(Done::Answer {
                    client,
                    serial,
                    text: text.to_string(),
                });
            });
        match spawned {
            Ok(thread) => self.threads.push(thread),
            Err(_) => scene.system_text(
                client,
                serial,
                &json!({ "failed": { "reason": "failed" } }).to_string(),
            ),
        }
    }
}

impl Drop for Backups {
    fn drop(&mut self) {
        self.stop.give();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// One request of the service and its answer, on a connection of its own; waits for the answer
/// however long the service takes (setting up a repository), or until the session goes.
fn ask_service(socket: &Path, request: &Value, stop: &Stop) -> std::io::Result<Value> {
    let connection = Connection::connect(socket)?;
    connection.send(request)?;
    if !stop.wait_for(connection.as_fd()) {
        return Err(std::io::Error::other("the session ended"));
    }
    match connection.receive::<Value>()? {
        Some(Some(answer)) => Ok(answer),
        Some(None) => Err(std::io::Error::other("an answer the session does not read")),
        None => Err(std::io::Error::other("the service closed the connection")),
    }
}

/// The follower: the service's state on one connection, Low Power Mode's hold on it, and the
/// service waited for while it is not there; until the order to stop.
fn follow(
    socket: &Path,
    low_power: &LowPower,
    changes: &oxyos_power::Changes,
    back: &Back,
    stop: &Stop,
) {
    let mut said = false;
    loop {
        let tried = identity(socket);
        let followed = match tried {
            Some(_) => followed(socket, low_power, changes, back, stop),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no socket",
            )),
        };
        if stop.given() {
            return;
        }
        match followed {
            // It was followed, then went: said so, and waited for again.
            Ok(()) => {
                eprintln!("[backups] the backup service went; followed again once it is back");
                said = false;
            }
            Err(error) if !said => {
                eprintln!(
                    "[backups] the backup service ({}): {error}; followed once it is there",
                    socket.display()
                );
                said = true;
            }
            Err(_) => {}
        }
        // The clients told there is none (the turn tells only what changed).
        back.send(Done::State(None));
        // A socket made again (a service started or restarted), never the one that failed.
        if !stop.wait_for_path(socket, tried) {
            return;
        }
    }
}

/// Follows the service on one connection until it closes it (`Ok`) or the order to stop (`Ok`,
/// with the order given); an error when it could not be followed at all.
fn followed(
    socket: &Path,
    low_power: &LowPower,
    changes: &oxyos_power::Changes,
    back: &Back,
    stop: &Stop,
) -> std::io::Result<()> {
    let connection = Connection::connect(socket)?;
    connection.send(&json!("watch"))?;
    // Its state first, before anything is held: a service that took the connection and says
    // nothing is not followed.
    let mut fds = [PollFd::from_borrowed_fd(connection.as_fd(), PollFlags::IN)];
    let timeout = Timespec::try_from(FIRST_STATE).map_err(std::io::Error::other)?;
    if poll(&mut fds, Some(&timeout))? == 0 {
        return Err(std::io::Error::other("it did not say its state"));
    }
    let mut first = true;
    let mut holding = false;
    loop {
        if !first {
            let mut fds = [
                PollFd::from_borrowed_fd(stop.fd(), PollFlags::IN),
                PollFd::from_borrowed_fd(connection.as_fd(), PollFlags::IN),
                PollFd::from_borrowed_fd(changes.as_fd(), PollFlags::IN),
            ];
            match poll(&mut fds, None) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            if stop.given() {
                return Ok(());
            }
            if fds[1].revents().is_empty() {
                // Low Power Mode changed: held or released to match.
                changes.take();
                holding = hold(&connection, low_power.on(), holding)?;
                continue;
            }
        }
        match connection.receive::<Value>()? {
            Some(Some(Value::Object(mut said))) => {
                if let Some(state @ Value::Object(_)) = said.remove("status") {
                    back.send(Done::State(Some(state)));
                } else if first {
                    return Err(std::io::Error::other("it did not say its state"));
                }
            }
            // `held`, `released`: what was held is known already.
            Some(Some(_)) if !first => {}
            Some(_) => return Err(std::io::Error::other("an answer the session does not read")),
            None if first => return Err(std::io::Error::other("it closed the connection")),
            None => return Ok(()),
        }
        if first {
            first = false;
            // Low Power Mode as it is now; its changes from here.
            changes.take();
            holding = hold(&connection, low_power.on(), false)?;
        }
    }
}

/// Holds scheduled backups back (`on`) or releases them, if that is not what the connection does
/// already; what it does now.
fn hold(connection: &Connection, on: bool, holding: bool) -> std::io::Result<bool> {
    if on != holding {
        let request = if on {
            json!({ "hold": { "reason": "lowPower" } })
        } else {
            json!({ "release": {} })
        };
        connection.send(&request)?;
        eprintln!(
            "[backups] scheduled backups {} (Low Power Mode {})",
            if on { "held" } else { "released" },
            if on { "on" } else { "off" }
        );
    }
    Ok(on)
}

#[cfg(test)]
mod tests;
