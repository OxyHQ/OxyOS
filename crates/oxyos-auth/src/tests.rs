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
