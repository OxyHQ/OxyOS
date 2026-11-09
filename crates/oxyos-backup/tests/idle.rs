//! The service wakes for nothing while it waits (docs/adr/0059-backups.mdx, decision 4;
//! docs/performance.mdx's idle budget): with a destination whose disk is connected and whose
//! backup is not due for an hour, its threads together wake no more than once over two seconds,
//! and its disk taken away is told at once, never found by a poll.
//!
//! One test in a binary of its own: it counts every thread of the process.

use oxyos_backup::disks::Played;
use oxyos_backup::engine::Engine;
use oxyos_backup::policy::Sources;
use oxyos_backup::service::{now, Options, Service};
use oxyos_backup::store::{Config, Destination, Keys, Kind, Record, Store};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Every thread's context switches but the calling one's.
fn switches() -> BTreeMap<String, u64> {
    let me = std::fs::read_link("/proc/thread-self")
        .ok()
        .and_then(|link| link.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap();
    let mut switches = BTreeMap::new();
    for task in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
        let id = task.file_name().to_string_lossy().into_owned();
        if id == me {
            continue;
        }
        let Ok(status) = std::fs::read_to_string(task.path().join("status")) else {
            continue;
        };
        let count: u64 = status
            .lines()
            .filter(|line| line.contains("ctxt_switches"))
            .filter_map(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
            .sum();
        switches.insert(id, count);
    }
    switches
}

#[test]
fn a_waiting_service_wakes_for_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let volumes = dir.path().join("volumes");
    let source = dir.path().join("home");
    std::fs::create_dir_all(volumes.join("1234-ABCD")).unwrap();
    std::fs::create_dir_all(&source).unwrap();
    // A destination backed up a minute ago: its next backup is in 59 minutes.
    let store = Store::open(&state).unwrap();
    let id = "0123456789abcdef".to_owned();
    store
        .put_config(&Config {
            destinations: vec![Destination {
                id: id.clone(),
                name: "Backup".into(),
                kind: Kind::Disk {
                    uuid: "1234-ABCD".into(),
                    fs_type: "ext4".into(),
                },
                folder: "f".into(),
            }],
            computer: "idle".into(),
            ..Config::default()
        })
        .unwrap();
    store.put_keys(&id, &Keys::generate(None).unwrap()).unwrap();
    let last = now() - 60;
    store
        .put_records(&BTreeMap::from([(
            id.clone(),
            Record {
                last_backup: Some(last),
                ..Record::default()
            },
        )]))
        .unwrap();
    let service = Service::open(
        Options {
            state,
            engine: Engine::new(&dir.path().join("cache")),
            disks: Arc::new(Played::open(&volumes).unwrap()),
            sources: Sources {
                paths: vec![source],
                excludes: vec![],
            },
            passwd: "/etc/passwd".into(),
            group: "/etc/group".into(),
            accounts: None,
            manifest: None,
            development: true,
            clock: Arc::new(now),
        },
        "idle",
    )
    .unwrap();
    let scheduler = service.clone();
    let thread = std::thread::spawn(move || scheduler.schedule());
    std::thread::sleep(Duration::from_millis(500));
    let status = service.status(true);
    assert!(status.running.is_none());
    assert_eq!(status.next_backup, Some(last + 3600));

    let before = switches();
    std::thread::sleep(Duration::from_secs(2));
    let after = switches();
    let woke: u64 = after
        .iter()
        .map(|(thread, n)| n - before.get(thread).copied().unwrap_or(*n))
        .sum();
    assert!(
        woke <= 1,
        "the service woke {woke} times while waiting: {after:?}"
    );

    // The disk taken away is told at once, by inotify (udev's monitor on a machine).
    std::fs::rename(volumes.join("1234-ABCD"), dir.path().join("away")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.status(true).destinations[0].available {
        assert!(Instant::now() < deadline, "the disk was never seen to go");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(service.status(true).next_backup, None);
    service.shutdown();
    thread.join().unwrap();
}
