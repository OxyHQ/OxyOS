//! Receiving from the `SOCK_SEQPACKET` Unix sockets every OxyOS service and the display protocol
//! carry one message a packet on, without losing a peer's last packet to its end.
//!
//! Linux looks at a SEQPACKET socket's queue, then at whether the peer is gone, and the two looks
//! are not one (`__unix_dgram_recvmsg` in `net/unix/af_unix.c`, `__skb_wait_for_more_packets` in
//! `net/core/datagram.c`): a peer that sends its last packet and closes or shuts down between them
//! is reported gone, by a receive that waits as by one that does not, with that packet still
//! queued. Believed, that end loses the packet (a compositor's last event, a service's answer, a
//! worker saying its session started). Once the end has been seen, everything the peer sent is in
//! the queue, so [`receive`] reads once more, without waiting, before it reports the end.

use rustix::fd::{BorrowedFd, OwnedFd};
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags};
use std::io::{self, IoSliceMut};
use std::mem::MaybeUninit;

/// The most descriptors one receive is sure to take; more sent with a packet may be closed, and
/// [`Packet::descriptors_truncated`] then says so. No OxyOS message carries more than one: the
/// room is for seeing that a peer sent too many, which each caller refuses.
pub const MAX_DESCRIPTORS: usize = 4;

/// One packet received.
#[derive(Debug)]
pub struct Packet {
    /// The bytes written into the buffer: the packet's length, or the buffer's if `truncated`.
    pub length: usize,
    /// The packet was longer than the buffer; the rest of it is gone.
    pub truncated: bool,
    /// The descriptors it carried, close-on-exec.
    pub descriptors: Vec<OwnedFd>,
    /// It carried more descriptors than there was room for; those are closed.
    pub descriptors_truncated: bool,
}

/// Receives one packet into `buffer`, with the descriptors it carried if `descriptors` (else
/// any are closed), as `recvmsg` with `flags` (`RecvFlags::DONTWAIT`: `WouldBlock` while none is
/// queued). `Ok(None)` is the end of the connection, once a second read has found the queue
/// empty too.
pub fn receive(
    fd: BorrowedFd<'_>,
    buffer: &mut [u8],
    descriptors: bool,
    flags: RecvFlags,
) -> io::Result<Option<Packet>> {
    if let Some(packet) = receive_once(fd, buffer, descriptors, flags)? {
        return Ok(Some(packet));
    }
    match receive_once(fd, buffer, descriptors, flags | RecvFlags::DONTWAIT) {
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
        received => received,
    }
}

/// One `recvmsg`: a packet, or `None` where the kernel says the connection ended.
fn receive_once(
    fd: BorrowedFd<'_>,
    buffer: &mut [u8],
    descriptors: bool,
    flags: RecvFlags,
) -> io::Result<Option<Packet>> {
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_DESCRIPTORS))];
    let room = if descriptors { space.len() } else { 0 };
    let mut control = RecvAncillaryBuffer::new(&mut space[..room]);
    let received = rustix::net::recvmsg(
        fd,
        &mut [IoSliceMut::new(buffer)],
        &mut control,
        RecvFlags::CMSG_CLOEXEC | flags,
    )?;
    let mut fds = Vec::new();
    for message in control.drain() {
        if let RecvAncillaryMessage::ScmRights(rights) = message {
            fds.extend(rights);
        }
    }
    if received.bytes == 0 && fds.is_empty() {
        return Ok(None);
    }
    Ok(Some(Packet {
        length: received.bytes,
        truncated: received.flags.contains(ReturnFlags::TRUNC),
        descriptors: fds,
        descriptors_truncated: received.flags.contains(ReturnFlags::CTRUNC),
    }))
}

#[cfg(test)]
mod tests;
