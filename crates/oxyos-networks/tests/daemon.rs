//! `oxyos-networks` against a played NetworkManager (`oxyos-fake-networkmanager`) on a private
//! bus of the test's own (`dbus-daemon`), never the system's: what the client reads, and what the
//! daemon holds after each call.

use oxyos_networks::{EthernetState, Failure, NetworkManager, Plan, Secret, Security, WifiState};
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
    manager.scan().unwrap();
    until(|| daemon.held()["scans"] == 1);
    let (told, seen) = std::sync::mpsc::channel();
    let address = daemon.address.clone();
    std::thread::spawn(move || {
        oxyos_networks::watch(Some(&address), Duration::from_millis(30), &|| false, || {
            told.send(()).is_ok()
        })
    });
    seen.recv_timeout(Duration::from_secs(5))
        .expect("told at start");
    daemon.act(json!({"ethernet": "plugged"}));
    seen.recv_timeout(Duration::from_secs(5))
        .expect("told of the cable");
    until(|| manager.state().unwrap().ethernet.state == EthernetState::Connected);
    let ethernet = manager.state().unwrap().ethernet;
    assert_eq!(ethernet.speed, 1000);
    assert_eq!(ethernet.details.unwrap().ipv4.unwrap().address, "10.0.0.20");
    daemon.act(json!({"ethernet": "unplugged"}));
    until(|| manager.state().unwrap().ethernet.state == EthernetState::Unplugged);
    daemon.act(json!({"access_point": {"ssid": "Biblioteca", "strength": 88}}));
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
