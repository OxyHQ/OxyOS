//! The session's side of backups, over a backup service played on a socket of the test's own (its
//! conversation as `oxyos_backup::ipc` writes it), and once over the real service: who may ask,
//! what is told to whom, what the service is asked and how its answers are passed on, Low Power
//! Mode's hold on the one connection, and the service followed again when it comes back.

use super::*;
use oxyos_compositor_core::Grants;
use oxyos_install::socket::Listener;
use oxyos_protocol::Event as Told;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Instant;

const SHELL: ClientId = 1;
const SETTINGS: ClientId = 2;
const APP: ClientId = 3;

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "oxyos-compositor-backups-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn state(destinations: Value, held: bool) -> Value {
    json!({
        "automatic": true,
        "destinations": destinations,
        "excluded": [],
        "running": null,
        "lastBackup": null,
        "nextBackup": null,
        "held": held,
        "administrator": true,
    })
}

fn disk() -> Value {
    json!([{
        "id": "d1", "name": "Backup Disk", "kind": "disk", "url": null, "available": true,
        "lastBackup": 1_700_000_000, "oldestBackup": 1_699_000_000, "backups": 3,
        "usedBytes": 1_200_000, "freeBytes": 245_000_000_000u64, "lastFailure": null
    }])
}

/// What the played service holds: every request each connection sent, in order, the holds, and
/// the connections that watch, which it tells its state.
#[derive(Default)]
struct Held {
    requests: Vec<Value>,
    holds: i32,
    watchers: Vec<Connection>,
    connections: Vec<Connection>,
    state: Value,
}

/// A backup service played on a socket: it answers as `oxyos-backupd` does.
struct Played {
    socket: PathBuf,
    held: Arc<Mutex<Held>>,
}

impl Played {
    fn serve(socket: &Path, state: Value) -> Self {
        let listener = Listener::bind(socket, 0o600).unwrap();
        let held = Arc::new(Mutex::new(Held {
            state,
            ..Held::default()
        }));
        let shared = held.clone();
        std::thread::spawn(move || {
            while let Ok(connection) = listener.accept() {
                let shared = shared.clone();
                shared
                    .lock()
                    .unwrap()
                    .connections
                    .push(connection.try_clone().unwrap());
                std::thread::spawn(move || converse(&connection, &shared));
            }
        });
        Self {
            socket: socket.to_owned(),
            held,
        }
    }

    /// Its state changed: every watcher told.
    fn tell(&self, state: Value) {
        let mut held = self.held.lock().unwrap();
        held.state = state.clone();
        for watcher in &held.watchers {
            let _ = watcher.send(&json!({ "status": state }));
        }
    }

    fn requests(&self) -> Vec<Value> {
        self.held.lock().unwrap().requests.clone()
    }

    fn holds(&self) -> i32 {
        self.held.lock().unwrap().holds
    }

    /// It goes: every connection closed (its holds with them) and its socket removed.
    fn end(self) {
        let mut held = self.held.lock().unwrap();
        for connection in held.connections.drain(..) {
            let _ = rustix::net::shutdown(connection.as_fd(), rustix::net::Shutdown::Both);
        }
        held.watchers.clear();
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn converse(connection: &Connection, shared: &Mutex<Held>) {
    let mut holding = false;
    while let Ok(Some(Some(request))) = connection.receive::<Value>() {
        let mut held = shared.lock().unwrap();
        held.requests.push(request.clone());
        let answer = if request == "watch" {
            held.watchers.push(connection.try_clone().unwrap());
            json!({ "status": held.state })
        } else if request == "status" {
            json!({ "status": held.state })
        } else if request.get("hold").is_some() {
            if !holding {
                holding = true;
                held.holds += 1;
            }
            json!("held")
        } else if request.get("release").is_some() {
            if holding {
                holding = false;
                held.holds -= 1;
            }
            json!("released")
        } else if request.get("backUpNow").is_some() {
            json!("started")
        } else if request.get("stop").is_some() {
            json!({ "refused": { "reason": "notRunning" } })
        } else if request.get("disks").is_some() {
            json!({ "disks": [{ "uuid": "4f2a-77c1", "label": "Archivo", "fsType": "ext4", "size": 0, "added": false }] })
        } else if let Some(add) = request.get("addDisk") {
            if add["password"]
                .as_str()
                .is_some_and(|p| p.chars().count() >= 8)
            {
                json!({ "added": { "id": "d2" } })
            } else {
                json!({ "refused": { "reason": "passwordTooShort" } })
            }
        } else if request.get("addShare").is_some() {
            json!({ "refused": { "reason": "shareRefused" } })
        } else if request.get("remove").is_some() {
            json!("removed")
        } else if request.get("setExcluded").is_some() || request.get("setAutomatic").is_some() {
            json!("set")
        } else {
            json!({ "refused": { "reason": "invalidRequest" } })
        };
        drop(held);
        if connection.send(&answer).is_err() {
            break;
        }
    }
    if holding {
        shared.lock().unwrap().holds -= 1;
    }
}

struct Session {
    scene: Scene,
    backups: Backups,
    low_power: LowPower,
    preferences: Preferences,
    told: BTreeMap<ClientId, Vec<(u32, String)>>,
    partial: BTreeMap<(ClientId, u32), String>,
    serial: u32,
}

impl Session {
    fn new(socket: &Path) -> Self {
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
                    settings: true,
                    ..Grants::default()
                },
            ),
            // An application of the platform's with another grant: told what it asks, nothing
            // of backups.
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
        let low_power = LowPower::default();
        let backups = Backups::new(socket.to_owned(), low_power.clone()).unwrap();
        Self {
            scene,
            backups,
            low_power,
            preferences: Preferences::default(),
            told: BTreeMap::new(),
            partial: BTreeMap::new(),
            serial: 0,
        }
    }

    fn turn(&mut self) {
        self.backups.serve(&mut self.scene, &self.preferences);
        for (client, event) in self.scene.drain_events() {
            if let Told::SystemText {
                serial, text, last, ..
            } = event
            {
                let whole = self.partial.entry((client, serial)).or_default();
                whole.push_str(text.as_str());
                if last {
                    let whole = self.partial.remove(&(client, serial)).unwrap();
                    self.told.entry(client).or_default().push((serial, whole));
                }
            }
        }
    }

    /// Turns until `done` holds, at most ten seconds.
    fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.turn();
            if done(self) {
                return;
            }
            assert!(Instant::now() < deadline, "not in ten seconds: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn ask(&mut self, client: ClientId, text: &str, with_input: bool) -> u32 {
        self.serial += 1;
        self.backups.handle(
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

    fn answer(&self, client: ClientId, serial: u32) -> Option<Value> {
        self.told
            .get(&client)?
            .iter()
            .find(|(told, _)| *told == serial)
            .map(|(_, text)| serde_json::from_str(text).unwrap())
    }

    fn answered(&mut self, client: ClientId, serial: u32) -> Value {
        self.until("answered", |s| s.answer(client, serial).is_some());
        self.answer(client, serial).unwrap()
    }

    /// The state last told `client` unasked; `Some(Value::Null)` for no service.
    fn state(&self, client: ClientId) -> Option<Value> {
        self.told
            .get(&client)?
            .iter()
            .rev()
            .find(|(serial, _)| *serial == 0)
            .map(|(_, text)| serde_json::from_str::<Value>(text).unwrap()["backup"].clone())
    }

    fn unasked(&self, client: ClientId) -> usize {
        self.told.get(&client).map_or(0, |told| {
            told.iter().filter(|(serial, _)| *serial == 0).count()
        })
    }
}

#[test]
fn the_shell_and_settings_are_told_the_state_as_it_changes_and_nobody_else() {
    let directory = scratch("told");
    let played = Played::serve(&directory.join("socket"), state(disk(), false));
    let mut session = Session::new(&played.socket);
    session.until("told the state", |s| s.state(SHELL).is_some());
    let told = session.state(SETTINGS).unwrap();
    assert_eq!(told["destinations"][0]["name"], "Backup Disk");
    // The person's preference beside the service's state, on by default.
    assert_eq!(told["inMenuBar"], true);
    assert_eq!(session.state(APP), None, "an application is told nothing");
    // Asked, the state as followed: the service is not asked again.
    let asked = session.ask(SHELL, r#"{"backup":{}}"#, false);
    assert_eq!(session.answered(SHELL, asked)["backup"], told);
    assert_eq!(played.requests(), vec![json!("watch")]);
    // A backup running, told as the service tells it.
    let mut running = state(disk(), false);
    running["running"] = json!({"destination": "d1", "phase": "backingUp", "asked": true,
        "percent": 0.25, "bytesDone": 25, "bytesTotal": 100, "filesDone": 1, "filesTotal": 4,
        "secondsRemaining": 3});
    played.tell(running);
    session.until("told it runs", |s| {
        s.state(SHELL).unwrap()["running"]["percent"] == 0.25
    });
    // The menu bar's item turned off: told again, once.
    let before = session.unasked(SHELL);
    session.preferences.backup_in_menu_bar = false;
    session.turn();
    session.turn();
    assert_eq!(session.unasked(SHELL), before + 1);
    assert_eq!(session.state(SHELL).unwrap()["inMenuBar"], false);
    // Nothing changed: nothing told.
    std::thread::sleep(Duration::from_millis(50));
    session.turn();
    assert_eq!(session.unasked(SHELL), before + 1);
    drop(session);
    played.end();
}

#[test]
fn who_may_ask_what_and_with_an_input() {
    let directory = scratch("asks");
    let played = Played::serve(&directory.join("socket"), state(disk(), false));
    let mut session = Session::new(&played.socket);
    session.until("told the state", |s| s.state(SETTINGS).is_some());
    let refused = |reason: &str| json!({"refused": {"reason": reason}});
    // An application asks nothing.
    let asked = session.ask(APP, r#"{"backup":{}}"#, true);
    assert_eq!(session.answered(APP, asked), refused("notPermitted"));
    // The shell backs up now, but configures nothing.
    let asked = session.ask(SHELL, r#"{"backupDisks":{}}"#, true);
    assert_eq!(session.answered(SHELL, asked), refused("notPermitted"));
    let asked = session.ask(SHELL, r#"{"backupAutomatic":{"enabled":false}}"#, true);
    assert_eq!(session.answered(SHELL, asked), refused("notPermitted"));
    // What changes anything cites an input.
    let asked = session.ask(SHELL, r#"{"backupNow":{}}"#, false);
    assert_eq!(session.answered(SHELL, asked), refused("needsInput"));
    let asked = session.ask(
        SETTINGS,
        r#"{"backupAddDisk":{"uuid":"4f2a-77c1","password":"correct horse"}}"#,
        false,
    );
    assert_eq!(session.answered(SETTINGS, asked), refused("needsInput"));
    // Read strictly: an unknown field, a missing one, a wrong type.
    for text in [
        r#"{"backupNow":{"now":true}}"#,
        r#"{"backupAddDisk":{"uuid":"4f2a-77c1"}}"#,
        r#"{"backupAutomatic":{"enabled":"yes"}}"#,
        r#"{"backupAddShare":{"share":{"host":"nas","share":"B","user":"u","path":"/"},"sharePassword":"s","password":"longenough"}}"#,
    ] {
        let asked = session.ask(SETTINGS, text, true);
        assert_eq!(
            session.answered(SETTINGS, asked),
            refused("invalidRequest"),
            "{text}"
        );
    }
    // None of it reached the service.
    assert_eq!(played.requests(), vec![json!("watch")]);
    drop(session);
    played.end();
}

#[test]
fn requests_are_the_services_and_its_refusals_are_failures() {
    let directory = scratch("requests");
    let played = Played::serve(&directory.join("socket"), state(disk(), false));
    let mut session = Session::new(&played.socket);
    session.until("told the state", |s| s.state(SETTINGS).is_some());
    let cases = [
        (SHELL, r#"{"backupNow":{}}"#, json!({"backupNow": {}})),
        (
            SETTINGS,
            r#"{"backupStop":{}}"#,
            json!({"failed": {"reason": "notRunning"}}),
        ),
        (
            SETTINGS,
            r#"{"backupDisks":{}}"#,
            json!({"backupDisks": [{"uuid": "4f2a-77c1", "label": "Archivo", "fsType": "ext4", "size": 0, "added": false}]}),
        ),
        (
            SETTINGS,
            r#"{"backupAddDisk":{"uuid":"4f2a-77c1","password":"correct horse"}}"#,
            json!({"backupAddDisk": {"id": "d2"}}),
        ),
        (
            SETTINGS,
            r#"{"backupAddDisk":{"uuid":"4f2a-77c1","password":"short"}}"#,
            json!({"failed": {"reason": "passwordTooShort"}}),
        ),
        (
            SETTINGS,
            r#"{"backupAddShare":{"share":{"host":"nas.local","share":"Backups","user":"nate"},"sharePassword":"s","password":"correct horse"}}"#,
            json!({"failed": {"reason": "shareRefused"}}),
        ),
        (
            SETTINGS,
            r#"{"backupRemove":{"id":"d1"}}"#,
            json!({"backupRemove": {}}),
        ),
        (
            SETTINGS,
            r#"{"backupExcluded":{"paths":["/home/nate/Movies"]}}"#,
            json!({"backupExcluded": {}}),
        ),
        (
            SETTINGS,
            r#"{"backupAutomatic":{"enabled":false}}"#,
            json!({"backupAutomatic": {}}),
        ),
    ];
    for (client, text, expected) in cases {
        let asked = session.ask(client, text, true);
        assert_eq!(session.answered(client, asked), expected, "{text}");
    }
    // Asked of the service in its own words, each on a connection of its own.
    let requests = played.requests();
    assert_eq!(
        &requests[1..],
        &[
            json!({"backUpNow": {}}),
            json!({"stop": {}}),
            json!({"disks": {}}),
            json!({"addDisk": {"uuid": "4f2a-77c1", "password": "correct horse"}}),
            json!({"addDisk": {"uuid": "4f2a-77c1", "password": "short"}}),
            json!({"addShare": {"share": {"host": "nas.local", "share": "Backups", "user": "nate"}, "sharePassword": "s", "password": "correct horse"}}),
            json!({"remove": {"id": "d1"}}),
            json!({"setExcluded": {"paths": ["/home/nate/Movies"]}}),
            json!({"setAutomatic": {"enabled": false}}),
        ]
    );
    drop(session);
    played.end();
}

#[test]
fn low_power_mode_holds_scheduled_backups_on_the_one_connection() {
    let directory = scratch("hold");
    let played = Played::serve(&directory.join("socket"), state(disk(), false));
    let mut session = Session::new(&played.socket);
    session.until("told the state", |s| s.state(SHELL).is_some());
    assert_eq!(played.holds(), 0);
    session.low_power.set(true);
    session.until("held", |_| played.holds() == 1);
    session.low_power.set(false);
    session.until("released", |_| played.holds() == 0);
    session.low_power.set(true);
    session.until("held again", |_| played.holds() == 1);
    // All on the one connection that watches.
    assert_eq!(
        played.requests(),
        vec![
            json!("watch"),
            json!({"hold": {"reason": "lowPower"}}),
            json!({"release": {}}),
            json!({"hold": {"reason": "lowPower"}}),
        ]
    );
    // The session ends: its hold goes with its connection.
    drop(session);
    let deadline = Instant::now() + Duration::from_secs(5);
    while played.holds() != 0 {
        assert!(Instant::now() < deadline, "the hold outlived the session");
        std::thread::sleep(Duration::from_millis(10));
    }
    played.end();
}

#[test]
fn without_the_service_none_is_told_and_it_is_followed_once_it_is_there() {
    let directory = scratch("absent");
    let socket = directory.join("run/oxyos/backupd/socket");
    let mut session = Session::new(&socket);
    // Asked before anything is known: answered once it is.
    let asked = session.ask(SETTINGS, r#"{"backup":{}}"#, false);
    session.until("told there is none", |s| s.state(SHELL).is_some());
    assert_eq!(session.state(SHELL), Some(Value::Null));
    assert_eq!(session.answered(SETTINGS, asked), json!({"backup": null}));
    let asked = session.ask(SHELL, r#"{"backupNow":{}}"#, true);
    assert_eq!(
        session.answered(SHELL, asked),
        json!({"failed": {"reason": "unavailable"}})
    );
    // Low Power Mode on while there is none: held once it is there.
    session.low_power.set(true);
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let played = Played::serve(&socket, state(json!([]), false));
    session.until("followed once it is there", |s| {
        s.state(SHELL).is_some_and(|state| state.is_object())
    });
    session.until("held on it", |_| played.holds() == 1);
    // It goes: none told; started again: followed and held again, without a clock.
    played.end();
    session.until("told it went", |s| s.state(SHELL) == Some(Value::Null));
    let again = Played::serve(&socket, state(disk(), true));
    session.until("followed again", |s| {
        s.state(SETTINGS)
            .is_some_and(|state| state["destinations"][0]["id"] == "d1")
    });
    session.until("held again", |_| again.holds() == 1);
    drop(session);
    again.end();
}

#[test]
fn the_real_service_is_followed_and_held() {
    use oxyos_backup::disks::Played as PlayedDisks;
    use oxyos_backup::engine::Engine;
    use oxyos_backup::policy::Sources;
    use oxyos_backup::service::{Options, Service};
    let directory = scratch("real");
    let home = directory.join("home");
    std::fs::create_dir_all(&home).unwrap();
    // No destination: nothing runs restic, so restic need not be installed.
    let service = Service::open(
        Options {
            state: directory.join("state"),
            engine: Engine::new(&directory.join("cache")),
            disks: Arc::new(PlayedDisks::open(&directory.join("volumes")).unwrap()),
            sources: Sources {
                paths: vec![home],
                excludes: Vec::new(),
            },
            passwd: "/etc/passwd".into(),
            group: "/etc/group".into(),
            accounts: None,
            manifest: None,
            development: true,
            clock: Arc::new(oxyos_backup::service::now),
        },
        "test",
    )
    .unwrap();
    let socket = directory.join("socket");
    let listener = Listener::bind(&socket, 0o600).unwrap();
    {
        let service = service.clone();
        std::thread::spawn(move || {
            while let Ok(connection) = listener.accept() {
                let service = service.clone();
                std::thread::spawn(move || {
                    let _ = service.serve(&connection);
                });
            }
        });
    }
    let mut session = Session::new(&socket);
    session.until("told the service's state", |s| {
        s.state(SETTINGS)
            .is_some_and(|state| state["administrator"] == true && state["held"] == false)
    });
    session.low_power.set(true);
    session.until("told it holds", |s| {
        s.state(SHELL).is_some_and(|state| state["held"] == true)
    });
    session.low_power.set(false);
    session.until("told it releases", |s| {
        s.state(SHELL).is_some_and(|state| state["held"] == false)
    });
    // With no destination, the service's own refusal, as a failure.
    let asked = session.ask(SHELL, r#"{"backupNow":{}}"#, true);
    assert_eq!(
        session.answered(SHELL, asked),
        json!({"failed": {"reason": "noDestination"}})
    );
    drop(session);
    service.shutdown();
}
