//! The session's side of Wi-Fi over a played daemon: who may ask, what is told to whom, and the
//! password sheet's path, the password reaching the daemon alone.

use super::*;
use oxyos_compositor_core::Grants;
use oxyos_networks::{Known, Network, Security, Wifi, WifiState};
use oxyos_protocol::{Event, Request};
use std::sync::Mutex;
use std::time::Instant;

const SHELL: ClientId = 1;
const SETTINGS: ClientId = 2;
const APP: ClientId = 3;
const PASSWORD: &str = "correct horse";

#[derive(Default)]
struct Machine {
    wifi: bool,
    joined: Option<String>,
    /// The saved networks and their passwords, as the daemon keeps them.
    saved: Vec<(String, Option<String>)>,
    /// Every password the daemon was given.
    given: Vec<String>,
    version: u64,
}

/// A daemon played in memory: Home (WPA, `PASSWORD`), Café (open), eduroam (enterprise).
#[derive(Clone)]
struct Played(Arc<Mutex<Machine>>);

fn nearby() -> [(&'static str, Security); 3] {
    [
        ("Home", Security::Wpa),
        ("Café", Security::Open),
        ("eduroam", Security::Enterprise),
    ]
}

impl Daemon for Played {
    fn state(&self) -> io::Result<State> {
        let m = self.0.lock().unwrap();
        let known: Vec<Known> = m
            .saved
            .iter()
            .map(|(ssid, _)| Known {
                id: format!("id-{ssid}"),
                ssid: ssid.clone(),
                auto_join: true,
            })
            .collect();
        let networks: Vec<Network> = if m.wifi {
            nearby()
                .iter()
                .map(|(ssid, security)| Network {
                    ssid: ssid.to_string(),
                    signal: 70,
                    security: *security,
                    known: m.saved.iter().any(|(s, _)| s == ssid),
                    current: m.joined.as_deref() == Some(*ssid),
                    supported: security.supported(),
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(State {
            available: true,
            wifi: Wifi {
                device: true,
                enabled: m.wifi,
                state: match (m.wifi, &m.joined) {
                    (false, _) => WifiState::Off,
                    (true, Some(_)) => WifiState::Connected,
                    (true, None) => WifiState::Disconnected,
                },
                current: networks.iter().find(|n| n.current).cloned(),
                networks,
                known,
                ..Wifi::default()
            },
            ..State::default()
        })
    }

    fn scan(&self) -> io::Result<()> {
        Ok(())
    }

    fn set_wifi(&self, enabled: bool) -> Result<(), Failure> {
        let mut m = self.0.lock().unwrap();
        m.wifi = enabled;
        if !enabled {
            m.joined = None;
        }
        m.version += 1;
        Ok(())
    }

    fn forget(&self, id: &str) -> Result<(), Failure> {
        let mut m = self.0.lock().unwrap();
        let before = m.saved.len();
        m.saved.retain(|(ssid, _)| format!("id-{ssid}") != id);
        m.version += 1;
        if m.saved.len() == before {
            Err(Failure::NotFound)
        } else {
            Ok(())
        }
    }

    fn plan(&self, ssid: &str) -> Result<Plan, Failure> {
        let state = self.state().unwrap();
        oxyos_networks::plan(&state.wifi.networks, ssid)
    }

    fn join(&self, ssid: &str, password: Option<&Secret>) -> Result<(), Failure> {
        let plan = self.plan(ssid)?;
        let mut m = self.0.lock().unwrap();
        if let Some(password) = password {
            m.given.push(password.expose().to_string());
        }
        let saved = m
            .saved
            .iter()
            .find(|(s, _)| s == ssid)
            .and_then(|(_, p)| p.clone());
        let accepted = match plan {
            Plan::Open => true,
            Plan::Known { security } if !security.needs_password() => true,
            Plan::Known { .. } | Plan::Password { .. } => {
                password
                    .map(|p| p.expose().to_string())
                    .or(saved)
                    .as_deref()
                    == Some(PASSWORD)
            }
            Plan::Unsupported { .. } => return Err(Failure::Unsupported),
        };
        if !accepted {
            return Err(Failure::WrongPassword);
        }
        m.saved.retain(|(s, _)| s != ssid);
        m.saved
            .push((ssid.into(), password.map(|p| p.expose().to_string())));
        m.joined = Some(ssid.into());
        m.version += 1;
        Ok(())
    }
}

impl Reach for Played {
    fn connect(&self) -> io::Result<Box<dyn Daemon>> {
        Ok(Box::new(self.clone()))
    }

    fn watch(&self, stop: &dyn Fn() -> bool, changed: &mut dyn FnMut() -> bool) -> io::Result<()> {
        let mut seen = None;
        loop {
            if stop() {
                return Ok(());
            }
            let version = self.0.lock().unwrap().version;
            if seen != Some(version) {
                seen = Some(version);
                if !changed() {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

struct Session {
    machine: Played,
    networks: Networks,
    scene: Scene,
    serial: u32,
    /// Everything each client was told, as text.
    told: BTreeMap<ClientId, Vec<String>>,
    /// Texts arriving in chunks, by client and what they belong to.
    partial: BTreeMap<(ClientId, String), String>,
}

impl Session {
    fn new(machine: Machine) -> Self {
        let machine = Played(Arc::new(Mutex::new(machine)));
        let mut scene = Scene::new();
        for (client, grants) in [
            (
                SHELL,
                Grants {
                    shell: true,
                    ..Grants::default()
                },
            ),
            (
                SETTINGS,
                Grants {
                    network: true,
                    ..Grants::default()
                },
            ),
            // An application that asks the session for something else (the store).
            (
                APP,
                Grants {
                    store: true,
                    ..Grants::default()
                },
            ),
        ] {
            scene.connect(client, grants).unwrap();
        }
        let networks = Networks::new(Arc::new(machine.clone())).unwrap();
        let mut session = Self {
            machine,
            networks,
            scene,
            serial: 0,
            told: BTreeMap::new(),
            partial: BTreeMap::new(),
        };
        session.until(|s| s.networks.state().is_some());
        session
    }

    fn turn(&mut self) {
        self.networks.serve(&mut self.scene);
        for (client, event) in self.scene.drain_events() {
            let (what, text, last) = match event {
                Event::SystemText {
                    serial, text, last, ..
                } => (format!("{serial}:"), text.as_str().to_string(), last),
                Event::SheetText {
                    sheet, text, last, ..
                } => (format!("sheet {sheet}:"), text.as_str().to_string(), last),
                Event::SheetDismissed { sheet } => {
                    (format!("dismissed {sheet}"), String::new(), true)
                }
                _ => continue,
            };
            let whole = self.partial.entry((client, what.clone())).or_default();
            whole.push_str(&text);
            if last {
                let whole = self.partial.remove(&(client, what.clone())).unwrap();
                self.told
                    .entry(client)
                    .or_default()
                    .push(format!("{what}{whole}"));
            }
        }
    }

    fn until(&mut self, mut condition: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.turn();
            if condition(self) {
                return;
            }
            assert!(Instant::now() < deadline, "not in time: {:?}", self.told);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// `client` asks `text`, citing an input or not; its serial.
    fn ask(&mut self, client: ClientId, text: &str, with_input: bool) -> u32 {
        self.serial += 1;
        self.networks.handle(
            &mut self.scene,
            SystemRequest {
                client,
                serial: self.serial,
                with_input,
                text: text.into(),
            },
        );
        self.serial
    }

    fn told(&self, client: ClientId) -> Vec<String> {
        self.told.get(&client).cloned().unwrap_or_default()
    }

    /// The answer `client` was given to its request `serial`.
    fn answer(&mut self, client: ClientId, serial: u32) -> Option<String> {
        self.turn();
        let prefix = format!("{serial}:");
        self.told(client)
            .iter()
            .find_map(|t| t.strip_prefix(&prefix).map(str::to_string))
    }

    fn wait_answer(&mut self, client: ClientId, serial: u32) -> String {
        self.until(|s| s.answer(client, serial).is_some());
        self.answer(client, serial).unwrap()
    }

    /// The last sheet the shell was sent: its number and text.
    fn sheet(&self) -> Option<(u32, Value)> {
        self.told(SHELL).iter().rev().find_map(|t| {
            let rest = t.strip_prefix("sheet ")?;
            let (number, text) = rest.split_once(':')?;
            Some((number.parse().ok()?, serde_json::from_str(text).ok()?))
        })
    }

    fn wait_sheet(&mut self, after: usize) -> (u32, Value) {
        self.until(|s| {
            s.told(SHELL)
                .iter()
                .filter(|t| t.starts_with("sheet "))
                .count()
                > after
        });
        self.sheet().unwrap()
    }

    fn sheets_sent(&self) -> usize {
        self.told(SHELL)
            .iter()
            .filter(|t| t.starts_with("sheet "))
            .count()
    }
}

fn machine() -> Machine {
    Machine {
        wifi: true,
        ..Machine::default()
    }
}

#[test]
fn the_vocabulary_is_this_parts_and_read_strictly() {
    assert!(ours(r#"{"join":{"ssid":"Home"}}"#));
    assert!(ours(r#"{"networks":{}}"#));
    assert!(!ours(r#"{"install":{"id":"x"}}"#), "the store's");
    assert!(!ours(r#"{"join":{},"scan":{}}"#));
    assert!(!ours("[]"));
    let ask = |text: &str| serde_json::from_str::<Ask>(text);
    assert!(ask(r#"{"join":{"ssid":"Home","password":"x"}}"#).is_err());
    assert!(ask(r#"{"wifi":{"enabled":"yes"}}"#).is_err());
    assert!(ask(r#"{"password":{"sheet":1}}"#).is_err());
}

#[test]
fn the_state_is_told_to_the_shell_and_the_network_grant_alone() {
    let mut s = Session::new(machine());
    let network = |told: Vec<String>| told.iter().any(|t| t.starts_with("0:{\"network\""));
    assert!(network(s.told(SHELL)) && network(s.told(SETTINGS)));
    assert!(!network(s.told(APP)));
    let asked = s.ask(SETTINGS, r#"{"networks":{}}"#, false);
    let state: Value = serde_json::from_str(&s.answer(SETTINGS, asked).unwrap()).unwrap();
    assert_eq!(state["network"]["wifi"]["networks"][0]["ssid"], "Home");
    let refused = s.ask(APP, r#"{"networks":{}}"#, false);
    assert_eq!(
        s.answer(APP, refused).unwrap(),
        r#"{"refused":{"reason":"notPermitted"}}"#
    );
    let unread = s.ask(SETTINGS, r#"{"join":{"name":"Home"}}"#, true);
    assert_eq!(
        s.answer(SETTINGS, unread).unwrap(),
        r#"{"refused":{"reason":"invalidRequest"}}"#
    );
}

#[test]
fn changes_need_an_input() {
    let mut s = Session::new(machine());
    for text in [
        r#"{"join":{"ssid":"Café"}}"#,
        r#"{"forget":{"id":"id-Home"}}"#,
        r#"{"wifi":{"enabled":false}}"#,
    ] {
        let asked = s.ask(SETTINGS, text, false);
        assert_eq!(
            s.answer(SETTINGS, asked).unwrap(),
            r#"{"refused":{"reason":"needsInput"}}"#,
            "{text}"
        );
    }
    assert!(s.machine.0.lock().unwrap().wifi);
}

#[test]
fn joining_an_open_network_tells_it_joining_then_joined() {
    let mut s = Session::new(machine());
    let asked = s.ask(SETTINGS, r#"{"join":{"ssid":"Café"}}"#, true);
    assert_eq!(
        s.wait_answer(SETTINGS, asked),
        r#"{"joined":{"ssid":"Café"}}"#
    );
    assert!(s
        .told(SHELL)
        .iter()
        .any(|t| t.contains(r#""joining":"Café""#)));
    s.until(|s| {
        s.told(SHELL)
            .last()
            .is_some_and(|t| t.contains(r#""joining":null"#) && t.contains(r#""current":{"#))
    });
    assert_eq!(s.sheets_sent(), 0, "nothing asked");
}

#[test]
fn a_protected_network_asks_the_shell_and_the_password_reaches_the_daemon_alone() {
    let mut s = Session::new(machine());
    let asked = s.ask(SETTINGS, r#"{"join":{"ssid":"Home"}}"#, true);
    let (sheet, text) = s.wait_sheet(0);
    assert_eq!(
        text,
        json!({"kind": "wifi-password", "ssid": "Home", "security": "wpa", "incorrect": false})
    );
    // Nothing connects while the person is asked: not told joining meanwhile.
    s.until(|s| {
        s.told(SETTINGS)
            .iter()
            .rev()
            .find(|t| t.starts_with("0:"))
            .is_some_and(|t| t.contains(r#""joining":null"#))
    });
    // Only the shell answers a password sheet, citing an input.
    let password = |sheet: u32, password: &str| {
        json!({"password": {"sheet": sheet, "password": password}}).to_string()
    };
    let from_settings = s.ask(SETTINGS, &password(sheet, PASSWORD), true);
    assert_eq!(
        s.answer(SETTINGS, from_settings).unwrap(),
        r#"{"refused":{"reason":"notPermitted"}}"#
    );
    let bare = s.ask(SHELL, &password(sheet, PASSWORD), false);
    assert_eq!(
        s.answer(SHELL, bare).unwrap(),
        r#"{"refused":{"reason":"needsInput"}}"#
    );
    let other = s.ask(SHELL, &password(sheet + 7, PASSWORD), true);
    assert_eq!(
        s.answer(SHELL, other).unwrap(),
        r#"{"refused":{"reason":"noSuchSheet"}}"#
    );
    assert!(s.machine.0.lock().unwrap().given.is_empty());
    s.ask(SHELL, &password(sheet, PASSWORD), true);
    assert_eq!(
        s.wait_answer(SETTINGS, asked),
        r#"{"joined":{"ssid":"Home"}}"#
    );
    let m = s.machine.0.lock().unwrap();
    assert_eq!(m.given, [PASSWORD]);
    assert_eq!(m.saved, [("Home".to_string(), Some(PASSWORD.to_string()))]);
    drop(m);
    assert!(s.told(SHELL).contains(&format!("dismissed {sheet}")));
    // No one but the daemon was told it: not Settings, not the application, not the shell back.
    for client in [SHELL, SETTINGS, APP] {
        assert!(
            !s.told(client).iter().any(|t| t.contains(PASSWORD)),
            "{client} was told the password"
        );
    }
}

#[test]
fn a_wrong_password_asks_again_saying_so_and_cancelling_ends_the_join() {
    let mut s = Session::new(machine());
    let asked = s.ask(SETTINGS, r#"{"join":{"ssid":"Home"}}"#, true);
    let (sheet, _) = s.wait_sheet(0);
    s.ask(
        SHELL,
        &json!({"password": {"sheet": sheet, "password": "wrong password"}}).to_string(),
        true,
    );
    let (again, text) = s.wait_sheet(1);
    assert_ne!(again, sheet);
    assert_eq!(text["incorrect"], true);
    assert_eq!(s.machine.0.lock().unwrap().given, ["wrong password"]);
    // Too short for WPA: asked again without bothering the daemon.
    s.ask(
        SHELL,
        &json!({"password": {"sheet": again, "password": "short"}}).to_string(),
        true,
    );
    let (third, text) = s.wait_sheet(2);
    assert_eq!(text["incorrect"], true);
    assert_eq!(s.machine.0.lock().unwrap().given.len(), 1);
    // Cancelled (a refusal needs no input).
    s.scene
        .handle(
            SHELL,
            Request::SheetAnswer {
                sheet: third,
                accepted: false,
                serial: 0,
            },
        )
        .unwrap();
    assert_eq!(
        s.wait_answer(SETTINGS, asked),
        r#"{"failed":{"reason":"cancelled"}}"#
    );
    assert!(s.machine.0.lock().unwrap().saved.is_empty(), "nothing kept");
    // The join over, another may begin.
    let next = s.ask(SETTINGS, r#"{"join":{"ssid":"Café"}}"#, true);
    assert_eq!(
        s.wait_answer(SETTINGS, next),
        r#"{"joined":{"ssid":"Café"}}"#
    );
}

#[test]
fn one_join_at_a_time_and_none_while_locked() {
    let mut s = Session::new(machine());
    let first = s.ask(SETTINGS, r#"{"join":{"ssid":"Home"}}"#, true);
    let (sheet, _) = s.wait_sheet(0);
    let second = s.ask(SHELL, r#"{"join":{"ssid":"Café"}}"#, true);
    assert_eq!(
        s.answer(SHELL, second).unwrap(),
        r#"{"failed":{"reason":"busy"}}"#
    );
    s.scene.lock();
    let locked = s.ask(
        SHELL,
        &json!({"password": {"sheet": sheet, "password": PASSWORD}}).to_string(),
        true,
    );
    assert_eq!(
        s.answer(SHELL, locked).unwrap(),
        r#"{"refused":{"reason":"locked"}}"#
    );
    assert!(s.machine.0.lock().unwrap().given.is_empty());
    // The asker going withdraws its sheet and ends its join.
    s.scene.disconnect(SETTINGS);
    s.until(|s| s.told(SHELL).contains(&format!("dismissed {sheet}")));
    s.until(|s| s.networks.joining.is_none());
    let _ = first;
}

#[test]
fn a_network_not_supported_says_so_in_a_sheet() {
    let mut s = Session::new(machine());
    let asked = s.ask(SHELL, r#"{"join":{"ssid":"eduroam"}}"#, true);
    assert_eq!(
        s.wait_answer(SHELL, asked),
        r#"{"failed":{"reason":"unsupported"}}"#
    );
    let (_, text) = s.sheet().unwrap();
    assert_eq!(
        text,
        json!({"kind": "wifi-unsupported", "ssid": "eduroam", "security": "enterprise"})
    );
    let missing = s.ask(SHELL, r#"{"join":{"ssid":"Nowhere"}}"#, true);
    assert_eq!(
        s.wait_answer(SHELL, missing),
        r#"{"failed":{"reason":"notFound"}}"#
    );
}

#[test]
fn forgetting_and_wifi_off_are_told() {
    let mut s = Session::new(Machine {
        wifi: true,
        joined: Some("Home".into()),
        saved: vec![("Home".into(), Some(PASSWORD.into()))],
        ..Machine::default()
    });
    let forget = s.ask(SETTINGS, r#"{"forget":{"id":"id-Home"}}"#, true);
    assert_eq!(s.wait_answer(SETTINGS, forget), r#"{"forgotten":{}}"#);
    s.until(|s| s.told(SETTINGS).iter().any(|t| t.contains(r#""known":[]"#)));
    let again = s.ask(SETTINGS, r#"{"forget":{"id":"id-Home"}}"#, true);
    assert_eq!(
        s.wait_answer(SETTINGS, again),
        r#"{"failed":{"reason":"notFound"}}"#
    );
    let off = s.ask(SHELL, r#"{"wifi":{"enabled":false}}"#, true);
    let answer: Value = serde_json::from_str(&s.wait_answer(SHELL, off)).unwrap();
    assert_eq!(answer["network"]["wifi"]["state"], "off");
    assert!(!s.machine.0.lock().unwrap().wifi);
    assert!(s.networks.take_changed());
}
