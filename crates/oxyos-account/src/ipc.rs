//! The service's socket, which the setup client (the login screen's, ADR 0020 decision 4) talks
//! to: `/run/oxyos/accountd/socket`.
//!
//! - **Transport:** a `SOCK_SEQPACKET` Unix socket; each packet is one JSON message of at most
//!   [`MAX_MESSAGE`] bytes. A longer or undecodable one ends the conversation.
//! - **Who may enroll:** the peer's credentials come from the kernel (`SO_PEERCRED`), never from
//!   what it sends: root, or the login program's user ([`Enrollers`]). Anyone else is answered
//!   `notPermitted` and nothing else.
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
//! - Messages are received into buffers overwritten afterwards; their secrets are [`Secret`]s.

use crate::api::{DeviceCredential, OxyApi, Profile};
use crate::secret::{self, Secret};
use crate::setup::{Enrolled, Pending, Progress, Request as SetupRequest, Setup, SetupError};
use crate::users::Users;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::net::{AddressFamily, RecvFlags, SendFlags, SocketAddrUnix, SocketFlags, SocketType};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;
use zeroize::Zeroizing;

/// Where the service listens.
pub const SOCKET: &str = "/run/oxyos/accountd/socket";

/// The longest message: a password of PAM's 512 bytes, escaped, and the rest, many times over.
pub const MAX_MESSAGE: usize = 16 * 1024;

/// How long a conversation may wait for the client (a person typing an authenticator's code).
pub const IDLE: Duration = Duration::from_secs(5 * 60);

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    NotPermitted,
    /// The service is setting up as many accounts as it does at once.
    Busy,
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
    setup: &'a Setup<A, U>,
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
}

impl<'a, A: OxyApi, U: Users> Conversation<'a, A, U> {
    pub fn new(setup: &'a Setup<A, U>) -> Self {
        Self {
            setup,
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
                match self.setup.enroll(request) {
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
            (Some(Request::SecondFactor { code }), true, Some(pending)) => {
                match self.setup.second_factor(pending, &code) {
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
    /// mode lets any process connect: the peer's credentials decide what it may do.
    pub fn bind(path: &Path) -> io::Result<Self> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_socket() => std::fs::remove_file(path)?,
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a socket", path.display()),
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let fd = rustix::net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        rustix::net::bind(&fd, &SocketAddrUnix::new(path)?)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
        rustix::net::listen(&fd, 16)?;
        Ok(Self { fd })
    }

    pub fn accept(&self) -> io::Result<Connection> {
        let fd = rustix::net::accept_with(&self.fd, SocketFlags::CLOEXEC)?;
        Ok(Connection { fd })
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

/// Runs one client's conversation to its end, if `enrollers` allows the peer. Each outcome is
/// handed to `log`.
pub fn serve<A: OxyApi, U: Users>(
    connection: &Connection,
    setup: &Setup<A, U>,
    enrollers: Enrollers,
    mut log: impl FnMut(&Peer, &Outcome),
) -> io::Result<()> {
    let peer = connection.peer()?;
    connection.set_timeout(IDLE)?;
    if !enrollers.allow(peer.uid) {
        return connection.send(&Answer::refused(Reason::NotPermitted));
    }
    let mut conversation = Conversation::new(setup);
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
