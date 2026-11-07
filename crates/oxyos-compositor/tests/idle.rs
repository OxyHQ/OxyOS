//! A session doing nothing wakes for nothing (docs/performance.mdx, *Idle*): the parts that follow
//! the system for it (the install service's update state, NetworkManager, BlueZ
//! (docs/adr/0055-bluetooth.mdx), the hardware controls' bus and backlights, and the sound's
//! PipeWire, docs/adr/0056-sound-devices.mdx) wait on their sources and on the order to stop,
//! never on a clock.
//! Here every source is missing, as on a machine where none has started yet: the process's
//! threads together may wake at most once a second and use at most 0.1 % of a core, as
//! `perf.py`'s idle budget holds an application to; a service's socket made is then seen at once,
//! and everything stops at once when told.
//!
//! One test in a binary of its own: it counts every thread of the process, and sets the
//! environment before any thread reads it.

use oxyos_compositor::bluetooth::Bluetooth;
use oxyos_compositor::controls::Controls;
use oxyos_compositor::installs::{Asker, Installs};
use oxyos_compositor::networks::Networks;
use oxyos_compositor::sound_devices::SoundDevices;
use oxyos_compositor_core::Scene;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Every thread's context switches but the calling one's, by thread, and the process's CPU time
/// in ns.
fn sample() -> (std::collections::BTreeMap<String, u64>, u64) {
    let me = rustix::thread::gettid().as_raw_nonzero().get().to_string();
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

fn directory(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oxyos-idle-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Whether `listener` (the install service) is connected to within `within` (a deadline that only
/// says it never was); the connection is closed at once, as a service that goes.
fn accepted(listener: oxyos_install::socket::Listener, within: Duration) -> bool {
    let (accepted, heard) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = accepted.send(listener.accept().is_ok());
    });
    heard.recv_timeout(within).unwrap_or(false)
}

/// Once every thread started and waits, the wakeups a second of every thread together (and which
/// woke, by thread), and the share of a core the process used, over `measured`.
fn idle_for(measured: Duration) -> (f64, f64, Vec<(String, u64)>) {
    std::thread::sleep(Duration::from_millis(500));
    let (switches, cpu) = sample();
    std::thread::sleep(measured);
    let (switches_after, cpu_after) = sample();
    // Each thread's; one that started meanwhile counts from none.
    let woke: Vec<(String, u64)> = switches_after
        .iter()
        .map(|(thread, after)| {
            (
                thread.clone(),
                after - switches.get(thread).copied().unwrap_or(0),
            )
        })
        .filter(|(_, woke)| *woke > 0)
        .collect();
    let wakeups = woke.iter().map(|(_, woke)| woke).sum::<u64>() as f64 / measured.as_secs_f64();
    let share = (cpu_after - cpu) as f64 / measured.as_nanos() as f64 * 100.0;
    (wakeups, share, woke)
}

/// The same parts on this machine's own sources: its system bus (NetworkManager, UPower, BlueZ),
/// PipeWire, its backlights and the install service's socket. Run by hand on a real machine, with
/// nothing happening on it (`cargo test -p oxyos-compositor --test idle -- --ignored`); the
/// machine's own services may say something meanwhile, which is not the parts' doing.
#[test]
#[ignore = "this machine's own bus, PipeWire and backlights: run by hand"]
fn on_this_machine_the_parts_do_not_wake_while_nothing_happens() {
    let mut scene = Scene::new();
    let mut installs = Installs::new(PathBuf::from(oxyos_install::ipc::SOCKET), None).unwrap();
    installs.serve(&mut scene, &|_| Asker::Unknown);
    let mut networks = Networks::on_bus(None).unwrap();
    networks.serve(&mut scene);
    let mut bluetooth = Bluetooth::on_bus(None).unwrap();
    bluetooth.serve(&mut scene);
    // The session's own sound and controls, reading the machine as a session does.
    let (mut sound, pipewire) = SoundDevices::pipewire().unwrap();
    sound.serve(&mut scene, None);
    let system = oxyos_controls::Session::new(pipewire);
    let mut controls = Controls::new(oxyos_controls::Worker::spawn(Box::new(system)).unwrap());
    controls.follow(false, PathBuf::from("/sys")).unwrap();
    // What they read as they start (NetworkManager's state, the first answers) settles first.
    std::thread::sleep(Duration::from_secs(2));
    let (wakeups, share, woke) = idle_for(Duration::from_secs(20));
    println!("{wakeups:.2} wakeups a second, {share:.3} % of a core: {woke:?}");
    assert!(
        wakeups <= 1.0,
        "{wakeups} wakeups a second while nothing happens: {woke:?}"
    );
    assert!(share <= 0.1, "{share} % of a core while nothing happens");
    let stopping = Instant::now();
    drop(controls);
    drop(sound);
    drop(bluetooth);
    drop(networks);
    drop(installs);
    assert!(
        stopping.elapsed() < Duration::from_secs(1),
        "{:?} to stop",
        stopping.elapsed()
    );
}

#[test]
fn the_parts_following_the_system_do_not_wake_while_nothing_happens() {
    let root = directory("parts");
    let runtime = root.join("run");
    std::fs::create_dir_all(&runtime).unwrap();
    // No system bus, no PipeWire, no backlight: their sockets and devices are not there yet.
    std::env::set_var(
        "DBUS_SYSTEM_BUS_ADDRESS",
        format!(
            "unix:path={}",
            runtime.join("dbus/system_bus_socket").display()
        ),
    );
    std::env::set_var("XDG_RUNTIME_DIR", &runtime);
    std::env::remove_var("PIPEWIRE_RUNTIME_DIR");
    std::env::remove_var("PIPEWIRE_REMOTE");
    let sysfs = root.join("sys");
    std::fs::create_dir_all(sysfs.join("class/backlight")).unwrap();
    let installd = root.join("run/oxyos/installd/socket");

    let mut scene = Scene::new();
    let mut installs = Installs::new(installd.clone(), None).unwrap();
    installs.serve(&mut scene, &|_| Asker::Unknown);
    let mut networks = Networks::on_bus(None).unwrap();
    networks.serve(&mut scene);
    let mut bluetooth = Bluetooth::on_bus(None).unwrap();
    bluetooth.serve(&mut scene);
    let fake = oxyos_controls::Fake::parse("battery = discharging\nbattery_percent = 50").unwrap();
    let mut controls = Controls::new(oxyos_controls::Worker::spawn(Box::new(fake)).unwrap());
    controls.follow(true, sysfs).unwrap();
    // PipeWire's socket is not there: the sound waits for it to be made.
    let (mut sound, _pipewire) = SoundDevices::pipewire().unwrap();
    sound.serve(&mut scene, Some(&mut controls));

    let (wakeups, share, woke) = idle_for(Duration::from_secs(3));
    assert!(
        wakeups <= 1.0,
        "{wakeups} wakeups a second while nothing happens: {woke:?}"
    );
    assert!(share <= 0.1, "{share} % of a core while nothing happens");

    // The install service starting is seen as its socket is made, below the directories it makes
    // as it starts; one that goes (its connection closed, its socket left there) is followed again
    // once its socket is made anew, whenever the watcher looks.
    std::fs::create_dir_all(installd.parent().unwrap()).unwrap();
    let first = oxyos_install::socket::Listener::bind(&installd, 0o600).unwrap();
    assert!(
        accepted(first, Duration::from_secs(10)),
        "the install service was not followed once its socket was made"
    );
    let again = oxyos_install::socket::Listener::bind(&installd, 0o600).unwrap();
    assert!(
        accepted(again, Duration::from_secs(10)),
        "the install service was not followed again once its socket was made anew"
    );

    // Told to stop, every part's threads end at once.
    let stopping = Instant::now();
    drop(sound);
    drop(controls);
    drop(bluetooth);
    drop(networks);
    drop(installs);
    assert!(
        stopping.elapsed() < Duration::from_secs(1),
        "{:?} to stop",
        stopping.elapsed()
    );
    let _ = std::fs::remove_dir_all(Path::new(&root));
}
