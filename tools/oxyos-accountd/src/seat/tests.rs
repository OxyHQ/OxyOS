use super::*;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::zvariant::OwnedValue;

fn set(cpu: u64, io: u64) -> Weights {
    Weights {
        cpu: Some(cpu),
        io: Some(io),
    }
}

// ---- The policy ----

#[test]
fn the_seats_holder_is_raised_and_nobody_else_is_touched() {
    let slices = BTreeMap::from([(1000, Weights::default()), (1002, Weights::default())]);
    assert_eq!(plan(Some(1002), &slices), vec![Change::Raise(1002)]);
    assert_eq!(weight(true), 400);
    assert_eq!(weight(false), 100);
}

#[test]
fn a_holder_raised_already_is_left_alone() {
    let slices = BTreeMap::from([(1000, Weights::default()), (1002, set(400, 400))]);
    assert_eq!(plan(Some(1002), &slices), vec![]);
    // Half raised (one weight lost): raised again, whole.
    let slices = BTreeMap::from([(
        1002,
        Weights {
            cpu: Some(400),
            io: None,
        },
    )]);
    assert_eq!(plan(Some(1002), &slices), vec![Change::Raise(1002)]);
}

#[test]
fn the_holder_before_is_lowered_and_another_weight_kept() {
    // Fast user switching: 1000 now holds the seat, 1002 held it. 1001 has a weight of its own
    // (an administrator's), which is not the seat's: left as it is.
    let slices = BTreeMap::from([
        (1000, Weights::default()),
        (1001, set(250, 100)),
        (1002, set(400, 400)),
    ]);
    assert_eq!(
        plan(Some(1000), &slices),
        vec![Change::Raise(1000), Change::Lower(1002)]
    );
}

#[test]
fn nobody_at_the_seat_leaves_nobody_raised() {
    let slices = BTreeMap::from([(1000, set(400, 400)), (1002, Weights::default())]);
    assert_eq!(plan(None, &slices), vec![Change::Lower(1000)]);
    assert_eq!(plan(None, &BTreeMap::new()), vec![]);
}

#[test]
fn the_benchmarks_tree_is_weighed_as_the_machine() {
    let root = std::env::temp_dir().join(format!("oxyos-accountd-seat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for slice in ["user-seat.slice", "user-other.slice"] {
        std::fs::create_dir_all(root.join(slice)).unwrap();
        std::fs::write(root.join(slice).join("cpu.weight"), "100\n").unwrap();
    }
    // The io controller on for one only: its io.weight written, the other's never made.
    std::fs::write(
        root.join("user-seat.slice").join("io.weight"),
        "default 100\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("not-a-slice")).unwrap();
    let applied = apply_to_tree(&root, "user-seat.slice").unwrap();
    assert_eq!(
        applied,
        vec![
            ("user-other.slice".into(), 100),
            ("user-seat.slice".into(), 400)
        ]
    );
    let read = |path: &str| std::fs::read_to_string(root.join(path)).unwrap();
    assert_eq!(read("user-seat.slice/cpu.weight"), "400\n");
    assert_eq!(read("user-seat.slice/io.weight"), "default 400\n");
    assert_eq!(read("user-other.slice/cpu.weight"), "100\n");
    assert!(!root.join("user-other.slice/io.weight").exists());
    assert!(!root.join("not-a-slice/cpu.weight").exists());
    assert!(apply_to_tree(&root, "user-gone.slice").is_err());
    let _ = std::fs::remove_dir_all(&root);
}

// ---- Followed: logind and systemd played on a bus of the test's own ----

#[derive(Default)]
struct Model {
    /// logind's users.
    users: Vec<u32>,
    /// Each session's user, by the session's id.
    sessions: HashMap<String, u32>,
    /// seat0's active session.
    active: Option<String>,
    /// Each user's slice as systemd has it: (CPUWeight, IOWeight), UNSET for none.
    slices: BTreeMap<u32, (u64, u64)>,
    /// Every SetUnitProperties: (unit, runtime, CPUWeight, IOWeight).
    calls: Vec<(String, bool, u64, u64)>,
}

type Shared = Arc<Mutex<Model>>;

fn session_path(id: &str) -> String {
    format!("/org/freedesktop/login1/session/_3{id}")
}

fn user_path(uid: u32) -> String {
    format!("/org/freedesktop/login1/user/_{uid}")
}

fn unit_path(uid: u32) -> String {
    format!("/org/freedesktop/systemd1/unit/user_2d{uid}_2eslice")
}

fn path(path: String) -> OwnedObjectPath {
    OwnedObjectPath::try_from(path).unwrap()
}

struct Logind(Shared);

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl Logind {
    fn list_users(&self) -> Vec<(u32, String, OwnedObjectPath)> {
        let model = self.0.lock().unwrap();
        model
            .users
            .iter()
            .map(|&uid| (uid, format!("user{uid}"), path(user_path(uid))))
            .collect()
    }
}

struct Seat(Shared);

#[zbus::interface(name = "org.freedesktop.login1.Seat")]
impl Seat {
    #[zbus(property)]
    fn active_session(&self) -> (String, OwnedObjectPath) {
        match &self.0.lock().unwrap().active {
            Some(id) => (id.clone(), path(session_path(id))),
            None => (String::new(), path("/".into())),
        }
    }
}

struct Session(Shared, String);

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl Session {
    #[zbus(property)]
    fn user(&self) -> (u32, OwnedObjectPath) {
        let uid = self.0.lock().unwrap().sessions[&self.1];
        (uid, path(user_path(uid)))
    }
}

#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "org.freedesktop.systemd1")]
enum SystemdError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NoSuchUnit(String),
}

struct Systemd(Shared);

fn uid_of(unit: &str) -> Option<u32> {
    unit.strip_prefix("user-")?
        .strip_suffix(".slice")?
        .parse()
        .ok()
}

#[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
impl Systemd {
    fn get_unit(&self, name: String) -> Result<OwnedObjectPath, SystemdError> {
        match uid_of(&name) {
            Some(uid) if self.0.lock().unwrap().slices.contains_key(&uid) => {
                Ok(path(unit_path(uid)))
            }
            _ => Err(SystemdError::NoSuchUnit(format!("Unit {name} not loaded."))),
        }
    }

    fn set_unit_properties(
        &self,
        name: String,
        runtime: bool,
        properties: Vec<(String, OwnedValue)>,
    ) -> Result<(), SystemdError> {
        let mut model = self.0.lock().unwrap();
        let uid = uid_of(&name)
            .filter(|uid| model.slices.contains_key(uid))
            .ok_or_else(|| SystemdError::NoSuchUnit(format!("Unit {name} not found.")))?;
        let value = |wanted: &str| {
            properties
                .iter()
                .find(|(name, _)| name == wanted)
                .and_then(|(_, value)| u64::try_from(value).ok())
                .expect("both weights, as u64")
        };
        let (cpu, io) = (value("CPUWeight"), value("IOWeight"));
        model.slices.insert(uid, (cpu, io));
        model.calls.push((name, runtime, cpu, io));
        Ok(())
    }
}

struct Slice(Shared, u32);

#[zbus::interface(name = "org.freedesktop.systemd1.Slice")]
impl Slice {
    #[zbus(property, name = "CPUWeight")]
    fn cpu_weight(&self) -> u64 {
        self.0
            .lock()
            .unwrap()
            .slices
            .get(&self.1)
            .map_or(UNSET, |weights| weights.0)
    }

    #[zbus(property, name = "IOWeight")]
    fn io_weight(&self) -> u64 {
        self.0
            .lock()
            .unwrap()
            .slices
            .get(&self.1)
            .map_or(UNSET, |weights| weights.1)
    }
}

struct Bus {
    address: String,
    daemon: Child,
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

fn private_bus(directory: &std::path::Path) -> Bus {
    let mut daemon = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .arg(format!(
            "--address=unix:path={}",
            directory.join("bus").display()
        ))
        .stdout(Stdio::piped())
        .spawn()
        .expect("dbus-daemon (the dbus-daemon package)");
    let mut address = String::new();
    BufReader::new(daemon.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    Bus {
        address: address.trim().into(),
        daemon,
    }
}

/// logind and systemd played on `address`, every session and slice of `model` served.
fn play(address: &str, model: &Shared) -> Connection {
    let (sessions, uids): (Vec<String>, Vec<u32>) = {
        let model = model.lock().unwrap();
        (
            model.sessions.keys().cloned().collect(),
            (1000..1010).collect(),
        )
    };
    let mut builder = zbus::blocking::connection::Builder::address(address)
        .unwrap()
        .name(LOGIND)
        .unwrap()
        .name(SYSTEMD)
        .unwrap()
        .serve_at(LOGIND_PATH, Logind(model.clone()))
        .unwrap()
        .serve_at(SEAT0, Seat(model.clone()))
        .unwrap()
        .serve_at(SYSTEMD_PATH, Systemd(model.clone()))
        .unwrap();
    for id in sessions {
        builder = builder
            .serve_at(session_path(&id), Session(model.clone(), id))
            .unwrap();
    }
    // Every slice a test may make: GetUnit names one only while the model has it.
    for uid in uids {
        builder = builder
            .serve_at(unit_path(uid), Slice(model.clone(), uid))
            .unwrap();
    }
    builder.build().unwrap()
}

/// Waits until the played systemd has had `count` calls, and returns them.
fn calls(model: &Shared, count: usize) -> Vec<(String, bool, u64, u64)> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let calls = model.lock().unwrap().calls.clone();
        if calls.len() >= count || Instant::now() > deadline {
            return calls;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn raised(uid: u32) -> (String, bool, u64, u64) {
    (slice(uid), true, SEAT_WEIGHT, SEAT_WEIGHT)
}

fn lowered(uid: u32) -> (String, bool, u64, u64) {
    (slice(uid), true, UNSET, UNSET)
}

fn seat_changed(played: &Connection) {
    let none: HashMap<&str, Value<'_>> = HashMap::new();
    played
        .emit_signal(
            None::<()>,
            SEAT0,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &("org.freedesktop.login1.Seat", none, vec!["ActiveSession"]),
        )
        .unwrap();
}

#[test]
fn the_seat_followed_raises_its_holder_and_lowers_the_one_before() {
    let directory: PathBuf =
        std::env::temp_dir().join(format!("oxyos-accountd-seat-bus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let bus = private_bus(&directory);
    // The person (1002) on seat0's session 3; the machine's owner (1000) over ssh, session 5;
    // 1001 lingering, its slice left raised from before the service started.
    let model: Shared = Arc::new(Mutex::new(Model {
        users: vec![1000, 1001, 1002],
        sessions: HashMap::from([("3".into(), 1002), ("5".into(), 1000)]),
        active: Some("3".into()),
        slices: BTreeMap::from([
            (1000, (UNSET, UNSET)),
            (1001, (400, 400)),
            (1002, (UNSET, UNSET)),
        ]),
        calls: Vec::new(),
    }));
    let played = play(&bus.address, &model);
    let (said, heard) = mpsc::channel();
    let follower = zbus::blocking::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .build()
        .unwrap();
    std::thread::spawn(move || {
        let mut log = |line: String| {
            let _ = said.send(line);
        };
        let _ = follow(&follower, &mut log);
    });

    // At its start: the seat's holder raised, the stale one lowered, nobody else touched.
    assert_eq!(calls(&model, 2), vec![lowered(1001), raised(1002)]);
    let lines: Vec<String> = heard
        .recv_timeout(Duration::from_secs(5))
        .into_iter()
        .collect();
    assert!(lines[0].contains("1001"), "{lines:?}");

    // Fast user switching: 1000 takes the seat (its session 5 now seat0's active one).
    model.lock().unwrap().active = Some("5".into());
    seat_changed(&played);
    assert_eq!(calls(&model, 4)[2..], [raised(1000), lowered(1002)]);

    // A signal that changes nothing writes nothing.
    seat_changed(&played);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(model.lock().unwrap().calls.len(), 4);

    // A terminal with no session switched to: nobody holds the seat.
    model.lock().unwrap().active = None;
    seat_changed(&played);
    assert_eq!(calls(&model, 5)[4..], [lowered(1000)]);

    // A user appearing whose slice was made again with the raise it was given before: lowered.
    {
        let mut model = model.lock().unwrap();
        model.users.push(1003);
        model.slices.insert(1003, (400, 400));
    }
    played
        .emit_signal(
            None::<()>,
            LOGIND_PATH,
            LOGIND_MANAGER,
            "UserNew",
            &(1003u32, path(user_path(1003))),
        )
        .unwrap();
    assert_eq!(calls(&model, 6)[5..], [lowered(1003)]);
    assert_eq!(model.lock().unwrap().calls.len(), 6);
    let _ = std::fs::remove_dir_all(&directory);
}
