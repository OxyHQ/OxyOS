//! The battery, from UPower over the system bus (docs/adr/0025-hardware-keys-and-control-center.mdx):
//! its display device, which is every battery of the machine as one (a laptop with two shows
//! one charge, as a Mac does), and whether the machine is on battery. UPower is in OxyOS's
//! image and on Debian's desktops; without it (nobody owns its name) there is no battery to
//! show, and the top bar shows none.

use crate::bus::{absent, failed, proxy};
use crate::Battery;
use oxyos_protocol::BatteryState;
use std::io;
use zbus::blocking::Connection;

const UPOWER: &str = "org.freedesktop.UPower";
const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
/// UPower's device kinds: a battery.
const KIND_BATTERY: u32 = 2;

/// What UPower says of its display device.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    pub present: bool,
    /// UPower's `Type`.
    pub kind: u32,
    pub percentage: f64,
    /// UPower's `State`: 0 unknown, 1 charging, 2 discharging, 3 empty, 4 fully charged,
    /// 5 pending charge (plugged in, not charging), 6 pending discharge.
    pub state: u32,
    pub on_battery: bool,
    /// Seconds.
    pub time_to_empty: i64,
    pub time_to_full: i64,
}

/// The battery a reading is.
pub fn battery(reading: Reading) -> Battery {
    if !reading.present || reading.kind != KIND_BATTERY {
        return Battery {
            on_ac: !reading.on_battery,
            ..Battery::ABSENT
        };
    }
    let on_ac = !reading.on_battery;
    let state = match reading.state {
        1 => BatteryState::Charging,
        2 | 3 | 6 => BatteryState::Discharging,
        4 => BatteryState::Full,
        5 => BatteryState::NotCharging,
        // Unknown: UPower says so between two readings; the power source decides.
        _ if on_ac => BatteryState::NotCharging,
        _ => BatteryState::Discharging,
    };
    let seconds = match state {
        BatteryState::Charging => reading.time_to_full,
        BatteryState::Discharging => reading.time_to_empty,
        _ => 0,
    };
    Battery {
        state,
        percent: reading.percentage.clamp(0.0, 100.0).round() as u8,
        on_ac,
        minutes: u32::try_from(seconds.max(0) / 60).unwrap_or(u32::MAX),
    }
}

/// The battery now, from UPower on `bus`.
pub fn read(bus: &Connection) -> io::Result<Battery> {
    let device = proxy(
        bus,
        UPOWER,
        DISPLAY_DEVICE.to_string(),
        "org.freedesktop.UPower.Device",
    )
    .map_err(failed)?;
    let present: bool = match device.get_property("IsPresent") {
        Ok(present) => present,
        Err(error) if absent(&error) => return Ok(Battery::ABSENT),
        Err(error) => return Err(failed(error)),
    };
    let upower =
        proxy(bus, UPOWER, "/org/freedesktop/UPower".to_string(), UPOWER).map_err(failed)?;
    Ok(battery(Reading {
        present,
        kind: device.get_property("Type").map_err(failed)?,
        percentage: device.get_property("Percentage").map_err(failed)?,
        state: device.get_property("State").map_err(failed)?,
        on_battery: upower.get_property("OnBattery").map_err(failed)?,
        time_to_empty: device.get_property("TimeToEmpty").map_err(failed)?,
        time_to_full: device.get_property("TimeToFull").map_err(failed)?,
    }))
}
