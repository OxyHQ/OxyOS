//! Passkeys on the session socket ([ADR 0050](../../../docs/adr/0050-keychain-autofill-and-passkeys.mdx),
//! decision 7, step 5): OxyOS as a platform authenticator for the session's applications, and the
//! session's sheet that verifies the person.
//!
//! **An application** (a registered process, known by its launch record, never by what it says)
//! asks WebAuthn's two ceremonies with CTAP2's semantics; binary is base64url without padding:
//!
//! ```text
//! → {"passkeyCreate": {"rpId", "rpName", "user": {"id", "name", "displayName"},
//!                      "clientDataHash", "excludeCredentials": [id…], "algorithms": [-7…]}}
//! ← {"passkeyCreated": {"credentialId", "attestationObject", "authenticatorData", "publicKey",
//!                       "publicKeyAlgorithm": -7, "transports": ["internal"]}}
//! → {"passkeyGet": {"rpId", "clientDataHash", "allowCredentials": [id…]}}
//! ← {"passkeyAssertion": {"credentialId", "authenticatorData", "signature", "userHandle"}}
//! ← {"refused": {"reason"}}
//! ```
//!
//! The reasons are WebAuthn's errors: `notAllowed` (`NotAllowedError`: the person cancelled or
//! did not verify, the time ran out, no passkey of that relying party, a relying party the
//! application may not use, a keychain that is off or waits for its previous password, another
//! ceremony of the same application under way, or no session to show the sheet, all alike),
//! `invalidState` (`InvalidStateError`: an excluded credential is in the keychain, said only once
//! the person saw it), `notSupported` (no ES256 in `algorithms`), `invalidRequest` (`TypeError`),
//! `locked` (asked first, of every application alike, whatever the relying party), `offline` (a
//! new passkey cannot be saved without Oxy, said after the person verified), `failed`.
//!
//! **The session** shows the sheet. A registrar connection of its own, once it sent
//! `{"passkeyPrompts": {}}` (answered `"passkeyPromptsOpen"` once its sheets are shown there; a
//! ceremony asked while no session shows them is `notAllowed` at once), is told unasked:
//!
//! ```text
//! ← {"confirmPasskey": {"id", "desktopEntryId", "pid", "rpId", "userName"?, "kind", "choices"}}
//! ← {"passkeyRetry": {"id", "reason": "wrongPassword" | "throttled", "retryAfter"?}}
//! ← {"passkeyDone": {"id"}}                         the sheet goes
//! → {"passkeyConfirm": {"id", "choice"?, "password"?}}
//! → {"passkeyCancel": {"id"}}
//! ```
//!
//! `kind` is `create` ("Create a passkey for …"), `get` ("Use your passkey for …", a chooser
//! when `choices` holds several), `excluded` (the person already has a passkey there) or `none`
//! (none for that relying party): the last two only inform, and are confirmed without a
//! password. A `create` or a `get` is confirmed with the account's password, which the service
//! checks itself (the keybag opened with it, throttled as Settings' verification): a wrong one is
//! told `passkeyRetry` and the sheet stays. Nothing is told to the application before the sheet
//! is done.

use crate::ipc::Connection;
use crate::secret::Secret;
use crate::session::{Answer, Request};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// How long a ceremony waits for the person, as WebAuthn's default timeout.
pub const PASSKEY_WAIT: std::time::Duration = std::time::Duration::from_secs(300);
/// At most this many ids in `allowCredentials` or `excludeCredentials`.
pub const MAX_CREDENTIAL_LIST: usize = 64;

/// The relying party's user, as WebAuthn's `PublicKeyCredentialUserEntity`.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PasskeyUser {
    /// The user handle, base64url.
    pub id: String,
    pub name: Secret,
    pub display_name: Secret,
}

/// `passkeyCreate`: CTAP2's authenticatorMakeCredential.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PasskeyCreate {
    pub rp_id: String,
    pub rp_name: Secret,
    pub user: PasskeyUser,
    pub client_data_hash: String,
    #[serde(default)]
    pub exclude_credentials: Vec<String>,
    pub algorithms: Vec<i64>,
}

/// `passkeyGet`: CTAP2's authenticatorGetAssertion.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PasskeyGet {
    pub rp_id: String,
    pub client_data_hash: String,
    #[serde(default)]
    pub allow_credentials: Vec<String>,
}

/// `passkeyCreated`: WebAuthn's `AuthenticatorAttestationResponse`, base64url.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyCreated {
    pub credential_id: String,
    pub attestation_object: String,
    pub authenticator_data: String,
    /// SubjectPublicKeyInfo, DER (`getPublicKey()`).
    pub public_key: String,
    pub public_key_algorithm: i64,
    pub transports: Vec<String>,
}

/// `passkeyAssertion`: WebAuthn's `AuthenticatorAssertionResponse`, base64url.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyAssertion {
    pub credential_id: String,
    pub authenticator_data: String,
    pub signature: String,
    pub user_handle: String,
}

/// What the sheet asks of the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptKind {
    /// Create a passkey (the password).
    Create,
    /// Use a passkey (the password, and which one when several).
    Get,
    /// The person already has a passkey for this relying party: told, then `invalidState`.
    Excluded,
    /// No passkey for this relying party: told, then `notAllowed`.
    None,
}

impl PromptKind {
    /// Whether the person confirms it with the account's password.
    pub fn verifies(self) -> bool {
        matches!(self, PromptKind::Create | PromptKind::Get)
    }
}

/// A passkey the person may choose.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyChoice {
    pub user_name: Secret,
    pub display_name: Secret,
}

/// The sheet to show, over the application's window.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmPasskey {
    pub id: String,
    pub desktop_entry_id: String,
    /// The application's process, whose window the sheet is over.
    pub pid: i32,
    pub rp_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<Secret>,
    pub kind: PromptKind,
    #[serde(default)]
    pub choices: Vec<PasskeyChoice>,
}

/// Why the sheet asks again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RetryReason {
    WrongPassword,
    Throttled,
}

/// What the service tells the session's prompts connection, unasked.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Prompt {
    ConfirmPasskey(ConfirmPasskey),
    #[serde(rename_all = "camelCase")]
    PasskeyRetry {
        id: String,
        reason: RetryReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after: Option<u64>,
    },
    PasskeyDone {
        id: String,
    },
}

/// The session's prompts connection: a registrar connection of its own, told each sheet to show.
pub struct PromptsClient {
    connection: Connection,
}

impl PromptsClient {
    /// Opens the session's prompts connection (the registrar's).
    pub fn connect(socket: &Path) -> io::Result<Self> {
        let connection = Connection::connect(socket)?;
        connection.set_timeout(super::AUTOFILL_WAIT)?;
        connection.send(&Request::PasskeyPrompts {})?;
        match connection.receive::<Answer>()? {
            Some(Some(Answer::PasskeyPromptsOpen)) => {}
            Some(Some(Answer::Refused { reason, .. })) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("the service refused the prompts: {reason:?}"),
                ))
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "not the answer asked for",
                ))
            }
        }
        connection.set_receive_timeout(None)?;
        Ok(Self { connection })
    }

    /// The next prompt, waiting for it; `None` once the service closed the connection. A message
    /// that is not a prompt is skipped.
    pub fn next(&self) -> io::Result<Option<Prompt>> {
        loop {
            match self.connection.receive::<Prompt>()? {
                None => return Ok(None),
                Some(Some(prompt)) => return Ok(Some(prompt)),
                Some(None) => continue,
            }
        }
    }

    /// The person confirmed `id` (with the passkey chosen, and the password when it asks one).
    pub fn confirm(
        &self,
        id: &str,
        choice: Option<usize>,
        password: Option<Secret>,
    ) -> io::Result<()> {
        self.connection.send(&Request::PasskeyConfirm {
            id: id.to_owned(),
            choice,
            password,
        })
    }

    /// The person cancelled `id`.
    pub fn cancel(&self, id: &str) -> io::Result<()> {
        self.connection
            .send(&Request::PasskeyCancel { id: id.to_owned() })
    }

    /// Another handle on the same connection, for sending from another thread than the one that
    /// waits in [`PromptsClient::next`].
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            connection: self.connection.try_clone()?,
        })
    }
}

impl rustix::fd::AsFd for PromptsClient {
    fn as_fd(&self) -> rustix::fd::BorrowedFd<'_> {
        self.connection.as_fd()
    }
}

/// What the session's sheets' connection was told, or that it ended.
pub type SheetEvent = Prompt;

/// The waits before connecting again to a service that went or could not be reached; the last
/// repeats.
const RECONNECT: [std::time::Duration; 5] = [
    std::time::Duration::from_secs(1),
    std::time::Duration::from_secs(2),
    std::time::Duration::from_secs(5),
    std::time::Duration::from_secs(10),
    std::time::Duration::from_secs(30),
];

/// The person's answer, on its way to the service.
enum Answered {
    Confirm {
        id: String,
        choice: Option<usize>,
        password: Option<Secret>,
    },
    Cancel {
        id: String,
    },
}

/// The session's prompts connection on a thread of its own, so the compositor's turn never waits
/// for the service: what it is told arrives in [`PasskeySheets::take`] (`wake_fd` readable
/// meanwhile), and the person's answers go back with [`PasskeySheets::confirm`] and
/// [`PasskeySheets::cancel`]. An answer is sent only for a sheet asked on the connection open now;
/// when the service goes, every sheet it asked is told done (it goes) and the thread connects
/// again. A service that refuses the session (no registrar: a session the login program did not
/// start) is said once and not asked again: no application's passkey is ever verified there.
pub struct PasskeySheets {
    events: std::sync::mpsc::Receiver<SheetEvent>,
    answers: std::sync::mpsc::Sender<Answered>,
    /// Readable while events wait (the compositor's waker).
    woken: std::os::unix::net::UnixStream,
    /// Wakes the thread for an answer; closed, it ends.
    poke: std::os::unix::net::UnixStream,
}

impl PasskeySheets {
    /// The sheets of the service at `socket`; `log` says what it could not do (never a name).
    pub fn spawn(
        socket: std::path::PathBuf,
        log: impl Fn(&str) + Send + 'static,
    ) -> io::Result<Self> {
        let (events_in, events) = std::sync::mpsc::channel();
        let (answers, answered) = std::sync::mpsc::channel();
        let (woken, wake) = std::os::unix::net::UnixStream::pair()?;
        woken.set_nonblocking(true)?;
        let (poke, poked) = std::os::unix::net::UnixStream::pair()?;
        poked.set_nonblocking(true)?;
        std::thread::Builder::new()
            .name("oxyos-passkey-sheets".into())
            .spawn(move || sheets_thread(&socket, &events_in, &wake, &answered, &poked, &log))?;
        Ok(Self {
            events,
            answers,
            woken,
            poke,
        })
    }

    /// What wakes the compositor's turn when something arrived.
    pub fn wake_fd(&self) -> io::Result<rustix::fd::OwnedFd> {
        Ok(self.woken.try_clone()?.into())
    }

    /// What arrived since the last call, in order.
    pub fn take(&self) -> Vec<SheetEvent> {
        use std::io::Read;
        let mut buffer = [0u8; 64];
        while matches!((&self.woken).read(&mut buffer), Ok(read) if read > 0) {}
        self.events.try_iter().collect()
    }

    /// The person confirmed `id` (the passkey chosen, and the password when it asks one).
    pub fn confirm(&self, id: &str, choice: Option<usize>, password: Option<Secret>) {
        self.send(Answered::Confirm {
            id: id.to_owned(),
            choice,
            password,
        });
    }

    /// The person cancelled `id`.
    pub fn cancel(&self, id: &str) {
        self.send(Answered::Cancel { id: id.to_owned() });
    }

    fn send(&self, answer: Answered) {
        if self.answers.send(answer).is_ok() {
            use std::io::Write;
            let _ = (&self.poke).write(&[1]);
        }
    }
}

fn sheets_thread(
    socket: &Path,
    events: &std::sync::mpsc::Sender<SheetEvent>,
    wake: &std::os::unix::net::UnixStream,
    answered: &std::sync::mpsc::Receiver<Answered>,
    poked: &std::os::unix::net::UnixStream,
    log: &dyn Fn(&str),
) {
    use rustix::event::{poll, PollFd, PollFlags};
    use std::io::{Read, Write};
    let tell = |event: SheetEvent| {
        let _ = events.send(event);
        let _ = (&*wake).write(&[1]);
    };
    let mut attempts = 0usize;
    let mut said = false;
    loop {
        let client = match PromptsClient::connect(socket) {
            Ok(client) => {
                said = false;
                attempts = 0;
                client
            }
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                log(&format!(
                    "the accounts' service shows this session no passkey sheets: {error}"
                ));
                return;
            }
            Err(error) => {
                if !std::mem::replace(&mut said, true) {
                    log(&format!(
                        "the accounts' service at {}: {error}; no passkey is verified until it answers",
                        socket.display()
                    ));
                }
                if !wait(poked, RECONNECT[attempts.min(RECONNECT.len() - 1)]) {
                    return;
                }
                attempts += 1;
                continue;
            }
        };
        // The sheets asked on this connection and not yet done.
        let mut open: std::collections::HashSet<String> = std::collections::HashSet::new();
        // Answers to sheets of an earlier connection are dropped.
        while answered.try_recv().is_ok() {}
        let ended: io::Error = loop {
            let mut fds = [
                PollFd::new(&client, PollFlags::IN),
                PollFd::new(poked, PollFlags::IN),
            ];
            match poll(&mut fds, None) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => break io::Error::from(error),
            }
            let (message, poke) = (!fds[0].revents().is_empty(), !fds[1].revents().is_empty());
            if poke {
                let mut bytes = [0u8; 64];
                // The session let go: it is ending.
                if let Ok(0) = (&*poked).read(&mut bytes) {
                    return;
                }
                while matches!((&*poked).read(&mut bytes), Ok(read) if read > 0) {}
                let mut sent = Ok(());
                for answer in answered.try_iter() {
                    sent = sent.and(match answer {
                        Answered::Confirm {
                            id,
                            choice,
                            password,
                        } if open.contains(&id) => client.confirm(&id, choice, password),
                        Answered::Cancel { id } if open.contains(&id) => client.cancel(&id),
                        _ => Ok(()),
                    });
                }
                if let Err(error) = sent {
                    break error;
                }
            }
            if !message {
                continue;
            }
            match client.next() {
                Ok(Some(prompt)) => {
                    match &prompt {
                        Prompt::ConfirmPasskey(confirm) => {
                            open.insert(confirm.id.clone());
                        }
                        Prompt::PasskeyDone { id } => {
                            if !open.remove(id) {
                                continue;
                            }
                        }
                        Prompt::PasskeyRetry { id, .. } => {
                            if !open.contains(id) {
                                continue;
                            }
                        }
                    }
                    tell(prompt);
                }
                Ok(None) => {
                    break io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the service closed the connection",
                    )
                }
                Err(error) => break error,
            }
        };
        // The service went, and with it what it asked: those sheets go.
        for id in open.drain() {
            tell(Prompt::PasskeyDone { id });
        }
        log(&format!("the passkey sheets' connection ended: {ended}"));
        if !wait(poked, RECONNECT[0]) {
            return;
        }
    }
}

/// Waits `wait`, or until the session lets go (false). Answers poked meanwhile are for sheets of
/// a connection that went: dropped when the next one opens.
fn wait(poked: &std::os::unix::net::UnixStream, wait: std::time::Duration) -> bool {
    use rustix::event::{poll, PollFd, PollFlags};
    use std::io::Read;
    let until = std::time::Instant::now() + wait;
    loop {
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return true;
        }
        let timeout = rustix::time::Timespec {
            tv_sec: left.as_secs() as i64,
            tv_nsec: i64::from(left.subsec_nanos()),
        };
        let mut fds = [PollFd::new(poked, PollFlags::IN)];
        match poll(&mut fds, Some(&timeout)) {
            Ok(0) => return true,
            Ok(_) => {
                let mut bytes = [0u8; 64];
                match (&*poked).read(&mut bytes) {
                    Ok(0) => return false,
                    _ => continue,
                }
            }
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => return true,
        }
    }
}
