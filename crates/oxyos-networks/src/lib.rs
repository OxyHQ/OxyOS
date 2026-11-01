//! The networks a session sees and joins (docs/adr/0038-networks.mdx), through NetworkManager,
//! the system's network daemon, on the system bus as the session's user: polkit decides each call.
//! Not to be confused with `oxyos-net`, the HTTP applications speak.
//!
//! [`NetworkManager`] reads the machine's state ([`State`]: Wi-Fi, its networks nearby and the
//! saved ones, Ethernet, the IP configuration of each), scans, joins a network by its name (with a
//! password the daemon keeps, never this process), forgets a saved network and turns Wi-Fi on and
//! off. [`watch`] follows the daemon's changes. Every call may wait on the daemon, so the session
//! makes them off its compositor's turn.
//!
//! What a joined network needs ([`Plan`]) is decided here from what its access point says; the
//! person's part (a password sheet, its retries) is the session's.

mod nm;
mod secret;

pub use nm::{watch, NetworkManager};
pub use secret::{clear, Secret};

use serde::Serialize;

/// The network as the daemon has it, as the session tells the shell and Settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// The daemon answers.
    pub available: bool,
    pub wifi: Wifi,
    pub ethernet: Ethernet,
    /// The name of the network being joined.
    pub joining: Option<String>,
}

impl State {
    /// No daemon: nothing to show.
    pub fn unavailable() -> Self {
        Self::default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WifiState {
    #[default]
    Unavailable,
    Off,
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Wifi {
    /// A Wi-Fi device exists.
    pub device: bool,
    pub enabled: bool,
    /// Off by the hardware's switch.
    pub hardware_blocked: bool,
    pub state: WifiState,
    pub current: Option<Network>,
    /// Nearby, strongest first, one per name.
    pub networks: Vec<Network>,
    /// The saved Wi-Fi connections.
    pub known: Vec<Known>,
    /// Of the network joined.
    pub details: Option<Details>,
}

/// How a network is protected, from what its access point says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Security {
    Open,
    /// Opportunistic Wireless Encryption: open, but encrypted.
    Owe,
    /// WPA or WPA2 Personal (a WPA3 network that also takes WPA2 is this).
    Wpa,
    /// WPA3 Personal only (SAE).
    Wpa3,
    Wep,
    /// 802.1X.
    Enterprise,
}

impl Security {
    /// Whether joining needs a password.
    pub fn needs_password(self) -> bool {
        matches!(self, Security::Wpa | Security::Wpa3)
    }

    /// Whether OxyOS joins it (docs/adr/0038-networks.mdx, decision 8).
    pub fn supported(self) -> bool {
        !matches!(self, Security::Wep | Security::Enterprise)
    }

    /// NetworkManager's key management for it, when it has a password.
    pub fn key_mgmt(self) -> Option<&'static str> {
        match self {
            Security::Wpa => Some("wpa-psk"),
            Security::Wpa3 => Some("sae"),
            Security::Owe => Some("owe"),
            _ => None,
        }
    }

    /// Its name in the vocabulary.
    pub fn name(self) -> &'static str {
        match self {
            Security::Open => "open",
            Security::Owe => "owe",
            Security::Wpa => "wpa",
            Security::Wpa3 => "wpa3",
            Security::Wep => "wep",
            Security::Enterprise => "enterprise",
        }
    }
}

/// NetworkManager's access point flags (`NM80211ApFlags`, `NM80211ApSecurityFlags`).
const AP_PRIVACY: u32 = 0x1;
const KEY_MGMT_PSK: u32 = 0x100;
const KEY_MGMT_802_1X: u32 = 0x200;
const KEY_MGMT_SAE: u32 = 0x400;
const KEY_MGMT_OWE: u32 = 0x800;
const KEY_MGMT_OWE_TM: u32 = 0x1000;
const KEY_MGMT_EAP_SUITE_B_192: u32 = 0x2000;

/// A network's protection from its access point's `Flags`, `WpaFlags` and `RsnFlags`.
pub fn security(flags: u32, wpa: u32, rsn: u32) -> Security {
    let keys = wpa | rsn;
    if keys & (KEY_MGMT_802_1X | KEY_MGMT_EAP_SUITE_B_192) != 0 {
        Security::Enterprise
    } else if keys & KEY_MGMT_PSK != 0 {
        Security::Wpa
    } else if keys & KEY_MGMT_SAE != 0 {
        Security::Wpa3
    } else if keys & (KEY_MGMT_OWE | KEY_MGMT_OWE_TM) != 0 {
        Security::Owe
    } else if flags & AP_PRIVACY != 0 {
        Security::Wep
    } else {
        Security::Open
    }
}

/// A network nearby.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Network {
    pub ssid: String,
    /// 0 to 100.
    pub signal: u8,
    pub security: Security,
    /// A saved connection has its name.
    pub known: bool,
    /// It is the one joined (or being joined by the daemon).
    pub current: bool,
    pub supported: bool,
}

/// A saved Wi-Fi connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Known {
    /// The connection's UUID.
    pub id: String,
    pub ssid: String,
    pub auto_join: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EthernetState {
    #[default]
    Unavailable,
    Unplugged,
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ethernet {
    pub device: bool,
    pub state: EthernetState,
    /// Mb/s; 0 unknown.
    pub speed: u32,
    pub details: Option<Details>,
}

/// A connected device's configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Details {
    pub interface: String,
    pub hardware_address: String,
    pub ipv4: Option<Ipv4>,
    pub router: Option<String>,
    pub dns: Vec<String>,
    pub ipv6: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ipv4 {
    pub address: String,
    pub prefix: u32,
    pub subnet_mask: String,
}

/// An IPv4 prefix length as a dotted mask: 24 is 255.255.255.0.
pub fn subnet_mask(prefix: u32) -> String {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix.min(32))
    };
    std::net::Ipv4Addr::from(mask).to_string()
}

/// An access point's name as text: its bytes, which need not be UTF-8, with what is not replaced.
pub fn ssid_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// What joining a network needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// A saved connection: activated as saved, the daemon has its password.
    Known { security: Security },
    /// Nothing to ask: open, or OWE.
    Open,
    /// A password the person must give.
    Password { security: Security },
    /// Not joined by OxyOS (enterprise, WEP).
    Unsupported { security: Security },
}

/// Why a call did not do what it was asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The daemon did not get the password it needs, or the access point refused it.
    WrongPassword,
    /// Joining needs a password and none was given.
    NeedsPassword,
    Cancelled,
    Unsupported,
    NotFound,
    /// polkit refused it.
    NotPermitted,
    /// No daemon, no Wi-Fi device, or Wi-Fi off.
    Unavailable,
    Timeout,
    Failed,
}

impl Failure {
    /// Its name in the vocabulary.
    pub fn reason(self) -> &'static str {
        match self {
            Failure::WrongPassword => "wrongPassword",
            Failure::NeedsPassword => "needsPassword",
            Failure::Cancelled => "cancelled",
            Failure::Unsupported => "unsupported",
            Failure::NotFound => "notFound",
            Failure::NotPermitted => "notPermitted",
            Failure::Unavailable => "unavailable",
            Failure::Timeout => "timeout",
            Failure::Failed => "failed",
        }
    }
}

/// NetworkManager's device states.
pub mod device_state {
    pub const UNMANAGED: u32 = 10;
    pub const UNAVAILABLE: u32 = 20;
    pub const DISCONNECTED: u32 = 30;
    pub const PREPARE: u32 = 40;
    pub const CONFIG: u32 = 50;
    pub const NEED_AUTH: u32 = 60;
    pub const IP_CONFIG: u32 = 70;
    pub const ACTIVATED: u32 = 100;
    pub const DEACTIVATING: u32 = 110;
    pub const FAILED: u32 = 120;
}

/// NetworkManager's device state reasons that mean the password was wrong or missing during the
/// handshake: no secrets (the daemon asked for new ones and nobody gave them), and the
/// supplicant's disconnect and timeout while authenticating.
pub fn secrets_reason(reason: u32) -> bool {
    matches!(reason, 7 | 8 | 11)
}

/// Wi-Fi's state from what the daemon says: Wi-Fi enabled (by software and by its switch) and the
/// Wi-Fi device's state, if there is one.
pub fn wifi_state(enabled: bool, hardware_enabled: bool, device: Option<u32>) -> WifiState {
    use device_state::*;
    match device {
        None => WifiState::Unavailable,
        Some(_) if !enabled || !hardware_enabled => WifiState::Off,
        Some(ACTIVATED) => WifiState::Connected,
        Some(state) if (PREPARE..ACTIVATED).contains(&state) => WifiState::Connecting,
        Some(_) => WifiState::Disconnected,
    }
}

/// Ethernet's state from its device's state and carrier.
pub fn ethernet_state(state: u32, carrier: bool) -> EthernetState {
    use device_state::*;
    match state {
        UNMANAGED => EthernetState::Unavailable,
        _ if !carrier => EthernetState::Unplugged,
        ACTIVATED => EthernetState::Connected,
        state if (PREPARE..ACTIVATED).contains(&state) => EthernetState::Connecting,
        _ => EthernetState::Disconnected,
    }
}

/// An access point as read: its name, signal and protection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessPoint {
    pub ssid: String,
    pub signal: u8,
    pub security: Security,
}

/// The networks nearby from the access points seen: one per name (the strongest), hidden ones
/// left out, strongest first (then by name, so the order is stable).
pub fn networks(points: &[AccessPoint], known: &[Known], current: Option<&str>) -> Vec<Network> {
    let mut networks: Vec<Network> = Vec::new();
    for point in points {
        if point.ssid.is_empty() {
            continue;
        }
        if let Some(seen) = networks.iter_mut().find(|n| n.ssid == point.ssid) {
            if point.signal > seen.signal {
                seen.signal = point.signal;
                seen.security = point.security;
                seen.supported = point.security.supported();
            }
            continue;
        }
        networks.push(Network {
            ssid: point.ssid.clone(),
            signal: point.signal.min(100),
            security: point.security,
            known: known.iter().any(|k| k.ssid == point.ssid),
            current: current == Some(point.ssid.as_str()),
            supported: point.security.supported(),
        });
    }
    networks.sort_by(|a, b| b.signal.cmp(&a.signal).then_with(|| a.ssid.cmp(&b.ssid)));
    networks
}

/// What joining `ssid` needs, from the networks nearby and the saved ones.
pub fn plan(networks: &[Network], ssid: &str) -> Result<Plan, Failure> {
    let network = networks
        .iter()
        .find(|n| n.ssid == ssid)
        .ok_or(Failure::NotFound)?;
    let security = network.security;
    Ok(if !security.supported() {
        Plan::Unsupported { security }
    } else if network.known {
        Plan::Known { security }
    } else if security.needs_password() {
        Plan::Password { security }
    } else {
        Plan::Open
    })
}

#[cfg(test)]
mod tests;
