//! The PAM tests run the system's real libpam and modules (`pam_permit`, `pam_deny`, `pam_exec`)
//! from a configuration of their own, read through `pam_start_confdir`: no account, no
//! `/etc/pam.d` change, no root.

use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const PASSWORD: &str = "correct horse battery";

/// A PAM configuration directory, removed when dropped.
struct Config(PathBuf);

impl Config {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("oxyos-auth-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // pam_exec hands the password on stdin, NUL-terminated; the script succeeds only for
        // PASSWORD, so a password the conversation mangled fails.
        let script = dir.join("check-password");
        fs::write(
            &script,
            format!("#!/bin/sh\n[ \"$(tr -d '\\000')\" = '{PASSWORD}' ]\n"),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        Self(dir)
    }

    fn service(&self, name: &str, text: &str) -> &Self {
        let text = text.replace("SCRIPT", &self.0.join("check-password").to_string_lossy());
        fs::write(self.0.join(name), text).unwrap();
        self
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn check(&self, service: &str, password: &str) -> Result<Outcome, Error> {
        Authenticator::with_confdir(service, "oxyos-test", self.path())
            .unwrap()
            .check(Secret::new(password.as_bytes().to_vec()))
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The stack Debian's common-auth uses: the password module, else deny.
const PASSWORD_STACK: &str = "\
auth [success=1 default=ignore] pam_exec.so quiet expose_authtok SCRIPT
auth requisite pam_deny.so
auth required pam_permit.so
account required pam_permit.so
";

#[test]
fn the_right_password_is_accepted() {
    let config = Config::new("right");
    config.service("oxyos", PASSWORD_STACK);
    assert_eq!(config.check("oxyos", PASSWORD), Ok(Outcome::Accepted));
}

#[test]
fn a_wrong_password_is_rejected() {
    let config = Config::new("wrong");
    config.service("oxyos", PASSWORD_STACK);
    assert_eq!(
        config.check("oxyos", "correct horse"),
        Ok(Outcome::Rejected)
    );
    assert_eq!(
        config.check("oxyos", &format!("{PASSWORD} ")),
        Ok(Outcome::Rejected)
    );
}

#[test]
fn an_empty_password_never_unlocks() {
    let config = Config::new("empty");
    config.service(
        "oxyos",
        "auth required pam_permit.so\naccount required pam_permit.so\n",
    );
    assert_eq!(config.check("oxyos", ""), Ok(Outcome::Rejected));
}

#[test]
fn a_nul_in_the_password_is_refused_before_pam() {
    let config = Config::new("nul");
    config.service(
        "oxyos",
        "auth required pam_permit.so\naccount required pam_permit.so\n",
    );
    assert_eq!(
        config.check("oxyos", "correct\0horse"),
        Err(Error::NulInSecret)
    );
}

#[test]
fn a_refused_account_is_rejected_even_with_the_right_password() {
    let config = Config::new("account");
    config.service(
        "oxyos",
        "auth [success=1 default=ignore] pam_exec.so quiet expose_authtok SCRIPT\n\
         auth requisite pam_deny.so\n\
         auth required pam_permit.so\n\
         account required pam_deny.so\n",
    );
    assert_eq!(config.check("oxyos", PASSWORD), Ok(Outcome::Rejected));
}

#[test]
fn a_denying_service_rejects() {
    let config = Config::new("deny");
    config.service(
        "oxyos",
        "auth required pam_deny.so\naccount required pam_permit.so\n",
    );
    assert_eq!(config.check("oxyos", PASSWORD), Ok(Outcome::Rejected));
}

#[test]
fn two_password_modules_share_one_prompt() {
    // pam_get_authtok hands the second module the token the first one asked for, so the
    // conversation's one answer serves both.
    let config = Config::new("second");
    config.service(
        "oxyos",
        "auth required pam_exec.so quiet expose_authtok SCRIPT\n\
         auth required pam_exec.so quiet expose_authtok SCRIPT\n\
         account required pam_permit.so\n",
    );
    assert_eq!(config.check("oxyos", PASSWORD), Ok(Outcome::Accepted));
}

#[test]
fn a_broken_configuration_never_unlocks() {
    let config = Config::new("broken");
    config.service(
        "oxyos",
        "auth required pam_oxyos_missing.so\naccount required pam_permit.so\n",
    );
    assert_ne!(config.check("oxyos", PASSWORD), Ok(Outcome::Accepted));
    // A service with no file and no `other` to fall back on.
    assert_ne!(
        config.check("oxyos-absent", PASSWORD),
        Ok(Outcome::Accepted)
    );
}

#[test]
fn the_session_user_comes_from_the_real_user_id() {
    let authenticator = Authenticator::for_session("oxyos").unwrap();
    assert!(!authenticator.user().to_bytes().is_empty());
    assert_eq!(
        Some(authenticator.user().to_owned()),
        user_name(rustix::process::getuid().as_raw())
    );
}

#[test]
fn a_secret_is_never_printed() {
    let secret = Secret::new(PASSWORD.as_bytes().to_vec());
    assert_eq!(format!("{secret:?}"), "Secret(…)");
}

#[test]
fn five_rejections_are_free() {
    let now = Instant::now();
    let mut throttle = Throttle::default();
    for _ in 0..4 {
        throttle.record(Outcome::Rejected, now);
        assert_eq!(throttle.wait(now), None);
    }
    throttle.record(Outcome::Rejected, now);
    assert_eq!(throttle.wait(now), Some(Duration::from_secs(30)));
}

#[test]
fn each_further_rejection_doubles_the_wait_up_to_fifteen_minutes() {
    let mut now = Instant::now();
    let mut throttle = Throttle::default();
    let mut waits = Vec::new();
    for _ in 0..12 {
        throttle.record(Outcome::Rejected, now);
        if let Some(wait) = throttle.wait(now) {
            waits.push(wait.as_secs());
            now += wait;
        }
    }
    assert_eq!(waits, [30, 60, 120, 240, 480, 900, 900, 900]);
    assert_eq!(throttle.wait(now), None);
}

#[test]
fn the_wait_runs_out() {
    let now = Instant::now();
    let mut throttle = Throttle::default();
    for _ in 0..5 {
        throttle.record(Outcome::Rejected, now);
    }
    assert_eq!(
        throttle.wait(now + Duration::from_secs(10)),
        Some(Duration::from_secs(20))
    );
    assert_eq!(throttle.wait(now + Duration::from_secs(30)), None);
}

#[test]
fn an_acceptance_resets_the_throttle() {
    let now = Instant::now();
    let mut throttle = Throttle::default();
    for _ in 0..6 {
        throttle.record(Outcome::Rejected, now);
    }
    throttle.record(Outcome::Accepted, now);
    assert_eq!(throttle.wait(now), None);
    for _ in 0..4 {
        throttle.record(Outcome::Rejected, now);
    }
    assert_eq!(throttle.wait(now), None);
}

/// The answer once the checker's descriptor says it is ready.
fn answer_of(checker: &mut Checker) -> Answer {
    let mut fds = [rustix::event::PollFd::from_borrowed_fd(
        checker.as_fd(),
        rustix::event::PollFlags::IN,
    )];
    let timeout = rustix::event::Timespec {
        tv_sec: 10,
        tv_nsec: 0,
    };
    assert_eq!(rustix::event::poll(&mut fds, Some(&timeout)).unwrap(), 1);
    checker.answer(Instant::now()).unwrap()
}

#[test]
fn the_checker_answers_off_the_thread_one_at_a_time() {
    let config = Config::new("checker");
    config.service("oxyos", PASSWORD_STACK);
    let authenticator = Authenticator::with_confdir("oxyos", "oxyos-test", config.path()).unwrap();
    let mut checker = Checker::spawn(authenticator).unwrap();
    let secret = |text: &str| Secret::new(text.as_bytes().to_vec());
    assert!(checker.answer(Instant::now()).is_none());
    assert_eq!(
        checker.attempt(secret("wrong"), Instant::now()),
        Attempt::Checking
    );
    assert_eq!(
        checker.attempt(secret(PASSWORD), Instant::now()),
        Attempt::Busy
    );
    assert_eq!(
        answer_of(&mut checker),
        Answer {
            outcome: Outcome::Rejected,
            wait: Duration::ZERO
        }
    );
    // Four more rejections and the fifth answer says to wait; until then nothing is checked.
    for _ in 0..4 {
        assert_eq!(
            checker.attempt(secret("wrong"), Instant::now()),
            Attempt::Checking
        );
        answer_of(&mut checker);
    }
    let Attempt::Throttled(wait) = checker.attempt(secret(PASSWORD), Instant::now()) else {
        panic!("not throttled");
    };
    assert!(wait > Duration::from_secs(29));
}

#[test]
fn a_broken_configuration_rejects_without_counting() {
    let config = Config::new("checker-broken");
    config.service("oxyos", "auth required pam_oxyos_missing.so\n");
    let authenticator = Authenticator::with_confdir("oxyos", "oxyos-test", config.path()).unwrap();
    let mut checker = Checker::spawn(authenticator).unwrap();
    for _ in 0..8 {
        assert_eq!(
            checker.attempt(Secret::new(PASSWORD.as_bytes().to_vec()), Instant::now()),
            Attempt::Checking
        );
        assert_eq!(answer_of(&mut checker).outcome, Outcome::Rejected);
    }
}

mod login {
    //! A login's transaction ([`crate::login`]) against the system's PAM with the tests' own
    //! configuration, and the greeter's messages ([`crate::greeter`]) on a real socket.

    use super::{Config, PASSWORD};
    use crate::greeter::{self, Answer, Greeter, ToLogin, Verdict};
    use crate::login::Login;
    use crate::user::User;
    use crate::{Error, Outcome, Secret};

    fn start(config: &Config, service: &str) -> Login {
        Login::start(service, "oxyos-test", Some(config.path())).unwrap()
    }

    const SESSION_STACK: &str = "\
auth [success=1 default=ignore] pam_exec.so quiet expose_authtok SCRIPT
auth requisite pam_deny.so
auth required pam_permit.so
account required pam_permit.so
session required pam_permit.so
";

    #[test]
    fn a_login_authenticates_then_opens_and_closes_its_session() {
        let config = Config::new("login-session");
        config.service("oxyos-login", SESSION_STACK);
        let mut login = start(&config, "oxyos-login");
        login.set_tty("/dev/tty1").unwrap();
        login.set_env("XDG_SESSION_CLASS", "user").unwrap();
        login.set_env("XDG_SEAT", "seat0").unwrap();
        assert_eq!(
            login.authenticate(Secret::new(PASSWORD.as_bytes().to_vec())),
            Ok(Outcome::Accepted)
        );
        login.open_session().unwrap();
        let env = login.environment();
        assert!(env.contains(&("XDG_SESSION_CLASS".into(), "user".into())));
        assert!(env.contains(&("XDG_SEAT".into(), "seat0".into())));
        login.close_session().unwrap();
        // Once: closing again does nothing.
        login.close_session().unwrap();
    }

    #[test]
    fn a_wrong_or_empty_password_never_signs_in() {
        let config = Config::new("login-wrong");
        config.service("oxyos-login", SESSION_STACK);
        let mut login = start(&config, "oxyos-login");
        assert_eq!(
            login.authenticate(Secret::new(b"correct horse".to_vec())),
            Ok(Outcome::Rejected)
        );
        assert_eq!(
            start(&config, "oxyos-login").authenticate(Secret::new(Vec::new())),
            Ok(Outcome::Rejected)
        );
        assert_eq!(
            start(&config, "oxyos-login").authenticate(Secret::new(b"a\0b".to_vec())),
            Err(Error::NulInSecret)
        );
    }

    #[test]
    fn a_refused_account_or_session_is_no_sign_in() {
        let config = Config::new("login-refused");
        config.service(
            "account",
            "auth required pam_permit.so\naccount required pam_deny.so\n",
        );
        config.service(
            "session",
            "auth required pam_permit.so\naccount required pam_permit.so\n\
             session required pam_deny.so\n",
        );
        assert_eq!(
            start(&config, "account").check_account(),
            Ok(Outcome::Rejected)
        );
        let mut login = start(&config, "session");
        assert_eq!(login.check_account(), Ok(Outcome::Accepted));
        assert!(login.open_session().is_err());
    }

    #[test]
    fn the_password_answers_one_prompt_only() {
        // A second hidden prompt (a code, a new password) is not the password's to answer.
        let config = Config::new("login-prompts");
        config.service(
            "oxyos-login",
            "auth required pam_exec.so quiet expose_authtok SCRIPT\n\
             auth required pam_exec.so quiet expose_authtok /bin/false\n\
             account required pam_permit.so\n",
        );
        let outcome = start(&config, "oxyos-login").authenticate(Secret::new(PASSWORD.into()));
        assert_ne!(outcome, Ok(Outcome::Accepted));
    }

    #[test]
    fn a_variable_is_named_and_given_whole() {
        let config = Config::new("login-env");
        config.service("oxyos-login", SESSION_STACK);
        let mut login = start(&config, "oxyos-login");
        assert!(login.set_env("A=B", "c").is_err());
        assert!(login.set_env("", "c").is_err());
        login.set_env("LANG", "es_ES.UTF-8").unwrap();
        assert_eq!(
            login.environment(),
            vec![("LANG".to_string(), "es_ES.UTF-8".to_string())]
        );
    }

    #[test]
    fn a_user_is_read_from_the_account_database() {
        let root = User::named("root").unwrap().unwrap();
        assert_eq!((root.uid, root.gid), (0, 0));
        assert_eq!(root.groups.first(), Some(&0));
        assert!(User::named("oxyos-no-such-user").unwrap().is_none());
        assert!(User::named("a\0b").unwrap().is_none());
    }

    #[test]
    fn only_root_starts_a_program_as_a_user() {
        if rustix::process::getuid().is_root() {
            return;
        }
        let root = User::named("root").unwrap().unwrap();
        let started = root.spawn("/bin/true".as_ref(), &[] as &[&str], &[]);
        assert!(started.is_err(), "a user cannot become another");
    }

    #[test]
    fn the_greeters_messages_round_trip_and_hide_the_password() {
        let attempt = ToLogin::Attempt {
            account: "oxy-aaaaaaaaaaaaaaaaaaaa".into(),
            secret: Secret::new(PASSWORD.into()),
        };
        assert!(!format!("{attempt:?}").contains(PASSWORD));
        assert!(ToLogin::decode(b"a").is_err());
        assert!(ToLogin::decode(b"a\x05abc").is_err());
        assert!(ToLogin::decode(b"a\x00").is_err());
        assert!(ToLogin::decode(b"x").is_err());
        let mut long = vec![b'a', 1, b'u'];
        long.extend(std::iter::repeat_n(b'p', greeter::MAX_SECRET_BYTES + 1));
        assert!(ToLogin::decode(&long).is_err());
        assert!(matches!(ToLogin::decode(b"s"), Ok(ToLogin::Shown)));
    }

    #[test]
    fn a_greeter_attempts_and_hears_the_answer_on_the_socket() {
        let dir = std::env::temp_dir().join(format!("oxyos-auth-greeter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("greeter");
        let me = rustix::process::getuid().as_raw();
        let listener = greeter::Listener::bind(&path, me).unwrap();
        assert!(listener.accept(me).unwrap().is_none(), "no one yet");
        let mut client = Greeter::connect(&path).unwrap();
        let connection = listener.accept(me).unwrap().unwrap();
        // Another uid's connection is refused (unless that is ours).
        let _other = Greeter::connect(&path).unwrap();
        assert!(listener.accept(me.wrapping_add(1)).unwrap().is_none());
        assert!(connection.receive().unwrap().is_none());
        // The login screen always names its account: without one, nothing is sent.
        assert_eq!(
            client.attempt(None, Secret::new(PASSWORD.into())),
            Some(Answer {
                verdict: Verdict::Rejected,
                wait_s: 0
            })
        );
        assert_eq!(
            client.attempt(Some("oxy-a"), Secret::new(PASSWORD.into())),
            None
        );
        assert_eq!(
            client.attempt(Some("oxy-a"), Secret::new(b"again".to_vec())),
            Some(Answer {
                verdict: Verdict::Busy,
                wait_s: 0
            })
        );
        let Some(ToLogin::Attempt { account, secret }) = connection.receive().unwrap() else {
            panic!("an attempt");
        };
        assert_eq!(account, "oxy-a");
        assert_eq!(&secret.0[..], PASSWORD.as_bytes());
        assert!(client.answer().is_none(), "not answered yet");
        let answer = Answer {
            verdict: Verdict::Throttled,
            wait_s: 30,
        };
        connection.answer(answer).unwrap();
        assert_eq!(client.answer(), Some(answer));
        client.shown().unwrap();
        assert!(matches!(
            connection.receive().unwrap(),
            Some(ToLogin::Shown)
        ));
        // The login program gone, an attempt is a rejection.
        assert_eq!(
            client.attempt(Some("oxy-a"), Secret::new(PASSWORD.into())),
            None
        );
        // Shut down, not only closed: a process another test forks meanwhile may hold a copy of
        // the descriptor until it executes.
        let gone = |fd: std::os::fd::BorrowedFd<'_>| {
            rustix::net::shutdown(fd, rustix::net::Shutdown::Both).unwrap()
        };
        gone(connection.as_fd());
        drop(connection);
        drop(listener);
        assert_eq!(
            client.answer().map(|answer| answer.verdict),
            Some(Verdict::Rejected)
        );
        // A greeter gone ends the connection.
        let listener = greeter::Listener::bind(&path, me).unwrap();
        let client = Greeter::connect(&path).unwrap();
        let connection = listener.accept(me).unwrap().unwrap();
        gone(client.as_fd());
        drop(client);
        assert_eq!(
            connection.receive().unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
