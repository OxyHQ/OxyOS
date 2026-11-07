//! Following PipeWire wakes for what PipeWire tells and the order to stop, never a clock: a client
//! following a quiet daemon of the test's own (`common::Daemon`) wakes, with every thread of the
//! process together, at most once a second and uses at most 0.1 % of a core (the idle budget,
//! docs/performance.mdx) while nothing changes and nothing is metered; a device plugged in is
//! still told, and the order to stop ends it at once.
//!
//! One test in a binary of its own: it counts every thread of the process.

mod common;

use common::{null_node, Daemon};
use oxyos_audio::{Audio, Event, PipeWire};
use std::collections::BTreeMap;
use std::io::Write;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

/// Every thread's context switches but the calling one's, by thread, and the process's CPU time
/// in ns.
fn sample() -> (BTreeMap<String, u64>, u64) {
    let me = std::fs::read_link("/proc/thread-self")
        .ok()
        .and_then(|link| link.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap();
    let mut switches = BTreeMap::new();
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
            .trim()
            .to_string();
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
fn a_quiet_pipewire_followed_wakes_nothing() {
    let daemon = Daemon::start(
        "idle",
        &[
            null_node("speakers", "Speakers", "Audio/Sink", ""),
            null_node("mic", "Microphone", "Audio/Source", ""),
        ],
    );
    daemon.configure("speakers", true);
    daemon.configure("mic", false);
    daemon.set_metadata("default.audio.sink", r#"{"name":"speakers"}"#);
    daemon.set_metadata("default.audio.source", r#"{"name":"mic"}"#);

    let audio = PipeWire::new().unwrap();
    let (stop, mut order) = std::os::unix::net::UnixStream::pair().unwrap();
    let following = {
        let audio = audio.clone();
        std::thread::spawn(move || audio.follow(stop.as_fd()))
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while audio.state().output.selected().is_none() {
        assert!(Instant::now() < deadline, "PipeWire not read");
        std::thread::sleep(Duration::from_millis(20));
    }
    audio.take_events();
    // Settled, then five quiet seconds measured.
    std::thread::sleep(Duration::from_millis(500));
    let (before, cpu_before) = sample();
    let measured = Duration::from_secs(5);
    std::thread::sleep(measured);
    let (after, cpu_after) = sample();
    let woke: BTreeMap<&String, u64> = after
        .iter()
        .map(|(thread, count)| (thread, count - before.get(thread).copied().unwrap_or(0)))
        .filter(|(_, woke)| *woke > 0)
        .collect();
    let total: u64 = woke.values().sum();
    assert!(
        total <= measured.as_secs(),
        "the threads woke {total} times in {measured:?} with nothing changing: {woke:?}"
    );
    let cpu = (cpu_after - cpu_before) as f64 / measured.as_nanos() as f64;
    assert!(cpu <= 0.001, "{:.3} % of a core while idle", cpu * 100.0);
    assert!(
        audio.take_events().is_empty(),
        "nothing changed, nothing told"
    );

    // A device plugged in is told.
    daemon.cli(&[
        "create-node",
        "adapter",
        &null_node("usb", "USB Headset", "Audio/Sink", "device.bus = usb"),
    ]);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if audio.take_events().iter().any(|event| {
            matches!(event, Event::State(state) if state.output.devices.iter().any(|d| d.id == "usb"))
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the device plugged in was not told"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // The order to stop ends it at once.
    let stopping = Instant::now();
    order.write_all(&[1]).unwrap();
    following.join().unwrap().unwrap();
    assert!(stopping.elapsed() < Duration::from_secs(1));
    assert!(!audio.state().available);
}
