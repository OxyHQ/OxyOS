//! A card's connectors changing (a monitor plugged in or out): the kernel's uevents for the
//! card, `HOTPLUG=1` on its `drm` device, read from the kernel's own netlink socket (the group
//! udev reads; no daemon is needed in between). What changed is found by reading the connectors
//! again (docs/adr/0018-several-displays.mdx).

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::net::{netlink, AddressFamily, SocketFlags, SocketType};
use std::io;

/// The kernel's multicast group of uevents (udev's is 2, after its own processing).
const KERNEL_GROUP: u32 = 1;

/// Hears hotplug events of one card.
pub struct Hotplug {
    socket: OwnedFd,
    /// The card's device name as uevents give it (`dri/card1`).
    device: String,
}

impl Hotplug {
    /// Listens for hotplug events of the card at `/dev/dri/NAME` (`name` is `card1`, for one).
    pub fn open(name: &str) -> io::Result<Self> {
        let socket = rustix::net::socket_with(
            AddressFamily::NETLINK,
            SocketType::DGRAM,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            Some(netlink::KOBJECT_UEVENT),
        )?;
        rustix::net::bind(&socket, &netlink::SocketAddrNetlink::new(0, KERNEL_GROUP))?;
        Ok(Self {
            socket,
            device: format!("dri/{name}"),
        })
    }

    /// Whether the card's connectors may have changed since the last call: every uevent waiting
    /// is read. Never blocks.
    pub fn changed(&self) -> io::Result<bool> {
        let mut changed = false;
        let mut buffer = [0u8; 8192];
        loop {
            match rustix::net::recvfrom(
                &self.socket,
                &mut buffer[..],
                rustix::net::RecvFlags::empty(),
            ) {
                Ok((length, _, from)) => {
                    // Only the kernel (port 0) sends to this group; anything else is ignored.
                    let from_kernel = from
                        .and_then(|from| netlink::SocketAddrNetlink::try_from(from).ok())
                        .is_some_and(|from| from.pid() == 0);
                    changed |= from_kernel && is_hotplug(&buffer[..length], &self.device);
                }
                Err(rustix::io::Errno::AGAIN) => return Ok(changed),
                Err(rustix::io::Errno::INTR) => {}
                // The socket's buffer overflowed: events were lost, any could have been ours.
                Err(rustix::io::Errno::NOBUFS) => changed = true,
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl AsFd for Hotplug {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.socket.as_fd()
    }
}

/// Whether a kernel uevent (`ACTION@DEVPATH`, then `KEY=VALUE` fields, NUL-separated) is a
/// hotplug of the card `device` (its `DEVNAME`, `dri/card1`).
pub fn is_hotplug(event: &[u8], device: &str) -> bool {
    let (mut drm, mut hotplug, mut ours) = (false, false, false);
    for field in event.split(|&byte| byte == 0).skip(1) {
        match field {
            b"SUBSYSTEM=drm" => drm = true,
            b"HOTPLUG=1" => hotplug = true,
            _ => {
                if let Some(name) = field.strip_prefix(b"DEVNAME=") {
                    ours = name == device.as_bytes();
                }
            }
        }
    }
    drm && hotplug && ours
}
