//! The readiness of what a unit started, for the service manager (docs/adr/0017-boot-logo.mdx):
//! `READY=1` on the socket `NOTIFY_SOCKET` names, sd_notify(3)'s protocol, once the first frame
//! is on the display: the session's, or the login screen's (the login program, `oxyos-login`,
//! sends it for its greeter, which runs in a logind session of its own, outside the unit). A
//! unit of `Type=notify` lets the boot logo go only then (`plymouth quit --retain-splash`), so the
//! logo is on screen until the splash, the same pixels, replaces it.
//!
//! Started from a terminal login there is no such socket and nothing is sent. Telling Plymouth
//! itself is the unit's: plymouthd answers only root.

use std::ffi::OsString;
use std::io;
use std::os::unix::net::{SocketAddr, UnixDatagram};

/// Where the service manager listens: a socket's path, or (written with a leading `@`) a name
/// in the abstract namespace.
#[derive(Debug, PartialEq, Eq)]
pub struct Notify(OsString);

impl Notify {
    /// The service manager's socket from the environment, taken out of it so that nothing the
    /// session starts (its shell, its applications) inherits it. Called first in `main`, before
    /// any thread or child exists.
    pub fn take() -> Option<Self> {
        let socket = std::env::var_os("NOTIFY_SOCKET")?;
        std::env::remove_var("NOTIFY_SOCKET");
        Self::from_socket(socket)
    }

    fn from_socket(socket: OsString) -> Option<Self> {
        (!socket.is_empty()).then_some(Self(socket))
    }

    /// Tells the service manager the session is ready: its first frame is shown.
    pub fn ready(&self) -> io::Result<()> {
        self.send(b"READY=1")
    }

    fn send(&self, message: &[u8]) -> io::Result<()> {
        let socket = UnixDatagram::unbound()?;
        socket.send_to_addr(message, &self.address()?)?;
        Ok(())
    }

    fn address(&self) -> io::Result<SocketAddr> {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::ffi::OsStrExt;
        let bytes = self.0.as_bytes();
        match bytes.strip_prefix(b"@") {
            Some(name) => SocketAddr::from_abstract_name(name),
            None => SocketAddr::from_pathname(&self.0),
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
