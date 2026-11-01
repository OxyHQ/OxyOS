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
}

fn open() -> String {
    "open".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionSpec {
    ssid: String,
    #[serde(default)]
    password: Option<String>,
}

struct Point {
    spec: PointSpec,
    path: String,
}

struct Connection {
    number: u32,
    uuid: String,
    ssid: String,
    key_mgmt: Option<String>,
    psk: Option<String>,
    permissions: Vec<String>,
    autoconnect: bool,
    registered: bool,
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
                "permissions": c.permissions,
            })).collect::<Vec<_>>(),
            "scans": self.scans,
            "attempts": self.attempts,
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
            key_mgmt,
            psk,
            permissions: Vec::new(),
            autoconnect: true,
            registered: false,
        });
        self.connections.len() - 1
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

/// Joins the access point `point` with the connection numbered `number`, through the device's
/// states as NetworkManager's: a protected network whose password is not the access point's fails
/// for want of secrets.
fn activate(model: Shared, number: u32, point: usize) {
    use std::thread::sleep;
    let (activation, was_up, accepted) = {
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
        let attempt = json!({"ssid": spec.ssid, "psk": connection.psk});
        m.attempts.push(attempt);
        (m.activation, m.wifi_state == 100, accepted)
    };
    let current = |model: &Shared| model.lock().unwrap().activation == activation;
    if was_up {
        model.lock().unwrap().active = None;
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
    if accepted {
        device_state(&model, WIFI_DEVICE, 70, 0);
        sleep(STEP);
        if !current(&model) {
            return;
        }
        model.lock().unwrap().active = Some(point);
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
        settings.insert(
            "connection".into(),
            HashMap::from([
                ("id".to_string(), owned(Value::from(c.ssid.clone()))),
                ("uuid".to_string(), owned(Value::from(c.uuid.clone()))),
                ("type".to_string(), owned(Value::from("802-11-wireless"))),
                ("autoconnect".to_string(), owned(Value::from(c.autoconnect))),
                (
                    "permissions".to_string(),
                    owned(Value::from(c.permissions.clone())),
                ),
            ]),
        );
        settings.insert(
            "802-11-wireless".into(),
            HashMap::from([
                (
                    "ssid".to_string(),
                    owned(Value::from(c.ssid.as_bytes().to_vec())),
                ),
                ("mode".to_string(), owned(Value::from("infrastructure"))),
            ]),
        );
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
        let text = |key: &str| -> Option<String> {
            String::try_from(
                settings
                    .get("802-11-wireless-security")?
                    .get(key)?
                    .try_clone()
                    .ok()?,
            )
            .ok()
        };
        c.psk = text("psk");
        c.key_mgmt = text("key-mgmt");
        m.save();
        Ok(())
    }

    fn delete(&self) -> Result<(), NmError> {
        let path = {
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
            let path = m.connections.remove(at).path();
            m.save();
            path
        };
        let model = self.model.clone();
        std::thread::spawn(move || {
            let _ = bus()
                .object_server()
                .remove::<SettingsConnection, _>(path.as_str());
            changed(NM_PATH, "org.freedesktop.NetworkManager");
            drop(model);
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

    #[zbus(property)]
    fn hw_address(&self) -> String {
        if self.wifi {
            "F4:A4:75:12:34:56"
        } else {
            "38:F3:AB:65:43:21"
        }
        .into()
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
        let mut m = self.model.lock().unwrap();
        m.scans += 1;
        m.save();
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
}

#[zbus::interface(name = "so.oxy.OxyOS.FakeNetworkManager")]
impl Control {
    /// Changes the machine: `{"ethernet":"plugged"}`, `{"access_point":{…}}`,
    /// `{"remove_access_point":"ssid"}`, `{"hardware_enabled":false}`.
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
        });
        Ok(())
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
        model.connections[at].registered = true;
    }
    if let Some(active) = &spec.active {
        if model.wifi_enabled {
            model.active = model.points.iter().position(|p| &p.spec.ssid == active);
            if model.active.is_some() {
                model.wifi_state = 100;
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
    }
    let connection = builder.build().expect("on the bus");
    let _ = BUS.set(connection);
    model.lock().unwrap().save();
    loop {
        std::thread::park();
    }
}
