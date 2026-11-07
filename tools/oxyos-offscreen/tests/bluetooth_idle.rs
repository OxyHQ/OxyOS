//! Following BlueZ wakes for its signals that can change what is shown and the order to stop,
//! never a clock (docs/adr/0050-bluetooth.mdx, docs/performance.mdx's idle budget): the session's
//! connection to the played BlueZ, while BlueZ signals only what no session follows (a stream's
//! volume as it plays), leaves the process's threads together waking at most once a second and
//! using at most 0.1 % of a core, those signals never delivered; a device's signal strength
//! (what a discovery signals, delivered with its device's properties) tells the session nothing; a
//! battery's change is told; and the order to stop ends it at once.
//!
//! One test in a binary of its own: it counts every thread of the process.

mod bluetooth;

use bluetooth::{Played, Session, MACHINE};
use std::time::Duration;

/// Every thread's context switches but the calling one's, by thread, and the process's CPU time
/// in ns.
fn sample() -> (std::collections::BTreeMap<String, u64>, u64) {
    let me = std::fs::read_link("/proc/thread-self")
        .ok()
        .and_then(|link| link.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap();
    let mut switches = std::collections::BTreeMap::new();
    for task in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
        if task.file_name().to_string_lossy() == me {
            continue;
        }
        let Ok(status) = std::fs::read_to_string(task.path().join("status")) else {
            continue;
        };
        let name = status
            .lines()
            .find_map(|line| line.strip_prefix("Name:"))
            .unwrap_or("")
            .trim();
        let count: u64 = status
            .lines()
            .filter(|line| line.contains("ctxt_switches"))
            .filter_map(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
            .sum();
        switches.insert(
            format!("{name} {}", task.file_name().to_string_lossy()),
            count,
        );
    }
    let cpu = std::fs::read_to_string("/proc/self/schedstat")
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    (switches, cpu)
}

#[test]
fn a_quiet_bluez_followed_wakes_nothing() {
    let played = Played::start("idle", MACHINE);
    let mut session = Session::start(&played);
    played.until("the agent", |s| s["agent"]["default"] == true);
    std::thread::sleep(Duration::from_millis(500));
    while session.events.try_recv().is_ok() {}

    let (switches, cpu) = sample();
    let measured = Duration::from_secs(3);
    // What BlueZ says of the AirPods' stream as music plays: never delivered.
    for volume in [90, 100, 110] {
        played.act(&format!(
            r#"{{"transport_volume": {{"address": "AC:90:85:11:22:33", "volume": {volume}}}}}"#
        ));
    }
    std::thread::sleep(measured);
    let (switches_after, cpu_after) = sample();
    let woke: Vec<(String, u64)> = switches_after
        .iter()
        .map(|(thread, after)| {
            (
                thread.clone(),
                after - switches.get(thread).copied().unwrap_or(*after),
            )
        })
        .filter(|(_, woke)| *woke > 0)
        .collect();
    let wakeups = woke.iter().map(|(_, woke)| woke).sum::<u64>() as f64 / measured.as_secs_f64();
    let share = (cpu_after - cpu) as f64 / measured.as_nanos() as f64 * 100.0;
    assert!(
        wakeups <= 1.0,
        "{wakeups} wakeups a second following a quiet BlueZ: {woke:?}"
    );
    assert!(share <= 0.1, "{share} % of a core following a quiet BlueZ");
    // A device's signal strength, twice: delivered (its device's properties are followed) but
    // nothing shown changes, so nothing is told.
    played.act(r#"{"rssi": {"address": "AC:90:85:11:22:33", "rssi": -40}}"#);
    played.act(r#"{"rssi": {"address": "AC:90:85:11:22:33", "rssi": -72}}"#);
    std::thread::sleep(Duration::from_millis(300));
    let told: Vec<_> = session.events.try_iter().collect();
    assert!(told.is_empty(), "told {told:?} of what is not shown");

    played.act(r#"{"battery": {"address": "AC:90:85:11:22:33", "percent": 75}}"#);
    session.until_state("the battery", |s| {
        s.device("AC:90:85:11:22:33")
            .is_some_and(|d| d.battery == Some(75))
    });
    assert!(session.stop() < Duration::from_secs(1));
}
