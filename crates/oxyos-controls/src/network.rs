//! The network, from NetworkManager over the system bus (docs/adr/0025-hardware-keys-and-control-center.mdx):
//! Debian's desktops and OxyOS's image run it, and it is what turns Wi-Fi on and off
//! (`WirelessEnabled`, which polkit gives the user of the active local session without asking,
//! as `nmcli radio wifi off` does). Without it there is no Wi-Fi to show.

use crate::bus::{absent, failed, proxy};
use crate::Network;
use oxyos_protocol::WifiState;
use std::io;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
/// NetworkManager's device types.
const TYPE_ETHERNET: u32 = 1;
const TYPE_WIFI: u32 = 2;
/// NetworkManager's device states: up, and the ones on the way to it.
const ACTIVATED: u32 = 100;
const PREPARE: u32 = 40;

/// Wi-Fi's state from what NetworkManager says: whether Wi-Fi is enabled (in software and by the
/// hardware's switch) and the Wi-Fi device's state, if there is one.
pub fn wifi_state(enabled: bool, hardware_enabled: bool, device: Option<u32>) -> WifiState {
    match device {
        None => WifiState::Unavailable,
        Some(_) if !enabled || !hardware_enabled => WifiState::Off,
        Some(ACTIVATED) => WifiState::Connected,
        Some(state) if (PREPARE..ACTIVATED).contains(&state) => WifiState::Connecting,
        Some(_) => WifiState::Disconnected,
    }
}

/// An access point's name as text: its bytes, which need not be UTF-8, with what is not
/// replaced.
pub fn ssid_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The network now, from NetworkManager on `bus`.
pub fn read(bus: &Connection) -> io::Result<Network> {
    let nm = proxy(bus, NM, NM_PATH.to_string(), NM).map_err(failed)?;
    let enabled: bool = match nm.get_property("WirelessEnabled") {
        Ok(enabled) => enabled,
        Err(error) if absent(&error) => return Ok(Network::UNAVAILABLE),
        Err(error) => return Err(failed(error)),
    };
    let hardware: bool = nm.get_property("WirelessHardwareEnabled").map_err(failed)?;
    let devices: Vec<OwnedObjectPath> = nm.get_property("Devices").map_err(failed)?;
    let mut wifi: Option<(u32, OwnedObjectPath)> = None;
    let mut wired = false;
    for path in devices {
        let device = proxy(bus, NM, path.to_string(), DEVICE).map_err(failed)?;
        let kind: u32 = device.get_property("DeviceType").map_err(failed)?;
        let state: u32 = device.get_property("State").map_err(failed)?;
        match kind {
            TYPE_ETHERNET => wired |= state == ACTIVATED,
            // The first Wi-Fi device, or the one that is up.
            TYPE_WIFI if wifi.is_none() || state == ACTIVATED => wifi = Some((state, path)),
            _ => {}
        }
    }
    let state = wifi_state(enabled, hardware, wifi.as_ref().map(|(state, _)| *state));
    let (mut ssid, mut signal) = (String::new(), 0);
    if let (Some((_, path)), WifiState::Connected | WifiState::Connecting) = (&wifi, state) {
        let wireless = proxy(
            bus,
            NM,
            path.to_string(),
            "org.freedesktop.NetworkManager.Device.Wireless",
        )
        .map_err(failed)?;
        let point: OwnedObjectPath = wireless.get_property("ActiveAccessPoint").map_err(failed)?;
        if point.as_str() != "/" {
            let point = proxy(
                bus,
                NM,
                point.to_string(),
                "org.freedesktop.NetworkManager.AccessPoint",
            )
            .map_err(failed)?;
            let bytes: Vec<u8> = point.get_property("Ssid").map_err(failed)?;
            ssid = ssid_text(&bytes);
            signal = point
                .get_property::<u8>("Strength")
                .map_err(failed)?
                .min(100);
        }
    }
    Ok(Network {
        wifi: state,
        signal,
        ssid,
        wired,
    })
}

/// Turns Wi-Fi on or off, as NetworkManager's own `nmcli radio wifi` does.
pub fn set_wifi(bus: &Connection, enabled: bool) -> io::Result<()> {
    let nm = proxy(bus, NM, NM_PATH.to_string(), NM).map_err(failed)?;
    nm.set_property("WirelessEnabled", enabled).map_err(failed)
}
