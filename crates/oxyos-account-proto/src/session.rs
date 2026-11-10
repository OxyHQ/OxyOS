//! The service's socket for sessions and their applications (`/run/oxyos/accountd/session`): its
//! messages, and the clients' side of asking it (`oxyos_account::session`).

use crate::erase::{ErasePreview, Erasing};
use crate::ipc::Connection;
use crate::keychain::passkey::{PasskeyAssertion, PasskeyCreate, PasskeyCreated, PasskeyGet};
use crate::keychain::{
    AuditView, CodeView, EnrollmentView, Field, ItemInput, ItemKind, ItemSummary, ItemView,
    JoinMethod, MemberView, SaveConfirmation, Status, Suggestion,
};
use crate::platform::{self, Checked, Local, Snapshot, Token, TokenReason};
use crate::Secret;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;
use std::time::Duration;

/// Where the service listens for sessions and their applications.
pub const SOCKET: &str = "/run/oxyos/accountd/session";

/// How long an application's `savePassword` waits for the person's answer in the session's sheet:
/// past it, nothing is saved and the sheet is withdrawn.
pub const SAVE_WAIT: Duration = Duration::from_secs(120);

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Request {
    /// The registrar presenting itself as its connection opens: answered `"accepted"` if the
    /// kernel's word on it says it is one, refused `notPermitted` (and closed) if not.
    Present {},
    #[serde(rename_all = "camelCase")]
    Register {
        desktop_entry_id: String,
        #[serde(default)]
        system: bool,
    },
    Check {
        password: Secret,
        local: Local,
    },
    Snapshot,
    Subscribe,
    Token {
        reason: TokenReason,
    },
    Remove {},
    // The keychain (ADR 0050 step 2, `crate::keychain`): the registrar's.
    #[serde(rename_all = "camelCase")]
    AutofillSuggest {
        desktop_entry_id: String,
        field: Field,
    },
    #[serde(rename_all = "camelCase")]
    AutofillFill {
        desktop_entry_id: String,
        #[serde(rename = "ref")]
        reference: String,
        field: Field,
    },
    AutofillGenerate {},
    KeychainLock {},
    // Settings'.
    KeychainStatus {},
    KeychainCreate {},
    #[serde(rename_all = "camelCase")]
    KeychainConfirmRecoveryKey {
        recovery_key: Secret,
    },
    #[serde(rename_all = "camelCase")]
    KeychainJoin {
        method: JoinMethod,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recovery_key: Option<Secret>,
    },
    KeychainEnrollments {},
    #[serde(rename_all = "camelCase")]
    KeychainApprove {
        enrollment_id: String,
        #[serde(default)]
        confirm: bool,
    },
    #[serde(rename_all = "camelCase")]
    KeychainDeny {
        enrollment_id: String,
    },
    KeychainMembers {},
    #[serde(rename_all = "camelCase")]
    KeychainRemoveMember {
        member_id: String,
    },
    KeychainVerify {
        password: Secret,
    },
    KeychainList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<Secret>,
        #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
        kind: Option<ItemKind>,
    },
    #[serde(rename_all = "camelCase")]
    KeychainReveal {
        item_id: String,
    },
    KeychainPut {
        item: ItemInput,
    },
    #[serde(rename_all = "camelCase")]
    KeychainDelete {
        item_id: String,
    },
    KeychainAudit {},
    KeychainRekey {
        previous: Secret,
        current: Secret,
    },
    KeychainLeave {},
    /// A strong password (ADR 0050 decision 8's generator), for a new or edited item.
    KeychainGenerate {},
    /// An item's verification code now.
    #[serde(rename_all = "camelCase")]
    KeychainCode {
        item_id: String,
    },
    // Saving a credential (ADR 0050 decision 5, `crate::keychain`): an application asks, the
    // registrar's connection that takes confirmations (`confirmSaves`) asks the person and answers
    // each (`saveDecision`).
    SavePassword {
        username: Secret,
        password: Secret,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        domain: Option<String>,
    },
    ConfirmSaves {},
    SaveDecision {
        id: String,
        save: bool,
    },
    // Passkeys (ADR 0050 step 5, `crate::keychain::passkey`): any registered application's.
    PasskeyCreate(PasskeyCreate),
    PasskeyGet(PasskeyGet),
    // The registrar's prompts connection.
    PasskeyPrompts {},
    PasskeyConfirm {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        choice: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<Secret>,
    },
    PasskeyCancel {
        id: String,
    },
    ErasePreview {},
    EraseAll {
        password: Secret,
    },
    // The local sign-in (`crate::pin`): Settings' and the shell's to read, Settings' to change.
    LocalSignIn,
    #[serde(rename_all = "camelCase")]
    SetPin {
        pin: Secret,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<Secret>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_pin: Option<Secret>,
    },
    RemovePin {
        password: Secret,
    },
    #[serde(rename_all = "camelCase")]
    SetPasswordSignIn {
        enabled: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<Secret>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_pin: Option<Secret>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    NotPermitted,
    InvalidRequest,
    TooMany,
    Failed,
    // The keychain's.
    /// The keychain is locked (the session is, or was never unlocked since the service started).
    Locked,
    /// Oxy cannot be reached.
    Offline,
    /// Oxy refused, or answered what does not verify: nothing of it was kept.
    Oxy,
    /// The account has no vault, or the machine is no member of it.
    NotMember,
    /// The account already has a vault (join it), or this machine is a member already.
    VaultExists,
    /// Another device removed this machine, or the vault was reset.
    Removed,
    /// The keybag is sealed under a password the machine no longer has (`keychainRekey`).
    NeedsPreviousPassword,
    /// Settings must verify the person first (`keychainVerify`).
    VerificationRequired,
    WrongPassword,
    /// Too many wrong passwords: try again later (an erase says when, `retryAfter`).
    Throttled,
    /// Not the recovery key shown, or not the vault's.
    RecoveryKeyMismatch,
    /// No such item, enrollment, member or suggestion (or its `ref` died with a lock).
    NotFound,
    /// The enrollment was denied.
    Denied,
    /// The enrollment's ten minutes are over.
    Expired,
    /// Another device wrote first: try again.
    Conflict,
    /// An erase asked by an account that is not one of the machine's administrators.
    NotAdministrator,
    /// An erase of a machine without a recovery; a save with no session to ask the person in.
    Unavailable,
    // The local sign-in's (`crate::pin`).
    WrongPin,
    /// Too many wrong PINs: the password, or Oxy online, first.
    PinLocked,
    /// Not 4 to 16 digits, or the password itself.
    UnusablePin,
    /// The password does not sign in on this computer.
    PasswordSignInOff,
    /// The password cannot be turned off without a PIN.
    NoPin,
    /// The account has an authenticator: set it up again at the login screen.
    SecondFactorRequired,
    // Passkeys', as WebAuthn's errors (`crate::keychain::passkey`).
    /// `NotAllowedError`: cancelled, not verified, timed out, no passkey, or a relying party the
    /// application may not use, all alike.
    NotAllowed,
    /// `InvalidStateError`: an excluded credential is in the keychain.
    InvalidState,
    /// `NotSupportedError`: no algorithm OxyOS makes (ES256).
    NotSupported,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
    Accepted,
    Registered,
    Refused {
        reason: Reason,
        /// Seconds until another attempt may be made (`throttled`).
        #[serde(
            default,
            rename = "retryAfter",
            skip_serializing_if = "Option::is_none"
        )]
        retry_after: Option<u64>,
    },
    Checked(Checked),
    Snapshot(Snapshot),
    Token(Option<Token>),
    Removed,
    // The keychain's (`crate::keychain`).
    AutofillSuggestions(Vec<Suggestion>),
    AutofillCredential {
        username: Secret,
        password: Secret,
    },
    AutofillCode {
        code: Secret,
    },
    GeneratedPassword(Secret),
    KeychainLocked,
    KeychainStatus(Status),
    #[serde(rename_all = "camelCase")]
    KeychainRecoveryKey {
        recovery_key: Secret,
    },
    KeychainEnrollment(EnrollmentView),
    KeychainEnrollments(Vec<EnrollmentView>),
    KeychainApproved,
    KeychainDenied,
    KeychainMembers(Vec<MemberView>),
    KeychainMemberRemoved,
    KeychainVerified,
    KeychainItems(Vec<ItemSummary>),
    KeychainItem(ItemView),
    #[serde(rename_all = "camelCase")]
    KeychainSaved {
        item_id: String,
    },
    KeychainDeleted,
    KeychainAudit(AuditView),
    KeychainLeft,
    KeychainCode(CodeView),
    // Saving a credential (`crate::keychain`): the application's answers, and what the
    // confirmations' connection is told.
    Saved {},
    NotSaved {},
    ConfirmingSaves,
    ConfirmSave(SaveConfirmation),
    SaveWithdrawn {
        id: String,
    },
    PasskeyCreated(PasskeyCreated),
    PasskeyAssertion(PasskeyAssertion),
    PasskeyPromptsOpen,
    ErasePreview(ErasePreview),
    Erasing(Erasing),
    LocalSignIn(platform::LocalView),
}

impl Answer {
    pub fn refused(reason: Reason) -> Self {
        Answer::Refused {
            reason,
            retry_after: None,
        }
    }
}

/// One request to the session socket and its one answer, within `wait`.
pub fn ask(socket: &Path, request: &Request, wait: Duration) -> io::Result<Answer> {
    let connection = Connection::connect(socket)?;
    connection.set_timeout(wait)?;
    connection.send(request)?;
    match connection.receive::<Answer>()? {
        Some(Some(answer)) => Ok(answer),
        Some(None) => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
        None => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the service closed the connection",
        )),
    }
}

/// The lock's online check of the session's own account, after PAM said `local`. Fails if the
/// service does not answer or refuses: the lock then keeps PAM's answer.
pub fn check(socket: &Path, password: Secret, local: Local) -> io::Result<Checked> {
    match ask(
        socket,
        &Request::Check { password, local },
        crate::ipc::CHECK_WAIT,
    )? {
        Answer::Checked(checked) => Ok(checked),
        Answer::Refused { reason, .. } => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refused: {reason:?}"),
        )),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
    }
}
