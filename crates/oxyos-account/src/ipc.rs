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
//!   → {"enroll": {"deviceId", "deviceSecret", "accountId", "password"}}
//!   ← {"enrolled": {"accountId", "user", "uid", "profile"}}           and the connection ends
//!   ← {"refused": {"reason", "retryAfterSeconds"?}}                     and the connection ends
//!   ← "secondFactorRequired"
//!     → {"secondFactor": {"code"}}
//!     ← {"enrolled": …} | {"refused": …}                               and the connection ends,
//!     ← {"refused": {"reason": "secondFactorInvalid"}}                  except: another code may
//!                                                                       be sent
//!   ```
//!
//!   Closing the connection abandons a setup; nothing of it is kept.
//! - **The accounts** (step 2, for the login screen and the login program, the same peers):
//!
//!   ```text
//!   → "list"
//!   ← {"accounts": [{"accountId", "user", "uid", "name", "picture"?, "state"}]}
//!                                                                     and the connection ends
//!   ```
//!
//!   `name` is what the screen calls the account (its display name, else its username), and
//!   `picture` a file of the pictures' directory; `state` is `ready`, or `signInAgain` for an
//!   account Oxy signed out, which is set up again online. Nothing secret is listed.
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
//! - Messages are received into buffers overwritten afterwards; their secrets are [`Secret`]s.

use crate::api::{DeviceCredential, OxyApi, Profile};
use crate::platform::{Checked, Local, Platform};
use crate::secret::{self, Secret};
use crate::setup::{Enrolled, Pending, Progress, Request as SetupRequest, SetupError};
use crate::store::State;
use crate::users::Users;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags,
    SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketAddrUnix, SocketFlags, SocketType,
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
        password: Secret,
    },
    SecondFactor {
        code: Secret,
    },
    List,
    Platform,
    Check {
        user: String,
        password: Secret,
        local: Local,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
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
    Accounts(Vec<Account>),
    Platform(PlatformConfig),
    Checked(Checked),
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

/// One client's setup, message by message.
pub struct Conversation<'a, A, U> {
    platform: &'a Arc<Platform<A, U>>,
    /// Oxy's API, as `platform` answers it.
    api: &'a str,
    started: bool,
    pending: Option<Pending>,
}

/// What a conversation did, for the service's log: never a secret.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Enrolled(Enrolled),
    Refused(SetupError),
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
}

impl<'a, A: OxyApi + Send + Sync + 'static, U: Users + Send + Sync + 'static>
    Conversation<'a, A, U>
{
    pub fn new(platform: &'a Arc<Platform<A, U>>, api: &'a str) -> Self {
        Self {
            platform,
            api,
            started: false,
            pending: None,
        }
    }

    /// Answers a message (`None`: one that could not be read).
    pub fn answer(&mut self, request: Option<Request>) -> (Turn, Outcome) {
        match (request, self.started, self.pending.as_ref()) {
            (
                Some(Request::Enroll {
                    device_id,
                    device_secret,
                    account_id,
                    password,
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
                };
                match self.platform.setup().enroll(request) {
                    Ok(Progress::Enrolled(enrolled)) => {
                        (Turn::End((&enrolled).into()), Outcome::Enrolled(enrolled))
                    }
                    Ok(Progress::SecondFactor(pending)) => {
                        self.pending = Some(pending);
                        (
                            Turn::Continue(Answer::SecondFactorRequired),
                            Outcome::SecondFactorRequired,
                        )
                    }
                    Err(error) => (Turn::End((&error).into()), Outcome::Refused(error)),
                }
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
            (Some(Request::SecondFactor { code }), true, Some(pending)) => {
                match self.platform.setup().second_factor(pending, &code) {
                    Ok(enrolled) => {
                        self.pending = None;
                        (Turn::End((&enrolled).into()), Outcome::Enrolled(enrolled))
                    }
                    Err(error @ SetupError::SecondFactorInvalid) => {
                        (Turn::Continue((&error).into()), Outcome::Refused(error))
                    }
                    Err(error) => {
                        self.pending = None;
                        (Turn::End((&error).into()), Outcome::Refused(error))
                    }
                }
            }
            _ => {
                self.pending = None;
                (
                    Turn::End(Answer::refused(Reason::InvalidRequest)),
                    Outcome::InvalidRequest,
                )
            }
        }
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
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let received = rustix::net::recvmsg(
            &self.fd,
            &mut [io::IoSliceMut::new(&mut packet[..])],
            &mut control,
            RecvFlags::CMSG_CLOEXEC,
        )?;
        let mut fds = Vec::new();
        for message in control.drain() {
            if let RecvAncillaryMessage::ScmRights(received) = message {
                fds.extend(received);
            }
        }
        if received.bytes == 0 && fds.is_empty() {
            return Ok(None);
        }
        if received.bytes > MAX_MESSAGE
            || received
                .flags
                .intersects(ReturnFlags::CTRUNC | ReturnFlags::TRUNC)
        {
            return Ok(Some((None, Vec::new())));
        }
        let message = serde_json::from_slice(&packet[..received.bytes]).ok();
        Ok(Some((message, fds)))
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
        let (length, _) = rustix::net::recv(&self.fd, &mut packet[..], RecvFlags::empty())?;
        if length == 0 {
            return Ok(None);
        }
        if length > MAX_MESSAGE {
            return Ok(Some(None));
        }
        Ok(Some(serde_json::from_slice(&packet[..length]).ok()))
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
        return refuse(connection, Reason::NotPermitted);
    }
    connection.set_timeout(IDLE)?;
    let mut conversation = Conversation::new(platform, api);
    loop {
        let Some(request) = connection.receive::<Request>()? else {
            return Ok(());
        };
        let (turn, outcome) = conversation.answer(request);
        log(&peer, &outcome);
        match turn {
            Turn::Continue(answer) => connection.send(&answer)?,
            Turn::End(answer) => return connection.send(&answer),
        }
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
