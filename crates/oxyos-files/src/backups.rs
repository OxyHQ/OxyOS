//! The person's backups, asked of the backup service (`oxyos-backupd`, docs/adr/0059-backups.mdx,
//! decisions 7, 8 and 10) on its socket, from the application's own process, as the person: the
//! state (whether there is anywhere backups are kept), the versions of an item, what a folder held
//! in a backup, and an item retrieved through the pipe the service hands over, which
//! [`crate::restore`] writes back.
//!
//! This is the client's half of the service's vocabulary (`crates/oxyos-backup/src/ipc.rs`), read
//! here so an application's process links neither udev nor the engine. The service answers only
//! about the asker's own home, by the socket's peer credentials; the scope (ADR 0028, decision 2)
//! is checked here first, so nothing outside what the application reaches is ever asked.
//!
//! Times are milliseconds since the epoch, as everything else the files service says.

use crate::Error;
use oxyos_service_socket::Connection;
use rustix::event::{poll, PollFd, PollFlags};
use rustix::fd::{AsFd, OwnedFd};
use serde_json::{json, Value};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Where the service listens (`oxyos_backup::ipc::SOCKET`).
pub const SOCKET: &str = "/run/oxyos/backupd/socket";

/// The variable a developer's run names another socket with (a development service's,
/// `oxyos-backupd --development --socket PATH`, as the scenarios start one).
pub const SOCKET_VARIABLE: &str = "OXYOS_BACKUPD_SOCKET";

/// How long an answer may take: a version is found in every backup of every destination
/// (restic's `find`), which on a large repository on a slow disk takes a while.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(600);

/// One backup, by the destination it is on and its id there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backup {
    pub destination: String,
    pub id: String,
}

impl Backup {
    /// A backup as a request names it: `{"destination", "id"}`.
    pub fn from_json(value: Option<&Value>) -> Result<Backup, Error> {
        let invalid = || Error::Invalid("`backup` is {destination, id}".into());
        let value = value.ok_or_else(invalid)?;
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty() && text.len() <= 256)
                .map(String::from)
                .ok_or_else(invalid)
        };
        Ok(Backup {
            destination: text("destination")?,
            id: text("id")?,
        })
    }
}

/// What was retrieved: a file's bytes or a folder as a tar archive, coming through `pipe`; the
/// service says on `connection` whether all of it was written once the pipe ends.
pub struct Retrieval {
    pub folder: bool,
    /// A file's size.
    pub size: Option<u64>,
    /// When the item was last changed, as the backup holds it (milliseconds).
    pub modified: Option<i64>,
    pub pipe: OwnedFd,
    connection: Connection,
}

impl Retrieval {
    /// Whether the service wrote all of it: to be asked once the pipe has ended, before what was
    /// read is put anywhere.
    pub fn finished(self) -> Result<(), Error> {
        match receive(&self.connection)? {
            Value::String(done) if done == "retrieved" => Ok(()),
            answer => Err(refusal(&answer)),
        }
    }
}

/// The backup service, as an application's process reaches it.
#[derive(Clone, Debug)]
pub struct Backups {
    socket: PathBuf,
}

impl Backups {
    /// The system's service, or the one `OXYOS_BACKUPD_SOCKET` names.
    pub fn from_environment() -> Backups {
        Backups::at(
            std::env::var_os(SOCKET_VARIABLE)
                .filter(|path| !path.is_empty())
                .map_or_else(|| PathBuf::from(SOCKET), PathBuf::from),
        )
    }

    pub fn at(socket: impl Into<PathBuf>) -> Backups {
        Backups {
            socket: socket.into(),
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    fn connect(&self) -> io::Result<Connection> {
        let connection = Connection::connect(&self.socket)?;
        connection.set_timeout(ANSWER_TIMEOUT)?;
        Ok(connection)
    }

    /// Asks `request` on a connection of its own; its answer.
    fn ask(&self, request: &Value) -> Result<Value, Error> {
        let connection = self.connect().map_err(unreachable)?;
        connection.send(request)?;
        receive(&connection)
    }

    /// Where backups are kept and when the last was made: `{destinations: [{id, name, kind,
    /// available, lastBackup, oldestBackup}], lastBackup, backingUp}`. A machine whose service is
    /// not running has none.
    pub fn state(&self) -> Result<Value, Error> {
        match self.ask(&json!("status")) {
            Ok(answer) => match answer.get("status") {
                Some(status) => Ok(state(status)),
                None => Err(refusal(&answer)),
            },
            Err(Error::Unavailable(_)) => Ok(state(&Value::Null)),
            Err(error) => Err(error),
        }
    }

    /// The versions of the item at `path` (a canonical path in the person's home), newest
    /// first: `[{backup: {destination, id, time}, isDirectory, size, lastModified}]`. An item no
    /// backup holds has none.
    pub fn versions(&self, path: &Path) -> Result<Value, Error> {
        let answer = self.ask(&json!({ "versions": { "path": text(path)? } }))?;
        let Some(versions) = answer.get("versions").and_then(Value::as_array) else {
            return match refusal(&answer) {
                Error::NotFound => Ok(json!([])),
                error => Err(error),
            };
        };
        Ok(versions
            .iter()
            .filter_map(|version| {
                let directory = folder(version.get("kind")?)?;
                Some(json!({
                    "backup": {
                        "destination": version.get("destination")?,
                        "id": version.get("snapshot")?,
                        "time": millis(version.get("time")),
                    },
                    "isDirectory": directory,
                    "size": if directory { Value::Null } else { version.get("size").cloned().unwrap_or(Value::Null) },
                    "lastModified": millis(version.get("modified")),
                }))
            })
            .collect())
    }

    /// What the folder at `path` held in `backup`: `[{name, isDirectory, size, lastModified}]`,
    /// its files and folders (links and devices are not given back).
    pub fn contents(&self, backup: &Backup, path: &Path) -> Result<Vec<Value>, Error> {
        let answer = self.ask(&json!({ "contents": {
            "destination": backup.destination, "snapshot": backup.id, "path": text(path)?,
        }}))?;
        let Some(items) = answer.get("contents").and_then(Value::as_array) else {
            return Err(refusal(&answer));
        };
        Ok(items
            .iter()
            .filter_map(|item| {
                let directory = folder(item.get("kind")?)?;
                let name = item.get("name")?.as_str()?;
                crate::valid_name(name).ok()?;
                Some(json!({
                    "name": name,
                    "isDirectory": directory,
                    "size": if directory { Value::Null } else { item.get("size").cloned().unwrap_or(Value::Null) },
                    "lastModified": millis(item.get("modified")),
                }))
            })
            .collect())
    }

    /// The item at `path` as `backup` held it, coming through a pipe.
    pub fn retrieve(&self, backup: &Backup, path: &Path) -> Result<Retrieval, Error> {
        let connection = self.connect().map_err(unreachable)?;
        connection.send(&json!({ "retrieve": {
            "destination": backup.destination, "snapshot": backup.id, "path": text(path)?,
        }}))?;
        let (answer, pipe) = match connection.receive_with_fd::<Value>()? {
            Some((Some(answer), pipe)) => (answer, pipe),
            Some((None, _)) => {
                return Err(Error::Invalid(
                    "the backup service's answer is not one".into(),
                ))
            }
            None => return Err(closed()),
        };
        let Some(retrieving) = answer.get("retrieving") else {
            return Err(refusal(&answer));
        };
        let pipe = pipe.ok_or_else(|| Error::Invalid("the backup service sent no pipe".into()))?;
        Ok(Retrieval {
            folder: retrieving.get("kind").and_then(folder) == Some(true),
            size: retrieving.get("size").and_then(Value::as_u64),
            modified: millis(retrieving.get("modified")).as_i64(),
            pipe,
            connection,
        })
    }
}

/// The state the service says, as an application is told it.
fn state(status: &Value) -> Value {
    let destinations: Vec<Value> = status
        .get("destinations")
        .and_then(Value::as_array)
        .map(|destinations| {
            destinations
                .iter()
                .map(|destination| {
                    json!({
                        "id": destination.get("id").cloned().unwrap_or(Value::Null),
                        "name": destination.get("name").cloned().unwrap_or(Value::Null),
                        "kind": destination.get("kind").cloned().unwrap_or(Value::Null),
                        "available": destination.get("available").and_then(Value::as_bool).unwrap_or(false),
                        "lastBackup": millis(destination.get("lastBackup")),
                        "oldestBackup": millis(destination.get("oldestBackup")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "destinations": destinations,
        "lastBackup": millis(status.get("lastBackup")),
        "backingUp": status.get("running").is_some_and(|running| !running.is_null()),
    })
}

/// Seconds as milliseconds, or null.
fn millis(seconds: Option<&Value>) -> Value {
    seconds
        .and_then(Value::as_i64)
        .and_then(|seconds| seconds.checked_mul(1000))
        .map_or(Value::Null, Value::from)
}

/// Whether a kind the service says is a folder (true) or a file (false); none for a link or a
/// device, which are not given back.
fn folder(kind: &Value) -> Option<bool> {
    match kind.as_str()? {
        "file" => Some(false),
        "folder" => Some(true),
        _ => None,
    }
}

fn text(path: &Path) -> Result<&str, Error> {
    path.to_str()
        .ok_or_else(|| Error::Invalid("a path that is not UTF-8".into()))
}

fn closed() -> Error {
    Error::Unavailable("the backup service closed the connection".into())
}

fn unreachable(error: io::Error) -> Error {
    Error::Unavailable(format!("the backup service is not running ({error})"))
}

/// The next answer.
fn receive(connection: &Connection) -> Result<Value, Error> {
    match connection.receive::<Value>() {
        Ok(Some(Some(answer))) => Ok(answer),
        Ok(Some(None)) => Err(Error::Invalid(
            "the backup service's answer is not one".into(),
        )),
        Ok(None) => Err(closed()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Err(Error::Unavailable(
                "the backup service did not answer".into(),
            ))
        }
        Err(error) => Err(error.into()),
    }
}

/// A refusal (`{"refused": {"reason"}}`), or an answer that was not expected, as an error.
fn refusal(answer: &Value) -> Error {
    let reason = answer
        .get("refused")
        .and_then(|refused| refused.get("reason"))
        .and_then(Value::as_str);
    match reason {
        Some("notFound") => Error::NotFound,
        Some("notPermitted") => Error::NotPermitted("the backup service refused it".into()),
        Some(
            reason @ ("notConnected" | "noDestination" | "unreachable" | "noBackup"
            | "shareRefused" | "inUse" | "wrongPassword"),
        ) => Error::Unavailable(format!("the backup is not reachable now ({reason})")),
        Some(reason) => Error::Io(io::Error::other(format!("the backup service: {reason}"))),
        None => Error::Invalid(format!("the backup service answered {answer}")),
    }
}

/// The service's state followed while anything watches it: one connection asking `watch`, its
/// changes said (`changed`) only when what an application is told changes (a backup's progress
/// is not). While the service is not there, the folder its socket appears in is watched
/// (inotify), so nothing wakes until it starts; when it goes, `changed` is said too.
pub struct Follower {
    /// Written to end the thread.
    stop: OwnedFd,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Follower {
    pub fn start(backups: Backups, changed: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        let (ending, stop) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?;
        let changed: Arc<dyn Fn() + Send + Sync> = Arc::new(changed);
        let thread = std::thread::Builder::new()
            .name("oxyos-files-backups".into())
            .spawn(move || follow(&backups, &ending, &*changed))?;
        Ok(Follower {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        let _ = rustix::io::write(&self.stop, &[1]);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Whether `ending` is readable (or `other` is): waits for either, without a timeout.
fn wait(ending: &OwnedFd, other: impl AsFd) -> (bool, bool) {
    let mut fds = [
        PollFd::new(ending, PollFlags::IN),
        PollFd::new(&other, PollFlags::IN),
    ];
    loop {
        match poll(&mut fds, None) {
            Ok(_) => break,
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => return (true, false),
        }
    }
    (!fds[0].revents().is_empty(), !fds[1].revents().is_empty())
}

fn follow(backups: &Backups, ending: &OwnedFd, changed: &(dyn Fn() + Send + Sync)) {
    let mut last = state(&Value::Null);
    loop {
        // The socket's folder (or the nearest that is there) watched before connecting, so a
        // service starting between the two is not missed.
        let appearing = appearing(backups.socket());
        match Connection::connect(backups.socket()) {
            Ok(connection) => {
                drop(appearing);
                if connection.send(&json!("watch")).is_err() {
                    continue;
                }
                loop {
                    let (stopped, readable) = wait(ending, &connection);
                    if stopped {
                        return;
                    }
                    if !readable {
                        continue;
                    }
                    let Ok(Some(Some(answer))) = connection.receive::<Value>() else {
                        break;
                    };
                    if let Some(status) = answer.get("status") {
                        let now = state(status);
                        if now != last {
                            last = now;
                            changed();
                        }
                    }
                }
                // The service went: none are there now.
                let none = state(&Value::Null);
                if last != none {
                    last = none;
                    changed();
                }
            }
            Err(_) => match appearing {
                Some(inotify) => {
                    let (stopped, _) = wait(ending, &inotify);
                    if stopped {
                        return;
                    }
                }
                // Nothing to watch (no folder of the path is there): nothing will appear.
                None => {
                    let _ = wait(ending, ending);
                    return;
                }
            },
        }
    }
}

/// An inotify descriptor watching the socket's folder, or the nearest of its folders that is
/// there, for what is made or moved in it.
fn appearing(socket: &Path) -> Option<OwnedFd> {
    use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
    let inotify = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK).ok()?;
    let mut folder = socket.parent()?;
    loop {
        let flags = WatchFlags::CREATE | WatchFlags::MOVED_TO | WatchFlags::ATTRIB;
        if inotify::add_watch(&inotify, folder, flags).is_ok() {
            return Some(inotify);
        }
        folder = folder.parent()?;
    }
}
