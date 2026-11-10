//! The services' sockets: `SOCK_SEQPACKET` Unix sockets, one JSON message per packet, the peer's
//! credentials from the kernel (ADR 0020), a descriptor with a message where a service hands one
//! over (`SCM_RIGHTS`). The account service's, the install service's and its fetcher's, the
//! backup service's: one owner for what each of them had a copy of.
//!
//! A service's messages are bounded: [`MAX_MESSAGE`] for most, its own bound where it keeps one
//! ([`BoundedConnection`] with that bound, as the account service's 16 KiB). Messages may hold
//! secrets (a password, a device secret), so a message's bytes are written into and read from a
//! buffer that is overwritten when dropped.
//!
//! [`bind_in_place`] is the making of a listening socket alone, for a socket that speaks another
//! format (the login program's greeter socket, `oxyos-auth`).

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::net::{
    AddressFamily, RecvFlags, SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketAddrUnix,
    SocketFlags, SocketType,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{self, IoSlice};
use std::mem::MaybeUninit;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;
use zeroize::Zeroizing;

/// The longest message of the services that keep no bound of their own.
pub const MAX_MESSAGE: usize = 64 * 1024;

/// A listening socket of the services that keep no bound of their own.
pub type Listener = BoundedListener<MAX_MESSAGE>;

/// A connection of the services that keep no bound of their own.
pub type Connection = BoundedConnection<MAX_MESSAGE>;

/// Makes a listening `SOCK_SEQPACKET` socket at `path`, replacing a socket an earlier run left
/// there (never another file), with `mode`, owned by `owner` when given (else by this process's
/// user), with room for `backlog` connections waiting.
///
/// The socket is made listening under a name of its own beside `path` and renamed into place, so
/// `path` is never a socket that refuses: a client that connects as the name appears (the session
/// waiting for a service, the login screen) would otherwise be refused between the bind and the
/// listen, with nothing to say when the socket starts listening.
pub fn bind_in_place(
    path: &Path,
    mode: u32,
    owner: Option<u32>,
    backlog: i32,
) -> io::Result<OwnedFd> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a socket", path.display()),
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "a socket needs a name"))?;
    let making = path.with_file_name(format!(
        ".{}.{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    // A run of this process's that ended between its bind and its rename left it.
    if std::fs::symlink_metadata(&making).is_ok_and(|metadata| metadata.file_type().is_socket()) {
        std::fs::remove_file(&making)?;
    }
    let fd = rustix::net::socket_with(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )?;
    let made = (|| -> io::Result<()> {
        rustix::net::bind(&fd, &SocketAddrUnix::new(&making)?)?;
        std::fs::set_permissions(&making, std::fs::Permissions::from_mode(mode))?;
        if let Some(uid) = owner {
            rustix::fs::chown(
                &making,
                Some(rustix::fs::Uid::from_raw(uid)),
                None::<rustix::fs::Gid>,
            )?;
        }
        rustix::net::listen(&fd, backlog)?;
        std::fs::rename(&making, path)
    })();
    if let Err(error) = made {
        let _ = std::fs::remove_file(&making);
        return Err(error);
    }
    Ok(fd)
}

/// A service's listening socket, its messages at most `MAX` bytes.
pub struct BoundedListener<const MAX: usize> {
    fd: OwnedFd,
}

impl<const MAX: usize> BoundedListener<MAX> {
    /// Listens at `path` ([`bind_in_place`]) with `mode`: the peer's credentials decide what it
    /// may do.
    pub fn bind(path: &Path, mode: u32) -> io::Result<Self> {
        Ok(Self {
            fd: bind_in_place(path, mode, None, 16)?,
        })
    }

    /// A listening socket systemd passed (socket activation's first descriptor, 3).
    pub fn from_fd(fd: OwnedFd) -> Self {
        Self { fd }
    }

    pub fn accept(&self) -> io::Result<BoundedConnection<MAX>> {
        let fd = rustix::net::accept_with(&self.fd, SocketFlags::CLOEXEC)?;
        Ok(BoundedConnection { fd })
    }
}

/// The peer's identity, from the kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

/// A connection to or from a service, its messages at most `MAX` bytes.
pub struct BoundedConnection<const MAX: usize> {
    fd: OwnedFd,
}

impl<const MAX: usize> BoundedConnection<MAX> {
    pub fn connect(path: &Path) -> io::Result<Self> {
        let fd = rustix::net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        rustix::net::connect(&fd, &SocketAddrUnix::new(path)?)?;
        Ok(Self { fd })
    }

    /// Both ends of a socket pair, for tests and for a service talking to a helper it started.
    pub fn pair() -> io::Result<(Self, Self)> {
        let (a, b) = rustix::net::socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        Ok((Self { fd: a }, Self { fd: b }))
    }

    /// A connection on a connected `SOCK_SEQPACKET` socket made elsewhere.
    pub fn from_fd(fd: OwnedFd) -> Self {
        Self { fd }
    }

    /// Another handle on the same connection (a duplicated descriptor), to send from one thread
    /// while another waits to receive: each packet is sent whole, so two senders never mix their
    /// messages.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            fd: self.fd.try_clone()?,
        })
    }

    pub fn peer(&self) -> io::Result<Peer> {
        let credentials = rustix::net::sockopt::socket_peercred(&self.fd)?;
        Ok(Peer {
            pid: credentials.pid.as_raw_nonzero().get(),
            uid: credentials.uid.as_raw(),
            gid: credentials.gid.as_raw(),
        })
    }

    /// Waits at most `timeout` to send each message (none: as long as it takes), alone: a clone
    /// shares the socket, so its receive timeout would be every clone's.
    pub fn set_send_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        use rustix::net::sockopt::{set_socket_timeout, Timeout};
        set_socket_timeout(&self.fd, Timeout::Send, timeout)?;
        Ok(())
    }

    /// Waits at most `timeout` for each message (none: as long as it takes).
    pub fn set_receive_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        use rustix::net::sockopt::{set_socket_timeout, Timeout};
        set_socket_timeout(&self.fd, Timeout::Recv, timeout)?;
        Ok(())
    }

    /// Waits at most `timeout` to send each message and for each message.
    pub fn set_timeout(&self, timeout: Duration) -> io::Result<()> {
        use rustix::net::sockopt::{set_socket_timeout, Timeout};
        set_socket_timeout(&self.fd, Timeout::Recv, Some(timeout))?;
        set_socket_timeout(&self.fd, Timeout::Send, Some(timeout))?;
        Ok(())
    }

    /// A message's packet, in a buffer overwritten when dropped. The buffer is sized beforehand
    /// so that a message that fits leaves no copy behind growing it.
    fn packet(message: &impl Serialize) -> io::Result<Zeroizing<Vec<u8>>> {
        let mut packet = Zeroizing::new(Vec::with_capacity(MAX));
        serde_json::to_writer(&mut *packet, message).map_err(io::Error::other)?;
        if packet.len() > MAX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "message too long",
            ));
        }
        Ok(packet)
    }

    pub fn send(&self, message: &impl Serialize) -> io::Result<()> {
        rustix::net::send(&self.fd, &Self::packet(message)?, SendFlags::NOSIGNAL)?;
        Ok(())
    }

    /// Sends a message with a descriptor (`SCM_RIGHTS`).
    pub fn send_with_fd(&self, message: &impl Serialize, fd: BorrowedFd<'_>) -> io::Result<()> {
        let packet = Self::packet(message)?;
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        let fds = [fd];
        if !control.push(SendAncillaryMessage::ScmRights(&fds)) {
            return Err(io::Error::other("no room for the descriptor"));
        }
        rustix::net::sendmsg(
            &self.fd,
            &[IoSlice::new(&packet)],
            &mut control,
            SendFlags::NOSIGNAL,
        )?;
        Ok(())
    }

    /// The next message: `None` once the peer has closed, `Some(None)` for one that is too long,
    /// is not a message or carried more than one descriptor. A descriptor it carried is closed.
    pub fn receive<T: DeserializeOwned>(&self) -> io::Result<Option<Option<T>>> {
        Ok(self.receive_with_fd()?.map(|(message, _)| message))
    }

    /// The next message and the descriptor it carried, if one (close-on-exec): `None` once the
    /// peer has closed, a `None` message for one that is too long, is not a message, or carried
    /// more descriptors than one. A packet too long or with more descriptors than one has its
    /// descriptors closed.
    pub fn receive_with_fd<T: DeserializeOwned>(
        &self,
    ) -> io::Result<Option<(Option<T>, Option<OwnedFd>)>> {
        // A packet longer than the buffer is cut to it: one byte more than a message can be.
        let mut packet = Zeroizing::new(vec![0u8; MAX + 1]);
        let Some(received) =
            oxyos_seqpacket::receive(self.fd.as_fd(), &mut packet[..], true, RecvFlags::empty())?
        else {
            return Ok(None);
        };
        let mut fds = received.descriptors;
        if received.length > MAX
            || received.truncated
            || received.descriptors_truncated
            || fds.len() > 1
        {
            return Ok(Some((None, None)));
        }
        let message = serde_json::from_slice(&packet[..received.length]).ok();
        Ok(Some((message, fds.pop())))
    }
}

impl<const MAX: usize> AsFd for BoundedListener<MAX> {
    /// What a service waits on for its next connection.
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl<const MAX: usize> AsFd for BoundedConnection<MAX> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

#[cfg(test)]
mod tests;
