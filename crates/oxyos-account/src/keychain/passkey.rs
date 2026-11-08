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
