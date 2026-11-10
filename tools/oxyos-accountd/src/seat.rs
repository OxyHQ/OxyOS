//! The person at the seat outranks other users ([ADR 0066](../../../docs/adr/0066-quality-of-service.mdx),
//! decision 8).
//!
//! The user slice (`user-UID.slice`) of whoever holds seat0 weighs [`SEAT_WEIGHT`] (its CPU and
//! I/O weight, 400) against every other user's [`DEFAULT_WEIGHT`] (100, what systemd gives a slice
//! that sets none), as a Mac keeps other users and its daemons below the person in front, and as
//! Fedora's uresourced does. Who holds the seat is logind's active session on seat0, whatever its
//! class: the person signed in, or the login screen's user while it shows (someone is typing a
//! password there).
//! - **Fast user switching:** the seat's active session changes; the new holder's slice is raised
//!   and the one before set back to its default at once (its applications go on running, at the
//!   weight of any other user's).
//! - **Locked:** the session is still the seat's active one (its lock screen is its own): nothing
//!   changes. A terminal with no session switched to leaves nobody raised.
//! - **The same user signed in elsewhere** (ssh) shares the slice: the slice is the user's.
//! - **The weights are runtime properties** (`SetUnitProperties`, runtime), which systemd keeps
//!   until the machine stops, written for the seat's holder and for a holder leaving the seat
//!   (`CPUWeight=`, `IOWeight=`: the default again), never for anyone else. A slice left raised
//!   (the service was not running when the seat changed, or the slice was gone before it could be
//!   lowered and is made again with what it was given) is lowered when the service starts and
//!   whenever its user appears; a user slice weighing [`SEAT_WEIGHT`] off the seat is taken for
//!   one this service raised.
//! - The I/O weight counts where the disk's controller weighs groups (`io.cost`, BFQ); elsewhere it
//!   is kept for when it does.
//!
//! **The seat's applications' memory** is carried up the same way. The session protects its focused
//! application's working set with `MemoryLow` on the application's scope, and gives its user
//! manager's `app.slice` the sum of those claims (decision 4); with cgroup2's
//! `memory_recursiveprot` a protection reaches a group only as far as each ancestor's covers it, and
//! those ancestors are system units only root sets. The session tells this service its claim over
//! its registrar's connection (`oxyos_account::session::MemoryClaim`), and for the seat's holder
//! alone the service sets `user@UID.service` and `user-UID.slice` to their floor and the claim, and
//! `user.slice` to its floor and the claim ([`lows`]), capped at [`CLAIM_CAP_PERCENT`] of the
//! machine's memory. The floors are the image's own (`/usr/lib/systemd/system/*/50-oxyos-memory.conf`,
//! decision 7), sized for the session's services alone; a static raise would hand what goes
//! unclaimed to the largest unprotected user (the protection a parent's children do not claim is
//! shared out by usage). Another user's claim is kept and given while that user holds the seat; a
//! claim ends with its session's connection; leaving the seat, or ending, sets the floors back.
//!
//! [`follow`] waits on logind's signals (seat0's properties, users appearing and going) and on the
//! sessions' claims, and looks again at each; nothing wakes it otherwise. [`apply_to_tree`] gives the same weights to a
//! control-group tree of slices, the saturation benchmark's (`perf.py saturation`, S7), so the
//! benchmark measures this policy, not a copy of it; [`apply_memory_claim_to_tree`] gives it the
//! memory protections a claim carries up (S6).

use oxyos_account::session::MemoryClaim;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, Value};

/// The CPU and I/O weight of the seat's holder's slice.
pub const SEAT_WEIGHT: u64 = 400;
/// The weight of every other user's slice: cgroup v2's default, which systemd gives a slice that
/// sets none.
pub const DEFAULT_WEIGHT: u64 = 100;
/// systemd's weight that is not set (`CGROUP_WEIGHT_INVALID`): written, the slice's own default
/// again; read, none was set.
const UNSET: u64 = u64::MAX;

const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";
const SEAT0: &str = "/org/freedesktop/login1/seat/seat0";
const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";

/// The weight a user's slice is given: [`SEAT_WEIGHT`] when its user holds the seat, else the
/// default.
pub fn weight(at_seat: bool) -> u64 {
    if at_seat {
        SEAT_WEIGHT
    } else {
        DEFAULT_WEIGHT
    }
}

/// User `uid`'s slice, as logind names it.
pub fn slice(uid: u32) -> String {
    format!("user-{uid}.slice")
}

/// The protections the image gives a user's manager (`user@.service`), a user's slice
/// (`user-.slice`) and the users' slice (`user.slice`), `MemoryLow`, before any claim: decision 7's
/// drop-ins, sized for the session's own services (the user manager's `session.slice`), which a
/// test holds equal to them.
pub const FLOORS: Lows = Lows {
    manager: 128 << 20,
    user: 128 << 20,
    users: 256 << 20,
};
/// The most of the machine's memory a session's applications may have protected: what is left is
/// the rest of the machine's, never the seat's to claim.
pub const CLAIM_CAP_PERCENT: u64 = 50;

/// The `MemoryLow` of the seat's holder's user manager and slice, and of the users' slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lows {
    pub manager: u64,
    pub user: u64,
    pub users: u64,
}

/// The protections that carry a claim of `claim` bytes up from the seat's holder's `app.slice`, on
/// a machine of `memory` bytes: each floor and the claim (capped), the user's slice at least its
/// manager's.
pub fn lows(claim: u64, memory: u64) -> Lows {
    let claim = claim.min(memory / 100 * CLAIM_CAP_PERCENT);
    let manager = FLOORS.manager.saturating_add(claim);
    Lows {
        manager,
        user: FLOORS.user.saturating_add(claim).max(manager),
        users: FLOORS.users.saturating_add(claim),
    }
}

/// What to write to a protection that is `current` (`None`: the unit is not there) and should be
/// `wanted` over its `floor`: a raise whenever it differs, the floor again only where it stands
/// above it (a raise given before), nothing else: a unit nobody raised keeps what it has.
pub fn settle(current: Option<u64>, wanted: u64, floor: u64) -> Option<u64> {
    let current = current?;
    if current == wanted {
        None
    } else if wanted > floor {
        Some(wanted)
    } else {
        (current > floor).then_some(floor)
    }
}

/// The sessions' claims, by their registrar's connection: its user and its bytes.
pub type Claims = BTreeMap<u64, (u32, u64)>;

/// `claim` taken into `claims`: its bytes kept, or its end forgotten.
pub fn take(claims: &mut Claims, claim: MemoryClaim) {
    match claim.bytes {
        Some(bytes) => {
            claims.insert(claim.registrar, (claim.uid, bytes));
        }
        None => {
            claims.remove(&claim.registrar);
        }
    }
}

/// User `uid`'s claim: the largest of its sessions'.
pub fn claim_of(claims: &Claims, uid: u32) -> u64 {
    claims
        .values()
        .filter(|(user, _)| *user == uid)
        .map(|(_, bytes)| *bytes)
        .max()
        .unwrap_or(0)
}

/// The machine's memory, in bytes.
pub fn machine_memory() -> u64 {
    let info = rustix::system::sysinfo();
    u64::from(info.totalram).saturating_mul(u64::from(info.mem_unit.max(1)))
}

/// A user slice's weights as configured: `None` for one not set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Weights {
    pub cpu: Option<u64>,
    pub io: Option<u64>,
}

/// What [`plan`] changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The seat's holder's slice to [`SEAT_WEIGHT`].
    Raise(u32),
    /// A slice raised off the seat back to its default.
    Lower(u32),
}

/// What to change, given who holds the seat and each user's slice's weights now (a user without a
/// slice now is left out): the holder raised unless it is, and any other slice weighing
/// [`SEAT_WEIGHT`] lowered. Nothing else is written.
pub fn plan(seat: Option<u32>, slices: &BTreeMap<u32, Weights>) -> Vec<Change> {
    let raised =
        |weights: &Weights| weights.cpu == Some(SEAT_WEIGHT) || weights.io == Some(SEAT_WEIGHT);
    slices
        .iter()
        .filter_map(|(&uid, weights)| {
            if Some(uid) == seat {
                let full = weights.cpu == Some(SEAT_WEIGHT) && weights.io == Some(SEAT_WEIGHT);
                (!full).then_some(Change::Raise(uid))
            } else {
                raised(weights).then_some(Change::Lower(uid))
            }
        })
        .collect()
}

fn failed(error: zbus::Error) -> String {
    error.to_string()
}

fn proxy<'a>(
    bus: &Connection,
    destination: &'a str,
    path: &'a str,
    interface: &'a str,
) -> zbus::Result<zbus::blocking::Proxy<'a>> {
    // Read each time, never from a cache: a proxy that caches subscribes to its own signals.
    zbus::blocking::proxy::Builder::new(bus)
        .destination(destination)?
        .path(path)?
        .interface(interface)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

/// The user of seat0's active session, if it has one.
pub fn seat_holder(bus: &Connection) -> Result<Option<u32>, String> {
    let seat = proxy(bus, LOGIND, SEAT0, "org.freedesktop.login1.Seat").map_err(failed)?;
    let (_, session): (String, OwnedObjectPath) =
        seat.get_property("ActiveSession").map_err(failed)?;
    if session.as_str() == "/" {
        return Ok(None);
    }
    let session = proxy(
        bus,
        LOGIND,
        session.as_str(),
        "org.freedesktop.login1.Session",
    )
    .map_err(failed)?;
    let (uid, _): (u32, OwnedObjectPath) = session.get_property("User").map_err(failed)?;
    Ok(Some(uid))
}

/// Every user logind knows (signed in somewhere, or lingering).
fn users(bus: &Connection) -> Result<Vec<u32>, String> {
    let manager = proxy(bus, LOGIND, LOGIND_PATH, LOGIND_MANAGER).map_err(failed)?;
    let users: Vec<(u32, String, OwnedObjectPath)> =
        manager.call("ListUsers", &()).map_err(failed)?;
    Ok(users.into_iter().map(|(uid, _, _)| uid).collect())
}

/// User `uid`'s slice's weights as systemd has them set, `None` without a slice now.
fn slice_weights(bus: &Connection, uid: u32) -> Result<Option<Weights>, String> {
    let manager = proxy(bus, SYSTEMD, SYSTEMD_PATH, SYSTEMD_MANAGER).map_err(failed)?;
    let unit: OwnedObjectPath = match manager.call("GetUnit", &(slice(uid))) {
        Ok(unit) => unit,
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit" =>
        {
            return Ok(None)
        }
        Err(error) => return Err(failed(error)),
    };
    let unit = proxy(
        bus,
        SYSTEMD,
        unit.as_str(),
        "org.freedesktop.systemd1.Slice",
    )
    .map_err(failed)?;
    let read = |name: &str| -> Result<Option<u64>, String> {
        let value: u64 = unit.get_property(name).map_err(failed)?;
        Ok((value != UNSET).then_some(value))
    };
    Ok(Some(Weights {
        cpu: read("CPUWeight")?,
        io: read("IOWeight")?,
    }))
}

/// Sets user `uid`'s slice's CPU and I/O weight for this boot (`UNSET`: its default again).
fn set(bus: &Connection, uid: u32, weight: u64) -> Result<(), String> {
    let manager = proxy(bus, SYSTEMD, SYSTEMD_PATH, SYSTEMD_MANAGER).map_err(failed)?;
    let properties = vec![
        ("CPUWeight", Value::U64(weight)),
        ("IOWeight", Value::U64(weight)),
    ];
    manager
        .call::<_, _, ()>("SetUnitProperties", &(slice(uid), true, properties))
        .map_err(failed)
}

/// `unit`'s `MemoryLow` as systemd has it, read on its `interface` (`Slice`, `Service`); `None`
/// without the unit now.
fn memory_low(bus: &Connection, unit: &str, interface: &str) -> Result<Option<u64>, String> {
    let manager = proxy(bus, SYSTEMD, SYSTEMD_PATH, SYSTEMD_MANAGER).map_err(failed)?;
    let path: OwnedObjectPath = match manager.call("GetUnit", &(unit)) {
        Ok(path) => path,
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit" =>
        {
            return Ok(None)
        }
        Err(error) => return Err(failed(error)),
    };
    let interface = format!("org.freedesktop.systemd1.{interface}");
    let unit = proxy(bus, SYSTEMD, path.as_str(), &interface).map_err(failed)?;
    unit.get_property("MemoryLow").map(Some).map_err(failed)
}

/// Sets `unit`'s `MemoryLow` for this boot.
fn set_memory_low(bus: &Connection, unit: &str, bytes: u64) -> Result<(), String> {
    let manager = proxy(bus, SYSTEMD, SYSTEMD_PATH, SYSTEMD_MANAGER).map_err(failed)?;
    let properties = vec![("MemoryLow", Value::U64(bytes))];
    manager
        .call::<_, _, ()>("SetUnitProperties", &(unit, true, properties))
        .map_err(failed)
}

/// Carries the seat's holder's claim up its user manager, its slice and the users' slice, and sets
/// every other user's back to their floors where they were raised ([`settle`]).
fn protect(
    bus: &Connection,
    holder: Option<u32>,
    users: impl Iterator<Item = u32>,
    claims: &Claims,
    memory: u64,
    log: &mut dyn FnMut(String),
) -> Result<(), String> {
    let claim = holder.map_or(0, |uid| claim_of(claims, uid));
    let held = lows(claim, memory);
    let mut written = Vec::new();
    let mut write = |unit: String, interface: &str, wanted: u64, floor: u64| -> Result<(), String> {
        if let Some(bytes) = settle(memory_low(bus, &unit, interface)?, wanted, floor) {
            match set_memory_low(bus, &unit, bytes) {
                Ok(()) => written.push(format!("{unit} {} MiB", bytes >> 20)),
                Err(error) => log(format!("{unit}'s protection not set: {error}")),
            }
        }
        Ok(())
    };
    for uid in users {
        let (manager, user) = if Some(uid) == holder {
            (held.manager, held.user)
        } else {
            (FLOORS.manager, FLOORS.user)
        };
        write(format!("user@{uid}.service"), "Service", manager, FLOORS.manager)?;
        write(slice(uid), "Slice", user, FLOORS.user)?;
    }
    write("user.slice".into(), "Slice", held.users, FLOORS.users)?;
    if !written.is_empty() {
        let whose = holder.map_or("nobody's".into(), |uid| format!("uid {uid}'s"));
        log(format!(
            "the seat's applications' memory ({whose}, {} MiB claimed): {}",
            claim >> 20,
            written.join(", ")
        ));
    }
    Ok(())
}

/// Looks at the seat and every user's slice and changes what [`plan`] says, then the memory
/// protections [`protect`] carries up for the holder's claim (`claims`, on a machine of
/// `memory` bytes); `also` (the holder before) is looked at though logind may no longer list it.
/// Returns the seat's holder.
pub fn reconcile(
    bus: &Connection,
    also: Option<u32>,
    claims: &Claims,
    memory: u64,
    log: &mut dyn FnMut(String),
) -> Result<Option<u32>, String> {
    let holder = seat_holder(bus)?;
    let mut slices = BTreeMap::new();
    for uid in users(bus)?.into_iter().chain(also).chain(holder) {
        if slices.contains_key(&uid) {
            continue;
        }
        if let Some(weights) = slice_weights(bus, uid)? {
            slices.insert(uid, weights);
        }
    }
    for change in plan(holder, &slices) {
        match change {
            Change::Raise(uid) => match set(bus, uid, SEAT_WEIGHT) {
                Ok(()) => log(format!(
                    "uid {uid} holds the seat: its slice weighs {SEAT_WEIGHT} (CPU and I/O)"
                )),
                Err(error) => log(format!(
                    "uid {uid} holds the seat; its slice not raised: {error}"
                )),
            },
            Change::Lower(uid) => match set(bus, uid, UNSET) {
                Ok(()) => log(format!(
                    "uid {uid} is not at the seat: its slice's weights back to the default"
                )),
                Err(error) => log(format!("uid {uid}'s slice not lowered: {error}")),
            },
        }
    }
    protect(bus, holder, slices.keys().copied(), claims, memory, log)?;
    Ok(holder)
}

/// A match rule for logind's signals.
fn rule(
    path: &'static str,
    interface: &'static str,
    member: &'static str,
) -> zbus::Result<zbus::MatchRule<'static>> {
    Ok(zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(LOGIND)?
        .path(path)?
        .interface(interface)?
        .member(member)?
        .build())
}

/// What woke [`follow`].
enum Woken {
    Bus(zbus::Result<zbus::Message>),
    Claim(MemoryClaim),
}

/// Follows the seat on `bus` (logind's and systemd's): weighs the slices and carries the holder's
/// claim up at once, then again at each signal of seat0's properties and of users appearing and
/// going, and at each claim `told` (kept in `claims`, which outlive a connection). Returns when
/// the bus closes, with why.
pub fn follow(
    bus: &Connection,
    told: &async_channel::Receiver<MemoryClaim>,
    claims: &mut Claims,
    memory: u64,
    log: &mut dyn FnMut(String),
) -> Result<(), String> {
    use futures_lite::StreamExt;
    // Listened to before the first look, so no change between them is missed.
    let stream =
        |rule: zbus::Result<zbus::MatchRule<'static>>| -> Result<zbus::MessageStream, String> {
            futures_lite::future::block_on(zbus::MessageStream::for_match_rule(
                rule.map_err(failed)?,
                bus.inner(),
                Some(16),
            ))
            .map_err(failed)
        };
    // Boxed: the claims' receiver is not Unpin.
    let mut changes = Box::pin(stream(rule(
        SEAT0,
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
    ))?
    .or(stream(rule(LOGIND_PATH, LOGIND_MANAGER, "UserNew"))?)
    .or(stream(rule(LOGIND_PATH, LOGIND_MANAGER, "UserRemoved"))?)
    .map(Woken::Bus)
    .or(told.clone().map(Woken::Claim)));
    let mut holder = reconcile(bus, None, claims, memory, log)?;
    loop {
        match futures_lite::future::block_on(changes.next()) {
            Some(Woken::Bus(Err(error))) => {
                log(format!("logind's signals: {error}"));
                continue;
            }
            Some(Woken::Bus(Ok(_))) => {}
            Some(Woken::Claim(claim)) => take(claims, claim),
            None => return Err("the system bus closed".into()),
        }
        match reconcile(bus, holder, claims, memory, log) {
            Ok(now) => holder = now,
            // logind or systemd not answering this once: the next signal or claim looks again.
            Err(error) => log(format!("the seat not looked at: {error}")),
        }
    }
}

/// The service's thread: follows the seat on the system bus and the sessions' claims `told`,
/// connecting again after a wait that doubles to ten minutes while the bus, logind or systemd
/// cannot be reached (the claims told meanwhile are kept).
pub fn run(told: async_channel::Receiver<MemoryClaim>, mut log: impl FnMut(String)) {
    let mut wait = std::time::Duration::from_secs(1);
    let mut claims = Claims::new();
    let memory = machine_memory();
    loop {
        let outcome = oxyos_seat::system_bus()
            .map_err(|error| error.to_string())
            .and_then(|bus| follow(&bus, &told, &mut claims, memory, &mut log));
        if let Err(error) = outcome {
            log(format!(
                "not following the seat: {error}; again in {} s",
                wait.as_secs()
            ));
        }
        std::thread::sleep(wait);
        wait = (wait * 2).min(std::time::Duration::from_secs(600));
    }
}

/// The weights [`weight`] gives, written to a tree of slices that is not systemd's: each
/// directory of `user_slices` named `*.slice` gets `cpu.weight` (and `io.weight` where the `io`
/// controller is on), [`SEAT_WEIGHT`] for `seat`, the default for the rest. The saturation
/// benchmark's (S7: the seat's user beside another user's build). Returns each slice and its
/// weight.
pub fn apply_to_tree(user_slices: &Path, seat: &str) -> io::Result<Vec<(String, u64)>> {
    let mut slices: Vec<String> = std::fs::read_dir(user_slices)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".slice"))
        .collect();
    slices.sort();
    if !slices.iter().any(|name| name == seat) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no {seat} in {}", user_slices.display()),
        ));
    }
    let mut applied = Vec::new();
    for name in slices {
        let weight = weight(name == seat);
        let directory = user_slices.join(&name);
        std::fs::write(directory.join("cpu.weight"), format!("{weight}\n"))?;
        if directory.join("io.weight").exists() {
            std::fs::write(directory.join("io.weight"), format!("default {weight}\n"))?;
        }
        applied.push((name, weight));
    }
    Ok(applied)
}

/// The protections [`lows`] gives a claim of `bytes`, written to a tree of slices that is not
/// systemd's: `memory.low` of `user_slices/seat/user@.service` (the manager), `user_slices/seat`
/// and `user_slices` itself (the users' slice). The machine's memory is the nearest `memory.max`
/// above them that is a number (the benchmark's machine), else the machine's. The saturation
/// benchmark's (S6, the seat's applications' claim). Returns each group, as its path from
/// `user_slices`' parent, and what it was given.
pub fn apply_memory_claim_to_tree(
    user_slices: &Path,
    seat: &str,
    bytes: u64,
) -> io::Result<Vec<(String, u64)>> {
    let seat_slice = user_slices.join(seat);
    let manager = seat_slice.join("user@.service");
    if !manager.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no {}", manager.display()),
        ));
    }
    let memory = user_slices
        .ancestors()
        .find_map(|group| {
            std::fs::read_to_string(group.join("memory.max"))
                .ok()?
                .trim()
                .parse::<u64>()
                .ok()
        })
        .unwrap_or_else(machine_memory);
    let lows = lows(bytes, memory);
    let users = user_slices
        .file_name()
        .map_or_else(|| "users".into(), |name| name.to_string_lossy().into_owned());
    let mut applied = Vec::new();
    for (group, name, low) in [
        (&manager, format!("{users}/{seat}/user@.service"), lows.manager),
        (&seat_slice, format!("{users}/{seat}"), lows.user),
        (&user_slices.to_path_buf(), users.clone(), lows.users),
    ] {
        std::fs::write(group.join("memory.low"), format!("{low}\n"))?;
        applied.push((name, low));
    }
    Ok(applied)
}

#[cfg(test)]
mod tests;
