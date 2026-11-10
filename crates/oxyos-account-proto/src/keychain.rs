//! The keychain's messages on the session's socket (ADR 0050, `oxyos_account::keychain`).

use crate::session::Request;
use crate::Secret;
use serde::{Deserialize, Serialize};

pub mod passkey;

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

/// What the session's sheet asks the person about an application's request to save
/// (`confirmSave`): never the password.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveConfirmation {
    pub id: String,
    pub desktop_entry_id: String,
    pub username: Secret,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

/// Whether a session request is a passkey ceremony (any registered application's).
pub fn is_passkey_request(request: &Request) -> bool {
    matches!(request, Request::PasskeyCreate(_) | Request::PasskeyGet(_))
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
