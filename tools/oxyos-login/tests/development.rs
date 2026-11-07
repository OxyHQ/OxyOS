//! The login program on its development path, as a process: a login screen's connection (as the
//! development compositor's), the accounts' service (a stand-in on a real socket, answering
//! `list` and `check`), PAM with a configuration of the test's own (`pam_start_confdir`:
//! `pam_exec` checks the password against a file the stand-in may replace, as the service
//! replaces a verifier), and the session recorded rather than started. Nothing needs root or a
//! seat.

use oxyos_account::ipc::{Account, Answer, Listener, Request};
use oxyos_account::platform::{CheckReason, Checked, Local, Verdict as Said};
use oxyos_auth::greeter::{Answer as Verdicted, Greeter, Verdict};
use oxyos_auth::Secret;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

const ADA: &str = "oxy-aaaaaaaaaaaaaaaaaaaa";
const PASSWORD: &str = "correct horse battery";
/// A password changed elsewhere: PAM does not know it, Oxy (the stand-in) accepts it.
const CHANGED: &str = "a password changed elsewhere";

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
    let verifier = dir.join("verifier");
    std::fs::write(&verifier, PASSWORD).unwrap();
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n[ \"$(tr -d '\\000')\" = \"$(cat '{}')\" ]\n",
            verifier.display()
        ),
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

/// What the stand-in says of a password PAM accepted.
#[derive(Clone, Copy)]
enum Oxy {
    /// Oxy agrees with PAM; and accepts [`CHANGED`], replacing the verifier with it.
    Agrees,
    /// Oxy signed the account out.
    SignedOut,
}

/// Answers every `list` with Ada alone, and each `check` as `oxy` says.
fn accounts(socket: &Path, verifier: PathBuf, oxy: Oxy) {
    let listener = Listener::bind(socket).unwrap();
    std::thread::spawn(move || loop {
        let Ok(connection) = listener.accept() else {
            return;
        };
        match connection.receive::<Request>() {
            Ok(Some(Some(Request::List))) => {
                let _ = connection.send(&Answer::Accounts(vec![Account {
                    account_id: "0123456789abcdef01234567".into(),
                    user: ADA.into(),
                    uid: 1001,
                    name: "Ada Lovelace".into(),
                    picture: None,
                    state: oxyos_account::store::State::Ready,
                }]));
            }
            Ok(Some(Some(Request::Check {
                user,
                password,
                local,
            }))) => {
                assert_eq!(user, ADA);
                let checked = match (oxy, local) {
                    (Oxy::SignedOut, _) => Checked {
                        result: Said::Rejected,
                        reason: Some(CheckReason::SignInAgain),
                    },
                    (Oxy::Agrees, Local::Rejected) if password.expose() == CHANGED => {
                        std::fs::write(&verifier, CHANGED).unwrap();
                        Checked::ACCEPTED
                    }
                    (Oxy::Agrees, Local::Accepted) => Checked::ACCEPTED,
                    (Oxy::Agrees, Local::Rejected) => Checked::REJECTED,
                };
                let _ = connection.send(&Answer::Checked(checked));
            }
            Ok(Some(Some(Request::UnlockKeychain { user, password }))) => {
                // Whose keychain, and that its password came: never the password itself.
                let line = format!("{user} {}\n", password.expose().len());
                let _ = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(verifier.with_file_name("unlocked"))
                    .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()));
                let _ = connection.send(&Answer::Keychain(
                    oxyos_account::keychain::KeychainState::Off,
                ));
            }
            _ => {}
        }
    });
}

/// The login program on its development path, with the stand-in answering as `oxy` says: its
/// process, the login screen's connection, the record of sessions.
fn login_program(dir: &Path, oxy: Oxy) -> (Child, Greeter, PathBuf) {
    let pam = configuration(dir);
    let accountd = dir.join("accountd");
    accounts(&accountd, dir.join("verifier"), oxy);
    let socket = dir.join("greeter");
    let record = dir.join("record");
    // Watched before the program starts: the name appears once, when its socket listens.
    let appearing = Appearing::watch(dir);
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
    appearing.wait(&socket);
    (login, Greeter::connect(&socket).unwrap(), record)
}

/// A directory watched for a name to appear in it: the login program's socket is renamed into
/// place once it listens (`greeter::Listener::bind`), so its name appearing is its readiness, and
/// a connection from then on is never refused.
struct Appearing(rustix::fd::OwnedFd);

impl Appearing {
    fn watch(dir: &Path) -> Self {
        use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
        let watch = inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC).unwrap();
        inotify::add_watch(&watch, dir, WatchFlags::CREATE | WatchFlags::MOVED_TO).unwrap();
        Self(watch)
    }

    /// Until `path` is there; the deadline only reports a socket that never came.
    fn wait(self, path: &Path) {
        use rustix::event::{poll, PollFd, PollFlags, Timespec};
        use rustix::fs::inotify;
        let name = path.file_name().unwrap().as_encoded_bytes();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut buffer = [std::mem::MaybeUninit::<u8>::uninit(); 4096];
        loop {
            let mut reader = inotify::Reader::new(&self.0, &mut buffer);
            while let Ok(event) = reader.next() {
                if event.file_name().map(|name| name.to_bytes()) == Some(name) {
                    return;
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "waited too long for {}", path.display());
            let timeout = Timespec::try_from(left).unwrap();
            let mut fds = [PollFd::new(&self.0, PollFlags::IN)];
            let _ = poll(&mut fds, Some(&timeout));
        }
    }
}

fn temporary(name: &str) -> Dir {
    let dir = Dir(std::env::temp_dir().join(format!("oxyos-login-{name}-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    dir
}

fn attempt(greeter: &mut Greeter, account: &str, password: &str) -> Verdicted {
    assert_eq!(
        greeter.attempt(Some(account), Secret::new(password.as_bytes().to_vec())),
        None
    );
    answer(greeter)
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
    let dir = temporary("dev");
    let (login, mut greeter, record) = login_program(&dir.0, Oxy::Agrees);
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
    // The accepted password, and it alone, unlocked Ada's keychain (ADR 0050 decision 2).
    let unlocked = dir.0.join("unlocked");
    let mut said = String::new();
    wait_for("the keychain's unlock", || {
        said = std::fs::read_to_string(&unlocked).unwrap_or_default();
        !said.is_empty()
    });
    assert_eq!(said, format!("{ADA} {}\n", PASSWORD.len()));
    stop(login);
}

#[test]
fn a_password_changed_elsewhere_signs_in_once_oxy_accepts_it() {
    let dir = temporary("changed");
    let (login, mut greeter, record) = login_program(&dir.0, Oxy::Agrees);
    // PAM does not know it; the service asks Oxy, which accepts it and replaces the verifier;
    // PAM, asked again, accepts it.
    assert_eq!(
        attempt(&mut greeter, ADA, CHANGED).verdict,
        Verdict::Accepted
    );
    let mut recorded = String::new();
    wait_for("the session", || {
        recorded = std::fs::read_to_string(&record).unwrap_or_default();
        !recorded.is_empty()
    });
    assert!(recorded.starts_with(&format!("session would start for {ADA} ")));
    assert!(!recorded.contains(CHANGED));
    stop(login);
}

#[test]
fn an_account_oxy_signed_out_does_not_sign_in_with_its_old_password() {
    let dir = temporary("signed-out");
    let (login, mut greeter, record) = login_program(&dir.0, Oxy::SignedOut);
    // PAM accepts it; the service says the account must sign in again online.
    assert_eq!(
        attempt(&mut greeter, ADA, PASSWORD).verdict,
        Verdict::Rejected
    );
    std::thread::sleep(Duration::from_millis(300));
    assert!(!record.exists(), "nobody signed in");
    assert!(!dir.0.join("unlocked").exists(), "no keychain unlocked");
    stop(login);
}
