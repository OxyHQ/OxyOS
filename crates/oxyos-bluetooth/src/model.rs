//! BlueZ's objects as the session follows them: read whole once (`GetManagedObjects`), then
//! changed by its signals (`InterfacesAdded`, `InterfacesRemoved`, `PropertiesChanged`) with no
//! call to BlueZ. Only what can change what a person is shown is kept; the [`State`] made from it
//! is told again only when it differs.

use crate::{is_audio, is_input, kind, Adapter, Busy, Device, State};
use std::collections::{BTreeMap, HashMap};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub const ADAPTER: &str = "org.bluez.Adapter1";
pub const DEVICE: &str = "org.bluez.Device1";
pub const BATTERY: &str = "org.bluez.Battery1";

/// An object's interfaces and their properties, as BlueZ sends them.
pub type Interfaces = HashMap<String, HashMap<String, OwnedValue>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AdapterObject {
    address: String,
    alias: String,
    name: String,
    powered: bool,
    /// BlueZ's `PowerState` (5.66 and later): `off-blocked` when rfkill blocks it.
    power_state: String,
    discovering: bool,
    discoverable: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DeviceObject {
    adapter: String,
    address: String,
    name: Option<String>,
    alias: Option<String>,
    icon: Option<String>,
    paired: bool,
    connected: bool,
    trusted: bool,
    uuids: Vec<String>,
    /// Its `Battery1`, while BlueZ has one for it.
    battery: Option<u8>,
}

/// The adapters and devices BlueZ has, by their objects' paths.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Objects {
    adapters: BTreeMap<String, AdapterObject>,
    devices: BTreeMap<String, DeviceObject>,
}

fn flag(value: &OwnedValue) -> Option<bool> {
    bool::try_from(&**value).ok()
}

fn text(value: &OwnedValue) -> Option<String> {
    match &**value {
        Value::Str(text) => Some(text.to_string()),
        Value::ObjectPath(path) => Some(path.to_string()),
        _ => None,
    }
}

fn texts(value: &OwnedValue) -> Option<Vec<String>> {
    match &**value {
        Value::Array(array) => Some(
            array
                .iter()
                .filter_map(|item| match item {
                    Value::Str(text) => Some(text.to_string()),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    }
}

impl AdapterObject {
    fn apply(&mut self, properties: &HashMap<String, OwnedValue>) {
        for (key, value) in properties {
            match key.as_str() {
                "Address" => self.address = text(value).unwrap_or_default(),
                "Alias" => self.alias = text(value).unwrap_or_default(),
                "Name" => self.name = text(value).unwrap_or_default(),
                "Powered" => self.powered = flag(value).unwrap_or(false),
                "PowerState" => self.power_state = text(value).unwrap_or_default(),
                "Discovering" => self.discovering = flag(value).unwrap_or(false),
                "Discoverable" => self.discoverable = flag(value).unwrap_or(false),
                _ => {}
            }
        }
    }
}

impl DeviceObject {
    fn apply(&mut self, properties: &HashMap<String, OwnedValue>) {
        for (key, value) in properties {
            match key.as_str() {
                "Adapter" => self.adapter = text(value).unwrap_or_default(),
                "Address" => self.address = text(value).unwrap_or_default(),
                "Name" => self.name = text(value),
                "Alias" => self.alias = text(value),
                "Icon" => self.icon = text(value),
                "Paired" => self.paired = flag(value).unwrap_or(false),
                "Connected" => self.connected = flag(value).unwrap_or(false),
                "Trusted" => self.trusted = flag(value).unwrap_or(false),
                "UUIDs" => self.uuids = texts(value).unwrap_or_default(),
                _ => {}
            }
        }
    }

    fn forget(&mut self, invalidated: &[String]) {
        for key in invalidated {
            match key.as_str() {
                "Name" => self.name = None,
                "Alias" => self.alias = None,
                "Icon" => self.icon = None,
                "UUIDs" => self.uuids.clear(),
                _ => {}
            }
        }
    }

    /// Whether it has a name of its own: one it says, or one the person gave it (BlueZ's alias
    /// without either is its address, dashed).
    fn named(&self) -> bool {
        self.name.as_deref().is_some_and(|name| !name.is_empty())
            || self
                .alias
                .as_deref()
                .is_some_and(|alias| !alias.is_empty() && alias != self.address.replace(':', "-"))
    }

    fn shown_name(&self) -> String {
        let dashed = self.address.replace(':', "-");
        match (&self.alias, &self.name) {
            (Some(alias), _) if !alias.is_empty() && *alias != dashed => alias.clone(),
            (_, Some(name)) if !name.is_empty() => name.clone(),
            _ => self.address.clone(),
        }
    }
}

impl Objects {
    /// What `GetManagedObjects` answered.
    pub fn read(objects: HashMap<OwnedObjectPath, Interfaces>) -> Self {
        let mut read = Self::default();
        for (path, interfaces) in objects {
            read.added(path.as_str(), &interfaces);
        }
        read
    }

    /// `InterfacesAdded`.
    pub fn added(&mut self, path: &str, interfaces: &Interfaces) {
        if let Some(properties) = interfaces.get(ADAPTER) {
            self.adapters
                .entry(path.to_string())
                .or_default()
                .apply(properties);
        }
        if let Some(properties) = interfaces.get(DEVICE) {
            self.devices
                .entry(path.to_string())
                .or_default()
                .apply(properties);
        }
        if let Some(properties) = interfaces.get(BATTERY) {
            let percentage = properties
                .get("Percentage")
                .and_then(|value| u8::try_from(&**value).ok());
            if let Some(device) = self.devices.get_mut(path) {
                device.battery = percentage.map(|p| p.min(100));
            }
        }
    }

    /// `InterfacesRemoved`.
    pub fn removed(&mut self, path: &str, interfaces: &[String]) {
        for interface in interfaces {
            match interface.as_str() {
                ADAPTER => {
                    self.adapters.remove(path);
                    self.devices.retain(|_, device| device.adapter != path);
                }
                DEVICE => {
                    self.devices.remove(path);
                }
                BATTERY => {
                    if let Some(device) = self.devices.get_mut(path) {
                        device.battery = None;
                    }
                }
                _ => {}
            }
        }
    }

    /// `PropertiesChanged` of `interface` at `path`.
    pub fn changed(
        &mut self,
        path: &str,
        interface: &str,
        changed: &HashMap<String, OwnedValue>,
        invalidated: &[String],
    ) {
        match interface {
            ADAPTER => {
                if let Some(adapter) = self.adapters.get_mut(path) {
                    adapter.apply(changed);
                }
            }
            DEVICE => {
                if let Some(device) = self.devices.get_mut(path) {
                    device.apply(changed);
                    device.forget(invalidated);
                }
            }
            BATTERY => {
                if let Some(device) = self.devices.get_mut(path) {
                    if let Some(value) = changed.get("Percentage") {
                        device.battery = u8::try_from(&**value).ok().map(|p| p.min(100));
                    }
                }
            }
            _ => {}
        }
    }

    /// The adapter used: the first BlueZ has (`hci0` before `hci1`).
    pub fn adapter_path(&self) -> Option<&str> {
        self.adapters.keys().next().map(String::as_str)
    }

    /// The object of the used adapter's device with this address.
    pub fn device_path(&self, address: &str) -> Option<&str> {
        let adapter = self.adapter_path()?;
        self.devices
            .iter()
            .find(|(_, device)| device.adapter == adapter && device.address == address)
            .map(|(path, _)| path.as_str())
    }

    /// A device's address, name and kind, by its object's path.
    pub fn describe(&self, path: &str) -> Option<(String, String, crate::Kind)> {
        let device = self.devices.get(path)?;
        Some((
            device.address.clone(),
            device.shown_name(),
            kind(device.icon.as_deref(), &device.uuids),
        ))
    }

    pub fn paired(&self, path: &str) -> bool {
        self.devices.get(path).is_some_and(|device| device.paired)
    }

    pub fn trusted(&self, path: &str) -> bool {
        self.devices.get(path).is_some_and(|device| device.trusted)
    }

    pub fn powered(&self) -> bool {
        self.adapter_path()
            .and_then(|path| self.adapters.get(path))
            .is_some_and(|adapter| adapter.powered)
    }

    pub fn discovering(&self) -> bool {
        self.adapter_path()
            .and_then(|path| self.adapters.get(path))
            .is_some_and(|adapter| adapter.discovering)
    }

    pub fn discoverable(&self) -> bool {
        self.adapter_path()
            .and_then(|path| self.adapters.get(path))
            .is_some_and(|adapter| adapter.discoverable)
    }

    /// What a person is shown, `busy` the session's calls under way by device address.
    pub fn state(&self, available: bool, busy: &BTreeMap<String, Busy>) -> State {
        if !available {
            return State::unavailable();
        }
        let Some(path) = self.adapter_path() else {
            return State {
                available,
                ..State::default()
            };
        };
        let adapter = &self.adapters[path];
        let mut devices: Vec<Device> = self
            .devices
            .values()
            .filter(|device| device.adapter == path && (device.paired || device.named()))
            .map(|device| {
                let kind = kind(device.icon.as_deref(), &device.uuids);
                Device {
                    id: device.address.clone(),
                    address: device.address.clone(),
                    name: device.shown_name(),
                    kind,
                    paired: device.paired,
                    connected: device.connected,
                    busy: busy.get(&device.address).copied(),
                    battery: device.battery,
                    audio: is_audio(kind, &device.uuids),
                    input: is_input(kind, &device.uuids),
                }
            })
            .collect();
        devices.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.address.cmp(&b.address))
        });
        State {
            available,
            adapter: Some(Adapter {
                name: if adapter.alias.is_empty() {
                    adapter.name.clone()
                } else {
                    adapter.alias.clone()
                },
                address: adapter.address.clone(),
                powered: adapter.powered,
                blocked: adapter.power_state == "off-blocked",
                discovering: adapter.discovering,
                discoverable: adapter.discoverable,
            }),
            devices,
        }
    }
}
