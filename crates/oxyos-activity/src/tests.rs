use super::*;
use std::path::PathBuf;
use std::time::Duration;

/// A directory of the test's own, empty.
fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("oxyos-activity-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// A machine of the test's own: its `/proc`, `/sys` and `/etc/passwd`.
struct Machine {
    root: PathBuf,
}

impl Machine {
    fn new(name: &str) -> Self {
        let root = scratch(name);
        for directory in [
            "proc/pressure",
            "proc/net",
            "sys/block/nvme0n1/device",
            "sys/block/loop0",
        ] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        std::fs::write(
            root.join("passwd"),
            format!(
                "root:x:0:0::/root:/bin/sh\nperson:x:{0}:{0}::/home/person:/bin/sh\n",
                me()
            ),
        )
        .unwrap();
        let machine = Self { root };
        machine.system(0, 0, 0, 0, 0, 0);
        machine
    }

    fn procfs(&self) -> Procfs {
        Procfs {
            proc: self.root.join("proc"),
            sys: self.root.join("sys"),
            passwd: self.root.join("passwd"),
            ticks: 100,
        }
    }

    fn proc(&self) -> PathBuf {
        self.root.join("proc")
    }

    /// The machine's counters: processor ticks (user, system, idle), memory stalled (µs), disk
    /// sectors read and the network's bytes received.
    fn system(&self, user: u64, system: u64, idle: u64, stalled: u64, sectors: u64, received: u64) {
        let proc = self.proc();
        std::fs::write(
            proc.join("stat"),
            format!("cpu  {user} 0 {system} {idle} 0 0 0 0 0 0\ncpu0 1 0 1 1 0 0 0 0 0 0\ncpu1 1 0 1 1 0 0 0 0 0 0\nbtime 1700000000\n"),
        )
        .unwrap();
        std::fs::write(proc.join("loadavg"), "0.50 0.25 0.10 2/300 999\n").unwrap();
        std::fs::write(
            proc.join("meminfo"),
            "MemTotal: 16000000 kB\nMemFree: 1000 kB\nMemAvailable: 10000000 kB\nBuffers: 100000 kB\nCached: 3000000 kB\nAnonPages: 4000000 kB\nShmem: 500000 kB\nSUnreclaim: 200000 kB\nKernelStack: 20000 kB\nPageTables: 80000 kB\nSwapTotal: 8000000 kB\nSwapFree: 7000000 kB\n",
        )
        .unwrap();
        std::fs::write(
            proc.join("pressure/memory"),
            format!("some avg10=0.00 avg60=0.00 avg300=0.00 total={stalled}\nfull avg10=0.00 avg60=0.00 avg300=0.00 total={}\n", stalled / 4),
        )
        .unwrap();
        std::fs::write(
            proc.join("diskstats"),
            format!(" 259 0 nvme0n1 10 0 {sectors} 0 20 0 {sectors} 0 0 0 0\n 259 1 nvme0n1p1 10 0 {sectors} 0 20 0 {sectors} 0 0 0 0\n 7 0 loop0 1 0 999 0 1 0 999 0 0 0 0\n"),
        )
        .unwrap();
        std::fs::write(
            proc.join("net/dev"),
            format!("Inter-|   Receive  |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets\n    lo: 999 9 0 0 0 0 0 0 999 9 0 0 0 0 0 0\n  wlan0: {received} 10 0 0 0 0 0 0 500 5 0 0 0 0 0 0\n"),
        )
        .unwrap();
    }

    /// A process: its stat (times in ticks), its status's users and memory (KiB), its threads'
    /// runs and its I/O. Who it is, as the stat file's owner says, is the test's own user (it can
    /// own no other's files): a test makes another's by telling the sampler another session user.
    #[allow(clippy::too_many_arguments)]
    fn process(
        &self,
        pid: u32,
        ppid: u32,
        comm: &str,
        ticks: u64,
        uid: u32,
        anon_kib: u64,
        runs: &[u64],
        io: (u64, u64),
    ) {
        let directory = self.proc().join(pid.to_string());
        std::fs::create_dir_all(directory.join("fd")).unwrap();
        std::fs::create_dir_all(directory.join("fdinfo")).unwrap();
        std::fs::write(
            directory.join("stat"),
            format!("{pid} ({comm}) S {ppid} 1 1 0 -1 4194304 100 0 0 0 {ticks} 0 0 0 20 0 {} 0 {} 1000 100\n", runs.len(), pid * 10),
        )
        .unwrap();
        std::fs::write(
            directory.join("status"),
            format!("Name:\t{comm}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\nVmRSS:\t{} kB\nRssAnon:\t{anon_kib} kB\nVmSwap:\t8 kB\n", anon_kib * 2),
        )
        .unwrap();
        std::fs::write(
            directory.join("io"),
            format!(
                "rchar: 1\nwchar: 1\nread_bytes: {}\nwrite_bytes: {}\ncancelled_write_bytes: 0\n",
                io.0, io.1
            ),
        )
        .unwrap();
        std::fs::write(
            directory.join("cmdline"),
            format!("/usr/bin/{comm}-and-more\0--flag\0"),
        )
        .unwrap();
        for (n, count) in runs.iter().enumerate() {
            let task = directory.join("task").join((pid + n as u32).to_string());
            std::fs::create_dir_all(&task).unwrap();
            std::fs::write(task.join("schedstat"), format!("1000 2000 {count}\n")).unwrap();
        }
    }
}

fn me() -> u32 {
    rustix::process::getuid().as_raw()
}

fn find(sample: &Sample, pid: u32) -> &Process {
    sample.processes.iter().find(|p| p.pid == pid).unwrap()
}

fn settings() -> Vec<App> {
    vec![App {
        id: "so.oxy.settings".into(),
        name: "Settings".into(),
        pid: 100,
        icon: None,
    }]
}

#[test]
fn the_first_sample_has_no_rates_and_the_next_rates_over_its_interval() {
    let machine = Machine::new("cpu");
    machine.process(
        100,
        1,
        "oxyos-bloom-hos",
        100,
        me(),
        4000,
        &[10, 20],
        (0, 0),
    );
    machine.process(101, 100, "helper", 50, me(), 1000, &[5], (0, 0));
    machine.process(200, 1, "sshd", 0, me(), 2000, &[1], (0, 0));
    let mut sampler = Sampler::new(machine.procfs(), me());
    let t0 = Instant::now();
    let first = sampler.sample(View::Cpu, t0, &settings(), &[]);
    assert_eq!(first.interval, 0.0);
    assert_eq!(first.system.cpu, None);
    assert_eq!(find(&first, 100).cpu, None);
    assert_eq!(first.system.processes, 3);
    assert_eq!(first.system.threads, 4);
    // Settings' process is named as the application; its helper is its, by its parent.
    assert_eq!(find(&first, 100).name, "Settings");
    assert_eq!(find(&first, 101).app.as_deref(), Some("so.oxy.settings"));
    assert_eq!(find(&first, 200).app, None);
    assert_eq!(find(&first, 100).cpu_time, 1.0);
    assert!(find(&first, 200).own);
    assert_eq!(find(&first, 200).user, "person");

    // Two seconds: Settings used 1 s (half of one core), its threads ran 30 times more; the
    // machine was a quarter busy.
    machine.process(
        100,
        1,
        "oxyos-bloom-hos",
        200,
        me(),
        4000,
        &[25, 35],
        (0, 0),
    );
    machine.system(300, 100, 1200, 0, 0, 0);
    let second = sampler.sample(View::Cpu, t0 + Duration::from_secs(2), &settings(), &[]);
    assert_eq!(second.interval, 2.0);
    let settings_process = find(&second, 100);
    assert!((settings_process.cpu.unwrap() - 50.0).abs() < 1e-9);
    match settings_process.columns {
        Columns::Cpu { wakeups, .. } => assert!((wakeups.unwrap() - 15.0).abs() < 1e-9),
        ref other => panic!("{other:?}"),
    }
    let cpu = second.system.cpu.unwrap();
    assert!((cpu.user - 18.75).abs() < 1e-9 && (cpu.system - 6.25).abs() < 1e-9);
    assert_eq!(cpu.cores, 2);
    assert_eq!(second.system.load, [0.5, 0.25, 0.1]);
}

#[test]
fn a_pid_used_again_is_another_process() {
    let machine = Machine::new("reused");
    machine.process(300, 1, "worker", 500, me(), 10, &[1], (0, 0));
    let mut sampler = Sampler::new(machine.procfs(), me());
    let t0 = Instant::now();
    sampler.sample(View::Cpu, t0, &[], &[]);
    // The same pid, started later (its start is pid * 10 in this machine: rewritten by hand).
    let stat = machine.proc().join("300/stat");
    let text = std::fs::read_to_string(&stat)
        .unwrap()
        .replace(" 3000 1000", " 7777 1000");
    std::fs::write(&stat, text).unwrap();
    let next = sampler.sample(View::Cpu, t0 + Duration::from_secs(2), &[], &[]);
    let process = find(&next, 300);
    assert_eq!(process.started, 7777);
    assert_eq!(process.cpu, None);
}

#[test]
fn memory_is_anonymous_and_swapped_and_the_machine_says_its_pressure() {
    let machine = Machine::new("memory");
    machine.process(100, 1, "oxyos-bloom-hos", 0, me(), 4000, &[1], (0, 0));
    let mut sampler = Sampler::new(machine.procfs(), me());
    let t0 = Instant::now();
    let first = sampler.sample(View::Memory, t0, &settings(), &[]);
    match find(&first, 100).columns {
        Columns::Memory {
            memory,
            swap,
            resident,
            gpu_memory,
        } => {
            assert_eq!(
                (memory, swap, resident),
                ((4000 + 8) * 1024, 8 * 1024, 8000 * 1024)
            );
            // An application's GPU is read: it holds none.
            assert_eq!(gpu_memory, Some(0));
        }
        ref other => panic!("{other:?}"),
    }
    let memory = first.system.memory;
    assert_eq!(memory.total, 16_000_000 * 1024);
    assert_eq!(memory.used, 6_000_000 * 1024);
    assert_eq!(memory.app, 4_500_000 * 1024);
    assert_eq!(memory.wired, 300_000 * 1024);
    assert_eq!(memory.cached, 2_600_000 * 1024);
    assert_eq!(memory.swap_used, 1_000_000 * 1024);
    assert_eq!(memory.pressure, None);
    // Some tasks stalled 0.5 s of the next 2 (a quarter), all of them an eighth of a second.
    machine.system(0, 0, 0, 500_000, 0, 0);
    let next = sampler.sample(View::Memory, t0 + Duration::from_secs(2), &settings(), &[]);
    let pressure = next.system.memory.pressure.unwrap();
    assert!((pressure.some - 25.0).abs() < 1e-9 && (pressure.full - 6.25).abs() < 1e-9);
    assert_eq!(next.system.memory.level, Level::Warning);
    assert_eq!(system::level(5.0, 0.0), Level::Normal);
    assert_eq!(system::level(45.0, 0.0), Level::Critical);
    assert_eq!(system::level(20.0, 12.0), Level::Critical);
}

#[test]
fn disks_count_the_hardware_alone_and_a_process_its_own_storage() {
    let machine = Machine::new("disk");
    machine.process(100, 1, "writer", 0, me(), 1, &[1], (4096, 8192));
    let mut sampler = Sampler::new(machine.procfs(), me());
    let t0 = Instant::now();
    let first = sampler.sample(View::Disk, t0, &[], &[]);
    // nvme0n1 alone: not its partition, not the loop device.
    machine.process(100, 1, "writer", 0, me(), 1, &[1], (4096, 8192 + 2_000_000));
    machine.system(0, 0, 0, 0, 4000, 1_000_000);
    let next = sampler.sample(View::Disk, t0 + Duration::from_secs(2), &[], &[]);
    assert_eq!(first.system.disk.read, 0);
    assert_eq!(next.system.disk.read, 4000 * 512);
    assert_eq!(next.system.disk.read_rate, Some(2000.0 * 512.0));
    match find(&next, 100).columns {
        Columns::Disk {
            written,
            write_rate,
            read_rate,
            ..
        } => {
            assert_eq!(written, Some(8192 + 2_000_000));
            assert_eq!(write_rate, Some(1_000_000.0));
            assert_eq!(read_rate, Some(0.0));
        }
        ref other => panic!("{other:?}"),
    }
    // The network: per interface, the loopback left out.
    let network = &next.system.network;
    assert_eq!(network.received, 1_000_000);
    assert_eq!(network.received_rate, Some(500_000.0));
    let interfaces = network.interfaces.as_ref().unwrap();
    assert_eq!(interfaces.len(), 1);
    assert_eq!(interfaces[0].name.as_deref(), Some("wlan0"));
}

#[test]
fn another_users_processes_are_not_the_persons_and_their_storage_is_not_read() {
    let machine = Machine::new("others");
    machine.process(100, 1, "daemon", 0, 0, 1, &[1], (1, 1));
    // The session's user is someone else than the files' owner (the test's own user).
    let mut sampler = Sampler::new(machine.procfs(), me() + 1);
    let sample = sampler.sample(View::Disk, Instant::now(), &[], &[]);
    let process = find(&sample, 100);
    assert!(!process.own);
    assert_eq!(
        process.columns,
        Columns::Disk {
            read: None,
            written: None,
            read_rate: None,
            write_rate: None
        }
    );
}

#[test]
fn energy_is_the_processor_and_gpu_time_of_each_process_and_application() {
    let machine = Machine::new("energy");
    machine.process(100, 1, "oxyos-bloom-hos", 0, me(), 1, &[1], (0, 0));
    machine.process(101, 100, "helper", 0, me(), 1, &[1], (0, 0));
    let mut sampler = Sampler::new(machine.procfs(), me());
    let t0 = Instant::now();
    let first = sampler.sample(View::Energy, t0, &settings(), &[]);
    assert_eq!(first.energy.as_ref().unwrap()[0].impact, None);
    // Two seconds: the host used 0.5 s of processor and, through the GPU, 0.5 s more; its helper
    // 0.2 s.
    machine.process(100, 1, "oxyos-bloom-hos", 50, me(), 1, &[1], (0, 0));
    machine.process(101, 100, "helper", 20, me(), 1, &[1], (0, 0));
    let directory = machine.proc().join("100");
    std::os::unix::fs::symlink("/dev/dri/renderD128", directory.join("fd/9")).unwrap();
    std::fs::write(
        directory.join("fdinfo/9"),
        "drm-client-id:\t3\ndrm-engine-render:\t0 ns\n",
    )
    .unwrap();
    sampler.sample(View::Energy, t0 + Duration::from_secs(2), &settings(), &[]);
    std::fs::write(
        directory.join("fdinfo/9"),
        "drm-client-id:\t3\ndrm-engine-render:\t500000000 ns\n",
    )
    .unwrap();
    machine.process(100, 1, "oxyos-bloom-hos", 100, me(), 1, &[1], (0, 0));
    machine.process(101, 100, "helper", 40, me(), 1, &[1], (0, 0));
    let third = sampler.sample(View::Energy, t0 + Duration::from_secs(4), &settings(), &[]);
    let energy = |pid| match find(&third, pid).columns {
        Columns::Energy { energy } => energy.unwrap(),
        ref other => panic!("{other:?}"),
    };
    assert!((energy(100) - 50.0).abs() < 1e-9);
    assert!((energy(101) - 10.0).abs() < 1e-9);
    let app = &third.energy.as_ref().unwrap()[0];
    assert!((app.impact.unwrap() - 60.0).abs() < 1e-9);
    assert_eq!(app.average, None);
}

#[test]
fn a_cut_name_is_its_commands_and_a_chosen_one_is_kept() {
    let machine = Machine::new("names");
    machine.process(100, 1, "oxyos-bloom-hos", 0, me(), 1, &[1], (0, 0));
    machine.process(101, 1, "renamed-itself!", 0, me(), 1, &[1], (0, 0));
    std::fs::write(machine.proc().join("101/cmdline"), "/usr/bin/other\0").unwrap();
    let mut sampler = Sampler::new(machine.procfs(), me());
    let sample = sampler.sample(View::Network, Instant::now(), &[], &[]);
    assert_eq!(find(&sample, 100).name, "oxyos-bloom-hos-and-more");
    assert_eq!(find(&sample, 101).name, "renamed-itself!");
    // The view's columns: none of a process for the network, which is per interface.
    assert_eq!(find(&sample, 100).columns, Columns::Network {});
}

#[test]
fn a_sample_is_the_json_activity_monitor_reads() {
    let machine = Machine::new("json");
    machine.process(100, 1, "oxyos-bloom-hos", 0, me(), 1, &[1], (0, 0));
    let mut sampler = Sampler::new(machine.procfs(), me());
    let sample = sampler.sample(View::Cpu, Instant::now(), &settings(), &[]);
    let json = serde_json::to_value(&sample).unwrap();
    assert_eq!(json["view"], "cpu");
    assert_eq!(json["apps"][0]["id"], "so.oxy.settings");
    let process = &json["processes"][0];
    for key in [
        "pid", "ppid", "started", "name", "user", "own", "app", "cpu", "cpuTime", "threads",
        "wakeups", "gpu", "gpuTime",
    ] {
        assert!(process.get(key).is_some(), "{key} missing in {process}");
    }
    assert!(json["system"]["memory"]["swapUsed"].is_u64());
    assert!(json.get("energy").is_none());
}

#[test]
fn addresses_are_read_as_the_kernel_writes_them() {
    assert_eq!(
        inspect::address("0100007F:0277").as_deref(),
        Some("127.0.0.1:631")
    );
    assert_eq!(
        inspect::address("00000000000000000000000001000000:0016").as_deref(),
        Some("[::1]:22")
    );
    assert_eq!(
        inspect::address("0000000000000000FFFF00000100007F:0050").as_deref(),
        Some("127.0.0.1:80")
    );
}

// ---- Inspecting and quitting real processes: the test's own children ----

fn child() -> std::process::Child {
    std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap()
}

fn started(pid: u32) -> u64 {
    let mut buffer = String::new();
    procfs::stat(&Procfs::system(), pid, &mut buffer)
        .unwrap()
        .started
}

#[test]
fn a_process_of_the_persons_is_inspected_whole() {
    let mut sleeping = child();
    let pid = sleeping.id();
    let inspected = inspect::inspect(&Procfs::system(), pid, started(pid), me(), None).unwrap();
    assert!(inspected.own && !inspected.restricted);
    assert_eq!(inspected.name, "sleep");
    assert_eq!(inspected.ppid, std::process::id());
    assert_eq!(
        inspected.arguments.as_deref(),
        Some(&["sleep".to_owned(), "30".to_owned()][..])
    );
    assert!(inspected
        .path
        .as_deref()
        .is_some_and(|p| p.ends_with("sleep")));
    let memory = inspected.memory.unwrap();
    assert!(
        memory.resident > 0 && memory.unique > 0 && memory.footprint == memory.unique + memory.gpu
    );
    assert!(inspected.files.unwrap().iter().any(|file| file.fd <= 2));
    // As another user would see it: restricted.
    let restricted =
        inspect::inspect(&Procfs::system(), pid, started(pid), me() + 1, None).unwrap();
    assert!(restricted.restricted && restricted.files.is_none() && restricted.memory.is_none());
    // Its start no longer its: gone.
    assert_eq!(
        inspect::inspect(&Procfs::system(), pid, started(pid) + 1, me(), None),
        None
    );
    sleeping.kill().unwrap();
    sleeping.wait().unwrap();
}

#[test]
fn a_port_listened_on_is_inspected() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let me_pid = std::process::id();
    let inspected =
        inspect::inspect(&Procfs::system(), me_pid, started(me_pid), me(), None).unwrap();
    let ports = inspected.ports.unwrap();
    assert!(
        ports.iter().any(|p| p.kind == "tcp"
            && p.local == format!("127.0.0.1:{port}")
            && p.state == Some("listen")),
        "{ports:?}"
    );
}

#[test]
fn only_the_persons_own_process_is_quit_and_only_the_one_chosen() {
    let procfs = Procfs::system();
    let mut first = child();
    let pid = first.id();
    let start = started(pid);
    // A start that is not its: the pid is another process's, nothing is sent.
    assert_eq!(
        quit::quit(&procfs, pid, start + 1, false, me(), &[]),
        Err(quit::Failure::NotFound)
    );
    // Another user's (as the session's user is someone else): refused.
    assert_eq!(
        quit::quit(&procfs, pid, start, false, me() + 1, &[]),
        Err(quit::Failure::NotPermitted)
    );
    // The session's own and what it runs under: refused.
    assert_eq!(
        quit::quit(&procfs, pid, start, false, me(), &[pid]),
        Err(quit::Failure::NotPermitted)
    );
    assert!(first.try_wait().unwrap().is_none());
    // Quit: SIGTERM.
    assert_eq!(quit::quit(&procfs, pid, start, false, me(), &[]), Ok(()));
    let status = first.wait().unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(status.signal(), Some(15));
    // Gone: not found.
    assert_eq!(
        quit::quit(&procfs, pid, start, false, me(), &[]),
        Err(quit::Failure::NotFound)
    );
    // Force Quit: SIGKILL.
    let mut second = child();
    let pid = second.id();
    assert_eq!(
        quit::quit(&procfs, pid, started(pid), true, me(), &[]),
        Ok(())
    );
    assert_eq!(second.wait().unwrap().signal(), Some(9));
}

#[test]
fn the_session_and_what_it_runs_under_are_protected() {
    let chain = quit::protected(&Procfs::system(), std::process::id());
    assert_eq!(chain[0], std::process::id());
    assert!(chain.len() >= 2, "{chain:?}");
}
