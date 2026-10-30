//! The system bus, as UPower, NetworkManager and BlueZ are read from it.

use std::io;
use zbus::blocking::{Connection, Proxy};

/// A proxy that asks every time: what it reads is what the service says now, and nothing runs
/// in the background to keep a copy.
pub(crate) fn proxy(
    bus: &Connection,
    destination: &'static str,
    path: String,
    interface: &'static str,
) -> zbus::Result<Proxy<'static>> {
    zbus::blocking::proxy::Builder::<Proxy<'static>>::new(bus)
        .destination(destination)?
        .path(path)?
        .interface(interface)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

/// Whether a call failed because nobody provides the service: not an error, a machine without
/// it.
pub(crate) fn absent(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, _, _) => unknown(name.as_str()),
        zbus::Error::FDO(error) => matches!(
            **error,
            zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)
        ),
        _ => false,
    }
}

fn unknown(name: &str) -> bool {
    matches!(
        name,
        "org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner"
    )
}

pub(crate) fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
