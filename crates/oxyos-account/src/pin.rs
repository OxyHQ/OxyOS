//! An account's local sign-in ([ADR 0020](../../docs/adr/0020-oxy-account-login.mdx), decision
//! 2): its **PIN**, a passcode for this computer alone, as Windows Hello's PIN or an iPhone's
//! passcode, and whether its Oxy **password** signs in on it.
//!
//! - **A PIN** is 4 to 16 digits ([`usable_pin`]). It signs in at the login screen and unlocks the
//!   lock offline, through PAM (`oxyos-pin-check`, `pam_exec`), and is never sent to Oxy.
//! - **Its verifier** is Argon2id in the PHC string format (`$argon2id$v=19$m=65536,t=3,p=1$…`),
//!   at the keybag's parameters (ADR 0050: 64 MiB, three passes, one lane), with a 16-byte salt
//!   from the system's CSPRNG. A verifier of other parameters is refused, never checked with
//!   weaker ones. It is kept in the service's own directory, readable by root alone, as the
//!   password's verifier is in `/etc/shadow`.
//! - **Its throttle** is its own, and survives the service: five wrong PINs are free, then each
//!   further attempt waits 30 s, doubling (to 8 min), as the lock's does (ADR 0010); the tenth
//!   wrong PIN in a row **locks it out**. A locked-out PIN signs nothing in until the account's
//!   password signs in at the login screen, or Oxy is asked online (the account set up again, or
//!   a new PIN given with the password). A right PIN resets the count.
//! - **The password** signs in while `password` is on (the default, and every account set up
//!   before PINs). Settings may turn it off once a PIN exists: the Unix user's verifier is locked,
//!   and the PIN is the account's one local credential. An account whose Oxy account has no
//!   password has only the PIN.
//!
//! The state is a file per account beside its account file, `<user>.local` (mode 0600, written
//! whole, [`crate::store::Store::put_local`]); an account without one has no PIN and signs in with
//! its password.

use crate::secret::Secret;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The fewest and most digits of a PIN.
pub const MIN_PIN_DIGITS: usize = 4;
pub const MAX_PIN_DIGITS: usize = 16;
/// Wrong PINs in a row that wait for nothing.
pub const FREE_FAILURES: u32 = 5;
/// The wait after the first failure past the free ones; each further one doubles it.
pub const FIRST_WAIT: Duration = Duration::from_secs(30);
/// Wrong PINs in a row that lock the PIN out.
pub const LOCKOUT_FAILURES: u32 = 10;

/// Whether `pin` can be a PIN: 4 to 16 ASCII digits.
pub fn usable_pin(pin: &Secret) -> bool {
    let pin = pin.expose();
    (MIN_PIN_DIGITS..=MAX_PIN_DIGITS).contains(&pin.len())
        && pin.bytes().all(|b| b.is_ascii_digit())
}

/// How long the next attempt waits after `failures` wrong PINs in a row.
pub fn wait_after(failures: u32) -> Duration {
    if failures < FREE_FAILURES {
        return Duration::ZERO;
    }
    let doublings = (failures - FREE_FAILURES).min(16);
    FIRST_WAIT.saturating_mul(1 << doublings)
}

/// An account's PIN: its verifier and the wrong attempts since the last right one.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinRecord {
    pub verifier: String,
    pub failures: u32,
}

impl std::fmt::Debug for PinRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinRecord")
            .field("failures", &self.failures)
            .finish_non_exhaustive()
    }
}

impl PinRecord {
    pub fn locked(&self) -> bool {
        self.failures >= LOCKOUT_FAILURES
    }
}

/// An account's local sign-in, as its `.local` file holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalSignIn {
    pub pin: Option<PinRecord>,
    /// Whether the account's password signs in on this computer.
    pub password: bool,
}

impl Default for LocalSignIn {
    /// An account set up before PINs: its password, and no PIN.
    fn default() -> Self {
        Self {
            pin: None,
            password: true,
        }
    }
}

impl LocalSignIn {
    pub fn pin_state(&self) -> PinState {
        match &self.pin {
            None => PinState::None,
            Some(record) if record.locked() => PinState::Locked,
            Some(_) => PinState::Set,
        }
    }
}

#[cfg(feature = "local")]
pub use verifier::{hash_pin, verify_pin, VerifierError};
// The messages, the protocol's own crate's (`oxyos-account-proto`): clients link them alone.
pub use oxyos_account_proto::pin::PinState;

/// What a PIN came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinCheck {
    Accepted,
    /// Wrong; the next attempt waits `wait` (zero: at once). The tenth in a row locks it out.
    Rejected {
        wait: Duration,
        locked: bool,
    },
    /// Not checked: the last wrong one's wait is not over.
    Throttled(Duration),
    /// Not checked: locked out, until the password signs in or Oxy is asked.
    Locked,
    /// The account has no PIN (or is not an enrolled account): nothing was checked.
    NoPin,
}

/// The service's PIN checks: the wait each account's last wrong PIN started, in memory (a service
/// started again waits from its start for an account past its free attempts); the count itself
/// is the account's file's. The caller holds the machine's lock (`Setup::lock_machine`), so one
/// PIN of an account is checked at a time and no setup writes its file meanwhile.
#[derive(Debug, Default)]
pub struct PinChecks {
    failed_at: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
}

impl PinChecks {
    pub fn new() -> Self {
        Self::default()
    }

    /// `user` signed in with the password (or was set up again online): its wrong PINs are
    /// forgotten, and a locked-out PIN signs in again.
    pub fn forget_failures(
        &self,
        store: &crate::store::Store,
        user: &crate::id::UserName,
    ) -> std::io::Result<()> {
        self.failed_at
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(user.as_str());
        let mut local = store.local(user)?;
        match local.pin.as_mut() {
            Some(record) if record.failures != 0 => {
                record.failures = 0;
                store.put_local(user, &local)
            }
            _ => Ok(()),
        }
    }
}

#[cfg(feature = "local")]
impl PinChecks {
    /// Checks `pin` for `user`, counting a wrong one, at `now`.
    pub fn check(
        &self,
        store: &crate::store::Store,
        user: &crate::id::UserName,
        pin: &Secret,
        now: std::time::Instant,
    ) -> std::io::Result<PinCheck> {
        let mut local = store.local(user)?;
        let Some(record) = local.pin.as_mut() else {
            return Ok(PinCheck::NoPin);
        };
        if record.locked() {
            return Ok(PinCheck::Locked);
        }
        let mut failed_at = self.failed_at.lock().unwrap_or_else(|p| p.into_inner());
        let wait = wait_after(record.failures);
        if !wait.is_zero() {
            // A service started again counts the wait from its start.
            let since = *failed_at.entry(user.as_str().to_owned()).or_insert(now);
            let left = wait.saturating_sub(now.saturating_duration_since(since));
            if !left.is_zero() {
                return Ok(PinCheck::Throttled(left));
            }
        }
        // Something typed that is no PIN was no guess at one.
        if !usable_pin(pin) {
            return Ok(PinCheck::Rejected {
                wait: Duration::ZERO,
                locked: false,
            });
        }
        let right = verify_pin(&record.verifier, pin)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        if right {
            failed_at.remove(user.as_str());
            if record.failures != 0 {
                record.failures = 0;
                store.put_local(user, &local)?;
            }
            return Ok(PinCheck::Accepted);
        }
        record.failures = record.failures.saturating_add(1);
        let (failures, locked) = (record.failures, record.locked());
        store.put_local(user, &local)?;
        failed_at.insert(user.as_str().to_owned(), now);
        Ok(PinCheck::Rejected {
            wait: if locked {
                Duration::ZERO
            } else {
                wait_after(failures)
            },
            locked,
        })
    }
}

#[cfg(feature = "local")]
mod verifier {
    use super::usable_pin;
    use crate::secret::Secret;
    use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
    use argon2::{Algorithm, Argon2, Params, Version};
    use std::fmt;

    /// The keybag's parameters (ADR 0050): 64 MiB, three passes, one lane, 32 bytes.
    const MEMORY_KIB: u32 = 64 * 1024;
    const ITERATIONS: u32 = 3;
    const PARALLELISM: u32 = 1;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum VerifierError {
        /// Not 4 to 16 digits.
        Unusable,
        /// A verifier of other parameters, or none at all.
        Malformed,
        /// The system's CSPRNG or the KDF failed.
        Failed(String),
    }

    impl fmt::Display for VerifierError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                VerifierError::Unusable => f.write_str("a PIN is 4 to 16 digits"),
                VerifierError::Malformed => f.write_str("not a PIN's verifier"),
                VerifierError::Failed(why) => write!(f, "the PIN's verifier: {why}"),
            }
        }
    }

    impl std::error::Error for VerifierError {}

    fn argon2() -> Argon2<'static> {
        let params = Params::new(MEMORY_KIB, ITERATIONS, PARALLELISM, Some(32))
            .expect("the keybag's parameters are Argon2's");
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
    }

    /// A new verifier of `pin`, with a fresh salt.
    pub fn hash_pin(pin: &Secret) -> Result<String, VerifierError> {
        if !usable_pin(pin) {
            return Err(VerifierError::Unusable);
        }
        let salt = SaltString::generate(&mut rand_core::OsRng);
        argon2()
            .hash_password(pin.expose().as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|error| VerifierError::Failed(error.to_string()))
    }

    /// Whether `pin` is the one `verifier` was made of. A verifier not of this module's
    /// parameters is refused.
    pub fn verify_pin(verifier: &str, pin: &Secret) -> Result<bool, VerifierError> {
        let hash = PasswordHash::new(verifier).map_err(|_| VerifierError::Malformed)?;
        let ours = hash.algorithm.as_str() == "argon2id"
            && hash.version == Some(0x13)
            && Params::try_from(&hash).is_ok_and(|params| {
                params.m_cost() == MEMORY_KIB
                    && params.t_cost() == ITERATIONS
                    && params.p_cost() == PARALLELISM
            })
            && hash.hash.is_some_and(|output| output.len() == 32)
            && hash.salt.is_some_and(|salt| salt.len() >= 16);
        if !ours {
            return Err(VerifierError::Malformed);
        }
        if !usable_pin(pin) {
            return Ok(false);
        }
        match argon2().verify_password(pin.expose().as_bytes(), &hash) {
            Ok(()) => Ok(true),
            Err(argon2::password_hash::Error::Password) => Ok(false),
            Err(error) => Err(VerifierError::Failed(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_is_four_to_sixteen_digits() {
        for pin in ["1234", "000000", "1234567890123456"] {
            assert!(usable_pin(&Secret::new(pin)), "{pin}");
        }
        for pin in [
            "",
            "123",
            "12345678901234567",
            "12a4",
            "١٢٣٤",
            "12 34",
            "1234\n",
        ] {
            assert!(!usable_pin(&Secret::new(pin)), "{pin:?}");
        }
    }

    #[test]
    fn the_throttle_waits_after_five_and_locks_out_at_ten() {
        for failures in 0..FREE_FAILURES {
            assert_eq!(wait_after(failures), Duration::ZERO);
        }
        assert_eq!(wait_after(5), Duration::from_secs(30));
        assert_eq!(wait_after(6), Duration::from_secs(60));
        assert_eq!(wait_after(9), Duration::from_secs(480));
        assert_eq!(wait_after(u32::MAX), Duration::from_secs(30 << 16));
        let record = |failures| PinRecord {
            verifier: String::new(),
            failures,
        };
        assert!(!record(9).locked());
        assert!(record(10).locked());
        let local = |failures| LocalSignIn {
            pin: Some(record(failures)),
            password: true,
        };
        assert_eq!(local(9).pin_state(), PinState::Set);
        assert_eq!(local(10).pin_state(), PinState::Locked);
        assert_eq!(LocalSignIn::default().pin_state(), PinState::None);
    }

    #[test]
    fn a_record_never_prints_its_verifier() {
        let record = PinRecord {
            verifier: "$argon2id$secret".into(),
            failures: 2,
        };
        assert!(!format!("{record:?}").contains("argon2"));
    }

    #[cfg(feature = "local")]
    #[test]
    fn a_verifier_checks_its_pin_and_no_other() {
        let verifier = hash_pin(&Secret::new("482916")).unwrap();
        assert!(verifier.starts_with("$argon2id$v=19$m=65536,t=3,p=1$"));
        assert!(!verifier.contains("482916"));
        assert_eq!(verify_pin(&verifier, &Secret::new("482916")), Ok(true));
        assert_eq!(verify_pin(&verifier, &Secret::new("482917")), Ok(false));
        // Not a PIN at all: no match, and no KDF run for it.
        assert_eq!(verify_pin(&verifier, &Secret::new("password")), Ok(false));
        // Each verifier its own salt.
        assert_ne!(hash_pin(&Secret::new("482916")).unwrap(), verifier);
        assert_eq!(hash_pin(&Secret::new("12")), Err(VerifierError::Unusable));
    }

    #[cfg(feature = "local")]
    #[test]
    fn a_weaker_verifier_is_refused() {
        let verifier = hash_pin(&Secret::new("482916")).unwrap();
        for weaker in [
            verifier.replace("m=65536", "m=1024"),
            verifier.replace("t=3", "t=1"),
            verifier.replace("$argon2id$", "$argon2i$"),
            "$6$salt$hash".to_owned(),
            String::new(),
        ] {
            assert_eq!(
                verify_pin(&weaker, &Secret::new("482916")),
                Err(VerifierError::Malformed),
                "{weaker}"
            );
        }
    }
}
