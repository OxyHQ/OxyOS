//! The compositor's seat through systemd-logind (docs/adr/0006-seat.mdx): it runs as the user
//! of a session on a seat, not as root. logind gives it the session's devices and takes them
//! back when the session goes away.
//! - [`Seat::take`] takes control of the caller's session. logind then owns the session's
//!   terminal (graphics mode, keyboard off, switches by request).
//! - [`Seat::open`] opens a device (`TakeDevice`): the DRM card already DRM master, input
//!   devices revocable.
//! - [`Seat::events`] reports what logind did since: a device paused (another session took
//!   the seat; a `Pause` is acknowledged with [`Seat::paused`]) or resumed (with a new
//!   descriptor for an input device).
//! - [`Seat::switch_to`] asks for another terminal (the user's Ctrl+Alt+Fn).
//! - The lock (docs/adr/0010-lock-screen.mdx): [`Seat::events`] also reports logind's `Lock`
//!   (`loginctl lock-session`) and the machine about to sleep; [`Seat::hold_sleep`] delays
//!   sleep until [`Seat::release_sleep`], once the session is locked on screen, and
//!   [`Seat::set_locked_hint`] tells logind whether it is locked.
//! - Idle (the idle lock, ADR 0010): [`Seat::set_idle_hint`] tells logind whether the session is
//!   idle (its `IdleHint`, which its `IdleAction` follows), and [`Seat::events`] reports when
//!   the user's processes begin or stop holding idle back with logind's own inhibitor
//!   (`systemd-inhibit --what=idle`, as a video player does).
//!
//! Dropping the [`Seat`] gives control back.
//!
//! [`notify`] is systemd's other interface a program on a seat speaks: telling the unit that
//! started it that it is on screen (sd_notify's `READY=1`, docs/adr/0017-boot-logo.mdx).

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

pub mod notify;

const LOGIND: &str = "org.freedesktop.login1";
const SESSION: &str = "org.freedesktop.login1.Session";

/// A device by its numbers (`st_rdev`), as logind names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Device {
    pub major: u32,
    pub minor: u32,
}

impl Device {
    /// The device a path names.
    pub fn of(path: &Path) -> io::Result<Self> {
        Ok(Self::from_rdev(rustix::fs::stat(path)?.st_rdev))
    }

    /// The device a descriptor is open on.
    pub fn of_fd(fd: BorrowedFd<'_>) -> io::Result<Self> {
        Ok(Self::from_rdev(rustix::fs::fstat(fd)?.st_rdev))
    }

    fn from_rdev(rdev: u64) -> Self {
        Self {
            major: rustix::fs::major(rdev),
            minor: rustix::fs::minor(rdev),
        }
    }
}

/// How logind paused a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pause {
    /// The session is going away: stop using the device, then acknowledge ([`Seat::paused`]).
    Pause,
    /// Already paused (DRM master dropped, input revoked); nothing to acknowledge.
    Force,
    /// The device is gone.
    Gone,
}

/// What logind did with the session's devices.
#[derive(Debug)]
pub enum Event {
    Paused(Device, Pause),
    /// The device is the session's again. A DRM card keeps its descriptor (DRM master again);
    /// an input device comes with a new one, the old being revoked.
    Resumed(Device, OwnedFd),
    /// logind asks the session to lock. (Its `Unlock` is never followed: only the session's
    /// authentication unlocks.)
    Lock,
    /// The machine is about to sleep (`true`), or woke (`false`).
    PrepareForSleep(bool),
    /// The session's user began (`true`) or stopped holding idle back: one of their processes
    /// holds a logind *block* inhibitor of `idle` ([`holds_idle`]). Reported once as the seat is
    /// taken if one is held already, then as it changes.
    IdleInhibited(bool),
}

/// One of logind's inhibitors, as `ListInhibitors` lists it: what it inhibits (`:`-separated),
/// who and why, its mode (`block`, `delay`), and the user and process holding it.
pub type Inhibitor = (String, String, String, String, u32, u32);

/// Whether `inhibitors` hold idle back for the session of user `uid`: one of that user's
/// processes holds a *block* inhibitor of `idle`. Another user's does not keep this session
/// from going idle and locking; delay inhibitors do not apply to idle.
pub fn holds_idle(inhibitors: &[Inhibitor], uid: u32) -> bool {
    inhibitors.iter().any(|(what, _, _, mode, owner, _)| {
        *owner == uid && mode == "block" && what.split(':').any(|what| what == "idle")
    })
}

fn failed(error: zbus::Error) -> io::Error {
    io::Error::other(error.to_string())
}

/// The session the compositor controls.
pub struct Seat {
    session: Proxy<'static>,
    seat: Proxy<'static>,
    manager: Proxy<'static>,
    /// The delay inhibitor for sleep, while held.
    sleep: std::cell::RefCell<Option<OwnedFd>>,
    vt: Option<u32>,
    events: Receiver<Event>,
    /// Readable when an event is queued (`AsFd`).
    wake: UnixStream,
}

impl Seat {
    /// Takes control of the caller's session (`XDG_SESSION_ID`, or the session of this
    /// process). The session must be on a seat; another controller is not displaced.
    pub fn take() -> io::Result<Self> {
        let connection = Connection::system().map_err(failed)?;
        let manager = Proxy::new(
            &connection,
            LOGIND,
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .map_err(failed)?;
        let path: OwnedObjectPath = match std::env::var("XDG_SESSION_ID") {
            Ok(id) if !id.is_empty() => manager.call("GetSession", &(id)),
            _ => manager.call("GetSessionByPID", &(std::process::id())),
        }
        .map_err(failed)?;
        let session = Proxy::new(&connection, LOGIND, path.clone(), SESSION).map_err(failed)?;
        let (_, seat_path): (String, OwnedObjectPath) =
            session.get_property("Seat").map_err(failed)?;
        if seat_path.as_str() == "/" {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the session is on no seat",
            ));
        }
        let seat = Proxy::new(
            &connection,
            LOGIND,
            seat_path,
            "org.freedesktop.login1.Seat",
        )
        .map_err(failed)?;
        let vt: u32 = session.get_property("VTNr").map_err(failed)?;
        session
            .call::<_, _, ()>("TakeControl", &(false))
            .map_err(failed)?;
        // logind's signals, turned into events the compositor's loop polls for.
        let (sender, events) = channel();
        let (wake, mut wake_write) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        let signals = Proxy::new(&connection, LOGIND, path, SESSION).map_err(failed)?;
        let incoming = signals.receive_all_signals().map_err(failed)?;
        let sleeping = manager.receive_signal("PrepareForSleep").map_err(failed)?;
        // The inhibitors held: logind says its `BlockInhibited` changed. Listened to before the
        // first look, so no change between them is missed.
        let properties = Proxy::new(
            &connection,
            LOGIND,
            "/org/freedesktop/login1",
            "org.freedesktop.DBus.Properties",
        )
        .map_err(failed)?
        .receive_signal("PropertiesChanged")
        .map_err(failed)?;
        let uid = rustix::process::getuid().as_raw();
        let idle_held = move |manager: &Proxy<'static>| {
            manager
                .call::<_, _, Vec<Inhibitor>>("ListInhibitors", &())
                .map(|inhibitors| holds_idle(&inhibitors, uid))
        };
        let mut held = idle_held(&manager).unwrap_or(false);
        if held {
            // The receiver is this function's: the event is queued.
            let _ = sender.send(Event::IdleInhibited(true));
            wake_write.write_all(&[1])?;
        }
        let (inhibit_sender, mut inhibit_wake, inhibit_manager) =
            (sender.clone(), wake_write.try_clone()?, manager.clone());
        std::thread::Builder::new()
            .name("oxyos-seat-inhibitors".into())
            .spawn(move || {
                type Changed = (
                    String,
                    HashMap<String, zbus::zvariant::OwnedValue>,
                    Vec<String>,
                );
                for message in properties {
                    let Ok((interface, changed, invalidated)) =
                        message.body().deserialize::<Changed>()
                    else {
                        continue;
                    };
                    let inhibitors_changed = interface == "org.freedesktop.login1.Manager"
                        && (changed.contains_key("BlockInhibited")
                            || invalidated.iter().any(|name| name == "BlockInhibited"));
                    if !inhibitors_changed {
                        continue;
                    }
                    // A failed look keeps what was known.
                    let Ok(now) = idle_held(&inhibit_manager) else {
                        continue;
                    };
                    if now == held {
                        continue;
                    }
                    held = now;
                    if inhibit_sender.send(Event::IdleInhibited(held)).is_err()
                        || inhibit_wake.write_all(&[1]).is_err()
                    {
                        return;
                    }
                }
            })?;
        let (sleep_sender, mut sleep_wake) = (sender.clone(), wake_write.try_clone()?);
        std::thread::Builder::new()
            .name("oxyos-seat-sleep".into())
            .spawn(move || {
                for message in sleeping {
                    let Ok(start) = message.body().deserialize::<bool>() else {
                        continue;
                    };
                    if sleep_sender.send(Event::PrepareForSleep(start)).is_err()
                        || sleep_wake.write_all(&[1]).is_err()
                    {
                        return;
                    }
                }
            })?;
        std::thread::Builder::new()
            .name("oxyos-seat".into())
            .spawn(move || {
                for message in incoming {
                    let header = message.header();
                    let event = match header.member().map(|member| member.as_str()) {
                        Some("PauseDevice") => {
                            message.body().deserialize::<(u32, u32, String)>().ok().map(
                                |(major, minor, kind)| {
                                    let kind = match kind.as_str() {
                                        "pause" => Pause::Pause,
                                        "gone" => Pause::Gone,
                                        _ => Pause::Force,
                                    };
                                    Event::Paused(Device { major, minor }, kind)
                                },
                            )
                        }
                        Some("ResumeDevice") => message
                            .body()
                            .deserialize::<(u32, u32, zbus::zvariant::OwnedFd)>()
                            .ok()
                            .map(|(major, minor, fd)| {
                                Event::Resumed(Device { major, minor }, fd.into())
                            }),
                        Some("Lock") => Some(Event::Lock),
                        _ => None,
                    };
                    if let Some(event) = event {
                        if sender.send(event).is_err() || wake_write.write_all(&[1]).is_err() {
                            return;
                        }
                    }
                }
            })?;
        Ok(Self {
            session,
            seat,
            manager,
            sleep: std::cell::RefCell::new(None),
            vt: (vt != 0).then_some(vt),
            events,
            wake,
        })
    }

    /// The session's terminal, if it has one.
    pub fn vt(&self) -> Option<u32> {
        self.vt
    }

    /// Opens a device of the session (`TakeDevice`); also whether the session is inactive
    /// now (the device then comes paused, and resumes with the session).
    pub fn open(&self, path: &Path) -> io::Result<(OwnedFd, bool)> {
        let device = Device::of(path)?;
        let (fd, inactive): (zbus::zvariant::OwnedFd, bool) = self
            .session
            .call("TakeDevice", &(device.major, device.minor))
            .map_err(failed)?;
        Ok((fd.into(), inactive))
    }

    /// Gives a device back (`ReleaseDevice`).
    pub fn close(&self, device: Device) -> io::Result<()> {
        self.session
            .call::<_, _, ()>("ReleaseDevice", &(device.major, device.minor))
            .map_err(failed)
    }

    /// Acknowledges a [`Pause::Pause`]: the compositor stopped using the device.
    pub fn paused(&self, device: Device) -> io::Result<()> {
        self.session
            .call::<_, _, ()>("PauseDeviceComplete", &(device.major, device.minor))
            .map_err(failed)
    }

    /// Asks for the seat's terminal `vt` (the user's Ctrl+Alt+Fn).
    pub fn switch_to(&self, vt: u32) -> io::Result<()> {
        self.seat
            .call::<_, _, ()>("SwitchTo", &(vt))
            .map_err(failed)
    }

    /// Holds sleep back (a logind *delay* inhibitor) until [`Seat::release_sleep`]: when the
    /// machine is about to sleep, the session has logind's few seconds to lock on screen first.
    pub fn hold_sleep(&self) -> io::Result<()> {
        if self.sleep.borrow().is_some() {
            return Ok(());
        }
        let fd: zbus::zvariant::OwnedFd = self
            .manager
            .call(
                "Inhibit",
                &(
                    "sleep",
                    "OxyOS",
                    "Locking the screen before sleeping",
                    "delay",
                ),
            )
            .map_err(failed)?;
        *self.sleep.borrow_mut() = Some(fd.into());
        Ok(())
    }

    /// Lets the machine sleep: the session is locked on screen.
    pub fn release_sleep(&self) {
        self.sleep.borrow_mut().take();
    }

    /// Tells logind whether the session is locked (its `LockedHint`, as `loginctl` shows it).
    pub fn set_locked_hint(&self, locked: bool) -> io::Result<()> {
        self.session
            .call::<_, _, ()>("SetLockedHint", &(locked))
            .map_err(failed)
    }

    /// Tells logind whether the session is idle (its `IdleHint`): no input for the time the
    /// session locks after, and nothing holding idle back. logind refuses it from a session
    /// whose type is not graphical (systemd 257 takes it from x11, wayland and mir sessions
    /// alone, and judges a terminal's session by its terminal's use).
    pub fn set_idle_hint(&self, idle: bool) -> io::Result<()> {
        self.session
            .call::<_, _, ()>("SetIdleHint", &(idle))
            .map_err(failed)
    }

    /// What logind did since the last call; never blocks.
    pub fn events(&self) -> Vec<Event> {
        let mut drained = [0u8; 64];
        while matches!((&self.wake).read(&mut drained), Ok(n) if n > 0) {}
        self.events.try_iter().collect()
    }
}

impl AsFd for Seat {
    /// Readable when logind did something ([`Seat::events`]).
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        let _ = self.session.call::<_, _, ()>("ReleaseControl", &());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inhibitor(what: &str, mode: &str, uid: u32) -> Inhibitor {
        let text = String::from;
        (
            text(what),
            text("mpv"),
            text("Playing"),
            text(mode),
            uid,
            42,
        )
    }

    #[test]
    fn only_the_users_own_block_inhibitor_of_idle_holds_idle_back() {
        assert!(!holds_idle(&[], 1000));
        assert!(holds_idle(&[inhibitor("idle", "block", 1000)], 1000));
        // Among others of what it inhibits.
        assert!(holds_idle(&[inhibitor("sleep:idle", "block", 1000)], 1000));
        // A delay inhibitor (the session's own, for sleep), another user's, or one of
        // something else does not.
        assert!(!holds_idle(&[inhibitor("idle", "delay", 1000)], 1000));
        assert!(!holds_idle(&[inhibitor("idle", "block", 1001)], 1000));
        assert!(!holds_idle(
            &[inhibitor("sleep:shutdown", "block", 1000)],
            1000
        ));
        assert!(!holds_idle(&[inhibitor("idleness", "block", 1000)], 1000));
        assert!(holds_idle(
            &[
                inhibitor("sleep", "delay", 1000),
                inhibitor("idle", "block", 1000)
            ],
            1000
        ));
    }

    #[test]
    fn a_device_is_named_by_its_numbers() {
        // /dev/null is character device 1:3 on Linux.
        assert_eq!(
            Device::of(Path::new("/dev/null")).unwrap(),
            Device { major: 1, minor: 3 }
        );
        let null = std::fs::File::open("/dev/null").unwrap();
        assert_eq!(
            Device::of_fd(null.as_fd()).unwrap(),
            Device { major: 1, minor: 3 }
        );
    }
}
