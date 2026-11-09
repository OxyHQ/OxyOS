//! `oxyos-networks` against a played NetworkManager (`oxyos-fake-networkmanager`) on a private
//! bus of the test's own (`dbus-daemon`), never the system's: what the client reads, and what the
//! daemon holds after each call.

use oxyos_networks::{
    EthernetState, Failure, KeptBecause, LinkQuality, NetworkManager, Plan, PrivateAddress, Rejoin,
    SavedKind, Secret, Security, WifiState, ROTATING_STABLE_ID,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon {
    bus: Child,
    fake: Child,
    address: String,
    state: PathBuf,
    _directory: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.fake.kill();
        let _ = self.bus.kill();
        let _ = self.fake.wait();
        let _ = self.bus.wait();
        let _ = std::fs::remove_dir_all(&self._directory);
    }
}

fn directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("oxyos-networks-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

/// A bus of the test's own and the played daemon on it, the machine `machine` describes.
fn daemon(name: &str, machine: Value) -> Daemon {
    let directory = directory(name);
    let mut bus = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .arg(format!(
            "--address=unix:path={}",
            directory.join("bus").display()
        ))
        .stdout(Stdio::piped())
        .spawn()
        .expect("dbus-daemon (the dbus-daemon package)");
    let mut address = String::new();
    BufReader::new(bus.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let address = address.trim().to_string();
    let machine_file = directory.join("machine.json");
    std::fs::write(&machine_file, machine.to_string()).unwrap();
    let state = directory.join("state.json");
    let fake = Command::new(env!("CARGO_BIN_EXE_oxyos-fake-networkmanager"))
        .args(["--address", &address, "--machine"])
        .arg(&machine_file)
        .arg("--state")
        .arg(&state)
        .spawn()
        .unwrap();
    let daemon = Daemon {
        bus,
        fake,
        address,
        state,
        _directory: directory,
    };
    let manager = NetworkManager::connect(Some(&daemon.address)).unwrap();
    until(|| manager.state().unwrap().available);
    daemon
}

fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "not in time");
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl Daemon {
    fn manager(&self) -> NetworkManager {
        NetworkManager::connect(Some(&self.address)).unwrap()
    }

    /// What the played daemon holds.
    fn held(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(&self.state).unwrap()).unwrap()
    }

    fn act(&self, what: Value) {
        let status = Command::new("busctl")
            .arg(format!("--address={}", self.address))
            .args([
                "call",
                "org.freedesktop.NetworkManager",
                "/so/oxy/OxyOS/FakeNetworkManager",
                "so.oxy.OxyOS.FakeNetworkManager",
                "Act",
                "s",
            ])
            .arg(what.to_string())
            .status()
            .unwrap();
        assert!(status.success());
    }
}

fn machine() -> Value {
    json!({
        "access_points": [
            {"ssid": "LukeSkyRouter", "strength": 73, "security": "wpa", "password": "correct horse"},
            {"ssid": "LukeSkyRouter", "strength": 40, "security": "wpa", "password": "correct horse"},
            {"ssid": "Café Libre", "strength": 55},
            {"ssid": "Vecinos", "strength": 30, "security": "wpa", "password": "hunter22!"},
            {"ssid": "eduroam", "strength": 60, "security": "enterprise"},
            {"ssid": "", "strength": 90, "security": "wpa"}
        ],
        "connections": [{"ssid": "LukeSkyRouter", "password": "correct horse"}],
        "active": "LukeSkyRouter",
        "ethernet": "unplugged"
    })
}

#[test]
fn the_state_as_the_daemon_has_it() {
    let daemon = daemon("state", machine());
    let state = daemon.manager().state().unwrap();
    assert!(state.available);
    assert_eq!(state.wifi.state, WifiState::Connected);
    let names: Vec<&str> = state
        .wifi
        .networks
        .iter()
        .map(|n| n.ssid.as_str())
        .collect();
    assert_eq!(names, ["LukeSkyRouter", "eduroam", "Café Libre", "Vecinos"]);
    let current = state.wifi.current.as_ref().unwrap();
    assert_eq!(
        (current.ssid.as_str(), current.signal),
        ("LukeSkyRouter", 73)
    );
    assert!(current.known && current.security == Security::Wpa);
    assert!(!state.wifi.networks[1].supported);
    assert_eq!(state.wifi.known.len(), 1);
    let details = state.wifi.details.unwrap();
    assert_eq!(details.interface, "wlp0s20f3");
    let ipv4 = details.ipv4.unwrap();
    assert_eq!(
        (ipv4.address.as_str(), ipv4.subnet_mask.as_str()),
        ("192.168.8.133", "255.255.255.0")
    );
    assert_eq!(details.router.as_deref(), Some("192.168.8.1"));
    assert_eq!(details.dns, ["192.168.8.1"]);
    assert_eq!(details.ipv6, ["2001:db8::133"], "link-local left out");
    assert_eq!(state.ethernet.state, EthernetState::Unplugged);
    assert!(state.ethernet.details.is_none());
}

#[test]
fn joining_an_open_network() {
    let daemon = daemon("open", machine());
    let manager = daemon.manager();
    assert_eq!(manager.plan("Café Libre"), Ok(Plan::Open));
    manager.join("Café Libre", None).unwrap();
    let state = manager.state().unwrap();
    assert_eq!(state.wifi.current.unwrap().ssid, "Café Libre");
    let held = daemon.held();
    assert_eq!(held["active"], "Café Libre");
    let added = &held["connections"][1];
    assert_eq!(
        (added["ssid"].as_str(), added["psk"].is_null()),
        (Some("Café Libre"), true)
    );
    assert!(
        added["permissions"].as_array().unwrap().is_empty(),
        "for every user"
    );
}

#[test]
fn a_protected_network_takes_its_password_and_keeps_no_wrong_one() {
    let daemon = daemon("protected", machine());
    let manager = daemon.manager();
    assert_eq!(
        manager.plan("Vecinos"),
        Ok(Plan::Password {
            security: Security::Wpa
        })
    );
    assert_eq!(manager.join("Vecinos", None), Err(Failure::NeedsPassword));
    let wrong = Secret::new("hunter23!".into());
    assert_eq!(
        manager.join("Vecinos", Some(&wrong)),
        Err(Failure::WrongPassword)
    );
    let held = daemon.held();
    assert_eq!(
        held["connections"].as_array().unwrap().len(),
        1,
        "the wrong one deleted: {held}"
    );
    assert_eq!(held["attempts"][0]["psk"], "hunter23!");
    let right = Secret::new("hunter22!".into());
    manager.join("Vecinos", Some(&right)).unwrap();
    let held = daemon.held();
    assert_eq!(held["active"], "Vecinos");
    let added = &held["connections"][1];
    assert_eq!(
        (added["psk"].as_str(), added["key_mgmt"].as_str()),
        (Some("hunter22!"), Some("wpa-psk"))
    );
    assert!(manager
        .state()
        .unwrap()
        .wifi
        .known
        .iter()
        .any(|k| k.ssid == "Vecinos"));
}

#[test]
fn a_known_network_joins_as_saved_and_takes_a_new_password() {
    let mut spec = machine();
    spec["active"] = Value::Null;
    spec["connections"] = json!([{"ssid": "LukeSkyRouter", "password": "old password"}]);
    let daemon = daemon("known", spec);
    let manager = daemon.manager();
    assert!(matches!(
        manager.plan("LukeSkyRouter"),
        Ok(Plan::Known { .. })
    ));
    assert_eq!(
        manager.join("LukeSkyRouter", None),
        Err(Failure::WrongPassword)
    );
    assert_eq!(
        daemon.held()["connections"].as_array().unwrap().len(),
        1,
        "kept"
    );
    // A mistyped replacement does not take the saved password's place.
    assert_eq!(
        manager.join("LukeSkyRouter", Some(&Secret::new("mistyped123".into()))),
        Err(Failure::WrongPassword)
    );
    assert_eq!(daemon.held()["connections"][0]["psk"], "old password");
    manager
        .join("LukeSkyRouter", Some(&Secret::new("correct horse".into())))
        .unwrap();
    let held = daemon.held();
    assert_eq!(
        held["connections"].as_array().unwrap().len(),
        1,
        "updated, not added"
    );
    assert_eq!(held["connections"][0]["psk"], "correct horse");
    assert_eq!(held["active"], "LukeSkyRouter");
}

#[test]
fn unsupported_and_missing_networks_are_not_tried() {
    let daemon = daemon("unsupported", machine());
    let manager = daemon.manager();
    assert_eq!(manager.join("eduroam", None), Err(Failure::Unsupported));
    assert_eq!(manager.join("Nowhere", None), Err(Failure::NotFound));
    assert!(daemon.held()["attempts"].as_array().unwrap().is_empty());
}

#[test]
fn refused_for_every_user_it_is_added_for_this_user() {
    let mut spec = machine();
    spec["deny_system"] = json!(true);
    let daemon = daemon("own", spec);
    let manager = daemon.manager();
    manager.join("Café Libre", None).unwrap();
    let held = daemon.held();
    let permissions = held["connections"][1]["permissions"][0].as_str().unwrap();
    assert!(
        permissions.starts_with("user:") && permissions.ends_with(':'),
        "{permissions}"
    );
    // Forgetting a connection every user has needs an administrator.
    let lukes = manager
        .state()
        .unwrap()
        .wifi
        .known
        .into_iter()
        .find(|k| k.ssid == "LukeSkyRouter")
        .unwrap();
    assert_eq!(manager.forget(&lukes.id), Err(Failure::NotPermitted));
}

#[test]
fn forgetting_a_network() {
    let daemon = daemon("forget", machine());
    let manager = daemon.manager();
    let id = manager.state().unwrap().wifi.known[0].id.clone();
    manager.forget(&id).unwrap();
    until(|| manager.state().unwrap().wifi.known.is_empty());
    assert!(daemon.held()["connections"].as_array().unwrap().is_empty());
    assert_eq!(manager.forget(&id), Err(Failure::NotFound));
}

#[test]
fn wifi_off_and_on() {
    let daemon = daemon("radio", machine());
    let manager = daemon.manager();
    manager.set_wifi(false).unwrap();
    until(|| manager.state().unwrap().wifi.state == WifiState::Off);
    let state = manager.state().unwrap();
    assert!(state.wifi.networks.is_empty() && state.wifi.current.is_none());
    assert_eq!(manager.join("Café Libre", None), Err(Failure::Unavailable));
    manager.set_wifi(true).unwrap();
    until(|| manager.state().unwrap().wifi.state == WifiState::Disconnected);
    assert_eq!(daemon.held()["wifi_enabled"], true);
}

#[test]
fn scans_and_watching_the_daemon() {
    let daemon = daemon("watch", machine());
    let manager = daemon.manager();
    // A scan returns once it ended (its `LastScan`), not at its limit.
    let asked = Instant::now();
    manager.scan().unwrap();
    assert!(
        asked.elapsed() < Duration::from_secs(5),
        "{:?}",
        asked.elapsed()
    );
    assert_eq!(daemon.held()["scans"], 1);
    let (told, seen) = std::sync::mpsc::channel();
    let address = daemon.address.clone();
    // Never told to stop: it ends with the test.
    let (never, _kept) = std::os::unix::net::UnixStream::pair().unwrap();
    std::thread::spawn(move || {
        use std::os::fd::AsFd;
        oxyos_networks::watch(
            Some(&address),
            Duration::from_millis(30),
            never.as_fd(),
            |changes: &oxyos_networks::Changes| {
                // The access point joined, as a read says it: its strength is followed.
                let point = NetworkManager::connect(Some(&address))
                    .and_then(|manager| manager.state())
                    .ok()
                    .and_then(|state| state.wifi.point);
                if told.send(changes.clone()).is_err() {
                    return oxyos_networks::Follow::Stop;
                }
                oxyos_networks::Follow::Point(point)
            },
        )
    });
    assert_eq!(
        seen.recv_timeout(Duration::from_secs(5))
            .expect("told at start"),
        oxyos_networks::Changes::Any
    );
    // The access point joined: its strength is told as that alone, by its object. Another's, and
    // the joined one seen again by a scan, are not delivered at all: had they been, they would
    // have been told before it, in the order they were signalled.
    let before = manager.state().unwrap();
    daemon.act(json!({"strength": {"ssid": "Café Libre", "strength": 90}}));
    daemon.act(json!({"seen": "LukeSkyRouter"}));
    daemon.act(json!({"strength": {"ssid": "LukeSkyRouter", "strength": 50}}));
    let told = seen
        .recv_timeout(Duration::from_secs(5))
        .expect("told of the strength");
    let oxyos_networks::Changes::Strengths(strengths) = told else {
        panic!("{told:?} is not the strength alone");
    };
    assert_eq!(strengths, [(before.wifi.point.clone().unwrap(), 50)]);
    let mut applied = before.clone();
    assert!(applied.with_strengths(&strengths));
    let read = manager.state().unwrap();
    assert_eq!(
        applied.wifi.current, read.wifi.current,
        "applied as a read gives it"
    );
    assert_eq!(read.wifi.current.unwrap().signal, 50);
    daemon.act(json!({"ethernet": "plugged"}));
    assert_eq!(
        seen.recv_timeout(Duration::from_secs(5))
            .expect("told of the cable"),
        oxyos_networks::Changes::Any
    );
    until(|| manager.state().unwrap().ethernet.state == EthernetState::Connected);
    let ethernet = manager.state().unwrap().ethernet;
    assert_eq!(ethernet.speed, 1000);
    assert_eq!(ethernet.details.unwrap().ipv4.unwrap().address, "10.0.0.20");
    daemon.act(json!({"ethernet": "unplugged"}));
    until(|| manager.state().unwrap().ethernet.state == EthernetState::Unplugged);
    daemon.act(json!({"access_point": {"ssid": "Biblioteca", "strength": 95}}));
    until(|| {
        manager
            .state()
            .unwrap()
            .wifi
            .networks
            .first()
            .is_some_and(|n| n.ssid == "Biblioteca")
    });
}

#[test]
fn no_daemon_is_unavailable() {
    let directory = directory("absent");
    let mut bus = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .arg(format!(
            "--address=unix:path={}",
            directory.join("bus").display()
        ))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut address = String::new();
    BufReader::new(bus.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let manager = NetworkManager::connect(Some(address.trim())).unwrap();
    assert!(!manager.state().unwrap().available);
    let _ = bus.kill();
    let _ = bus.wait();
    let _ = std::fs::remove_dir_all(directory);
}

/// The played Wi-Fi device's own address.
const PERMANENT: &str = "F4:A4:75:12:34:56";

/// Whether an address is one of the daemon's making: locally administered, unicast.
fn private(address: &str) -> bool {
    let first = u8::from_str_radix(&address[..2], 16).unwrap();
    first & 0x02 != 0 && first & 0x01 == 0
}

fn joined_address(manager: &NetworkManager) -> String {
    manager
        .state()
        .unwrap()
        .wifi
        .details
        .unwrap()
        .hardware_address
}

#[test]
fn a_new_network_joins_with_a_fixed_private_address_and_each_mode_is_the_daemons() {
    let daemon = daemon("private", machine());
    let manager = daemon.manager();
    // A network joined before has none set: the hardware's address, the daemon's default.
    let lukes = manager.state().unwrap().wifi.known[0].clone();
    assert_eq!(lukes.private_address, PrivateAddress::Off);
    assert_eq!(joined_address(&manager), PERMANENT);
    // A new one is joined with a fixed private address of the daemon's making, the address shown.
    manager.join("Café Libre", None).unwrap();
    let held = daemon.held();
    let added = &held["connections"][1];
    assert_eq!(
        (added["assigned"].as_str(), added["stable_id"].is_null()),
        (Some("stable"), true)
    );
    let fixed = held["address"].as_str().unwrap().to_string();
    assert!(private(&fixed) && fixed != PERMANENT, "{fixed}");
    assert_eq!(joined_address(&manager), fixed);
    let cafe = manager
        .state()
        .unwrap()
        .wifi
        .known
        .into_iter()
        .find(|k| k.ssid == "Café Libre")
        .unwrap();
    assert_eq!(cafe.private_address, PrivateAddress::Fixed);
    // Rotating, on the network joined: set, and joined again with another address.
    let attempts = held["attempts"].as_array().unwrap().len();
    assert_eq!(
        manager.set_private_address(&cafe.id, PrivateAddress::Rotating),
        Ok(Rejoin::Rejoined)
    );
    let held = daemon.held();
    assert_eq!(
        held["attempts"].as_array().unwrap().len(),
        attempts + 1,
        "joined again"
    );
    assert_eq!(
        (
            held["connections"][1]["assigned"].as_str(),
            held["connections"][1]["stable_id"].as_str()
        ),
        (Some("stable"), Some(ROTATING_STABLE_ID))
    );
    let rotating = held["address"].as_str().unwrap().to_string();
    assert!(private(&rotating) && rotating != fixed);
    // The same address while the computer runs; another after it starts again.
    manager.set_wifi(false).unwrap();
    until(|| manager.state().unwrap().wifi.state == WifiState::Off);
    manager.set_wifi(true).unwrap();
    until(|| manager.state().unwrap().wifi.state == WifiState::Disconnected);
    manager.join("Café Libre", None).unwrap();
    assert_eq!(daemon.held()["address"], rotating.as_str());
    daemon.act(json!({"restart": {}}));
    until(|| daemon.held()["address"] == rotating.as_str());
    manager.set_wifi(false).unwrap();
    until(|| manager.state().unwrap().wifi.state == WifiState::Off);
    manager.set_wifi(true).unwrap();
    until(|| manager.state().unwrap().wifi.state == WifiState::Disconnected);
    manager.join("Café Libre", None).unwrap();
    let after_restart = daemon.held()["address"].as_str().unwrap().to_string();
    assert!(private(&after_restart) && after_restart != rotating);
    // Fixed again: the address it had, whatever the boot.
    assert_eq!(
        manager.set_private_address(&cafe.id, PrivateAddress::Fixed),
        Ok(Rejoin::Rejoined)
    );
    assert_eq!(daemon.held()["address"], fixed.as_str());
    assert!(
        daemon.held()["connections"][1]["stable_id"].is_null(),
        "the daemon's own id"
    );
    // Off: the hardware's own.
    assert_eq!(
        manager.set_private_address(&cafe.id, PrivateAddress::Off),
        Ok(Rejoin::Rejoined)
    );
    assert_eq!(daemon.held()["connections"][1]["assigned"], "permanent");
    assert_eq!(joined_address(&manager), PERMANENT);
    // Not the network joined: set, nothing joined again.
    let attempts = daemon.held()["attempts"].as_array().unwrap().len();
    assert_eq!(
        manager.set_private_address(&lukes.id, PrivateAddress::Rotating),
        Ok(Rejoin::NotJoined)
    );
    assert_eq!(
        daemon.held()["attempts"].as_array().unwrap().len(),
        attempts
    );
    assert_eq!(
        daemon.held()["connections"][0]["stable_id"],
        ROTATING_STABLE_ID
    );
    // Its password kept: an update that carries no secrets keeps them.
    assert_eq!(daemon.held()["connections"][0]["psk"], "correct horse");
    assert_eq!(
        manager.set_private_address("no-such-id", PrivateAddress::Off),
        Err(Failure::NotFound)
    );
    assert_eq!(
        manager.set_private_address(&lukes.id, PrivateAddress::Other),
        Err(Failure::Unsupported)
    );
}

#[test]
fn a_private_address_for_every_user_needs_an_administrator() {
    let mut spec = machine();
    spec["deny_system"] = json!(true);
    let daemon = daemon("private-refused", spec);
    let manager = daemon.manager();
    let lukes = manager.state().unwrap().wifi.known[0].clone();
    assert_eq!(
        manager.set_private_address(&lukes.id, PrivateAddress::Fixed),
        Err(Failure::NotPermitted)
    );
    assert!(
        daemon.held()["connections"][0]["assigned"].is_null(),
        "unchanged"
    );
}

#[test]
fn forgetting_the_network_joined_leaves_it_and_its_password_goes() {
    let daemon = daemon("forget-joined", machine());
    let manager = daemon.manager();
    let id = manager.state().unwrap().wifi.known[0].id.clone();
    manager.forget(&id).unwrap();
    until(|| manager.state().unwrap().wifi.state == WifiState::Disconnected);
    let held = daemon.held();
    assert!(held["connections"].as_array().unwrap().is_empty());
    assert!(held["active"].is_null() && held["joined_with"].is_null());
    assert!(
        !held.to_string().contains("correct horse"),
        "its password gone: {held}"
    );
}

fn saved_machine(deny_system: bool) -> Value {
    let mut spec = machine();
    spec["deny_system"] = json!(deny_system);
    spec["connections"] = json!([
        {"ssid": "LukeSkyRouter", "password": "correct horse"},
        {"ssid": "Vecinos", "password": "hunter22!", "permissions": ["user:someone:"]},
        {"type": "vpn", "name": "Work VPN"},
        {"type": "wireguard", "name": "wg-home"},
        {"type": "ethernet", "name": "Office static"},
        {"type": "ethernet", "name": "Wired connection 1", "flags": 2}
    ]);
    spec
}

#[test]
fn resetting_removes_what_the_person_may_and_says_what_it_keeps() {
    let daemon = daemon("reset", saved_machine(false));
    let manager = daemon.manager();
    let plan = manager.reset_plan().unwrap();
    let names = |saved: &[oxyos_networks::Saved]| -> Vec<(SavedKind, String)> {
        saved.iter().map(|s| (s.kind, s.name.clone())).collect()
    };
    assert_eq!(
        names(&plan.removed),
        [
            (SavedKind::Wifi, "LukeSkyRouter".into()),
            (SavedKind::Wifi, "Vecinos".into()),
            (SavedKind::Vpn, "Work VPN".into()),
            (SavedKind::Vpn, "wg-home".into()),
        ]
    );
    // Ethernet's configuration someone made is kept; the daemon's own is not even counted.
    assert_eq!(plan.kept.len(), 1);
    assert_eq!(
        (plan.kept[0].saved.name.as_str(), plan.kept[0].because),
        ("Office static", KeptBecause::Ethernet)
    );
    // Asking changes nothing.
    assert_eq!(daemon.held()["connections"].as_array().unwrap().len(), 6);
    let done = manager.reset().unwrap();
    assert_eq!(done.removed, plan.removed);
    assert_eq!(done.kept, plan.kept);
    let held = daemon.held();
    let left: Vec<&str> = held["connections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(left, ["Office static", "Wired connection 1"]);
    assert!(!held.to_string().contains("correct horse") && !held.to_string().contains("hunter22!"));
    until(|| manager.state().unwrap().wifi.state == WifiState::Disconnected);
    assert!(manager.state().unwrap().wifi.known.is_empty());
}

#[test]
fn resetting_keeps_what_needs_an_administrator() {
    let daemon = daemon("reset-refused", saved_machine(true));
    let manager = daemon.manager();
    let plan = manager.reset_plan().unwrap();
    // The person's own network goes; those for every user need an administrator.
    assert_eq!(plan.removed.len(), 1);
    assert_eq!(plan.removed[0].name, "Vecinos");
    let kept: Vec<(&str, KeptBecause)> = plan
        .kept
        .iter()
        .map(|k| (k.saved.name.as_str(), k.because))
        .collect();
    assert_eq!(
        kept,
        [
            ("LukeSkyRouter", KeptBecause::NotPermitted),
            ("Work VPN", KeptBecause::NotPermitted),
            ("wg-home", KeptBecause::NotPermitted),
            ("Office static", KeptBecause::Ethernet),
        ]
    );
    let done = manager.reset().unwrap();
    assert_eq!(done.removed.len(), 1);
    assert_eq!(daemon.held()["connections"].as_array().unwrap().len(), 5);
}

#[test]
fn diagnostics_read_the_link_as_the_daemon_has_it() {
    let mut spec = machine();
    spec["access_points"][0] = json!({
        "ssid": "LukeSkyRouter", "strength": 73, "security": "wpa", "password": "correct horse",
        "frequency": 5180, "bssid": "A0:B1:C2:D3:E4:F5", "rate": 866, "max_rate": 1201, "width": 80
    });
    spec["connectivity"] = json!({"check": true, "state": 4});
    let joined = daemon("survey", spec);
    let manager = joined.manager();
    let survey = manager.survey().unwrap();
    let link = survey.link.unwrap();
    assert_eq!(
        (
            link.ssid.as_str(),
            link.bssid.as_str(),
            link.interface.as_str()
        ),
        ("LukeSkyRouter", "A0:B1:C2:D3:E4:F5", "wlp0s20f3")
    );
    assert_eq!((link.signal, link.quality), (73, LinkQuality::Good));
    assert_eq!(
        (link.frequency, link.band, link.channel, link.width),
        (5180, Some("5"), Some(36), Some(80))
    );
    assert_eq!((link.tx_rate, link.max_rate), (Some(866), Some(1201)));
    assert_eq!(link.security, Security::Wpa);
    assert_eq!(
        (link.address.as_str(), link.permanent_address.as_str()),
        (PERMANENT, PERMANENT)
    );
    assert_eq!(link.private_address, PrivateAddress::Off);
    assert_eq!(
        (survey.connectivity.checked, survey.connectivity.state),
        (true, "full")
    );
    assert_eq!(
        joined.held()["connectivity_checks"],
        1,
        "the daemon's check, run as asked"
    );
    assert_eq!(survey.router, Some("192.168.8.1".parse().unwrap()));
    assert_eq!(survey.dns, ["192.168.8.1"]);
    // No check configured: not checked, whatever the daemon assumes; nothing run.
    let mut spec = machine();
    spec["active"] = Value::Null;
    let quiet = daemon("survey-none", spec);
    let survey = quiet.manager().survey().unwrap();
    assert!(survey.link.is_none() && survey.router.is_none());
    assert_eq!(
        (survey.connectivity.checked, survey.connectivity.state),
        (false, "unknown")
    );
    assert_eq!(quiet.held()["connectivity_checks"], 0);
}

#[test]
fn a_network_that_admits_only_the_hardwares_address_is_set_but_not_joined_again() {
    let mut spec = machine();
    spec["access_points"][0]["admits"] = json!([PERMANENT]);
    spec["access_points"][1]["admits"] = json!([PERMANENT]);
    let daemon = daemon("private-filtered", spec);
    let manager = daemon.manager();
    let lukes = manager.state().unwrap().wifi.known[0].clone();
    // Fixed: set, and joining again with it fails, said as such; the setting stays.
    assert_eq!(
        manager.set_private_address(&lukes.id, PrivateAddress::Fixed),
        Ok(Rejoin::Failed(Failure::Failed))
    );
    assert_eq!(daemon.held()["connections"][0]["assigned"], "stable");
    until(|| manager.state().unwrap().wifi.state == WifiState::Disconnected);
    // Off joins again with the hardware's address, which it admits.
    assert_eq!(
        manager.set_private_address(&lukes.id, PrivateAddress::Off),
        Ok(Rejoin::NotJoined),
        "not joined now"
    );
    manager.join("LukeSkyRouter", None).unwrap();
    assert_eq!(daemon.held()["address"], PERMANENT);
}
