//! Local transport for the Oxy display protocol.
//!
//! - `SOCK_SEQPACKET` Unix sockets: one protocol message per packet, so a file descriptor always
//!   arrives with the message it belongs to and a short or oversized packet is detected.
//! - Peer credentials come from the kernel (`SO_PEERCRED`), never from the peer's messages. The
//!   session authority maps them to compositor grants.
//! - At most one descriptor per message, only on `AttachBuffer`; any other descriptor is closed
//!   and the message rejected. Received descriptors are close-on-exec.
//! - Shared-memory buffers must be sealed regular files at least as large as their layout. The
//!   compositor reads them with `pread`, so a misbehaving client cannot fault the compositor.

use oxy_protocol::{
    read_event, read_request, write_event, write_request, BufferLayout, Event, Request,
    MAX_PACKET_BYTES,
};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{FileType, MemfdFlags, SealFlags};
use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags,
    SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketAddrUnix, SocketFlags, SocketType,
};
use std::io::{self, Cursor, IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::path::Path;
use std::time::Duration;

/// Framed packet: 4-byte length plus the largest payload, plus one byte to detect oversize.
const PACKET_CAPACITY: usize = 4 + MAX_PACKET_BYTES + 1;
/// Descriptors accepted in one receive before the message is rejected as malformed.
const FD_CAPACITY: usize = 4;

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

/// Kernel-reported identity of the peer process at connect time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

pub struct Listener {
    fd: OwnedFd,
}

impl Listener {
    /// Binds a listening socket. The caller owns the path and its directory permissions.
    pub fn bind(path: &Path) -> io::Result<Self> {
        let fd = rustix::net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        rustix::net::bind(&fd, &SocketAddrUnix::new(path)?)?;
        rustix::net::listen(&fd, 16)?;
        Ok(Self { fd })
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        rustix::io::ioctl_fionbio(&self.fd, nonblocking)?;
        Ok(())
    }

    pub fn accept(&self) -> io::Result<Connection> {
        let fd = rustix::net::accept_with(&self.fd, SocketFlags::CLOEXEC)?;
        Ok(Connection { fd })
    }
}

impl AsFd for Listener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
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

    /// A connected pair, for a launcher that hands one end to a child it started.
    pub fn pair() -> io::Result<(Self, Self)> {
        let (a, b) = rustix::net::socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        Ok((Self { fd: a }, Self { fd: b }))
    }

    pub fn peer_credentials(&self) -> io::Result<Credentials> {
        let credentials = rustix::net::sockopt::socket_peercred(&self.fd)?;
        Ok(Credentials {
            pid: credentials.pid.as_raw_nonzero().get(),
            uid: credentials.uid.as_raw(),
            gid: credentials.gid.as_raw(),
        })
    }

    pub fn set_timeouts(&self, timeout: Duration) -> io::Result<()> {
        use rustix::net::sockopt::{set_socket_timeout, Timeout};
        set_socket_timeout(&self.fd, Timeout::Recv, Some(timeout))?;
        set_socket_timeout(&self.fd, Timeout::Send, Some(timeout))?;
        Ok(())
    }

    fn send_packet(&self, packet: &[u8], fd: Option<BorrowedFd<'_>>) -> io::Result<()> {
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        let fds;
        if let Some(fd) = fd {
            fds = [fd];
            if !control.push(SendAncillaryMessage::ScmRights(&fds)) {
                return Err(invalid("ancillary buffer too small"));
            }
        }
        let sent = rustix::net::sendmsg(
            &self.fd,
            &[IoSlice::new(packet)],
            &mut control,
            SendFlags::NOSIGNAL,
        )?;
        if sent != packet.len() {
            return Err(invalid("short packet send"));
        }
        Ok(())
    }

    /// Receives one packet and every descriptor that came with it.
    fn recv_packet(
        &self,
        storage: &mut [u8; PACKET_CAPACITY],
    ) -> io::Result<(usize, Vec<OwnedFd>)> {
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(FD_CAPACITY))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let received = rustix::net::recvmsg(
            &self.fd,
            &mut [IoSliceMut::new(storage)],
            &mut control,
            RecvFlags::CMSG_CLOEXEC,
        )?;
        let mut fds = Vec::new();
        for message in control.drain() {
            if let RecvAncillaryMessage::ScmRights(rights) = message {
                fds.extend(rights);
            }
        }
        if received.bytes == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "peer closed"));
        }
        if received.flags.contains(ReturnFlags::TRUNC) || received.bytes > PACKET_CAPACITY - 1 {
            return Err(invalid("oversized packet"));
        }
        if received.flags.contains(ReturnFlags::CTRUNC) {
            return Err(invalid("too many descriptors"));
        }
        Ok((received.bytes, fds))
    }

    pub fn send_request(&self, request: &Request, fd: Option<BorrowedFd<'_>>) -> io::Result<()> {
        if request.carries_fd() != fd.is_some() {
            return Err(invalid("descriptor does not match the request"));
        }
        let mut packet = Vec::with_capacity(PACKET_CAPACITY);
        write_request(&mut packet, request)?;
        self.send_packet(&packet, fd)
    }

    /// Receives one request. A descriptor is returned exactly when the request carries one;
    /// any mismatch closes the received descriptors and is an error.
    pub fn recv_request(&self) -> io::Result<(Request, Option<OwnedFd>)> {
        let mut storage = [0; PACKET_CAPACITY];
        let (length, mut fds) = self.recv_packet(&mut storage)?;
        let mut cursor = Cursor::new(&storage[..length]);
        let request = read_request(&mut cursor)?;
        if cursor.position() as usize != length {
            return Err(invalid("trailing bytes in packet"));
        }
        match (request.carries_fd(), fds.len()) {
            (true, 1) => Ok((request, fds.pop())),
            (false, 0) => Ok((request, None)),
            _ => Err(invalid("descriptor does not match the request")),
        }
    }

    pub fn send_event(&self, event: &Event) -> io::Result<()> {
        let mut packet = Vec::with_capacity(PACKET_CAPACITY);
        write_event(&mut packet, event)?;
        self.send_packet(&packet, None)
    }

    /// Sends without blocking. Returns `Ok(false)` if the peer's receive queue is full, so the
    /// compositor can keep the event in a bounded outbox instead of waiting on a slow client.
    pub fn try_send_event(&self, event: &Event) -> io::Result<bool> {
        let mut packet = Vec::with_capacity(PACKET_CAPACITY);
        write_event(&mut packet, event)?;
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        match rustix::net::sendmsg(
            &self.fd,
            &[IoSlice::new(&packet)],
            &mut control,
            SendFlags::NOSIGNAL | SendFlags::DONTWAIT,
        ) {
            Ok(sent) if sent == packet.len() => Ok(true),
            Ok(_) => Err(invalid("short packet send")),
            Err(rustix::io::Errno::AGAIN) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub fn recv_event(&self) -> io::Result<Event> {
        let mut storage = [0; PACKET_CAPACITY];
        let (length, fds) = self.recv_packet(&mut storage)?;
        if !fds.is_empty() {
            return Err(invalid("events carry no descriptors"));
        }
        let mut cursor = Cursor::new(&storage[..length]);
        let event = read_event(&mut cursor)?;
        if cursor.position() as usize != length {
            return Err(invalid("trailing bytes in packet"));
        }
        Ok(event)
    }

    /// Sends raw bytes; for tests of malformed peers.
    #[doc(hidden)]
    pub fn send_raw(&self, packet: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<()> {
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(FD_CAPACITY))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        if !fds.is_empty() && !control.push(SendAncillaryMessage::ScmRights(fds)) {
            return Err(invalid("ancillary buffer too small"));
        }
        rustix::net::sendmsg(
            &self.fd,
            &[IoSlice::new(packet)],
            &mut control,
            SendFlags::NOSIGNAL,
        )?;
        Ok(())
    }
}

impl AsFd for Connection {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportError {
    InvalidLayout,
    NotRegularFile,
    NotSealed,
    TooSmall,
    Io,
}

/// A validated client buffer in sealed shared memory.
#[derive(Debug)]
pub struct SharedBuffer {
    fd: OwnedFd,
    layout: BufferLayout,
}

impl SharedBuffer {
    /// Accepts `fd` only if it is a regular file sealed against shrinking and large enough for
    /// `layout`. The descriptor is closed on failure.
    pub fn import(fd: OwnedFd, layout: BufferLayout) -> Result<Self, ImportError> {
        let required = layout.required_bytes().ok_or(ImportError::InvalidLayout)?;
        let stat = rustix::fs::fstat(&fd).map_err(|_| ImportError::Io)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
            return Err(ImportError::NotRegularFile);
        }
        let seals = rustix::fs::fcntl_get_seals(&fd).map_err(|_| ImportError::NotSealed)?;
        if !seals.contains(SealFlags::SHRINK) {
            return Err(ImportError::NotSealed);
        }
        if u64::try_from(stat.st_size).unwrap_or(0) < required {
            return Err(ImportError::TooSmall);
        }
        Ok(Self { fd, layout })
    }

    pub fn layout(&self) -> BufferLayout {
        self.layout
    }

    /// Reads the pixels of one row (without stride padding) into `out`.
    pub fn read_row(&self, row: u32, out: &mut [u8]) -> io::Result<()> {
        let layout = self.layout;
        let width = layout.extent.width as usize * layout.format.bytes_per_pixel() as usize;
        if row >= layout.extent.height || out.len() != width {
            return Err(invalid("row out of range"));
        }
        let mut offset = u64::from(layout.offset) + u64::from(layout.stride) * u64::from(row);
        let mut filled = 0;
        while filled < out.len() {
            let read = rustix::io::pread(&self.fd, &mut out[filled..], offset)?;
            if read == 0 {
                return Err(invalid("buffer shorter than its layout"));
            }
            filled += read;
            offset += read as u64;
        }
        Ok(())
    }
}

/// Creates sealed shared memory of `bytes` for a client buffer. Size can no longer change.
pub fn create_shared_memory(name: &str, bytes: u64) -> io::Result<OwnedFd> {
    let fd = rustix::fs::memfd_create(name, MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)?;
    rustix::fs::ftruncate(&fd, bytes)?;
    rustix::fs::fcntl_add_seals(&fd, SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL)?;
    Ok(fd)
}

/// Writes `bytes` at `offset`; for clients filling their shared memory.
pub fn write_at(fd: BorrowedFd<'_>, mut offset: u64, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let written = rustix::io::pwrite(fd, bytes, offset)?;
        if written == 0 {
            return Err(invalid("short write"));
        }
        bytes = &bytes[written..];
        offset += written as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
