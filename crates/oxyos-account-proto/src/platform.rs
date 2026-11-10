//! The platform's messages: a password check, the session's snapshot of its account, an
//! application's token, the local sign-in (`oxyos_account::platform`).

use crate::Secret;
use serde::{Deserialize, Serialize};

/// What PAM said of a password, given to [`Platform::check`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Local {
    Accepted,
    Rejected,
}

/// What a check came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Accepted,
    Rejected,
}

/// Why a check was rejected, when the person should be told more than "wrong password".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckReason {
    /// Oxy refused a password the machine knows: it was changed elsewhere.
    PasswordChanged,
    /// Oxy signed the account out: it must be set up again online.
    SignInAgain,
}

/// A check's answer: `{"result", "reason"?}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    pub result: Verdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CheckReason>,
}

impl Checked {
    pub const ACCEPTED: Checked = Checked {
        result: Verdict::Accepted,
        reason: None,
    };
    pub const REJECTED: Checked = Checked {
        result: Verdict::Rejected,
        reason: None,
    };

    /// Rejected, saying why.
    pub fn rejected(reason: CheckReason) -> Self {
        Checked {
            result: Verdict::Rejected,
            reason: Some(reason),
        }
    }
}

/// The SDK's `OxyHostSnapshot` (Oxy ADR 0058, decision 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub status: Status,
    pub revision: u64,
    pub account: Option<SnapshotAccount>,
    pub online: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    SignedIn,
    SignedOut,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotAccount {
    pub id: String,
    pub handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// The account's picture on this machine (its avatar's image, as the accounts' service last
    /// read it: a root-owned file the session reads, [`crate::store`]); none without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// The account's languages, primary first, as the platform last read them online.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub languages: Option<Vec<String>>,
}

/// An application's token, as it is handed to the application.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Token {
    pub access_token: Secret,
    pub expires_at: String,
}

/// Why a token is asked for (the SDK's `getAccessToken` reason).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenReason {
    #[serde(rename = "boot")]
    Boot,
    #[serde(rename = "preflight")]
    Preflight,
    /// The token the application had was refused: a fresh one is minted.
    #[serde(rename = "response-401")]
    Response401,
}

/// An account's local sign-in, as Settings and the lock screen are told it (never a verifier).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalView {
    pub pin: crate::pin::PinState,
    /// Whether the password signs in on this computer.
    pub password: bool,
}
