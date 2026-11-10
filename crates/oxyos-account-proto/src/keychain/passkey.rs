//! Passkeys' messages (ADR 0050 step 5, `oxyos_account::keychain::passkey`).

use crate::Secret;
use serde::{Deserialize, Serialize};

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
