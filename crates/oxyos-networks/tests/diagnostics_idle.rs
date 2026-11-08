//! Wi-Fi Diagnostics run when asked and leave nothing behind (docs/adr/0038-networks.mdx,
//! 8 November 2026): the machine's own probe ([`oxyos_networks::SystemProbe`]) echoes a router
//! (here the loopback address, which answers as a router does) through an unprivileged ICMP
//! socket and resolves a name through the system's resolver (`localhost`, which needs no
//! network), says only what it measured, and once it returned no thread of it is left and nothing
//! of the process wakes: no timer, no polling, until it is asked again.
//!
//! One test in a binary of its own: it counts every thread of the process.

use oxyos_networks::{
    diagnose, Connectivity, DnsVerdict, Link, LinkQuality, PrivateAddress, RouterVerdict, Security,
    Survey, SystemProbe,
};
use std::collections::BTreeMap;
use std::time::Duration;

/// Every thread's context switches but the calling one's, by thread id.
fn threads() -> BTreeMap<String, u64> {
    let me = std::fs::read_link("/proc/thread-self")
        .ok()
        .and_then(|link| {
            link.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap();
    let mut switches = BTreeMap::new();
    for task in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
        let id = task.file_name().to_string_lossy().into_owned();
        if id == me {
            continue;
        }
        let Ok(status) = std::fs::read_to_string(task.path().join("status")) else {
            continue;
        };
        let count: u64 = status
            .lines()
            .filter(|line| line.contains("ctxt_switches"))
            .filter_map(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
            .sum();
        switches.insert(id, count);
    }
    switches
}

/// Whether this machine lets this process's group send echoes without privileges
/// (`net.ipv4.ping_group_range`, which systemd opens to every group).
fn echoes_allowed() -> bool {
    let range = std::fs::read_to_string("/proc/sys/net/ipv4/ping_group_range").unwrap();
    let mut bounds = range.split_whitespace().map(|n| n.parse::<u64>().unwrap());
    let (low, high) = (bounds.next().unwrap(), bounds.next().unwrap());
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let gid: u64 = status
        .lines()
        .find_map(|line| line.strip_prefix("Gid:"))
        .and_then(|ids| ids.split_whitespace().nth(1))
        .unwrap()
        .parse()
        .unwrap();
    (low..=high).contains(&gid)
}

#[test]
fn diagnostics_say_what_they_measured_and_leave_nothing_running() {
    let survey = Survey {
        link: Some(Link {
            ssid: "Home".into(),
            interface: "lo".into(),
            bssid: "A0:B1:C2:D3:E4:F5".into(),
            signal: 70,
            quality: LinkQuality::of(70),
            frequency: 2437,
            band: Some("2.4"),
            channel: Some(6),
            width: None,
            tx_rate: None,
            max_rate: None,
            security: Security::Wpa,
            address: "02:00:00:00:00:01".into(),
            permanent_address: "F4:A4:75:12:34:56".into(),
            private_address: PrivateAddress::Fixed,
        }),
        connectivity: Connectivity::of(0, false),
        router: Some("127.0.0.1".parse().unwrap()),
        dns: vec!["127.0.0.53".into()],
    };
    let before = threads();
    let found = diagnose(&survey, &SystemProbe, "localhost");
    if echoes_allowed() {
        assert_eq!(found.router.verdict, RouterVerdict::Reached);
        assert_eq!((found.router.sent, found.router.answered), (3, 3));
        assert!(found.router.ms.is_some_and(|ms| ms < 1000.0));
    } else {
        // Said as it is, nothing made up.
        assert_eq!(found.router.verdict, RouterVerdict::NotAllowed);
        assert_eq!((found.router.sent, found.router.ms), (0, None));
    }
    assert_eq!(found.dns.verdict, DnsVerdict::Resolved);
    assert!(found.dns.addresses >= 1 && found.dns.ms.is_some());
    assert!(!found.connectivity.checked);

    // Returned: no thread of it left, and nothing wakes.
    let after = threads();
    let left: Vec<&String> = after
        .keys()
        .filter(|id| !before.contains_key(*id))
        .collect();
    assert!(left.is_empty(), "threads left running: {left:?}");
    std::thread::sleep(Duration::from_secs(2));
    let later = threads();
    let woke: Vec<(&String, u64)> = later
        .iter()
        .filter_map(|(id, count)| {
            let woke = count - after.get(id).copied().unwrap_or(*count);
            (woke > 0).then_some((id, woke))
        })
        .collect();
    assert!(woke.is_empty(), "woke after diagnostics returned: {woke:?}");
}
