use super::*;

fn point(ssid: &str, signal: u8, security: Security) -> AccessPoint {
    AccessPoint {
        ssid: ssid.into(),
        signal,
        security,
    }
}

fn known(ssid: &str) -> Known {
    Known {
        id: format!("uuid-{ssid}"),
        ssid: ssid.into(),
        auto_join: true,
        private_address: PrivateAddress::Fixed,
    }
}

#[test]
fn security_from_the_access_points_flags() {
    assert_eq!(security(0, 0, 0), Security::Open);
    assert_eq!(security(1, 0, 0), Security::Wep);
    // WPA2 Personal (CCMP pairwise and group, PSK).
    assert_eq!(security(1, 0, 0x188), Security::Wpa);
    // WPA3 transition (PSK and SAE) joins as WPA2; WPA3 alone is SAE.
    assert_eq!(security(1, 0, 0x588), Security::Wpa);
    assert_eq!(security(1, 0, 0x488), Security::Wpa3);
    assert_eq!(security(1, 0x288, 0x288), Security::Enterprise);
    assert_eq!(security(1, 0, 0x2000), Security::Enterprise);
    assert_eq!(security(1, 0, 0x888), Security::Owe);
}

#[test]
fn what_is_supported_and_needs_a_password() {
    assert!(Security::Wpa.needs_password() && Security::Wpa3.needs_password());
    assert!(!Security::Open.needs_password() && !Security::Owe.needs_password());
    assert!(!Security::Enterprise.supported() && !Security::Wep.supported());
    assert_eq!(Security::Wpa3.key_mgmt(), Some("sae"));
    assert_eq!(Security::Wpa.key_mgmt(), Some("wpa-psk"));
    assert_eq!(Security::Open.key_mgmt(), None);
}

#[test]
fn networks_one_per_name_strongest_first_hidden_left_out() {
    let points = [
        point("Café", 40, Security::Open),
        point("Home", 50, Security::Wpa),
        point("", 99, Security::Wpa),
        point("Home", 80, Security::Wpa),
        point("Office", 80, Security::Enterprise),
    ];
    let networks = networks(&points, &[known("Home")], Some("Home"));
    let names: Vec<&str> = networks.iter().map(|n| n.ssid.as_str()).collect();
    assert_eq!(names, ["Home", "Office", "Café"]);
    assert_eq!(networks[0].signal, 80);
    assert!(networks[0].known && networks[0].current);
    assert!(!networks[1].supported && !networks[1].known);
    assert!(!networks[2].current);
}

#[test]
fn a_plan_for_each_kind_of_network() {
    let networks = networks(
        &[
            point("Home", 80, Security::Wpa),
            point("Café", 40, Security::Open),
            point("Lab", 60, Security::Wpa3),
            point("Office", 70, Security::Enterprise),
            point("Old", 30, Security::Wep),
        ],
        &[known("Home")],
        None,
    );
    assert_eq!(
        plan(&networks, "Home"),
        Ok(Plan::Known {
            security: Security::Wpa
        })
    );
    assert_eq!(plan(&networks, "Café"), Ok(Plan::Open));
    assert_eq!(
        plan(&networks, "Lab"),
        Ok(Plan::Password {
            security: Security::Wpa3
        })
    );
    assert_eq!(
        plan(&networks, "Office"),
        Ok(Plan::Unsupported {
            security: Security::Enterprise
        })
    );
    assert!(matches!(
        plan(&networks, "Old"),
        Ok(Plan::Unsupported { .. })
    ));
    assert_eq!(plan(&networks, "Elsewhere"), Err(Failure::NotFound));
}

#[test]
fn states_from_the_daemons() {
    assert_eq!(wifi_state(true, true, None), WifiState::Unavailable);
    assert_eq!(wifi_state(false, true, Some(30)), WifiState::Off);
    assert_eq!(wifi_state(true, false, Some(100)), WifiState::Off);
    assert_eq!(wifi_state(true, true, Some(100)), WifiState::Connected);
    assert_eq!(wifi_state(true, true, Some(60)), WifiState::Connecting);
    assert_eq!(wifi_state(true, true, Some(30)), WifiState::Disconnected);
    assert_eq!(ethernet_state(20, false), EthernetState::Unplugged);
    assert_eq!(ethernet_state(100, true), EthernetState::Connected);
    assert_eq!(ethernet_state(70, true), EthernetState::Connecting);
    assert_eq!(ethernet_state(30, true), EthernetState::Disconnected);
    assert_eq!(ethernet_state(10, true), EthernetState::Unavailable);
    assert!(secrets_reason(7) && secrets_reason(8) && secrets_reason(11) && !secrets_reason(5));
}

#[test]
fn masks_and_names() {
    assert_eq!(subnet_mask(24), "255.255.255.0");
    assert_eq!(subnet_mask(20), "255.255.240.0");
    assert_eq!(subnet_mask(0), "0.0.0.0");
    assert_eq!(subnet_mask(32), "255.255.255.255");
    assert_eq!(ssid_text(b"caf\xc3\xa9"), "café");
    assert_eq!(ssid_text(b"bad\xff"), "bad\u{fffd}");
}

#[test]
fn a_secret_says_nothing_and_checks_wpa_lengths() {
    let secret = Secret::new("hunter22".into());
    assert_eq!(format!("{secret:?}"), "Secret(…)");
    assert!(secret.valid_wpa());
    assert!(!Secret::new("short".into()).valid_wpa());
    assert!(Secret::new("a".repeat(63)).valid_wpa());
    // Bytes of UTF-8, as WPA and the sheet count them: four characters, eight bytes.
    assert!(Secret::new("ññññ".into()).valid_wpa());
    assert!(!Secret::new("ñ".repeat(32)).valid_wpa());
    assert!(!Secret::new("g".repeat(64)).valid_wpa());
    assert!(Secret::new("0123456789abcdef".repeat(4)).valid_wpa());
    let mut text = String::from("hunter22");
    clear(&mut text);
    assert!(text.is_empty());
}

#[test]
fn the_state_as_the_shell_and_settings_read_it() {
    let state = State {
        available: true,
        wifi: Wifi {
            device: true,
            enabled: true,
            state: WifiState::Connected,
            known: vec![known("Home")],
            ..Wifi::default()
        },
        ethernet: Ethernet {
            device: true,
            state: EthernetState::Unplugged,
            ..Ethernet::default()
        },
        joining: None,
    };
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(json["wifi"]["state"], "connected");
    assert_eq!(json["wifi"]["hardwareBlocked"], false);
    assert_eq!(json["wifi"]["known"][0]["autoJoin"], true);
    assert_eq!(json["ethernet"]["state"], "unplugged");
    assert!(json["joining"].is_null());
}

#[test]
fn a_bus_is_waited_for_by_its_socket() {
    use std::path::PathBuf;
    assert_eq!(
        bus_socket(Some("unix:path=/run/played/bus,guid=0123")),
        Some(PathBuf::from("/run/played/bus"))
    );
    assert_eq!(
        bus_socket(Some("tcp:host=localhost,port=1;unix:path=/run/b")),
        Some(PathBuf::from("/run/b"))
    );
    assert_eq!(bus_socket(Some("unix:abstract=/tmp/dbus-x")), None);
}

#[test]
fn a_signal_is_shown_as_the_wifi_glyphs_arcs() {
    // As the shell's top bar and Settings draw it (`signalBars`).
    let shown: Vec<u8> = [0, 1, 33, 34, 66, 67, 100].map(arcs).to_vec();
    assert_eq!(shown, [0, 1, 1, 2, 2, 3, 3]);
}

#[test]
fn a_private_address_from_the_connections_settings_and_back() {
    use PrivateAddress::*;
    // The daemon's default (no setting, `preserve`) and the hardware's own address are off.
    assert_eq!(PrivateAddress::of(None, None), Off);
    assert_eq!(PrivateAddress::of(Some(""), None), Off);
    assert_eq!(PrivateAddress::of(Some("preserve"), None), Off);
    assert_eq!(
        PrivateAddress::of(Some("permanent"), Some(ROTATING_STABLE_ID)),
        Off
    );
    // `stable` is fixed unless its stable-id changes with the boot or every activation.
    assert_eq!(PrivateAddress::of(Some("stable"), None), Fixed);
    assert_eq!(
        PrivateAddress::of(Some("stable"), Some("${CONNECTION}")),
        Fixed
    );
    assert_eq!(
        PrivateAddress::of(Some("stable-ssid"), Some("${BOOT}")),
        Fixed
    );
    assert_eq!(
        PrivateAddress::of(Some("stable"), Some(ROTATING_STABLE_ID)),
        Rotating
    );
    assert_eq!(
        PrivateAddress::of(Some("stable"), Some("x-${RANDOM}")),
        Rotating
    );
    assert_eq!(PrivateAddress::of(Some("random"), None), Rotating);
    // A fixed hardware address someone set is none of the three.
    assert_eq!(PrivateAddress::of(Some("02:11:22:33:44:55"), None), Other);
    // Each of the three set reads back as itself; another is never set from here.
    for mode in [Fixed, Rotating, Off] {
        let (assigned, stable_id) = mode.settings().unwrap();
        assert_eq!(PrivateAddress::of(Some(assigned), stable_id), mode);
    }
    assert_eq!(Other.settings(), None);
    assert_eq!(
        Fixed.settings(),
        Some(("stable", None)),
        "the daemon's own per-connection id"
    );
    assert_eq!(serde_json::to_value(Rotating).unwrap(), "rotating");
    assert_eq!(
        PrivateAddress::default(),
        Fixed,
        "a new network's, as Apple's"
    );
}

#[test]
fn a_band_and_channel_from_a_frequency() {
    use crate::diagnostics::band_and_channel;
    assert_eq!(band_and_channel(2412), (Some("2.4"), Some(1)));
    assert_eq!(band_and_channel(2437), (Some("2.4"), Some(6)));
    assert_eq!(band_and_channel(2484), (Some("2.4"), Some(14)));
    assert_eq!(band_and_channel(5180), (Some("5"), Some(36)));
    assert_eq!(band_and_channel(5825), (Some("5"), Some(165)));
    assert_eq!(band_and_channel(5935), (Some("6"), Some(2)));
    assert_eq!(band_and_channel(5955), (Some("6"), Some(1)));
    assert_eq!(band_and_channel(6115), (Some("6"), Some(33)));
    assert_eq!(band_and_channel(60480), (None, None));
}

#[test]
fn the_neighbour_table_says_whether_the_router_answered_arp() {
    use crate::diagnostics::neighbour_in;
    let table = "IP address       HW type     Flags       HW address            Mask     Device\n\
        192.168.8.1      0x1         0x2         a0:b1:c2:d3:e4:f5     *        wlp0s20f3\n\
        192.168.8.7      0x1         0x0         00:00:00:00:00:00     *        wlp0s20f3\n";
    assert!(neighbour_in(table, "192.168.8.1".parse().unwrap()));
    assert!(
        !neighbour_in(table, "192.168.8.7".parse().unwrap()),
        "incomplete"
    );
    assert!(!neighbour_in(table, "10.0.0.1".parse().unwrap()));
}

/// A probe that answers as told, counting what it was asked.
struct Played {
    ping: Ping,
    neighbour: bool,
    resolved: Resolved,
    asked: std::sync::Mutex<Vec<String>>,
}

impl Probe for Played {
    fn ping(&self, address: std::net::Ipv4Addr, count: u32, _: std::time::Duration) -> Ping {
        self.asked
            .lock()
            .unwrap()
            .push(format!("ping {address} {count}"));
        self.ping.clone()
    }
    fn neighbour(&self, address: std::net::Ipv4Addr) -> bool {
        self.asked
            .lock()
            .unwrap()
            .push(format!("neighbour {address}"));
        self.neighbour
    }
    fn resolve(&self, name: &str, _: std::time::Duration) -> Resolved {
        self.asked.lock().unwrap().push(format!("resolve {name}"));
        self.resolved.clone()
    }
}

fn survey(joined: bool) -> Survey {
    Survey {
        link: joined.then(|| Link {
            ssid: "Home".into(),
            interface: "wlp0s20f3".into(),
            bssid: "A0:B1:C2:00:00:01".into(),
            signal: 73,
            quality: LinkQuality::of(73),
            frequency: 5180,
            band: Some("5"),
            channel: Some(36),
            width: Some(80),
            tx_rate: Some(866),
            max_rate: Some(1201),
            security: Security::Wpa,
            address: "02:11:22:33:44:55".into(),
            permanent_address: "F4:A4:75:12:34:56".into(),
            private_address: PrivateAddress::Fixed,
        }),
        connectivity: Connectivity::of(4, true),
        router: Some("192.168.8.1".parse().unwrap()),
        dns: vec!["192.168.8.1".into()],
    }
}

#[test]
fn diagnostics_say_what_was_measured_and_nothing_else() {
    use std::time::Duration;
    let ms = Duration::from_millis;
    // The router answering two echoes of three: reached, the median of those answered.
    let probe = Played {
        ping: Ping::Sent {
            sent: 3,
            answered: vec![ms(3), ms(1), ms(2)],
        },
        neighbour: true,
        resolved: Resolved::Addresses {
            count: 2,
            took: ms(14),
        },
        asked: Default::default(),
    };
    let found = diagnose(&survey(true), &probe, "oxy.so");
    assert_eq!(found.router.verdict, RouterVerdict::Reached);
    assert_eq!(
        (found.router.sent, found.router.answered, found.router.ms),
        (3, 3, Some(2.0))
    );
    assert_eq!(found.dns.verdict, DnsVerdict::Resolved);
    assert_eq!((found.dns.ms, found.dns.addresses), (Some(14), 2));
    assert_eq!(found.dns.servers, ["192.168.8.1"]);
    // Asked once each; the neighbour table only when no echo came back.
    assert_eq!(
        *probe.asked.lock().unwrap(),
        ["ping 192.168.8.1 3", "resolve oxy.so"]
    );
    // Silent to echoes but there by ARP; then not there at all.
    for (neighbour, verdict) in [
        (true, RouterVerdict::Present),
        (false, RouterVerdict::Unreachable),
    ] {
        let probe = Played {
            ping: Ping::Sent {
                sent: 3,
                answered: Vec::new(),
            },
            neighbour,
            resolved: Resolved::TimedOut,
            asked: Default::default(),
        };
        let found = diagnose(&survey(true), &probe, "oxy.so");
        assert_eq!(found.router.verdict, verdict);
        assert_eq!((found.router.answered, found.router.ms), (0, None));
        assert_eq!(
            (found.dns.verdict, found.dns.ms),
            (DnsVerdict::TimedOut, None)
        );
    }
    // A machine that lets no user ping says so, nothing made up.
    let probe = Played {
        ping: Ping::NotAllowed,
        neighbour: true,
        resolved: Resolved::Failed { took: ms(30) },
        asked: Default::default(),
    };
    let found = diagnose(&survey(true), &probe, "oxy.so");
    assert_eq!(
        (found.router.verdict, found.router.sent),
        (RouterVerdict::NotAllowed, 0)
    );
    assert_eq!(
        (found.dns.verdict, found.dns.ms),
        (DnsVerdict::Failed, Some(30))
    );
    // Wi-Fi not joined: nothing is checked, nothing asked.
    let found = diagnose(&survey(false), &probe, "oxy.so");
    assert_eq!(found.router.verdict, RouterVerdict::NotChecked);
    assert_eq!(found.dns.verdict, DnsVerdict::NotChecked);
    assert_eq!(probe.asked.lock().unwrap().len(), 2);
    // The daemon's check alone says whether the internet answers.
    assert_eq!(
        Connectivity::of(4, false),
        Connectivity {
            checked: false,
            state: "unknown"
        }
    );
    assert_eq!(Connectivity::of(2, true).state, "portal");
}
