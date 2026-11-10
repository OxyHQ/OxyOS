//! The readiness of what a unit started, for the service manager (docs/adr/0017-boot-logo.mdx):
//! `READY=1` on the socket `NOTIFY_SOCKET` names, sd_notify(3)'s protocol, once the first frame
//! is on the display: the session's, or the login screen's (the login program, `oxyos-login`,
//! sends it for its greeter, which runs in a logind session of its own, outside the unit). A
//! unit of `Type=notify` lets the boot logo go only then (`plymouth quit --retain-splash`), so the
//! logo is on screen until the splash, the same pixels, replaces it.
//!
//! Started from a terminal login there is no such socket and nothing is sent. Telling Plymouth
//! itself is the unit's: plymouthd answers only root.
//!
//! **The watchdog** (docs/adr/0061-crashes-and-resetting-an-application.mdx, decision 6, amended):
//! a unit with `WatchdogSec=` is killed and started again by systemd when its program has not
//! said `WATCHDOG=1` for that long, as launchd's watchdog does a wedged daemon. A program says it
//! from the loop that would be wedged, never from a thread of its own that would go on saying it
//! while that loop hangs: [`wait_readable`] is the accept loop of OxyOS's system services saying
//! it at half the watchdog's time while it waits for the next connection.

use rustix::fd::AsFd;
use std::ffi::OsString;
use std::io;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::time::Duration;

/// Where the service manager listens: a socket's path, or (written with a leading `@`) a name
/// in the abstract namespace; and how often this process is to say it is alive, when its unit
/// has a watchdog.
#[derive(Debug, PartialEq, Eq)]
pub struct Notify {
    socket: OsString,
    watchdog: Option<Duration>,
}

impl Notify {
    /// The service manager's socket from the environment, and its watchdog when it is this
    /// process's (`WATCHDOG_USEC`, `WATCHDOG_PID`), taken out of it so that nothing the program
    /// starts (a session's shell, its applications) inherits them. Called first in `main`,
    /// before any thread or child exists.
    pub fn take() -> Option<Self> {
        let socket = std::env::var_os("NOTIFY_SOCKET");
        let usec = std::env::var_os("WATCHDOG_USEC");
        let pid = std::env::var_os("WATCHDOG_PID");
        for name in ["NOTIFY_SOCKET", "WATCHDOG_USEC", "WATCHDOG_PID"] {
            std::env::remove_var(name);
        }
        let watchdog = watchdog(usec, pid, std::process::id());
        Self::from_socket(socket?).map(|notify| Self { watchdog, ..notify })
    }

    fn from_socket(socket: OsString) -> Option<Self> {
        (!socket.is_empty()).then_some(Self {
            socket,
            watchdog: None,
        })
    }

    /// Tells the service manager the session is ready: its first frame is shown.
    pub fn ready(&self) -> io::Result<()> {
        self.send(b"READY=1")
    }

    /// How often to say [`Notify::alive`]: half the unit's `WatchdogSec=`, as sd_watchdog_enabled(3)
    /// advises, when it has one.
    pub fn watchdog(&self) -> Option<Duration> {
        self.watchdog
    }

    /// Tells the service manager the program's main loop is alive (`WATCHDOG=1`).
    pub fn alive(&self) -> io::Result<()> {
        self.send(b"WATCHDOG=1")
    }

    fn send(&self, message: &[u8]) -> io::Result<()> {
        let socket = UnixDatagram::unbound()?;
        socket.send_to_addr(message, &self.address()?)?;
        Ok(())
    }

    fn address(&self) -> io::Result<SocketAddr> {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::ffi::OsStrExt;
        let bytes = self.socket.as_bytes();
        match bytes.strip_prefix(b"@") {
            Some(name) => SocketAddr::from_abstract_name(name),
            None => SocketAddr::from_pathname(&self.socket),
        }
    }
}

/// The pace of a watchdog systemd set for process `own`: half of `WATCHDOG_USEC`, only when
/// `WATCHDOG_PID` is absent or names `own` (a watchdog set for another process is not this one's).
fn watchdog(usec: Option<OsString>, pid: Option<OsString>, own: u32) -> Option<Duration> {
    let usec: u64 = usec?.to_str()?.parse().ok().filter(|usec| *usec > 0)?;
    if let Some(pid) = pid {
        if pid.to_str()?.parse::<u32>().ok()? != own {
            return None;
        }
    }
    Some(Duration::from_micros(usec / 2))
}

/// Waits until `fd` can be read (a listening socket has a connection), saying the program is
/// alive at its watchdog's pace meanwhile and once as it returns: the accept loop of a system
/// service, which never waits on anything but its next connection. Without a watchdog it simply
/// waits.
pub fn wait_readable(fd: impl AsFd, notify: Option<&Notify>) -> io::Result<()> {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};
    let pace = notify.and_then(Notify::watchdog);
    let timeout = pace.map(|pace| Timespec {
        tv_sec: pace.as_secs().try_into().unwrap_or(i64::MAX),
        tv_nsec: pace.subsec_nanos().into(),
    });
    loop {
        let mut ready = [PollFd::new(&fd, PollFlags::IN)];
        let result = poll(&mut ready, timeout.as_ref());
        if let Some(notify) = notify.filter(|notify| notify.watchdog.is_some()) {
            // The service manager gone is its own business; the service goes on.
            let _ = notify.alive();
        }
        match result {
            Ok(0) => continue,
            Ok(_) => return Ok(()),
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn receive(socket: &UnixDatagram) -> Vec<u8> {
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buffer = [0u8; 64];
        let length = socket.recv(&mut buffer).unwrap();
        buffer[..length].to_vec()
    }

    #[test]
    fn ready_reaches_a_socket_by_its_path() {
        let directory = std::env::temp_dir().join(format!("oxyos-notify-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("notify");
        let _ = std::fs::remove_file(&path);
        let listener = UnixDatagram::bind(&path).unwrap();
        Notify::from_socket(path.clone().into())
            .unwrap()
            .ready()
            .unwrap();
        assert_eq!(receive(&listener), b"READY=1");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn ready_reaches_a_socket_in_the_abstract_namespace() {
        use std::os::linux::net::SocketAddrExt;
        let name = format!("oxyos-notify-test-{}", std::process::id());
        let listener =
            UnixDatagram::bind_addr(&SocketAddr::from_abstract_name(name.as_bytes()).unwrap())
                .unwrap();
        Notify::from_socket(format!("@{name}").into())
            .unwrap()
            .ready()
            .unwrap();
        assert_eq!(receive(&listener), b"READY=1");
    }

    #[test]
    fn a_watchdog_is_this_processs_alone_and_said_at_half_its_time() {
        let os = |text: &str| Some(OsString::from(text));
        assert_eq!(
            watchdog(os("30000000"), os("42"), 42),
            Some(Duration::from_secs(15))
        );
        // systemd names no process when it is the main one's.
        assert_eq!(
            watchdog(os("30000000"), None, 42),
            Some(Duration::from_secs(15))
        );
        // Another process's, none, zero or nonsense: no watchdog.
        assert_eq!(watchdog(os("30000000"), os("7"), 42), None);
        assert_eq!(watchdog(None, os("42"), 42), None);
        assert_eq!(watchdog(os("0"), None, 42), None);
        assert_eq!(watchdog(os("soon"), None, 42), None);
    }

    #[test]
    fn the_accept_loop_says_it_is_alive_while_it_waits_and_returns_with_a_connection() {
        let directory = std::env::temp_dir().join(format!("oxyos-watchdog-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("notify");
        let _ = std::fs::remove_file(&path);
        let manager = UnixDatagram::bind(&path).unwrap();
        let notify = Notify {
            socket: path.clone().into(),
            watchdog: Some(Duration::from_millis(50)),
        };
        let (listening, connecting) = std::os::unix::net::UnixStream::pair().unwrap();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(220));
            use std::io::Write;
            (&connecting).write_all(b"x").unwrap();
            connecting
        });
        wait_readable(&listening, Some(&notify)).unwrap();
        let _connecting = writer.join().unwrap();
        // Said at least every 50 ms while it waited, and as it returned.
        manager.set_nonblocking(true).unwrap();
        let mut said = 0;
        let mut buffer = [0u8; 64];
        while let Ok(length) = manager.recv(&mut buffer) {
            assert_eq!(&buffer[..length], b"WATCHDOG=1");
            said += 1;
        }
        assert!(said >= 4, "said {said} times");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn no_socket_is_no_service_manager() {
        assert_eq!(Notify::from_socket(OsString::new()), None);
    }

    #[test]
    fn a_service_manager_gone_is_an_error_to_report_not_to_end_on() {
        let path = std::env::temp_dir().join(format!("oxyos-notify-gone-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(Notify::from_socket(path.into()).unwrap().ready().is_err());
    }
}
