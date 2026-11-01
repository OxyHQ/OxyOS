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
