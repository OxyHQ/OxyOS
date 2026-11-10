//! Scopes' names, the fixture's layout in a directory of the test's own, and the user manager's
//! side against a played one on a bus of the test's own (`played`).

use super::*;
use played::{Call, Played};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("oxyos-scopes-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

struct Bus(Child);

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn private_bus(directory: &Path) -> (String, Bus) {
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
    (address.trim().to_string(), Bus(bus))
}

fn connect(address: &str) -> zbus::blocking::Connection {
    zbus::blocking::connection::Builder::address(address)
        .unwrap()
        .build()
        .unwrap()
}

#[test]
fn names_are_systemds() {
    assert_eq!(escape("so.oxy.settings"), "so.oxy.settings");
    // A dash would read as the name's separator; a leading dot would hide the name.
    assert_eq!(escape("so.oxy.my-app"), "so.oxy.my\\x2dapp");
    assert_eq!(escape(".x/y z"), "\\x2ex\\x2fy\\x20z");
    assert_eq!(
        Unit::application("so.oxy.dropsea", 4242),
        Unit {
            name: "app-oxyos-so.oxy.dropsea-4242.scope".into(),
            slice: Slice::App
        }
    );
    assert_eq!(
        Unit::background("search", 7),
        Unit {
            name: "oxyos-search-7.scope".into(),
            slice: Slice::Background
        }
    );
    assert_eq!(Slice::App.name(), "app.slice");
    assert_eq!(Slice::Background.name(), "background.slice");
}

#[test]
fn the_fixture_makes_the_user_managers_layout() {
    let root = scratch("fixture");
    let app = Unit::application("so.oxy.settings", 100);
    let search = Unit::background("search", 101);
    // Weighed before it is there: not yet.
    let weighed = Properties {
        cpu_weight: Some(CpuWeight::Weight(400)),
        io_weight: Some(50),
    };
    assert!(!fixture::set(&root, &app, weighed).unwrap());
    place(&Manager::Fixture(root.clone()), &app, "Settings", 100).unwrap();
    place(&Manager::Fixture(root.clone()), &search, "Spotlight", 101).unwrap();
    let read = |path: PathBuf| std::fs::read_to_string(path).unwrap();
    let scope = root.join("app.slice/app-oxyos-so.oxy.settings-100.scope");
    assert_eq!(fixture::scope(&root, &app), scope);
    assert_eq!(read(scope.join("cgroup.procs")), "100");
    assert_eq!(
        read(root.join("background.slice/oxyos-search-101.scope/cgroup.procs")),
        "101"
    );
    assert!(fixture::set(&root, &app, weighed).unwrap());
    assert_eq!(read(scope.join("cpu.weight")), "400");
    assert_eq!(read(scope.join("io.weight")), "default 50");
    let idle = Properties {
        cpu_weight: Some(CpuWeight::Idle),
        io_weight: None,
    };
    assert!(fixture::set(&root, &app, idle).unwrap());
    assert_eq!(read(scope.join("cpu.idle")), "1");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_user_manager_places_and_weighs() {
    let directory = scratch("user");
    let (address, _bus) = private_bus(&directory);
    let played = Played::serve(&address).unwrap();
    let connection = connect(&address);
    let app = Unit::application("so.oxy.dropsea", 4242);
    let weighed = Properties {
        cpu_weight: Some(CpuWeight::Weight(20)),
        io_weight: Some(10),
    };
    // Not there yet: told so, nothing changed.
    assert!(!user::set(&connection, &app, weighed).unwrap());
    // Placed once its job ended (after the call's answer).
    user::place(&connection, &app, "Dropsea", 4242).unwrap();
    assert!(user::set(&connection, &app, weighed).unwrap());
    let idle = Properties {
        cpu_weight: Some(CpuWeight::Idle),
        io_weight: None,
    };
    assert!(user::set(&connection, &app, idle).unwrap());
    let search = Unit::background("search", 4243);
    user::place(&connection, &search, "Spotlight", 4243).unwrap();
    assert_eq!(
        played.calls(),
        vec![
            Call::Start {
                name: "app-oxyos-so.oxy.dropsea-4242.scope".into(),
                slice: "app.slice".into(),
                pids: vec![4242],
                cpu_weight: Some(DEFAULT_CPU_WEIGHT),
            },
            Call::Set {
                name: "app-oxyos-so.oxy.dropsea-4242.scope".into(),
                runtime: true,
                cpu_weight: Some(20),
                io_weight: Some(10),
            },
            // `CPUWeight=idle` is the weight 0 on the bus.
            Call::Set {
                name: "app-oxyos-so.oxy.dropsea-4242.scope".into(),
                runtime: true,
                cpu_weight: Some(0),
                io_weight: None,
            },
            Call::Start {
                name: "oxyos-search-4243.scope".into(),
                slice: "background.slice".into(),
                pids: vec![4243],
                cpu_weight: Some(DEFAULT_CPU_WEIGHT),
            },
        ]
    );
    // The same scope asked for again fails: a placement that failed says so.
    assert!(user::place(&connection, &app, "Dropsea", 4242).is_err());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn scopes_apply_what_is_asked_once_the_scope_is_there() {
    let root = scratch("scopes");
    let scopes = Scopes::start(Manager::Fixture(root.clone())).unwrap();
    let app = Unit::application("so.oxy.inbox", 9);
    // Asked twice before its process is placed: the latest wins, merged with the first.
    scopes.set(
        &app,
        Properties {
            cpu_weight: Some(CpuWeight::Weight(400)),
            io_weight: Some(100),
        },
    );
    scopes.set(
        &app,
        Properties {
            cpu_weight: Some(CpuWeight::Weight(20)),
            io_weight: None,
        },
    );
    std::thread::sleep(Duration::from_millis(120));
    place(&Manager::Fixture(root.clone()), &app, "Inbox", 9).unwrap();
    let scope = fixture::scope(&root, &app);
    let deadline = Instant::now() + Duration::from_secs(3);
    while std::fs::read_to_string(scope.join("io.weight")).is_err() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(scope.join("cpu.weight")).unwrap(),
        "20"
    );
    assert_eq!(
        std::fs::read_to_string(scope.join("io.weight")).unwrap(),
        "default 100"
    );
    let _ = std::fs::remove_dir_all(root);
}
