//! An application starts with every signal at its default and none blocked, whatever the session's
//! own (docs/adr/0067-hangs-and-force-quit.mdx): a session that ignores SIGQUIT, as a shell's
//! background job does (POSIX), or blocks it, hands neither on, and Force Quit's request for a hung
//! application's report reaches it. A test binary of its own: the dispositions it changes are the
//! whole process's.

use oxyos_apps::{Launcher, Runtime};
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The signals a session most often inherits ignored: a shell's background job ignores SIGINT and
/// SIGQUIT, `nohup` SIGHUP, and a careless parent anything.
const INHERITED: [libc::c_int; 4] = [libc::SIGQUIT, libc::SIGINT, libc::SIGHUP, libc::SIGTERM];

/// This process ignores `signals`.
#[allow(unsafe_code)]
fn ignore(signals: &[libc::c_int]) {
    for signal in signals {
        // SAFETY: signal(2) with SIG_IGN installs no handler, so no code of ours can run on a
        // signal; `signal` is a valid signal number.
        let previous = unsafe { libc::signal(*signal, libc::SIG_IGN) };
        assert_ne!(previous, libc::SIG_ERR, "signal {signal}");
    }
}

/// The calling thread blocks `signal`, as the thread that starts the application.
#[allow(unsafe_code)]
fn block(signal: libc::c_int) {
    // SAFETY: the set is initialized by sigemptyset before it is read, and pthread_sigmask only
    // reads it; blocking a signal runs no code of ours.
    let blocked = unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, signal);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut())
    };
    assert_eq!(blocked, 0);
}

/// A mask of `/proc/PID/status` (`SigIgn`, `SigBlk`: the signals ignored, blocked).
fn mask(pid: u32, field: &str) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{field}:")))
        .unwrap_or_else(|| panic!("no {field} in /proc/{pid}/status"));
    u64::from_str_radix(line.trim(), 16).unwrap()
}

/// The bit of `signal` in a `/proc` mask.
fn bit(signal: libc::c_int) -> u64 {
    1 << (signal - 1)
}

#[test]
fn an_application_starts_with_every_signal_at_its_default_and_force_quits_report_reaches_it() {
    let dir = std::env::temp_dir().join(format!("oxyos-apps-signals-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("so.oxy.sleeper.desktop"),
        "[Desktop Entry]\nType=Application\nName=Sleeper\nExec=/bin/sleep 30\n",
    )
    .unwrap();
    let runtime = Runtime {
        socket: PathBuf::from("/run/oxyos-test"),
        bloom_host: None,
        bloom_bundle: None,
        runtimes: PathBuf::from("/nonexistent-runtimes"),
        window: (960, 600),
        env: Vec::new(),
    };
    let mut launcher = Launcher::new(runtime, vec![dir.clone()], vec![]);

    // The session as a shell's background job leaves it: these ignored, SIGQUIT blocked too.
    ignore(&INHERITED);
    block(libc::SIGQUIT);
    let ours = std::process::id();
    for signal in INHERITED {
        assert_ne!(
            mask(ours, "SigIgn") & bit(signal),
            0,
            "the test ignores {signal}"
        );
    }

    // The application has none of it: nothing ignored, nothing blocked.
    let pid = launcher.launch("so.oxy.sleeper").unwrap();
    assert_eq!(mask(pid, "SigIgn"), 0, "signals ignored by the application");
    assert_eq!(mask(pid, "SigBlk"), 0, "signals blocked in the application");

    // So Force Quit's request for its report reaches it: a program without a handler of its own
    // ends on SIGQUIT, asked, so never an unexpected quit.
    assert!(launcher.end_hung(pid));
    let deadline = Instant::now() + Duration::from_secs(10);
    let ended = loop {
        if let Some(exit) = launcher.reap().into_iter().find(|exit| exit.pid == pid) {
            break exit;
        }
        assert!(Instant::now() < deadline, "SIGQUIT did not end it");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        ended.status.and_then(|status| status.signal()),
        Some(libc::SIGQUIT)
    );
    assert!(ended.asked && !ended.abnormal());
    let _ = std::fs::remove_dir_all(&dir);
}
