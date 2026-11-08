//! The service's socket, which the setup client (the login screen's, ADR 0020 decision 4) talks
//! to: `/run/oxyos/accountd/socket`.
//!
//! - **Transport:** a `SOCK_SEQPACKET` Unix socket; each packet is one JSON message of at most
//!   [`MAX_MESSAGE`] bytes. A longer or undecodable one ends the conversation.
//! - **Who may enroll:** the peer's credentials come from the kernel (`SO_PEERCRED`), never from
//!   what it sends: root, or the login program's user ([`Enrollers`]). Anyone else is answered
//!   `notPermitted` and nothing else, once its first message is read ([`refuse`]).
//! - **Every answer follows the message it answers.** The service never closes a connection with a
//!   message of the client's unread: Linux resets a Unix socket closed with its receive queue not
//!   empty, and the client would read `ECONNRESET` instead of the answer before it.
//! - **A conversation** is one setup on one connection:
//!
//!   ```text
//!   → {"enroll": {"deviceId", "deviceSecret", "accountId", "password"?, "pin"?,
//!                 "appearance"?: "light" | "dark" | "auto"}}
//!   ← {"progress": "securing" | "verifying" | "preparing" | "signingIn" | "finishing"}
//!                                                                     as each step starts, then:
//!   ← {"enrolled": {"accountId", "user", "uid", "profile"}}           and the connection ends
//!   ← {"refused": {"reason", "retryAfterSeconds"?}}                     and the connection ends
//!   ← {"failed": {"reason": "unreachable" | "localFailure"}}           the sign-in is kept:
//!     → "retry"
//!     ← progress, then as `enroll`'s answer
//!   ← "secondFactorRequired"
//!     → {"secondFactor": {"code"}}
//!     ← progress, then {"enrolled": …} | {"refused": …}                and the connection ends,
//!     ← {"refused": {"reason": "secondFactorInvalid"}}                  except: another code may
//!     ← {"failed": {"reason": "unreachable"}}                           be sent
//!     ← {"failed": {"reason": "localFailure"}}                          or "retry"
//!   ```
//!
//!   The answers a setup can end in, for the login screen:
//!
//!   - `enrolled`: set up.
//!   - `failed: unreachable`: Oxy could not be reached, or failed for a moment, even after the
//!     service asked again. The sign-in is kept (the QR's proof unspent, or redeemed with this
//!     setup's key and redeemable again with it, Oxy ADR 0058 R7): try again (`"retry"`), never a
//!     second scan.
//!   - `failed: localFailure`: the machine could not be set up (its user, its file, its
//!     verifier); what this setup did on it was undone. The sign-in is kept, and the platform
//!     credential Oxy issued, if it did, too: `"retry"` sets the machine up with it without asking
//!     Oxy again.
//!   - `refused: credentialRefused` (also `accountNotOnDevice`, `accountMismatch`): the sign-in no
//!     longer holds (the QR's proof refused or already spent, its window passed): sign in again.
//!   - any other `refused` (`conflict`, `failed` when undoing failed too, `oxy`, `locked`, …):
//!     the setup ends as it said.
//!
//!   A `"retry"` is answered only after a `failed` answer; anything else out of turn ends the
//!   conversation, `invalidRequest`. Closing the connection abandons a setup: nothing of it is
//!   kept, and a platform credential it was issued and never set up with is ended online, as it is
//!   when the conversation's turn times out ([`IDLE`]) or a refusal ends it. `pin` (4 to 16
//!   digits, never the password) becomes the account's PIN on this computer (`crate::pin`).
//!   Without `password` (a sign-in by the QR) the PIN is required (`pinRequired` without one) and
//!   Oxy's device-proof route issues the platform credential (`crate::setup`). `appearance` seeds
//!   the preferences of a home this setup makes (an account set up again keeps its own); any other
//!   value is not a request. A progress message is a step's name and nothing else.
//! - **The accounts** (step 2, for the login screen and the login program, the same peers):
//!
//!   ```text
//!   → "list"
//!   ← {"accounts": [{"accountId", "user", "uid", "name", "picture"?, "state", "pin",
//!                    "password"}]}
//!                                                                     and the connection ends
//!   ```
//!
//!   `name` is what the screen calls the account (its display name, else its username), and
//!   `picture` a file of the pictures' directory; `state` is `ready`, or `signInAgain` for an
//!   account Oxy signed out, which is set up again online; `pin` is `none`, `set` or `locked`
//!   (too many wrong PINs), and `password` whether the password signs in on this computer: which
//!   field the screen shows. Nothing secret is listed.
//! - **Where Oxy is** (the login screen's setup reads it; nothing secret):
//!
//!   ```text
//!   → "platform"
//!   ← {"platform": {"api", "clientId"}}                                and the connection ends
//!   ```
//!
//!   `clientId` is OxyOS's public client id, `null` when none is configured.
//! - **The online check** (step 3; the login program's, after its worker's PAM answer):
//!
//!   ```text
//!   → {"check": {"user": "oxy-…", "password", "local": "accepted" | "rejected"}}
//!   ← {"checked": {"result": "accepted" | "rejected",
//!                  "reason"?: "passwordChanged" | "signInAgain"}}       and the connection ends
//!   ```
//! - **The keychain's unlock** (ADR 0050 step 2; the login program's, as root alone, once PAM
//!   accepted the password; the login screen's user may not):
//!
//!   ```text
//!   → {"unlockKeychain": {"user": "oxy-…", "password"}}
//!   ← {"keychain": "off" | "member" | … | "needsPreviousPassword" | "locked"}
//!                                                                     and the connection ends
//!   ```
//! - **A PIN** (`crate::pin`; `oxyos-pin-check`, which PAM runs for the login program's and the
//!   lock's PIN, as their own user): any peer may ask for its own account's, root for any
//!   enrolled account's; everyone else is answered `none`, as an account without a PIN is.
//!
//!   ```text
//!   → {"verifyPin": {"user": "oxy-…", "pin"}}
//!   ← {"pin": {"result": "accepted" | "rejected" | "throttled" | "locked" | "none",
//!              "retryAfterSeconds"?}}                                   and the connection ends
//!   ```
//!
//!   It is checked here, never online, and each wrong one counted.
//! - Messages are received into buffers overwritten afterwards; their secrets are [`Secret`]s.

use crate::api::{DeviceCredential, OxyApi, Profile};
use crate::pin::{PinCheck, PinState};
use crate::platform::{Checked, Local, Platform};
use crate::secret::{self, Secret};
use crate::setup::{
    Appearance, Attempt, Enrolled, Request as SetupRequest, SetupError, Stage, Step,
};
use crate::store::State;
use crate::users::Users;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::net::{
    AddressFamily, RecvFlags, SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketAddrUnix,
    SocketFlags, SocketType,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io;
use std::mem::MaybeUninit;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use zeroize::Zeroizing;

/// Where the service listens.
pub const SOCKET: &str = "/run/oxyos/accountd/socket";

/// The longest message: a password of PAM's 512 bytes, escaped, and the rest, many times over.
pub const MAX_MESSAGE: usize = 16 * 1024;

/// How long a conversation may wait for the client (a person typing an authenticator's code).
pub const IDLE: Duration = Duration::from_secs(5 * 60);

/// How long a peer that may not enroll is waited for, to read what it sends before refusing it.
pub const REFUSAL_WAIT: Duration = Duration::from_secs(5);

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

impl From<&crate::store::Listed> for Account {
    fn from(listed: &crate::store::Listed) -> Self {
        Account {
            account_id: listed.account.as_str().to_owned(),
            user: listed.user.as_str().to_owned(),
            uid: listed.uid,
            name: listed.name.clone(),
            picture: listed
                .picture
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            state: listed.state,
            pin: listed.pin,
            password: listed.password,
        }
    }
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

impl From<PinCheck> for PinAnswer {
    fn from(check: PinCheck) -> Self {
        let seconds = |wait: Duration| (!wait.is_zero()).then(|| wait.as_secs().max(1));
        let (result, retry_after_seconds) = match check {
            PinCheck::Accepted => (PinResult::Accepted, None),
            PinCheck::Rejected { locked: true, .. } => (PinResult::Locked, None),
            PinCheck::Rejected { wait, .. } => (PinResult::Rejected, seconds(wait)),
            PinCheck::Throttled(wait) => (PinResult::Throttled, seconds(wait)),
            PinCheck::Locked => (PinResult::Locked, None),
            PinCheck::NoPin => (PinResult::None, None),
        };
        PinAnswer {
            result,
            retry_after_seconds,
        }
    }
}

impl Answer {
    pub fn refused(reason: Reason) -> Self {
        Answer::Refused {
            reason,
            retry_after_seconds: None,
        }
    }
}

impl From<&Enrolled> for Answer {
    fn from(enrolled: &Enrolled) -> Self {
        Answer::Enrolled {
            account_id: enrolled.account.as_str().to_owned(),
            user: enrolled.user.as_str().to_owned(),
            uid: enrolled.uid,
            profile: enrolled.profile.clone(),
        }
    }
}

impl From<&SetupError> for Answer {
    fn from(error: &SetupError) -> Self {
        let reason = match error {
            SetupError::NotConfigured => Reason::NotConfigured,
            SetupError::InvalidAccount => Reason::InvalidAccount,
            SetupError::UnusablePassword => Reason::UnusablePassword,
            SetupError::UnusablePin => Reason::UnusablePin,
            SetupError::PinRequired => Reason::PinRequired,
            SetupError::Offline => Reason::Offline,
            SetupError::CredentialRefused => Reason::CredentialRefused,
            SetupError::AccountNotOnDevice => Reason::AccountNotOnDevice,
            SetupError::WrongPassword => Reason::WrongPassword,
            SetupError::SecondFactorInvalid => Reason::SecondFactorInvalid,
            SetupError::Locked { retry_after } => {
                return Answer::Refused {
                    reason: Reason::Locked,
                    retry_after_seconds: *retry_after,
                }
            }
            SetupError::AccountMismatch => Reason::AccountMismatch,
            SetupError::NoUsername => Reason::NoUsername,
            SetupError::Oxy(_) => Reason::Oxy,
            SetupError::Conflict(_) => Reason::Conflict,
            SetupError::Failed(_) | SetupError::RollbackFailed { .. } => Reason::Failed,
        };
        Answer::refused(reason)
    }
}

/// Who may enroll an account: root, and the login program's user when it has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Enrollers {
    pub login: Option<u32>,
}

impl Enrollers {
    pub fn allow(&self, uid: u32) -> bool {
        uid == 0 || Some(uid) == self.login
    }
}

/// What a turn of the conversation came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Turn {
    /// Answer, and wait for the client's next message.
    Continue(Answer),
    /// Answer, and end the conversation.
    End(Answer),
}

/// One client's setup, message by message. Dropped with a setup that did not finish, it ends
/// what the setup still holds ([`crate::setup::Setup::abandon`]).
pub struct Conversation<'a, A: OxyApi + Send + Sync + 'static, U: Users + Send + Sync + 'static> {
    platform: &'a Arc<Platform<A, U>>,
    /// Oxy's API, as `platform` answers it.
    api: &'a str,
    started: bool,
    /// The setup under way: sent, not ended.
    attempt: Option<Attempt>,
}

/// What a conversation did, for the service's log: never a secret.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Enrolled(Enrolled),
    Refused(SetupError),
    /// The setup failed in a way it may try again; its sign-in is kept.
    Interrupted(SetupError),
    SecondFactorRequired,
    InvalidRequest,
    /// The accounts were listed: how many, and the files that are no account's.
    Listed {
        accounts: usize,
        broken: Vec<String>,
    },
    /// The accounts could not be read.
    Unlisted(String),
    /// Where Oxy is was asked.
    Platform,
    /// A password was checked online for `user`.
    Checked {
        user: String,
        checked: Checked,
    },
    /// The login program unlocked `user`'s keychain.
    KeychainUnlocked {
        user: String,
        state: crate::keychain::KeychainState,
    },
    /// `user`'s PIN was checked.
    PinChecked {
        user: String,
        result: PinResult,
    },
}

impl<'a, A: OxyApi + Send + Sync + 'static, U: Users + Send + Sync + 'static>
    Conversation<'a, A, U>
{
    pub fn new(platform: &'a Arc<Platform<A, U>>, api: &'a str) -> Self {
        Self {
            platform,
            api,
            started: false,
            attempt: None,
        }
    }

    /// Answers a message (`None`: one that could not be read).
    pub fn answer(&mut self, request: Option<Request>) -> (Turn, Outcome) {
        self.answer_with(request, &mut |_| {})
    }

    /// Answers a message, each step of a setup reported to `progress` as it starts.
    pub fn answer_with(
        &mut self,
        request: Option<Request>,
        progress: &mut dyn FnMut(Stage),
    ) -> (Turn, Outcome) {
        let asked = self
            .attempt
            .as_ref()
            .map(|attempt| attempt.second_factor_asked());
        match (request, self.started, asked) {
            (
                Some(Request::Enroll {
                    device_id,
                    device_secret,
                    account_id,
                    password,
                    pin,
                    appearance,
                }),
                false,
                _,
            ) => {
                self.started = true;
                let request = SetupRequest {
                    credential: DeviceCredential {
                        device_id,
                        device_secret,
                    },
                    account: account_id,
                    password,
                    pin,
                    appearance,
                };
                let setup = self.platform.setup();
                match setup.start(request) {
                    Ok(mut attempt) => {
                        let result = setup.advance(&mut attempt, progress);
                        self.attempt = Some(attempt);
                        self.after(result)
                    }
                    Err(error) => (Turn::End((&error).into()), Outcome::Refused(error)),
                }
            }
            (Some(Request::Retry), true, Some(false)) => {
                let attempt = self
                    .attempt
                    .as_mut()
                    .expect("a setup under way, as matched");
                let result = self.platform.setup().advance(attempt, progress);
                self.after(result)
            }
            (Some(Request::List), false, _) => {
                self.started = true;
                match self.platform.setup().store().list() {
                    Ok((accounts, broken)) => (
                        Turn::End(Answer::Accounts(
                            accounts.iter().map(Account::from).collect(),
                        )),
                        Outcome::Listed {
                            accounts: accounts.len(),
                            broken,
                        },
                    ),
                    Err(error) => (
                        Turn::End(Answer::refused(Reason::Failed)),
                        Outcome::Unlisted(error.to_string()),
                    ),
                }
            }
            (Some(Request::Platform), false, _) => {
                self.started = true;
                let answer = Answer::Platform(PlatformConfig {
                    api: self.api.to_owned(),
                    client_id: self
                        .platform
                        .setup()
                        .platform()
                        .map(|client| client.client_id.clone()),
                });
                (Turn::End(answer), Outcome::Platform)
            }
            (
                Some(Request::Check {
                    user,
                    password,
                    local,
                }),
                false,
                _,
            ) => {
                self.started = true;
                let checked = self.platform.check(&user, password, local);
                (
                    Turn::End(Answer::Checked(checked)),
                    Outcome::Checked { user, checked },
                )
            }
            (Some(Request::SecondFactor { code }), true, Some(true)) => {
                let attempt = self
                    .attempt
                    .as_mut()
                    .expect("a setup under way, as matched");
                let result = self
                    .platform
                    .setup()
                    .second_factor_of(attempt, &code, progress)
                    .map(Step::Enrolled);
                self.after(result)
            }
            _ => {
                self.abandon();
                (
                    Turn::End(Answer::refused(Reason::InvalidRequest)),
                    Outcome::InvalidRequest,
                )
            }
        }
    }

    /// Where a setup's turn leaves the conversation: set up, a code asked for, a failure it may
    /// try again (its sign-in kept), or a refusal that ends it (what the setup held ended).
    fn after(&mut self, result: Result<Step, SetupError>) -> (Turn, Outcome) {
        match result {
            Ok(Step::Enrolled(enrolled)) => {
                self.attempt = None;
                (Turn::End((&enrolled).into()), Outcome::Enrolled(enrolled))
            }
            Ok(Step::SecondFactor) => (
                Turn::Continue(Answer::SecondFactorRequired),
                Outcome::SecondFactorRequired,
            ),
            Err(error @ SetupError::SecondFactorInvalid) => {
                (Turn::Continue((&error).into()), Outcome::Refused(error))
            }
            Err(error @ (SetupError::Offline | SetupError::Failed(_))) => {
                let reason = match error {
                    SetupError::Offline => Failure::Unreachable,
                    _ => Failure::LocalFailure,
                };
                (
                    Turn::Continue(Answer::Failed { reason }),
                    Outcome::Interrupted(error),
                )
            }
            Err(error) => {
                self.abandon();
                (Turn::End((&error).into()), Outcome::Refused(error))
            }
        }
    }

    /// Ends the setup under way, if any, and what it holds.
    fn abandon(&mut self) {
        if let Some(mut attempt) = self.attempt.take() {
            self.platform.setup().abandon(&mut attempt);
        }
    }
}

impl<A: OxyApi + Send + Sync + 'static, U: Users + Send + Sync + 'static> Drop
    for Conversation<'_, A, U>
{
    fn drop(&mut self) {
        self.abandon();
    }
}

/// The service's listening socket.
pub struct Listener {
    fd: OwnedFd,
}

impl Listener {
    /// Listens at `path`, replacing a socket an earlier run left there (never another file). Its
    /// mode lets any process connect: the peer's credentials decide what it may do. The socket
    /// is made listening under a name of its own beside `path` and renamed into place, so `path`
    /// is never a socket that refuses: a client that connects as the name appears (the login
    /// program, the login screen) would otherwise be refused between the bind and the listen.
    pub fn bind(path: &Path) -> io::Result<Self> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_socket() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a socket", path.display()),
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "a socket needs a name"))?;
        let making = path.with_file_name(format!(
            ".{}.{}",
            name.to_string_lossy(),
            std::process::id()
        ));
        // A run of this process's that ended between its bind and its rename left it.
        if std::fs::symlink_metadata(&making).is_ok_and(|metadata| metadata.file_type().is_socket())
        {
            std::fs::remove_file(&making)?;
        }
        let fd = rustix::net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        let made = (|| -> io::Result<()> {
            rustix::net::bind(&fd, &SocketAddrUnix::new(&making)?)?;
            std::fs::set_permissions(&making, std::fs::Permissions::from_mode(0o666))?;
            rustix::net::listen(&fd, 16)?;
            std::fs::rename(&making, path)
        })();
        if let Err(error) = made {
            let _ = std::fs::remove_file(&making);
            return Err(error);
        }
        Ok(Self { fd })
    }

    pub fn accept(&self) -> io::Result<Connection> {
        let fd = rustix::net::accept_with(&self.fd, SocketFlags::CLOEXEC)?;
        Ok(Connection { fd })
    }
}

impl AsFd for Listener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// The peer's identity, from the kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

pub struct Connection {
    fd: OwnedFd,
}

impl Connection {
    pub fn connect(path: &Path) -> io::Result<Self> {
        let fd = rustix::net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        rustix::net::connect(&fd, &SocketAddrUnix::new(path)?)?;
        Ok(Self { fd })
    }

    /// Two connected ends (the tests' stand-in for a peer).
    #[cfg(test)]
    pub(crate) fn pair() -> io::Result<(Self, Self)> {
        let (a, b) = rustix::net::socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        Ok((Self { fd: a }, Self { fd: b }))
    }

    /// A connection on a descriptor the tests made.
    #[cfg(test)]
    pub(crate) fn from_fd(fd: OwnedFd) -> Self {
        Self { fd }
    }

    /// Another handle on the same connection (a duplicated descriptor), to send from one thread
    /// while another waits to receive.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            fd: rustix::io::fcntl_dupfd_cloexec(&self.fd, 0)?,
        })
    }

    /// Waits at most `timeout` to send each message.
    pub fn set_send_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        use rustix::net::sockopt::{set_socket_timeout, Timeout};
        set_socket_timeout(&self.fd, Timeout::Send, timeout)?;
        Ok(())
    }

    pub fn peer(&self) -> io::Result<Peer> {
        let credentials = rustix::net::sockopt::socket_peercred(&self.fd)?;
        Ok(Peer {
            pid: credentials.pid.as_raw_nonzero().get(),
            uid: credentials.uid.as_raw(),
            gid: credentials.gid.as_raw(),
        })
    }

    /// The peer, as a pidfd of the process that connected (`SO_PEERPIDFD`).
    pub fn peer_pidfd(&self) -> io::Result<OwnedFd> {
        crate::pidfd::peer_pidfd(self.fd.as_fd())
    }

    /// Waits at most `timeout` for each message (none: as long as it takes).
    pub fn set_receive_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        use rustix::net::sockopt::{set_socket_timeout, Timeout};
        set_socket_timeout(&self.fd, Timeout::Recv, timeout)?;
        Ok(())
    }

    /// Sends `message` with a descriptor (`SCM_RIGHTS`).
    pub fn send_with(&self, message: &impl Serialize, fd: BorrowedFd<'_>) -> io::Result<()> {
        let packet = secret::json(message).map_err(io::Error::other)?;
        if packet.len() > MAX_MESSAGE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "message too long",
            ));
        }
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        let fds = [fd];
        if !control.push(SendAncillaryMessage::ScmRights(&fds)) {
            return Err(io::Error::other("no room for the descriptor"));
        }
        rustix::net::sendmsg(
            &self.fd,
            &[io::IoSlice::new(&packet)],
            &mut control,
            SendFlags::NOSIGNAL,
        )?;
        Ok(())
    }

    /// The next message and the descriptors it carried (`SCM_RIGHTS`, close-on-exec): `None` once
    /// the peer has closed, a `None` message for one that is too long, is not a message, or
    /// carried more descriptors than one (those are closed).
    pub fn receive_with<T: DeserializeOwned>(
        &self,
    ) -> io::Result<Option<(Option<T>, Vec<OwnedFd>)>> {
        let mut packet = Zeroizing::new(vec![0u8; MAX_MESSAGE + 1]);
        let Some(received) =
            oxyos_seqpacket::receive(self.fd.as_fd(), &mut packet[..], true, RecvFlags::empty())?
        else {
            return Ok(None);
        };
        if received.length > MAX_MESSAGE
            || received.truncated
            || received.descriptors_truncated
            || received.descriptors.len() > 1
        {
            return Ok(Some((None, Vec::new())));
        }
        let message = serde_json::from_slice(&packet[..received.length]).ok();
        Ok(Some((message, received.descriptors)))
    }

    pub fn set_timeout(&self, timeout: Duration) -> io::Result<()> {
        use rustix::net::sockopt::{set_socket_timeout, Timeout};
        set_socket_timeout(&self.fd, Timeout::Recv, Some(timeout))?;
        set_socket_timeout(&self.fd, Timeout::Send, Some(timeout))?;
        Ok(())
    }

    pub fn send(&self, message: &impl Serialize) -> io::Result<()> {
        let packet = secret::json(message).map_err(io::Error::other)?;
        if packet.len() > MAX_MESSAGE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "message too long",
            ));
        }
        rustix::net::send(&self.fd, &packet, SendFlags::NOSIGNAL)?;
        Ok(())
    }

    /// The next message: `None` once the peer has closed, `Some(None)` for one that is too long
    /// or is not a message.
    pub fn receive<T: DeserializeOwned>(&self) -> io::Result<Option<Option<T>>> {
        let mut packet = Zeroizing::new(vec![0u8; MAX_MESSAGE + 1]);
        // A packet longer than the buffer is cut to it: one byte more than a message can be.
        let Some(received) =
            oxyos_seqpacket::receive(self.fd.as_fd(), &mut packet[..], false, RecvFlags::empty())?
        else {
            return Ok(None);
        };
        if received.length > MAX_MESSAGE || received.truncated {
            return Ok(Some(None));
        }
        Ok(Some(
            serde_json::from_slice(&packet[..received.length]).ok(),
        ))
    }
}

impl AsFd for Connection {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// Answers a connection's first message with a refusal, without looking at it; if the peer sends
/// nothing within [`REFUSAL_WAIT`] or closes, nothing is answered.
pub fn refuse(connection: &Connection, reason: Reason) -> io::Result<()> {
    connection.set_timeout(REFUSAL_WAIT)?;
    match connection.receive::<serde::de::IgnoredAny>()? {
        Some(_) => connection.send(&Answer::refused(reason)),
        None => Ok(()),
    }
}

/// Runs one client's conversation to its end, if `enrollers` allows the peer. Each outcome is
/// handed to `log`. `api` is Oxy's API, as `platform` tells the login screen.
pub fn serve<A: OxyApi + Send + Sync + 'static, U: Users + Send + Sync + 'static>(
    connection: &Connection,
    platform: &Arc<Platform<A, U>>,
    api: &str,
    enrollers: Enrollers,
    mut log: impl FnMut(&Peer, &Outcome),
) -> io::Result<()> {
    let peer = connection.peer()?;
    if !enrollers.allow(peer.uid) {
        // A PIN is anyone's to check for its own account; nothing else is theirs.
        connection.set_timeout(REFUSAL_WAIT)?;
        return match connection.receive::<Request>()? {
            None => Ok(()),
            Some(Some(Request::VerifyPin { user, pin })) => {
                let answer = verify_pin(platform, &peer, &user, &pin);
                log(
                    &peer,
                    &Outcome::PinChecked {
                        user,
                        result: answer.result,
                    },
                );
                connection.send(&Answer::Pin(answer))
            }
            Some(_) => connection.send(&Answer::refused(Reason::NotPermitted)),
        };
    }
    connection.set_timeout(IDLE)?;
    let mut conversation = Conversation::new(platform, api);
    loop {
        let Some(request) = connection.receive::<Request>()? else {
            return Ok(());
        };
        // The keychain's unlock is the login program's alone, which runs as root: never the login
        // screen's user, whose screen runs JavaScript (ADR 0050 decision 2).
        if let Some(Request::UnlockKeychain { user, password }) = request {
            let answer = match (
                peer.uid,
                platform.keychains(),
                platform.setup().store().get_user(&user),
            ) {
                (0, Some(keychains), Ok(Some(enrollment))) => {
                    let state = keychains.unlock(enrollment.uid, &user, &password);
                    log(
                        &peer,
                        &Outcome::KeychainUnlocked {
                            user: user.clone(),
                            state,
                        },
                    );
                    Answer::Keychain(state)
                }
                (0, None, _) => Answer::Keychain(crate::keychain::KeychainState::Locked),
                (0, _, _) => Answer::refused(Reason::InvalidAccount),
                _ => Answer::refused(Reason::NotPermitted),
            };
            return connection.send(&answer);
        }
        if let Some(Request::VerifyPin { user, pin }) = request {
            let answer = verify_pin(platform, &peer, &user, &pin);
            log(
                &peer,
                &Outcome::PinChecked {
                    user,
                    result: answer.result,
                },
            );
            return connection.send(&Answer::Pin(answer));
        }
        // Each step of a setup is told as it starts; a client gone is found out by the answer.
        let (turn, outcome) = conversation.answer_with(request, &mut |stage| {
            let _ = connection.send(&Answer::Progress(stage));
        });
        log(&peer, &outcome);
        match turn {
            Turn::Continue(answer) => connection.send(&answer)?,
            Turn::End(answer) => return connection.send(&answer),
        }
    }
}

/// `user`'s PIN for `peer`: root's for any enrolled account, anyone else's for its own alone (an
/// account of another uid is answered as one without a PIN, nothing checked or counted).
fn verify_pin<A: OxyApi + Send + Sync + 'static, U: Users + Send + Sync + 'static>(
    platform: &Arc<Platform<A, U>>,
    peer: &Peer,
    user: &str,
    pin: &Secret,
) -> PinAnswer {
    let own = peer.uid == 0
        || platform
            .setup()
            .store()
            .get_user(user)
            .is_ok_and(|enrollment| enrollment.is_some_and(|e| e.uid == peer.uid));
    if !own {
        return PinCheck::NoPin.into();
    }
    #[cfg(feature = "local")]
    {
        platform.check_pin(user, pin).into()
    }
    #[cfg(not(feature = "local"))]
    {
        let _ = pin;
        PinCheck::NoPin.into()
    }
}

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
