//! The administrator's password that authorizes erasing the machine
//! ([ADR 0054](../../../docs/adr/0054-recovery-and-erasing.mdx)), checked as the lock checks one
//! ([ADR 0010](../../../docs/adr/0010-lock-screen.mdx)): by PAM, under this service's own name
//! (`/etc/pam.d/oxyos-accountd`: the system's `common-auth` and `common-account`), against the
//! account's verifier on this machine and never online, and throttled as the lock is
//! (`oxyos_auth::Throttle`, one per account: five rejections free, then 30 s doubling to 15 min, an
//! acceptance resetting it). A check that failed without judging the password (PAM broken) is not
//! counted. One password is checked at a time.

use oxyos_account::erase::{PasswordCheck, Passwords};
use oxyos_account::{Secret, UserName};
use oxyos_auth::{Authenticator, Error, Outcome, Throttle};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

/// The PAM service the administrator's password is checked under.
pub const PAM_SERVICE: &str = "oxyos-accountd";

pub struct PamPasswords {
    service: String,
    /// A PAM configuration directory instead of `/etc/pam.d` (the tests').
    confdir: Option<PathBuf>,
    throttles: Mutex<HashMap<String, Throttle>>,
}

impl PamPasswords {
    pub fn new() -> Self {
        Self {
            service: PAM_SERVICE.into(),
            confdir: None,
            throttles: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    fn with_confdir(service: &str, confdir: PathBuf) -> Self {
        Self {
            service: service.into(),
            confdir: Some(confdir),
            throttles: Mutex::new(HashMap::new()),
        }
    }

    fn authenticator(&self, user: &UserName) -> Result<Authenticator, Error> {
        match &self.confdir {
            Some(confdir) => Authenticator::with_confdir(&self.service, user.as_str(), confdir),
            None => Authenticator::for_user(&self.service, user.as_str()),
        }
    }
}

impl Passwords for PamPasswords {
    fn check(&self, user: &UserName, password: Secret) -> PasswordCheck {
        // Held for the check: one at a time.
        let mut throttles = self.throttles.lock().unwrap_or_else(|p| p.into_inner());
        let throttle = throttles.entry(user.as_str().to_owned()).or_default();
        if let Some(wait) = throttle.wait(Instant::now()) {
            return PasswordCheck::Throttled(wait);
        }
        let secret = oxyos_auth::Secret::new(password.expose().as_bytes().to_vec());
        drop(password);
        let checked = self
            .authenticator(user)
            .and_then(|authenticator| authenticator.check(secret));
        match checked {
            Ok(Outcome::Accepted) => {
                throttle.record(Outcome::Accepted, Instant::now());
                PasswordCheck::Accepted
            }
            // A password holding a NUL is no password: a guess all the same.
            Ok(Outcome::Rejected) | Err(Error::NulInSecret) => {
                throttle.record(Outcome::Rejected, Instant::now());
                PasswordCheck::Rejected
            }
            Err(error) => PasswordCheck::Failed(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Against the system's real libpam and modules, from a configuration of the tests' own read
    //! through `pam_start_confdir`, as `oxyos-auth`'s tests run it: no account, no `/etc/pam.d`
    //! change, no root.

    use super::*;
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    const PASSWORD: &str = "correct horse battery";

    /// A PAM configuration directory whose service accepts PASSWORD alone (pam_exec hands the
    /// password to the committed `check-password`), removed when dropped.
    struct Config(PathBuf);

    impl Config {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("oxyos-accountd-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/check-password");
            fs::write(
                dir.join(PAM_SERVICE),
                format!(
                    "auth [success=1 default=ignore] pam_exec.so quiet expose_authtok {}\n\
                     auth requisite pam_deny.so\n\
                     auth required pam_permit.so\n\
                     account required pam_permit.so\n",
                    script.display()
                ),
            )
            .unwrap();
            fs::write(dir.join("broken"), "auth required pam_oxyos_missing.so\n").unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Config {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn user() -> UserName {
        oxyos_account::AccountId::parse("0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e")
            .unwrap()
            .user_name()
    }

    #[test]
    fn the_administrators_own_password_is_accepted_and_another_rejected() {
        let config = Config::new("right");
        let passwords = PamPasswords::with_confdir(PAM_SERVICE, config.path().to_owned());
        assert_eq!(
            passwords.check(&user(), Secret::new("correct horse")),
            PasswordCheck::Rejected
        );
        assert_eq!(
            passwords.check(&user(), Secret::new(PASSWORD)),
            PasswordCheck::Accepted
        );
        assert_eq!(
            passwords.check(&user(), Secret::new("")),
            PasswordCheck::Rejected
        );
    }

    #[test]
    fn five_rejections_are_free_then_each_attempt_waits_as_the_locks_do() {
        let config = Config::new("throttle");
        let passwords = PamPasswords::with_confdir(PAM_SERVICE, config.path().to_owned());
        for _ in 0..5 {
            assert_eq!(
                passwords.check(&user(), Secret::new("wrong")),
                PasswordCheck::Rejected
            );
        }
        // Throttled, the right password is not even checked.
        let PasswordCheck::Throttled(wait) = passwords.check(&user(), Secret::new(PASSWORD)) else {
            panic!("the sixth attempt waits");
        };
        assert!(wait > Duration::from_secs(25) && wait <= Duration::from_secs(30));
    }

    #[test]
    fn an_acceptance_resets_the_throttle() {
        let config = Config::new("reset");
        let passwords = PamPasswords::with_confdir(PAM_SERVICE, config.path().to_owned());
        for _ in 0..4 {
            passwords.check(&user(), Secret::new("wrong"));
        }
        assert_eq!(
            passwords.check(&user(), Secret::new(PASSWORD)),
            PasswordCheck::Accepted
        );
        for _ in 0..4 {
            assert_eq!(
                passwords.check(&user(), Secret::new("wrong")),
                PasswordCheck::Rejected
            );
        }
    }

    #[test]
    fn a_broken_pam_is_no_judgement_and_is_not_counted() {
        let config = Config::new("broken");
        let passwords = PamPasswords::with_confdir("broken", config.path().to_owned());
        for _ in 0..8 {
            assert!(matches!(
                passwords.check(&user(), Secret::new(PASSWORD)),
                PasswordCheck::Failed(_)
            ));
        }
    }
}
