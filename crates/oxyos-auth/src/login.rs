//! A login's PAM transaction ([ADR 0020](../../docs/adr/0020-oxy-account-login.mdx), step 2), as
//! a login manager's session worker holds one: for one user, from the password to the end of the
//! session it opens. The login program's worker (`oxyos-login`) is its one user; it runs as root,
//! since only root may check another user's password, open a session for it (`pam_systemd`) and
//! become it.
//!
//! The order is PAM's: [`Login::authenticate`] (or [`Login::check_account`] for a session that
//! signs in with no password, the login screen's own), then [`Login::open_session`], which
//! establishes the user's credentials and opens the session, then [`Login::close_session`] when
//! it ends. Dropping the transaction closes a session left open and ends it.

use crate::pam::{self, Transaction};
use crate::{Error, Outcome, Secret};
use std::ffi::{CStr, CString};
use std::path::Path;
use zeroize::Zeroizing;

pub struct Login {
    transaction: Transaction,
    user: CString,
    /// Whether credentials were established and the session opened, so they are deleted and
    /// closed.
    credentials: bool,
    session: bool,
}

fn text(value: &str) -> Result<CString, Error> {
    CString::new(value).map_err(|_| Error::Pam("a name with a NUL".into()))
}

impl Login {
    /// Starts a transaction of `service` (from `/etc/pam.d`, or `confdir` for tests and the
    /// development path) for `user`.
    pub fn start(service: &str, user: &str, confdir: Option<&Path>) -> Result<Self, Error> {
        let user = text(user)?;
        let confdir = confdir
            .map(|dir| text(&dir.to_string_lossy()))
            .transpose()?;
        let transaction =
            Transaction::start(&text(service)?, confdir.as_deref(), &user).map_err(Error::Pam)?;
        Ok(Self {
            transaction,
            user,
            credentials: false,
            session: false,
        })
    }

    pub fn user(&self) -> &CStr {
        &self.user
    }

    fn checked(&self, code: libc::c_int, what: &str) -> Result<(), Error> {
        if code == pam::PAM_SUCCESS {
            Ok(())
        } else {
            Err(Error::Pam(format!(
                "{what}: {} ({code})",
                self.transaction.message(code)
            )))
        }
    }

    /// The terminal the session is on (`PAM_TTY`, `/dev/tty1`): `pam_systemd` takes its seat and
    /// terminal number from it.
    pub fn set_tty(&mut self, tty: &str) -> Result<(), Error> {
        let code = self.transaction.set_item(pam::PAM_TTY, &text(tty)?);
        self.checked(code, "the terminal")
    }

    /// Sets `name` to `value` in the transaction's environment, before the session opens:
    /// `pam_systemd` reads the session's class, type, seat and terminal from it.
    pub fn set_env(&mut self, name: &str, value: &str) -> Result<(), Error> {
        if name.is_empty() || name.contains('=') {
            return Err(Error::Pam(format!("{name:?} is no variable's name")));
        }
        let code = self.transaction.putenv(&text(&format!("{name}={value}"))?);
        self.checked(code, "the environment")
    }

    /// Authenticates `secret` for the user, then checks the account. A password that must be
    /// changed does not sign in: a login screen has no way to change it yet, and a session must
    /// never start with one PAM asked to change.
    pub fn authenticate(&mut self, secret: Secret) -> Result<Outcome, Error> {
        if secret.0.contains(&0) {
            return Err(Error::NulInSecret);
        }
        if secret.0.is_empty() {
            return Ok(Outcome::Rejected);
        }
        let mut bytes = Zeroizing::new(secret.0.to_vec());
        bytes.push(0);
        let password = CStr::from_bytes_with_nul(&bytes).map_err(|_| Error::NulInSecret)?;
        self.transaction.give_password(password);
        let code = self
            .transaction
            .authenticate(pam::PAM_DISALLOW_NULL_AUTHTOK);
        match code {
            pam::PAM_SUCCESS => self.check_account(),
            pam::PAM_AUTH_ERR
            | pam::PAM_USER_UNKNOWN
            | pam::PAM_MAXTRIES
            | pam::PAM_PERM_DENIED => Ok(Outcome::Rejected),
            code => self
                .checked(code, "authenticating")
                .map(|()| Outcome::Rejected),
        }
    }

    /// Checks the account alone (`pam_acct_mgmt`): whether it may be used now.
    pub fn check_account(&mut self) -> Result<Outcome, Error> {
        match self.transaction.account(0) {
            pam::PAM_SUCCESS => Ok(Outcome::Accepted),
            pam::PAM_NEW_AUTHTOK_REQD
            | pam::PAM_ACCT_EXPIRED
            | pam::PAM_PERM_DENIED
            | pam::PAM_AUTH_ERR
            | pam::PAM_USER_UNKNOWN => Ok(Outcome::Rejected),
            code => self
                .checked(code, "the account")
                .map(|()| Outcome::Rejected),
        }
    }

    /// Establishes the user's credentials and opens the session (`pam_systemd` registers it with
    /// logind). Only after an acceptance.
    pub fn open_session(&mut self) -> Result<(), Error> {
        let code = self.transaction.credentials(pam::PAM_ESTABLISH_CRED);
        self.checked(code, "the credentials")?;
        self.credentials = true;
        let code = self.transaction.open_session();
        self.checked(code, "opening the session")?;
        self.session = true;
        Ok(())
    }

    /// The environment the modules set (`XDG_SESSION_ID`, `XDG_RUNTIME_DIR`, the locale…) and
    /// what [`Login::set_env`] set, `NAME=value` each.
    pub fn environment(&self) -> Vec<(String, String)> {
        self.transaction
            .environment()
            .into_iter()
            .filter_map(|line| {
                let (name, value) = line.split_once('=')?;
                Some((name.to_owned(), value.to_owned()))
            })
            .collect()
    }

    /// Closes the session and deletes the credentials, once.
    pub fn close_session(&mut self) -> Result<(), Error> {
        let mut result = Ok(());
        if std::mem::take(&mut self.session) {
            let code = self.transaction.close_session();
            result = self.checked(code, "closing the session");
        }
        if std::mem::take(&mut self.credentials) {
            let code = self.transaction.credentials(pam::PAM_DELETE_CRED);
            result = result.and(self.checked(code, "deleting the credentials"));
        }
        result
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        if let Err(error) = self.close_session() {
            eprintln!("[auth] {error}");
        }
    }
}
