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

// ---- The memory the seat's applications claim ----

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

#[test]
fn a_claim_is_carried_up_over_each_floor_and_capped() {
    assert_eq!(lows(0, 16 * GIB), FLOORS);
    assert_eq!(
        lows(300 * MIB, 16 * GIB),
        Lows {
            manager: 428 * MIB,
            user: 428 * MIB,
            users: 556 * MIB,
        }
    );
    // Never more than half the machine claimed.
    assert_eq!(lows(u64::MAX, 4 * GIB), lows(2 * GIB, 4 * GIB));
    assert_eq!(lows(u64::MAX, 4 * GIB).manager, 128 * MIB + 2 * GIB);
}

#[test]
fn a_protection_is_raised_to_its_claim_and_lowered_only_where_raised() {
    // Not there: nothing.
    assert_eq!(settle(None, 428 * MIB, 128 * MIB), None);
    // The claim: raised, and changed with it, either way.
    assert_eq!(settle(Some(128 * MIB), 428 * MIB, 128 * MIB), Some(428 * MIB));
    assert_eq!(settle(Some(428 * MIB), 300 * MIB, 128 * MIB), Some(300 * MIB));
    assert_eq!(settle(Some(428 * MIB), 428 * MIB, 128 * MIB), None);
    // No claim: a raise taken back to the floor; a unit at or under it (a machine without the
    // image's floors) left as it is.
    assert_eq!(settle(Some(428 * MIB), 128 * MIB, 128 * MIB), Some(128 * MIB));
    assert_eq!(settle(Some(0), 128 * MIB, 128 * MIB), None);
    assert_eq!(settle(Some(128 * MIB), 128 * MIB, 128 * MIB), None);
}

#[test]
fn a_users_claim_is_its_sessions_largest_and_ends_with_them() {
    let mut claims = Claims::new();
    let claim = |registrar, uid, bytes| MemoryClaim {
        uid,
        registrar,
        bytes,
    };
    take(&mut claims, claim(1, 1002, Some(300 * MIB)));
    take(&mut claims, claim(2, 1002, Some(100 * MIB)));
    take(&mut claims, claim(3, 1000, Some(500 * MIB)));
    assert_eq!(claim_of(&claims, 1002), 300 * MIB);
    take(&mut claims, claim(1, 1002, Some(50 * MIB)));
    assert_eq!(claim_of(&claims, 1002), 100 * MIB);
    take(&mut claims, claim(2, 1002, None));
    assert_eq!(claim_of(&claims, 1002), 50 * MIB);
    take(&mut claims, claim(1, 1002, None));
    assert_eq!(claim_of(&claims, 1002), 0);
    assert_eq!(claim_of(&claims, 1000), 500 * MIB);
}

/// A size as systemd reads one: bytes, or K, M, G, T (1024's).
fn size(text: &str) -> u64 {
    let text = text.trim();
    let (number, unit) = text.split_at(text.trim_end_matches(char::is_alphabetic).len());
    let shift = match unit {
        "" => 0,
        "K" => 10,
        "M" => 20,
        "G" => 30,
        "T" => 40,
        other => panic!("{other}: not a size this test reads"),
    };
    number.parse::<u64>().unwrap() << shift
}

/// The last `MemoryLow=` of `section` in the image's drop-in `path` (from the repository's root).
fn image_memory_low(path: &str, section: &str) -> u64 {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join(path)).unwrap();
    let mut current = String::new();
    let mut low = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            current = line.trim_matches(['[', ']']).to_owned();
        } else if let Some(value) = line.strip_prefix("MemoryLow=") {
            if current == section {
                low = Some(size(value));
            }
        }
    }
    low.unwrap_or_else(|| panic!("no MemoryLow in {path}'s [{section}]"))
}

#[test]
fn the_floors_are_the_images_own() {
    let drop_in = |unit: &str| {
        format!("config/includes.chroot_after_packages/usr/lib/systemd/system/{unit}.d/50-oxyos-memory.conf")
    };
    assert_eq!(
        Lows {
            manager: image_memory_low(&drop_in("user@.service"), "Service"),
            user: image_memory_low(&drop_in("user-.slice"), "Slice"),
            users: image_memory_low(&drop_in("user.slice"), "Slice"),
        },
        FLOORS
    );
}

#[test]
fn the_benchmarks_tree_is_given_the_claims_protections() {
    let root = std::env::temp_dir().join(format!("oxyos-accountd-memory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let users = root.join("machine/user.slice");
    std::fs::create_dir_all(users.join("user-seat.slice/user@.service")).unwrap();
    std::fs::create_dir_all(users.join("user-other.slice")).unwrap();
    // The benchmark's machine: 1 GiB, so a claim of 900 MiB is capped at 512.
    std::fs::write(root.join("machine/memory.max"), format!("{}\n", GIB)).unwrap();
    std::fs::write(users.join("memory.max"), "max\n").unwrap();
    let applied = apply_memory_claim_to_tree(&users, "user-seat.slice", 900 * MIB).unwrap();
    assert_eq!(
        applied,
        vec![
            ("user.slice/user-seat.slice/user@.service".into(), 640 * MIB),
            ("user.slice/user-seat.slice".into(), 640 * MIB),
            ("user.slice".into(), 768 * MIB),
        ]
    );
    let read = |path: &str| std::fs::read_to_string(users.join(path)).unwrap();
    assert_eq!(read("user-seat.slice/user@.service/memory.low"), format!("{}\n", 640 * MIB));
    assert_eq!(read("memory.low"), format!("{}\n", 768 * MIB));
    assert!(!users.join("user-other.slice/memory.low").exists());
    // No claim: the floors.
    let applied = apply_memory_claim_to_tree(&users, "user-seat.slice", 0).unwrap();
    assert_eq!(applied[2].1, FLOORS.users);
    assert!(apply_memory_claim_to_tree(&users, "user-other.slice", 0).is_err());
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
    /// systemd's units there now, each with the properties set on it (none set: unset).
    units: BTreeMap<String, BTreeMap<String, u64>>,
    /// Every SetUnitProperties: the unit, runtime, and each property.
    calls: Vec<Call>,
}

type Call = (String, bool, Vec<(String, u64)>);
type Shared = Arc<Mutex<Model>>;

fn session_path(id: &str) -> String {
    format!("/org/freedesktop/login1/session/_3{id}")
}

fn user_path(uid: u32) -> String {
    format!("/org/freedesktop/login1/user/_{uid}")
}

/// A unit's object, its name escaped as systemd escapes it.
fn unit_path(name: &str) -> String {
    let escaped: String = name
        .bytes()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => (byte as char).to_string(),
            _ => format!("_{byte:02x}"),
        })
        .collect();
    format!("/org/freedesktop/systemd1/unit/{escaped}")
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

#[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
impl Systemd {
    fn get_unit(&self, name: String) -> Result<OwnedObjectPath, SystemdError> {
        if self.0.lock().unwrap().units.contains_key(&name) {
            Ok(path(unit_path(&name)))
        } else {
            Err(SystemdError::NoSuchUnit(format!("Unit {name} not loaded.")))
        }
    }

    fn set_unit_properties(
        &self,
        name: String,
        runtime: bool,
        properties: Vec<(String, OwnedValue)>,
    ) -> Result<(), SystemdError> {
        let mut model = self.0.lock().unwrap();
        let Some(unit) = model.units.get_mut(&name) else {
            return Err(SystemdError::NoSuchUnit(format!("Unit {name} not found.")));
        };
        let set: Vec<(String, u64)> = properties
            .iter()
            .map(|(key, value)| (key.clone(), u64::try_from(value).expect("a u64 property")))
            .collect();
        for (key, value) in &set {
            unit.insert(key.clone(), *value);
        }
        model.calls.push((name, runtime, set));
        Ok(())
    }
}

/// A unit's property as the played systemd has it (unset weights `UNSET`, protection 0).
fn property(model: &Shared, unit: &str, key: &str) -> u64 {
    let unset = if key == "MemoryLow" { 0 } else { UNSET };
    model.lock().unwrap().units.get(unit).and_then(|unit| unit.get(key).copied()).unwrap_or(unset)
}

struct SliceUnit(Shared, String);

#[zbus::interface(name = "org.freedesktop.systemd1.Slice")]
impl SliceUnit {
    #[zbus(property, name = "CPUWeight")]
    fn cpu_weight(&self) -> u64 {
        property(&self.0, &self.1, "CPUWeight")
    }

    #[zbus(property, name = "IOWeight")]
    fn io_weight(&self) -> u64 {
        property(&self.0, &self.1, "IOWeight")
    }

    #[zbus(property, name = "MemoryLow")]
    fn memory_low(&self) -> u64 {
        property(&self.0, &self.1, "MemoryLow")
    }
}

struct ServiceUnit(Shared, String);

#[zbus::interface(name = "org.freedesktop.systemd1.Service")]
impl ServiceUnit {
    #[zbus(property, name = "MemoryLow")]
    fn memory_low(&self) -> u64 {
        property(&self.0, &self.1, "MemoryLow")
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

/// logind and systemd played on `address`: every session of `model`, and every unit a test may
/// make (GetUnit names one only while the model has it).
fn play(address: &str, model: &Shared) -> Connection {
    let sessions: Vec<String> = model.lock().unwrap().sessions.keys().cloned().collect();
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
    let mut slices = vec!["user.slice".to_owned()];
    slices.extend((1000..1010).map(slice));
    for name in slices {
        builder = builder
            .serve_at(unit_path(&name), SliceUnit(model.clone(), name))
            .unwrap();
    }
    for uid in 1000..1010 {
        let name = format!("user@{uid}.service");
        builder = builder
            .serve_at(unit_path(&name), ServiceUnit(model.clone(), name))
            .unwrap();
    }
    builder.build().unwrap()
}

/// Waits until the played systemd has had `count` calls, and returns them.
fn calls(model: &Shared, count: usize) -> Vec<Call> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let calls = model.lock().unwrap().calls.clone();
        if calls.len() >= count || Instant::now() > deadline {
            return calls;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn raised(uid: u32) -> Call {
    weighed(uid, SEAT_WEIGHT)
}

fn lowered(uid: u32) -> Call {
    weighed(uid, UNSET)
}

fn weighed(uid: u32, weight: u64) -> Call {
    (
        slice(uid),
        true,
        vec![("CPUWeight".into(), weight), ("IOWeight".into(), weight)],
    )
}

fn protected(unit: &str, bytes: u64) -> Call {
    (unit.into(), true, vec![("MemoryLow".into(), bytes)])
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

/// A follower of the seat on `address`, its claims told through the returned sender, on a machine
/// of `memory` bytes; what it says sent to the returned receiver.
fn follower(
    address: &str,
    memory: u64,
) -> (async_channel::Sender<MemoryClaim>, mpsc::Receiver<String>) {
    let (said, heard) = mpsc::channel();
    let (claimed, claims) = async_channel::unbounded();
    let bus = zbus::blocking::connection::Builder::address(address)
        .unwrap()
        .build()
        .unwrap();
    std::thread::spawn(move || {
        let mut log = |line: String| {
            let _ = said.send(line);
        };
        let _ = follow(&bus, &claims, &mut Claims::new(), memory, &mut log);
    });
    (claimed, heard)
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("oxyos-accountd-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn units(entries: &[(&str, &[(&str, u64)])]) -> BTreeMap<String, BTreeMap<String, u64>> {
    entries
        .iter()
        .map(|(unit, properties)| {
            (
                unit.to_string(),
                properties.iter().map(|(key, value)| (key.to_string(), *value)).collect(),
            )
        })
        .collect()
}

#[test]
fn the_seat_followed_raises_its_holder_and_lowers_the_one_before() {
    let directory = scratch("seat-bus");
    let bus = private_bus(&directory);
    // The person (1002) on seat0's session 3; the machine's owner (1000) over ssh, session 5;
    // 1001 lingering, its slice left raised from before the service started.
    let model: Shared = Arc::new(Mutex::new(Model {
        users: vec![1000, 1001, 1002],
        sessions: HashMap::from([("3".into(), 1002), ("5".into(), 1000)]),
        active: Some("3".into()),
        units: units(&[
            ("user-1000.slice", &[]),
            ("user-1001.slice", &[("CPUWeight", 400), ("IOWeight", 400)]),
            ("user-1002.slice", &[]),
        ]),
        calls: Vec::new(),
    }));
    let played = play(&bus.address, &model);
    let (_claims, heard) = follower(&bus.address, 16 * GIB);

    // At its start: the seat's holder raised, the stale one lowered, nobody else touched.
    assert_eq!(calls(&model, 2), vec![lowered(1001), raised(1002)]);
    let line = heard.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(line.contains("1001"), "{line}");

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
        model.units.insert(
            "user-1003.slice".into(),
            BTreeMap::from([("CPUWeight".into(), 400), ("IOWeight".into(), 400)]),
        );
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

#[test]
fn the_seats_applications_claim_is_carried_up_and_handed_back() {
    let directory = scratch("memory-bus");
    let bus = private_bus(&directory);
    let floors = |uid: u32| -> Vec<(String, Vec<(&'static str, u64)>)> {
        vec![
            (format!("user@{uid}.service"), vec![("MemoryLow", FLOORS.manager)]),
            (slice(uid), vec![("MemoryLow", FLOORS.user)]),
        ]
    };
    // The image's floors on every unit; the person (1002) at the seat, already weighed.
    let model: Shared = Arc::new(Mutex::new(Model {
        users: vec![1000, 1002],
        sessions: HashMap::from([("3".into(), 1002), ("5".into(), 1000)]),
        active: Some("3".into()),
        units: BTreeMap::new(),
        calls: Vec::new(),
    }));
    {
        let mut model = model.lock().unwrap();
        for uid in [1000, 1002] {
            for (unit, properties) in floors(uid) {
                model.units.insert(
                    unit,
                    properties.into_iter().map(|(key, value)| (key.into(), value)).collect(),
                );
            }
        }
        let seat = model.units.get_mut("user-1002.slice").unwrap();
        seat.insert("CPUWeight".into(), 400);
        seat.insert("IOWeight".into(), 400);
        model
            .units
            .insert("user.slice".into(), BTreeMap::from([("MemoryLow".into(), FLOORS.users)]));
    }
    let played = play(&bus.address, &model);
    let (claims, _heard) = follower(&bus.address, 4 * GIB);
    let claim = |registrar, uid, bytes| {
        claims
            .try_send(MemoryClaim {
                uid,
                registrar,
                bytes,
            })
            .unwrap()
    };
    // Nothing claimed: nothing written (every unit at its floor).
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(model.lock().unwrap().calls, vec![]);

    // The person's session claims 300 MiB: carried up its manager, its slice and the users'.
    claim(7, 1002, Some(300 * MIB));
    assert_eq!(
        calls(&model, 3),
        vec![
            protected("user@1002.service", 428 * MIB),
            protected("user-1002.slice", 428 * MIB),
            protected("user.slice", 556 * MIB),
        ]
    );
    // Another user's session claims: kept, not given while it is not at the seat.
    claim(9, 1000, Some(200 * MIB));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(model.lock().unwrap().calls.len(), 3);

    // 1000 takes the seat: its claim given, the person's handed back to the floors.
    model.lock().unwrap().active = Some("5".into());
    seat_changed(&played);
    assert_eq!(
        calls(&model, 10)[3..],
        [
            raised(1000),
            lowered(1002),
            protected("user@1000.service", 328 * MIB),
            protected("user-1000.slice", 328 * MIB),
            protected("user@1002.service", FLOORS.manager),
            protected("user-1002.slice", FLOORS.user),
            protected("user.slice", 456 * MIB),
        ]
    );
    // Its session ends: its claim with it, the floors again.
    claim(9, 1000, None);
    assert_eq!(
        calls(&model, 13)[10..],
        [
            protected("user@1000.service", FLOORS.manager),
            protected("user-1000.slice", FLOORS.user),
            protected("user.slice", FLOORS.users),
        ]
    );
    // The person back at the seat: its claim, still told, given again.
    model.lock().unwrap().active = Some("3".into());
    seat_changed(&played);
    assert_eq!(
        calls(&model, 18)[13..],
        [
            raised(1002),
            lowered(1000),
            protected("user@1002.service", 428 * MIB),
            protected("user-1002.slice", 428 * MIB),
            protected("user.slice", 556 * MIB),
        ]
    );
    // A claim past half the machine (4 GiB) is capped at 2 GiB.
    claim(7, 1002, Some(64 * GIB));
    assert_eq!(
        calls(&model, 21)[18..],
        [
            protected("user@1002.service", 128 * MIB + 2 * GIB),
            protected("user-1002.slice", 128 * MIB + 2 * GIB),
            protected("user.slice", 256 * MIB + 2 * GIB),
        ]
    );
    let _ = std::fs::remove_dir_all(&directory);
}
