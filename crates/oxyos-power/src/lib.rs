//! Battery and energy in an OxyOS session (docs/adr/0057-battery-and-energy.mdx): what the session
//! asks of the system's own power services and what it keeps itself.
//!
//! - [`upower`]: the battery's health (its maximum capacity, its cycles), its charge limit (UPower's
//!   charge threshold, which UPower writes to the battery's sysfs as root once polkit allows it)
//!   and what UPower does when the battery runs out (its critical action);
//! - [`profiles`]: Low Power Mode, a hold of power-profiles-daemon's `power-saver` profile, released
//!   when the session no longer holds it (or goes);
//! - [`sleep`]: the machine put to sleep through logind, which applies its inhibitors;
//! - [`history`]: the charge and the display's use over the last ten days, recorded as UPower and
//!   the session say they change, never on a clock;
//! - [`energy`]: the processor's and the GPU's time each application used, from `/proc`;
//! - [`follow`]: power-profiles-daemon followed, so a hold lost with it is held again;
//! - [`LowPower`]: Low Power Mode as the session's parts obey it;
//! - [`played`]: UPower, power-profiles-daemon and logind played on a bus of a test's own (never
//!   the system's), for tests and the development compositor.
//!
//! Every call may wait on another process, so the session makes them off its compositor's turn.

use std::io;
use std::io::Write;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use zbus::blocking::Connection;

pub mod energy;
pub mod follow;
pub mod history;
pub mod played;
pub mod profiles;
pub mod sleep;
#[cfg(test)]
mod tests;
pub mod upower;

/// A connection to the bus the services are on ([`connect`]).
pub type Bus = Connection;

pub(crate) fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

/// Whether a call failed because nobody provides the service: not an error, a machine without
/// it.
pub(crate) fn absent(error: &zbus::Error) -> bool {
    let unknown = |name: &str| {
        matches!(
            name,
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.UnknownMethod"
        )
    };
    match error {
        zbus::Error::MethodError(name, _, _) => unknown(name.as_str()),
        zbus::Error::FDO(error) => matches!(
            **error,
            zbus::fdo::Error::ServiceUnknown(_)
                | zbus::fdo::Error::NameHasNoOwner(_)
                | zbus::fdo::Error::UnknownObject(_)
                | zbus::fdo::Error::UnknownMethod(_)
        ),
        _ => false,
    }
}

/// Whether polkit (or the service) refused the call: the person may not do it.
pub(crate) fn refused(error: &zbus::Error) -> bool {
    let denied = |name: &str| {
        name == "org.freedesktop.DBus.Error.AccessDenied"
            || name.ends_with(".PermissionDenied")
            || name.ends_with(".NotAuthorized")
            || name.ends_with(".AccessDenied")
    };
    match error {
        zbus::Error::MethodError(name, _, _) => denied(name.as_str()),
        zbus::Error::FDO(error) => matches!(**error, zbus::fdo::Error::AccessDenied(_)),
        _ => false,
    }
}

/// A proxy that asks every time: what it reads is what the service says now.
pub(crate) fn proxy<'a>(
    bus: &Connection,
    destination: &'static str,
    path: impl Into<String>,
    interface: &'static str,
) -> zbus::Result<zbus::blocking::Proxy<'a>> {
    zbus::blocking::proxy::Builder::<zbus::blocking::Proxy<'a>>::new(bus)
        .destination(destination)?
        .path(path.into())?
        .interface(interface)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

/// The system bus, as the session keeps a connection (its executor on a thread of its own that
/// wakes for nothing, [`oxyos_seat::system_bus`]), or the bus at `address` (a test's, the
/// development compositor's played services).
pub fn connect(address: Option<&str>) -> io::Result<Connection> {
    let Some(address) = address else {
        return oxyos_seat::system_bus();
    };
    zbus::blocking::connection::Builder::address(address)
        .and_then(|builder| builder.build())
        .map_err(failed)
}

/// Why a change the person asked for was not made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The machine cannot (no battery, no charge limit, no power-profiles-daemon).
    Unsupported,
    /// polkit refused it.
    NotPermitted,
    /// The service is not there.
    Unavailable,
    Failed,
}

impl Failure {
    pub fn name(self) -> &'static str {
        match self {
            Failure::Unsupported => "unsupported",
            Failure::NotPermitted => "notPermitted",
            Failure::Unavailable => "unavailable",
            Failure::Failed => "failed",
        }
    }

    pub(crate) fn of(error: &zbus::Error) -> Self {
        if absent(error) {
            Failure::Unavailable
        } else if refused(error) {
            Failure::NotPermitted
        } else {
            Failure::Failed
        }
    }
}

/// Low Power Mode as the session's parts obey it (docs/adr/0057-battery-and-energy.mdx, decision
/// 4): whether it is on now, and a descriptor per part that becomes readable whenever it changes,
/// so a thread waiting on its own work (the content indexer) waits on it too, never on a clock.
/// Clones share it; the session sets it, every part reads it.
#[derive(Clone, Default)]
pub struct LowPower {
    inner: Arc<LowPowerInner>,
}

#[derive(Default)]
struct LowPowerInner {
    on: AtomicBool,
    told: Mutex<Vec<UnixStream>>,
}

/// What a part waits on to learn Low Power Mode changed: readable once it did, until read
/// ([`Changes::take`]).
pub struct Changes {
    read: UnixStream,
}

impl Changes {
    /// Empties it: whether it changed since the last call.
    pub fn take(&self) -> bool {
        let mut buffer = [0u8; 16];
        let mut changed = false;
        while matches!(std::io::Read::read(&mut &self.read, &mut buffer), Ok(n) if n > 0) {
            changed = true;
        }
        changed
    }
}

impl AsFd for Changes {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.read.as_fd()
    }
}

impl LowPower {
    /// Whether Low Power Mode is on: work that can wait (indexing files, checking for updates)
    /// waits, and what is drawn goes at most 60 times a second.
    pub fn on(&self) -> bool {
        self.inner.on.load(Ordering::Acquire)
    }

    /// The session's: on or off. Every part waiting is told when it changes.
    pub fn set(&self, on: bool) {
        if self.inner.on.swap(on, Ordering::AcqRel) == on {
            return;
        }
        let mut told = self.inner.told.lock().expect("Low Power Mode's parts");
        // A part that went (its end dropped) is told no more.
        told.retain(|write| match (&*write).write(&[1]) {
            Ok(_) => true,
            Err(error) => error.kind() == io::ErrorKind::WouldBlock,
        });
    }

    /// What a part waits on to learn it changed.
    pub fn changes(&self) -> io::Result<Changes> {
        let (read, write) = UnixStream::pair()?;
        read.set_nonblocking(true)?;
        write.set_nonblocking(true)?;
        self.inner
            .told
            .lock()
            .expect("Low Power Mode's parts")
            .push(write);
        Ok(Changes { read })
    }
}
