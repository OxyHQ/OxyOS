//! The session's authentication path ([ADR 0010](../../docs/adr/0010-lock-screen.mdx)): a password
//! checked by PAM, the system's established mechanism, for the session's own user, and the
//! throttle that slows guessing. Only an accepted password may unlock the session; this crate
//! decides nothing else.

use std::ffi::{CStr, CString};
use std::fmt;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

mod checker;
mod pam;

pub use checker::{Answer, Attempt, Checker};

/// A password on its way to PAM: overwritten in memory when dropped, never printed.
pub struct Secret(Zeroizing<Vec<u8>>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// What PAM said of a password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Accepted,
    Rejected,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The session's user has no account entry.
    NoUser,
    /// A password with a NUL byte cannot reach PAM; it is no password.
    NulInSecret,
    /// PAM did not answer: its configuration is missing or broken, or it failed. Never an unlock.
    Pam(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoUser => f.write_str("the session's user has no account entry"),
            Error::NulInSecret => f.write_str("a password cannot hold a NUL byte"),
            Error::Pam(message) => write!(f, "PAM: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// Checks passwords for one user under one PAM service.
pub struct Authenticator {
    service: CString,
    user: CString,
    confdir: Option<CString>,
}

impl Authenticator {
    /// The session's own user, found by the process's real user id (never by `$USER`, which a
    /// process inherits and could be told), under `service` from `/etc/pam.d`.
    pub fn for_session(service: &str) -> Result<Self, Error> {
        let user = user_name(rustix::process::getuid().as_raw()).ok_or(Error::NoUser)?;
        Ok(Self {
            service: CString::new(service)
                .map_err(|_| Error::Pam("a service name with a NUL".into()))?,
            user,
            confdir: None,
        })
    }

    /// `user` under `service` read from `confdir` instead of `/etc/pam.d`: for tests, with their
    /// own PAM configuration (Linux-PAM's `pam_start_confdir`).
    pub fn with_confdir(
        service: &str,
        user: &str,
        confdir: &std::path::Path,
    ) -> Result<Self, Error> {
        let text =
            |value: &str| CString::new(value).map_err(|_| Error::Pam("a name with a NUL".into()));
        Ok(Self {
            service: text(service)?,
            user: text(user)?,
            confdir: Some(text(&confdir.to_string_lossy())?),
        })
    }

    pub fn user(&self) -> &CStr {
        &self.user
    }

    /// One attempt: PAM authenticates `secret` for the user, then checks the account. It may take
    /// seconds (a failure's delay), so a session calls it off its compositor's thread.
    ///
    /// A password that must be changed still unlocks: the session already exists, and changing it
    /// is the system's business, not the lock screen's. An expired or refused account does not.
    pub fn check(&self, secret: Secret) -> Result<Outcome, Error> {
        if secret.0.contains(&0) {
            return Err(Error::NulInSecret);
        }
        // An empty password never unlocks, whatever the modules make of PAM_DISALLOW_NULL_AUTHTOK.
        if secret.0.is_empty() {
            return Ok(Outcome::Rejected);
        }
        let mut bytes = Zeroizing::new(secret.0.to_vec());
        bytes.push(0);
        let password = CStr::from_bytes_with_nul(&bytes).map_err(|_| Error::NulInSecret)?;
        let codes = pam::check(&self.service, self.confdir.as_ref(), &self.user, password)
            .map_err(Error::Pam)?;
        match (codes.authenticate, codes.account) {
            (pam::PAM_SUCCESS, Some(pam::PAM_SUCCESS | pam::PAM_NEW_AUTHTOK_REQD)) => {
                Ok(Outcome::Accepted)
            }
            // The password was right but the account may not be used (expired, refused).
            (pam::PAM_SUCCESS, Some(_)) => Ok(Outcome::Rejected),
            (
                pam::PAM_AUTH_ERR
                | pam::PAM_USER_UNKNOWN
                | pam::PAM_MAXTRIES
                | pam::PAM_PERM_DENIED,
                _,
            ) => Ok(Outcome::Rejected),
            (code, account) => Err(Error::Pam(format!(
                "{} (authenticate {code}, account {account:?})",
                codes.message
            ))),
        }
    }
}

/// The user name of `uid`, from the system's account database.
#[allow(unsafe_code)]
fn user_name(uid: u32) -> Option<CString> {
    let mut buffer = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: an all-zero passwd is a valid value of the C struct (null pointers, zero ids).
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is valid and `buffer` is as long as said; getpwuid_r fills `entry`
    // with strings inside `buffer` and sets `found` to `entry` or leaves it null.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() || entry.pw_name.is_null() {
        return None;
    }
    // SAFETY: pw_name points into `buffer`, NUL-terminated, while `buffer` lives.
    Some(unsafe { CStr::from_ptr(entry.pw_name) }.to_owned())
}

/// Slows guessing, as macOS's lock screen does: after five rejections in a row, each further
/// attempt waits, 30 s and then twice as long each time, at most 15 min. An acceptance resets it.
#[derive(Debug, Default)]
pub struct Throttle {
    rejections: u32,
    until: Option<Instant>,
}

impl Throttle {
    const FREE_ATTEMPTS: u32 = 5;
    const FIRST_WAIT: Duration = Duration::from_secs(30);
    const LONGEST_WAIT: Duration = Duration::from_secs(15 * 60);

    /// None if an attempt may be made at `now`, else how long is left to wait.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.until
            .filter(|until| *until > now)
            .map(|until| until - now)
    }

    /// Records what an attempt made at `now` came to.
    pub fn record(&mut self, outcome: Outcome, now: Instant) {
        match outcome {
            Outcome::Accepted => *self = Self::default(),
            Outcome::Rejected => {
                self.rejections = self.rejections.saturating_add(1);
                if self.rejections >= Self::FREE_ATTEMPTS {
                    let doublings = (self.rejections - Self::FREE_ATTEMPTS).min(16);
                    let wait = Self::FIRST_WAIT
                        .saturating_mul(1 << doublings)
                        .min(Self::LONGEST_WAIT);
                    self.until = Some(now + wait);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
