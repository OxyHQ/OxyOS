//! Low Power Mode through power-profiles-daemon (docs/adr/0057-battery-and-energy.mdx, decision
//! 3): while it is on, the session holds the daemon's `power-saver` profile (`HoldProfile`), which
//! the daemon applies through whatever the machine has (the platform's profile, the processor's
//! energy preference, its own actions) and releases on `ReleaseProfile` or when the connection that
//! holds it goes, so a session that ends or fails leaves the machine as the person set it. A hold
//! is never the person's choice of profile (`ActiveProfile`, which the daemon keeps across boots):
//! Low Power Mode is the session's, as the person's preference says.
//!
//! polkit's `org.freedesktop.UPower.PowerProfiles.hold-profile` lets the active session's user
//! hold one without a password.

use crate::{absent, failed, proxy, Failure};
use std::collections::HashMap;
use std::io;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

pub const DAEMON: &str = "org.freedesktop.UPower.PowerProfiles";
pub const PATH: &str = "/org/freedesktop/UPower/PowerProfiles";
pub const POWER_SAVER: &str = "power-saver";
/// Who holds it, as the daemon lists its holds.
pub const APPLICATION: &str = "so.oxy.oxyos";
pub const REASON: &str = "Low Power Mode";

/// Whether the daemon is there and has the power-saver profile.
pub fn available(bus: &Connection) -> io::Result<bool> {
    let daemon = proxy(bus, DAEMON, PATH, DAEMON).map_err(failed)?;
    match daemon.get_property::<Vec<HashMap<String, OwnedValue>>>("Profiles") {
        Ok(profiles) => Ok(profiles.iter().any(|profile| {
            profile
                .get("Profile")
                .and_then(|name| <&str>::try_from(name).ok())
                == Some(POWER_SAVER)
        })),
        Err(error) if absent(&error) => Ok(false),
        Err(zbus::Error::FDO(error))
            if matches!(
                *error,
                zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::UnknownProperty(_)
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(failed(error)),
    }
}

/// The daemon's active profile now (`power-saver` while held), none without the daemon.
pub fn active(bus: &Connection) -> io::Result<Option<String>> {
    let daemon = proxy(bus, DAEMON, PATH, DAEMON).map_err(failed)?;
    match daemon.get_property::<String>("ActiveProfile") {
        Ok(profile) => Ok(Some(profile)),
        Err(error) if absent(&error) => Ok(None),
        Err(zbus::Error::FDO(error)) if matches!(*error, zbus::fdo::Error::ServiceUnknown(_)) => {
            Ok(None)
        }
        Err(error) => Err(failed(error)),
    }
}

/// A hold of the power-saver profile, on the connection that took it: the daemon releases it when
/// [`Hold::release`] is called or the connection goes.
#[derive(Debug)]
pub struct Hold {
    cookie: u32,
}

impl Hold {
    /// Holds power-saver on `bus`.
    pub fn take(bus: &Connection) -> Result<Self, Failure> {
        if !available(bus).map_err(|_| Failure::Unavailable)? {
            return Err(Failure::Unsupported);
        }
        let daemon = proxy(bus, DAEMON, PATH, DAEMON).map_err(|error| Failure::of(&error))?;
        let cookie: u32 = daemon
            .call("HoldProfile", &(POWER_SAVER, REASON, APPLICATION))
            .map_err(|error| Failure::of(&error))?;
        Ok(Self { cookie })
    }

    /// The daemon's number for it (its `ProfileReleased` names it).
    pub fn cookie(&self) -> u32 {
        self.cookie
    }

    /// Released; a hold the daemon released already (another program chose a profile, the daemon
    /// restarted) is nothing to release.
    pub fn release(self, bus: &Connection) {
        if let Ok(daemon) = proxy(bus, DAEMON, PATH, DAEMON) {
            let _ = daemon.call::<_, _, ()>("ReleaseProfile", &(self.cookie,));
        }
    }
}
