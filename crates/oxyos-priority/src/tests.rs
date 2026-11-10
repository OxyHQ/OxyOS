use super::played::Played;
use super::*;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

/// Whether this kernel reports a fair thread's slice (`sched_getattr`'s runtime, Linux 6.12 on):
/// an older one ignores a requested slice and reports none.
fn kernel_has_slices() -> bool {
    scheduling(0).is_ok_and(|s| s.slice_ns != 0)
}

/// A nice above the calling thread's own (at most 19), which needs no privilege to take: the tests
/// run at whatever nice they were started at (a build at the least priority, nice 19, for one).
fn a_higher_nice() -> i32 {
    (scheduling(0).unwrap().nice + 1).min(19)
}

/// Runs `f` on a thread of its own, so what it changes of its thread's scheduling goes with it.
fn on_a_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(f).join().unwrap()
}

#[test]
fn a_slice_is_requested_with_the_nice_and_policy_kept() {
    on_a_thread(|| {
        // A nice of its own (raising one needs no privilege), which the request must keep.
        let nice = a_higher_nice();
        rustix::process::setpriority_process(None, nice).unwrap();
        let reported = kernel_has_slices();
        request_slice(0, SLICE).unwrap();
        let now = scheduling(0).unwrap();
        assert_eq!(now.policy, libc::SCHED_OTHER as u32);
        assert_eq!(now.nice, nice);
        assert!(now.reset_on_fork, "what it starts gets the default slice");
        if reported {
            assert_eq!(now.slice_ns, 1_000_000);
            // What it starts begins with the default slice, not its own.
            let child = on_a_thread(|| scheduling(0).unwrap());
            assert_ne!(child.slice_ns, 1_000_000);
            assert_eq!(child.nice, nice, "a nice of 0 or above is inherited");
        } else {
            eprintln!("this kernel reports no slice (before 6.12): the request is kept, unread");
        }
        // Asked again, nothing is refused: the flag it set is kept.
        request_slice(0, SLICE).unwrap();
    });
}

#[test]
fn another_thread_of_the_process_is_given_its_slice() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let (done, wait) = std::sync::mpsc::channel::<()>();
    let worker = std::thread::spawn(move || {
        sender.send(tid()).unwrap();
        wait.recv().unwrap();
        scheduling(0).unwrap()
    });
    let worker_tid = receiver.recv().unwrap();
    let reported = kernel_has_slices();
    request_slice(worker_tid, SLICE).unwrap();
    done.send(()).unwrap();
    let seen = worker.join().unwrap();
    assert!(seen.reset_on_fork);
    if reported {
        assert_eq!(seen.slice_ns, 1_000_000);
    }
}

#[test]
fn only_the_fair_policies_have_a_slice() {
    // A thread whose policy is not the fair scheduler's (asked of a policy number no thread can
    // have here: only the check is exercised, never a real-time policy).
    assert!(!fair(libc::SCHED_FIFO as u32));
    assert!(!fair(libc::SCHED_RR as u32));
    assert!(fair(libc::SCHED_OTHER as u32));
    assert!(fair(libc::SCHED_IDLE as u32));
}

/// A bus of the test's own.
struct Bus {
    address: String,
    daemon: Child,
    _directory: std::path::PathBuf,
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
        let _ = std::fs::remove_dir_all(&self._directory);
    }
}

fn bus() -> Bus {
    let directory =
        std::env::temp_dir().join(format!("oxyos-priority-{}-{}", std::process::id(), tid()));
    std::fs::create_dir_all(&directory).unwrap();
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
        address: address.trim().to_string(),
        daemon,
        _directory: directory,
    }
}

/// This process's control group (cgroup v2).
fn own_cgroup() -> std::path::PathBuf {
    let text = std::fs::read_to_string("/proc/self/cgroup").unwrap();
    std::path::PathBuf::from(text.lines().find_map(|l| l.strip_prefix("0::")).unwrap())
}

#[test]
fn the_played_rtkit_gives_a_thread_of_the_run_its_nice_and_slice() {
    let bus = bus();
    let played = Played::serve(&bus.address, &own_cgroup()).unwrap();
    let rtkit = Rtkit::At(bus.address.clone());
    let raised = on_a_thread(move || {
        // A nice above the thread's own, which needs no privilege: the path a session's −15 takes.
        let wanted = a_higher_nice();
        let raised = raise(tid(), &rtkit, wanted, SLICE);
        (
            wanted,
            raised.tid,
            raised.nice,
            raised.slice.is_ok(),
            scheduling(0).unwrap(),
        )
    });
    let (wanted, thread, nice, sliced, now) = raised;
    assert_eq!(nice, Some(Ok(wanted)));
    assert!(sliced);
    assert_eq!(now.nice, wanted);
    assert!(now.reset_on_fork, "rtkit marks it, as the real one does");
    if kernel_has_slices() {
        assert_eq!(now.slice_ns, 1_000_000, "the slice survives rtkit's policy");
    }
    assert_eq!(
        played.record(),
        [format!(
            "granted {}/{thread} nice {wanted}",
            std::process::id()
        )]
    );
}

#[test]
fn the_played_rtkit_clamps_to_its_least_nice_and_needs_the_capability_for_it() {
    let bus = bus();
    let played = Played::serve(&bus.address, &own_cgroup()).unwrap();
    let rtkit = Rtkit::At(bus.address.clone());
    let (thread, nice) = on_a_thread(move || {
        let raised = raise(tid(), &rtkit, -20, SLICE);
        (raised.tid, raised.nice)
    });
    let record = played.record();
    assert_eq!(record.len(), 1);
    // Asked for no lower than −15, rtkit's `MinNiceLevel`.
    assert!(
        record[0].contains(&format!("{thread} nice -15")),
        "{record:?}"
    );
    let privileged = rustix::process::geteuid().is_root();
    match nice {
        Some(Ok(-15)) => assert!(record[0].starts_with("granted"), "{record:?}"),
        // Without CAP_SYS_NICE the played service cannot lower a nice below 0: the caller is told.
        Some(Err(Refusal::Failed(_))) if !privileged => {
            assert!(record[0].starts_with("failed"), "{record:?}")
        }
        other => panic!("{other:?}, {record:?}"),
    }
}

#[test]
fn the_played_rtkit_never_changes_a_thread_outside_the_run() {
    let bus = bus();
    let played =
        Played::serve(&bus.address, std::path::Path::new("/oxyos-elsewhere.slice")).unwrap();
    let rtkit = Rtkit::At(bus.address.clone());
    let (nice, before, now) = on_a_thread(move || {
        let before = scheduling(0).unwrap().nice;
        let raised = raise(tid(), &rtkit, a_higher_nice(), SLICE);
        (raised.nice, before, scheduling(0).unwrap())
    });
    assert!(matches!(nice, Some(Err(Refusal::Refused(_)))), "{nice:?}");
    assert_eq!(now.nice, before, "its nice untouched");
    assert!(played.record()[0].contains("outside the run"));
}

#[test]
fn the_played_rtkit_refuses_another_process_thread() {
    let bus = bus();
    let played = Played::serve(&bus.address, &own_cgroup()).unwrap();
    let connection = zbus::blocking::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .build()
        .unwrap();
    let proxy = zbus::blocking::Proxy::new(&connection, RTKIT, RTKIT_PATH, RTKIT).unwrap();
    let answer = proxy.call_method("MakeThreadHighPriorityWithPID", &(1u64, 1u64, 5i32));
    assert!(answer.is_err());
    assert!(played.record()[0].contains("not the caller's own process"));
}

#[test]
fn no_rtkit_on_the_bus_is_said_and_the_slice_still_requested() {
    let bus = bus();
    let rtkit = Rtkit::At(bus.address.clone());
    let raised = on_a_thread(move || raise(tid(), &rtkit, TURN_NICE, SLICE));
    assert_eq!(raised.nice, Some(Err(Refusal::Absent)));
    assert!(raised.slice.is_ok());
    assert!(
        raised.describe().contains("no rtkit"),
        "{}",
        raised.describe()
    );
}

#[test]
fn the_development_compositor_asks_the_played_rtkit_alone() {
    // Never the system's: without the variable, nowhere.
    if std::env::var_os(PLAYED_BUS_VARIABLE).is_none() {
        assert_eq!(Rtkit::played_from_env(), Rtkit::Nowhere);
    }
}
