//! The built-in display's and the keyboard's backlights (docs/adr/0025-hardware-keys-and-control-center.mdx),
//! found in sysfs where udev finds them, read there, and set through logind's `SetBrightness` on
//! the session (no root, no writable sysfs): `oxyos_seat::Brightness`.
//!
//! - **The display's** is a device of the `backlight` class, the firmware's interface first,
//!   then the platform's, then the GPU's own (`raw`), as systemd and GNOME choose: a laptop's
//!   firmware knows its panel best.
//! - **The keyboard's** is an LED of the `leds` class whose name ends in `kbd_backlight` (the
//!   kernel's convention: `tpacpi::kbd_backlight`, `asus::kbd_backlight`). One with
//!   `brightness_hw_changed` is changed by the firmware itself, as a ThinkPad's Fn+Space does:
//!   its key only says it did.

use crate::{Level, FULL_LEVEL, STEPS};
use std::io;
use std::path::{Path, PathBuf};

/// What a backlight is set through: logind in a session, something else in tests.
pub trait Setter: Send {
    /// Sets the device `name` of `subsystem` (`backlight`, `leds`) to `value`, its own units.
    fn set(&self, subsystem: &str, name: &str, value: u32) -> io::Result<()>;
}

impl Setter for oxyos_seat::Brightness {
    fn set(&self, subsystem: &str, name: &str, value: u32) -> io::Result<()> {
        oxyos_seat::Brightness::set(self, subsystem, name, value)
    }
}

/// A backlight device found in sysfs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// `backlight` or `leds`.
    pub subsystem: &'static str,
    pub name: String,
    path: PathBuf,
}

fn read_number(path: &Path) -> io::Result<u32> {
    let text = std::fs::read_to_string(path)?;
    text.trim()
        .parse()
        .map_err(|_| io::Error::other(format!("{}: {:?}", path.display(), text.trim())))
}

/// The display's: the most trusted kind of `class/backlight`'s devices under `sysfs` (`/sys`),
/// by name among the same kind.
pub fn display(sysfs: &Path) -> Option<Device> {
    let rank = |kind: &str| match kind {
        "firmware" => 0,
        "platform" => 1,
        "raw" => 2,
        _ => 3,
    };
    let mut found: Vec<(u8, String, PathBuf)> = std::fs::read_dir(sysfs.join("class/backlight"))
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let path = entry.path();
            let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
            Some((rank(kind.trim()), name, path))
        })
        .filter(|(rank, _, _)| *rank < 3)
        .collect();
    found.sort();
    found.into_iter().next().map(|(_, name, path)| Device {
        subsystem: "backlight",
        name,
        path,
    })
}

/// The keyboard's: the first `class/leds` device under `sysfs` whose name ends in
/// `kbd_backlight`.
pub fn keyboard(sysfs: &Path) -> Option<Device> {
    let mut found: Vec<(String, PathBuf)> = std::fs::read_dir(sysfs.join("class/leds"))
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            name.ends_with("kbd_backlight")
                .then(|| (name, entry.path()))
        })
        .collect();
    found.sort();
    found.into_iter().next().map(|(name, path)| Device {
        subsystem: "leds",
        name,
        path,
    })
}

/// A raw brightness of `max` as a level, rounded.
pub fn level_of(raw: u32, max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    ((u64::from(raw.min(max)) * u64::from(FULL_LEVEL) + u64::from(max) / 2) / u64::from(max)) as u32
}

/// A level as a raw brightness of `max`, rounded.
pub fn raw_of(level: u32, max: u32) -> u32 {
    ((u64::from(level.min(FULL_LEVEL)) * u64::from(max) + u64::from(FULL_LEVEL) / 2)
        / u64::from(FULL_LEVEL)) as u32
}

impl Device {
    /// Its brightness as a level: sixteen steps, or as many as it has.
    pub fn level(&self) -> io::Result<Level> {
        let max = read_number(&self.path.join("max_brightness"))?;
        let raw = read_number(&self.path.join("brightness"))?;
        let mut level = Level::new(level_of(raw, max), false);
        level.steps = STEPS.min(max.max(1));
        level.fine_steps = (STEPS * 4).min(max.max(1));
        Ok(level)
    }

    /// Sets it to a level, through `setter`.
    pub fn set(&self, setter: &dyn Setter, level: u32) -> io::Result<()> {
        let max = read_number(&self.path.join("max_brightness"))?;
        setter.set(self.subsystem, &self.name, raw_of(level, max))
    }

    /// Whether the firmware changes it itself (the kernel reports its changes,
    /// `brightness_hw_changed`).
    pub fn changed_by_firmware(&self) -> bool {
        self.path.join("brightness_hw_changed").exists()
    }
}
