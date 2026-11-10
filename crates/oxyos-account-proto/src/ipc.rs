//! The accounts service's socket (`/run/oxyos/accountd/socket`): its messages, and the clients'
//! side of asking it (`oxyos_account::ipc`).

use crate::api::Profile;
use crate::pin::PinState;
use crate::platform::{Checked, Local};
use crate::setup::{Appearance, Stage};
use crate::store::State;
use crate::Secret;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;
use std::time::Duration;

/// Where the service listens.
pub const SOCKET: &str = "/run/oxyos/accountd/socket";

/// The longest message: a password of PAM's 512 bytes, escaped, and the rest, many times over.
pub const MAX_MESSAGE: usize = 16 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Request {
    #[serde(rename_all = "camelCase")]
    Enroll {
        device_id: String,
        device_secret: Secret,
        account_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<Secret>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pin: Option<Secret>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        appearance: Option<Appearance>,
    },
    SecondFactor {
        code: Secret,
    },
    /// The setup again, after a `failed` answer.
    Retry,
    List,
    Platform,
    Check {
        user: String,
        password: Secret,
        local: Local,
    },
    /// The login program's, once PAM accepted `password` for `user` (root alone).
    UnlockKeychain {
        user: String,
        password: Secret,
    },
    /// `oxyos-pin-check`'s: the peer's own account's PIN, or any one's as root.
    VerifyPin {
        user: String,
        pin: Secret,
    },
}

/// An enrolled account as `list` gives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub account_id: String,
    pub user: String,
    pub uid: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// Absent in what an older service lists: `ready`.
    #[serde(default = "ready")]
    pub state: State,
    /// Its PIN; absent in what an older service lists: none.
    #[serde(default = "no_pin")]
    pub pin: PinState,
    /// Whether its password signs in here; absent in what an older service lists: it does.
    #[serde(default = "signs_in")]
    pub password: bool,
}

fn no_pin() -> PinState {
    PinState::None
}

fn signs_in() -> bool {
    true
}

fn ready() -> State {
    State::Ready
}

/// Where Oxy is, for the login screen's setup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformConfig {
    pub api: String,
    pub client_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    NotConfigured,
    NotPermitted,
    InvalidRequest,
    InvalidAccount,
    UnusablePassword,
    UnusablePin,
    PinRequired,
    Offline,
    CredentialRefused,
    AccountNotOnDevice,
    WrongPassword,
    SecondFactorInvalid,
    Locked,
    AccountMismatch,
    NoUsername,
    Oxy,
    Conflict,
    Failed,
}

/// Why a setup failed in a way it may try again, its sign-in kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Failure {
    /// Oxy could not be reached, or failed for a moment.
    Unreachable,
    /// The machine could not be set up; what the setup did on it was undone.
    LocalFailure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
    /// A setup's step started; its answer follows.
    Progress(Stage),
    /// Oxy asked the setup to wait this long (a 429 on the redemption): it asks again after, and
    /// its steps and answer follow.
    #[serde(rename_all = "camelCase")]
    Waiting {
        retry_after_seconds: u64,
    },
    SecondFactorRequired,
    #[serde(rename_all = "camelCase")]
    Enrolled {
        account_id: String,
        user: String,
        uid: u32,
        profile: Profile,
    },
    #[serde(rename_all = "camelCase")]
    Refused {
        reason: Reason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_seconds: Option<u64>,
    },
    /// The setup failed, its sign-in kept: `"retry"` (or, at the second factor, a code).
    Failed {
        reason: Failure,
    },
    Accounts(Vec<Account>),
    Platform(PlatformConfig),
    Checked(Checked),
    Keychain(crate::keychain::KeychainState),
    Pin(PinAnswer),
}

/// What a PIN came to, as `oxyos-pin-check` is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinAnswer {
    pub result: PinResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PinResult {
    Accepted,
    Rejected,
    Throttled,
    Locked,
    None,
}

impl Answer {
    pub fn refused(reason: Reason) -> Self {
        Answer::Refused {
            reason,
            retry_after_seconds: None,
        }
    }
}

/// The service's socket's mode: any process may connect; the peer's credentials decide what it
/// may do.
pub const MODE: u32 = 0o666;

/// The service's listening socket: the services' own (`oxyos-service-socket`), at this service's
/// bound, [`MAX_MESSAGE`]. Bound with [`MODE`].
pub type Listener = oxyos_service_socket::BoundedListener<MAX_MESSAGE>;

/// A connection to or from the service, at its bound.
pub type Connection = oxyos_service_socket::BoundedConnection<MAX_MESSAGE>;

/// How long `oxyos-pin-check` waits for its answer (Argon2id at 64 MiB).
pub const PIN_WAIT: Duration = Duration::from_secs(20);

/// `user`'s PIN, checked by the service at `socket` (`oxyos-pin-check`'s).
pub fn verify_pin_at(socket: &Path, user: &str, pin: Secret) -> io::Result<PinAnswer> {
    let request = Request::VerifyPin {
        user: user.to_owned(),
        pin,
    };
    match ask(socket, &request, PIN_WAIT)? {
        Answer::Pin(answer) => Ok(answer),
        _ => Err(unexpected("verifyPin")),
    }
}

/// How long the login screen and the login program wait for the accounts.
pub const LIST_WAIT: Duration = Duration::from_secs(5);

/// How long a check waits for the service: it may ask Oxy, a few calls each bounded.
pub const CHECK_WAIT: Duration = Duration::from_secs(45);

/// One request to the service at `socket`, its one answer within `wait`.
fn ask(socket: &Path, request: &Request, wait: Duration) -> io::Result<Answer> {
    let connection = Connection::connect(socket)?;
    connection.set_timeout(wait)?;
    connection.send(request)?;
    match connection.receive::<Answer>()? {
        Some(Some(Answer::Refused { reason, .. })) => Err(io::Error::new(
            if reason == Reason::NotPermitted {
                io::ErrorKind::PermissionDenied
            } else {
                io::ErrorKind::Other
            },
            format!("refused: {reason:?}"),
        )),
        Some(Some(answer)) => Ok(answer),
        Some(None) => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
        None => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the service closed the connection",
        )),
    }
}

fn unexpected(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("not an answer to {what}"),
    )
}

/// The enrolled accounts, from the service at `socket`: for the login screen (they are what it
/// shows) and the login program (only an enrolled account's user may sign in). Fails if the
/// service does not answer, or refuses.
pub fn list(socket: &Path) -> io::Result<Vec<Account>> {
    match ask(socket, &Request::List, LIST_WAIT)? {
        Answer::Accounts(accounts) => Ok(accounts),
        _ => Err(unexpected("list")),
    }
}

/// Where Oxy is, from the service at `socket` (the login screen's setup).
pub fn platform(socket: &Path) -> io::Result<PlatformConfig> {
    match ask(socket, &Request::Platform, LIST_WAIT)? {
        Answer::Platform(platform) => Ok(platform),
        _ => Err(unexpected("platform")),
    }
}

/// How long the login program waits for the keychain's unlock (Argon2id at 64 MiB).
pub const UNLOCK_WAIT: Duration = Duration::from_secs(30);

/// Unlocks `user`'s keychain with the password PAM accepted, at the service at `socket` (the login
/// program's, as root).
pub fn unlock_keychain(
    socket: &Path,
    user: &str,
    password: Secret,
) -> io::Result<crate::keychain::KeychainState> {
    let request = Request::UnlockKeychain {
        user: user.to_owned(),
        password,
    };
    match ask(socket, &request, UNLOCK_WAIT)? {
        Answer::Keychain(state) => Ok(state),
        _ => Err(unexpected("unlockKeychain")),
    }
}

/// The online check of `user`'s password after PAM said `local`, by the service at `socket` (the
/// login program's). Fails if the service does not answer: the login program then refuses.
pub fn check(socket: &Path, user: &str, password: Secret, local: Local) -> io::Result<Checked> {
    let request = Request::Check {
        user: user.to_owned(),
        password,
        local,
    };
    match ask(socket, &request, CHECK_WAIT)? {
        Answer::Checked(checked) => Ok(checked),
        _ => Err(unexpected("check")),
    }
}
