//! The machine put to sleep by the session (docs/adr/0057-battery-and-energy.mdx, decision 6):
//! logind's `Suspend`, not interactive, as the active session's user (polkit's
//! `org.freedesktop.login1.suspend`). logind applies the inhibitors: a program's block inhibitor of
//! sleep refuses it, and a delay inhibitor (the session's own, so the lock is on screen first,
//! ADR 0010) holds it back.

use crate::{failed, proxy, Failure};
use std::io;
use zbus::blocking::Connection;

pub const LOGIND: &str = "org.freedesktop.login1";
pub const MANAGER_PATH: &str = "/org/freedesktop/login1";
pub const MANAGER: &str = "org.freedesktop.login1.Manager";

/// Whether the machine can sleep and the user may make it (`CanSuspend` says `yes`; `challenge`
/// would ask a password, which an idle machine has nobody to answer).
pub fn can_suspend(bus: &Connection) -> io::Result<bool> {
    let manager = proxy(bus, LOGIND, MANAGER_PATH, MANAGER).map_err(failed)?;
    let answer: String = manager.call("CanSuspend", &()).map_err(failed)?;
    Ok(answer == "yes")
}

/// Asks logind to suspend the machine.
pub fn suspend(bus: &Connection) -> Result<(), Failure> {
    let manager = proxy(bus, LOGIND, MANAGER_PATH, MANAGER).map_err(|error| Failure::of(&error))?;
    match can_suspend(bus) {
        Ok(true) => {}
        Ok(false) => return Err(Failure::NotPermitted),
        Err(_) => return Err(Failure::Unavailable),
    }
    manager
        .call::<_, _, ()>("Suspend", &(false,))
        .map_err(|error| Failure::of(&error))
}
