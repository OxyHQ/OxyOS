//! UPower, power-profiles-daemon and logind played on a bus of a test's own, never the system's
//! (docs/adr/0057-battery-and-energy.mdx, *Tests*): the part of each one's D-Bus API the session
//! uses, for a machine a JSON object describes, as the tests of `oxyos-power` and the development
//! compositor's scenarios (`--power-bus`) run them. Like the daemons:
//!
//! - UPower: its display device and the battery (`battery_BAT0`), `OnBattery`, `EnumerateDevices`,
//!   `GetCriticalAction`, `PropertiesChanged` as the machine changes, and `EnableChargeThreshold`,
//!   which writes the battery's thresholds to a sysfs directory of the test's own (its
//!   `charge_control_start_threshold` and `charge_control_end_threshold`: the thresholds when
//!   enabled, 0 and 100 when not, as UPower writes them), polkit's refusal played (`deny`);
//!   with a `state_dir`, as UPower 1.90.9 keeps the choice (`up-device-battery.c`): written to
//!   `charging-threshold-status` ("1" or "0") before the thresholds, and read again whenever the
//!   battery appears (the services served, which is UPower starting, and a battery plugged in
//!   again, `act`'s `present`), the thresholds then written again if it says "1". Nothing is
//!   written again when the machine wakes, as UPower writes nothing then. `NativePath` is the
//!   battery's sysfs directory (UPower's is the power supply's name in
//!   `/sys/class/power_supply`, which the session joins to that folder: an absolute path is
//!   itself);
//! - power-profiles-daemon: `Profiles`, `ActiveProfile`, `HoldProfile`, `ReleaseProfile` and
//!   `ProfileReleased`; a hold goes with the connection that took it, as the daemon's does;
//! - logind: `CanSuspend` and `Suspend`, which counts.
//!
//! [`Played::act`] changes the machine (unplugged, a lower charge) and [`Played::describe`] says
//! what the services hold, so a test asserts the machine's state, not only what was drawn.

use crate::profiles;
use crate::sleep::{LOGIND, MANAGER_PATH};
use crate::upower::{DEVICE, UPOWER, UPOWER_PATH};
use futures_lite::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const BATTERY: &str = "/org/freedesktop/UPower/devices/battery_BAT0";

/// The battery as described.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatterySpec {
    pub percent: f64,
    /// `charging`, `discharging`, `full` or `pending-charge` (plugged, held at its limit).
    pub state: String,
    #[serde(default)]
    pub time_to_empty: i64,
    #[serde(default)]
    pub time_to_full: i64,
    #[serde(default = "full")]
    pub energy_full: f64,
    #[serde(default = "full")]
    pub energy_full_design: f64,
    /// -1: the firmware says none.
    #[serde(default = "no_cycles")]
    pub cycles: i32,
    /// `none`, `low`, `critical` or `action`.
    #[serde(default = "none")]
    pub warning: String,
    /// UPower can limit its charge (its hardware database's thresholds).
    #[serde(default)]
    pub threshold: Option<Threshold>,
    /// It is in the machine (false: taken out, `act`'s `present`).
    #[serde(default = "yes")]
    pub present: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Threshold {
    pub start: u32,
    pub end: u32,
    #[serde(default)]
    pub enabled: bool,
}

fn full() -> f64 {
    50.0
}

fn no_cycles() -> i32 {
    -1
}

fn none() -> String {
    "none".into()
}

fn critical() -> String {
    "HybridSleep".into()
}

fn yes() -> bool {
    true
}

/// The machine as described.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    /// None: a machine with no battery.
    #[serde(default)]
    pub battery: Option<BatterySpec>,
    #[serde(default = "yes")]
    pub on_ac: bool,
    #[serde(default = "critical")]
    pub critical_action: String,
    /// power-profiles-daemon runs.
    #[serde(default = "yes")]
    pub profiles: bool,
    /// logind may suspend it.
    #[serde(default = "yes")]
    pub can_suspend: bool,
    /// Where `EnableChargeThreshold` writes the thresholds (the battery's sysfs directory).
    #[serde(default)]
    pub sysfs: Option<PathBuf>,
    /// UPower's own state directory (`/var/lib/upower`), where it keeps whether the limit is on;
    /// none: the threshold's `enabled` as described, kept nowhere.
    #[serde(default)]
    pub state_dir: Option<PathBuf>,
    /// What polkit refuses: `charge_limit`, `hold`, `suspend`.
    #[serde(default)]
    pub deny: Vec<String>,
}

struct Hold {
    cookie: u32,
    profile: String,
    reason: String,
    application: String,
    sender: String,
}

struct Model {
    machine: Machine,
    holds: Vec<Hold>,
    next_cookie: u32,
    /// The person's profile, under any hold.
    chosen: String,
    suspends: u32,
    calls: Vec<String>,
}

impl Model {
    fn active(&self) -> String {
        // power-saver wins over performance, as the daemon's holds do.
        if self
            .holds
            .iter()
            .any(|hold| hold.profile == profiles::POWER_SAVER)
        {
            profiles::POWER_SAVER.into()
        } else if let Some(hold) = self.holds.first() {
            hold.profile.clone()
        } else {
            self.chosen.clone()
        }
    }

    fn denied(&self, what: &str) -> bool {
        self.machine.deny.iter().any(|denied| denied == what)
    }

    /// The battery appeared (UPower started, or it was plugged in again): UPower's
    /// `up_device_battery_recover_battery_charging_threshold` reads the choice it kept and, if it
    /// says "1", writes the thresholds again; `ChargeThresholdEnabled` is what it read (false if
    /// the write failed). Without a state directory, the threshold is as described.
    fn recover(&mut self) {
        let Some(state_dir) = self.machine.state_dir.clone() else {
            return;
        };
        let sysfs = self.machine.sysfs.clone();
        let Some(threshold) = self
            .machine
            .battery
            .as_mut()
            .and_then(|battery| battery.threshold.as_mut())
        else {
            return;
        };
        let kept = std::fs::read_to_string(state_dir.join(STATE_FILE)).is_ok_and(|kept| kept == "1");
        threshold.enabled = kept
            && sysfs.as_deref().is_none_or(|sysfs| {
                write_thresholds(sysfs, threshold.start, threshold.end).is_ok()
            });
    }

    fn sysfs(&self) -> Json {
        let Some(directory) = &self.machine.sysfs else {
            return Json::Null;
        };
        let read = |name: &str| {
            std::fs::read_to_string(directory.join(name))
                .ok()
                .map(|text| text.trim().to_owned())
        };
        json!({
            "start": read("charge_control_start_threshold"),
            "end": read("charge_control_end_threshold"),
        })
    }
}

/// UPower's file of the choice, in its state directory.
const STATE_FILE: &str = "charging-threshold-status";

/// Writes the battery's thresholds, as UPower's `up_device_supply_battery_set_battery_charge_thresholds`.
fn write_thresholds(sysfs: &std::path::Path, start: u32, end: u32) -> std::io::Result<()> {
    for (name, value) in [
        ("charge_control_start_threshold", start),
        ("charge_control_end_threshold", end),
    ] {
        std::fs::write(sysfs.join(name), value.to_string())?;
    }
    Ok(())
}

type Shared = Arc<Mutex<Model>>;

/// The played services' connection, as their interfaces and follower reach it to signal; taken
/// when they go, so what the connection serves never keeps it open.
type Signaller = Arc<Mutex<Option<zbus::blocking::Connection>>>;

fn signalling(signaller: &Signaller) -> Option<zbus::blocking::Connection> {
    signaller.lock().unwrap().clone()
}

fn changed(bus: &zbus::blocking::Connection, path: &str, interface: &str, names: &[&str]) {
    let none: HashMap<&str, Value<'_>> = HashMap::new();
    let _ = bus.emit_signal(
        None::<()>,
        path,
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
        &(interface, none, names.to_vec()),
    );
}

fn state_number(state: &str) -> u32 {
    match state {
        "charging" => 1,
        "discharging" => 2,
        "empty" => 3,
        "full" => 4,
        "pending-charge" => 5,
        "pending-discharge" => 6,
        _ => 0,
    }
}

fn warning_number(warning: &str) -> u32 {
    match warning {
        "low" => 3,
        "critical" => 4,
        "action" => 5,
        _ => 1,
    }
}

struct Root {
    model: Shared,
}

#[zbus::interface(name = "org.freedesktop.UPower")]
impl Root {
    fn enumerate_devices(&self) -> Vec<OwnedObjectPath> {
        let model = self.model.lock().unwrap();
        model
            .machine
            .battery
            .iter()
            .map(|_| OwnedObjectPath::try_from(BATTERY).unwrap())
            .collect()
    }

    fn get_display_device(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(DISPLAY_DEVICE).unwrap()
    }

    fn get_critical_action(&self) -> String {
        self.model.lock().unwrap().machine.critical_action.clone()
    }

    #[zbus(property)]
    fn on_battery(&self) -> bool {
        !self.model.lock().unwrap().machine.on_ac
    }

    #[zbus(property)]
    fn daemon_version(&self) -> String {
        "1.90.9".into()
    }
}

struct Device {
    model: Shared,
    /// The display device: every battery as one, with no health of its own.
    display: bool,
}

impl Device {
    fn battery<T>(&self, read: impl FnOnce(&BatterySpec) -> T, otherwise: T) -> T {
        let model = self.model.lock().unwrap();
        model.machine.battery.as_ref().map_or(otherwise, read)
    }
}

#[zbus::interface(name = "org.freedesktop.UPower.Device")]
impl Device {
    fn enable_charge_threshold(&self, enabled: bool) -> zbus::fdo::Result<()> {
        let mut model = self.model.lock().unwrap();
        model.calls.push(format!("EnableChargeThreshold {enabled}"));
        if self.display {
            return Err(zbus::fdo::Error::Failed("not a battery".into()));
        }
        if model.denied("charge_limit") {
            return Err(zbus::fdo::Error::AccessDenied(
                "Not authorized: org.freedesktop.UPower.enable-charging-limit".into(),
            ));
        }
        let sysfs = model.machine.sysfs.clone();
        let state_dir = model.machine.state_dir.clone();
        let Some(threshold) = model
            .machine
            .battery
            .as_mut()
            .and_then(|battery| battery.threshold.as_mut())
        else {
            return Err(zbus::fdo::Error::Failed(
                "setting battery charge thresholds is unsupported".into(),
            ));
        };
        // The choice kept first, then the thresholds, as UPower does.
        if let Some(state_dir) = state_dir {
            std::fs::write(state_dir.join(STATE_FILE), if enabled { "1" } else { "0" })
                .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        }
        let (start, end) = if enabled {
            (threshold.start, threshold.end)
        } else {
            (0, 100)
        };
        if let Some(directory) = sysfs {
            write_thresholds(&directory, start, end).map_err(|error| {
                zbus::fdo::Error::Failed(format!("failed on setting charging threshold: {error}"))
            })?;
        }
        threshold.enabled = enabled;
        Ok(())
    }

    #[zbus(property, name = "Type")]
    fn kind(&self) -> u32 {
        // A battery; the display device of a machine without one is nothing (0).
        self.battery(|_| 2, 0)
    }

    #[zbus(property)]
    fn power_supply(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn is_present(&self) -> bool {
        self.battery(|battery| battery.present, false)
    }

    #[zbus(property)]
    fn percentage(&self) -> f64 {
        self.battery(|battery| battery.percent, 0.0)
    }

    #[zbus(property)]
    fn state(&self) -> u32 {
        self.battery(|battery| state_number(&battery.state), 0)
    }

    #[zbus(property)]
    fn time_to_empty(&self) -> i64 {
        self.battery(|battery| battery.time_to_empty, 0)
    }

    #[zbus(property)]
    fn time_to_full(&self) -> i64 {
        self.battery(|battery| battery.time_to_full, 0)
    }

    #[zbus(property)]
    fn warning_level(&self) -> u32 {
        self.battery(|battery| warning_number(&battery.warning), 1)
    }

    #[zbus(property)]
    fn energy_full(&self) -> f64 {
        self.battery(|battery| battery.energy_full, 0.0)
    }

    #[zbus(property)]
    fn energy_full_design(&self) -> f64 {
        self.battery(|battery| battery.energy_full_design, 0.0)
    }

    #[zbus(property)]
    fn charge_cycles(&self) -> i32 {
        if self.display {
            return -1;
        }
        self.battery(|battery| battery.cycles, -1)
    }

    #[zbus(property)]
    fn charge_threshold_supported(&self) -> bool {
        !self.display && self.battery(|battery| battery.present && battery.threshold.is_some(), false)
    }

    /// The battery's sysfs directory (UPower's: its name in `/sys/class/power_supply`).
    #[zbus(property)]
    fn native_path(&self) -> String {
        if self.display {
            return String::new();
        }
        let model = self.model.lock().unwrap();
        model
            .machine
            .sysfs
            .as_ref()
            .map_or_else(|| "BAT0".into(), |sysfs| sysfs.display().to_string())
    }

    #[zbus(property)]
    fn charge_threshold_enabled(&self) -> bool {
        self.battery(
            |battery| battery.present && battery.threshold.is_some_and(|threshold| threshold.enabled),
            false,
        )
    }

    #[zbus(property)]
    fn charge_start_threshold(&self) -> u32 {
        self.battery(
            |battery| battery.threshold.map_or(u32::MAX, |t| t.start),
            u32::MAX,
        )
    }

    #[zbus(property)]
    fn charge_end_threshold(&self) -> u32 {
        self.battery(
            |battery| battery.threshold.map_or(u32::MAX, |t| t.end),
            u32::MAX,
        )
    }
}

struct PowerProfiles {
    model: Shared,
    bus: Signaller,
}

#[zbus::interface(name = "org.freedesktop.UPower.PowerProfiles")]
impl PowerProfiles {
    fn hold_profile(
        &self,
        profile: String,
        reason: String,
        application_id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<u32> {
        let cookie = {
            let mut model = self.model.lock().unwrap();
            model.calls.push(format!("HoldProfile {profile}"));
            if model.denied("hold") {
                return Err(zbus::fdo::Error::AccessDenied(
                    "Not authorized: org.freedesktop.UPower.PowerProfiles.hold-profile".into(),
                ));
            }
            if profile != profiles::POWER_SAVER && profile != "performance" {
                return Err(zbus::fdo::Error::InvalidArgs(profile));
            }
            model.next_cookie += 1;
            let cookie = model.next_cookie;
            model.holds.push(Hold {
                cookie,
                profile,
                reason,
                application: application_id,
                sender: header.sender().map(|s| s.to_string()).unwrap_or_default(),
            });
            cookie
        };
        if let Some(bus) = signalling(&self.bus) {
            changed(
                &bus,
                profiles::PATH,
                profiles::DAEMON,
                &["ActiveProfile", "ActiveProfileHolds"],
            );
        }
        Ok(cookie)
    }

    fn release_profile(&self, cookie: u32) -> zbus::fdo::Result<()> {
        let released = {
            let mut model = self.model.lock().unwrap();
            model.calls.push(format!("ReleaseProfile {cookie}"));
            let before = model.holds.len();
            model.holds.retain(|hold| hold.cookie != cookie);
            before != model.holds.len()
        };
        if !released {
            return Err(zbus::fdo::Error::InvalidArgs(format!("no hold {cookie}")));
        }
        if let Some(bus) = signalling(&self.bus) {
            changed(
                &bus,
                profiles::PATH,
                profiles::DAEMON,
                &["ActiveProfile", "ActiveProfileHolds"],
            );
        }
        Ok(())
    }

    #[zbus(property)]
    fn active_profile(&self) -> String {
        self.model.lock().unwrap().active()
    }

    /// Choosing a profile releases every hold, each said by `ProfileReleased`, as the daemon does.
    #[zbus(property)]
    fn set_active_profile(&mut self, profile: String) {
        let released: Vec<u32> = {
            let mut model = self.model.lock().unwrap();
            model.calls.push(format!("ActiveProfile {profile}"));
            model.chosen = profile;
            model.holds.drain(..).map(|hold| hold.cookie).collect()
        };
        if let Some(bus) = signalling(&self.bus) {
            for cookie in released {
                let _ = bus.emit_signal(
                    None::<()>,
                    profiles::PATH,
                    profiles::DAEMON,
                    "ProfileReleased",
                    &(cookie,),
                );
            }
        }
    }

    #[zbus(property)]
    fn profiles(&self) -> Vec<HashMap<String, OwnedValue>> {
        ["power-saver", "balanced", "performance"]
            .iter()
            .map(|name| {
                HashMap::from([
                    (
                        "Profile".to_string(),
                        OwnedValue::try_from(Value::from(*name)).unwrap(),
                    ),
                    (
                        "Driver".to_string(),
                        OwnedValue::try_from(Value::from("placeholder")).unwrap(),
                    ),
                ])
            })
            .collect()
    }

    #[zbus(property)]
    fn active_profile_holds(&self) -> Vec<HashMap<String, OwnedValue>> {
        let model = self.model.lock().unwrap();
        model
            .holds
            .iter()
            .map(|hold| {
                HashMap::from([
                    (
                        "Profile".to_string(),
                        OwnedValue::try_from(Value::from(hold.profile.as_str())).unwrap(),
                    ),
                    (
                        "Reason".to_string(),
                        OwnedValue::try_from(Value::from(hold.reason.as_str())).unwrap(),
                    ),
                    (
                        "ApplicationId".to_string(),
                        OwnedValue::try_from(Value::from(hold.application.as_str())).unwrap(),
                    ),
                ])
            })
            .collect()
    }
}

struct Logind {
    model: Shared,
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl Logind {
    fn can_suspend(&self) -> String {
        let model = self.model.lock().unwrap();
        if !model.machine.can_suspend {
            "na".into()
        } else if model.denied("suspend") {
            "no".into()
        } else {
            "yes".into()
        }
    }

    fn suspend(&self, interactive: bool) -> zbus::fdo::Result<()> {
        let mut model = self.model.lock().unwrap();
        model.calls.push(format!("Suspend {interactive}"));
        if !model.machine.can_suspend || model.denied("suspend") {
            return Err(zbus::fdo::Error::AccessDenied("Access denied".into()));
        }
        model.suspends += 1;
        Ok(())
    }
}

/// The played machine's control (`so.oxy.OxyOS.PlayedPower` at [`CONTROL_PATH`]), for a test or a
/// development compositor in another process: `Act` changes the machine as [`Played::act`],
/// `Describe` says what the services hold as [`Played::describe`].
pub const CONTROL_PATH: &str = "/so/oxy/OxyOS/PlayedPower";
pub const CONTROL: &str = "so.oxy.OxyOS.PlayedPower";

struct Control {
    model: Shared,
    bus: Signaller,
}

#[zbus::interface(name = "so.oxy.OxyOS.PlayedPower")]
impl Control {
    fn act(&self, change: String) -> zbus::fdo::Result<()> {
        let change: Json = serde_json::from_str(&change)
            .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        let Some(bus) = signalling(&self.bus) else {
            return Err(zbus::fdo::Error::Failed("not served".into()));
        };
        act(&self.model, Some(&bus), &change).map_err(zbus::fdo::Error::InvalidArgs)
    }

    fn describe(&self) -> String {
        describe(&self.model).to_string()
    }
}

/// Changes the machine (`Played::act`), UPower saying what changed on `bus`.
fn act(
    model: &Shared,
    bus: Option<&zbus::blocking::Connection>,
    change: &Json,
) -> Result<(), String> {
    let object = change.as_object().ok_or("a change is an object")?;
    let mut device: Vec<&str> = Vec::new();
    let mut root = false;
    let mut recover = false;
    {
        let mut model = model.lock().unwrap();
        for (key, value) in object {
            if key == "on_ac" {
                model.machine.on_ac = value.as_bool().ok_or("on_ac: true or false")?;
                root = true;
                continue;
            }
            let battery = model
                .machine
                .battery
                .as_mut()
                .ok_or("the machine has no battery")?;
            match key.as_str() {
                "percent" => {
                    battery.percent = value.as_f64().ok_or("percent: a number")?;
                    device.push("Percentage");
                }
                "state" => {
                    battery.state = value.as_str().ok_or("state: a name")?.into();
                    device.push("State");
                }
                "time_to_empty" => {
                    battery.time_to_empty = value.as_i64().ok_or("seconds")?;
                    device.push("TimeToEmpty");
                }
                "time_to_full" => {
                    battery.time_to_full = value.as_i64().ok_or("seconds")?;
                    device.push("TimeToFull");
                }
                "warning" => {
                    battery.warning = value.as_str().ok_or("warning: a name")?.into();
                    device.push("WarningLevel");
                }
                // Taken out, or put back: put back, UPower recovers its choice as it does when
                // a battery appears.
                "present" => {
                    let present = value.as_bool().ok_or("present: true or false")?;
                    recover |= present && !battery.present;
                    battery.present = present;
                    device.push("IsPresent");
                    device.push("ChargeThresholdEnabled");
                }
                other => return Err(format!("not a change of the machine: {other}")),
            }
        }
        if recover {
            model.recover();
        }
    }
    if let Some(bus) = bus {
        if !device.is_empty() {
            changed(bus, DISPLAY_DEVICE, DEVICE, &device);
            changed(bus, BATTERY, DEVICE, &device);
        }
        if root {
            changed(bus, UPOWER_PATH, UPOWER, &["OnBattery"]);
        }
    }
    Ok(())
}

fn describe(model: &Shared) -> Json {
    let model = model.lock().unwrap();
    json!({
        "on_ac": model.machine.on_ac,
        "battery": model.machine.battery.as_ref().map(|battery| json!({
            "percent": battery.percent,
            "state": battery.state,
            "charge_limit": battery.threshold.map(|t| t.enabled),
        })),
        "sysfs": model.sysfs(),
        "active_profile": if model.machine.profiles { Json::from(model.active()) } else { Json::Null },
        "holds": model.holds.iter().map(|hold| json!({
            "profile": hold.profile, "reason": hold.reason, "application": hold.application,
        })).collect::<Vec<_>>(),
        "suspends": model.suspends,
        "calls": model.calls,
    })
}

/// The played services, serving until dropped.
pub struct Played {
    model: Shared,
    bus: Signaller,
    connection: Option<zbus::blocking::Connection>,
    following: Option<std::thread::JoinHandle<()>>,
}

impl Played {
    /// Serves `machine` on the bus at `address`, under UPower's, power-profiles-daemon's (when
    /// it runs) and logind's names.
    pub fn serve(address: &str, machine: Machine) -> zbus::Result<Self> {
        let model = Arc::new(Mutex::new(Model {
            holds: Vec::new(),
            next_cookie: 0,
            chosen: "balanced".into(),
            suspends: 0,
            calls: Vec::new(),
            machine: machine.clone(),
        }));
        // UPower starting: the battery appears to it, and it recovers the choice it kept.
        model.lock().unwrap().recover();
        let bus: Signaller = Arc::new(Mutex::new(None));
        let mut builder = zbus::blocking::connection::Builder::address(address)?
            .name(UPOWER)?
            .name(LOGIND)?
            .serve_at(
                UPOWER_PATH,
                Root {
                    model: model.clone(),
                },
            )?
            .serve_at(
                DISPLAY_DEVICE,
                Device {
                    model: model.clone(),
                    display: true,
                },
            )?
            .serve_at(
                BATTERY,
                Device {
                    model: model.clone(),
                    display: false,
                },
            )?
            .serve_at(
                MANAGER_PATH,
                Logind {
                    model: model.clone(),
                },
            )?
            .serve_at(
                CONTROL_PATH,
                Control {
                    model: model.clone(),
                    bus: bus.clone(),
                },
            )?;
        if machine.profiles {
            builder = builder.name(profiles::DAEMON)?.serve_at(
                profiles::PATH,
                PowerProfiles {
                    model: model.clone(),
                    bus: bus.clone(),
                },
            )?;
        }
        let connection = builder.build()?;
        *bus.lock().unwrap() = Some(connection.clone());
        // A hold goes with the connection that took it.
        let following = {
            let (model, bus) = (model.clone(), bus.clone());
            let rule = zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender("org.freedesktop.DBus")?
                .member("NameOwnerChanged")?
                .build();
            let mut stream = futures_lite::future::block_on(zbus::MessageStream::for_match_rule(
                rule,
                connection.inner(),
                Some(64),
            ))?;
            std::thread::Builder::new()
                .name("played-power".into())
                .spawn(move || {
                    while let Some(Ok(message)) = futures_lite::future::block_on(stream.next()) {
                        let Ok((name, _, owner)) =
                            message.body().deserialize::<(String, String, String)>()
                        else {
                            continue;
                        };
                        if !owner.is_empty() || !name.starts_with(':') {
                            continue;
                        }
                        let gone = {
                            let mut model = model.lock().unwrap();
                            let before = model.holds.len();
                            model.holds.retain(|hold| hold.sender != name);
                            before != model.holds.len()
                        };
                        if let (true, Some(bus)) = (gone, signalling(&bus)) {
                            changed(
                                &bus,
                                profiles::PATH,
                                profiles::DAEMON,
                                &["ActiveProfile", "ActiveProfileHolds"],
                            );
                        }
                    }
                })
                .ok()
        };
        Ok(Self {
            model,
            bus,
            connection: Some(connection),
            following,
        })
    }

    /// Changes the machine: any of the battery's `percent`, `state`, `time_to_empty`,
    /// `time_to_full`, `warning`, `present`, and `on_ac`; UPower then says what changed, as it does.
    pub fn act(&self, change: &Json) -> Result<(), String> {
        act(&self.model, self.connection.as_ref(), change)
    }

    /// What the services hold now.
    pub fn describe(&self) -> Json {
        describe(&self.model)
    }
}

impl Drop for Played {
    /// The connection closed and its follower waited for: nothing of it outlives it.
    fn drop(&mut self) {
        self.bus.lock().unwrap().take();
        if let Some(connection) = self.connection.take() {
            let _ = connection.close();
        }
        if let Some(following) = self.following.take() {
            let _ = following.join();
        }
    }
}
