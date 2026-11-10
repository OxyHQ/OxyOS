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
//! [`follow`] waits on logind's signals (seat0's properties, users appearing and going) and looks
//! again at each; nothing wakes it otherwise. [`apply_to_tree`] gives the same weights to a
//! control-group tree of slices, the saturation benchmark's (`perf.py saturation`, S7), so the
//! benchmark measures this policy, not a copy of it.

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

/// Looks at the seat and every user's slice and changes what [`plan`] says; `also` (the holder
/// before) is looked at though logind may no longer list it. Returns the seat's holder.
pub fn reconcile(
    bus: &Connection,
    also: Option<u32>,
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

/// Follows the seat on `bus` (logind's and systemd's): weighs the slices at once, then again at
/// each signal of seat0's properties and of users appearing and going. Returns when the bus
/// closes, with why.
pub fn follow(bus: &Connection, log: &mut dyn FnMut(String)) -> Result<(), String> {
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
    let mut changes = stream(rule(
        SEAT0,
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
    ))?
    .or(stream(rule(LOGIND_PATH, LOGIND_MANAGER, "UserNew"))?)
    .or(stream(rule(LOGIND_PATH, LOGIND_MANAGER, "UserRemoved"))?);
    let mut holder = reconcile(bus, None, log)?;
    loop {
        match futures_lite::future::block_on(changes.next()) {
            Some(Ok(_)) => match reconcile(bus, holder, log) {
                Ok(now) => holder = now,
                // logind or systemd not answering this once: the next signal looks again.
                Err(error) => log(format!("the seat not looked at: {error}")),
            },
            Some(Err(error)) => log(format!("logind's signals: {error}")),
            None => return Err("the system bus closed".into()),
        }
    }
}

/// The service's thread: follows the seat on the system bus, connecting again after a wait that
/// doubles to ten minutes while the bus, logind or systemd cannot be reached.
pub fn run(mut log: impl FnMut(String)) {
    let mut wait = std::time::Duration::from_secs(1);
    loop {
        let outcome = oxyos_seat::system_bus()
            .map_err(|error| error.to_string())
            .and_then(|bus| follow(&bus, &mut log));
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

#[cfg(test)]
mod tests;
