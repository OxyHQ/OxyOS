//! The accounts' service restarted under a running session (a package upgrade does it): the
//! session's registrar connects again, presents the session and registers every process still
//! running, without ever giving up; and the service takes a session it verified before its
//! restart though the session program's file was replaced since, by what it kept of it, never by
//! what the session says. As in `session`, the test is the registrar (its program standing for the
//! session program) and its children the launched applications.

use super::erase::{eraser, FakePasswords, FakeSystem};
use super::platform::{machine_as, Machine};
use super::session::{me, wait_for};
use super::*;
use crate::pidfd;
use crate::session::{
    reconnect_wait, Reason, Registrar, RegistrarThread, Registrars, Sessions, RECONNECT_MAX,
    RECONNECT_REFUSED_MAX,
};
use rustix::fd::{AsFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The program the test's parent runs (cargo's test runner), standing for the login program.
fn parent_program() -> PathBuf {
    let parent = rustix::process::getppid().expect("the test has a parent");
    std::fs::read_link(format!("/proc/{}/exe", parent.as_raw_nonzero())).unwrap()
}

/// One run of the service on the session socket at `path`, stopped as a restart stops it: every
/// connection it served closed under its peer, its socket gone.
struct Run {
    sessions: Arc<Sessions<FakeApi, FakeUsers>>,
    stopping: Arc<AtomicBool>,
    /// The listener's descriptor and each served connection's (duplicates, to shut them down).
    open: Arc<Mutex<Vec<OwnedFd>>>,
    path: PathBuf,
    accepting: Option<std::thread::JoinHandle<()>>,
}

impl Run {
    fn start(machine: &Machine, path: &Path, registrars: Registrars) -> Self {
        let listener = crate::ipc::Listener::bind(path).unwrap();
        let sessions = Arc::new(Sessions::new(
            machine.platform.clone(),
            registrars,
            eraser(
                &Arc::new(FakeSystem::default()),
                &Arc::new(FakePasswords::default()),
            ),
        ));
        let stopping = Arc::new(AtomicBool::new(false));
        let open = Arc::new(Mutex::new(vec![listener
            .as_fd()
            .try_clone_to_owned()
            .unwrap()]));
        let accepting = {
            let (sessions, stopping, open) = (sessions.clone(), stopping.clone(), open.clone());
            std::thread::spawn(move || loop {
                let Ok(connection) = listener.accept() else {
                    return;
                };
                if stopping.load(Ordering::SeqCst) {
                    return;
                }
                open.lock()
                    .unwrap()
                    .push(connection.as_fd().try_clone_to_owned().unwrap());
                let kind = sessions.classify(&connection);
                let sessions = sessions.clone();
                std::thread::spawn(move || sessions.serve(&connection, kind, |_| {}));
            })
        };
        Self {
            sessions,
            stopping,
            open,
            path: path.to_path_buf(),
            accepting: Some(accepting),
        }
    }

    /// The service stops: what it held goes with it.
    fn stop(mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        let _ = std::fs::remove_file(&self.path);
        for fd in self.open.lock().unwrap().drain(..) {
            let _ = rustix::net::shutdown(&fd, rustix::net::Shutdown::Both);
        }
        if let Some(accepting) = self.accepting.take() {
            accepting.join().unwrap();
        }
    }

    fn registered(&self) -> Vec<i32> {
        let mut pids: Vec<i32> = self
            .sessions
            .registered()
            .into_iter()
            .map(|(_, pid)| pid)
            .collect();
        pids.sort_unstable();
        pids
    }
}

/// The session program as the test runs it (`now`), or another file in its place (`replaced`,
/// as an upgrade leaves it), the registrars it verified kept in `verified`.
fn registrars(program: PathBuf, verified: Option<PathBuf>) -> Registrars {
    Registrars {
        program,
        parent: parent_program(),
        owner: me(),
        verified,
    }
}

/// A machine, its session socket's path, the registrars' directory (0700, the test's) and a file
/// standing for the session program an upgrade installed: another file than the one running.
struct Upgrade {
    machine: Machine,
    path: PathBuf,
    verified: PathBuf,
    replaced: PathBuf,
}

fn upgrade() -> Upgrade {
    let machine = machine_as(FakeApi::default(), PASSWORD, Some(me()));
    let path = machine.dir.path().join("session");
    let verified = machine.dir.path().join("registrars");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&verified)
        .unwrap();
    let replaced = machine.dir.path().join("oxyos-session");
    std::fs::write(&replaced, b"the upgrade's session program").unwrap();
    std::fs::set_permissions(&replaced, std::fs::Permissions::from_mode(0o755)).unwrap();
    Upgrade {
        machine,
        path,
        verified,
        replaced,
    }
}

fn launched() -> (Child, OwnedFd) {
    let child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let pidfd = pidfd::open(child.id()).unwrap();
    (child, pidfd)
}

fn end(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

type Said = Arc<Mutex<Vec<String>>>;

fn registrar_thread(path: &Path) -> (RegistrarThread, Said) {
    let said: Said = Arc::default();
    let log = said.clone();
    let thread = RegistrarThread::spawn(path.to_path_buf(), move |line| {
        log.lock().unwrap().push(line.to_owned())
    })
    .unwrap();
    (thread, said)
}

#[test]
fn the_session_registers_its_running_applications_again_when_the_service_restarts() {
    let upgrade = upgrade();
    let first = Run::start(
        &upgrade.machine,
        &upgrade.path,
        registrars(
            std::env::current_exe().unwrap(),
            Some(upgrade.verified.clone()),
        ),
    );
    let (session, said) = registrar_thread(&upgrade.path);
    let (shell, shell_pidfd) = launched();
    let (dropsea, dropsea_pidfd) = launched();
    let mut pids = vec![shell.id() as i32, dropsea.id() as i32];
    pids.sort_unstable();
    session.register("so.oxy.shell", true, shell_pidfd);
    session.register("so.oxy.dropsea", false, dropsea_pidfd);
    wait_for("both registered", || first.registered() == pids);

    // The upgrade replaces the session program and restarts the service: the new one finds
    // another file at the program's path than the one the session runs.
    first.stop();
    let second = Run::start(
        &upgrade.machine,
        &upgrade.path,
        registrars(upgrade.replaced.clone(), Some(upgrade.verified.clone())),
    );
    wait_for("both registered again", || second.registered() == pids);
    // Once more: a service started again after that still knows it.
    second.stop();
    let third = Run::start(
        &upgrade.machine,
        &upgrade.path,
        registrars(upgrade.replaced.clone(), Some(upgrade.verified.clone())),
    );
    wait_for("both registered a third time", || {
        third.registered() == pids
    });
    // A launch after the restarts is registered on the new connection.
    let (settings, settings_pidfd) = launched();
    session.register("so.oxy.settings", true, settings_pidfd);
    wait_for("the new launch registered", || {
        third.registered().len() == 3
    });
    let said = said.lock().unwrap().clone();
    assert!(
        said.iter().any(|line| line.contains("connected again")),
        "{said:?}"
    );
    assert!(
        !said.iter().any(|line| line.contains("refused the session")),
        "{said:?}"
    );
    for child in [shell, dropsea, settings] {
        end(child);
    }
    third.stop();
}

#[test]
fn the_session_never_gives_up_on_a_service_that_refuses_it_or_is_gone() {
    let upgrade = upgrade();
    // A service that cannot know the session: its program replaced, nothing kept.
    let (session, said) = registrar_thread(&upgrade.path);
    let refusing = Run::start(
        &upgrade.machine,
        &upgrade.path,
        registrars(upgrade.replaced.clone(), None),
    );
    let (app, pidfd) = launched();
    session.register("so.oxy.dropsea", false, pidfd);
    wait_for("the refusal said", || {
        said.lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("refused the session: NotPermitted"))
    });
    assert!(refusing.registered().is_empty());
    // Said once, however many times it is tried.
    std::thread::sleep(Duration::from_millis(3500));
    assert_eq!(
        said.lock()
            .unwrap()
            .iter()
            .filter(|line| line.contains("refused"))
            .count(),
        1
    );
    // Then no service at all for a while, then one that takes it: the launch is registered.
    refusing.stop();
    std::thread::sleep(Duration::from_millis(1500));
    let taking = Run::start(
        &upgrade.machine,
        &upgrade.path,
        registrars(std::env::current_exe().unwrap(), None),
    );
    wait_for("the launch registered at last", || {
        taking.registered() == [app.id() as i32]
    });
    end(app);
    taking.stop();
}

#[test]
fn what_was_kept_must_be_the_same_process_running_the_same_file() {
    let upgrade = upgrade();
    // Verified once, as the session program is now.
    let first = Run::start(
        &upgrade.machine,
        &upgrade.path,
        registrars(
            std::env::current_exe().unwrap(),
            Some(upgrade.verified.clone()),
        ),
    );
    assert_eq!(
        Registrar::connect(&upgrade.path)
            .unwrap()
            .present()
            .unwrap(),
        Ok(())
    );
    first.stop();
    let kept = upgrade.verified.join(std::process::id().to_string());
    let original = std::fs::read_to_string(&kept).unwrap();
    let present = || {
        let run = Run::start(
            &upgrade.machine,
            &upgrade.path,
            registrars(upgrade.replaced.clone(), Some(upgrade.verified.clone())),
        );
        let answer = Registrar::connect(&upgrade.path)
            .unwrap()
            .present()
            .unwrap();
        run.stop();
        answer
    };
    assert_eq!(present(), Ok(()));
    // Another start time (another process with the pid), another file run, another parent: none
    // is the registrar verified.
    let record: serde_json::Value = serde_json::from_str(&original).unwrap();
    for field in [
        "uid",
        "started",
        "device",
        "inode",
        "parent",
        "parentStarted",
        "parentDevice",
        "parentInode",
    ] {
        let mut changed = record.clone();
        changed[field] = serde_json::json!(record[field].as_i64().unwrap() + 1);
        std::fs::write(&kept, changed.to_string()).unwrap();
        assert_eq!(present(), Err(Reason::NotPermitted), "{field}");
    }
    // Kept where others may write it, it is not the service's word.
    std::fs::write(&kept, &original).unwrap();
    std::fs::set_permissions(&kept, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(present(), Err(Reason::NotPermitted));
    std::fs::set_permissions(&kept, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::set_permissions(&upgrade.verified, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(present(), Err(Reason::NotPermitted));
    std::fs::set_permissions(&upgrade.verified, std::fs::Permissions::from_mode(0o700)).unwrap();
    // Nothing kept: refused.
    std::fs::remove_file(&kept).unwrap();
    assert_eq!(present(), Err(Reason::NotPermitted));
}

#[test]
fn a_kept_registrar_whose_process_is_gone_is_forgotten() {
    let upgrade = upgrade();
    // A registrar of a process that has exited, as a session that ended leaves it.
    let (child, _) = launched();
    let gone = child.id();
    end(child);
    let stale = upgrade.verified.join(gone.to_string());
    std::fs::write(&stale, b"{}").unwrap();
    std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o600)).unwrap();
    let run = Run::start(
        &upgrade.machine,
        &upgrade.path,
        registrars(
            std::env::current_exe().unwrap(),
            Some(upgrade.verified.clone()),
        ),
    );
    assert_eq!(
        Registrar::connect(&upgrade.path)
            .unwrap()
            .present()
            .unwrap(),
        Ok(())
    );
    run.stop();
    assert!(!stale.exists());
    assert!(upgrade
        .verified
        .join(std::process::id().to_string())
        .exists());
}

#[test]
fn the_waits_to_connect_again_double_and_never_end() {
    let waits: Vec<u64> = (0..8).map(|n| reconnect_wait(n, false).as_secs()).collect();
    assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30, 30]);
    assert_eq!(reconnect_wait(u32::MAX, false), RECONNECT_MAX);
    assert_eq!(reconnect_wait(9, true), RECONNECT_REFUSED_MAX);
    assert_eq!(reconnect_wait(1_000, true), RECONNECT_REFUSED_MAX);
}
