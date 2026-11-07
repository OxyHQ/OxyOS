//! A played BlueZ (docs/adr/0050-bluetooth.mdx), for the development compositor's scenarios
//! (`regress.py`'s `bluetooth`) and the tests of the session's Bluetooth: the part of BlueZ's D-Bus
//! API the session uses, on a bus of the caller's own, for a machine a JSON file describes. Never
//! installed; built with the development compositor, which its scenarios run beside.
//!
//! It answers as BlueZ 5.82 does: `org.bluez` owns `/` (its object manager), `/org/bluez` (the
//! agent manager), `/org/bluez/hci0` (the adapter) and a `dev_XX_XX_…` object per device known,
//! with each change signalled (`InterfacesAdded`, `InterfacesRemoved`, `PropertiesChanged`). A
//! discovery is its client's: it ends when that client stops it or leaves the bus, and the
//! devices it found but did not pair go with it. Pairing asks the registered agent as the device's
//! pairing method says (numeric comparison, a PIN, a passkey typed or shown, or nothing), and a
//! device's own requests (pairing, a service) are played by `Act` on its control interface
//! (`so.oxy.OxyOS.FakeBlueZ` at `/so/oxy/OxyOS/FakeBlueZ`), whose `State` says what it holds; the
//! same is written to `--state` after every change.
//!
//! The machine: `{"adapter": {"name", "address", "powered", "blocked"} | null, "devices": [{"address",
//! "name", "icon", "uuids", "paired", "trusted", "connected", "battery", "nearby" (appears only
//! while discovering), "pairing": "just-works" | "confirm" | "pin" | "passkey" | "show-passkey" |
//! "show-pin" | "refuse", "passkey", "pin", "connect": "ok" | "fail" | "none"}]}`.
//!
//! `Act`'s JSON: `{"appear": DEVICE}`, `{"vanish": ADDRESS}`, `{"connect": ADDRESS}` (it connects by
//! itself), `{"disconnect": ADDRESS}`, `{"battery": {"address", "percent"}}`, `{"rssi": {"address",
//! "rssi"}}`, `{"transport_volume": {"address", "volume"}}` (what BlueZ signals of a stream as it
//! plays, which no session follows), `{"name": {"address", "name"}}`, `{"pair_request": ADDRESS}` (it asks to pair),
//! `{"service_request": {"address", "uuid"}}`, `{"typed": ADDRESS}` (the code shown was typed on it),
//! `{"cancel": ADDRESS}` (BlueZ cancels what it asked), `{"powered": BOOL}`, `{"blocked": BOOL}`.

use serde_json::{json, Value as Json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

const ADAPTER_PATH: &str = "/org/bluez/hci0";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const BATTERY: &str = "org.bluez.Battery1";
const AGENT: &str = "org.bluez.Agent1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const CONTROL_PATH: &str = "/so/oxy/OxyOS/FakeBlueZ";
const CONTROL: &str = "so.oxy.OxyOS.FakeBlueZ";
/// As long as BlueZ waits for an agent's answer, or for a code shown to be typed.
const AGENT_TIMEOUT: Duration = Duration::from_secs(60);

type Properties = HashMap<String, Value<'static>>;

#[derive(Clone, Debug, Default)]
struct Adapter {
    name: String,
    address: String,
    powered: bool,
    blocked: bool,
    discoverable: bool,
}

#[derive(Debug, Default)]
struct Device {
    address: String,
    name: Option<String>,
    alias: Option<String>,
    icon: Option<String>,
    uuids: Vec<String>,
    paired: bool,
    trusted: bool,
    connected: bool,
    battery: Option<u8>,
    rssi: i16,
    /// Found only by a discovery.
    nearby: bool,
    /// An object of BlueZ's now.
    present: bool,
    pairing: String,
    passkey: u32,
    pin: String,
    connect: String,
    /// A pairing waiting for the code shown to be typed: told true when it was, false cancelled.
    waiting: Option<mpsc::Sender<bool>>,
}

fn path_of(address: &str) -> String {
    format!("{ADAPTER_PATH}/dev_{}", address.replace(':', "_"))
}

impl Device {
    fn of(spec: &Json) -> Device {
        let text = |key: &str| spec.get(key).and_then(Json::as_str).map(str::to_string);
        let flag = |key: &str| spec.get(key).and_then(Json::as_bool).unwrap_or(false);
        let paired = flag("paired");
        let nearby = flag("nearby") && !paired;
        Device {
            address: text("address").unwrap_or_default(),
            name: text("name"),
            alias: None,
            icon: text("icon"),
            uuids: spec
                .get("uuids")
                .and_then(Json::as_array)
                .map(|uuids| {
                    uuids
                        .iter()
                        .filter_map(|u| u.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            paired,
            trusted: flag("trusted") || paired,
            connected: flag("connected"),
            battery: spec
                .get("battery")
                .and_then(Json::as_u64)
                .map(|b| b.min(100) as u8),
            rssi: -60,
            nearby,
            present: !nearby,
            pairing: text("pairing").unwrap_or_else(|| "just-works".into()),
            passkey: spec.get("passkey").and_then(Json::as_u64).unwrap_or(123456) as u32,
            pin: text("pin").unwrap_or_else(|| "0000".into()),
            connect: text("connect").unwrap_or_else(|| "ok".into()),
            waiting: None,
        }
    }

    fn alias(&self) -> String {
        self.alias
            .clone()
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| self.address.replace(':', "-"))
    }

    fn properties(&self) -> Properties {
        let mut properties: Properties = HashMap::new();
        properties.insert("Address".into(), Value::from(self.address.clone()));
        properties.insert("AddressType".into(), Value::from("public"));
        if let Some(name) = &self.name {
            properties.insert("Name".into(), Value::from(name.clone()));
        }
        properties.insert("Alias".into(), Value::from(self.alias()));
        if let Some(icon) = &self.icon {
            properties.insert("Icon".into(), Value::from(icon.clone()));
        }
        properties.insert("Paired".into(), Value::from(self.paired));
        properties.insert("Bonded".into(), Value::from(self.paired));
        properties.insert("Trusted".into(), Value::from(self.trusted));
        properties.insert("Blocked".into(), Value::from(false));
        properties.insert("Connected".into(), Value::from(self.connected));
        properties.insert("ServicesResolved".into(), Value::from(self.connected));
        properties.insert("LegacyPairing".into(), Value::from(false));
        properties.insert("UUIDs".into(), Value::from(self.uuids.clone()));
        properties.insert(
            "Adapter".into(),
            Value::from(ObjectPath::from_static_str_unchecked(ADAPTER_PATH)),
        );
        if self.nearby {
            properties.insert("RSSI".into(), Value::from(self.rssi));
        }
        properties
    }

    fn interfaces(&self) -> HashMap<String, Properties> {
        let mut interfaces = HashMap::new();
        interfaces.insert(DEVICE.to_string(), self.properties());
        if let Some(battery) = self.battery {
            let mut properties: Properties = HashMap::new();
            properties.insert("Percentage".into(), Value::from(battery));
            interfaces.insert(BATTERY.to_string(), properties);
        }
        interfaces
    }
}

#[derive(Debug, Default)]
struct Machine {
    adapter: Option<Adapter>,
    devices: Vec<Device>,
    /// The clients discovering, by their unique names.
    discovery: BTreeSet<String>,
    filtered: bool,
    /// The agent registered: its client, object and capability; and whether it is the default.
    agent: Option<(String, String, String)>,
    default_agent: bool,
    /// Every call made of it that changed something, in order: what a scenario checks.
    log: Vec<String>,
    /// What the agent answered each device's own requests: `pair ADDRESS` or `service ADDRESS`,
    /// `accepted` or `refused`.
    answers: BTreeMap<String, String>,
}

impl Machine {
    fn device(&mut self, address: &str) -> Option<&mut Device> {
        self.devices.iter_mut().find(|d| d.address == address)
    }

    fn by_path(&mut self, path: &str) -> Option<&mut Device> {
        self.devices
            .iter_mut()
            .find(|d| d.present && path_of(&d.address) == path)
    }

    fn discovering(&self) -> bool {
        !self.discovery.is_empty()
    }

    fn adapter_properties(&self) -> Properties {
        let adapter = self.adapter.clone().unwrap_or_default();
        let mut properties: Properties = HashMap::new();
        properties.insert("Address".into(), Value::from(adapter.address.clone()));
        properties.insert("Name".into(), Value::from(adapter.name.clone()));
        properties.insert("Alias".into(), Value::from(adapter.name.clone()));
        properties.insert("Powered".into(), Value::from(adapter.powered));
        properties.insert("PowerState".into(), Value::from(power_state(&adapter)));
        properties.insert("Discoverable".into(), Value::from(adapter.discoverable));
        properties.insert("DiscoverableTimeout".into(), Value::from(180u32));
        properties.insert("Pairable".into(), Value::from(true));
        properties.insert("Discovering".into(), Value::from(self.discovering()));
        properties
    }

    fn state(&self) -> Json {
        let devices: BTreeMap<String, Json> = self
            .devices
            .iter()
            .map(|d| {
                (
                    d.address.clone(),
                    json!({
                        "present": d.present, "paired": d.paired, "trusted": d.trusted,
                        "connected": d.connected, "alias": d.alias, "battery": d.battery,
                    }),
                )
            })
            .collect();
        json!({
            "adapter": self.adapter.as_ref().map(|a| json!({
                "powered": a.powered, "blocked": a.blocked, "discoverable": a.discoverable,
                "discovering": self.discovering(), "discovery_clients": self.discovery.len(),
                "filtered": self.filtered,
            })),
            "agent": self.agent.as_ref().map(|(_, path, capability)| json!({
                "path": path, "capability": capability, "default": self.default_agent,
            })),
            "devices": devices,
            "log": self.log,
            "answers": self.answers,
        })
    }
}

fn power_state(adapter: &Adapter) -> &'static str {
    if adapter.blocked {
        "off-blocked"
    } else if adapter.powered {
        "on"
    } else {
        "off"
    }
}

struct Fake {
    bus: Connection,
    machine: Mutex<Machine>,
    state: std::path::PathBuf,
}

/// A D-Bus error to answer with: its name and message.
type Error = (&'static str, String);

fn bluez_error(name: &'static str, message: &str) -> Error {
    (name, message.to_string())
}

impl Fake {
    fn save(&self, machine: &Machine) {
        let text = serde_json::to_string_pretty(&machine.state()).unwrap_or_default();
        let partial = self.state.with_extension("partial");
        if std::fs::write(&partial, text).is_ok() {
            let _ = std::fs::rename(&partial, &self.state);
        }
    }

    fn changed(&self, path: &str, interface: &str, properties: Properties) {
        let _ = self.bus.emit_signal(
            None::<&str>,
            path,
            PROPERTIES,
            "PropertiesChanged",
            &(interface, properties, Vec::<String>::new()),
        );
    }

    fn one(name: &str, value: Value<'static>) -> Properties {
        let mut properties: Properties = HashMap::new();
        properties.insert(name.to_string(), value);
        properties
    }

    fn added(&self, path: &str, interfaces: HashMap<String, Properties>) {
        let object = ObjectPath::try_from(path.to_string()).expect("a path");
        let _ = self.bus.emit_signal(
            None::<&str>,
            "/",
            OBJECT_MANAGER,
            "InterfacesAdded",
            &(object, interfaces),
        );
    }

    fn removed(&self, path: &str, interfaces: Vec<String>) {
        let object = ObjectPath::try_from(path.to_string()).expect("a path");
        let _ = self.bus.emit_signal(
            None::<&str>,
            "/",
            OBJECT_MANAGER,
            "InterfacesRemoved",
            &(object, interfaces),
        );
    }

    fn managed(&self) -> HashMap<OwnedObjectPath, HashMap<String, Properties>> {
        let machine = self.machine.lock().unwrap();
        let mut objects = HashMap::new();
        let object = |path: &str| OwnedObjectPath::try_from(path.to_string()).expect("a path");
        let mut manager = HashMap::new();
        manager.insert("org.bluez.AgentManager1".to_string(), Properties::new());
        objects.insert(object("/org/bluez"), manager);
        if machine.adapter.is_some() {
            let mut adapter = HashMap::new();
            adapter.insert(ADAPTER.to_string(), machine.adapter_properties());
            objects.insert(object(ADAPTER_PATH), adapter);
            for device in machine.devices.iter().filter(|d| d.present) {
                objects.insert(object(&path_of(&device.address)), device.interfaces());
            }
        }
        objects
    }

    /// A discovery ends: its devices not paired go.
    fn discovery_ended(&self, machine: &mut Machine) {
        self.changed(
            ADAPTER_PATH,
            ADAPTER,
            Self::one("Discovering", Value::from(false)),
        );
        for device in machine.devices.iter_mut() {
            if device.nearby && device.present && !device.paired {
                device.present = false;
                self.removed(&path_of(&device.address), vec![DEVICE.to_string()]);
            }
        }
    }

    fn set_powered(&self, machine: &mut Machine, powered: bool) {
        let Some(adapter) = machine.adapter.as_mut() else {
            return;
        };
        adapter.powered = powered;
        if !powered {
            adapter.discoverable = false;
        }
        let state = power_state(adapter);
        let mut properties = Self::one("Powered", Value::from(powered));
        properties.insert("PowerState".into(), Value::from(state));
        if !powered {
            properties.insert("Discoverable".into(), Value::from(false));
        }
        self.changed(ADAPTER_PATH, ADAPTER, properties);
        if !powered {
            if !machine.discovery.is_empty() {
                machine.discovery.clear();
                self.discovery_ended(machine);
            }
            for device in machine.devices.iter_mut().filter(|d| d.connected) {
                device.connected = false;
                let mut properties = Self::one("Connected", Value::from(false));
                properties.insert("ServicesResolved".into(), Value::from(false));
                self.changed(&path_of(&device.address), DEVICE, properties);
            }
        }
    }

    /// Asks the registered agent `method` with `body`; its error's name, if it refused.
    fn ask_agent<B>(&self, method: &str, body: &B) -> Result<zbus::Message, String>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let agent = self.machine.lock().unwrap().agent.clone();
        let Some((owner, path, _)) = agent else {
            return Err("org.bluez.Error.AuthenticationFailed".into());
        };
        self.bus
            .call_method(
                Some(owner.as_str()),
                path.as_str(),
                Some(AGENT),
                method,
                body,
            )
            .map_err(|error| match error {
                zbus::Error::MethodError(name, _, _) => name.to_string(),
                other => format!("org.bluez.Error.Failed: {other}"),
            })
    }

    fn cancel_agent(&self) {
        let agent = self.machine.lock().unwrap().agent.clone();
        if let Some((owner, path, _)) = agent {
            let _ = self.bus.call_method(
                Some(owner.as_str()),
                path.as_str(),
                Some(AGENT),
                "Cancel",
                &(),
            );
        }
    }

    /// Pairs `address` as its pairing method says, asking the agent; Ok once paired.
    fn pair(&self, address: &str) -> Result<(), Error> {
        let (pairing, passkey, pin, path) = {
            let mut machine = self.machine.lock().unwrap();
            machine.log.push(format!("Pair {address}"));
            let Some(device) = machine.device(address) else {
                return Err(bluez_error("org.bluez.Error.DoesNotExist", "no device"));
            };
            if device.paired {
                return Err(bluez_error(
                    "org.bluez.Error.AlreadyExists",
                    "Already Exists",
                ));
            }
            (
                device.pairing.clone(),
                device.passkey,
                device.pin.clone(),
                path_of(address),
            )
        };
        let object = ObjectPath::try_from(path.clone()).expect("a path");
        let refused = |name: String| -> Error {
            if name.ends_with(".Canceled") {
                bluez_error(
                    "org.bluez.Error.AuthenticationCanceled",
                    "Authentication Canceled",
                )
            } else {
                bluez_error(
                    "org.bluez.Error.AuthenticationRejected",
                    "Authentication Rejected",
                )
            }
        };
        match pairing.as_str() {
            "refuse" => {
                return Err(bluez_error(
                    "org.bluez.Error.AuthenticationRejected",
                    "Authentication Rejected",
                ))
            }
            "confirm" => {
                self.ask_agent("RequestConfirmation", &(&object, passkey))
                    .map_err(refused)?;
            }
            "pin" => {
                let reply = self
                    .ask_agent("RequestPinCode", &(&object,))
                    .map_err(refused)?;
                let given: String = reply.body().deserialize().unwrap_or_default();
                if given != pin {
                    return Err(bluez_error(
                        "org.bluez.Error.AuthenticationFailed",
                        "Authentication Failed",
                    ));
                }
            }
            "passkey" => {
                let reply = self
                    .ask_agent("RequestPasskey", &(&object,))
                    .map_err(refused)?;
                let given: u32 = reply.body().deserialize().unwrap_or(u32::MAX);
                if given != passkey {
                    return Err(bluez_error(
                        "org.bluez.Error.AuthenticationFailed",
                        "Authentication Failed",
                    ));
                }
            }
            "show-passkey" | "show-pin" => {
                let (typed, waiting) = mpsc::channel();
                if let Some(device) = self.machine.lock().unwrap().device(address) {
                    device.waiting = Some(typed);
                }
                if pairing == "show-pin" {
                    let _ = self.ask_agent("DisplayPinCode", &(&object, pin.as_str()));
                } else {
                    let _ = self.ask_agent("DisplayPasskey", &(&object, passkey, 0u16));
                }
                let typed = waiting.recv_timeout(AGENT_TIMEOUT).unwrap_or(false);
                if let Some(device) = self.machine.lock().unwrap().device(address) {
                    device.waiting = None;
                }
                if !typed {
                    return Err(bluez_error(
                        "org.bluez.Error.AuthenticationCanceled",
                        "Authentication Canceled",
                    ));
                }
            }
            _ => {}
        }
        self.paired(address);
        Ok(())
    }

    fn paired(&self, address: &str) {
        let mut machine = self.machine.lock().unwrap();
        if let Some(device) = machine.device(address) {
            device.paired = true;
            device.nearby = false;
            let mut properties = Self::one("Paired", Value::from(true));
            properties.insert("Bonded".into(), Value::from(true));
            self.changed(&path_of(address), DEVICE, properties);
        }
        self.save(&machine);
    }

    fn connect(&self, address: &str) -> Result<(), Error> {
        let mut machine = self.machine.lock().unwrap();
        machine.log.push(format!("Connect {address}"));
        let powered = machine.adapter.as_ref().is_some_and(|a| a.powered);
        let Some(device) = machine.device(address) else {
            return Err(bluez_error("org.bluez.Error.DoesNotExist", "no device"));
        };
        if !powered {
            return Err(bluez_error(
                "org.bluez.Error.NotReady",
                "Resource Not Ready",
            ));
        }
        let outcome = match device.connect.as_str() {
            "fail" => Err(bluez_error(
                "org.bluez.Error.Failed",
                "br-connection-page-timeout",
            )),
            "none" => Err(bluez_error(
                "org.bluez.Error.NotAvailable",
                "br-connection-profile-unavailable",
            )),
            _ => {
                if !device.connected {
                    device.connected = true;
                    let mut properties = Self::one("Connected", Value::from(true));
                    properties.insert("ServicesResolved".into(), Value::from(true));
                    self.changed(&path_of(address), DEVICE, properties);
                }
                Ok(())
            }
        };
        self.save(&machine);
        outcome
    }

    fn disconnect(&self, address: &str) {
        let mut machine = self.machine.lock().unwrap();
        machine.log.push(format!("Disconnect {address}"));
        if let Some(device) = machine.device(address) {
            if device.connected {
                device.connected = false;
                let mut properties = Self::one("Connected", Value::from(false));
                properties.insert("ServicesResolved".into(), Value::from(false));
                self.changed(&path_of(address), DEVICE, properties);
            }
        }
        self.save(&machine);
    }

    /// A method call of a client's: its answer, or the error to answer with. Pairing and
    /// connecting take as long as the agent and the device do, so they answer from a thread of
    /// their own (`None`).
    fn handle(self: &Arc<Self>, message: &zbus::Message) -> Option<Result<Answer, Error>> {
        let header = message.header();
        let path = header.path()?.to_string();
        let interface = header
            .interface()
            .map(|i| i.to_string())
            .unwrap_or_default();
        let member = header.member()?.to_string();
        let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
        let body = message.body();
        let unknown = || {
            Some(Err(bluez_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                "unknown",
            )))
        };
        match (path.as_str(), interface.as_str(), member.as_str()) {
            ("/", OBJECT_MANAGER, "GetManagedObjects") => Some(Ok(Answer::Objects(self.managed()))),
            (CONTROL_PATH, CONTROL, "State") => {
                let machine = self.machine.lock().unwrap();
                Some(Ok(Answer::Text(machine.state().to_string())))
            }
            (CONTROL_PATH, CONTROL, "Act") => {
                let what: String = body.deserialize().ok()?;
                let fake = self.clone();
                // Acts that ask the agent wait for its answer: on a thread, answered at once.
                std::thread::spawn(move || fake.act(&what));
                Some(Ok(Answer::Nothing))
            }
            ("/org/bluez", "org.bluez.AgentManager1", "RegisterAgent") => {
                let (agent, capability): (OwnedObjectPath, String) = body.deserialize().ok()?;
                let mut machine = self.machine.lock().unwrap();
                if machine.agent.is_some() {
                    return Some(Err(bluez_error(
                        "org.bluez.Error.AlreadyExists",
                        "Already Exists",
                    )));
                }
                machine.agent = Some((sender, agent.to_string(), capability));
                machine.log.push("RegisterAgent".into());
                self.save(&machine);
                Some(Ok(Answer::Nothing))
            }
            ("/org/bluez", "org.bluez.AgentManager1", "RequestDefaultAgent") => {
                let mut machine = self.machine.lock().unwrap();
                if machine
                    .agent
                    .as_ref()
                    .is_some_and(|(owner, _, _)| *owner == sender)
                {
                    machine.default_agent = true;
                    self.save(&machine);
                    Some(Ok(Answer::Nothing))
                } else {
                    Some(Err(bluez_error(
                        "org.bluez.Error.DoesNotExist",
                        "Does Not Exist",
                    )))
                }
            }
            ("/org/bluez", "org.bluez.AgentManager1", "UnregisterAgent") => {
                let mut machine = self.machine.lock().unwrap();
                machine.agent = None;
                machine.default_agent = false;
                machine.log.push("UnregisterAgent".into());
                self.save(&machine);
                Some(Ok(Answer::Nothing))
            }
            (_, PROPERTIES, "Set") => {
                let (of, name, value): (String, String, zbus::zvariant::OwnedValue) =
                    body.deserialize().ok()?;
                Some(self.set(&path, &of, &name, value))
            }
            (_, PROPERTIES, "GetAll") => {
                let machine = self.machine.lock().unwrap();
                if path == ADAPTER_PATH {
                    Some(Ok(Answer::Properties(machine.adapter_properties())))
                } else {
                    unknown()
                }
            }
            (ADAPTER_PATH, ADAPTER, member) => Some(self.adapter_call(member, &sender, message)),
            (_, DEVICE, "Pair" | "Connect") => {
                let address = self.machine.lock().unwrap().by_path(&path)?.address.clone();
                let (fake, message, member) = (self.clone(), message.clone(), member.clone());
                std::thread::spawn(move || {
                    let outcome = if member == "Pair" {
                        fake.pair(&address)
                    } else {
                        fake.connect(&address)
                    };
                    fake.answer(&message, outcome.map(|()| Answer::Nothing));
                });
                None
            }
            (_, DEVICE, "Disconnect") => {
                let address = self.machine.lock().unwrap().by_path(&path)?.address.clone();
                self.disconnect(&address);
                Some(Ok(Answer::Nothing))
            }
            (_, DEVICE, "CancelPairing") => {
                let mut machine = self.machine.lock().unwrap();
                let device = machine.by_path(&path)?;
                if let Some(waiting) = device.waiting.take() {
                    let _ = waiting.send(false);
                }
                machine.log.push("CancelPairing".into());
                self.save(&machine);
                Some(Ok(Answer::Nothing))
            }
            _ => unknown(),
        }
    }

    fn adapter_call(
        &self,
        member: &str,
        sender: &str,
        message: &zbus::Message,
    ) -> Result<Answer, Error> {
        let mut machine = self.machine.lock().unwrap();
        let powered = machine.adapter.as_ref().is_some_and(|a| a.powered);
        let outcome = match member {
            "SetDiscoveryFilter" => {
                machine.filtered = true;
                Ok(Answer::Nothing)
            }
            "StartDiscovery" if !powered => Err(bluez_error(
                "org.bluez.Error.NotReady",
                "Resource Not Ready",
            )),
            "StartDiscovery" => {
                if machine.discovery.contains(sender) {
                    return Err(bluez_error(
                        "org.bluez.Error.InProgress",
                        "Operation already in progress",
                    ));
                }
                let first = machine.discovery.is_empty();
                machine.discovery.insert(sender.to_string());
                machine.log.push("StartDiscovery".into());
                if first {
                    self.changed(
                        ADAPTER_PATH,
                        ADAPTER,
                        Self::one("Discovering", Value::from(true)),
                    );
                    for device in machine.devices.iter_mut() {
                        if device.nearby && !device.present {
                            device.present = true;
                            self.added(&path_of(&device.address), device.interfaces());
                        }
                    }
                }
                Ok(Answer::Nothing)
            }
            "StopDiscovery" => {
                if !machine.discovery.remove(sender) {
                    return Err(bluez_error(
                        "org.bluez.Error.Failed",
                        "No discovery started",
                    ));
                }
                machine.log.push("StopDiscovery".into());
                if machine.discovery.is_empty() {
                    self.discovery_ended(&mut machine);
                }
                Ok(Answer::Nothing)
            }
            "RemoveDevice" => {
                let Ok((object,)) = message.body().deserialize::<(OwnedObjectPath,)>() else {
                    return Err(bluez_error(
                        "org.bluez.Error.InvalidArguments",
                        "Invalid arguments",
                    ));
                };
                let Some(at) = machine
                    .devices
                    .iter()
                    .position(|d| d.present && path_of(&d.address) == object.as_str())
                else {
                    return Err(bluez_error(
                        "org.bluez.Error.DoesNotExist",
                        "Does Not Exist",
                    ));
                };
                // Forgotten: no longer an object of BlueZ's, its keys gone (kept here, not present,
                // so what was done to it can be told).
                let device = &mut machine.devices[at];
                let mut interfaces = vec![DEVICE.to_string()];
                if device.battery.take().is_some() {
                    interfaces.push(BATTERY.to_string());
                }
                device.present = false;
                device.paired = false;
                device.trusted = false;
                device.connected = false;
                device.alias = None;
                let address = device.address.clone();
                machine.log.push(format!("RemoveDevice {address}"));
                self.removed(object.as_str(), interfaces);
                Ok(Answer::Nothing)
            }
            _ => Err(bluez_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                "unknown",
            )),
        };
        self.save(&machine);
        outcome
    }

    fn set(
        &self,
        path: &str,
        of: &str,
        name: &str,
        value: zbus::zvariant::OwnedValue,
    ) -> Result<Answer, Error> {
        let mut machine = self.machine.lock().unwrap();
        let outcome = match (of, name) {
            (ADAPTER, "Powered") if path == ADAPTER_PATH => {
                let on = bool::try_from(&*value).unwrap_or(false);
                machine.log.push(format!("Powered {on}"));
                if on && machine.adapter.as_ref().is_some_and(|a| a.blocked) {
                    Err(bluez_error(
                        "org.bluez.Error.Failed",
                        "Blocked through rfkill",
                    ))
                } else {
                    self.set_powered(&mut machine, on);
                    Ok(Answer::Nothing)
                }
            }
            (ADAPTER, "Discoverable") if path == ADAPTER_PATH => {
                let on = bool::try_from(&*value).unwrap_or(false);
                machine.log.push(format!("Discoverable {on}"));
                if let Some(adapter) = machine.adapter.as_mut() {
                    adapter.discoverable = on;
                }
                self.changed(
                    ADAPTER_PATH,
                    ADAPTER,
                    Self::one("Discoverable", Value::from(on)),
                );
                Ok(Answer::Nothing)
            }
            (DEVICE, "Trusted") => {
                let on = bool::try_from(&*value).unwrap_or(false);
                match machine.by_path(path) {
                    Some(device) => {
                        device.trusted = on;
                        self.changed(path, DEVICE, Self::one("Trusted", Value::from(on)));
                        Ok(Answer::Nothing)
                    }
                    None => Err(bluez_error(
                        "org.freedesktop.DBus.Error.UnknownObject",
                        "no object",
                    )),
                }
            }
            (DEVICE, "Alias") => {
                let alias: String = match &*value {
                    Value::Str(text) => text.to_string(),
                    _ => String::new(),
                };
                match machine.by_path(path) {
                    Some(device) => {
                        // An empty alias gives the device its own name back, as BlueZ does.
                        device.alias = (!alias.is_empty()).then_some(alias);
                        let shown = device.alias();
                        let address = device.address.clone();
                        self.changed(path, DEVICE, Self::one("Alias", Value::from(shown)));
                        machine.log.push(format!("Alias {address}"));
                        Ok(Answer::Nothing)
                    }
                    None => Err(bluez_error(
                        "org.freedesktop.DBus.Error.UnknownObject",
                        "no object",
                    )),
                }
            }
            _ => Err(bluez_error(
                "org.freedesktop.DBus.Error.PropertyReadOnly",
                "read-only",
            )),
        };
        self.save(&machine);
        outcome
    }

    fn answer(&self, call: &zbus::Message, outcome: Result<Answer, Error>) {
        let header = call.header();
        let _ = match outcome {
            Ok(Answer::Nothing) => self.bus.reply(&header, &()),
            Ok(Answer::Text(text)) => self.bus.reply(&header, &text),
            Ok(Answer::Objects(objects)) => self.bus.reply(&header, &objects),
            Ok(Answer::Properties(properties)) => self.bus.reply(&header, &properties),
            Err((name, message)) => self.bus.reply_error(&header, name, &message),
        };
    }

    /// The machine changes by itself, as its `Act` says (the module doc).
    fn act(&self, what: &str) {
        let Ok(act) = serde_json::from_str::<Json>(what) else {
            eprintln!("[fake-bluez] not an act: {what}");
            return;
        };
        let address_of = |value: &Json| {
            value
                .as_str()
                .or_else(|| value.get("address").and_then(Json::as_str))
                .unwrap_or_default()
                .to_string()
        };
        if let Some(spec) = act.get("appear") {
            let mut device = Device::of(spec);
            device.present = true;
            let mut machine = self.machine.lock().unwrap();
            self.added(&path_of(&device.address), device.interfaces());
            machine.devices.retain(|d| d.address != device.address);
            machine.devices.push(device);
            self.save(&machine);
        } else if let Some(value) = act.get("vanish") {
            let address = address_of(value);
            let mut machine = self.machine.lock().unwrap();
            if let Some(device) = machine.device(&address) {
                if device.present {
                    device.present = false;
                    self.removed(&path_of(&address), vec![DEVICE.to_string()]);
                }
            }
            self.save(&machine);
        } else if let Some(value) = act.get("connect") {
            let address = address_of(value);
            let _ = self.connect(&address);
        } else if let Some(value) = act.get("disconnect") {
            self.disconnect(&address_of(value));
        } else if let Some(value) = act.get("battery") {
            let address = address_of(value);
            let percent = value.get("percent").and_then(Json::as_u64);
            let mut machine = self.machine.lock().unwrap();
            if let Some(device) = machine.device(&address) {
                let had = device.battery.is_some();
                device.battery = percent.map(|p| p.min(100) as u8);
                let path = path_of(&address);
                match (had, device.battery) {
                    (false, Some(battery)) => {
                        let mut interfaces = HashMap::new();
                        interfaces.insert(
                            BATTERY.to_string(),
                            Self::one("Percentage", Value::from(battery)),
                        );
                        self.added(&path, interfaces);
                    }
                    (true, Some(battery)) => self.changed(
                        &path,
                        BATTERY,
                        Self::one("Percentage", Value::from(battery)),
                    ),
                    (true, None) => self.removed(&path, vec![BATTERY.to_string()]),
                    (false, None) => {}
                }
            }
            self.save(&machine);
        } else if let Some(value) = act.get("rssi") {
            let address = address_of(value);
            let rssi = value.get("rssi").and_then(Json::as_i64).unwrap_or(-50) as i16;
            let mut machine = self.machine.lock().unwrap();
            if let Some(device) = machine.device(&address) {
                device.rssi = rssi;
                self.changed(
                    &path_of(&address),
                    DEVICE,
                    Self::one("RSSI", Value::from(rssi)),
                );
            }
        } else if let Some(value) = act.get("transport_volume") {
            let address = address_of(value);
            let volume = value.get("volume").and_then(Json::as_u64).unwrap_or(100) as u16;
            self.changed(
                &format!("{}/fd0", path_of(&address)),
                "org.bluez.MediaTransport1",
                Self::one("Volume", Value::from(volume)),
            );
        } else if let Some(value) = act.get("name") {
            let address = address_of(value);
            let name = value
                .get("name")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string();
            let mut machine = self.machine.lock().unwrap();
            if let Some(device) = machine.device(&address) {
                device.name = Some(name.clone());
                let mut properties = Self::one("Name", Value::from(name));
                properties.insert("Alias".into(), Value::from(device.alias()));
                self.changed(&path_of(&address), DEVICE, properties);
            }
            self.save(&machine);
        } else if let Some(value) = act.get("pair_request") {
            let address = address_of(value);
            let (pairing, passkey) = {
                let mut machine = self.machine.lock().unwrap();
                match machine.device(&address) {
                    Some(device) => (device.pairing.clone(), device.passkey),
                    None => return,
                }
            };
            let object = ObjectPath::try_from(path_of(&address)).expect("a path");
            let asked = if pairing == "confirm" {
                self.ask_agent("RequestConfirmation", &(&object, passkey))
            } else {
                self.ask_agent("RequestAuthorization", &(&object,))
            };
            if asked.is_ok() {
                self.paired(&address);
            }
            let answer = if asked.is_ok() { "accepted" } else { "refused" };
            let mut machine = self.machine.lock().unwrap();
            machine
                .log
                .push(format!("pair request {address}: {answer}"));
            machine
                .answers
                .insert(format!("pair {address}"), answer.into());
            self.save(&machine);
        } else if let Some(value) = act.get("service_request") {
            let address = address_of(value);
            let uuid = value
                .get("uuid")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string();
            let object = ObjectPath::try_from(path_of(&address)).expect("a path");
            let asked = self.ask_agent("AuthorizeService", &(&object, uuid.as_str()));
            let answer = if asked.is_ok() { "allowed" } else { "refused" };
            let mut machine = self.machine.lock().unwrap();
            machine
                .log
                .push(format!("service request {address}: {answer}"));
            machine
                .answers
                .insert(format!("service {address}"), answer.into());
            self.save(&machine);
        } else if let Some(value) = act.get("typed") {
            let address = address_of(value);
            if let Some(device) = self.machine.lock().unwrap().device(&address) {
                if let Some(waiting) = device.waiting.take() {
                    let _ = waiting.send(true);
                }
            }
        } else if let Some(value) = act.get("cancel") {
            let address = address_of(value);
            self.cancel_agent();
            if let Some(device) = self.machine.lock().unwrap().device(&address) {
                if let Some(waiting) = device.waiting.take() {
                    let _ = waiting.send(false);
                }
            }
        } else if let Some(on) = act.get("powered").and_then(Json::as_bool) {
            let mut machine = self.machine.lock().unwrap();
            self.set_powered(&mut machine, on);
            self.save(&machine);
        } else if let Some(blocked) = act.get("blocked").and_then(Json::as_bool) {
            let mut machine = self.machine.lock().unwrap();
            if let Some(adapter) = machine.adapter.as_mut() {
                adapter.blocked = blocked;
            }
            self.set_powered(&mut machine, !blocked);
            self.save(&machine);
        } else {
            eprintln!("[fake-bluez] unknown act: {what}");
        }
    }

    /// A client left the bus: its discovery ends and its agent goes, as BlueZ's do.
    fn left(&self, client: &str) {
        let mut machine = self.machine.lock().unwrap();
        if machine.discovery.remove(client) && machine.discovery.is_empty() {
            self.discovery_ended(&mut machine);
        }
        if machine
            .agent
            .as_ref()
            .is_some_and(|(owner, _, _)| owner == client)
        {
            machine.agent = None;
            machine.default_agent = false;
        }
        self.save(&machine);
    }
}

enum Answer {
    Nothing,
    Text(String),
    Objects(HashMap<OwnedObjectPath, HashMap<String, Properties>>),
    Properties(Properties),
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut address, mut machine, mut state) = (None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--address" => address = args.next(),
            "--machine" => machine = args.next(),
            "--state" => state = args.next(),
            other => {
                eprintln!("usage: oxyos-fake-bluez --address ADDRESS --machine FILE --state FILE ({other}?)");
                std::process::exit(2);
            }
        }
    }
    let (Some(address), Some(machine), Some(state)) = (address, machine, state) else {
        eprintln!("usage: oxyos-fake-bluez --address ADDRESS --machine FILE --state FILE");
        std::process::exit(2);
    };
    let spec: Json = serde_json::from_str(&std::fs::read_to_string(&machine).expect("the machine"))
        .expect("the machine's JSON");
    let adapter = spec
        .get("adapter")
        .filter(|a| !a.is_null())
        .map(|a| Adapter {
            name: a
                .get("name")
                .and_then(Json::as_str)
                .unwrap_or("OxyOS")
                .to_string(),
            address: a
                .get("address")
                .and_then(Json::as_str)
                .unwrap_or("00:1A:7D:DA:71:13")
                .to_string(),
            powered: a.get("powered").and_then(Json::as_bool).unwrap_or(true)
                && !a.get("blocked").and_then(Json::as_bool).unwrap_or(false),
            blocked: a.get("blocked").and_then(Json::as_bool).unwrap_or(false),
            discoverable: false,
        });
    let devices = spec
        .get("devices")
        .and_then(Json::as_array)
        .map(|devices| devices.iter().map(Device::of).collect())
        .unwrap_or_default();
    let bus = zbus::blocking::connection::Builder::address(address.as_str())
        .and_then(|builder| builder.name("org.bluez"))
        .and_then(|builder| builder.build())
        .expect("the bus");
    let fake = Arc::new(Fake {
        bus: bus.clone(),
        machine: Mutex::new(Machine {
            adapter,
            devices,
            ..Machine::default()
        }),
        state: state.into(),
    });
    fake.save(&fake.machine.lock().unwrap());
    // Clients leaving the bus.
    {
        let fake = fake.clone();
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")
            .and_then(|b| b.member("NameOwnerChanged"))
            .expect("a rule")
            .build();
        let owners = zbus::blocking::MessageIterator::for_match_rule(rule, &bus, Some(64))
            .expect("following the bus's names");
        std::thread::spawn(move || {
            for message in owners.flatten() {
                if let Ok((name, _, owner)) =
                    message.body().deserialize::<(String, String, String)>()
                {
                    if owner.is_empty() && name.starts_with(':') {
                        fake.left(&name);
                    }
                }
            }
        });
    }
    for message in zbus::blocking::MessageIterator::from(&bus).flatten() {
        if message.message_type() != zbus::message::Type::MethodCall {
            continue;
        }
        if let Some(outcome) = fake.handle(&message) {
            fake.answer(&message, outcome);
        }
    }
}
