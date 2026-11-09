//! Wi-Fi Diagnostics (docs/adr/0038-networks.mdx, 8 November 2026): what the link to the network
//! joined is, as NetworkManager has it ([`Survey`], read by `NetworkManager::survey`), then, once,
//! whether the router answers and whether the system's resolver resolves a name, each with a plain
//! verdict and only numbers measured. Nothing here runs unless asked: no timer, no thread left.
//!
//! The router is reached the way `ping` reaches it on Debian without privileges: an ICMP datagram
//! socket (`IPPROTO_ICMP`, which the kernel allows to the groups of `net.ipv4.ping_group_range`,
//! every group on a machine with systemd's defaults), echo requests the kernel numbers itself. A
//! router that does not answer echoes but answered the kernel's ARP for its address is there. The
//! name is resolved by `getaddrinfo`, what every application's lookups go through.

use crate::{PrivateAddress, Security};
use serde::Serialize;
use std::net::{Ipv4Addr, ToSocketAddrs};
use std::time::{Duration, Instant};

/// The name the resolver is asked for: OxyOS's own site.
pub const DIAGNOSTIC_NAME: &str = "oxy.so";
/// Echo requests sent to the router, each waited for at most [`ECHO_WAIT`].
const ECHOES: u32 = 3;
const ECHO_WAIT: Duration = Duration::from_secs(1);
/// The longest a name is waited for: glibc's resolver tries each server 5 s, twice.
const RESOLVE_LIMIT: Duration = Duration::from_secs(10);

/// The link to the network joined, as the daemon has it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub ssid: String,
    pub interface: String,
    /// The access point's hardware address.
    pub bssid: String,
    /// 0 to 100, the daemon's.
    pub signal: u8,
    pub quality: LinkQuality,
    /// MHz.
    pub frequency: u32,
    /// "2.4", "5" or "6" GHz; none for a frequency outside them.
    pub band: Option<&'static str>,
    pub channel: Option<u32>,
    /// The channel's width in MHz, when the daemon says it.
    pub width: Option<u32>,
    /// The rate the card sends at now, Mb/s, when the daemon says it.
    pub tx_rate: Option<u32>,
    /// The fastest the access point offers, Mb/s.
    pub max_rate: Option<u32>,
    pub security: Security,
    /// The address the computer uses on it, and its hardware's own.
    pub address: String,
    pub permanent_address: String,
    pub private_address: PrivateAddress,
}

/// A signal's strength in words: as many arcs as the Wi-Fi glyph fills ([`crate::arcs`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkQuality {
    Good,
    Fair,
    Poor,
}

impl LinkQuality {
    pub fn of(signal: u8) -> Self {
        match crate::arcs(signal) {
            3 => LinkQuality::Good,
            2 => LinkQuality::Fair,
            _ => LinkQuality::Poor,
        }
    }
}

/// Whether the internet answers, as NetworkManager's connectivity check says: only when one is
/// configured (Debian's package configures none; `network-manager-config-connectivity-debian`
/// does) and enabled; else not checked, whatever the daemon's state assumes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connectivity {
    pub checked: bool,
    /// `full`, `limited`, `portal`, `none` or `unknown`.
    pub state: &'static str,
}

impl Connectivity {
    /// From the daemon's `NMConnectivityState`, and whether its check is configured and enabled.
    pub fn of(state: u32, checked: bool) -> Self {
        Connectivity {
            checked,
            state: if !checked {
                "unknown"
            } else {
                match state {
                    1 => "none",
                    2 => "portal",
                    3 => "limited",
                    4 => "full",
                    _ => "unknown",
                }
            },
        }
    }
}

/// What the daemon says, read as diagnostics begin.
#[derive(Clone, Debug, PartialEq)]
pub struct Survey {
    /// None: Wi-Fi is not joined.
    pub link: Option<Link>,
    pub connectivity: Connectivity,
    /// The network joined's IPv4 router.
    pub router: Option<Ipv4Addr>,
    /// The DNS servers the daemon configured, in its order.
    pub dns: Vec<String>,
}

/// What the router did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RouterVerdict {
    /// It answered echoes.
    Reached,
    /// It answered none, but answered for its address (ARP): it is there and ignores pings.
    Present,
    /// Nothing answered.
    Unreachable,
    /// This machine does not let a user send echoes (`net.ipv4.ping_group_range`).
    NotAllowed,
    /// No router to check (no IPv4 router, or Wi-Fi not joined).
    NotChecked,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterCheck {
    pub address: Option<String>,
    pub verdict: RouterVerdict,
    pub sent: u32,
    pub answered: u32,
    /// The median round trip of those answered, ms (a tenth).
    pub ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DnsVerdict {
    Resolved,
    Failed,
    /// No answer within ten seconds.
    TimedOut,
    NotChecked,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsCheck {
    pub servers: Vec<String>,
    pub name: String,
    pub verdict: DnsVerdict,
    /// How long the resolver took, ms.
    pub ms: Option<u64>,
    /// How many addresses it gave.
    pub addresses: usize,
}

/// Wi-Fi Diagnostics as the session tells them.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub link: Option<Link>,
    pub connectivity: Connectivity,
    pub router: RouterCheck,
    pub dns: DnsCheck,
}

/// Echoes sent, or why none could be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ping {
    /// The round trips of the echoes answered, of `sent`.
    Sent {
        sent: u32,
        answered: Vec<Duration>,
    },
    /// The machine does not let this user send echoes.
    NotAllowed,
    Failed,
}

/// What the resolver did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Addresses { count: usize, took: Duration },
    Failed { took: Duration },
    TimedOut,
}

/// What diagnostics ask of the machine beyond the daemon: the system's own ([`SystemProbe`]), or a
/// played network's in the development compositor.
pub trait Probe: Send + Sync {
    /// `count` echo requests to `address`, each waited for at most `wait`.
    fn ping(&self, address: Ipv4Addr, count: u32, wait: Duration) -> Ping;
    /// Whether the kernel's neighbour table has `address`'s hardware address: it answered ARP.
    fn neighbour(&self, address: Ipv4Addr) -> bool;
    /// `name` through the system's resolver, waited for at most `limit`.
    fn resolve(&self, name: &str, limit: Duration) -> Resolved;
}

/// The machine's own: an unprivileged ICMP socket, `/proc/net/arp`, `getaddrinfo`.
pub struct SystemProbe;

impl Probe for SystemProbe {
    fn ping(&self, address: Ipv4Addr, count: u32, wait: Duration) -> Ping {
        use rustix::net::{AddressFamily, SocketType};
        let socket = match rustix::net::socket(
            AddressFamily::INET,
            SocketType::DGRAM,
            Some(rustix::net::ipproto::ICMP),
        ) {
            Ok(socket) => socket,
            Err(rustix::io::Errno::ACCESS | rustix::io::Errno::PERM) => return Ping::NotAllowed,
            Err(_) => return Ping::Failed,
        };
        if rustix::net::connect(&socket, &std::net::SocketAddrV4::new(address, 0)).is_err() {
            return Ping::Failed;
        }
        let mut answered = Vec::new();
        for sequence in 1..=count as u16 {
            // Type 8 (echo request), code 0; the kernel sets the identifier and the checksum.
            let mut request = [0u8; 16];
            request[0] = 8;
            request[6..8].copy_from_slice(&sequence.to_be_bytes());
            request[8..].copy_from_slice(b"oxyos-wi");
            let sent = Instant::now();
            if rustix::net::send(&socket, &request, rustix::net::SendFlags::empty()).is_err() {
                return Ping::Failed;
            }
            let deadline = sent + wait;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let timeout = rustix::event::Timespec::try_from(left).unwrap_or_default();
                let mut readable = [rustix::event::PollFd::new(
                    &socket,
                    rustix::event::PollFlags::IN,
                )];
                match rustix::event::poll(&mut readable, Some(&timeout)) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(_) => return Ping::Failed,
                }
                let mut reply = [0u8; 64];
                let Ok((length, _)) =
                    rustix::net::recv(&socket, &mut reply[..], rustix::net::RecvFlags::DONTWAIT)
                else {
                    continue;
                };
                // An echo reply (type 0) to this request.
                if length >= 8 && reply[0] == 0 && reply[6..8] == sequence.to_be_bytes() {
                    answered.push(sent.elapsed());
                    break;
                }
            }
        }
        Ping::Sent {
            sent: count,
            answered,
        }
    }

    fn neighbour(&self, address: Ipv4Addr) -> bool {
        let Ok(table) = std::fs::read_to_string("/proc/net/arp") else {
            return false;
        };
        neighbour_in(&table, address)
    }

    fn resolve(&self, name: &str, limit: Duration) -> Resolved {
        let (done, outcome) = std::sync::mpsc::channel();
        let name = name.to_string();
        let started = Instant::now();
        // getaddrinfo has no deadline of its own: it runs beside, and a late answer goes nowhere.
        let asked = std::thread::Builder::new()
            .name("networks-resolve".into())
            .spawn(move || {
                let addresses = (name.as_str(), 0).to_socket_addrs().map(Iterator::count);
                let _ = done.send(addresses);
            });
        let Ok(asked) = asked else {
            return Resolved::Failed {
                took: started.elapsed(),
            };
        };
        let answer = outcome.recv_timeout(limit);
        let took = started.elapsed();
        if answer.is_ok() {
            // It answered: its thread ends now, nothing of it left behind.
            let _ = asked.join();
        }
        match answer {
            Ok(Ok(count)) if count > 0 => Resolved::Addresses { count, took },
            Ok(_) => Resolved::Failed { took },
            Err(_) => Resolved::TimedOut,
        }
    }
}

/// Whether `/proc/net/arp`'s `table` has a complete entry (`ATF_COM`) for `address`.
pub(crate) fn neighbour_in(table: &str, address: Ipv4Addr) -> bool {
    const COMPLETE: u32 = 0x2;
    table.lines().skip(1).any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        fields.len() >= 4
            && fields[0].parse::<Ipv4Addr>().ok() == Some(address)
            && u32::from_str_radix(fields[2].trim_start_matches("0x"), 16)
                .is_ok_and(|flags| flags & COMPLETE != 0)
            && fields[3] != "00:00:00:00:00:00"
    })
}

/// The median of `times`, in ms to a tenth.
fn median_ms(times: &[Duration]) -> Option<f64> {
    let mut times = times.to_vec();
    times.sort();
    let middle = times.get(times.len() / 2)?;
    Some((middle.as_secs_f64() * 10_000.0).round() / 10.0)
}

/// Diagnostics of what `survey` says, the router and the resolver checked once through `probe`
/// (the resolver asked for `name`). Without a Wi-Fi network joined, nothing is checked.
pub fn diagnose(survey: &Survey, probe: &dyn Probe, name: &str) -> Diagnostics {
    let joined = survey.link.is_some();
    let router = match survey.router.filter(|_| joined) {
        None => RouterCheck {
            address: None,
            verdict: RouterVerdict::NotChecked,
            sent: 0,
            answered: 0,
            ms: None,
        },
        Some(address) => {
            let (verdict, sent, answered) = match probe.ping(address, ECHOES, ECHO_WAIT) {
                Ping::Sent { sent, answered } if !answered.is_empty() => {
                    (RouterVerdict::Reached, sent, answered)
                }
                Ping::Sent { sent, answered } => {
                    let verdict = if probe.neighbour(address) {
                        RouterVerdict::Present
                    } else {
                        RouterVerdict::Unreachable
                    };
                    (verdict, sent, answered)
                }
                Ping::NotAllowed => (RouterVerdict::NotAllowed, 0, Vec::new()),
                Ping::Failed => (RouterVerdict::Unreachable, 0, Vec::new()),
            };
            RouterCheck {
                address: Some(address.to_string()),
                verdict,
                sent,
                answered: answered.len() as u32,
                ms: median_ms(&answered),
            }
        }
    };
    let dns = if joined {
        let (verdict, ms, addresses) = match probe.resolve(name, RESOLVE_LIMIT) {
            Resolved::Addresses { count, took } => {
                (DnsVerdict::Resolved, Some(took.as_millis() as u64), count)
            }
            Resolved::Failed { took } => (DnsVerdict::Failed, Some(took.as_millis() as u64), 0),
            Resolved::TimedOut => (DnsVerdict::TimedOut, None, 0),
        };
        DnsCheck {
            servers: survey.dns.clone(),
            name: name.to_string(),
            verdict,
            ms,
            addresses,
        }
    } else {
        DnsCheck {
            servers: survey.dns.clone(),
            name: name.to_string(),
            verdict: DnsVerdict::NotChecked,
            ms: None,
            addresses: 0,
        }
    };
    Diagnostics {
        link: survey.link.clone(),
        connectivity: survey.connectivity.clone(),
        router,
        dns,
    }
}

/// A frequency's band and channel (IEEE 802.11's numbering): 2.4 GHz from 2412 MHz (channel 1,
/// 2484 MHz channel 14), 5 GHz from 5000 MHz, 6 GHz from 5950 MHz (5935 MHz is its channel 2).
pub fn band_and_channel(frequency: u32) -> (Option<&'static str>, Option<u32>) {
    match frequency {
        2484 => (Some("2.4"), Some(14)),
        2412..=2472 => (Some("2.4"), Some((frequency - 2407) / 5)),
        5935 => (Some("6"), Some(2)),
        5955..=7115 => (Some("6"), Some((frequency - 5950) / 5)),
        5160..=5925 => (Some("5"), Some((frequency - 5000) / 5)),
        _ => (None, None),
    }
}
