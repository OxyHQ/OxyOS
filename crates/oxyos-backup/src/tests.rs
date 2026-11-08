//! The engine and the service over Debian's restic, on directories of the tests' own: a backup
//! restored byte for byte, versions, thinning, a full destination, the service's schedule, its
//! conversations and its refusals.
//!
//! restic must be installed (`apt install restic`). Without it these tests say so and pass,
//! unless `OXYOS_BACKUP_ENGINE=required` (the Debian job of CI, which installs it), where its
//! absence fails them.

use crate::disks::Played;
use crate::engine::{self, Backup, Engine, Location, Repository, Stop};
use crate::ipc::{Answer, Status};
use crate::policy::{self, Sources};
use crate::service::{Options, Service};
use oxyos_install::socket::Connection;
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// restic, if it is installed.
pub(crate) fn engine(cache: &Path) -> Option<Engine> {
    match Engine::from_path(cache) {
        Some(engine) => Some(engine),
        None if std::env::var("OXYOS_BACKUP_ENGINE").as_deref() == Ok("required") => {
            panic!("restic is not installed, and OXYOS_BACKUP_ENGINE=required")
        }
        None => {
            eprintln!(
                "restic is not installed: the engine's tests are skipped (apt install restic)"
            );
            None
        }
    }
}

fn repository(path: &Path, password: &str) -> Repository {
    Repository {
        location: Location::directory(path),
        password: Zeroizing::new(password.into()),
    }
}

/// A tree with what a home holds: nested folders, a link, an empty file, names in other scripts,
/// a file larger than restic's chunks, and a file only its owner may read.
fn tree(root: &Path) {
    fs::create_dir_all(root.join("Documents/Taxes 2026")).unwrap();
    fs::create_dir_all(root.join("Pictures/Ñandú")).unwrap();
    fs::write(root.join("Documents/notes.txt"), "first\n").unwrap();
    fs::write(
        root.join("Documents/Taxes 2026/return.pdf"),
        b"%PDF-1.7 \x00\x01\x02",
    )
    .unwrap();
    fs::write(root.join("Pictures/Ñandú/写真.jpg"), [7u8; 10]).unwrap();
    fs::write(root.join("empty"), b"").unwrap();
    let mut large = Vec::with_capacity(3 << 20);
    let mut x: u32 = 1;
    for _ in 0..(3 << 20) {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        large.push((x >> 24) as u8);
    }
    fs::write(root.join("Pictures/large.raw"), &large).unwrap();
    symlink("Documents/notes.txt", root.join("notes")).unwrap();
    fs::write(root.join("Documents/private"), "mine").unwrap();
    fs::set_permissions(
        root.join("Documents/private"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
}

/// Whether two trees hold the same items, bytes, links and modes.
fn same(a: &Path, b: &Path) {
    let mut left: Vec<PathBuf> = walk(a)
        .into_iter()
        .map(|p| p.strip_prefix(a).unwrap().to_owned())
        .collect();
    let mut right: Vec<PathBuf> = walk(b)
        .into_iter()
        .map(|p| p.strip_prefix(b).unwrap().to_owned())
        .collect();
    left.sort();
    right.sort();
    assert_eq!(left, right, "the trees hold different items");
    for item in left {
        let (x, y) = (a.join(&item), b.join(&item));
        let (mx, my) = (
            fs::symlink_metadata(&x).unwrap(),
            fs::symlink_metadata(&y).unwrap(),
        );
        assert_eq!(mx.file_type(), my.file_type(), "{}", item.display());
        assert_eq!(
            mx.permissions().mode(),
            my.permissions().mode(),
            "{}",
            item.display()
        );
        if mx.file_type().is_symlink() {
            assert_eq!(fs::read_link(&x).unwrap(), fs::read_link(&y).unwrap());
        } else if mx.is_file() {
            assert!(
                fs::read(&x).unwrap() == fs::read(&y).unwrap(),
                "{} differs",
                item.display()
            );
        }
    }
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut items = Vec::new();
    for entry in fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        items.push(path.clone());
        if entry.file_type().unwrap().is_dir() {
            items.extend(walk(&path));
        }
    }
    items
}

#[test]
fn a_backup_is_restored_byte_for_byte_and_opened_by_either_key() {
    let dir = tempfile::tempdir().unwrap();
    let Some(engine) = engine(&dir.path().join("cache")) else {
        return;
    };
    let home = dir.path().join("home");
    tree(&home);
    let machine = repository(&dir.path().join("repo"), "machine key");
    engine
        .init(&machine, Some("the person's password"))
        .unwrap();
    let backup = Backup {
        paths: vec![home.clone()],
        excludes: vec![],
        host: "ada-laptop".into(),
        tags: vec!["oxyos".into()],
        time: None,
    };
    let mut progressed = false;
    let first = engine
        .backup(&machine, &backup, &Stop::new(), |_| progressed = true)
        .unwrap();
    assert!(!first.snapshot_id.is_empty());
    assert!(first.files_new >= 6);
    let _ = progressed; // restic says nothing of progress for a backup this quick

    // The person's password opens it, as on another computer; another does not.
    let person = repository(&dir.path().join("repo"), "the person's password");
    assert_eq!(
        engine.snapshots(&person, Some("ada-laptop")).unwrap().len(),
        1
    );
    assert!(matches!(
        engine.open(&repository(&dir.path().join("repo"), "guess")),
        Err(engine::Error::WrongPassword)
    ));
    assert!(matches!(
        engine.open(&repository(&dir.path().join("nothing"), "machine key")),
        Err(engine::Error::NoRepository)
    ));

    // A change, a second backup: two versions of the file, each read back as it was.
    std::thread::sleep(Duration::from_millis(1100));
    fs::write(home.join("Documents/notes.txt"), "second, longer\n").unwrap();
    let second = engine
        .backup(&machine, &backup, &Stop::new(), |_| {})
        .unwrap();
    assert_eq!(second.files_changed, 1);
    let notes = format!("{}/Documents/notes.txt", home.display());
    let found = engine.find(&machine, "ada-laptop", &notes).unwrap();
    assert_eq!(found.len(), 2);
    let sizes: Vec<Option<u64>> = found.iter().map(|f| f.node.size).collect();
    assert!(sizes.contains(&Some(6)) && sizes.contains(&Some(15)));
    for version in &found {
        let mut dump = engine
            .dump(&machine, &version.snapshot, &notes, false)
            .unwrap();
        let mut bytes = Vec::new();
        dump.output().read_to_end(&mut bytes).unwrap();
        dump.finish().unwrap();
        assert_eq!(Some(bytes.len() as u64), version.node.size);
    }

    // A folder as a tar archive.
    let pictures = format!("{}/Pictures", home.display());
    let mut dump = engine
        .dump(&machine, &second.snapshot_id, &pictures, true)
        .unwrap();
    let mut archive = Vec::new();
    dump.output().read_to_end(&mut archive).unwrap();
    dump.finish().unwrap();
    assert!(archive.len() > 3 << 20);

    // What a folder held.
    let listed = engine.ls(&machine, &second.snapshot_id, &pictures).unwrap();
    let mut names: Vec<&str> = listed.iter().map(|n| n.path.as_str()).collect();
    names.sort();
    assert!(names.iter().any(|n| n.ends_with("Pictures/large.raw")));
    assert!(names.iter().any(|n| n.ends_with("Pictures/Ñandú")));

    // The whole tree restored elsewhere: the same items, bytes, links and modes.
    let target = dir.path().join("restored");
    engine
        .restore(
            &person,
            &second.snapshot_id,
            &[home.display().to_string()],
            &target,
            &Stop::new(),
            |_| {},
        )
        .unwrap();
    same(&home, &target.join(home.strip_prefix("/").unwrap()));

    let stats = engine.stats(&machine).unwrap();
    assert!(stats.total_size > 0);
    engine.check(&machine, Some("100%"), &Stop::new()).unwrap();
}

#[test]
fn backups_are_thinned_as_time_machine_thins_them() {
    let dir = tempfile::tempdir().unwrap();
    let Some(engine) = engine(&dir.path().join("cache")) else {
        return;
    };
    let source = dir.path().join("source");
    fs::create_dir_all(&source).unwrap();
    let repo = repository(&dir.path().join("repo"), "machine key");
    engine.init(&repo, None).unwrap();
    // A day ago every hour; the last month once a day (several a day); three months by week.
    // In the past (restic thins only up to now), within one daylight-saving period.
    let newest = jiff::civil::datetime(2026, 10, 1, 12, 0, 0, 0);
    let mut times = Vec::new();
    for hour in 0..24 {
        times.push(newest - jiff::Span::new().hours(hour));
    }
    for day in 1..31 {
        for hour in [9, 17] {
            times.push(
                (newest - jiff::Span::new().days(day))
                    .with()
                    .hour(hour)
                    .build()
                    .unwrap(),
            );
        }
    }
    for day in (31..101).step_by(2) {
        times.push(
            (newest - jiff::Span::new().days(day))
                .with()
                .hour(10)
                .build()
                .unwrap(),
        );
    }
    for (n, time) in times.iter().enumerate() {
        fs::write(source.join("file"), n.to_string()).unwrap();
        let backup = Backup {
            paths: vec![source.clone()],
            excludes: vec![],
            host: "m".into(),
            tags: vec![],
            time: Some(time.strftime("%Y-%m-%d %H:%M:%S").to_string()),
        };
        engine.backup(&repo, &backup, &Stop::new(), |_| {}).unwrap();
    }
    let removed = engine.forget(&repo, "m", &policy::keep()).unwrap();
    assert!(!removed.is_empty());
    // Instants, as restic thins them (the times above are in restic's local time zone).
    let kept: Vec<i64> = engine
        .snapshots(&repo, Some("m"))
        .unwrap()
        .iter()
        .map(|s| s.seconds().unwrap())
        .collect();
    let newest = *kept.iter().max().unwrap();
    const HOUR: i64 = 3600;
    const DAY: i64 = 24 * HOUR;
    // Every hour of the last day.
    let last_day = kept.iter().filter(|t| **t > newest - DAY).count();
    assert_eq!(last_day, 24, "{kept:?}");
    // One a day for the month before.
    let daily = kept
        .iter()
        .filter(|t| **t <= newest - DAY && **t > newest - 31 * DAY)
        .count();
    assert!((28..=31).contains(&daily), "{daily} daily backups");
    // One a week before, as far back as they go.
    let weekly = kept.iter().filter(|t| **t <= newest - 31 * DAY).count();
    assert!((9..=11).contains(&weekly), "{weekly} weekly backups");
    assert!(*kept.iter().min().unwrap() <= newest - 93 * DAY);
}

#[test]
fn a_stopped_backup_keeps_nothing_half_made_and_the_next_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let Some(engine) = engine(&dir.path().join("cache")) else {
        return;
    };
    let source = dir.path().join("source");
    fs::create_dir_all(&source).unwrap();
    for n in 0..40 {
        fs::write(source.join(format!("f{n}")), vec![n as u8; 1 << 20]).unwrap();
    }
    let repo = repository(&dir.path().join("repo"), "k");
    engine.init(&repo, None).unwrap();
    let backup = Backup {
        paths: vec![source.clone()],
        excludes: vec![],
        host: "m".into(),
        tags: vec![],
        time: None,
    };
    let stop = Stop::new();
    stop.stop();
    assert!(matches!(
        engine.backup(&repo, &backup, &stop, |_| {}),
        Err(engine::Error::Stopped)
    ));
    let stop = Stop::new();
    let stopping = stop.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        stopping.stop();
    });
    let first = engine.backup(&repo, &backup, &stop, |_| {});
    stopper.join().unwrap();
    // Either it was stopped (no snapshot) or it had already finished.
    let snapshots = engine.snapshots(&repo, Some("m")).unwrap();
    match first {
        Err(engine::Error::Stopped) => assert!(snapshots.is_empty()),
        Ok(_) => assert_eq!(snapshots.len(), 1),
        Err(other) => panic!("{other}"),
    }
    // Its lock is gone: the next backup runs and finishes.
    engine.backup(&repo, &backup, &Stop::new(), |_| {}).unwrap();
}

// The service.

pub(crate) struct Fixture {
    pub dir: tempfile::TempDir,
    pub service: Service,
    pub volumes: PathBuf,
    pub home: PathBuf,
    pub clock: Arc<AtomicI64>,
    scheduler: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    pub fn new(engine: Engine, dir: tempfile::TempDir) -> Self {
        let volumes = dir.path().join("volumes");
        let home = dir.path().join("home");
        tree(&home);
        let clock = Arc::new(AtomicI64::new(crate::service::now()));
        let ticking = clock.clone();
        let options = Options {
            state: dir.path().join("state"),
            engine,
            disks: Arc::new(Played::open(&volumes).unwrap()),
            sources: Sources {
                paths: vec![home.clone()],
                excludes: vec![],
            },
            passwd: "/etc/passwd".into(),
            group: "/etc/group".into(),
            accounts: None,
            manifest: None,
            development: true,
            clock: Arc::new(move || ticking.load(Ordering::SeqCst)),
        };
        let service = Service::open(options, "test-computer").unwrap();
        Self {
            dir,
            service,
            volumes,
            home,
            clock,
            scheduler: None,
        }
    }

    pub fn schedule(&mut self) {
        let service = self.service.clone();
        self.scheduler = Some(std::thread::spawn(move || service.schedule()));
    }

    /// A disk connected: a folder of the played volumes.
    pub fn connect(&self, uuid: &str, label: &str) {
        let staging = self.dir.path().join(format!(".{uuid}"));
        if !staging.exists() {
            fs::create_dir_all(&staging).unwrap();
            fs::write(staging.join(".oxyos-label"), label).unwrap();
        }
        fs::rename(&staging, self.volumes.join(uuid)).unwrap();
    }

    pub fn disconnect(&self, uuid: &str) {
        fs::rename(
            self.volumes.join(uuid),
            self.dir.path().join(format!(".{uuid}")),
        )
        .unwrap();
    }

    /// A conversation with the service, as its own user.
    pub fn connect_client(&self) -> Connection {
        let (client, server) = Connection::pair().unwrap();
        let service = self.service.clone();
        std::thread::spawn(move || {
            let _ = service.serve(&server);
        });
        client.set_timeout(Duration::from_secs(60)).unwrap();
        client
    }

    pub fn status(&self) -> Status {
        self.service.status(true)
    }

    pub fn until(&self, what: &str, test: impl Fn(&Status) -> bool) -> Status {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let status = self.status();
            if test(&status) {
                return status;
            }
            assert!(Instant::now() < deadline, "never {what}: {status:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.shutdown();
        if let Some(scheduler) = self.scheduler.take() {
            let _ = scheduler.join();
        }
    }
}

fn ask(connection: &Connection, request: &str) -> Value {
    let request: Value = serde_json::from_str(request).unwrap();
    connection.send(&request).unwrap();
    loop {
        let answer: Value = connection.receive().unwrap().unwrap().unwrap();
        // A watching connection is told the state between answers.
        if answer.get("status").is_none() || request == "status" || request == "watch" {
            return answer;
        }
    }
}

#[test]
fn a_disk_added_is_backed_up_at_once_then_every_hour_and_when_it_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let Some(engine) = engine(&dir.path().join("cache")) else {
        return;
    };
    let mut fixture = Fixture::new(engine, dir);
    fixture.schedule();
    let client = fixture.connect_client();

    // Nothing to back up to.
    assert_eq!(
        ask(&client, r#"{"backUpNow":{}}"#)["refused"]["reason"],
        "noDestination"
    );
    assert_eq!(
        ask(&client, r#"{"disks":{}}"#),
        serde_json::json!({"disks": []})
    );

    fixture.connect("1234-ABCD", "Backup");
    let disks = loop {
        let disks = ask(&client, r#"{"disks":{}}"#);
        if disks["disks"].as_array().is_some_and(|d| !d.is_empty()) {
            break disks;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(disks["disks"][0]["uuid"], "1234-ABCD");
    assert_eq!(disks["disks"][0]["label"], "Backup");
    assert_eq!(disks["disks"][0]["added"], false);

    assert_eq!(
        ask(
            &client,
            r#"{"addDisk":{"uuid":"1234-ABCD","password":"short"}}"#
        )["refused"]["reason"],
        "passwordTooShort"
    );
    assert_eq!(
        ask(
            &client,
            r#"{"addDisk":{"uuid":"9999-0000","password":"long enough"}}"#
        )["refused"]["reason"],
        "notConnected"
    );
    let added = ask(
        &client,
        r#"{"addDisk":{"uuid":"1234-ABCD","password":"correct horse"}}"#,
    );
    let id = added["added"]["id"].as_str().expect("added").to_owned();
    assert_eq!(
        ask(
            &client,
            r#"{"addDisk":{"uuid":"1234-ABCD","password":"correct horse"}}"#
        )["refused"]["reason"],
        "alreadyAdded"
    );
    // The repository is in the disk's own folder, opened by the person's password.
    let backups = fixture.volumes.join("1234-ABCD/OxyOS Backups");
    assert_eq!(fs::read_dir(&backups).unwrap().count(), 1);

    // Added: backed up at once.
    let status = fixture.until("backed up", |s| {
        s.destinations[0].last_backup.is_some() && s.running.is_none()
    });
    let first = status.last_backup.unwrap();
    assert_eq!(status.destinations[0].id, id);
    assert_eq!(status.destinations[0].backups, 1);
    assert!(status.destinations[0].used_bytes.unwrap() > 0);
    assert_eq!(status.next_backup, Some(first + policy::INTERVAL));

    // Not due yet: Back Up Now is.
    assert_eq!(
        ask(&client, r#"{"backUpNow":{}}"#),
        serde_json::json!("started")
    );
    fixture.until("backed up again", |s| {
        s.running.is_none()
            && s.destinations[0].backups >= 1
            && s.destinations[0].last_failure.is_none()
    });

    // An hour later, on its own.
    let before = fixture.status().destinations[0].last_backup.unwrap();
    fixture.clock.fetch_add(policy::INTERVAL, Ordering::SeqCst);
    fixture.service.wake();
    fixture.until("the hour's backup", |s| {
        s.destinations[0].last_backup.unwrap() > before && s.running.is_none()
    });

    // The disk taken away: not available, no backup tried, nothing failed.
    fixture.disconnect("1234-ABCD");
    fixture.until("the disk gone", |s| !s.destinations[0].available);
    assert_eq!(
        ask(&client, r#"{"backUpNow":{}}"#)["refused"]["reason"],
        "notConnected"
    );
    // Three hours away; connected again: backed up at once.
    let before = fixture.status().destinations[0].last_backup.unwrap();
    fixture
        .clock
        .fetch_add(3 * policy::INTERVAL, Ordering::SeqCst);
    fixture.connect("1234-ABCD", "Backup");
    let status = fixture.until("backed up when it came back", |s| {
        s.destinations[0].last_backup.unwrap() > before && s.running.is_none()
    });
    assert!(status.destinations[0].last_failure.is_none());
}

#[test]
fn a_file_is_browsed_back_in_time_and_retrieved_from_its_own_home_only() {
    let dir = tempfile::tempdir().unwrap();
    let Some(engine) = engine(&dir.path().join("cache")) else {
        return;
    };
    let mut fixture = Fixture::new(engine, dir);
    fixture.connect("1234-ABCD", "Backup");
    fixture.schedule();
    let client = fixture.connect_client();
    ask(
        &client,
        r#"{"addDisk":{"uuid":"1234-ABCD","password":"correct horse"}}"#,
    );
    fixture.until("backed up", |s| {
        s.destinations[0].last_backup.is_some() && s.running.is_none()
    });
    let notes = fixture.home.join("Documents/notes.txt");
    std::thread::sleep(Duration::from_millis(1100));
    fs::write(&notes, "second version\n").unwrap();
    ask(&client, r#"{"backUpNow":{}}"#);
    fixture.until("backed up again", |s| {
        s.destinations[0].backups == 2 && s.running.is_none()
    });
    fs::write(&notes, "third, not backed up\n").unwrap();

    let path = notes.display().to_string();
    let versions = ask(
        &client,
        &serde_json::json!({"versions": {"path": path}}).to_string(),
    );
    let versions = versions["versions"].as_array().unwrap().clone();
    assert_eq!(versions.len(), 2, "{versions:?}");
    assert!(versions[0]["time"].as_i64() >= versions[1]["time"].as_i64());
    assert_eq!(versions[0]["size"], 15);
    assert_eq!(versions[1]["size"], 6);
    assert_eq!(versions[0]["kind"], "file");

    // The older version's bytes, through the pipe handed over.
    let request = serde_json::json!({"retrieve": {
        "destination": versions[1]["destination"], "snapshot": versions[1]["snapshot"], "path": path}});
    client.send(&request).unwrap();
    let (answer, fd) = client.receive_with_fd::<Value>().unwrap().unwrap();
    assert_eq!(
        answer.unwrap(),
        serde_json::json!({"retrieving": {"kind": "file", "size": 6}})
    );
    let mut bytes = String::new();
    fs::File::from(fd.unwrap())
        .read_to_string(&mut bytes)
        .unwrap();
    assert_eq!(bytes, "first\n");
    let done: Value = client.receive().unwrap().unwrap().unwrap();
    assert_eq!(done, serde_json::json!("retrieved"));

    // A folder: what it held, and itself as an archive.
    let documents = fixture.home.join("Documents").display().to_string();
    let contents = ask(&client, &serde_json::json!({"contents": {
        "destination": versions[0]["destination"], "snapshot": versions[0]["snapshot"], "path": documents}}).to_string());
    let mut names: Vec<String> = contents["contents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["Taxes 2026", "notes.txt", "private"]);
    client
        .send(&serde_json::json!({"retrieve": {
            "destination": versions[0]["destination"], "snapshot": versions[0]["snapshot"], "path": documents}}))
        .unwrap();
    let (answer, fd) = client.receive_with_fd::<Value>().unwrap().unwrap();
    assert_eq!(answer.unwrap()["retrieving"]["kind"], "folder");
    let mut archive = Vec::new();
    fs::File::from(fd.unwrap())
        .read_to_end(&mut archive)
        .unwrap();
    assert!(archive.windows(10).any(|w| w == b"return.pdf"));
    let done: Value = client.receive().unwrap().unwrap().unwrap();
    assert_eq!(done, serde_json::json!("retrieved"));

    // Outside what is backed up, a path that climbs, a snapshot that is not one: not found.
    for (path, snapshot) in [
        (
            "/etc/passwd".to_owned(),
            versions[0]["snapshot"].as_str().unwrap().to_owned(),
        ),
        (
            format!("{}/../state/config.json", fixture.home.display()),
            versions[0]["snapshot"].as_str().unwrap().to_owned(),
        ),
        (path.clone(), "latest".to_owned()),
    ] {
        let answer = ask(
            &client,
            &serde_json::json!({"retrieve": {
            "destination": versions[0]["destination"], "snapshot": snapshot, "path": path}})
            .to_string(),
        );
        assert_eq!(answer["refused"]["reason"], "notFound", "{path}");
        let answer = ask(
            &client,
            &serde_json::json!({"versions": {"path": path}}).to_string(),
        );
        if path
            != fixture
                .home
                .join("Documents/notes.txt")
                .display()
                .to_string()
        {
            assert_eq!(answer["refused"]["reason"], "notFound", "{path}");
        }
    }
}

#[test]
fn a_hold_keeps_scheduled_backups_back_but_not_one_asked_for() {
    let dir = tempfile::tempdir().unwrap();
    let Some(engine) = engine(&dir.path().join("cache")) else {
        return;
    };
    let mut fixture = Fixture::new(engine, dir);
    fixture.connect("1234-ABCD", "Backup");
    let client = fixture.connect_client();
    ask(
        &client,
        r#"{"addDisk":{"uuid":"1234-ABCD","password":"correct horse"}}"#,
    );
    // Low Power Mode, before the schedule starts: nothing runs on its own.
    let session = fixture.connect_client();
    assert_eq!(
        ask(&session, r#"{"hold":{"reason":"lowPower"}}"#),
        serde_json::json!("held")
    );
    fixture.schedule();
    std::thread::sleep(Duration::from_millis(500));
    let status = fixture.status();
    assert!(status.held);
    assert!(status.running.is_none() && status.last_backup.is_none());
    assert_eq!(status.next_backup, None);
    // Asked for, it runs.
    assert_eq!(
        ask(&client, r#"{"backUpNow":{}}"#),
        serde_json::json!("started")
    );
    fixture.until("backed up while held", |s| {
        s.last_backup.is_some() && s.running.is_none()
    });
    // The hold goes with its connection.
    let before = fixture.status().last_backup.unwrap();
    fixture.clock.fetch_add(policy::INTERVAL, Ordering::SeqCst);
    drop(session);
    fixture.until("the hold gone", |s| !s.held);
    fixture.until("the scheduled backup", |s| {
        s.last_backup.unwrap() > before && s.running.is_none()
    });
}

#[test]
fn only_an_administrator_changes_where_and_what_and_settings_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let Some(engine) = engine(&dir.path().join("cache")) else {
        return;
    };
    let fixture = Fixture::new(engine, dir);
    // Another user (not the development service's): refused everything but nothing more said.
    assert!(matches!(
        fixture
            .service
            .answer(u32::MAX - 1, crate::ipc::Request::Disks {}),
        Answer::Refused { .. }
    ));
    assert!(!fixture.service.administrator(u32::MAX - 1));
    let client = fixture.connect_client();
    let home = fixture.home.display().to_string();
    assert_eq!(
        ask(&client, r#"{"setExcluded":{"paths":["/etc"]}}"#)["refused"]["reason"],
        "invalidRequest"
    );
    assert_eq!(
        ask(
            &client,
            &serde_json::json!({"setExcluded": {"paths": [format!("{home}/Pictures")]}})
                .to_string()
        ),
        serde_json::json!("set")
    );
    assert_eq!(
        ask(&client, r#"{"setAutomatic":{"enabled":false}}"#),
        serde_json::json!("set")
    );
    let status = fixture.status();
    assert!(!status.automatic);
    assert_eq!(
        status.excluded,
        vec![PathBuf::from(format!("{home}/Pictures"))]
    );
    assert_eq!(
        ask(&client, r#"{"remove":{"id":"0123456789abcdef"}}"#)["refused"]["reason"],
        "notFound"
    );
    assert_eq!(
        ask(&client, r#"{"stop":{}}"#)["refused"]["reason"],
        "notRunning"
    );
    assert_eq!(
        ask(&client, r#"{"nonsense":{}}"#)["refused"]["reason"],
        "invalidRequest"
    );
}

// A share over SMB.

/// Debian's samba, serving one share on a port of the test's own, as root (smbd changes to the
/// share's user, which only root can): the Debian job of CI, in its container.
struct Samba {
    child: std::process::Child,
    /// smbd in the foreground ends when its standard input does: held open while it serves.
    _input: std::process::ChildStdin,
    port: u16,
    share: PathBuf,
}

impl Samba {
    fn start(dir: &Path, password: &str) -> Option<Self> {
        let required = std::env::var("OXYOS_BACKUP_ENGINE").as_deref() == Ok("required");
        let smbd = ["/usr/sbin/smbd", "/usr/bin/smbd"]
            .into_iter()
            .find(|p| Path::new(p).exists());
        let Some(smbd) = smbd else {
            assert!(
                !required,
                "samba is not installed, and OXYOS_BACKUP_ENGINE=required"
            );
            eprintln!("samba is not installed: the share's tests are skipped (apt install samba)");
            return None;
        };
        if !rustix::process::geteuid().is_root() {
            assert!(!required, "the share's tests run smbd, which needs root");
            eprintln!("not root: the share's tests, which run smbd, are skipped");
            return None;
        }
        let root = dir.join("samba");
        let share = dir.join("share");
        for folder in ["private", "lock", "state", "cache", "pid", "ncalrpc"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        fs::create_dir_all(&share).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let r = root.display();
        let configuration = root.join("smb.conf");
        fs::write(
            &configuration,
            format!(
                "[global]\n\
                 smb ports = {port}\n\
                 interfaces = 127.0.0.1/8\n\
                 bind interfaces only = yes\n\
                 server role = standalone server\n\
                 disable netbios = yes\n\
                 load printers = no\n\
                 printcap name = /dev/null\n\
                 map to guest = never\n\
                 private dir = {r}/private\n\
                 lock directory = {r}/lock\n\
                 state directory = {r}/state\n\
                 cache directory = {r}/cache\n\
                 pid directory = {r}/pid\n\
                 ncalrpc dir = {r}/ncalrpc\n\
                 passdb backend = tdbsam:{r}/private/passdb.tdb\n\
                 log file = {r}/log\n\
                 [Backups]\n\
                 path = {}\n\
                 read only = no\n\
                 valid users = root\n",
                share.display()
            ),
        )
        .unwrap();
        let mut smbpasswd = std::process::Command::new("smbpasswd")
            .args(["-c"])
            .arg(&configuration)
            .args(["-s", "-a", "root"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        smbpasswd
            .stdin
            .take()
            .unwrap()
            .write_all(format!("{password}\n{password}\n").as_bytes())
            .unwrap();
        assert!(smbpasswd.wait().unwrap().success());
        // A process group of its own: smbd signals its group as it ends or fails, which must
        // never reach the tests' runner.
        use std::os::unix::process::CommandExt;
        let output = fs::File::create(root.join("smbd.out")).unwrap();
        let mut child = std::process::Command::new(smbd)
            .process_group(0)
            .args([
                "--foreground",
                "--no-process-group",
                "--debug-stdout",
                "--configfile",
            ])
            .arg(&configuration)
            .stdin(std::process::Stdio::piped())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            let said = || fs::read_to_string(root.join("smbd.out")).unwrap_or_default();
            if let Ok(Some(status)) = child.try_wait() {
                panic!("smbd ended ({status}):\n{}", said());
            }
            assert!(
                Instant::now() < deadline,
                "smbd never listened:\n{}",
                said()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        Some(Self {
            child,
            _input: input,
            port,
            share,
        })
    }

    fn share(&self) -> crate::share::Share {
        crate::share::Share {
            host: "127.0.0.1".into(),
            port: Some(self.port),
            share: "Backups".into(),
            user: "root".into(),
            domain: None,
        }
    }
}

impl Drop for Samba {
    fn drop(&mut self) {
        // smbd and the children it forked for each connection: its group.
        if let Some(group) = rustix::process::Pid::from_raw(self.child.id() as i32) {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_share_is_backed_up_through_rclone_and_restored() {
    let dir = tempfile::tempdir().unwrap();
    let Some(engine) = engine(&dir.path().join("cache")) else {
        return;
    };
    let Some(samba) = Samba::start(dir.path(), "share secret") else {
        return;
    };
    // Through the engine: a repository on the share, a home backed up and restored.
    let obscured = crate::share::obscure(engine.rclone(), "share secret").unwrap();
    let repository = Repository {
        location: samba.share().location("OxyOS Backups/engine", &obscured),
        password: Zeroizing::new("machine key".into()),
    };
    engine
        .init(&repository, Some("the person's password"))
        .unwrap();
    assert!(samba.share.join("OxyOS Backups/engine/config").exists());
    let home = dir.path().join("home");
    tree(&home);
    let backup = Backup {
        paths: vec![home.clone()],
        excludes: vec![],
        host: "m".into(),
        tags: vec![],
        time: None,
    };
    let summary = engine
        .backup(&repository, &backup, &Stop::new(), |_| {})
        .unwrap();
    let target = dir.path().join("restored");
    engine
        .restore(
            &repository,
            &summary.snapshot_id,
            &[home.display().to_string()],
            &target,
            &Stop::new(),
            |_| {},
        )
        .unwrap();
    same(&home, &target.join(home.strip_prefix("/").unwrap()));

    // Through the service: added, backed up; a wrong password and a share that is not there
    // said as such.
    let mut fixture = Fixture::new(engine, tempfile::tempdir().unwrap());
    fixture.schedule();
    let client = fixture.connect_client();
    let share = serde_json::to_value(samba.share()).unwrap();
    let wrong = ask(&client, &serde_json::json!({"addShare": {"share": share, "sharePassword": "nope", "password": "correct horse"}}).to_string());
    assert_eq!(wrong["refused"]["reason"], "shareRefused", "{wrong}");
    let mut elsewhere = samba.share();
    elsewhere.port = Some(1);
    let elsewhere = serde_json::to_value(elsewhere).unwrap();
    let absent = ask(&client, &serde_json::json!({"addShare": {"share": elsewhere, "sharePassword": "share secret", "password": "correct horse"}}).to_string());
    assert_eq!(absent["refused"]["reason"], "unreachable", "{absent}");
    let added = ask(&client, &serde_json::json!({"addShare": {"share": share, "sharePassword": "share secret", "password": "correct horse"}}).to_string());
    assert!(added["added"]["id"].is_string(), "{added}");
    let status = fixture.until("backed up to the share", |s| {
        s.destinations
            .first()
            .is_some_and(|d| d.last_backup.is_some())
            && s.running.is_none()
    });
    assert_eq!(status.destinations[0].kind, "share");
    assert_eq!(
        status.destinations[0].url.as_deref(),
        Some(format!("smb://127.0.0.1:{}/Backups", samba.port).as_str())
    );
}
