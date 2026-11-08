//! The battery's details from UPower (docs/adr/0057-battery-and-energy.mdx, decisions 1 and 2):
//! its health, read from the battery device itself (UPower's display device is every battery as
//! one, and has no design capacity or cycles), its charge limit and UPower's critical action.
//!
//! The charge limit is UPower's charge threshold (`EnableChargeThreshold`, UPower 1.90): UPower,
//! running as root, writes the battery's `charge_control_start_threshold` and
//! `charge_control_end_threshold` (the thresholds its hardware database gives the machine, 75 and
//! 80 by default), keeps the choice in its own state and applies it again at every start; polkit's
//! `org.freedesktop.UPower.enable-charging-limit` lets the active session's user do it without a
//! password. The session never writes sysfs.
//!
//! **What is in effect, not only what was chosen.** UPower's `ChargeThresholdEnabled` is the choice
//! it kept, which it writes again only when it starts and when a battery appears (UPower 1.90.9,
//! `up_device_battery_update_info`): never when the machine wakes, and it never reads the
//! thresholds back. Firmware that forgets them (a power cycle, which asus-wmi's EC does and
//! hibernation is) or another program writing them would leave it saying on while the battery
//! charges full. So the limit is said to be on only while the battery's own thresholds, read from
//! its sysfs directory, are UPower's; and [`keep_charge_limit`] asks UPower to write its own
//! choice again where they are not.

use crate::{absent, failed, proxy, Failure};
use serde::Serialize;
use std::io;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

pub const UPOWER: &str = "org.freedesktop.UPower";
pub const UPOWER_PATH: &str = "/org/freedesktop/UPower";
pub const DEVICE: &str = "org.freedesktop.UPower.Device";
/// UPower's device kinds: a battery.
const KIND_BATTERY: u32 = 2;

/// A battery whose maximum capacity is below this share of its design is worn, as a Mac's
/// *Service Recommended* says.
pub const WORN_BELOW: u8 = 80;

/// Where the kernel shows the power supplies; UPower's `NativePath` names one in it.
pub const POWER_SUPPLY: &str = "/sys/class/power_supply";

/// The charge limit, as UPower has it and the battery holds it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChargeLimit {
    /// In effect: UPower's choice, and the battery's thresholds what UPower writes for it (where
    /// they can be read).
    pub enabled: bool,
    /// UPower's choice (`ChargeThresholdEnabled`), whatever the battery holds.
    #[serde(skip)]
    pub kept: bool,
    /// Where charging stops while it is enabled, 0–100; none where the firmware chooses.
    pub end: Option<u32>,
    /// Where it starts again.
    pub start: Option<u32>,
}

/// The battery's health and charge limit.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Details {
    /// UPower's object for the battery.
    #[serde(skip)]
    pub path: String,
    /// Watt-hours it holds full now, and held when new.
    pub energy_full: f64,
    pub energy_full_design: f64,
    /// Charge cycles, where the firmware reports them.
    pub cycles: Option<u32>,
    /// Its charge limit; none where UPower cannot set one.
    pub charge_limit: Option<ChargeLimit>,
}

impl Details {
    /// What it holds full as a share of its design, in whole percent, at most 100 (a new battery
    /// may hold more than its design); none where the firmware says no design.
    pub fn max_capacity(&self) -> Option<u8> {
        if self.energy_full_design <= 0.0 || self.energy_full <= 0.0 {
            return None;
        }
        Some(
            (self.energy_full / self.energy_full_design * 100.0)
                .round()
                .clamp(0.0, 100.0) as u8,
        )
    }

    /// Whether it is worn ([`WORN_BELOW`]).
    pub fn worn(&self) -> bool {
        self.max_capacity()
            .is_some_and(|capacity| capacity < WORN_BELOW)
    }
}

/// The machine's battery: the first of UPower's devices that is a battery powering the machine
/// and present. None without UPower or a battery.
pub fn battery_path(bus: &Connection) -> io::Result<Option<String>> {
    let upower = proxy(bus, UPOWER, UPOWER_PATH, UPOWER).map_err(failed)?;
    let devices: Vec<OwnedObjectPath> = match upower.call("EnumerateDevices", &()) {
        Ok(devices) => devices,
        Err(error) if absent(&error) => return Ok(None),
        Err(error) => return Err(failed(error)),
    };
    for path in devices {
        let device = proxy(bus, UPOWER, path.as_str().to_string(), DEVICE).map_err(failed)?;
        let kind: u32 = device.get_property("Type").map_err(failed)?;
        let supply: bool = device.get_property("PowerSupply").map_err(failed)?;
        let present: bool = device.get_property("IsPresent").map_err(failed)?;
        if kind == KIND_BATTERY && supply && present {
            return Ok(Some(path.as_str().to_string()));
        }
    }
    Ok(None)
}

/// The battery's details now; none without one.
pub fn details(bus: &Connection) -> io::Result<Option<Details>> {
    let Some(path) = battery_path(bus)? else {
        return Ok(None);
    };
    let device = proxy(bus, UPOWER, path.clone(), DEVICE).map_err(failed)?;
    let cycles: i32 = device.get_property("ChargeCycles").unwrap_or(-1);
    // UPower before 1.90 has no charge threshold.
    let supported: bool = device
        .get_property("ChargeThresholdSupported")
        .unwrap_or(false);
    let charge_limit = if supported {
        // UPower says G_MAXUINT where a threshold is the firmware's.
        let threshold = |name: &str| {
            device
                .get_property::<u32>(name)
                .ok()
                .filter(|value| *value <= 100)
        };
        let kept: bool = device
            .get_property("ChargeThresholdEnabled")
            .map_err(failed)?;
        let end = threshold("ChargeEndThreshold");
        let start = threshold("ChargeStartThreshold");
        let native: String = device.get_property("NativePath").unwrap_or_default();
        let held = held(&native);
        // A threshold UPower leaves to the firmware is not compared; one the battery's directory
        // does not show is not either.
        let holds = |wanted: Option<u32>, read: Option<u32>| match (wanted, read) {
            (Some(wanted), Some(read)) => wanted == read,
            _ => true,
        };
        let in_effect =
            holds(start, held.and_then(|held| held.0)) && holds(end, held.and_then(|held| held.1));
        Some(ChargeLimit {
            enabled: kept && in_effect,
            kept,
            end,
            start,
        })
    } else {
        None
    };
    Ok(Some(Details {
        energy_full: device.get_property("EnergyFull").map_err(failed)?,
        energy_full_design: device.get_property("EnergyFullDesign").map_err(failed)?,
        cycles: u32::try_from(cycles).ok().filter(|cycles| *cycles > 0),
        charge_limit,
        path,
    }))
}

/// The thresholds the battery holds now (start, end), read from its directory in
/// [`POWER_SUPPLY`] (`native`, UPower's `NativePath`; an absolute path is itself); each none where
/// it cannot be read.
fn held(native: &str) -> Option<(Option<u32>, Option<u32>)> {
    if native.is_empty() || native.contains("..") {
        return None;
    }
    let directory = std::path::Path::new(POWER_SUPPLY).join(native);
    let read = |name: &str| {
        std::fs::read_to_string(directory.join(name))
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok())
    };
    Some((
        read("charge_control_start_threshold"),
        read("charge_control_end_threshold"),
    ))
}

/// Keeps UPower's own choice in effect: where UPower says the limit is on but the battery does not
/// hold its thresholds (the firmware forgot them, as UPower never looks), UPower is asked to write
/// them again (`EnableChargeThreshold(true)`, its own choice, through polkit as any change). True
/// if it was asked. The session never writes sysfs itself.
pub fn keep_charge_limit(bus: &Connection) -> Result<bool, Failure> {
    let limit = details(bus)
        .map_err(|_| Failure::Unavailable)?
        .and_then(|details| details.charge_limit);
    match limit {
        Some(limit) if limit.kept && !limit.enabled => set_charge_limit(bus, true).map(|()| true),
        _ => Ok(false),
    }
}

/// Limits the battery's charge to its threshold, or lets it charge full, through UPower.
pub fn set_charge_limit(bus: &Connection, enabled: bool) -> Result<(), Failure> {
    let path = battery_path(bus)
        .map_err(|_| Failure::Unavailable)?
        .ok_or(Failure::Unsupported)?;
    let device = proxy(bus, UPOWER, path, DEVICE).map_err(|error| Failure::of(&error))?;
    let supported: bool = device
        .get_property("ChargeThresholdSupported")
        .unwrap_or(false);
    if !supported {
        return Err(Failure::Unsupported);
    }
    device
        .call::<_, _, ()>("EnableChargeThreshold", &(enabled,))
        .map_err(|error| Failure::of(&error))
}

/// What UPower does once the battery reaches its action level (`GetCriticalAction`):
/// `HybridSleep`, `Hibernate`, `Suspend`, `PowerOff` or `Ignore`; none without UPower.
pub fn critical_action(bus: &Connection) -> io::Result<Option<String>> {
    let upower = proxy(bus, UPOWER, UPOWER_PATH, UPOWER).map_err(failed)?;
    match upower.call::<_, _, String>("GetCriticalAction", &()) {
        Ok(action) => Ok(Some(action)),
        Err(error) if absent(&error) => Ok(None),
        Err(error) => Err(failed(error)),
    }
}
