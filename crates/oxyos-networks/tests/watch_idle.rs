//! Following NetworkManager wakes for its signals that change what is shown and the order to stop,
//! never a clock: watching a played daemon (`oxyos-fake-networkmanager`, on a bus of the test's
//! own) that says nothing a session shows (another access point's strength, the joined one seen
//! again), the process's threads together wake at most once a second and use at most 0.1 % of a
//! core (the idle budget, docs/performance.mdx); a change is still told, and the order to stop
//! ends it at once.
//!
//! One test in a binary of its own: it counts every thread of the process.

use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsFd;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Every thread's context switches but the calling one's, by thread, and the process's CPU time
/// in ns.
fn sample() -> (std::collections::BTreeMap<String, u64>, u64) {
    let me = std::fs::read_link("/proc/thread-self")
        .ok()
        .and_then(|link| {
            link.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
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

/// The bus and the played daemon, ended however the test ends.
struct Children(Vec<std::process::Child>);

impl Drop for Children {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn a_quiet_daemon_followed_wakes_nothing() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-networks-idle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
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
    let machine = directory.join("machine.json");
    std::fs::write(
        &machine,
        r#"{"access_points": [{"ssid": "Home", "strength": 70, "security": "wpa", "password": "correct horse"},
            {"ssid": "Café Libre", "strength": 55}],
            "connections": [{"ssid": "Home", "password": "correct horse"}], "active": "Home",
            "ethernet": "unplugged"}"#,
    )
    .unwrap();
    let fake = Command::new(env!("CARGO_BIN_EXE_oxyos-fake-networkmanager"))
        .args(["--address", &address, "--machine"])
        .arg(&machine)
        .arg("--state")
        .arg(directory.join("state.json"))
        .spawn()
        .unwrap();
    let _children = Children(vec![bus, fake]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !oxyos_networks::NetworkManager::connect(Some(&address))
        .and_then(|manager| manager.state())
        .is_ok_and(|state| state.available)
    {
        assert!(Instant::now() < deadline, "the played daemon did not start");
        std::thread::sleep(Duration::from_millis(20));
    }

    let (stop, mut order) = std::os::unix::net::UnixStream::pair().unwrap();
    let (told, heard) = std::sync::mpsc::channel();
    let watching = {
        let address = address.clone();
        std::thread::spawn(move || {
            oxyos_networks::watch(
                Some(&address),
                Duration::from_millis(30),
                stop.as_fd(),
                |_| {
                    // The access point joined, as a read says it: its strength is followed.
                    let point = oxyos_networks::NetworkManager::connect(Some(&address))
                        .and_then(|manager| manager.state())
                        .ok()
                        .and_then(|state| state.wifi.point);
                    if told.send(point.is_some()).is_ok() {
                        oxyos_networks::Follow::Point(point)
                    } else {
                        oxyos_networks::Follow::Stop
                    }
                },
            )
        })
    };
    assert!(
        heard
            .recv_timeout(Duration::from_secs(5))
            .expect("told at start"),
        "an access point is joined"
    );
    std::thread::sleep(Duration::from_millis(500));
    while heard.try_recv().is_ok() {}

    let act = |what: &str| {
        let status = Command::new("busctl")
            .arg(format!("--address={address}"))
            .args([
                "call",
                "org.freedesktop.NetworkManager",
                "/so/oxy/OxyOS/FakeNetworkManager",
                "so.oxy.OxyOS.FakeNetworkManager",
                "Act",
                "s",
                what,
            ])
            .status()
            .unwrap();
        assert!(status.success());
    };
    let (switches, cpu) = sample();
    let measured = Duration::from_secs(3);
    // What a session never shows is never delivered: another access point's strength, and the
    // joined one seen again by a scan.
    act(r#"{"strength": {"ssid": "Café Libre", "strength": 90}}"#);
    act(r#"{"seen": "Home"}"#);
    std::thread::sleep(measured);
    let (switches_after, cpu_after) = sample();
    // Each thread's, a thread that ended counted as none.
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
        "{wakeups} wakeups a second following a quiet daemon: {woke:?}"
    );
    assert!(share <= 0.1, "{share} % of a core following a quiet daemon");

    act(r#"{"ethernet": "plugged"}"#);
    heard
        .recv_timeout(Duration::from_secs(5))
        .expect("told of the cable");

    let stopping = Instant::now();
    order.write_all(&[1]).unwrap();
    watching.join().unwrap().unwrap();
    assert!(stopping.elapsed() < Duration::from_secs(1));
    let _ = std::fs::remove_dir_all(&directory);
}
