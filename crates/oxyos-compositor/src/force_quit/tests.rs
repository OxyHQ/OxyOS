use super::*;
use oxyos_apps::Runtime;
use oxyos_compositor_core::scene::Scene;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;

/// SIGKILL's number, on every Linux architecture.
const SIGKILL: i32 = 9;

/// A registry of one application, `so.oxy.stubborn`, which ignores SIGQUIT itself: the shell
/// sets it to be ignored after the launcher reset every signal, and `sleep` keeps it across exec.
struct Registry(PathBuf);

impl Registry {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("oxyos-force-quit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("so.oxy.stubborn.desktop"),
            "[Desktop Entry]\nType=Application\nName=Stubborn\nExec=/bin/sh -c \"trap '' QUIT; exec /bin/sleep 30\"\n",
        )
        .unwrap();
        Self(dir)
    }

    fn launcher(&self) -> Launcher {
        let runtime = Runtime {
            socket: PathBuf::from("/run/oxyos-test"),
            bloom_host: None,
            bloom_bundle: None,
            runtimes: PathBuf::from("/nonexistent-runtimes"),
            window: (960, 600),
            env: Vec::new(),
        };
        Launcher::new(runtime, vec![self.0.clone()], vec![])
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The signal that ended `pid`, once the launcher finds it ended (within 10 s), or none.
fn ended_by(launcher: &mut Launcher, pid: u32) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(exit) = launcher.reap().into_iter().find(|exit| exit.pid == pid) {
            return exit.status.and_then(|status| status.signal());
        }
        assert!(Instant::now() < deadline, "{pid} was not ended");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_hung_application_that_ignores_sigquit_is_killed_once_its_report_wait_is_up() {
    let registry = Registry::new("stubborn");
    let mut launcher = registry.launcher();
    let mut compositor = Compositor::new(Scene::new(), 0);
    let mut force_quit = ForceQuit::default();
    // Wait until the shell has set SIGQUIT to be ignored and become `sleep`.
    let pid = launcher.launch("so.oxy.stubborn").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .is_ok_and(|comm| comm.trim() == "sleep")
    {
        assert!(Instant::now() < deadline, "{pid} never became sleep");
        std::thread::sleep(Duration::from_millis(10));
    }
    let now = Instant::now();
    force_quit.end_unanswered(&mut launcher, pid, Duration::from_secs(3), now);
    assert_eq!(force_quit.deadline(), Some(now + HANG_REPORT_WAIT));
    // Asked for its report, it ignores the request: still running before the wait is up.
    force_quit.serve(&mut compositor, &mut launcher, now);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        launcher.reap().iter().all(|exit| exit.pid != pid),
        "SIGQUIT ended it"
    );
    // The wait up, it is killed all the same, and nothing is left to wake for.
    force_quit.serve(&mut compositor, &mut launcher, now + HANG_REPORT_WAIT);
    assert_eq!(force_quit.deadline(), None);
    assert_eq!(ended_by(&mut launcher, pid), Some(SIGKILL));
}

#[test]
fn a_hung_process_that_cannot_be_asked_for_its_report_is_not_kept_waiting() {
    let registry = Registry::new("not-ours");
    let mut launcher = registry.launcher();
    let mut force_quit = ForceQuit::default();
    // Not the launcher's: neither asked nor killed, and nothing to wake for.
    force_quit.end_unanswered(
        &mut launcher,
        std::process::id(),
        Duration::from_secs(3),
        Instant::now(),
    );
    assert_eq!(force_quit.deadline(), None);
}
