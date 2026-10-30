//! The services' sockets: `SOCK_SEQPACKET` Unix sockets, one JSON message per packet, the peer's
//! credentials from the kernel, as `oxyos-accountd`'s (ADR 0020). The fetcher's also carry a
//! descriptor with a message (`SCM_RIGHTS`).

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, SocketAddrUnix, SocketFlags, SocketType,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{self, IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

/// The longest message.
pub const MAX_MESSAGE: usize = 64 * 1024;

/// A listening socket.
pub struct Listener {
    fd: OwnedFd,
}

impl Listener {
    /// Listens at `path`, replacing a socket an earlier run left there (never another file),
    /// with `mode`: the peer's credentials decide what it may do.
    pub fn bind(path: &Path, mode: u32) -> io::Result<Self> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_socket() => std::fs::remove_file(path)?,
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a socket", path.display()),
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let fd = rustix::net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        rustix::net::bind(&fd, &SocketAddrUnix::new(path)?)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
        rustix::net::listen(&fd, 16)?;
        Ok(Self { fd })
    }

    /// A listening socket systemd passed (socket activation's first descriptor, 3).
    pub fn from_fd(fd: OwnedFd) -> Self {
        Self { fd }
    }

    pub fn accept(&self) -> io::Result<Connection> {
        let fd = rustix::net::accept_with(&self.fd, SocketFlags::CLOEXEC)?;
        Ok(Connection { fd })
    }
}

/// The peer's identity, from the kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

pub struct Connection {
    fd: OwnedFd,
}

impl Connection {
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

    /// Another handle on the same connection: what a watcher sends on from another thread
    /// (each packet is sent whole, so two senders never mix their messages).
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

    /// How long a send may wait, alone: a clone shares the socket, so its receive timeout would
    /// be every clone's.
    pub fn set_send_timeout(&self, timeout: Duration) -> io::Result<()> {
        use rustix::net::sockopt::{set_socket_timeout, Timeout};
        set_socket_timeout(&self.fd, Timeout::Send, Some(timeout))?;
        Ok(())
    }

    pub fn set_timeout(&self, timeout: Duration) -> io::Result<()> {
        use rustix::net::sockopt::{set_socket_timeout, Timeout};
        set_socket_timeout(&self.fd, Timeout::Recv, Some(timeout))?;
        set_socket_timeout(&self.fd, Timeout::Send, Some(timeout))?;
        Ok(())
    }

    fn packet(message: &impl Serialize) -> io::Result<Vec<u8>> {
        let packet = serde_json::to_vec(message).map_err(io::Error::other)?;
        if packet.len() > MAX_MESSAGE {
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

    /// Sends a message with a descriptor.
    pub fn send_with_fd(&self, message: &impl Serialize, fd: BorrowedFd) -> io::Result<()> {
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

    /// The next message: `None` once the peer has closed, `Some(None)` for one that is too long
    /// or is not a message.
    pub fn receive<T: DeserializeOwned>(&self) -> io::Result<Option<Option<T>>> {
        Ok(self.receive_with_fd()?.map(|(message, _)| message))
    }

    /// The next message and the descriptor it carried, if one. A packet carrying more than one
    /// descriptor has them all closed and reads as no message.
    pub fn receive_with_fd<T: DeserializeOwned>(
        &self,
    ) -> io::Result<Option<(Option<T>, Option<OwnedFd>)>> {
        let mut packet = vec![0u8; MAX_MESSAGE + 1];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let received = rustix::net::recvmsg(
            &self.fd,
            &mut [IoSliceMut::new(&mut packet)],
            &mut control,
            RecvFlags::CMSG_CLOEXEC,
        )?;
        let mut fds: Vec<OwnedFd> = Vec::new();
        for message in control.drain() {
            if let RecvAncillaryMessage::ScmRights(received) = message {
                fds.extend(received);
            }
        }
        if received.bytes == 0 && fds.is_empty() {
            return Ok(None);
        }
        let length = received.bytes;
        let message = if length > MAX_MESSAGE || fds.len() > 1 {
            None
        } else {
            serde_json::from_slice(&packet[..length]).ok()
        };
        let fd = if fds.len() == 1 { fds.pop() } else { None };
        Ok(Some((message, fd)))
    }
}

impl AsFd for Connection {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
