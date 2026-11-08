use super::*;
use crate::energy::{Meter, Proc, Read, Running, Usage};
use crate::history::{Charge, Event, History, Record};
use crate::played::{Machine, Played};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("oxyos-power-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

// ---- The history ----

const HOUR: u64 = 3600;
/// Noon of 6 November 2026, UTC.
const NOON: u64 = 1_793_966_400;

fn at(at: u64, event: Event) -> Record {
    Record { at, event }
}

#[test]
fn the_history_is_a_line_per_change_and_what_cannot_be_read_is_skipped() {
    let records = history::parse(
        "1000 o 1\n1000 s 1\n1010 b 64 d\nnonsense\n1020 b 101 d\n1030 b 63 x\n1040 z 1\n1050 z 0\n1060 s 0\n1070 o 0\n",
    );
    assert_eq!(
        records,
        vec![
            at(1000, Event::Session(true)),
            at(1000, Event::Display(true)),
            at(1010, Event::Battery(64, Charge::Discharging)),
            at(1040, Event::Asleep(true)),
            at(1050, Event::Asleep(false)),
            at(1060, Event::Display(false)),
            at(1070, Event::Session(false)),
        ]
    );
    assert_eq!(
        history::path(Some(Path::new("/state")), Some(Path::new("/home/ana"))),
        Some(PathBuf::from("/state/oxyos/battery-history"))
    );
    assert_eq!(
        history::path(None, Some(Path::new("/home/ana"))),
        Some(PathBuf::from(
            "/home/ana/.local/state/oxyos/battery-history"
        ))
    );
}

#[test]
fn the_charts_say_the_charge_at_each_bins_end_and_nothing_where_no_session_ran() {
    let start = NOON - 3 * HOUR;
    let records = vec![
        at(start, Event::Session(true)),
        at(start, Event::Display(true)),
        at(start, Event::Battery(80, Charge::Discharging)),
        at(start + HOUR, Event::Battery(70, Charge::Discharging)),
        // Plugged in at 11:00 for half an hour.
        at(start + 2 * HOUR, Event::Battery(60, Charge::Charging)),
        at(start + 2 * HOUR + 1800, Event::Battery(75, Charge::Plugged)),
    ];
    let charts = history::charts(&records, NOON);
    let day = &charts.day;
    assert_eq!((day.end, day.bin, day.levels.len()), (NOON, 900, 96));
    // The last three hours are the twelve last quarters; before them, no session ran.
    assert!(day.levels[..84].iter().all(Option::is_none));
    assert_eq!(
        &day.levels[84..],
        &[80, 80, 80, 70, 70, 70, 70, 60, 60, 75, 75, 75].map(Some)
    );
    assert_eq!(
        &day.charging[84..],
        &[false, false, false, false, false, false, false, false, true, true, false, false]
    );
    // The display was on all the while.
    assert!(day.screen[84..].iter().all(|seconds| *seconds == 900));
    assert!(day.screen[..84].iter().all(|seconds| *seconds == 0));
    let days = &charts.days;
    assert_eq!((days.end, days.bin, days.levels.len()), (NOON, 3600, 240));
    assert_eq!(&days.levels[237..], &[Some(70), Some(60), Some(75)]);
    assert_eq!(&days.screen[237..], &[3600, 3600, 3600]);
}

#[test]
fn the_display_off_asleep_or_a_session_that_ended_unseen_is_no_use_of_the_screen() {
    let start = NOON - 4 * HOUR;
    let records = vec![
        at(start, Event::Session(true)),
        at(start, Event::Display(true)),
        at(start, Event::Battery(90, Charge::Discharging)),
        // Off a quarter of an hour into its first hour, on again half an hour in.
        at(start + 900, Event::Display(false)),
        at(start + 1800, Event::Display(true)),
        // Asleep its second hour, the display off with it.
        at(start + HOUR, Event::Asleep(true)),
        at(start + 2 * HOUR, Event::Asleep(false)),
        at(start + 2 * HOUR, Event::Display(true)),
        // Its last record, then nothing until a session started an hour later (it ended unseen).
        at(
            start + 2 * HOUR + 600,
            Event::Battery(85, Charge::Discharging),
        ),
        at(start + 3 * HOUR, Event::Session(true)),
        at(start + 3 * HOUR, Event::Display(true)),
        at(start + 3 * HOUR, Event::Battery(84, Charge::Discharging)),
    ];
    let days = history::charts(&records, NOON).days;
    assert_eq!(&days.screen[236..], &[900 + 1800, 0, 600, 3600]);
    // Between the session that ended unseen and the next, no charge is known.
    assert_eq!(
        &days.levels[236..],
        &[Some(90), Some(90), Some(85), Some(84)]
    );
    let day = history::charts(&records, NOON).day;
    // 14:10 to 15:00 no session ran.
    assert_eq!(&day.levels[88..92], &[Some(85), None, None, None]);
}

#[test]
fn the_history_file_keeps_ten_days_and_is_appended_to() {
    let directory = scratch("history");
    let path = directory.join("oxyos").join("battery-history");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let old = NOON - 11 * 24 * HOUR;
    std::fs::write(
        &path,
        format!(
            "{old} o 1\n{old} b 50 d\n{} o 1\n{} b 64 d\n",
            NOON - HOUR,
            NOON - HOUR
        ),
    )
    .unwrap();
    let mut history = History::open(Some(path.clone()), NOON);
    // What is older than ten days is gone from the file, written anew.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("{} o 1\n{} b 64 d\n", NOON - HOUR, NOON - HOUR)
    );
    history.record(NOON, Event::Session(true));
    history.record(NOON, Event::Battery(64, Charge::Discharging));
    // The same battery again records nothing.
    history.record(NOON + 10, Event::Battery(64, Charge::Discharging));
    history.record(NOON + 20, Event::Battery(63, Charge::Discharging));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!(
            "{} o 1\n{} b 64 d\n{NOON} o 1\n{NOON} b 64 d\n{} b 63 d\n",
            NOON - HOUR,
            NOON - HOUR,
            NOON + 20
        )
    );
    assert_eq!(history.records().len(), 5);
    // A file that cannot be written is kept for the session alone.
    let mut kept = History::open(Some(PathBuf::from("/proc/oxyos/battery-history")), NOON);
    kept.record(NOON, Event::Display(true));
    assert_eq!(kept.records().len(), 1);
    assert_eq!(kept.path(), None);
}

// ---- Energy ----

/// A process in a `/proc` of the test's own: its stat (times in ticks of 100 a second, its start
/// at `start` seconds since boot), its children and its DRM descriptors' fdinfo.
fn process(proc: &Path, pid: u32, ticks: (u64, u64, u64, u64), start: u64, children: &[u32]) {
    let directory = proc.join(pid.to_string());
    std::fs::create_dir_all(directory.join("task").join(pid.to_string())).unwrap();
    std::fs::create_dir_all(directory.join("fd")).unwrap();
    std::fs::create_dir_all(directory.join("fdinfo")).unwrap();
    let (utime, stime, cutime, cstime) = ticks;
    // The name holds a space and a parenthesis, as a process may name itself.
    std::fs::write(
        directory.join("stat"),
        format!(
            "{pid} (oxyos (host) x) S 1 1 1 0 -1 4194304 100 0 0 0 {utime} {stime} {cutime} {cstime} 20 0 4 0 {} 1000 100\n",
            start * 100
        ),
    )
    .unwrap();
    std::fs::write(
        directory
            .join("task")
            .join(pid.to_string())
            .join("children"),
        children
            .iter()
            .map(|child| format!("{child} "))
            .collect::<String>(),
    )
    .unwrap();
}

fn drm(proc: &Path, pid: u32, fd: u32, client: u32, render_ns: u64) {
    let directory = proc.join(pid.to_string());
    std::os::unix::fs::symlink(
        "/dev/dri/renderD128",
        directory.join("fd").join(fd.to_string()),
    )
    .unwrap();
    std::fs::write(
        directory.join("fdinfo").join(fd.to_string()),
        format!(
            "pos:\t0\nflags:\t02100002\ndrm-driver:\ti915\ndrm-pdev:\t0000:00:02.0\ndrm-client-id:\t{client}\ndrm-engine-render:\t{render_ns} ns\ndrm-engine-copy:\t1000000 ns\ndrm-engine-capacity-video:\t2\n"
        ),
    )
    .unwrap();
}

#[test]
fn an_applications_energy_is_its_processes_processor_and_gpu_time() {
    let proc = scratch("proc");
    std::fs::write(proc.join("uptime"), "1000.00 4000.00\n").unwrap();
    // Its process (200 ticks of its own, 100 of a child it waited for) and a child still running
    // (50), started at 900 s.
    process(&proc, 4242, (150, 50, 80, 20), 900, &[4243]);
    process(&proc, 4243, (40, 10, 0, 0), 950, &[]);
    // The GPU: the same client open twice is counted once; another client of the child's too.
    drm(&proc, 4242, 7, 11, 2_000_000_000);
    drm(&proc, 4242, 8, 11, 2_000_000_000);
    drm(&proc, 4243, 3, 12, 500_000_000);
    // Not a DRM device: no fdinfo read.
    std::os::unix::fs::symlink("/dev/null", proc.join("4242").join("fd").join("1")).unwrap();
    let reader = Proc {
        root: &proc,
        ticks: 100,
    };
    let (usage, age) = reader.usage(4242, reader.uptime_ns().unwrap()).unwrap();
    assert_eq!(
        usage,
        Usage {
            cpu_ns: 3_500_000_000,
            gpu_ns: 2_001_000_000 + 501_000_000,
        }
    );
    assert_eq!(age, Duration::from_secs(100));
    // Gone: nothing.
    assert_eq!(reader.usage(9999, 0), None);
}

fn read(pid: u32, cpu_s: f64, gpu_s: f64, age_s: u64) -> Read {
    Read {
        pid,
        usage: Usage {
            cpu_ns: (cpu_s * 1e9) as u64,
            gpu_ns: (gpu_s * 1e9) as u64,
        },
        age: Duration::from_secs(age_s),
    }
}

#[test]
fn an_application_uses_significant_energy_at_a_fifth_of_a_core_over_a_minute() {
    let running = vec![
        Running {
            pid: 10,
            id: "so.oxy.inbox".into(),
            name: "Inbox".into(),
        },
        Running {
            pid: 20,
            id: "so.oxy.allo".into(),
            name: "Allo".into(),
        },
    ];
    let mut meter = Meter::default();
    let t0 = Duration::from_secs(1000);
    // Just started: its use since it started (Inbox: 3 s in 10 s; Allo 0.5 s in 10 s).
    meter.sample(
        t0,
        &running,
        &[read(10, 2.0, 1.0, 10), read(20, 0.5, 0.0, 10)],
    );
    let report = meter.report(t0);
    assert_eq!(report[0].id, "so.oxy.inbox");
    assert!((report[0].recent - 0.3).abs() < 1e-9 && report[0].significant);
    assert!((report[1].recent - 0.05).abs() < 1e-9 && !report[1].significant);
    // Two minutes later (a battery change): Inbox used 6 s more (a twentieth of a core: no
    // longer significant), Allo 60 s (half a core: significant).
    let t1 = t0 + Duration::from_secs(120);
    meter.sample(
        t1,
        &running,
        &[read(10, 8.0, 1.0, 130), read(20, 60.5, 0.0, 130)],
    );
    let report = meter.report(t1);
    assert_eq!(report[0].id, "so.oxy.allo");
    assert!((report[0].recent - 0.5).abs() < 1e-9 && report[0].significant);
    assert!((report[1].recent - 0.05).abs() < 1e-9 && !report[1].significant);
    assert_eq!((report[1].cpu, report[1].gpu), (8.0, 1.0));
    // Allo quit: kept with what it used, never significant.
    meter.sample(
        t1 + Duration::from_secs(30),
        &running[..1],
        &[read(10, 8.5, 1.0, 160)],
    );
    let report = meter.report(t1 + Duration::from_secs(30));
    let allo = report.iter().find(|app| app.id == "so.oxy.allo").unwrap();
    assert!(!allo.running && !allo.significant && allo.cpu == 60.5);
}

#[test]
fn a_drm_clients_memory_is_its_regions_total_once_per_client() {
    let proc = scratch("proc-drm-memory");
    process(&proc, 77, (1, 1, 0, 0), 10, &[]);
    let directory = proc.join("77");
    // One client open twice (counted once) holding 12 MiB in system memory and 4096 KiB in the
    // device's; a driver from before the totals says only `drm-memory-*`.
    for (fd, client, text) in [
        (
            5,
            21,
            "drm-total-system0:\t12 MiB\ndrm-total-local0:\t4096 KiB\ndrm-shared-system0:\t2 MiB\n",
        ),
        (
            6,
            21,
            "drm-total-system0:\t12 MiB\ndrm-total-local0:\t4096 KiB\n",
        ),
        (9, 22, "drm-memory-vram:\t1024 KiB\n"),
    ] {
        std::os::unix::fs::symlink(
            "/dev/dri/renderD128",
            directory.join("fd").join(fd.to_string()),
        )
        .unwrap();
        std::fs::write(
            directory.join("fdinfo").join(fd.to_string()),
            format!("drm-pdev:\t0000:00:02.0\ndrm-client-id:\t{client}\ndrm-engine-render:\t5 ns\n{text}"),
        )
        .unwrap();
    }
    let reader = Proc {
        root: &proc,
        ticks: 100,
    };
    let drm = reader.drm(77, &mut Default::default());
    assert_eq!(drm.busy_ns, 10);
    assert_eq!(drm.memory, (12 << 20) + (4096 << 10) + (1024 << 10));
}

#[test]
fn an_applications_average_covers_twelve_hours_or_since_it_began() {
    let running = vec![Running {
        pid: 10,
        id: "so.oxy.inbox".into(),
        name: "Inbox".into(),
    }];
    let hour = Duration::from_secs(3600);
    let mut meter = Meter::default();
    // Started an hour before its first sample, having used 36 s by then: a hundredth of a core.
    meter.sample(hour * 2, &running, &[read(10, 36.0, 0.0, 3600)]);
    let average = |meter: &Meter, at| meter.report(at)[0].average;
    assert!((average(&meter, hour * 2) - 0.01).abs() < 1e-9);
    // Sampled every hour for a day, using 360 s an hour (a tenth of a core) from the fourth on.
    let mut used = 36.0;
    for n in 3..=26 {
        used += if n >= 4 { 360.0 } else { 0.0 };
        meter.sample(
            hour * n,
            &running,
            &[read(10, used, 0.0, 3600 * (n as u64 - 1))],
        );
    }
    // The last twelve hours: a tenth of a core, whatever came before.
    assert!((average(&meter, hour * 26) - 0.1).abs() < 1e-9);
}

// ---- Low Power Mode's signal ----

#[test]
fn low_power_mode_tells_each_part_waiting_once_per_change() {
    let low_power = LowPower::default();
    let changes = low_power.changes().unwrap();
    assert!(!low_power.on() && !changes.take());
    low_power.set(true);
    // Set again: no change, nothing told.
    low_power.set(true);
    assert!(low_power.on());
    let mut fds = [rustix::event::PollFd::new(
        &changes,
        rustix::event::PollFlags::IN,
    )];
    assert_eq!(rustix::event::poll(&mut fds, None).unwrap(), 1);
    assert!(changes.take());
    assert!(!changes.take());
    // A part that went is told no more; another still is.
    drop(changes);
    let other = low_power.clone().changes().unwrap();
    low_power.set(false);
    assert!(!low_power.on() && other.take());
}

// ---- The played services on a bus of the test's own ----

struct Children(Vec<Child>);

impl Drop for Children {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn private_bus(directory: &Path) -> (String, Children) {
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
    (address.trim().to_string(), Children(vec![bus]))
}

fn thinkpad(sysfs: &Path) -> Machine {
    serde_json::from_value(serde_json::json!({
        "battery": {
            "percent": 64.0, "state": "discharging", "time_to_empty": 12000,
            "energy_full": 49.6, "energy_full_design": 57.0, "cycles": 412,
            "threshold": {"start": 75, "end": 80}
        },
        "on_ac": false,
        "critical_action": "HybridSleep",
        "sysfs": sysfs,
    }))
    .unwrap()
}

#[test]
fn upower_says_the_batterys_health_and_limits_its_charge_through_sysfs() {
    let directory = scratch("upower");
    let sysfs = directory.join("BAT0");
    std::fs::create_dir_all(&sysfs).unwrap();
    std::fs::write(sysfs.join("charge_control_start_threshold"), "0").unwrap();
    std::fs::write(sysfs.join("charge_control_end_threshold"), "100").unwrap();
    let (address, _bus) = private_bus(&directory);
    let played = Played::serve(&address, thinkpad(&sysfs)).unwrap();
    let bus = connect(Some(&address)).unwrap();
    let details = upower::details(&bus).unwrap().unwrap();
    assert_eq!(details.max_capacity(), Some(87));
    assert_eq!(details.cycles, Some(412));
    assert!(!details.worn());
    assert_eq!(
        details.charge_limit,
        Some(upower::ChargeLimit {
            enabled: false,
            end: Some(80),
            start: Some(75)
        })
    );
    assert_eq!(
        upower::critical_action(&bus).unwrap().as_deref(),
        Some("HybridSleep")
    );
    // The battery as the top bar reads it, from the display device.
    let battery = oxyos_controls_free_read(&bus);
    assert_eq!(battery, (64.0, 2, true, 1));
    // Limited: UPower writes the thresholds; let charge full: 0 and 100.
    upower::set_charge_limit(&bus, true).unwrap();
    let read = |name: &str| std::fs::read_to_string(sysfs.join(name)).unwrap();
    assert_eq!(
        (
            read("charge_control_start_threshold"),
            read("charge_control_end_threshold")
        ),
        ("75".into(), "80".into())
    );
    assert!(
        upower::details(&bus)
            .unwrap()
            .unwrap()
            .charge_limit
            .unwrap()
            .enabled
    );
    upower::set_charge_limit(&bus, false).unwrap();
    assert_eq!(
        (
            read("charge_control_start_threshold"),
            read("charge_control_end_threshold")
        ),
        ("0".into(), "100".into())
    );
    assert_eq!(played.describe()["battery"]["charge_limit"], false);
}

/// The display device's percentage, state, on battery and warning level, read as the controls
/// read it.
fn oxyos_controls_free_read(bus: &zbus::blocking::Connection) -> (f64, u32, bool, u32) {
    let device = proxy(
        bus,
        upower::UPOWER,
        "/org/freedesktop/UPower/devices/DisplayDevice",
        upower::DEVICE,
    )
    .unwrap();
    let root = proxy(bus, upower::UPOWER, upower::UPOWER_PATH, upower::UPOWER).unwrap();
    (
        device.get_property("Percentage").unwrap(),
        device.get_property("State").unwrap(),
        root.get_property("OnBattery").unwrap(),
        device.get_property("WarningLevel").unwrap(),
    )
}

#[test]
fn a_charge_limit_refused_unsupported_or_without_a_battery_says_why() {
    let directory = scratch("upower-refused");
    let (address, _bus) = private_bus(&directory);
    let mut machine = thinkpad(&directory);
    machine.deny = vec!["charge_limit".into()];
    let played = Played::serve(&address, machine).unwrap();
    let bus = connect(Some(&address)).unwrap();
    assert_eq!(
        upower::set_charge_limit(&bus, true),
        Err(Failure::NotPermitted)
    );
    drop(played);
    // A battery whose firmware has no threshold.
    let directory = scratch("upower-unsupported");
    let (address, _bus) = private_bus(&directory);
    let mut machine = thinkpad(&directory);
    machine.battery.as_mut().unwrap().threshold = None;
    let _played = Played::serve(&address, machine).unwrap();
    let bus = connect(Some(&address)).unwrap();
    assert_eq!(
        upower::set_charge_limit(&bus, true),
        Err(Failure::Unsupported)
    );
    assert_eq!(upower::details(&bus).unwrap().unwrap().charge_limit, None);
    // A desktop: no battery, nothing to say or limit.
    let directory = scratch("upower-desktop");
    let (address, _bus) = private_bus(&directory);
    let _played = Played::serve(&address, serde_json::from_str::<Machine>("{}").unwrap()).unwrap();
    let bus = connect(Some(&address)).unwrap();
    assert_eq!(upower::details(&bus).unwrap(), None);
    assert_eq!(
        upower::set_charge_limit(&bus, true),
        Err(Failure::Unsupported)
    );
    // No UPower at all.
    let directory = scratch("upower-none");
    let (address, _bus) = private_bus(&directory);
    let bus = connect(Some(&address)).unwrap();
    assert_eq!(upower::details(&bus).unwrap(), None);
    assert_eq!(upower::critical_action(&bus).unwrap(), None);
    assert!(!profiles::available(&bus).unwrap());
}

/// Waits until `done` says so, at most five seconds.
fn until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "not in five seconds");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn low_power_mode_holds_power_saver_until_released_or_the_session_goes() {
    let directory = scratch("profiles");
    let (address, _bus) = private_bus(&directory);
    let played = Played::serve(&address, thinkpad(&directory)).unwrap();
    let bus = connect(Some(&address)).unwrap();
    assert!(profiles::available(&bus).unwrap());
    assert_eq!(profiles::active(&bus).unwrap().as_deref(), Some("balanced"));
    let hold = profiles::Hold::take(&bus).unwrap();
    assert_eq!(
        profiles::active(&bus).unwrap().as_deref(),
        Some("power-saver")
    );
    assert_eq!(
        played.describe()["holds"],
        serde_json::json!([{"profile": "power-saver", "reason": "Low Power Mode", "application": "so.oxy.oxyos"}])
    );
    hold.release(&bus);
    assert_eq!(profiles::active(&bus).unwrap().as_deref(), Some("balanced"));
    // The session's connection gone (it ended, it failed): the daemon releases its hold.
    let session = connect(Some(&address)).unwrap();
    let held = profiles::Hold::take(&session).unwrap();
    assert_eq!(
        profiles::active(&bus).unwrap().as_deref(),
        Some("power-saver")
    );
    // The hold is never released: its connection goes.
    let _ = held.cookie();
    let _ = session.close();
    until(|| profiles::active(&bus).unwrap().as_deref() == Some("balanced"));
    // Refused by polkit.
    drop(played);
    let directory = scratch("profiles-refused");
    let (address, _bus) = private_bus(&directory);
    let mut machine = thinkpad(&directory);
    machine.deny = vec!["hold".into()];
    let _played = Played::serve(&address, machine).unwrap();
    let bus = connect(Some(&address)).unwrap();
    assert_eq!(
        profiles::Hold::take(&bus).unwrap_err(),
        Failure::NotPermitted
    );
    // Without the daemon: unsupported.
    let directory = scratch("profiles-none");
    let (address, _bus) = private_bus(&directory);
    let mut machine = thinkpad(&directory);
    machine.profiles = false;
    let _played = Played::serve(&address, machine).unwrap();
    let bus = connect(Some(&address)).unwrap();
    assert_eq!(
        profiles::Hold::take(&bus).unwrap_err(),
        Failure::Unsupported
    );
}

#[test]
fn a_hold_released_by_another_programs_choice_is_followed() {
    let directory = scratch("follow");
    let (address, _bus) = private_bus(&directory);
    let _played = Played::serve(&address, thinkpad(&directory)).unwrap();
    let bus = connect(Some(&address)).unwrap();
    let hold = profiles::Hold::take(&bus).unwrap();
    let (stop, mut order) = std::os::unix::net::UnixStream::pair().unwrap();
    let (told, heard) = std::sync::mpsc::channel();
    let following = {
        let address = address.clone();
        std::thread::spawn(move || {
            let bus = connect(Some(&address)).unwrap();
            follow::profiles(&bus, stop.as_fd(), |change| told.send(change).is_ok())
        })
    };
    // `powerprofilesctl set balanced`, once the follower listens.
    std::thread::sleep(Duration::from_millis(200));
    let other = connect(Some(&address)).unwrap();
    proxy(&other, profiles::DAEMON, profiles::PATH, profiles::DAEMON)
        .unwrap()
        .set_property("ActiveProfile", "balanced")
        .unwrap();
    assert_eq!(
        heard.recv_timeout(Duration::from_secs(5)).unwrap(),
        follow::Change::Released(hold.cookie())
    );
    // The order to stop ends it at once.
    std::io::Write::write_all(&mut order, &[1]).unwrap();
    following.join().unwrap().unwrap();
}

#[test]
fn the_machine_sleeps_through_logind_unless_it_may_not() {
    let directory = scratch("sleep");
    let (address, _bus) = private_bus(&directory);
    let played = Played::serve(&address, thinkpad(&directory)).unwrap();
    let bus = connect(Some(&address)).unwrap();
    assert!(sleep::can_suspend(&bus).unwrap());
    sleep::suspend(&bus).unwrap();
    assert_eq!(played.describe()["suspends"], 1);
    drop(played);
    let directory = scratch("sleep-refused");
    let (address, _bus) = private_bus(&directory);
    let mut machine = thinkpad(&directory);
    machine.deny = vec!["suspend".into()];
    let played = Played::serve(&address, machine).unwrap();
    let bus = connect(Some(&address)).unwrap();
    assert_eq!(sleep::suspend(&bus), Err(Failure::NotPermitted));
    assert_eq!(played.describe()["suspends"], 0);
}

#[test]
fn the_played_machine_changes_and_upower_says_so() {
    let directory = scratch("act");
    let (address, _bus) = private_bus(&directory);
    let played = Played::serve(&address, thinkpad(&directory)).unwrap();
    let bus = connect(Some(&address)).unwrap();
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(upower::UPOWER)
        .unwrap()
        .member("PropertiesChanged")
        .unwrap()
        .build();
    let mut signals =
        zbus::blocking::MessageIterator::for_match_rule(rule, &bus, Some(16)).unwrap();
    played
        .act(&serde_json::json!({"percent": 9.0, "warning": "low"}))
        .unwrap();
    let message = signals.next().unwrap().unwrap();
    assert_eq!(
        message.header().path().unwrap().as_str(),
        "/org/freedesktop/UPower/devices/DisplayDevice"
    );
    assert_eq!(oxyos_controls_free_read(&bus), (9.0, 2, true, 3));
    played.act(&serde_json::json!({"on_ac": true})).unwrap();
    assert!(!oxyos_controls_free_read(&bus).2);
    assert!(played.act(&serde_json::json!({"colour": "red"})).is_err());
}

fn descriptors() -> Vec<String> {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
        .map(|target| target.display().to_string())
        .collect()
}

#[test]
#[ignore = "counts the process's descriptors: run alone"]
fn the_played_services_and_their_clients_leave_no_descriptor() {
    let directory = scratch("descriptors");
    let (address, _bus) = private_bus(&directory);
    // The process's D-Bus runtime, made once.
    drop(connect(Some(&address)).unwrap());
    std::thread::sleep(Duration::from_millis(200));
    let before = descriptors();
    {
        let played = Played::serve(&address, thinkpad(&directory)).unwrap();
        let bus = connect(Some(&address)).unwrap();
        let hold = profiles::Hold::take(&bus).unwrap();
        hold.release(&bus);
        drop(bus);
        drop(played);
    }
    std::thread::sleep(Duration::from_millis(1000));
    let after = descriptors();
    assert!(after.len() <= before.len(), "{before:?} → {after:?}");
}
