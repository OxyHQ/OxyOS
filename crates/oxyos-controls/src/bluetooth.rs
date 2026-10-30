//! Bluetooth, from BlueZ over the system bus (docs/adr/0025-hardware-keys-and-control-center.mdx):
//! whether there is an adapter, whether it is powered, and how many devices are connected. Only
//! looked at: pairing and turning it on and off are a decision of their own.

use crate::bus::{absent, failed, proxy};
use crate::Bluetooth;
use oxyos_protocol::BluetoothState;
use std::collections::HashMap;
use std::io;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

type Interfaces = HashMap<String, HashMap<String, OwnedValue>>;

/// Bluetooth from what BlueZ says: each adapter's `Powered`, and how many devices are connected.
pub fn status(adapters: &[bool], connected: usize) -> Bluetooth {
    let state = if adapters.is_empty() {
        BluetoothState::Unavailable
    } else if adapters.iter().any(|powered| *powered) {
        BluetoothState::On
    } else {
        BluetoothState::Off
    };
    Bluetooth {
        state,
        connected: if state == BluetoothState::On {
            connected.min(usize::from(u8::MAX)) as u8
        } else {
            0
        },
    }
}

fn flag(properties: &HashMap<String, OwnedValue>, name: &str) -> bool {
    properties
        .get(name)
        .and_then(|value| bool::try_from(value).ok())
        .unwrap_or(false)
}

/// Bluetooth now, from BlueZ on `bus`.
pub fn read(bus: &Connection) -> io::Result<Bluetooth> {
    let manager = proxy(
        bus,
        "org.bluez",
        "/".to_string(),
        "org.freedesktop.DBus.ObjectManager",
    )
    .map_err(failed)?;
    let objects: HashMap<OwnedObjectPath, Interfaces> = match manager.call("GetManagedObjects", &())
    {
        Ok(objects) => objects,
        Err(error) if absent(&error) => return Ok(Bluetooth::UNAVAILABLE),
        Err(error) => return Err(failed(error)),
    };
    let mut adapters = Vec::new();
    let mut connected = 0;
    for interfaces in objects.values() {
        if let Some(adapter) = interfaces.get("org.bluez.Adapter1") {
            adapters.push(flag(adapter, "Powered"));
        }
        if let Some(device) = interfaces.get("org.bluez.Device1") {
            connected += usize::from(flag(device, "Connected"));
        }
    }
    Ok(status(&adapters, connected))
}
