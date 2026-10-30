//! The login program on its development path, as a process: a login screen's connection (as the
//! development compositor's), the accounts' service (a stand-in on a real socket, answering
//! `list`), PAM with a configuration of the test's own (`pam_start_confdir`: `pam_exec` checks
//! the password), and the session recorded rather than started. Nothing needs root or a seat.

use oxyos_account::ipc::{Account, Answer, Listener, Request};
use oxyos_auth::greeter::{Answer as Verdicted, Greeter, Verdict};
use oxyos_auth::Secret;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

const ADA: &str = "oxy-aaaaaaaaaaaaaaaaaaaa";
const PASSWORD: &str = "correct horse battery";

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn configuration(dir: &Path) -> PathBuf {
    let pam = dir.join("pam");
    std::fs::create_dir_all(&pam).unwrap();
    let script = pam.join("check-password");
    std::fs::write(
        &script,
        format!("#!/bin/sh\n[ \"$(tr -d '\\000')\" = '{PASSWORD}' ]\n"),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(
        pam.join("oxyos-login"),
        format!(
            "auth [success=1 default=ignore] pam_exec.so quiet expose_authtok {}\n\
             auth requisite pam_deny.so\n\
             auth required pam_permit.so\n\
             account required pam_permit.so\n\
             session required pam_permit.so\n",
            script.display()
        ),
    )
    .unwrap();
    pam
}

/// Answers every `list` with Ada alone.
fn accounts(socket: &Path) {
    let listener = Listener::bind(socket).unwrap();
    std::thread::spawn(move || loop {
        let Ok(connection) = listener.accept() else {
            return;
        };
        if let Ok(Some(Some(Request::List))) = connection.receive::<Request>() {
            let _ = connection.send(&Answer::Accounts(vec![Account {
                account_id: "0123456789abcdef01234567".into(),
                user: ADA.into(),
                uid: 1001,
                name: "Ada Lovelace".into(),
                picture: None,
            }]));
        }
    });
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "waited too long for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn answer(greeter: &mut Greeter) -> Verdicted {
    let mut answer = None;
    wait_for("an answer", || {
        answer = greeter.answer();
        answer.is_some()
    });
    answer.unwrap()
}

fn stop(mut login: Child) {
    let pid = rustix::process::Pid::from_raw(login.id() as i32).unwrap();
    rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
    assert!(login.wait().unwrap().success());
}

#[test]
fn a_wrong_password_is_refused_and_the_right_one_starts_the_session() {
    let dir = Dir(std::env::temp_dir().join(format!("oxyos-login-dev-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let pam = configuration(&dir.0);
    let accountd = dir.0.join("accountd");
    accounts(&accountd);
    let socket = dir.0.join("greeter");
    let record = dir.0.join("record");
    let login = Command::new(env!("CARGO_BIN_EXE_oxyos-login"))
        .arg("--dev")
        .arg("--pam-confdir")
        .arg(&pam)
        .arg("--record")
        .arg(&record)
        .arg("--socket")
        .arg(&socket)
        .arg("--accountd")
        .arg(&accountd)
        .spawn()
        .unwrap();
    wait_for("the login screen's socket", || socket.exists());
    let mut greeter = Greeter::connect(&socket).unwrap();
    let attempt = |greeter: &mut Greeter, account: &str, password: &str| {
        assert_eq!(
            greeter.attempt(Some(account), Secret::new(password.as_bytes().to_vec())),
            None
        );
        answer(greeter)
    };
    // An account the service does not list is refused, whatever the password.
    assert_eq!(
        attempt(&mut greeter, "root", PASSWORD).verdict,
        Verdict::Rejected
    );
    // A wrong password for Ada, by PAM.
    assert_eq!(
        attempt(&mut greeter, ADA, "correct horse").verdict,
        Verdict::Rejected
    );
    assert!(!record.exists(), "nobody signed in");
    // The right one: accepted, and the session would start, for Ada alone.
    assert_eq!(
        attempt(&mut greeter, ADA, PASSWORD),
        Verdicted {
            verdict: Verdict::Accepted,
            wait_s: 0
        }
    );
    let mut recorded = String::new();
    wait_for("the session", || {
        recorded = std::fs::read_to_string(&record).unwrap_or_default();
        !recorded.is_empty()
    });
    assert!(
        recorded.starts_with(&format!("session would start for {ADA} (user session): ")),
        "{recorded}"
    );
    assert!(recorded.contains("--shell"), "{recorded}");
    assert!(recorded.contains("XDG_SESSION_CLASS"), "{recorded}");
    assert_eq!(recorded.lines().count(), 1);
    assert!(!recorded.contains(PASSWORD));
    stop(login);
}
