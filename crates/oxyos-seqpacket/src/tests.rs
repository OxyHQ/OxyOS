use super::*;
use rustix::fd::AsFd;
use rustix::net::{AddressFamily, SendAncillaryBuffer, SendAncillaryMessage, SendFlags};
use rustix::net::{SocketFlags, SocketType};

fn pair() -> (OwnedFd, OwnedFd) {
    rustix::net::socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap()
}

/// A peer's last packet, sent just before it closes, is read before the end, polled or waited
/// for: the kernel can report the end with it still queued (the module's comment). Many rounds,
/// since each one is a race, mostly polled (busily), where the race is widest. Every receiver
/// over this one tests the same in its own terms.
#[test]
fn the_last_packet_before_the_end_is_read_before_the_end() {
    for round in 0..2000 {
        let (ours, theirs) = pair();
        let peer = std::thread::spawn(move || {
            rustix::net::send(&theirs, b"last", SendFlags::NOSIGNAL).unwrap();
        });
        let mut buffer = [0u8; 16];
        let packet = loop {
            let flags = if round % 4 == 3 {
                RecvFlags::empty()
            } else {
                RecvFlags::DONTWAIT
            };
            match receive(ours.as_fd(), &mut buffer, false, flags) {
                Ok(Some(packet)) => break packet,
                Ok(None) => panic!("round {round}: the end before the packet"),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => std::hint::spin_loop(),
                Err(error) => panic!("round {round}: {error}"),
            }
        };
        assert_eq!(&buffer[..packet.length], b"last");
        peer.join().unwrap();
        assert!(
            receive(ours.as_fd(), &mut buffer, false, RecvFlags::DONTWAIT)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn packets_their_descriptors_and_what_did_not_fit() {
    let (ours, theirs) = pair();
    let send = |bytes: &[u8], fds: &[BorrowedFd<'_>]| {
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(16))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        if !fds.is_empty() {
            assert!(control.push(SendAncillaryMessage::ScmRights(fds)));
        }
        rustix::net::sendmsg(
            &theirs,
            &[io::IoSlice::new(bytes)],
            &mut control,
            SendFlags::NOSIGNAL,
        )
        .unwrap();
    };
    let mut buffer = [0u8; 4];
    // Nothing queued yet.
    assert_eq!(
        receive(ours.as_fd(), &mut buffer, true, RecvFlags::DONTWAIT)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    send(b"abc", &[]);
    let packet = receive(ours.as_fd(), &mut buffer, true, RecvFlags::empty())
        .unwrap()
        .unwrap();
    assert_eq!(
        (&buffer[..packet.length], packet.truncated),
        (&b"abc"[..], false)
    );
    // Longer than the buffer: cut, and said.
    send(b"abcdef", &[]);
    let packet = receive(ours.as_fd(), &mut buffer, true, RecvFlags::empty())
        .unwrap()
        .unwrap();
    assert_eq!((packet.length, packet.truncated), (4, true));
    // A descriptor with its packet, close-on-exec.
    let (a, b) = pair();
    send(b"fd", &[a.as_fd()]);
    let packet = receive(ours.as_fd(), &mut buffer, true, RecvFlags::empty())
        .unwrap()
        .unwrap();
    assert_eq!(packet.descriptors.len(), 1);
    assert!(!packet.descriptors_truncated);
    let flags = rustix::io::fcntl_getfd(&packet.descriptors[0]).unwrap();
    assert!(flags.contains(rustix::io::FdFlags::CLOEXEC));
    // Too many for the room: some are closed and the packet says so, for the caller to refuse.
    let many = [a.as_fd(); 2 * MAX_DESCRIPTORS];
    send(b"many", &many);
    let packet = receive(ours.as_fd(), &mut buffer, true, RecvFlags::empty())
        .unwrap()
        .unwrap();
    assert!(packet.descriptors.len() < many.len() && packet.descriptors_truncated);
    // None asked for, none taken.
    send(b"one", &[a.as_fd(), b.as_fd()]);
    let packet = receive(ours.as_fd(), &mut buffer, false, RecvFlags::empty())
        .unwrap()
        .unwrap();
    assert!(packet.descriptors.is_empty() && packet.descriptors_truncated);
    // A packet of descriptors alone is a packet, not the end.
    send(b"", &[a.as_fd()]);
    let packet = receive(ours.as_fd(), &mut buffer, true, RecvFlags::empty())
        .unwrap()
        .unwrap();
    assert_eq!((packet.length, packet.descriptors.len()), (0, 1));
    drop(theirs);
    assert!(receive(ours.as_fd(), &mut buffer, true, RecvFlags::empty())
        .unwrap()
        .is_none());
}
