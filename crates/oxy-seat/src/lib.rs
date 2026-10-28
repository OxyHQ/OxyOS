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
//!
//! Dropping the [`Seat`] gives control back.

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

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
}

fn failed(error: zbus::Error) -> io::Error {
    io::Error::other(error.to_string())
}

/// The session the compositor controls.
pub struct Seat {
    session: Proxy<'static>,
    seat: Proxy<'static>,
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
        std::thread::Builder::new()
            .name("oxy-seat".into())
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
