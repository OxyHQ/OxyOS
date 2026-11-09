//! A played NetworkManager for tests and scenarios (docs/adr/0038-networks.mdx): it answers the
//! part of `org.freedesktop.NetworkManager`'s D-Bus API that `oxyos-networks` uses, on a bus of a
//! test's own (never the system's), for a machine a JSON file describes: a Wi-Fi device and the
//! access points it sees (each with the password it takes), saved connections, an Ethernet port.
//! Joining goes through the daemon's device states and signals as NetworkManager's does; a wrong
//! password fails for want of secrets. A control interface (`so.oxy.OxyOS.FakeNetworkManager` at
//! `/so/oxy/OxyOS/FakeNetworkManager`) changes the machine (`Act`: a cable plugged, an access
//! point appearing) and says what the daemon holds (`State`: its connections with their
//! passwords, what was tried), so a test asserts the daemon's state, not only what was drawn.
//!
//! It also plays the network behind the daemon for Wi-Fi Diagnostics (`Probe`: the router's echoes
//! and ARP, the resolver), counting each probe, and each connection's private address as the
//! daemon makes it (`assigned-mac-address`, `stable-id`): the device's address is the one the
//! connection joined with, a fixed one the same at every join, a rotating one another after a
//! restart (`{"restart":{}}`).
//!
//! Never installed: a test fixture, like the development compositor.
//!
//! Usage: oxyos-fake-networkmanager --address ADDRESS --machine FILE [--state FILE]

use serde::Deserialize;
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const WIFI_DEVICE: &str = "/org/freedesktop/NetworkManager/Devices/1";
const ETHERNET_DEVICE: &str = "/org/freedesktop/NetworkManager/Devices/2";
const STEP: Duration = Duration::from_millis(40);
/// The Wi-Fi device's own address.
const PERMANENT: &str = "F4:A4:75:12:34:56";

type Settings = HashMap<String, HashMap<String, OwnedValue>>;

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Machine {
    /// A Wi-Fi device exists (true unless said).
    wifi: Option<bool>,
    wifi_enabled: Option<bool>,
    hardware_enabled: Option<bool>,
    access_points: Vec<PointSpec>,
    connections: Vec<ConnectionSpec>,
    /// The saved network joined at start.
    active: Option<String>,
    /// "none", "unplugged" or "connected".
    ethernet: Option<String>,
    /// polkit refuses connections for every user (`settings.modify.system`).
    deny_system: bool,
    /// The router behind the access points, for diagnostics' echoes.
    router: RouterSpec,
    /// The resolver, for diagnostics' lookups.
    resolver: ResolverSpec,
    /// The daemon's connectivity check.
    connectivity: ConnectivitySpec,
}

#[derive(Deserialize, Clone)]
#[serde(default, deny_unknown_fields)]
struct RouterSpec {
    /// This user may send echoes (`net.ipv4.ping_group_range`).
    allowed: bool,
    /// It answers echoes, each in these ms.
    answers: bool,
    ms: Vec<f64>,
    /// It answered ARP for its address.
    neighbour: bool,
}

impl Default for RouterSpec {
    fn default() -> Self {
        Self {
            allowed: true,
            answers: true,
            ms: vec![1.4, 1.1, 1.2],
            neighbour: true,
        }
    }
}

#[derive(Deserialize, Clone)]
#[serde(default, deny_unknown_fields)]
struct ResolverSpec {
    /// "answers", "fails" or "silent".
    does: String,
    ms: u64,
    addresses: usize,
}

impl Default for ResolverSpec {
    fn default() -> Self {
        Self {
            does: "answers".into(),
            ms: 14,
            addresses: 2,
        }
    }
}

#[derive(Deserialize, Clone, Default)]
#[serde(default, deny_unknown_fields)]
struct ConnectivitySpec {
    /// A check is configured and enabled.
    check: bool,
    /// Its `NMConnectivityState` (4 full, 3 limited, 2 portal, 1 none).
    state: u32,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct PointSpec {
    ssid: String,
    strength: u8,
    #[serde(default = "open")]
    security: String,
    #[serde(default)]
    password: Option<String>,
    /// MHz.
    #[serde(default)]
    frequency: Option<u32>,
    #[serde(default)]
    bssid: Option<String>,
    /// The rate the card sends at to it, and the fastest it offers, Mb/s; its channel's width, MHz.
    #[serde(default)]
    rate: Option<u32>,
    #[serde(default)]
    max_rate: Option<u32>,
    #[serde(default)]
    width: Option<u32>,
    /// The only addresses it lets join (a network filtering by address); any when none.
    #[serde(default)]
    admits: Option<Vec<String>>,
}

fn open() -> String {
    "open".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionSpec {
    /// A Wi-Fi network's.
    #[serde(default)]
    ssid: String,
    #[serde(default)]
    password: Option<String>,
    /// "wifi" (the default), "vpn", "wireguard" or "ethernet".
    #[serde(default, rename = "type")]
    kind: Option<String>,
    /// Another kind's name.
    #[serde(default)]
    name: Option<String>,
    /// The users it is for (`user:NAME:`); none, every user.
    #[serde(default)]
    permissions: Vec<String>,
    /// `assigned-mac-address` and `stable-id`, unset unless said.
    #[serde(default)]
    assigned: Option<String>,
    #[serde(default)]
    stable_id: Option<String>,
    /// `NMSettingsConnectionFlags` (2: the daemon made it).
    #[serde(default)]
    flags: u32,
}

struct Point {
    spec: PointSpec,
    path: String,
}

struct Connection {
    number: u32,
    uuid: String,
    ssid: String,
    /// NetworkManager's type: `802-11-wireless`, `vpn`, `wireguard`, `802-3-ethernet`.
    kind: String,
    /// Its `connection.id`.
    name: String,
    key_mgmt: Option<String>,
    psk: Option<String>,
    permissions: Vec<String>,
    autoconnect: bool,
    registered: bool,
    assigned: Option<String>,
    stable_id: Option<String>,
    flags: u32,
}

impl Connection {
    fn path(&self) -> String {
        format!("{NM_PATH}/Settings/{}", self.number)
    }
}

struct Model {
    wifi: bool,
    wifi_enabled: bool,
    hardware_enabled: bool,
    wifi_state: u32,
    active: Option<usize>,
    points: Vec<Point>,
    next_point: u32,
    connections: Vec<Connection>,
    next_connection: u32,
    /// None: no port.
    carrier: Option<bool>,
    ethernet_state: u32,
    deny_system: bool,
    scans: u32,
    attempts: Vec<Json>,
    /// The connection the Wi-Fi device joined with, by its number.
    active_connection: Option<u32>,
    /// The Wi-Fi device's address now: the one its connection joined with.
    address: String,
    /// This boot's id (`${BOOT}`), another after `{"restart":{}}`.
    boot: u32,
    router: RouterSpec,
    resolver: ResolverSpec,
    connectivity: ConnectivitySpec,
    /// Diagnostics' probes, in order, and the connectivity checks run.
    probes: Vec<String>,
    checks: u32,
    /// Bumped by every activation, so an older one's steps stop.
    activation: u64,
    state_file: Option<String>,
}

type Shared = Arc<Mutex<Model>>;
static BUS: OnceLock<zbus::blocking::Connection> = OnceLock::new();

fn bus() -> &'static zbus::blocking::Connection {
    BUS.get().expect("connected")
}

fn flags(security: &str) -> (u32, u32, u32) {
    match security {
        "wpa" => (1, 0, 0x188),
        "wpa3" => (1, 0, 0x488),
        "wep" => (1, 0, 0),
        "enterprise" => (1, 0, 0x288),
        "owe" => (0, 0, 0x888),
        _ => (0, 0, 0),
    }
}

fn path(text: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(text.to_string()).expect("a valid path")
}

fn owned(value: Value<'_>) -> OwnedValue {
    OwnedValue::try_from(value).expect("no descriptors")
}

impl Model {
    fn describe(&self) -> Json {
        json!({
            "wifi_enabled": self.wifi_enabled,
            "wifi_state": self.wifi_state,
            "active": self.active.map(|at| self.points[at].spec.ssid.clone()),
            "connections": self.connections.iter().map(|c| json!({
                "uuid": c.uuid, "ssid": c.ssid, "psk": c.psk, "key_mgmt": c.key_mgmt,
                "permissions": c.permissions, "type": c.kind, "name": c.name,
                "assigned": c.assigned, "stable_id": c.stable_id,
            })).collect::<Vec<_>>(),
            "scans": self.scans,
            "attempts": self.attempts,
            "address": self.address,
            "joined_with": self.active_connection
                .and_then(|number| self.connections.iter().find(|c| c.number == number))
                .map(|c| c.uuid.clone()),
            "probes": self.probes,
            "connectivity_checks": self.checks,
            "ethernet": match self.carrier {
                None => "none",
                Some(false) => "unplugged",
                Some(true) if self.ethernet_state == 100 => "connected",
                Some(true) => "disconnected",
            },
        })
    }

    fn save(&self) {
        if let Some(file) = &self.state_file {
            let temporary = format!("{file}.new");
            if std::fs::write(&temporary, self.describe().to_string()).is_ok() {
                let _ = std::fs::rename(temporary, file);
            }
        }
    }

    fn add_connection(
        &mut self,
        ssid: &str,
        key_mgmt: Option<String>,
        psk: Option<String>,
    ) -> usize {
        self.next_connection += 1;
        let number = self.next_connection;
        self.connections.push(Connection {
            number,
            uuid: format!("00000000-0000-4000-8000-{number:012}"),
            ssid: ssid.into(),
            kind: "802-11-wireless".into(),
            name: ssid.into(),
            key_mgmt,
            psk,
            permissions: Vec::new(),
            autoconnect: true,
            registered: false,
            assigned: None,
            stable_id: None,
            flags: 0,
        });
        self.connections.len() - 1
    }

    /// The address a connection joins with, as NetworkManager makes it: the hardware's for none,
    /// `preserve` or `permanent`; for `stable`, one hashed from its stable-id (its substitutions
    /// made; unset, `default${CONNECTION}`) with a key of this machine's, locally administered;
    /// for `random`, another at every activation; an explicit one as it is.
    fn address_of(&self, connection: &Connection) -> String {
        let expand = |id: &str| {
            id.replace("${CONNECTION}", &connection.uuid)
                .replace("${BOOT}", &format!("boot-{}", self.boot))
                .replace("${RANDOM}", &format!("random-{}", self.activation))
                .replace("${NETWORK_SSID}", &connection.ssid)
        };
        let seed = match connection.assigned.as_deref() {
            None | Some("preserve" | "permanent") => return PERMANENT.into(),
            Some("stable") => expand(
                connection
                    .stable_id
                    .as_deref()
                    .unwrap_or("default${CONNECTION}"),
            ),
            Some("stable-ssid") => expand("${NETWORK_SSID}"),
            Some("random") => format!("random-{}", self.activation),
            Some(explicit) => return explicit.to_uppercase(),
        };
        // FNV-1a, keyed by this played machine.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in format!("fake-machine-key/{seed}").bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let mut bytes = hash.to_be_bytes();
        bytes[0] = (bytes[0] & 0xfe) | 0x02;
        bytes[..6]
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    }

    fn add_point(&mut self, spec: PointSpec) -> String {
        self.next_point += 1;
        let path = format!("{NM_PATH}/AccessPoint/{}", self.next_point);
        self.points.push(Point {
            spec,
            path: path.clone(),
        });
        path
    }
}

fn changed(path: &str, interface: &str) {
    let changes: HashMap<&str, Value> = HashMap::new();
    let invalidated: Vec<&str> = Vec::new();
    let _ = bus().emit_signal(
        None::<&str>,
        path,
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
        &(interface, changes, invalidated),
    );
}

fn device_state(model: &Shared, device: &str, new: u32, reason: u32) {
    let old = {
        let mut m = model.lock().unwrap();
        let state = if device == WIFI_DEVICE {
            &mut m.wifi_state
        } else {
            &mut m.ethernet_state
        };
        let old = *state;
        *state = new;
        m.save();
        old
    };
    let _ = bus().emit_signal(
        None::<&str>,
        device,
        "org.freedesktop.NetworkManager.Device",
        "StateChanged",
        &(new, old, reason),
    );
    changed(device, "org.freedesktop.NetworkManager.Device");
    changed(NM_PATH, "org.freedesktop.NetworkManager");
}

fn register_connection(model: &Shared, at: usize) {
    let (path, number) = {
        let m = model.lock().unwrap();
        (m.connections[at].path(), m.connections[at].number)
    };
    let _ = bus().object_server().at(
        path,
        SettingsConnection {
            model: model.clone(),
            number,
        },
    );
    let mut m = model.lock().unwrap();
    if let Some(connection) = m.connections.iter_mut().find(|c| c.number == number) {
        connection.registered = true;
    }
    m.save();
}

/// The active connection object of the connection numbered `number`, as the daemon has one for
/// each activation (its `Connection`).
fn register_active(model: &Shared, number: u32) {
    let _ = bus().object_server().at(
        format!("{NM_PATH}/ActiveConnection/{number}"),
        Active {
            model: model.clone(),
            number,
        },
    );
}

/// Joins the access point `point` with the connection numbered `number`, through the device's
/// states as NetworkManager's: a protected network whose password is not the access point's fails
/// for want of secrets.
fn activate(model: Shared, number: u32, point: usize) {
    use std::thread::sleep;
    let (activation, was_up, accepted, admitted) = {
        let mut m = model.lock().unwrap();
        m.activation += 1;
        let connection = m
            .connections
            .iter()
            .find(|c| c.number == number)
            .expect("saved");
        let spec = &m.points[point].spec;
        let accepted = match spec.security.as_str() {
            "wpa" | "wpa3" => connection.psk.is_some() && connection.psk == spec.password,
            _ => true,
        };
        // The address it would join with, which a network filtering by address may refuse.
        let address = m.address_of(connection);
        let admitted = spec
            .admits
            .as_ref()
            .is_none_or(|admits| admits.iter().any(|a| a.eq_ignore_ascii_case(&address)));
        let attempt = json!({"ssid": spec.ssid, "psk": connection.psk, "address": address});
        m.attempts.push(attempt);
        (m.activation, m.wifi_state == 100, accepted, admitted)
    };
    let current = |model: &Shared| model.lock().unwrap().activation == activation;
    if was_up {
        {
            let mut m = model.lock().unwrap();
            m.active = None;
            m.active_connection = None;
            m.address = PERMANENT.into();
        }
        device_state(&model, WIFI_DEVICE, 110, 60);
        device_state(&model, WIFI_DEVICE, 30, 60);
    }
    for state in [40, 50] {
        sleep(STEP);
        if !current(&model) {
            return;
        }
        device_state(&model, WIFI_DEVICE, state, 0);
    }
    sleep(STEP);
    if !current(&model) {
        return;
    }
    if !admitted {
        // The access point refuses the association: the supplicant fails, no secrets asked.
        device_state(&model, WIFI_DEVICE, 120, 10);
        device_state(&model, WIFI_DEVICE, 30, 10);
    } else if accepted {
        device_state(&model, WIFI_DEVICE, 70, 0);
        sleep(STEP);
        if !current(&model) {
            return;
        }
        {
            let mut m = model.lock().unwrap();
            m.active = Some(point);
            m.active_connection = Some(number);
            // The address it joins with, as the daemon sets it as the connection activates.
            let address = m
                .connections
                .iter()
                .find(|c| c.number == number)
                .map(|c| m.address_of(c));
            if let Some(address) = address {
                m.address = address;
            }
        }
        register_active(&model, number);
        changed(
            WIFI_DEVICE,
            "org.freedesktop.NetworkManager.Device.Wireless",
        );
        device_state(&model, WIFI_DEVICE, 100, 0);
    } else {
        device_state(&model, WIFI_DEVICE, 60, 0);
        sleep(STEP);
        device_state(&model, WIFI_DEVICE, 120, 7);
        device_state(&model, WIFI_DEVICE, 30, 7);
    }
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.NetworkManager")]
enum NmError {
    #[zbus(error)]
    ZBus(zbus::Error),
    PermissionDenied(String),
    UnknownConnection(String),
    UnknownDevice(String),
}

struct Manager {
    model: Shared,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl Manager {
    #[zbus(property)]
    fn wireless_enabled(&self) -> bool {
        self.model.lock().unwrap().wifi_enabled
    }

    #[zbus(property)]
    fn set_wireless_enabled(&mut self, enabled: bool) {
        let model = self.model.clone();
        std::thread::spawn(move || {
            let up = {
                let mut m = model.lock().unwrap();
                m.wifi_enabled = enabled;
                m.activation += 1;
                if !enabled {
                    m.active = None;
                    m.active_connection = None;
                    m.address = PERMANENT.into();
                }
                m.save();
                m.wifi_state
            };
            if enabled {
                device_state(&model, WIFI_DEVICE, 30, 0);
            } else if up != 20 {
                device_state(&model, WIFI_DEVICE, 20, 0);
            }
            changed(NM_PATH, "org.freedesktop.NetworkManager");
        });
    }

    #[zbus(property)]
    fn wireless_hardware_enabled(&self) -> bool {
        self.model.lock().unwrap().hardware_enabled
    }

    #[zbus(property)]
    fn devices(&self) -> Vec<OwnedObjectPath> {
        let m = self.model.lock().unwrap();
        let mut devices = Vec::new();
        if m.wifi {
            devices.push(path(WIFI_DEVICE));
        }
        if m.carrier.is_some() {
            devices.push(path(ETHERNET_DEVICE));
        }
        devices
    }

    #[zbus(property)]
    fn version(&self) -> String {
        "1.52.1".into()
    }

    /// What polkit lets the caller do, as NetworkManager asks it: `auth` for what needs an
    /// administrator's password (no agent here answers it).
    fn get_permissions(&self) -> HashMap<String, String> {
        let m = self.model.lock().unwrap();
        HashMap::from([
            (
                "org.freedesktop.NetworkManager.settings.modify.system".to_string(),
                if m.deny_system { "auth" } else { "yes" }.to_string(),
            ),
            (
                "org.freedesktop.NetworkManager.settings.modify.own".to_string(),
                "yes".to_string(),
            ),
            (
                "org.freedesktop.NetworkManager.network-control".to_string(),
                "yes".to_string(),
            ),
        ])
    }

    #[zbus(property)]
    fn connectivity(&self) -> u32 {
        let m = self.model.lock().unwrap();
        if m.connectivity.check {
            m.connectivity.state
        } else {
            // No check: the daemon assumes the internet answers when a device is up.
            if m.wifi_state == 100 || m.ethernet_state == 100 {
                4
            } else {
                1
            }
        }
    }

    #[zbus(property)]
    fn connectivity_check_available(&self) -> bool {
        self.model.lock().unwrap().connectivity.check
    }

    #[zbus(property)]
    fn connectivity_check_enabled(&self) -> bool {
        self.model.lock().unwrap().connectivity.check
    }

    /// Runs the check now: its state, counted.
    fn check_connectivity(&self) -> u32 {
        {
            let mut m = self.model.lock().unwrap();
            m.checks += 1;
            m.save();
        }
        self.connectivity()
    }

    fn activate_connection(
        &self,
        connection: OwnedObjectPath,
        _device: OwnedObjectPath,
        specific: OwnedObjectPath,
    ) -> Result<OwnedObjectPath, NmError> {
        let (number, point) = {
            let m = self.model.lock().unwrap();
            let saved = m
                .connections
                .iter()
                .find(|c| c.path() == connection.as_str())
                .ok_or_else(|| NmError::UnknownConnection(connection.to_string()))?;
            let point = m
                .points
                .iter()
                .position(|p| p.path == specific.as_str())
                .or_else(|| m.points.iter().position(|p| p.spec.ssid == saved.ssid))
                .ok_or_else(|| NmError::UnknownConnection("no access point".into()))?;
            (saved.number, point)
        };
        let model = self.model.clone();
        std::thread::spawn(move || activate(model, number, point));
        Ok(path(&format!("{NM_PATH}/ActiveConnection/{number}")))
    }

    fn add_and_activate_connection(
        &self,
        settings: Settings,
        device: OwnedObjectPath,
        specific: OwnedObjectPath,
    ) -> Result<(OwnedObjectPath, OwnedObjectPath), NmError> {
        if device.as_str() != WIFI_DEVICE {
            return Err(NmError::UnknownDevice(device.to_string()));
        }
        let text = |group: &str, key: &str| -> Option<String> {
            String::try_from(settings.get(group)?.get(key)?.try_clone().ok()?).ok()
        };
        let permissions: Vec<String> = settings
            .get("connection")
            .and_then(|c| c.get("permissions"))
            .and_then(|p| Vec::<String>::try_from(p.try_clone().ok()?).ok())
            .unwrap_or_default();
        let ssid: Vec<u8> = settings
            .get("802-11-wireless")
            .and_then(|w| w.get("ssid"))
            .and_then(|s| Vec::<u8>::try_from(s.try_clone().ok()?).ok())
            .ok_or_else(|| NmError::UnknownConnection("no ssid".into()))?;
        let ssid = String::from_utf8_lossy(&ssid).into_owned();
        let (at, number, point) = {
            let mut m = self.model.lock().unwrap();
            if m.deny_system && permissions.is_empty() {
                return Err(NmError::PermissionDenied(
                    "Insufficient privileges (settings.modify.system)".into(),
                ));
            }
            let point = m
                .points
                .iter()
                .position(|p| p.path == specific.as_str())
                .ok_or_else(|| NmError::UnknownConnection("no access point".into()))?;
            let at = m.add_connection(
                &ssid,
                text("802-11-wireless-security", "key-mgmt"),
                text("802-11-wireless-security", "psk"),
            );
            m.connections[at].permissions = permissions;
            m.connections[at].assigned = text("802-11-wireless", "assigned-mac-address");
            m.connections[at].stable_id = text("connection", "stable-id");
            (at, m.connections[at].number, point)
        };
        register_connection(&self.model, at);
        let model = self.model.clone();
        std::thread::spawn(move || activate(model, number, point));
        Ok((
            path(&format!("{NM_PATH}/Settings/{number}")),
            path(&format!("{NM_PATH}/ActiveConnection/{number}")),
        ))
    }
}

struct SettingsRoot {
    model: Shared,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
impl SettingsRoot {
    fn list_connections(&self) -> Vec<OwnedObjectPath> {
        let m = self.model.lock().unwrap();
        m.connections
            .iter()
            .filter(|c| c.registered)
            .map(|c| path(&c.path()))
            .collect()
    }
}

struct SettingsConnection {
    model: Shared,
    number: u32,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl SettingsConnection {
    /// Its settings, without its secrets, as NetworkManager's.
    fn get_settings(&self) -> Result<Settings, NmError> {
        let m = self.model.lock().unwrap();
        let c = m
            .connections
            .iter()
            .find(|c| c.number == self.number)
            .ok_or_else(|| NmError::UnknownConnection(self.number.to_string()))?;
        let mut settings: Settings = HashMap::new();
        let mut connection = HashMap::from([
            ("id".to_string(), owned(Value::from(c.name.clone()))),
            ("uuid".to_string(), owned(Value::from(c.uuid.clone()))),
            ("type".to_string(), owned(Value::from(c.kind.clone()))),
            ("autoconnect".to_string(), owned(Value::from(c.autoconnect))),
            (
                "permissions".to_string(),
                owned(Value::from(c.permissions.clone())),
            ),
        ]);
        if let Some(stable_id) = &c.stable_id {
            connection.insert(
                "stable-id".to_string(),
                owned(Value::from(stable_id.clone())),
            );
        }
        settings.insert("connection".into(), connection);
        if c.kind != "802-11-wireless" {
            return Ok(settings);
        }
        let mut wireless = HashMap::from([
            (
                "ssid".to_string(),
                owned(Value::from(c.ssid.as_bytes().to_vec())),
            ),
            ("mode".to_string(), owned(Value::from("infrastructure"))),
        ]);
        if let Some(assigned) = &c.assigned {
            wireless.insert(
                "assigned-mac-address".to_string(),
                owned(Value::from(assigned.clone())),
            );
        }
        settings.insert("802-11-wireless".into(), wireless);
        if let Some(key_mgmt) = &c.key_mgmt {
            settings.insert(
                "802-11-wireless-security".into(),
                HashMap::from([("key-mgmt".to_string(), owned(Value::from(key_mgmt.clone())))]),
            );
        }
        Ok(settings)
    }

    /// Its secrets, to whoever may change it (polkit's `settings.modify.*`).
    fn get_secrets(&self, setting: String) -> Result<Settings, NmError> {
        let m = self.model.lock().unwrap();
        let c = m
            .connections
            .iter()
            .find(|c| c.number == self.number)
            .ok_or_else(|| NmError::UnknownConnection(self.number.to_string()))?;
        if m.deny_system && c.permissions.is_empty() {
            return Err(NmError::PermissionDenied("settings.modify.system".into()));
        }
        let mut secrets: Settings = HashMap::new();
        if let (true, Some(psk)) = (setting == "802-11-wireless-security", &c.psk) {
            secrets.insert(
                setting,
                HashMap::from([("psk".to_string(), owned(Value::from(psk.clone())))]),
            );
        }
        Ok(secrets)
    }

    fn update(&self, settings: Settings) -> Result<(), NmError> {
        let mut m = self.model.lock().unwrap();
        let denied = m.deny_system;
        let c = m
            .connections
            .iter_mut()
            .find(|c| c.number == self.number)
            .ok_or_else(|| NmError::UnknownConnection(self.number.to_string()))?;
        if denied && c.permissions.is_empty() {
            return Err(NmError::PermissionDenied("settings.modify.system".into()));
        }
        let text = |group: &str, key: &str| -> Option<String> {
            String::try_from(settings.get(group)?.get(key)?.try_clone().ok()?).ok()
        };
        // An update that carries no secrets keeps those saved, as NetworkManager's does
        // (nm-settings-connection.c: "we do not want to remove all secrets").
        if let Some(psk) = text("802-11-wireless-security", "psk") {
            c.psk = Some(psk);
        }
        c.key_mgmt = text("802-11-wireless-security", "key-mgmt");
        c.assigned = text("802-11-wireless", "assigned-mac-address");
        c.stable_id = text("connection", "stable-id");
        m.save();
        Ok(())
    }

    #[zbus(property)]
    fn flags(&self) -> u32 {
        let m = self.model.lock().unwrap();
        m.connections
            .iter()
            .find(|c| c.number == self.number)
            .map_or(0, |c| c.flags)
    }

    fn delete(&self) -> Result<(), NmError> {
        let gone = {
            let mut m = self.model.lock().unwrap();
            let denied = m.deny_system;
            let at = m
                .connections
                .iter()
                .position(|c| c.number == self.number)
                .ok_or_else(|| NmError::UnknownConnection(self.number.to_string()))?;
            if denied && m.connections[at].permissions.is_empty() {
                return Err(NmError::PermissionDenied("settings.modify.system".into()));
            }
            let removed = m.connections.remove(at);
            // The connection joined with: the device leaves the network, as the daemon's does.
            let joined = m.active_connection == Some(removed.number);
            if joined {
                m.activation += 1;
                m.active = None;
                m.active_connection = None;
                m.address = PERMANENT.into();
            }
            m.save();
            (removed.path(), joined)
        };
        let model = self.model.clone();
        std::thread::spawn(move || {
            let (removed, joined) = gone;
            let _ = bus()
                .object_server()
                .remove::<SettingsConnection, _>(removed.as_str());
            let _ = bus().emit_signal(
                None::<&str>,
                removed.as_str(),
                "org.freedesktop.NetworkManager.Settings.Connection",
                "Removed",
                &(),
            );
            let _ = bus().emit_signal(
                None::<&str>,
                format!("{NM_PATH}/Settings").as_str(),
                "org.freedesktop.NetworkManager.Settings",
                "ConnectionRemoved",
                &(path(&removed),),
            );
            if joined {
                device_state(&model, WIFI_DEVICE, 110, 39);
                device_state(&model, WIFI_DEVICE, 30, 39);
            }
            changed(NM_PATH, "org.freedesktop.NetworkManager");
        });
        Ok(())
    }
}

struct Device {
    model: Shared,
    wifi: bool,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device")]
impl Device {
    #[zbus(property)]
    fn device_type(&self) -> u32 {
        if self.wifi {
            2
        } else {
            1
        }
    }

    #[zbus(property)]
    fn state(&self) -> u32 {
        let m = self.model.lock().unwrap();
        if self.wifi {
            m.wifi_state
        } else {
            m.ethernet_state
        }
    }

    #[zbus(property)]
    fn interface(&self) -> String {
        if self.wifi { "wlp0s20f3" } else { "enp0s31f6" }.into()
    }

    /// The address in use: the Wi-Fi device's, the one its connection joined with.
    #[zbus(property)]
    fn hw_address(&self) -> String {
        if self.wifi {
            self.model.lock().unwrap().address.clone()
        } else {
            "38:F3:AB:65:43:21".into()
        }
    }

    #[zbus(property)]
    fn active_connection(&self) -> OwnedObjectPath {
        let m = self.model.lock().unwrap();
        match m.active_connection.filter(|_| self.wifi) {
            Some(number) => path(&format!("{NM_PATH}/ActiveConnection/{number}")),
            None => path("/"),
        }
    }

    #[zbus(property, name = "Ip4Config")]
    fn ip4_config(&self) -> OwnedObjectPath {
        if self.state() == 100 {
            path(&format!("{NM_PATH}/IP4Config/{}", self.device_type()))
        } else {
            path("/")
        }
    }

    #[zbus(property, name = "Ip6Config")]
    fn ip6_config(&self) -> OwnedObjectPath {
        if self.state() == 100 {
            path(&format!("{NM_PATH}/IP6Config/{}", self.device_type()))
        } else {
            path("/")
        }
    }
}

struct Wireless {
    model: Shared,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
impl Wireless {
    #[zbus(property)]
    fn access_points(&self) -> Vec<OwnedObjectPath> {
        let m = self.model.lock().unwrap();
        if !m.wifi_enabled || !m.hardware_enabled {
            return Vec::new();
        }
        m.points.iter().map(|p| path(&p.path)).collect()
    }

    #[zbus(property)]
    fn active_access_point(&self) -> OwnedObjectPath {
        let m = self.model.lock().unwrap();
        m.active
            .map(|at| path(&m.points[at].path))
            .unwrap_or_else(|| path("/"))
    }

    fn get_all_access_points(&self) -> Vec<OwnedObjectPath> {
        self.access_points()
    }

    fn request_scan(&self, _options: HashMap<String, OwnedValue>) {
        let scans = {
            let mut m = self.model.lock().unwrap();
            m.scans += 1;
            m.save();
            m.scans
        };
        // It ends a moment later, as NetworkManager says: its `LastScan`, alone.
        std::thread::spawn(move || {
            std::thread::sleep(STEP);
            let invalidated: Vec<&str> = Vec::new();
            let _ = bus().emit_signal(
                None::<&str>,
                WIFI_DEVICE,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &(
                    "org.freedesktop.NetworkManager.Device.Wireless",
                    HashMap::from([("LastScan", Value::from(i64::from(scans) * 1000))]),
                    &invalidated,
                ),
            );
        });
    }

    #[zbus(property)]
    fn last_scan(&self) -> i64 {
        i64::from(self.model.lock().unwrap().scans) * 1000
    }

    #[zbus(property)]
    fn perm_hw_address(&self) -> String {
        PERMANENT.into()
    }

    /// The rate it sends at to the access point joined, kb/s.
    #[zbus(property)]
    fn bitrate(&self) -> u32 {
        let m = self.model.lock().unwrap();
        m.active.and_then(|at| m.points[at].spec.rate).unwrap_or(0) * 1000
    }
}

/// An activation's object: the connection it is of.
struct Active {
    model: Shared,
    number: u32,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl Active {
    #[zbus(property)]
    fn connection(&self) -> OwnedObjectPath {
        let _ = &self.model;
        path(&format!("{NM_PATH}/Settings/{}", self.number))
    }
}

/// The DNS configuration the daemon applied: each device up's servers.
struct DnsManager {
    model: Shared,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.DnsManager")]
impl DnsManager {
    #[zbus(property)]
    fn configuration(&self) -> Vec<HashMap<String, OwnedValue>> {
        let m = self.model.lock().unwrap();
        let entry = |server: &str, interface: &str, priority: i32| {
            HashMap::from([
                (
                    "nameservers".to_string(),
                    owned(Value::from(vec![server.to_string()])),
                ),
                (
                    "interface".to_string(),
                    owned(Value::from(interface.to_string())),
                ),
                ("priority".to_string(), owned(Value::from(priority))),
                ("vpn".to_string(), owned(Value::from(false))),
            ])
        };
        let mut configuration = Vec::new();
        if m.ethernet_state == 100 {
            configuration.push(entry("10.0.0.1", "enp0s31f6", 100));
        }
        if m.wifi_state == 100 {
            configuration.push(entry("192.168.8.1", "wlp0s20f3", 600));
        }
        configuration
    }
}

struct Wired {
    model: Shared,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wired")]
impl Wired {
    #[zbus(property)]
    fn carrier(&self) -> bool {
        self.model.lock().unwrap().carrier.unwrap_or(false)
    }

    #[zbus(property)]
    fn speed(&self) -> u32 {
        if self.carrier() {
            1000
        } else {
            0
        }
    }
}

struct AccessPoint {
    model: Shared,
    path: String,
}

impl AccessPoint {
    fn spec(&self) -> Option<PointSpec> {
        let m = self.model.lock().unwrap();
        m.points
            .iter()
            .find(|p| p.path == self.path)
            .map(|p| p.spec.clone())
    }
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
impl AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> Vec<u8> {
        self.spec().map(|s| s.ssid.into_bytes()).unwrap_or_default()
    }

    #[zbus(property)]
    fn strength(&self) -> u8 {
        self.spec().map_or(0, |s| s.strength)
    }

    #[zbus(property)]
    fn flags(&self) -> u32 {
        self.spec().map_or(0, |s| flags(&s.security).0)
    }

    #[zbus(property)]
    fn wpa_flags(&self) -> u32 {
        self.spec().map_or(0, |s| flags(&s.security).1)
    }

    #[zbus(property)]
    fn rsn_flags(&self) -> u32 {
        self.spec().map_or(0, |s| flags(&s.security).2)
    }

    #[zbus(property)]
    fn hw_address(&self) -> String {
        let number = self.path.rsplit('/').next().unwrap_or("0");
        self.spec()
            .and_then(|s| s.bssid)
            .unwrap_or_else(|| format!("A0:B1:C2:00:00:{:02X}", number.parse::<u8>().unwrap_or(0)))
    }

    #[zbus(property)]
    fn frequency(&self) -> u32 {
        self.spec().and_then(|s| s.frequency).unwrap_or(2437)
    }

    #[zbus(property)]
    fn max_bitrate(&self) -> u32 {
        self.spec().and_then(|s| s.max_rate).unwrap_or(0) * 1000
    }

    #[zbus(property)]
    fn bandwidth(&self) -> u32 {
        self.spec().and_then(|s| s.width).unwrap_or(0)
    }
}

struct Ip4 {
    wifi: bool,
}

fn address(text: &str, prefix: u32) -> HashMap<String, OwnedValue> {
    HashMap::from([
        ("address".to_string(), owned(Value::from(text.to_string()))),
        ("prefix".to_string(), owned(Value::from(prefix))),
    ])
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.IP4Config")]
impl Ip4 {
    #[zbus(property)]
    fn address_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![address(
            if self.wifi {
                "192.168.8.133"
            } else {
                "10.0.0.20"
            },
            24,
        )]
    }

    #[zbus(property)]
    fn gateway(&self) -> String {
        if self.wifi { "192.168.8.1" } else { "10.0.0.1" }.into()
    }

    #[zbus(property)]
    fn nameserver_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![HashMap::from([(
            "address".to_string(),
            owned(Value::from(if self.wifi {
                "192.168.8.1"
            } else {
                "10.0.0.1"
            })),
        )])]
    }
}

struct Ip6 {
    wifi: bool,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.IP6Config")]
impl Ip6 {
    #[zbus(property)]
    fn address_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![
            address("fe80::1234:5678:9abc:def0", 64),
            address(
                if self.wifi {
                    "2001:db8::133"
                } else {
                    "2001:db8:1::20"
                },
                64,
            ),
        ]
    }

    #[zbus(property)]
    fn nameservers(&self) -> Vec<Vec<u8>> {
        Vec::new()
    }
}

struct Control {
    model: Shared,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Act {
    /// "plugged" or "unplugged".
    Ethernet(String),
    AccessPoint(PointSpec),
    RemoveAccessPoint(String),
    HardwareEnabled(bool),
    /// An access point's strength, as NetworkManager signals it: its `Strength` alone (with
    /// `LastSeen`, as NetworkManager's scans say it).
    Strength(StrengthSpec),
    /// An access point seen again by a scan: its `LastSeen` alone, nothing else changed.
    Seen(String),
    /// The computer started again: another boot's id (`${BOOT}`), for the addresses of the next
    /// joins.
    Restart {},
    /// The network behind the daemon changed, for diagnostics: the router, the resolver, the
    /// daemon's connectivity check.
    Router(RouterSpec),
    Resolver(ResolverSpec),
    Connectivity(ConnectivitySpec),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrengthSpec {
    ssid: String,
    strength: u8,
}

/// Each access point named `ssid`: `PropertiesChanged` on it with `changes` alone, as
/// NetworkManager tells what a scan changed.
fn point_changed(model: &Shared, ssid: &str, changes: HashMap<&str, Value>) {
    let paths: Vec<String> = model
        .lock()
        .unwrap()
        .points
        .iter()
        .filter(|point| point.spec.ssid == ssid)
        .map(|point| point.path.clone())
        .collect();
    let invalidated: Vec<&str> = Vec::new();
    for path in paths {
        let _ = bus().emit_signal(
            None::<&str>,
            path.as_str(),
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &(
                "org.freedesktop.NetworkManager.AccessPoint",
                changes.clone(),
                &invalidated,
            ),
        );
    }
}

#[zbus::interface(name = "so.oxy.OxyOS.FakeNetworkManager")]
impl Control {
    /// Changes the machine: `{"ethernet":"plugged"}`, `{"access_point":{…}}`,
    /// `{"remove_access_point":"ssid"}`, `{"hardware_enabled":false}`,
    /// `{"strength":{"ssid":…,"strength":N}}`, `{"seen":"ssid"}`, `{"restart":{}}`, and the
    /// network behind it: `{"router":{…}}`, `{"resolver":{…}}`, `{"connectivity":{…}}`.
    fn act(&self, what: String) -> zbus::fdo::Result<()> {
        let act: Act = serde_json::from_str(&what)
            .map_err(|error| zbus::fdo::Error::InvalidArgs(error.to_string()))?;
        let model = self.model.clone();
        std::thread::spawn(move || match act {
            Act::Ethernet(how) => {
                let plugged = how == "plugged";
                model.lock().unwrap().carrier = Some(plugged);
                changed(
                    ETHERNET_DEVICE,
                    "org.freedesktop.NetworkManager.Device.Wired",
                );
                if plugged {
                    for state in [30, 40, 70, 100] {
                        device_state(&model, ETHERNET_DEVICE, state, 0);
                        std::thread::sleep(STEP);
                    }
                } else {
                    device_state(&model, ETHERNET_DEVICE, 20, 40);
                }
            }
            Act::AccessPoint(spec) => {
                let path = model.lock().unwrap().add_point(spec);
                let _ = bus().object_server().at(
                    path.clone(),
                    AccessPoint {
                        model: model.clone(),
                        path,
                    },
                );
                changed(
                    WIFI_DEVICE,
                    "org.freedesktop.NetworkManager.Device.Wireless",
                );
            }
            Act::RemoveAccessPoint(ssid) => {
                {
                    let mut m = model.lock().unwrap();
                    let active = m.active.map(|at| m.points[at].path.clone());
                    m.points.retain(|p| p.spec.ssid != ssid);
                    m.active = active.and_then(|a| m.points.iter().position(|p| p.path == a));
                }
                changed(
                    WIFI_DEVICE,
                    "org.freedesktop.NetworkManager.Device.Wireless",
                );
            }
            Act::HardwareEnabled(enabled) => {
                model.lock().unwrap().hardware_enabled = enabled;
                changed(NM_PATH, "org.freedesktop.NetworkManager");
            }
            Act::Strength(spec) => {
                for point in model.lock().unwrap().points.iter_mut() {
                    if point.spec.ssid == spec.ssid {
                        point.spec.strength = spec.strength;
                    }
                }
                point_changed(
                    &model,
                    &spec.ssid,
                    HashMap::from([
                        ("Strength", Value::from(spec.strength)),
                        ("LastSeen", Value::from(1000i32)),
                    ]),
                );
            }
            Act::Seen(ssid) => {
                point_changed(
                    &model,
                    &ssid,
                    HashMap::from([("LastSeen", Value::from(1000i32))]),
                );
            }
            Act::Restart {} => {
                let mut m = model.lock().unwrap();
                m.boot += 1;
                m.save();
            }
            Act::Router(router) => model.lock().unwrap().router = router,
            Act::Resolver(resolver) => model.lock().unwrap().resolver = resolver,
            Act::Connectivity(connectivity) => model.lock().unwrap().connectivity = connectivity,
        });
        Ok(())
    }

    /// What the network behind the daemon does for Wi-Fi Diagnostics, counted:
    /// `{"ping":ADDRESS,"count":N}` (`{"rtts_ms":[…]}`, or `{"not_allowed":true}`),
    /// `{"neighbour":ADDRESS}` (`{"neighbour":bool}`) and `{"resolve":NAME}`
    /// (`{"addresses":N,"ms":N}`, `{"failed":true,"ms":N}` or `{"timed_out":true}`).
    fn probe(&self, what: String) -> zbus::fdo::Result<String> {
        let asked: Json = serde_json::from_str(&what)
            .map_err(|error| zbus::fdo::Error::InvalidArgs(error.to_string()))?;
        let mut m = self.model.lock().unwrap();
        let answer = if let Some(address) = asked.get("ping").and_then(Json::as_str) {
            let count = asked.get("count").and_then(Json::as_u64).unwrap_or(1) as usize;
            m.probes.push(format!("ping {address}"));
            let reached = m.wifi_state == 100 && address == "192.168.8.1";
            if !m.router.allowed {
                json!({"not_allowed": true})
            } else if reached && m.router.answers {
                let times: Vec<f64> = m.router.ms.iter().copied().cycle().take(count).collect();
                json!({"rtts_ms": times})
            } else {
                json!({"rtts_ms": []})
            }
        } else if let Some(address) = asked.get("neighbour").and_then(Json::as_str) {
            m.probes.push(format!("neighbour {address}"));
            json!({"neighbour": m.wifi_state == 100 && address == "192.168.8.1" && m.router.neighbour})
        } else if let Some(name) = asked.get("resolve").and_then(Json::as_str) {
            m.probes.push(format!("resolve {name}"));
            match m.resolver.does.as_str() {
                "answers" => json!({"addresses": m.resolver.addresses, "ms": m.resolver.ms}),
                "fails" => json!({"failed": true, "ms": m.resolver.ms}),
                _ => json!({"timed_out": true}),
            }
        } else {
            return Err(zbus::fdo::Error::InvalidArgs(what));
        };
        m.save();
        Ok(answer.to_string())
    }

    /// What the daemon holds: its connections (with their passwords), what was tried, the scans.
    fn state(&self) -> String {
        self.model.lock().unwrap().describe().to_string()
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut address, mut machine, mut state_file) = (None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--address" => address = args.next(),
            "--machine" => machine = args.next(),
            "--state" => state_file = args.next(),
            _ => {
                eprintln!("usage: oxyos-fake-networkmanager --address ADDRESS --machine FILE [--state FILE]");
                std::process::exit(2);
            }
        }
    }
    let (Some(address), Some(machine)) = (address, machine) else {
        eprintln!(
            "usage: oxyos-fake-networkmanager --address ADDRESS --machine FILE [--state FILE]"
        );
        std::process::exit(2);
    };
    let spec: Machine =
        serde_json::from_str(&std::fs::read_to_string(&machine).expect("the machine"))
            .expect("the machine's JSON");
    let mut model = Model {
        wifi: spec.wifi.unwrap_or(true),
        wifi_enabled: spec.wifi_enabled.unwrap_or(true),
        hardware_enabled: spec.hardware_enabled.unwrap_or(true),
        wifi_state: 30,
        active: None,
        points: Vec::new(),
        next_point: 0,
        connections: Vec::new(),
        next_connection: 0,
        carrier: match spec.ethernet.as_deref().unwrap_or("none") {
            "none" => None,
            "connected" => Some(true),
            _ => Some(false),
        },
        ethernet_state: 20,
        deny_system: spec.deny_system,
        scans: 0,
        attempts: Vec::new(),
        activation: 0,
        state_file,
        active_connection: None,
        address: PERMANENT.into(),
        boot: 1,
        router: spec.router.clone(),
        resolver: spec.resolver.clone(),
        connectivity: spec.connectivity.clone(),
        probes: Vec::new(),
        checks: 0,
    };
    if model.carrier == Some(true) {
        model.ethernet_state = 100;
    }
    if !model.wifi_enabled || !model.hardware_enabled {
        model.wifi_state = 20;
    }
    for point in spec.access_points {
        model.add_point(point);
    }
    for connection in &spec.connections {
        let key_mgmt = model
            .points
            .iter()
            .find(|p| p.spec.ssid == connection.ssid)
            .and_then(|p| match p.spec.security.as_str() {
                "wpa" => Some("wpa-psk".to_string()),
                "wpa3" => Some("sae".to_string()),
                _ => None,
            });
        let at = model.add_connection(&connection.ssid, key_mgmt, connection.password.clone());
        let saved = &mut model.connections[at];
        saved.registered = true;
        saved.permissions = connection.permissions.clone();
        saved.assigned = connection.assigned.clone();
        saved.stable_id = connection.stable_id.clone();
        saved.flags = connection.flags;
        match connection.kind.as_deref().unwrap_or("wifi") {
            "wifi" => {}
            kind => {
                saved.kind = match kind {
                    "ethernet" => "802-3-ethernet",
                    "wireguard" => "wireguard",
                    _ => "vpn",
                }
                .into();
                saved.name = connection.name.clone().unwrap_or_default();
                saved.ssid = String::new();
                saved.key_mgmt = None;
            }
        }
    }
    if let Some(active) = &spec.active {
        if model.wifi_enabled {
            model.active = model.points.iter().position(|p| &p.spec.ssid == active);
            if model.active.is_some() {
                model.wifi_state = 100;
                let joined = model
                    .connections
                    .iter()
                    .find(|c| c.kind == "802-11-wireless" && &c.ssid == active)
                    .map(|c| (c.number, model.address_of(c)));
                if let Some((number, address)) = joined {
                    model.active_connection = Some(number);
                    model.address = address;
                }
            }
        }
    }
    let model: Shared = Arc::new(Mutex::new(model));
    let mut builder = zbus::blocking::connection::Builder::address(address.as_str())
        .expect("an address")
        .name("org.freedesktop.NetworkManager")
        .expect("the name")
        .serve_at(
            NM_PATH,
            Manager {
                model: model.clone(),
            },
        )
        .expect("served")
        .serve_at(
            format!("{NM_PATH}/Settings"),
            SettingsRoot {
                model: model.clone(),
            },
        )
        .expect("served")
        .serve_at(
            WIFI_DEVICE,
            Device {
                model: model.clone(),
                wifi: true,
            },
        )
        .expect("served")
        .serve_at(
            WIFI_DEVICE,
            Wireless {
                model: model.clone(),
            },
        )
        .expect("served")
        .serve_at(
            ETHERNET_DEVICE,
            Device {
                model: model.clone(),
                wifi: false,
            },
        )
        .expect("served")
        .serve_at(
            ETHERNET_DEVICE,
            Wired {
                model: model.clone(),
            },
        )
        .expect("served")
        .serve_at(
            "/so/oxy/OxyOS/FakeNetworkManager",
            Control {
                model: model.clone(),
            },
        )
        .expect("served")
        .serve_at(
            format!("{NM_PATH}/DnsManager"),
            DnsManager {
                model: model.clone(),
            },
        )
        .expect("served");
    for (number, wifi) in [(2, true), (1, false)] {
        builder = builder
            .serve_at(format!("{NM_PATH}/IP4Config/{number}"), Ip4 { wifi })
            .expect("served")
            .serve_at(format!("{NM_PATH}/IP6Config/{number}"), Ip6 { wifi })
            .expect("served");
    }
    {
        let m = model.lock().unwrap();
        for point in &m.points {
            builder = builder
                .serve_at(
                    point.path.clone(),
                    AccessPoint {
                        model: model.clone(),
                        path: point.path.clone(),
                    },
                )
                .expect("served");
        }
        for connection in &m.connections {
            builder = builder
                .serve_at(
                    connection.path(),
                    SettingsConnection {
                        model: model.clone(),
                        number: connection.number,
                    },
                )
                .expect("served");
        }
        if let Some(number) = m.active_connection {
            builder = builder
                .serve_at(
                    format!("{NM_PATH}/ActiveConnection/{number}"),
                    Active {
                        model: model.clone(),
                        number,
                    },
                )
                .expect("served");
        }
    }
    let connection = builder.build().expect("on the bus");
    let _ = BUS.set(connection);
    model.lock().unwrap().save();
    loop {
        std::thread::park();
    }
}
