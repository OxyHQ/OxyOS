//! Restarting and shutting the machine down, and its power key, for the session
//! (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx): logind's `Reboot` and `PowerOff`, never
//! interactive, as the active session's user (polkit's `org.freedesktop.login1.reboot` and
//! `power-off`); `CanReboot` and `CanPowerOff` asked first, and anything but `yes` refused without
//! asking (`challenge`: another person's session is open or a program's block inhibitor holds the
//! machine, which only an administrator may override). The power key: logind's `handle-power-key`
//! block inhibitor, held for the session's life so logind never turns the machine off at a press,
//! and its `HandlePowerKey`, what the session then does with one.

use crate::sleep::{LOGIND, MANAGER, MANAGER_PATH};
use crate::{failed, proxy, Failure};
use std::io;
use std::os::fd::OwnedFd;
use zbus::blocking::Connection;

/// How the machine ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Restart,
    ShutDown,
}

impl End {
    fn methods(self) -> (&'static str, &'static str) {
        match self {
            End::Restart => ("CanReboot", "Reboot"),
            End::ShutDown => ("CanPowerOff", "PowerOff"),
        }
    }
}

/// What logind's `Can…` answers means for a call that asks no one.
pub fn verdict(answer: &str) -> Result<(), Failure> {
    match answer {
        "yes" => Ok(()),
        // The machine cannot at all.
        "na" => Err(Failure::Unsupported),
        // `no`, and `challenge`: polkit would ask an administrator, and nobody is asked.
        _ => Err(Failure::NotPermitted),
    }
}

/// Whether the session's user may end the machine so now, as logind says.
pub fn can(bus: &Connection, end: End) -> Result<(), Failure> {
    let manager = proxy(bus, LOGIND, MANAGER_PATH, MANAGER).map_err(|error| Failure::of(&error))?;
    let answer: String = manager
        .call(end.methods().0, &())
        .map_err(|error| Failure::of(&error))?;
    verdict(&answer)
}

/// Asks logind to restart or shut the machine down, once it said the user may.
pub fn end(bus: &Connection, end: End) -> Result<(), Failure> {
    can(bus, end)?;
    let manager = proxy(bus, LOGIND, MANAGER_PATH, MANAGER).map_err(|error| Failure::of(&error))?;
    manager
        .call::<_, _, ()>(end.methods().1, &(false,))
        .map_err(|error| Failure::of(&error))
}

/// logind's `HandlePowerKey`: what it would do at a press, which the session follows itself.
pub fn handle_power_key(bus: &Connection) -> io::Result<String> {
    let manager = proxy(bus, LOGIND, MANAGER_PATH, MANAGER).map_err(failed)?;
    manager.get_property("HandlePowerKey").map_err(failed)
}

/// Holds logind's handling of the power key back until the descriptor is closed: a block
/// inhibitor of `handle-power-key`, which polkit gives the active session's user.
pub fn hold_power_key(bus: &Connection) -> io::Result<OwnedFd> {
    let manager = proxy(bus, LOGIND, MANAGER_PATH, MANAGER).map_err(failed)?;
    let fd: zbus::zvariant::OwnedFd = manager
        .call(
            "Inhibit",
            &(
                "handle-power-key",
                "OxyOS",
                "The power key asks what to do",
                "block",
            ),
        )
        .map_err(failed)?;
    Ok(fd.into())
}

/// What the session does at a press of the power key, from logind's `HandlePowerKey`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerKey {
    /// Restart, Sleep, Cancel or Shut Down, never an abrupt power-off: `poweroff` (logind's
    /// default), `reboot`, `halt`, `kexec` and anything else.
    Ask,
    /// `suspend`, `hibernate`, `hybrid-sleep`, `suspend-then-hibernate`, `sleep`.
    Sleep,
    /// `lock`.
    Lock,
    /// `ignore`.
    Ignore,
}

impl PowerKey {
    pub fn of(policy: &str) -> Self {
        match policy {
            "suspend" | "hibernate" | "hybrid-sleep" | "suspend-then-hibernate" | "sleep" => {
                PowerKey::Sleep
            }
            "lock" => PowerKey::Lock,
            "ignore" => PowerKey::Ignore,
            _ => PowerKey::Ask,
        }
    }
}
