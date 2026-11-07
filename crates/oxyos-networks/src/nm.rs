//! NetworkManager's client (docs/adr/0038-networks.mdx, decision 2), on the system bus or, for
//! tests and the development compositor, on a bus of their own where a played daemon answers.
//! Every property is read when asked (nothing is cached), so what it says is what the daemon says.

use crate::secret::Secret;
use crate::{
    device_state, ethernet_state, networks, plan, secrets_reason, security, ssid_text, subnet_mask,
    wifi_state, AccessPoint, Details, Ethernet, Failure, Ipv4, Known, Plan, State, Wifi, WifiState,
};
use futures_lite::StreamExt;
use std::collections::HashMap;
use std::io;
use std::os::fd::BorrowedFd;
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const SETTINGS: &str = "org.freedesktop.NetworkManager.Settings";
const CONNECTION: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const WIRED: &str = "org.freedesktop.NetworkManager.Device.Wired";
const ACCESS_POINT: &str = "org.freedesktop.NetworkManager.AccessPoint";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// The longest a scan asked for is waited for (a card that never says it ended, a scan not
/// allowed so soon after another): its access points are then read as they are.
const SCAN_LIMIT: Duration = Duration::from_secs(10);
const IP4: &str = "org.freedesktop.NetworkManager.IP4Config";
const IP6: &str = "org.freedesktop.NetworkManager.IP6Config";
const TYPE_ETHERNET: u32 = 1;
const TYPE_WIFI: u32 = 2;
const WIFI_TYPE: &str = "802-11-wireless";
/// How long a join may take before it is given up, as a Mac gives up.
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(45);

type Settings = HashMap<String, HashMap<String, OwnedValue>>;

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

/// Whether a call failed because nobody provides the service: a machine without the daemon.
fn absent(error: &zbus::Error) -> bool {
    let name = match error {
        zbus::Error::MethodError(name, _, _) => name.as_str().to_string(),
        zbus::Error::FDO(error) => {
            return matches!(
                **error,
                zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)
            )
        }
        _ => return false,
    };
    matches!(
        name.as_str(),
        "org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner"
    )
}

/// Whether polkit (through the daemon) refused a call.
fn refused(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, _, _) => name.as_str().ends_with(".PermissionDenied"),
        zbus::Error::FDO(error) => matches!(**error, zbus::fdo::Error::AccessDenied(_)),
        _ => false,
    }
}

fn failure(error: zbus::Error) -> Failure {
    if refused(&error) {
        Failure::NotPermitted
    } else if absent(&error) {
        Failure::Unavailable
    } else {
        Failure::Failed
    }
}

/// The session's user's name, for a connection only it may use.
fn user_name() -> Option<String> {
    if let Some(name) = std::env::var("USER").ok().filter(|name| !name.is_empty()) {
        return Some(name);
    }
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let uid = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .to_string();
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (fields.len() > 2 && fields[2] == uid).then(|| fields[0].to_string())
    })
}

/// A device as read.
struct Device {
    path: OwnedObjectPath,
    kind: u32,
    state: u32,
}

/// The Wi-Fi device's access point of a name, the strongest.
struct Target {
    device: OwnedObjectPath,
    point: OwnedObjectPath,
}

/// NetworkManager on a bus.
pub struct NetworkManager {
    bus: Connection,
}

impl NetworkManager {
    /// On the system bus, or on the bus at `address` (a played daemon's).
    pub fn connect(address: Option<&str>) -> io::Result<Self> {
        let bus = match address {
            Some(address) => zbus::blocking::connection::Builder::address(address)
                .map_err(failed)?
                .build()
                .map_err(failed)?,
            None => Connection::system().map_err(failed)?,
        };
        Ok(Self { bus })
    }

    fn proxy(&self, path: &str, interface: &'static str) -> zbus::Result<Proxy<'static>> {
        zbus::blocking::proxy::Builder::<Proxy<'static>>::new(&self.bus)
            .destination(NM)?
            .path(path.to_string())?
            .interface(interface)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
    }

    fn property<T>(&self, path: &str, interface: &'static str, name: &str) -> zbus::Result<T>
    where
        T: TryFrom<OwnedValue>,
        T::Error: Into<zbus::Error>,
    {
        self.proxy(path, interface)?.get_property(name)
    }

    fn devices(&self) -> zbus::Result<Vec<Device>> {
        let paths: Vec<OwnedObjectPath> = self.property(NM_PATH, NM, "Devices")?;
        let mut devices = Vec::new();
        for path in paths {
            let kind: u32 = self.property(&path, DEVICE, "DeviceType")?;
            if kind != TYPE_WIFI && kind != TYPE_ETHERNET {
                continue;
            }
            let state: u32 = self.property(&path, DEVICE, "State")?;
            devices.push(Device { path, kind, state });
        }
        Ok(devices)
    }

    /// The Wi-Fi device: the first, or the one that is up.
    fn wifi_device(devices: &[Device]) -> Option<&Device> {
        let wifi = devices.iter().filter(|d| d.kind == TYPE_WIFI);
        wifi.clone()
            .find(|d| d.state == device_state::ACTIVATED)
            .or_else(|| wifi.clone().next())
    }

    fn access_points(&self, device: &str) -> zbus::Result<Vec<(OwnedObjectPath, AccessPoint)>> {
        let paths: Vec<OwnedObjectPath> = self.property(device, WIRELESS, "AccessPoints")?;
        let mut points = Vec::new();
        for path in paths {
            // An access point gone between the list and its read is not there.
            let read = || -> zbus::Result<AccessPoint> {
                let bytes: Vec<u8> = self.property(&path, ACCESS_POINT, "Ssid")?;
                let signal: u8 = self.property(&path, ACCESS_POINT, "Strength")?;
                let flags: u32 = self.property(&path, ACCESS_POINT, "Flags")?;
                let wpa: u32 = self.property(&path, ACCESS_POINT, "WpaFlags")?;
                let rsn: u32 = self.property(&path, ACCESS_POINT, "RsnFlags")?;
                Ok(AccessPoint {
                    ssid: ssid_text(&bytes),
                    signal: signal.min(100),
                    security: security(flags, wpa, rsn),
                })
            };
            if let Ok(point) = read() {
                points.push((path, point));
            }
        }
        Ok(points)
    }

    /// The saved Wi-Fi connections: their paths, and what they are.
    fn known(&self) -> zbus::Result<Vec<(OwnedObjectPath, Known)>> {
        let paths: Vec<OwnedObjectPath> = self
            .proxy(SETTINGS_PATH, SETTINGS)?
            .call("ListConnections", &())?;
        let mut known = Vec::new();
        for path in paths {
            let Ok(settings) = self
                .proxy(&path, CONNECTION)
                .and_then(|connection| connection.call::<_, _, Settings>("GetSettings", &()))
            else {
                continue;
            };
            if let Some(entry) = known_of(&settings) {
                known.push((path, entry));
            }
        }
        known.sort_by(|a, b| a.1.ssid.cmp(&b.1.ssid));
        Ok(known)
    }

    fn details(&self, device: &str) -> zbus::Result<Details> {
        let interface: String = self.property(device, DEVICE, "Interface")?;
        let hardware_address: String = self
            .property(device, DEVICE, "HwAddress")
            .unwrap_or_default();
        let mut details = Details {
            interface,
            hardware_address,
            ..Details::default()
        };
        let ip4: OwnedObjectPath = self.property(device, DEVICE, "Ip4Config")?;
        if ip4.as_str() != "/" {
            let addresses: Vec<HashMap<String, OwnedValue>> =
                self.property(&ip4, IP4, "AddressData")?;
            details.ipv4 = addresses.first().and_then(|address| {
                let text = string_of(address.get("address")?)?;
                let prefix = u32_of(address.get("prefix")?)?;
                Some(Ipv4 {
                    address: text,
                    prefix,
                    subnet_mask: subnet_mask(prefix),
                })
            });
            let gateway: String = self.property(&ip4, IP4, "Gateway").unwrap_or_default();
            details.router = (!gateway.is_empty()).then_some(gateway);
            let servers: Vec<HashMap<String, OwnedValue>> = self
                .property(&ip4, IP4, "NameserverData")
                .unwrap_or_default();
            details.dns = servers
                .iter()
                .filter_map(|server| string_of(server.get("address")?))
                .collect();
        }
        let ip6: OwnedObjectPath = self.property(device, DEVICE, "Ip6Config")?;
        if ip6.as_str() != "/" {
            let addresses: Vec<HashMap<String, OwnedValue>> =
                self.property(&ip6, IP6, "AddressData").unwrap_or_default();
            details.ipv6 = addresses
                .iter()
                .filter_map(|address| string_of(address.get("address")?))
                .filter(|address| !address.to_ascii_lowercase().starts_with("fe80"))
                .collect();
            let servers: Vec<Vec<u8>> = self.property(&ip6, IP6, "Nameservers").unwrap_or_default();
            details.dns.extend(servers.iter().filter_map(|bytes| {
                let octets: [u8; 16] = bytes.as_slice().try_into().ok()?;
                Some(std::net::Ipv6Addr::from(octets).to_string())
            }));
        }
        Ok(details)
    }

    /// The network now. Without the daemon, [`State::unavailable`].
    pub fn state(&self) -> io::Result<State> {
        match self.read() {
            Ok(state) => Ok(state),
            Err(error) if absent(&error) => Ok(State::unavailable()),
            Err(error) => Err(failed(error)),
        }
    }

    fn read(&self) -> zbus::Result<State> {
        let enabled: bool = self.property(NM_PATH, NM, "WirelessEnabled")?;
        let hardware: bool = self.property(NM_PATH, NM, "WirelessHardwareEnabled")?;
        let devices = self.devices()?;
        let known: Vec<Known> = self.known()?.into_iter().map(|(_, k)| k).collect();
        let mut wifi = Wifi {
            enabled,
            hardware_blocked: !hardware,
            known: known.clone(),
            ..Wifi::default()
        };
        if let Some(device) = Self::wifi_device(&devices) {
            wifi.device = true;
            wifi.state = wifi_state(enabled, hardware, Some(device.state));
            if !matches!(wifi.state, WifiState::Off) {
                let active: OwnedObjectPath =
                    self.property(&device.path, WIRELESS, "ActiveAccessPoint")?;
                let joined = matches!(wifi.state, WifiState::Connected | WifiState::Connecting)
                    && active.as_str() != "/";
                wifi.point = joined.then(|| active.to_string());
                let current = if joined {
                    let bytes: Vec<u8> = self.property(&active, ACCESS_POINT, "Ssid")?;
                    Some(ssid_text(&bytes))
                } else {
                    None
                };
                wifi.points = self
                    .access_points(&device.path)?
                    .into_iter()
                    .map(|(path, point)| (path.to_string(), point))
                    .collect();
                let points: Vec<AccessPoint> =
                    wifi.points.iter().map(|(_, point)| point.clone()).collect();
                wifi.networks = networks(&points, &known, current.as_deref());
                wifi.current = wifi.networks.iter().find(|n| n.current).cloned();
                if wifi.state == WifiState::Connected {
                    wifi.details = self.details(&device.path).ok();
                }
            }
        }
        let mut ethernet = Ethernet::default();
        let wired: Vec<&Device> = devices.iter().filter(|d| d.kind == TYPE_ETHERNET).collect();
        let mut best: Option<(u8, &Device, bool)> = None;
        for device in wired {
            let carrier: bool = self
                .property(&device.path, WIRED, "Carrier")
                .unwrap_or(false);
            // The one up, else one with a cable, else the first.
            let rank = if device.state == device_state::ACTIVATED {
                2
            } else {
                u8::from(carrier)
            };
            if best.is_none_or(|(seen, _, _)| rank > seen) {
                best = Some((rank, device, carrier));
            }
        }
        if let Some((_, device, carrier)) = best {
            ethernet.device = true;
            ethernet.state = ethernet_state(device.state, carrier);
            ethernet.speed = if carrier {
                self.property(&device.path, WIRED, "Speed").unwrap_or(0)
            } else {
                0
            };
            if device.state == device_state::ACTIVATED {
                ethernet.details = self.details(&device.path).ok();
            }
        }
        Ok(State {
            available: true,
            wifi,
            ethernet,
            joining: None,
        })
    }

    /// Asks the Wi-Fi device to look for networks and returns once the scan ended (the device's
    /// `LastScan` changed), or after [`SCAN_LIMIT`]: its networks are then read whole by the caller,
    /// as the watch follows no scan's access points. The daemon refuses scans too close together,
    /// which is no error to anyone; the one running then is waited for.
    pub fn scan(&self) -> io::Result<()> {
        let devices = self.devices().map_err(failed)?;
        let Some(device) = Self::wifi_device(&devices) else {
            return Ok(());
        };
        // Its end, listened for before it is asked so it is not missed: the device's `LastScan`.
        let rule = daemon_rule(
            Some(&device.path),
            PROPERTIES,
            "PropertiesChanged",
            Some(WIRELESS),
        )
        .map_err(failed)?;
        let mut device_changed = futures_lite::future::block_on(
            zbus::MessageStream::for_match_rule(rule, self.bus.inner(), Some(16)),
        )
        .map_err(failed)?;
        let options: HashMap<&str, Value> = HashMap::new();
        match self
            .proxy(&device.path, WIRELESS)
            .and_then(|wireless| wireless.call::<_, _, ()>("RequestScan", &(options,)))
        {
            // Not allowed: one is running, or one ended a moment ago; its end is waited for too.
            Ok(()) => {}
            Err(zbus::Error::MethodError(name, _, _)) if name.as_str().ends_with(".NotAllowed") => {
            }
            Err(error) => return Err(failed(error)),
        }
        futures_lite::future::block_on(futures_lite::future::or(
            async {
                while let Some(message) = device_changed.next().await {
                    let ended = message.ok().and_then(|message| {
                        message
                            .body()
                            .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                            .ok()
                    });
                    if ended.is_some_and(|(_, changed, _)| changed.contains_key("LastScan")) {
                        return;
                    }
                }
            },
            async {
                async_io::Timer::after(SCAN_LIMIT).await;
            },
        ));
        Ok(())
    }

    /// Turns Wi-Fi on or off, as `nmcli radio wifi` does.
    pub fn set_wifi(&self, enabled: bool) -> Result<(), Failure> {
        self.proxy(NM_PATH, NM)
            .and_then(|nm| {
                nm.set_property("WirelessEnabled", enabled)
                    .map_err(Into::into)
            })
            .map_err(failure)
    }

    /// Forgets a saved connection by its UUID.
    pub fn forget(&self, id: &str) -> Result<(), Failure> {
        let known = self.known().map_err(failure)?;
        let (path, _) = known
            .iter()
            .find(|(_, k)| k.id == id)
            .ok_or(Failure::NotFound)?;
        self.proxy(path, CONNECTION)
            .and_then(|connection| connection.call::<_, _, ()>("Delete", &()))
            .map_err(failure)
    }

    /// What joining `ssid` needs.
    pub fn plan(&self, ssid: &str) -> Result<Plan, Failure> {
        let state = self.read().map_err(failure)?;
        if !state.wifi.device || state.wifi.state == WifiState::Off {
            return Err(Failure::Unavailable);
        }
        plan(&state.wifi.networks, ssid)
    }

    fn target(&self, ssid: &str) -> Result<Target, Failure> {
        let devices = self.devices().map_err(failure)?;
        let device = Self::wifi_device(&devices).ok_or(Failure::Unavailable)?;
        let point = self
            .access_points(&device.path)
            .map_err(failure)?
            .into_iter()
            .filter(|(_, point)| point.ssid == ssid)
            .max_by_key(|(_, point)| point.signal)
            .ok_or(Failure::NotFound)?
            .0;
        Ok(Target {
            device: device.path.clone(),
            point,
        })
    }

    /// Joins `ssid` and waits until the daemon has joined it or given up: activated as saved when
    /// a connection has its name (with `password`, the saved one's password is replaced), else
    /// added (with `password` for a network that needs one) and activated. A connection added
    /// here that does not join is deleted again, so a wrong password is never kept.
    pub fn join(&self, ssid: &str, password: Option<&Secret>) -> Result<(), Failure> {
        let plan = self.plan(ssid)?;
        let security = match plan {
            Plan::Unsupported { .. } => return Err(Failure::Unsupported),
            Plan::Password { .. } if password.is_none() => return Err(Failure::NeedsPassword),
            Plan::Known { security } | Plan::Password { security } => Some(security),
            Plan::Open => None,
        };
        let target = self.target(ssid)?;
        // Already joined to it: nothing to do.
        let state: u32 = self
            .property(&target.device, DEVICE, "State")
            .map_err(failure)?;
        let active: OwnedObjectPath = self
            .property(&target.device, WIRELESS, "ActiveAccessPoint")
            .map_err(failure)?;
        if state == device_state::ACTIVATED && active.as_str() != "/" {
            let bytes: Vec<u8> = self
                .property(&active, ACCESS_POINT, "Ssid")
                .map_err(failure)?;
            if ssid_text(&bytes) == ssid && password.is_none() {
                return Ok(());
            }
        }
        // Its changes are followed before it is asked, so none is missed.
        let mut changes = self.device_changes(&target.device).map_err(failure)?;
        let known = self.known().map_err(failure)?;
        let saved = known.iter().find(|(_, k)| k.ssid == ssid).map(|(p, _)| p);
        let nm = self.proxy(NM_PATH, NM).map_err(failure)?;
        // A saved connection whose password is replaced keeps the one it had until the new one
        // joins: the daemon may have refused the old one for a moment's failure, not for being
        // wrong, and a mistyped replacement must not take its place.
        let mut restore: Option<(OwnedObjectPath, &str, Option<Secret>)> = None;
        let added = match saved {
            Some(connection) => {
                if let (Some(password), Some(key_mgmt)) =
                    (password, security.and_then(|s| s.key_mgmt()))
                {
                    let previous = self.saved_password(connection);
                    self.set_password(connection, key_mgmt, Some(password))?;
                    restore = Some((connection.clone(), key_mgmt, previous));
                }
                nm.call::<_, _, OwnedObjectPath>(
                    "ActivateConnection",
                    &(connection, &target.device, &target.point),
                )
                .map_err(failure)?;
                None
            }
            None => {
                let mut settings = new_connection(ssid, security, password);
                let added = match add_and_activate(&nm, &settings, &target) {
                    Err(error) if refused(&error) => {
                        // For this user alone (`settings.modify.own`), as GNOME does for a user
                        // who is not an administrator.
                        let user = user_name().ok_or(Failure::NotPermitted)?;
                        settings
                            .get_mut("connection")
                            .expect("built")
                            .insert("permissions", Value::from(vec![format!("user:{user}:")]));
                        add_and_activate(&nm, &settings, &target)
                    }
                    other => other,
                }
                .map_err(failure)?;
                Some(added)
            }
        };
        let outcome = wait(&mut changes, Instant::now() + JOIN_TIMEOUT, security);
        if outcome.is_err() {
            if let Some(added) = added {
                let _ = self
                    .proxy(&added, CONNECTION)
                    .and_then(|connection| connection.call::<_, _, ()>("Delete", &()));
            }
            if let Some((connection, key_mgmt, previous)) = restore {
                let _ = self.set_password(&connection, key_mgmt, previous.as_ref());
            }
        }
        outcome
    }

    /// The password a saved connection has, as the daemon gives it to whoever may change the
    /// connection (`GetSecrets`); none if it keeps none or does not say.
    fn saved_password(&self, connection: &OwnedObjectPath) -> Option<Secret> {
        let secrets: Settings = self
            .proxy(connection, CONNECTION)
            .ok()?
            .call("GetSecrets", &("802-11-wireless-security",))
            .ok()?;
        let psk = secrets.get("802-11-wireless-security")?.get("psk")?;
        Some(Secret::new(String::try_from(psk.try_clone().ok()?).ok()?))
    }

    /// Sets a saved connection's password to `password`, or to none.
    fn set_password(
        &self,
        connection: &OwnedObjectPath,
        key_mgmt: &str,
        password: Option<&Secret>,
    ) -> Result<(), Failure> {
        let proxy = self.proxy(connection, CONNECTION).map_err(failure)?;
        let saved: Settings = proxy.call("GetSettings", &()).map_err(failure)?;
        let mut settings: HashMap<String, HashMap<String, Value>> = HashMap::new();
        for (group, values) in saved {
            let values = values
                .into_iter()
                .map(|(key, value)| (key, Value::from(value)))
                .collect();
            settings.insert(group, values);
        }
        let security = settings
            .entry("802-11-wireless-security".to_string())
            .or_default();
        security.insert("key-mgmt".to_string(), Value::from(key_mgmt.to_string()));
        match password {
            Some(password) => {
                security.insert(
                    "psk".to_string(),
                    Value::from(password.expose().to_string()),
                );
                security.insert("psk-flags".to_string(), Value::from(0u32));
            }
            None => {
                security.remove("psk");
            }
        }
        proxy
            .call::<_, _, ()>("Update", &(settings,))
            .map_err(failure)
    }

    fn device_changes(&self, device: &OwnedObjectPath) -> zbus::Result<zbus::MessageStream> {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NM)?
            .path(device.as_str().to_string())?
            .interface(DEVICE)?
            .member("StateChanged")?
            .build();
        async_io::block_on(zbus::MessageStream::for_match_rule(
            rule,
            self.bus.inner(),
            Some(64),
        ))
    }
}

impl Drop for NetworkManager {
    /// Its connection closed as it goes: zbus closes a dropped connection's socket only once its
    /// reader notices, which may be never on a quiet bus.
    fn drop(&mut self) {
        let _ = self.bus.clone().close();
    }
}

/// A saved connection's entry if it is Wi-Fi.
fn known_of(settings: &Settings) -> Option<Known> {
    let connection = settings.get("connection")?;
    if connection.get("type").and_then(string_of).as_deref() != Some(WIFI_TYPE) {
        return None;
    }
    let ssid: Vec<u8> = settings
        .get(WIFI_TYPE)?
        .get("ssid")?
        .try_clone()
        .ok()?
        .try_into()
        .ok()?;
    Some(Known {
        id: connection.get("uuid").and_then(string_of)?,
        ssid: ssid_text(&ssid),
        auto_join: connection
            .get("autoconnect")
            .and_then(|value| bool::try_from(value.try_clone().ok()?).ok())
            .unwrap_or(true),
    })
}

fn string_of(value: &OwnedValue) -> Option<String> {
    String::try_from(value.try_clone().ok()?).ok()
}

fn u32_of(value: &OwnedValue) -> Option<u32> {
    u32::try_from(value.try_clone().ok()?).ok()
}

/// A new Wi-Fi connection's settings: its name, infrastructure, and the password the daemon
/// keeps (`psk-flags` 0: the daemon's to store).
fn new_connection(
    ssid: &str,
    security: Option<crate::Security>,
    password: Option<&Secret>,
) -> HashMap<&'static str, HashMap<&'static str, Value<'static>>> {
    let mut settings = HashMap::new();
    settings.insert(
        "connection",
        HashMap::from([
            ("id", Value::from(ssid.to_string())),
            ("type", Value::from(WIFI_TYPE)),
        ]),
    );
    settings.insert(
        WIFI_TYPE,
        HashMap::from([
            ("ssid", Value::from(ssid.as_bytes().to_vec())),
            ("mode", Value::from("infrastructure")),
        ]),
    );
    if let Some(key_mgmt) = security.and_then(|s| s.key_mgmt()) {
        let mut wireless_security = HashMap::from([("key-mgmt", Value::from(key_mgmt))]);
        if let Some(password) = password {
            wireless_security.insert("psk", Value::from(password.expose().to_string()));
            wireless_security.insert("psk-flags", Value::from(0u32));
        }
        settings.insert("802-11-wireless-security", wireless_security);
    }
    settings
}

fn add_and_activate(
    nm: &Proxy<'static>,
    settings: &HashMap<&'static str, HashMap<&'static str, Value<'static>>>,
    target: &Target,
) -> zbus::Result<OwnedObjectPath> {
    let (connection, _active): (OwnedObjectPath, OwnedObjectPath) = nm.call(
        "AddAndActivateConnection",
        &(settings, &target.device, &target.point),
    )?;
    Ok(connection)
}

/// Waits for the device's activation to end: activated, failed (a protected network's handshake
/// failing for want of secrets is a wrong password), or the deadline. The device's states before
/// the activation began (the previous network's going) are not its outcome.
fn wait(
    changes: &mut zbus::MessageStream,
    deadline: Instant,
    security: Option<crate::Security>,
) -> Result<(), Failure> {
    use device_state::*;
    enum Next {
        Message(Option<zbus::Result<zbus::Message>>),
        Late,
    }
    let protected = security.is_some_and(|s| s.needs_password());
    async_io::block_on(async {
        let mut begun = false;
        loop {
            let next =
                futures_lite::future::or(async { Next::Message(changes.next().await) }, async {
                    async_io::Timer::at(deadline).await;
                    Next::Late
                })
                .await;
            let message = match next {
                Next::Late => return Err(Failure::Timeout),
                Next::Message(None) => return Err(Failure::Failed),
                Next::Message(Some(Err(_))) => continue,
                Next::Message(Some(Ok(message))) => message,
            };
            let Ok((new, _old, reason)) = message.body().deserialize::<(u32, u32, u32)>() else {
                continue;
            };
            match new {
                ACTIVATED if begun => return Ok(()),
                PREPARE | CONFIG | NEED_AUTH | IP_CONFIG => begun = true,
                FAILED => {
                    return Err(if protected && secrets_reason(reason) {
                        Failure::WrongPassword
                    } else {
                        Failure::Failed
                    })
                }
                DISCONNECTED | UNAVAILABLE if begun => {
                    return Err(if protected && secrets_reason(reason) {
                        Failure::WrongPassword
                    } else {
                        Failure::Failed
                    })
                }
                _ => {}
            }
        }
    })
}

/// A connection made without an internal executor, its executor (what reads its socket) run by
/// whoever waits on it with futures-lite's block_on ([`ticked`]). Dropped, the connection closes
/// and its executor is run until its tasks end, as zbus's own executor thread would, so nothing
/// of it (its socket) outlives it.
pub(crate) struct Ticked {
    bus: Option<zbus::Connection>,
    executor: zbus::Executor<'static>,
}

impl Ticked {
    pub(crate) fn connect(builder: zbus::connection::Builder<'_>) -> io::Result<Self> {
        let bus = futures_lite::future::block_on(builder.internal_executor(false).build())
            .map_err(failed)?;
        let executor = bus.executor().clone();
        Ok(Self {
            bus: Some(bus),
            executor,
        })
    }

    pub(crate) fn bus(&self) -> &zbus::Connection {
        self.bus.as_ref().expect("until dropped")
    }

    /// `future`, the connection's executor run as it is waited for.
    pub(crate) fn wait<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        futures_lite::future::block_on(ticked(&self.executor, future))
    }
}

impl Drop for Ticked {
    fn drop(&mut self) {
        self.bus = None;
        futures_lite::future::block_on(async {
            while !self.executor.is_empty() {
                self.executor.tick().await;
            }
        });
    }
}

/// `future`, with `executor` ticked as it is waited for (a connection made without an internal
/// executor: its socket is read on the waiting thread).
pub(crate) async fn ticked<T>(
    executor: &zbus::Executor<'static>,
    future: impl std::future::Future<Output = T>,
) -> T {
    futures_lite::future::or(future, async {
        loop {
            executor.tick().await;
        }
    })
    .await
}

/// What changed, as a signal of the daemon's says: only access points' strengths (each access
/// point's object path and its strength), which [`State::with_strengths`] applies with no call to
/// the daemon; or anything else, after which the state is read again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Changes {
    Strengths(Vec<(String, u8)>),
    Any,
}

impl Changes {
    /// These and `later` as one.
    fn and(self, later: Option<Changes>) -> Changes {
        match (self, later) {
            (changes, None) => changes,
            (Changes::Strengths(mut seen), Some(Changes::Strengths(more))) => {
                for (path, strength) in more {
                    seen.retain(|(known, _)| *known != path);
                    seen.push((path, strength));
                }
                Changes::Strengths(seen)
            }
            _ => Changes::Any,
        }
    }
}

/// What a signal of the daemon's changed: an access point's strength alone (`Strengths`), nothing
/// shown (when it was last seen alone, or a scan's list of access points: `None`), or anything
/// else.
fn changes_of(message: &zbus::Message) -> Option<Changes> {
    let header = message.header();
    let properties = header.member().map(|member| member.as_str()) == Some("PropertiesChanged")
        && header.interface().map(|interface| interface.as_str()) == Some(PROPERTIES);
    let path = header.path().map(|path| path.to_string());
    let read = properties
        .then(|| {
            message
                .body()
                .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                .ok()
        })
        .flatten();
    let (Some((interface, changed, invalidated)), Some(path)) = (read, path) else {
        return Some(Changes::Any);
    };
    // Those properties alone (a signal naming none says nothing of what changed).
    let only = |these: &[&str]| {
        invalidated.is_empty()
            && !changed.is_empty()
            && changed.keys().all(|key| these.contains(&key.as_str()))
    };
    // A scan's access points and when it ended: what a scan asked for reads whole as it ends
    // (`NetworkManager::scan`); the lists shown ask for one as they open and every ten seconds.
    if interface == WIRELESS && only(&["AccessPoints", "LastScan"]) {
        return None;
    }
    if interface != ACCESS_POINT || !only(&["Strength", "LastSeen"]) {
        return Some(Changes::Any);
    }
    match changed
        .get("Strength")
        .map(|value| u8::try_from(value.clone()))
    {
        Some(Ok(strength)) => Some(Changes::Strengths(vec![(path, strength.min(100))])),
        Some(Err(_)) => Some(Changes::Any),
        None => None,
    }
}

/// What the watch follows next, as its caller says after each change: the access point joined
/// (its object's path; its strength is followed), or nothing more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Follow {
    Point(Option<String>),
    Stop,
}

/// A match on the daemon's signals: of `interface`'s `member`, at `path` if given, and for
/// `PropertiesChanged`, of the properties of `of` (its first argument).
fn daemon_rule(
    path: Option<&str>,
    interface: &'static str,
    member: &'static str,
    of: Option<&'static str>,
) -> zbus::Result<zbus::MatchRule<'static>> {
    let mut builder = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(NM)?
        .interface(interface)?
        .member(member)?;
    if let Some(path) = path {
        builder = builder.path(path.to_string())?;
    }
    if let Some(of) = of {
        builder = builder.arg(0, of)?;
    }
    Ok(builder.build())
}

/// The next message of any of `streams`, and which; `None` once one ends (the bus closed).
async fn next_of(
    streams: &mut [zbus::MessageStream],
) -> Option<(usize, zbus::Result<zbus::Message>)> {
    futures_lite::future::poll_fn(|cx| {
        for (index, stream) in streams.iter_mut().enumerate() {
            match stream.poll_next(cx) {
                std::task::Poll::Ready(Some(message)) => {
                    return std::task::Poll::Ready(Some((index, message)))
                }
                std::task::Poll::Ready(None) => return std::task::Poll::Ready(None),
                std::task::Poll::Pending => {}
            }
        }
        std::task::Poll::Pending
    })
    .await
}

/// Follows the daemon: calls `changed` once it is seen ([`Changes::Any`]), and again with what
/// changed after each change it signals, until `changed` says to stop or `stop` is readable (the
/// caller's order to end, waited on with the bus: the thread wakes only for a signal or the
/// order). Only what can change what is shown is followed, each a match the bus daemon applies so
/// no other signal is delivered: the daemon's own state and Wi-Fi's switches, its devices' states
/// and properties (a cable, the access point joined; not a scan's list of access points, read
/// whole as a scan asked for ends), the saved connections, its name's owner, and the strength of
/// the access point joined, which `changed` names after each change. Changes that come together
/// are one call, `settle` after the first; a strength alone is told at once. Without the bus,
/// returns the error; returns when the bus closes.
pub fn watch(
    address: Option<&str>,
    settle: Duration,
    stop: BorrowedFd<'_>,
    mut changed: impl FnMut(&Changes) -> Follow,
) -> io::Result<()> {
    let stop = async_io::Async::new(stop.try_clone_to_owned()?)?;
    // A connection of its own, its executor (what reads its socket) run on this thread as it
    // waits, with futures-lite's block_on: a thread waiting long in async-io's (as an internal
    // executor's does) keeps the reactor's own thread waking with a backoff after every message.
    let builder = match address {
        Some(address) => zbus::connection::Builder::address(address),
        None => zbus::connection::Builder::system(),
    }
    .map_err(failed)?;
    // Declared before the streams on it, so dropped after them.
    let connection = Ticked::connect(builder)?;
    let bus = connection.bus();
    let rules = [
        daemon_rule(Some(NM_PATH), PROPERTIES, "PropertiesChanged", Some(NM)),
        daemon_rule(None, DEVICE, "StateChanged", None),
        daemon_rule(None, PROPERTIES, "PropertiesChanged", Some(DEVICE)),
        daemon_rule(None, PROPERTIES, "PropertiesChanged", Some(WIRELESS)),
        daemon_rule(None, PROPERTIES, "PropertiesChanged", Some(WIRED)),
        daemon_rule(Some(SETTINGS_PATH), SETTINGS, "NewConnection", None),
        daemon_rule(Some(SETTINGS_PATH), SETTINGS, "ConnectionRemoved", None),
    ];
    // Its own name owner changing (it started or stopped) too.
    let owner = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(failed)?
        .member("NameOwnerChanged")
        .map_err(failed)?
        .arg(0, NM)
        .map_err(failed)?
        .build();
    let mut streams = Vec::new();
    for rule in rules {
        let rule = rule.map_err(failed)?;
        streams.push(
            connection
                .wait(zbus::MessageStream::for_match_rule(rule, bus, Some(64)))
                .map_err(failed)?,
        );
    }
    streams.push(
        connection
            .wait(zbus::MessageStream::for_match_rule(owner, bus, Some(16)))
            .map_err(failed)?,
    );
    let fixed = streams.len();
    // The access point joined: its strength, followed while it is the one joined.
    let mut point: Option<String> = None;
    let mut follow = |streams: &mut Vec<zbus::MessageStream>, now: Follow| -> io::Result<bool> {
        let Follow::Point(now) = now else {
            return Ok(false);
        };
        if now != point {
            streams.truncate(fixed);
            if let Some(path) = &now {
                let rule = daemon_rule(
                    Some(path),
                    PROPERTIES,
                    "PropertiesChanged",
                    Some(ACCESS_POINT),
                )
                .map_err(failed)?;
                streams.push(
                    connection
                        .wait(zbus::MessageStream::for_match_rule(rule, bus, Some(16)))
                        .map_err(failed)?,
                );
            }
            point = now;
        }
        Ok(true)
    };
    if !follow(&mut streams, changed(&Changes::Any))? {
        return Ok(());
    }
    loop {
        let next = connection.wait(futures_lite::future::or(
            async {
                match next_of(&mut streams).await {
                    Some((_, Ok(message))) => Some(changes_of(&message)),
                    Some((_, Err(_))) => Some(Some(Changes::Any)),
                    None => None,
                }
            },
            async {
                let _ = stop.readable().await;
                None
            },
        ));
        let first = match next {
            // Stopped, or the bus closed.
            None => return Ok(()),
            // Nothing shown changed: no wait, no call.
            Some(None) => continue,
            Some(Some(changes)) => changes,
        };
        // A strength alone is told at once; what follows anything else at once is the same change.
        let changes = match first {
            Changes::Strengths(_) => first,
            Changes::Any => {
                let until = Instant::now() + settle;
                let (changes, ended) = connection.wait(async {
                    let mut changes = Changes::Any;
                    loop {
                        let more = futures_lite::future::or(
                            async { Some(next_of(&mut streams).await) },
                            async {
                                async_io::Timer::at(until).await;
                                None
                            },
                        )
                        .await;
                        match more {
                            Some(Some((_, Ok(message)))) => {
                                changes = changes.and(changes_of(&message))
                            }
                            Some(Some((_, Err(_)))) => {}
                            // The bus closed: told now, and the next wait ends the watch.
                            Some(None) => return (changes, true),
                            None => return (changes, false),
                        }
                    }
                });
                if ended {
                    changed(&changes);
                    return Ok(());
                }
                changes
            }
        };
        if !follow(&mut streams, changed(&changes))? {
            return Ok(());
        }
    }
}
