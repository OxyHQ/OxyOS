//! What changes the battery, the network, Bluetooth and the backlights, followed as it changes instead
//! of looked at on a clock (docs/adr/0025-hardware-keys-and-control-center.mdx): a session doing
//! nothing wakes for nothing (docs/performance.mdx, *Idle*). Each follower waits on its source and
//! on `stop` (readable once its caller's order to end is given) at once, never on a timer, calls
//! `changed` with what changed until it returns false, and returns when its source goes (the
//! caller follows it again once it is back):
//!
//! - [`bus`]: the system bus's signals from UPower (the battery) and, where nothing else follows
//!   them (the login screen), BlueZ (Bluetooth) and NetworkManager, and their names' owners (a service started or gone):
//!   only those that can change what is shown, each told as what it changed;
//! - [`backlights`]: the display's backlight and a keyboard's the firmware changes itself, by the
//!   kernel's notification of their sysfs attributes (`actual_brightness`, which the backlight
//!   class notifies whenever the brightness is set, by a key or by a program, and
//!   `brightness_hw_changed`);
//!
//! The volumes are told by the session's sound as it follows PipeWire (`oxyos_audio`, docs/adr/
//! 0056-sound-devices.mdx), not here.

use crate::backlight;
use crate::Control;
use futures_lite::StreamExt;
use rustix::event::{poll, PollFd, PollFlags};
use std::collections::HashMap;
use std::io::{self, Read, Seek};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What changed: the battery, the network, Bluetooth, or a level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Battery,
    Network,
    Bluetooth,
    Level(Control),
}

const UPOWER: &str = "org.freedesktop.UPower";
const BLUEZ: &str = "org.bluez";
const NM: &str = "org.freedesktop.NetworkManager";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// Signals that come together are one change, told this long after the first.
const SETTLE: Duration = Duration::from_millis(100);

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

/// What is followed on the bus, and what each says changed: only the signals that can change what
/// the top bar and Control Center show, so the bus daemon delivers no other to the session.
struct Rule {
    sender: &'static str,
    path: Option<&'static str>,
    interface: &'static str,
    member: &'static str,
    /// For `PropertiesChanged`, the interface whose properties (its first argument).
    of: Option<&'static str>,
    /// For `PropertiesChanged`, the properties shown (a change of none of them is nothing).
    shown: &'static [&'static str],
    change: Change,
}

/// UPower's: its display device's charge, state, time left and warning level, and whether the
/// machine is on battery; not its energy, rate or voltage, which change at every reading.
const UPOWER_RULES: [Rule; 2] = [
    Rule {
        sender: UPOWER,
        path: Some("/org/freedesktop/UPower/devices/DisplayDevice"),
        interface: PROPERTIES,
        member: "PropertiesChanged",
        of: Some("org.freedesktop.UPower.Device"),
        shown: &[
            "IsPresent",
            "Type",
            "Percentage",
            "State",
            "TimeToEmpty",
            "TimeToFull",
            "WarningLevel",
        ],
        change: Change::Battery,
    },
    Rule {
        sender: UPOWER,
        path: Some("/org/freedesktop/UPower"),
        interface: PROPERTIES,
        member: "PropertiesChanged",
        of: Some(UPOWER),
        shown: &["OnBattery"],
        change: Change::Battery,
    },
];

/// BlueZ's: an adapter on or off, a device connected or not, and adapters and devices that come
/// and go; not a discovery's devices' signal strengths and data.
const BLUEZ_RULES: [Rule; 4] = [
    Rule {
        sender: BLUEZ,
        path: None,
        interface: PROPERTIES,
        member: "PropertiesChanged",
        of: Some("org.bluez.Adapter1"),
        shown: &["Powered", "PowerState"],
        change: Change::Bluetooth,
    },
    Rule {
        sender: BLUEZ,
        path: None,
        interface: PROPERTIES,
        member: "PropertiesChanged",
        of: Some("org.bluez.Device1"),
        shown: &["Connected"],
        change: Change::Bluetooth,
    },
    Rule {
        sender: BLUEZ,
        path: Some("/"),
        interface: "org.freedesktop.DBus.ObjectManager",
        member: "InterfacesAdded",
        of: None,
        shown: &[],
        change: Change::Bluetooth,
    },
    Rule {
        sender: BLUEZ,
        path: Some("/"),
        interface: "org.freedesktop.DBus.ObjectManager",
        member: "InterfacesRemoved",
        of: None,
        shown: &[],
        change: Change::Bluetooth,
    },
];

/// NetworkManager's, where nothing else follows it (the login screen): its own state and Wi-Fi's
/// switches, and its devices' states; not its access points or scans.
const NM_RULES: [Rule; 2] = [
    Rule {
        sender: NM,
        path: Some("/org/freedesktop/NetworkManager"),
        interface: PROPERTIES,
        member: "PropertiesChanged",
        of: Some(NM),
        shown: &[
            "WirelessEnabled",
            "WirelessHardwareEnabled",
            "State",
            "Devices",
        ],
        change: Change::Network,
    },
    Rule {
        sender: NM,
        path: None,
        interface: "org.freedesktop.NetworkManager.Device",
        member: "StateChanged",
        of: None,
        shown: &[],
        change: Change::Network,
    },
];

impl Rule {
    fn matching(&self) -> zbus::Result<zbus::MatchRule<'static>> {
        let mut builder = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(self.sender)?
            .interface(self.interface)?
            .member(self.member)?;
        if let Some(path) = self.path {
            builder = builder.path(path)?;
        }
        if let Some(of) = self.of {
            builder = builder.arg(0, of)?;
        }
        Ok(builder.build())
    }

    /// What `message` (one this rule matched) changed, if anything shown.
    fn change_of(&self, message: &zbus::Message) -> Option<Change> {
        if self.of.is_none() {
            return Some(self.change);
        }
        type Changed = (
            String,
            HashMap<String, zbus::zvariant::OwnedValue>,
            Vec<String>,
        );
        let Ok((_, changed, invalidated)) = message.body().deserialize::<Changed>() else {
            return Some(self.change);
        };
        changed
            .keys()
            .chain(invalidated.iter())
            .any(|name| self.shown.contains(&name.as_str()))
            .then_some(self.change)
    }
}

/// What a stream's message says changed, if anything shown.
type Says = Box<dyn Fn(&zbus::Message) -> Option<Change>>;

/// The owner of `service` changing: it started or stopped.
fn owner_rule(service: &'static str) -> zbus::Result<zbus::MatchRule<'static>> {
    Ok(zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, service)?
        .build())
}

/// Follows the system bus (the module's doc), with NetworkManager's and BlueZ's signals where
/// `own` (no other part of the session follows them; docs/adr/0055-bluetooth.mdx): each
/// rule a match the bus daemon applies, so a signal that cannot change what is shown is never
/// delivered, and of those delivered, a change of no shown property is nothing. Without the bus,
/// returns the error; returns when the bus closes or `stop` is readable.
pub fn bus(
    own: bool,
    stop: BorrowedFd<'_>,
    mut changed: impl FnMut(Change) -> bool,
) -> io::Result<()> {
    let stop = async_io::Async::new(stop.try_clone_to_owned()?)?;
    // Its executor waits on a thread of its own without waking for nothing, and ends with it
    // (`oxyos_seat::system_bus`); its signals are waited for with futures-lite's block_on.
    let bus = oxyos_seat::system_bus()?;
    let inner = bus.inner();
    let mut rules: Vec<&Rule> = UPOWER_RULES.iter().collect();
    let mut owners = vec![(UPOWER, Change::Battery)];
    if own {
        rules.extend(BLUEZ_RULES.iter().chain(NM_RULES.iter()));
        owners.push((BLUEZ, Change::Bluetooth));
        owners.push((NM, Change::Network));
    }
    // Each stream and what its messages say changed.
    let mut streams = Vec::new();
    let mut says: Vec<Says> = Vec::new();
    for rule in rules {
        let matching = rule.matching().map_err(failed)?;
        streams.push(
            futures_lite::future::block_on(zbus::MessageStream::for_match_rule(
                matching,
                inner,
                Some(64),
            ))
            .map_err(failed)?,
        );
        says.push(Box::new(move |message| rule.change_of(message)));
    }
    for (service, change) in owners {
        let matching = owner_rule(service).map_err(failed)?;
        streams.push(
            futures_lite::future::block_on(zbus::MessageStream::for_match_rule(
                matching,
                inner,
                Some(8),
            ))
            .map_err(failed)?,
        );
        says.push(Box::new(move |_| Some(change)));
    }
    loop {
        let signal = futures_lite::future::block_on(futures_lite::future::or(
            async { Some(next(&mut streams).await) },
            async {
                let _ = stop.readable().await;
                None
            },
        ));
        // Stopped, or the bus closed.
        let Some(Some((index, message))) = signal else {
            return Ok(());
        };
        let Some(first) = says[index](&message) else {
            continue;
        };
        // What follows at once is said once, each change once.
        let mut said = vec![first];
        let until = Instant::now() + SETTLE;
        let ended = futures_lite::future::block_on(async {
            loop {
                let more =
                    futures_lite::future::or(async { Some(next(&mut streams).await) }, async {
                        async_io::Timer::at(until).await;
                        None
                    })
                    .await;
                match more {
                    Some(Some((index, message))) => said.extend(says[index](&message)),
                    Some(None) => return true,
                    None => return false,
                }
            }
        });
        said.dedup();
        let mut seen: Vec<Change> = Vec::new();
        for change in said {
            if !seen.contains(&change) {
                seen.push(change);
                if !changed(change) {
                    return Ok(());
                }
            }
        }
        if ended {
            return Ok(());
        }
    }
}

/// The next signal of any of `streams`, and which; `None` once one ends (the bus closed).
async fn next(streams: &mut [zbus::MessageStream]) -> Option<(usize, zbus::Message)> {
    futures_lite::future::poll_fn(|cx| {
        for (index, stream) in streams.iter_mut().enumerate() {
            match stream.poll_next(cx) {
                std::task::Poll::Ready(Some(Ok(message))) => {
                    return std::task::Poll::Ready(Some((index, message)))
                }
                // A message that could not be read is skipped.
                std::task::Poll::Ready(Some(Err(_))) => {}
                std::task::Poll::Ready(None) => return std::task::Poll::Ready(None),
                std::task::Poll::Pending => {}
            }
        }
        std::task::Poll::Pending
    })
    .await
}

/// An attribute the kernel notifies, opened, and the level it is.
struct Notified {
    file: std::fs::File,
    control: Control,
}

impl Notified {
    fn open(path: PathBuf, control: Control) -> Option<Self> {
        let mut notified = Self {
            file: std::fs::File::open(path).ok()?,
            control,
        };
        // Read once: a notification is what comes after the last read.
        notified.read();
        Some(notified)
    }

    fn read(&mut self) {
        let mut text = String::new();
        let _ = self.file.rewind();
        let _ = self.file.read_to_string(&mut text);
    }
}

/// Follows the backlights under `sysfs` (`/sys`; the module's doc) until `stop` is readable or
/// `changed` returns false. A machine with none waits for `stop` alone.
pub fn backlights(
    sysfs: &Path,
    stop: BorrowedFd<'_>,
    mut changed: impl FnMut(Change) -> bool,
) -> io::Result<()> {
    let mut watched: Vec<Notified> = Vec::new();
    if let Some(display) = backlight::display(sysfs) {
        watched.extend(Notified::open(
            display.attribute("actual_brightness"),
            Control::DisplayBrightness,
        ));
    }
    if let Some(keyboard) =
        backlight::keyboard(sysfs).filter(backlight::Device::changed_by_firmware)
    {
        watched.extend(Notified::open(
            keyboard.attribute("brightness_hw_changed"),
            Control::KeyboardBrightness,
        ));
    }
    loop {
        let mut fds = vec![PollFd::from_borrowed_fd(stop, PollFlags::IN)];
        // A sysfs attribute's notification is POLLPRI and POLLERR; a plain file never has one.
        fds.extend(
            watched
                .iter()
                .map(|notified| PollFd::from_borrowed_fd(notified.file.as_fd(), PollFlags::PRI)),
        );
        match poll(&mut fds, None) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
        if !fds[0].revents().is_empty() {
            return Ok(());
        }
        let notified: Vec<usize> = fds[1..]
            .iter()
            .enumerate()
            .filter(|(_, fd)| !fd.revents().is_empty())
            .map(|(index, _)| index)
            .collect();
        for index in notified {
            watched[index].read();
            if !changed(Change::Level(watched[index].control)) {
                return Ok(());
            }
        }
    }
}
