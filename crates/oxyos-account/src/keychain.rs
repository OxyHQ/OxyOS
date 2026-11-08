//! The keychain on the service's sockets ([ADR 0050](../../docs/adr/0050-keychain-autofill-and-passkeys.mdx),
//! step 2): the messages, the seam the service's keychain is reached through ([`Keychains`]), and
//! the registrar's client for AutoFill ([`RegistrarClient`]).
//!
//! Who may ask what (decision 5), always by the kernel's word on the peer:
//!
//! - **The login program** (root, on the service's own socket), once PAM accepted a password:
//!   `{"unlockKeychain": {"user", "password"}}` → `{"keychain": state}`, the keychain's
//!   [`KeychainState`] after it (`off`, `member`, … once unlocked; `needsPreviousPassword`;
//!   `locked` when it could not be).
//! - **The registrar** (the session program, on the session socket; trusted as the session), naming
//!   the focused application by its launch record's desktop entry id:
//!
//!   ```text
//!   → {"autofillSuggest": {"desktopEntryId", "field"}}
//!   ← {"autofillSuggestions": [{"ref", "title", "username"}]}
//!   → {"autofillFill": {"desktopEntryId", "ref", "field"}}
//!   ← {"autofillCredential": {"username", "password"}} | {"autofillCode": {"code"}}
//!   → {"autofillGenerate": {}}            ← {"generatedPassword": "…"}
//!   → {"keychainLock": {}}                ← "keychainLocked"
//!   ```
//!
//!   `field` is `username`, `email`, `password`, `newPassword` or `oneTimeCode`. A suggestion is
//!   a login associated with the application (its `apps` name the desktop entry id), never a
//!   secret; a `oneTimeCode` field is offered the logins that have a verification code, a
//!   `newPassword` field none (it is offered `autofillGenerate`'s password). `ref` is an opaque
//!   handle made at random for this unlock, never the item's id, and dies with the lock; a fill
//!   checks the association and the unlock again. Its own `check` accepted, the registrar's
//!   account is unlocked with that password.
//! - **Settings** (an application registered as the system's `so.oxy.settings`, on the session
//!   socket): `keychainStatus`, `keychainCreate`, `keychainConfirmRecoveryKey`, `keychainJoin`,
//!   `keychainEnrollments`, `keychainApprove`, `keychainDeny`, `keychainMembers`,
//!   `keychainRemoveMember`, `keychainVerify`, `keychainList`, `keychainReveal`, `keychainPut`,
//!   `keychainDelete`, `keychainAudit`, `keychainRekey`, `keychainLeave`, `keychainGenerate` (a
//!   strong password, the generator's) and `keychainCode` (an item's verification code now) (ADR
//!   0050 says each one's shape). Listing, reading, writing, deleting, auditing, a code, a strong
//!   password and removing a member need a verification first (`keychainVerify {password}`, the
//!   keybag opened with it), good until five minutes pass without one of them, or the lock.
//! - **Anyone else** is refused `notPermitted`; while locked every keychain request is refused
//!   `locked`.

use crate::ipc::Connection;
use crate::secret::Secret;
use crate::session::{Answer, Reason, Request};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;
use std::time::Duration;

#[cfg(feature = "keychain")]
pub mod api;
#[cfg(feature = "keychain")]
pub mod service;

/// What an application's field is for (ADR 0050 decision 6, ADR 0008's content type).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Field {
    Username,
    Email,
    Password,
    NewPassword,
    OneTimeCode,
}

/// A login offered to a field: never a secret but its title and username.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: Secret,
    pub username: Secret,
}

/// What a fill gives.
#[derive(Debug, PartialEq, Eq)]
pub enum Fill {
    Credential { username: Secret, password: Secret },
    Code(Secret),
}

/// Where the keychain stands for the peer's account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeychainState {
    /// Locked with the session (or never unlocked since the service started).
    Locked,
    /// The machine learned a password that does not open the keybag: the previous one re-seals
    /// it (`keychainRekey`), or the machine leaves the vault (`keychainLeave`).
    NeedsPreviousPassword,
    /// Unlocked, and this machine is no member of a vault.
    Off,
    /// A vault being made: the recovery key is shown, waiting to be typed back.
    Creating,
    /// Joining by approval: the enrollment waits for another device.
    Joining,
    /// A member of the account's vault.
    Member,
    /// Another device removed this machine, or the vault was reset: the local copy was forgotten.
    Removed,
    /// The roster still has this machine's member, but Oxy binds this device to none (the
    /// machine was set up again): another device removes it, and this one joins again.
    Unbound,
}

/// `keychainStatus`'s answer.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: KeychainState,
    /// Whether the account has a vault at Oxy; `null` when not known (offline, locked).
    pub vault: Option<bool>,
    /// Settings proved the person (`keychainVerify`) within five minutes.
    pub verified: bool,
    /// The join's code, once both devices can show it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Secret>,
    /// Live items kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<usize>,
}

/// How a machine joins an existing vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JoinMethod {
    /// Another device approves, both showing the same six-digit code.
    Approval,
    /// The recovery key the vault was made with.
    RecoveryKey,
}

/// An enrollment as Settings shows it.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentView {
    pub enrollment_id: String,
    /// `waiting` (no device or approver yet), `code` (both can show the code), or `approved`.
    pub status: EnrollmentProgress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Secret>,
    /// The device asking (on the approving machine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnrollmentProgress {
    Waiting,
    Code,
    Approved,
}

/// A member device as Settings › Contraseñas › Dispositivos lists it.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberView {
    pub member_id: String,
    pub name: String,
    pub platform: String,
    /// This machine.
    #[serde(rename = "self")]
    pub own: bool,
    /// Its device session ended: the next member that sees it removes it.
    pub orphaned: bool,
}

/// An item's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Login,
    Passkey,
    Totp,
    Note,
    Wifi,
}

/// An item as the list shows it: no password, code, key or note.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemSummary {
    pub item_id: String,
    #[serde(rename = "type")]
    pub kind: ItemKind,
    pub title: Secret,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<Secret>,
    /// The hosts of a login's URLs, or a passkey's relying party.
    pub domains: Vec<Secret>,
    /// A login with a verification code, or a code's own item: Settings shows its code
    /// (`keychainCode`) and lists it among the codes.
    #[serde(default)]
    pub has_code: bool,
    pub modified_at: String,
}

/// `keychainCode`'s answer: an item's verification code now, its period and the seconds it has
/// left (RFC 6238, the service's own TOTP).
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeView {
    pub code: Secret,
    pub period: u64,
    pub remaining: u64,
}

/// An item's fields, by type: what `keychainPut` takes and `keychainReveal` gives. A passkey is
/// shown (its relying party and user, never its private key) and never written here.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ItemFields {
    Login {
        title: Secret,
        username: Secret,
        password: Secret,
        #[serde(default)]
        urls: Vec<Secret>,
        /// Desktop entry ids of the applications it is offered to.
        #[serde(default)]
        apps: Vec<Secret>,
        /// An `otpauth://totp/` URI.
        #[serde(default)]
        totp: Option<Secret>,
        #[serde(default)]
        notes: Option<Secret>,
    },
    Passkey {
        rp_id: Secret,
        rp_name: Secret,
        user_name: Secret,
        user_display_name: Secret,
    },
    Totp {
        title: Secret,
        #[serde(default)]
        issuer: Option<Secret>,
        #[serde(default)]
        account: Option<Secret>,
        uri: Secret,
        #[serde(default)]
        notes: Option<Secret>,
    },
    Note {
        title: Secret,
        body: Secret,
    },
    Wifi {
        ssid: Secret,
        #[serde(default)]
        password: Option<Secret>,
        /// `wpa2-psk`, `wpa3-sae`, `wep` or `open`.
        security: String,
        #[serde(default)]
        hidden: bool,
    },
}

/// `keychainPut`'s item: a new one without `itemId`, else the item it replaces.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(flatten)]
    pub fields: ItemFields,
}

/// `keychainReveal`'s item.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemView {
    pub item_id: String,
    pub created_at: String,
    pub modified_at: String,
    #[serde(flatten)]
    pub fields: ItemFields,
}

/// `keychainAudit`'s answer: passwords used by several logins (each group's item ids), and weak
/// ones.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditView {
    pub reused_count: usize,
    pub weak_count: usize,
    pub reused: Vec<Vec<String>>,
    pub weak: Vec<String>,
}

/// The keychain as the sockets reach it: the service's (`service::KeychainService`), set on the
/// platform when the service runs one.
pub trait Keychains: Send + Sync {
    /// The login program's unlock of `user` (uid `uid`), PAM having accepted `password`.
    fn unlock(&self, uid: u32, user: &str, password: &Secret) -> KeychainState;
    /// A registrar connection of `uid`'s session (its user `user`) opened.
    fn session_opened(&self, uid: u32, user: &str);
    /// A registrar connection of `uid`'s session closed; `remaining` are still open. With none,
    /// the session is gone and the keychain locks.
    fn session_closed(&self, uid: u32, remaining: usize);
    /// Locks `uid`'s keychain now.
    fn lock(&self, uid: u32);
    /// A request of `uid`'s registrar (the session).
    fn registrar(&self, uid: u32, user: &str, request: Request) -> Answer;
    /// A request of `uid`'s Settings.
    fn settings(&self, uid: u32, user: &str, request: Request) -> Answer;
    /// `user`'s account was removed from the machine: its keychain is locked and erased.
    fn forget(&self, uid: u32, user: &str);
}

/// Whether a session request is the keychain's.
pub fn is_keychain_request(request: &Request) -> bool {
    matches!(
        request,
        Request::AutofillSuggest { .. }
            | Request::AutofillFill { .. }
            | Request::AutofillGenerate {}
            | Request::KeychainLock {}
            | Request::KeychainStatus {}
            | Request::KeychainCreate {}
            | Request::KeychainConfirmRecoveryKey { .. }
            | Request::KeychainJoin { .. }
            | Request::KeychainEnrollments {}
            | Request::KeychainApprove { .. }
            | Request::KeychainDeny { .. }
            | Request::KeychainMembers {}
            | Request::KeychainRemoveMember { .. }
            | Request::KeychainVerify { .. }
            | Request::KeychainList { .. }
            | Request::KeychainReveal { .. }
            | Request::KeychainPut { .. }
            | Request::KeychainDelete { .. }
            | Request::KeychainAudit {}
            | Request::KeychainRekey { .. }
            | Request::KeychainLeave {}
            | Request::KeychainGenerate {}
            | Request::KeychainCode { .. }
    )
}

/// Whether a session request is the registrar's alone (AutoFill, the lock).
pub fn is_registrar_request(request: &Request) -> bool {
    matches!(
        request,
        Request::AutofillSuggest { .. }
            | Request::AutofillFill { .. }
            | Request::AutofillGenerate {}
            | Request::KeychainLock {}
    )
}

/// How long the registrar's client waits for an answer.
pub const AUTOFILL_WAIT: Duration = Duration::from_secs(5);

/// The session's own connection to the keychain, for AutoFill and the lock (the compositor's): a
/// registrar connection of its own, apart from the one that registers launches. Each call is one
/// request and its answer; `Err` is the connection, `Ok(Err(reason))` the service's refusal
/// (`locked` while the session is).
pub struct RegistrarClient {
    connection: Connection,
}

impl RegistrarClient {
    pub fn connect(socket: &Path) -> io::Result<Self> {
        let connection = Connection::connect(socket)?;
        connection.set_timeout(AUTOFILL_WAIT)?;
        Ok(Self { connection })
    }

    fn ask(&self, request: &Request) -> io::Result<Result<Answer, Reason>> {
        self.connection.send(request)?;
        match self.connection.receive::<Answer>()? {
            Some(Some(Answer::Refused { reason, .. })) => Ok(Err(reason)),
            Some(Some(answer)) => Ok(Ok(answer)),
            Some(None) => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the service closed the connection",
            )),
        }
    }

    fn unexpected() -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, "not the answer asked for")
    }

    /// The logins `desktop_entry_id` may be offered in a `field`.
    pub fn autofill_suggest(
        &self,
        desktop_entry_id: &str,
        field: Field,
    ) -> io::Result<Result<Vec<Suggestion>, Reason>> {
        let request = Request::AutofillSuggest {
            desktop_entry_id: desktop_entry_id.to_owned(),
            field,
        };
        match self.ask(&request)? {
            Ok(Answer::AutofillSuggestions(suggestions)) => Ok(Ok(suggestions)),
            Ok(_) => Err(Self::unexpected()),
            Err(reason) => Ok(Err(reason)),
        }
    }

    /// The suggestion `reference` for `desktop_entry_id`'s `field`: its username and password, or
    /// its verification code now.
    pub fn autofill_fill(
        &self,
        desktop_entry_id: &str,
        reference: &str,
        field: Field,
    ) -> io::Result<Result<Fill, Reason>> {
        let request = Request::AutofillFill {
            desktop_entry_id: desktop_entry_id.to_owned(),
            reference: reference.to_owned(),
            field,
        };
        match self.ask(&request)? {
            Ok(Answer::AutofillCredential { username, password }) => {
                Ok(Ok(Fill::Credential { username, password }))
            }
            Ok(Answer::AutofillCode { code }) => Ok(Ok(Fill::Code(code))),
            Ok(_) => Err(Self::unexpected()),
            Err(reason) => Ok(Err(reason)),
        }
    }

    /// A new strong password (ADR 0050 decision 8).
    pub fn autofill_generate(&self) -> io::Result<Result<Secret, Reason>> {
        match self.ask(&Request::AutofillGenerate {})? {
            Ok(Answer::GeneratedPassword(password)) => Ok(Ok(password)),
            Ok(_) => Err(Self::unexpected()),
            Err(reason) => Ok(Err(reason)),
        }
    }

    /// Locks the keychain (the session locked).
    pub fn keychain_lock(&self) -> io::Result<Result<(), Reason>> {
        match self.ask(&Request::KeychainLock {})? {
            Ok(Answer::KeychainLocked) => Ok(Ok(())),
            Ok(_) => Err(Self::unexpected()),
            Err(reason) => Ok(Err(reason)),
        }
    }
}
