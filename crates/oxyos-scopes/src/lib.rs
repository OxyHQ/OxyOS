//! One control group per application (docs/adr/0066-quality-of-service.mdx, decision 3).
//!
//! The session launches each application into a scope of its own in the person's user manager's
//! `app.slice`, named as systemd's desktop integration names them (`app-<launcher>-<id>-<pid>.scope`,
//! <https://systemd.io/DESKTOP_ENVIRONMENTS/>), and runs its search service in `background.slice`;
//! the compositor and the shell stay in the logind session's scope, a sibling of the user manager,
//! so together they weigh as much as every application together. An application with thirty busy
//! threads is one group, not thirty shares of the compositor's run queue.
//!
//! A process is placed **by itself, before it runs the application's program**: the launcher starts
//! `oxyos-scope` (`tools/oxyos-scope`), which asks for its scope with its own pid and waits for the
//! job that moves it there, then executes the program in the same process (as `systemd-run --scope`
//! does). Nothing the program does can happen outside its scope, the launcher's child and its pidfd
//! stay the application's, and the compositor's turn never waits on the bus.
//!
//! Scopes are made by a [`Manager`]: the person's user manager over the user bus (the session's), or
//! the saturation benchmark's fixture (`OXYOS_CGROUP_FIXTURE`, a delegated control group that stands
//! for the user manager's, docs/performance.mdx): the same layout made with the control group
//! filesystem's own files, so the development compositor's applications are placed as a session's
//! are. [`Scopes`] changes a scope's weights while it runs (decisions 4 and 5), off its caller's
//! thread.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub mod played;

/// The environment variable naming the saturation benchmark's fixture: a control group standing for
/// the person's user manager's (`user@UID.service`), in which `app.slice` and `background.slice` are
/// made as the user manager makes them.
pub const FIXTURE_ENV: &str = "OXYOS_CGROUP_FIXTURE";
/// The launcher's name in its applications' scopes, as systemd's desktop integration has it.
pub const LAUNCHER: &str = "oxyos";
/// A scope's CPU weight as it is made: systemd's default, set so the user manager enables the CPU
/// controller in `app.slice` and each application is a group of its own (without a unit asking
/// for a CPU property, `app.slice`'s applications share one group, thread by thread).
pub const DEFAULT_CPU_WEIGHT: u64 = 100;
/// How long a placement waits for the user manager's job before the program runs where it is.
pub const PLACE_TIMEOUT: Duration = Duration::from_secs(5);

const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";
const NO_SUCH_UNIT: &str = "org.freedesktop.systemd1.NoSuchUnit";

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

/// The user manager's slice a scope is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slice {
    /// `app.slice`: the person's applications.
    App,
    /// `background.slice`: work the person did not ask for now (the search service's index).
    Background,
}

impl Slice {
    pub fn name(self) -> &'static str {
        match self {
            Slice::App => "app.slice",
            Slice::Background => "background.slice",
        }
    }
}

/// A scope: its unit's name and the slice it is in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Unit {
    pub name: String,
    pub slice: Slice,
}

impl Unit {
    /// An application's: `app-oxyos-<id>-<pid>.scope` in `app.slice`, the id escaped as systemd
    /// escapes a unit name's part (so a `-` in it never reads as the name's separator).
    pub fn application(id: &str, pid: u32) -> Self {
        Self {
            name: format!("app-{LAUNCHER}-{}-{pid}.scope", escape(id)),
            slice: Slice::App,
        }
    }

    /// The slice itself (`app.slice`): what its children's protection is drawn from.
    pub fn slice(slice: Slice) -> Self {
        Self {
            name: slice.name().to_string(),
            slice,
        }
    }

    /// A background service's: `oxyos-<name>-<pid>.scope` in `background.slice`.
    pub fn background(name: &str, pid: u32) -> Self {
        Self {
            name: format!("{LAUNCHER}-{}-{pid}.scope", escape(name)),
            slice: Slice::Background,
        }
    }
}

/// A unit name's part escaped as `systemd-escape` does: ASCII letters, digits, `:`, `_` and `.`
/// (not first) kept, every other byte `\xNN`.
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for (index, byte) in text.bytes().enumerate() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b':' | b'_' => escaped.push(byte as char),
            b'.' if index > 0 => escaped.push('.'),
            _ => escaped.push_str(&format!("\\x{byte:02x}")),
        }
    }
    escaped
}

/// Where scopes are made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Manager {
    /// The person's user manager, over the user bus (`DBUS_SESSION_BUS_ADDRESS`, else
    /// `$XDG_RUNTIME_DIR/bus`).
    User,
    /// The saturation benchmark's fixture: the control group standing for the user manager's.
    Fixture(PathBuf),
}

impl Manager {
    /// The fixture `OXYOS_CGROUP_FIXTURE` names, if it names one.
    pub fn fixture_from_env() -> Option<Self> {
        std::env::var_os(FIXTURE_ENV)
            .filter(|dir| !dir.is_empty())
            .map(|dir| Self::Fixture(PathBuf::from(dir)))
    }
}

/// Whether the user bus is there to ask the user manager (the login screen's user has none).
pub fn user_bus_available() -> bool {
    match std::env::var_os("DBUS_SESSION_BUS_ADDRESS") {
        Some(address) => !address.is_empty(),
        None => std::env::var_os("XDG_RUNTIME_DIR")
            .is_some_and(|dir| Path::new(&dir).join("bus").exists()),
    }
}

/// A scope's CPU weight: a weight (1 to 10000, 100 the default) or the idle class, the group's
/// `SCHED_IDLE` (`cpu.idle`, systemd's `CPUWeight=idle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuWeight {
    Weight(u64),
    Idle,
}

/// What [`Scopes::set`] changes of a running scope; what is `None` is left as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Properties {
    pub cpu_weight: Option<CpuWeight>,
    /// The disk's weight (1 to 10000, 100 the default), where the I/O controller is delegated.
    pub io_weight: Option<u64>,
    /// Memory the kernel reclaims from it only when nothing unprotected is left (`MemoryLow`,
    /// `memory.low`), in bytes: the focused application's working set (decision 4).
    pub memory_low: Option<u64>,
}

impl Properties {
    fn merged(self, newer: Properties) -> Properties {
        Properties {
            cpu_weight: newer.cpu_weight.or(self.cpu_weight),
            io_weight: newer.io_weight.or(self.io_weight),
            memory_low: newer.memory_low.or(self.memory_low),
        }
    }
}

/// Puts the process `pid` (the caller, for `oxyos-scope`) into `unit`'s scope, made by `manager`;
/// with the user manager, waits until the job that moves it is done.
pub fn place(manager: &Manager, unit: &Unit, description: &str, pid: u32) -> io::Result<()> {
    match manager {
        Manager::User => {
            let connection = zbus::blocking::Connection::session().map_err(failed)?;
            user::place(&connection, unit, description, pid)
        }
        Manager::Fixture(root) => fixture::place(root, unit, pid),
    }
}

/// The user manager's side.
pub mod user {
    use super::*;
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, Value};

    fn manager(connection: &Connection) -> io::Result<Proxy<'static>> {
        Proxy::new(connection, SYSTEMD, SYSTEMD_PATH, MANAGER).map_err(failed)
    }

    /// `StartTransientUnit` for `unit` with the process `pid`, then its job awaited (the user
    /// manager signals the jobs it ends to a client that subscribed).
    pub fn place(
        connection: &Connection,
        unit: &Unit,
        description: &str,
        pid: u32,
    ) -> io::Result<()> {
        let manager = manager(connection)?;
        manager.call_method("Subscribe", &()).map_err(failed)?;
        let removed = manager.receive_signal("JobRemoved").map_err(failed)?;
        // Each job the manager ends, as it ends it: the signal may come before the call's answer.
        let (ended, jobs) = mpsc::channel();
        std::thread::Builder::new()
            .name("scope-jobs".into())
            .spawn(move || {
                for message in removed {
                    let Ok((_, job, _, result)) =
                        message
                            .body()
                            .deserialize::<(u32, OwnedObjectPath, String, String)>()
                    else {
                        continue;
                    };
                    if ended.send((job, result)).is_err() {
                        break;
                    }
                }
            })?;
        let properties: Vec<(&str, Value<'_>)> = vec![
            ("Description", Value::from(description)),
            ("Slice", Value::from(unit.slice.name())),
            ("PIDs", Value::from(vec![pid])),
            ("CPUWeight", Value::from(DEFAULT_CPU_WEIGHT)),
            // Collected once its processes are gone, even ended by a fault (or the out-of-memory
            // killer), so a name is never left behind.
            ("CollectMode", Value::from("inactive-or-failed")),
        ];
        let auxiliary: Vec<(&str, Vec<(&str, Value<'_>)>)> = Vec::new();
        let job: OwnedObjectPath = manager
            .call(
                "StartTransientUnit",
                &(unit.name.as_str(), "fail", properties, auxiliary),
            )
            .map_err(failed)?;
        let deadline = Instant::now() + PLACE_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match jobs.recv_timeout(left) {
                Ok((ended, result)) if ended == job => {
                    return match result.as_str() {
                        "done" => Ok(()),
                        other => Err(failed(format!("its job ended {other}"))),
                    };
                }
                Ok(_) => continue,
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("its job did not end in {PLACE_TIMEOUT:?}"),
                    ))
                }
            }
        }
    }

    /// `SetUnitProperties` for this boot (`runtime`): Ok(false) when the scope is not there (yet).
    pub fn set(connection: &Connection, unit: &Unit, properties: Properties) -> io::Result<bool> {
        let mut changed: Vec<(&str, Value<'_>)> = Vec::new();
        if let Some(weight) = properties.cpu_weight {
            // systemd's `CPUWeight=idle` is the weight 0 on the bus.
            let weight = match weight {
                CpuWeight::Weight(weight) => weight,
                CpuWeight::Idle => 0,
            };
            changed.push(("CPUWeight", Value::from(weight)));
        }
        if let Some(weight) = properties.io_weight {
            changed.push(("IOWeight", Value::from(weight)));
        }
        if let Some(bytes) = properties.memory_low {
            changed.push(("MemoryLow", Value::from(bytes)));
        }
        if changed.is_empty() {
            return Ok(true);
        }
        match manager(connection)?
            .call_method("SetUnitProperties", &(unit.name.as_str(), true, changed))
        {
            Ok(_) => Ok(true),
            Err(zbus::Error::MethodError(name, _, _)) if name.as_str() == NO_SUCH_UNIT => Ok(false),
            Err(error) => Err(failed(error)),
        }
    }
}

/// The saturation benchmark's fixture: the user manager's layout made in a delegated control group
/// with the control group filesystem's own files, as the user manager makes it (each level enabling
/// its parent's controllers, so a scope has its own `cpu.weight`).
pub mod fixture {
    use super::*;

    /// Where `unit`'s scope is in the fixture `root`.
    pub fn scope(root: &Path, unit: &Unit) -> PathBuf {
        if unit.name == unit.slice.name() {
            return root.join(&unit.name);
        }
        root.join(unit.slice.name()).join(&unit.name)
    }

    /// `child` made in `parent` (if it is not there), the parent's controllers enabled for its
    /// children first, as systemd enables them: each one its own write, so one the parent cannot
    /// give is left out and the others are not.
    fn make(parent: &Path, child: &Path) -> io::Result<()> {
        if let Ok(controllers) = std::fs::read_to_string(parent.join("cgroup.controllers")) {
            for controller in controllers.split_whitespace() {
                let _ = std::fs::write(
                    parent.join("cgroup.subtree_control"),
                    format!("+{controller}"),
                );
            }
        }
        match std::fs::create_dir(child) {
            Err(error) if error.kind() != io::ErrorKind::AlreadyExists => Err(error),
            _ => Ok(()),
        }
    }

    /// Writes `value` into a control file of `dir`: an existing one (the control group's own), or,
    /// in a plain directory standing for one (the tests'), a new one.
    fn write(dir: &Path, file: &str, value: &str) -> io::Result<()> {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(file))?
            .write_all(value.as_bytes())
    }

    /// `pid` moved into `unit`'s scope, made with the default CPU weight.
    pub fn place(root: &Path, unit: &Unit, pid: u32) -> io::Result<()> {
        let slice = root.join(unit.slice.name());
        let scope = scope(root, unit);
        make(root, &slice)?;
        make(&slice, &scope)?;
        if scope.join("cpu.weight").exists() {
            write(&scope, "cpu.weight", &DEFAULT_CPU_WEIGHT.to_string())?;
        }
        write(&scope, "cgroup.procs", &pid.to_string())
    }

    /// `properties` written into `unit`'s scope: Ok(false) when it is not there (yet). In a
    /// control group, a controller not enabled for it has no file, and that property is left out.
    pub fn set(root: &Path, unit: &Unit, properties: Properties) -> io::Result<bool> {
        let scope = scope(root, unit);
        if !scope.is_dir() {
            return Ok(false);
        }
        let real = scope.join("cgroup.controllers").exists();
        let has = |file: &str| !real || scope.join(file).exists();
        match properties.cpu_weight {
            Some(CpuWeight::Idle) if has("cpu.idle") => write(&scope, "cpu.idle", "1")?,
            Some(CpuWeight::Weight(weight)) if has("cpu.weight") => {
                if scope.join("cpu.idle").exists() {
                    write(&scope, "cpu.idle", "0")?;
                }
                write(&scope, "cpu.weight", &weight.to_string())?;
            }
            _ => {}
        }
        if let Some(weight) = properties.io_weight.filter(|_| has("io.weight")) {
            write(&scope, "io.weight", &format!("default {weight}"))?;
        }
        if let Some(bytes) = properties.memory_low.filter(|_| has("memory.low")) {
            write(&scope, "memory.low", &bytes.to_string())?;
        }
        Ok(true)
    }
}

/// The user bus as the session keeps a connection: its executor on a thread of its own waiting with
/// futures-lite's block_on, never async-io's, so a session doing nothing does not wake for the
/// bus's traffic (docs/performance.mdx, *Idle*; `oxyos_seat::system_bus`). It ends with the
/// connection.
fn user_bus() -> io::Result<zbus::blocking::Connection> {
    let connection = futures_lite::future::block_on(
        zbus::connection::Builder::session()
            .map_err(failed)?
            .internal_executor(false)
            .build(),
    )
    .map_err(failed)?;
    let executor = connection.executor().clone();
    std::thread::Builder::new()
        .name("oxyos-user-bus".into())
        .spawn(move || {
            futures_lite::future::block_on(async move {
                while !executor.is_empty() {
                    executor.tick().await;
                }
            })
        })?;
    Ok(connection.into())
}

/// How often a property set for a scope that is not there yet is tried again (its process is still
/// being placed), and for how long.
const RETRY: Duration = Duration::from_millis(50);
const RETRY_FOR: Duration = Duration::from_secs(5);

enum Job {
    Set(Unit, Properties),
    Forget(Unit),
}

/// Changes running scopes' properties (decisions 4 and 5: the weights the compositor gives each
/// application by what the person sees), on a thread of its own: [`Scopes::set`] never waits. The
/// latest properties asked for a scope win; one asked for a scope still being placed is applied as
/// soon as it is there. While nothing is asked, the thread waits with no deadline.
pub struct Scopes {
    jobs: mpsc::Sender<Job>,
}

impl Scopes {
    /// Its thread, applying through `manager`: the user manager's connection is made at the first
    /// change asked, so a session that never weighs a scope never connects.
    pub fn start(manager: Manager) -> io::Result<Self> {
        let (jobs, asked) = mpsc::channel();
        std::thread::Builder::new()
            .name("scopes".into())
            .spawn(move || serve(manager, asked))?;
        Ok(Self { jobs })
    }

    /// Sets `properties` on `unit`'s scope.
    pub fn set(&self, unit: &Unit, properties: Properties) {
        let _ = self.jobs.send(Job::Set(unit.clone(), properties));
    }

    /// Forgets what is still to be set on `unit`'s scope (its process ended).
    pub fn forget(&self, unit: &Unit) {
        let _ = self.jobs.send(Job::Forget(unit.clone()));
    }
}

fn serve(manager: Manager, asked: mpsc::Receiver<Job>) {
    let mut connection: Option<zbus::blocking::Connection> = None;
    let mut warned = false;
    // Each scope's properties still to set: what, when it was first asked, and when to try next.
    let mut pending: HashMap<Unit, (Properties, Instant, Instant)> = HashMap::new();
    loop {
        let job = match pending.values().map(|(_, _, next)| *next).min() {
            None => match asked.recv() {
                Ok(job) => Some(job),
                Err(_) => return,
            },
            Some(next) => {
                match asked.recv_timeout(next.saturating_duration_since(Instant::now())) {
                    Ok(job) => Some(job),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        };
        let now = Instant::now();
        match job {
            Some(Job::Set(unit, properties)) => {
                let entry = pending
                    .entry(unit)
                    .or_insert((Properties::default(), now, now));
                entry.0 = entry.0.merged(properties);
                entry.2 = now;
            }
            Some(Job::Forget(unit)) => {
                pending.remove(&unit);
            }
            None => {}
        }
        let due: Vec<Unit> = pending
            .iter()
            .filter(|(_, (_, _, next))| *next <= now)
            .map(|(unit, _)| unit.clone())
            .collect();
        for unit in due {
            let (properties, first, _) = pending[&unit];
            let applied = match &manager {
                Manager::Fixture(root) => fixture::set(root, &unit, properties),
                Manager::User => {
                    if connection.is_none() {
                        connection = user_bus().ok();
                    }
                    match &connection {
                        Some(connection) => user::set(connection, &unit, properties),
                        None => Err(failed("no user bus")),
                    }
                }
            };
            match applied {
                Ok(true) => {
                    pending.remove(&unit);
                }
                Ok(false) if first.elapsed() < RETRY_FOR => {
                    if let Some(entry) = pending.get_mut(&unit) {
                        entry.2 = now + RETRY;
                    }
                }
                Ok(false) => {
                    pending.remove(&unit);
                }
                Err(error) => {
                    if !warned {
                        eprintln!("[scopes] {}: {error}", unit.name);
                        warned = true;
                    }
                    pending.remove(&unit);
                    // A connection that failed is made again at the next change.
                    if matches!(manager, Manager::User) {
                        connection = None;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
