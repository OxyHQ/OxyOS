//! Whether this machine is on a network, and through what: what
//! `@react-native-community/netinfo` asks the platform (the Bloom host's `RNCNetInfo`), from the
//! kernel, as Android's `ConnectivityManager` answers it.
//!
//! - **Connected** is a default route (IPv4's in `/proc/net/route`, IPv6's in
//!   `/proc/net/ipv6_route`) up through an interface that is up (`/sys/class/net/<name>/operstate`
//!   `up`, or `unknown` for links with no carrier to report, such as a tunnel). The route with the
//!   lowest metric is the one the machine uses.
//! - **Its type** is that interface's: Wi-Fi when it is wireless (`wireless` or `phy80211` in
//!   sysfs), cellular for a `wwan` device, a VPN for a tunnel (`wireguard`, or a link with no
//!   hardware address), Ethernet for the rest of the Ethernet links, else other.
//! - **Whether the internet answers** is not the kernel's to say: NetInfo checks it itself over
//!   HTTP (its `reachabilityUrl`) when the platform does not, so it is not answered here.
//! - **Changes** come from rtnetlink: the socket is readable when a link, an address or a route
//!   changes, and [`Connectivity::refresh`] says whether that changed the answer.

use std::fs;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

/// What carries the machine's traffic, by NetInfo's names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    None,
    Wifi,
    Ethernet,
    Cellular,
    Vpn,
    Other,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::None => "none",
            Kind::Wifi => "wifi",
            Kind::Ethernet => "ethernet",
            Kind::Cellular => "cellular",
            Kind::Vpn => "vpn",
            Kind::Other => "other",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub kind: Kind,
    /// The interface of the default route used, when connected.
    pub interface: Option<String>,
}

impl State {
    pub fn connected(&self) -> bool {
        self.kind != Kind::None
    }
}

/// `RTF_UP`: the route is usable.
const ROUTE_UP: u32 = 0x1;

/// The default routes in the routing tables under `root` (`/` on a machine): (metric, interface).
fn default_routes(root: &Path) -> Vec<(u32, String)> {
    let mut routes = Vec::new();
    let hex = |text: &str| u32::from_str_radix(text, 16).ok();
    if let Ok(text) = fs::read_to_string(root.join("proc/net/route")) {
        // Iface Destination Gateway Flags RefCnt Use Metric Mask ...
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 8 {
                continue;
            }
            let (Some(destination), Some(flags), Ok(metric), Some(mask)) = (
                hex(fields[1]),
                hex(fields[3]),
                fields[6].parse::<u32>(),
                hex(fields[7]),
            ) else {
                continue;
            };
            if destination == 0 && mask == 0 && flags & ROUTE_UP != 0 {
                routes.push((metric, fields[0].to_string()));
            }
        }
    }
    if let Ok(text) = fs::read_to_string(root.join("proc/net/ipv6_route")) {
        // destination prefix source prefix next-hop metric refcount use flags interface
        for line in text.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 {
                continue;
            }
            let default = fields[0].bytes().all(|b| b == b'0') && fields[1] == "00";
            let (Some(metric), Some(flags)) = (hex(fields[5]), hex(fields[8])) else {
                continue;
            };
            // The kernel lists unreachable defaults on the loopback, never usable.
            if default && flags & ROUTE_UP != 0 && fields[9] != "lo" {
                routes.push((metric, fields[9].to_string()));
            }
        }
    }
    routes.sort();
    routes
}

/// The interface's type, from sysfs under `root`, if it is up.
fn interface_kind(root: &Path, name: &str) -> Option<Kind> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return None;
    }
    let dir = root.join("sys/class/net").join(name);
    let read = |file: &str| fs::read_to_string(dir.join(file)).map(|text| text.trim().to_string());
    match read("operstate").ok()?.as_str() {
        "up" | "unknown" => {}
        _ => return None,
    }
    let device_type = read("uevent").ok().and_then(|uevent| {
        uevent
            .lines()
            .find_map(|line| line.strip_prefix("DEVTYPE=").map(str::to_string))
    });
    let link_type = read("type").ok().and_then(|text| text.parse::<u32>().ok());
    Some(
        if dir.join("wireless").exists() || dir.join("phy80211").exists() {
            Kind::Wifi
        } else if device_type.as_deref() == Some("wwan") {
            Kind::Cellular
        } else if device_type.as_deref() == Some("wireguard") || link_type == Some(65534) {
            // ARPHRD_NONE: a tunnel (tun, WireGuard), which carries a VPN.
            Kind::Vpn
        } else if link_type == Some(1) {
            Kind::Ethernet
        } else {
            Kind::Other
        },
    )
}

/// The machine's connectivity, read from the kernel's files under `root` (`/`).
pub fn current(root: &Path) -> State {
    for (_, interface) in default_routes(root) {
        if let Some(kind) = interface_kind(root, &interface) {
            return State {
                kind,
                interface: Some(interface),
            };
        }
    }
    State {
        kind: Kind::None,
        interface: None,
    }
}

/// The machine's connectivity as it changes.
pub struct Connectivity {
    /// rtnetlink, subscribed to links, addresses and routes; None where it cannot be opened (then
    /// the answer is read again only when asked).
    socket: Option<OwnedFd>,
    state: State,
}

impl Connectivity {
    pub fn new() -> Self {
        let socket = watch()
            .map_err(|error| eprintln!("[net] connectivity changes cannot be watched: {error}"))
            .ok();
        Self {
            socket,
            state: current(Path::new("/")),
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Readable when a link, an address or a route changed.
    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        self.socket.as_ref().map(AsFd::as_fd)
    }

    /// Takes what the kernel said and reads the answer again: whether it changed.
    pub fn refresh(&mut self) -> bool {
        if let Some(socket) = &self.socket {
            let mut buffer = [0u8; 8192];
            while matches!(rustix::net::recv(socket, &mut buffer, rustix::net::RecvFlags::DONTWAIT), Ok((n, _)) if n > 0)
            {
            }
        }
        let state = current(Path::new("/"));
        let changed = state != self.state;
        self.state = state;
        changed
    }
}

impl Default for Connectivity {
    fn default() -> Self {
        Self::new()
    }
}

/// An rtnetlink socket told of every link, address and route change.
fn watch() -> std::io::Result<OwnedFd> {
    use rustix::net::{netlink, AddressFamily, SocketFlags, SocketType};
    // RTMGRP_LINK, RTMGRP_IPV4_IFADDR, RTMGRP_IPV4_ROUTE, RTMGRP_IPV6_IFADDR, RTMGRP_IPV6_ROUTE.
    const GROUPS: u32 = 0x1 | 0x10 | 0x40 | 0x100 | 0x400;
    let socket = rustix::net::socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        // NETLINK_ROUTE is the default protocol.
        None,
    )?;
    rustix::net::bind(&socket, &netlink::SocketAddrNetlink::new(0, GROUPS))?;
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A machine's /proc and /sys for one test, removed when dropped.
    struct Machine(PathBuf);

    impl Machine {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("oxyos-connectivity-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("proc/net")).expect("proc");
            Self(root)
        }

        fn routes(&self, v4: &str, v6: &str) -> &Self {
            let header = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n";
            fs::write(self.0.join("proc/net/route"), format!("{header}{v4}")).expect("route");
            fs::write(self.0.join("proc/net/ipv6_route"), v6).expect("ipv6_route");
            self
        }

        fn link(
            &self,
            name: &str,
            state: &str,
            kind: u32,
            devtype: Option<&str>,
            wireless: bool,
        ) -> &Self {
            let dir = self.0.join("sys/class/net").join(name);
            fs::create_dir_all(&dir).expect("link");
            fs::write(dir.join("operstate"), format!("{state}\n")).expect("operstate");
            fs::write(dir.join("type"), format!("{kind}\n")).expect("type");
            let uevent = devtype.map_or(format!("INTERFACE={name}\n"), |t| {
                format!("DEVTYPE={t}\nINTERFACE={name}\n")
            });
            fs::write(dir.join("uevent"), uevent).expect("uevent");
            if wireless {
                fs::create_dir_all(dir.join("wireless")).expect("wireless");
            }
            self
        }
    }

    impl Drop for Machine {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const ETH_DEFAULT: &str = "eth0\t00000000\t0108A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";
    const WLAN_DEFAULT: &str = "wlan0\t00000000\t0108A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n";
    const LAN_ONLY: &str = "eth0\t0008A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";
    const V6_LO: &str = "00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200       lo\n";

    #[test]
    fn connected_is_a_default_route_up_through_an_interface_that_is_up() {
        let machine = Machine::new("default");
        machine
            .routes(&format!("{WLAN_DEFAULT}{ETH_DEFAULT}{LAN_ONLY}"), V6_LO)
            .link("eth0", "up", 1, None, false)
            .link("wlan0", "up", 1, Some("wlan"), true);
        // The lowest metric is the route used.
        assert_eq!(
            current(&machine.0),
            State {
                kind: Kind::Ethernet,
                interface: Some("eth0".into())
            }
        );
        // With the cable out, Wi-Fi carries it.
        machine.link("eth0", "down", 1, None, false);
        assert_eq!(
            current(&machine.0),
            State {
                kind: Kind::Wifi,
                interface: Some("wlan0".into())
            }
        );
    }

    #[test]
    fn a_network_with_no_default_route_is_not_a_connection() {
        let machine = Machine::new("none");
        machine
            .routes(LAN_ONLY, V6_LO)
            .link("eth0", "up", 1, None, false);
        let state = current(&machine.0);
        assert_eq!(
            state,
            State {
                kind: Kind::None,
                interface: None
            }
        );
        assert!(!state.connected());
        // No routing tables at all: none either.
        assert_eq!(current(Path::new("/nonexistent")).kind, Kind::None);
    }

    #[test]
    fn an_ipv6_default_route_is_a_connection_and_a_tunnel_is_a_vpn() {
        let machine = Machine::new("v6");
        let v6 = "00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 00000003 wg0\n";
        machine.routes("", &format!("{V6_LO}{v6}")).link(
            "wg0",
            "unknown",
            65534,
            Some("wireguard"),
            false,
        );
        assert_eq!(
            current(&machine.0),
            State {
                kind: Kind::Vpn,
                interface: Some("wg0".into())
            }
        );
        machine.link("wg0", "unknown", 65534, None, false);
        assert_eq!(current(&machine.0).kind, Kind::Vpn);
        machine.link("wg0", "up", 1, Some("wwan"), false);
        assert_eq!(current(&machine.0).kind, Kind::Cellular);
        machine.link("wg0", "up", 772, None, false);
        assert_eq!(current(&machine.0).kind, Kind::Other);
    }
}
