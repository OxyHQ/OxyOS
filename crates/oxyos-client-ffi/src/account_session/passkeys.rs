//! Passkeys for an application (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7):
//! WebAuthn's JSON shapes in, the accounts' service's ceremonies out, as a platform's WebAuthn API
//! takes `PublicKeyCredentialCreationOptionsJSON` and `PublicKeyCredentialRequestOptionsJSON` and
//! answers `RegistrationResponseJSON` and `AuthenticationResponseJSON` (WebAuthn Level 3, §5.1.8
//! and §5.1.9; binary as base64url without padding).
//!
//! The client data is made here, as a platform makes it for an application: `{"type", "challenge",
//! "origin", "crossOrigin": false}` in WebAuthn's own serialization (§5.8.1.1), the origin
//! `https://` and the relying party's id (an application's relying party is a domain associated
//! with it, so a relying party checks the origin it expects); the service receives only its
//! SHA-256, as an authenticator does. The relying party's id must be given (an application has no
//! origin to default it from). Every credential is the platform's (`authenticatorAttachment`
//! `platform`); one asked only of a security key (`cross-platform`) is refused until security keys
//! come. Extensions are not run: `clientExtensionResults` is empty.
//!
//! A refusal is WebAuthn's error by its DOMException name, with the service's word for it:
//! `{"error": {"name": "NotAllowedError" | "InvalidStateError" | "NotSupportedError" | "TypeError"
//! | "UnknownError", "reason"}}`. An application never learns more than WebAuthn tells a page.

use super::{failed, session_socket, SessionError};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use oxyos_account_proto::ipc::Connection;
use oxyos_account_proto::keychain::passkey::{PasskeyCreate, PasskeyGet, PasskeyUser};
use oxyos_account_proto::session::{self, Reason};
use oxyos_account_proto::Secret;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

/// How long a ceremony may take: the person's five minutes on the sheet, and a new passkey's write
/// to Oxy after it.
pub const PASSKEY_WAIT: Duration = Duration::from_secs(390);
/// The most credential ids a list may name, as the service takes them.
const MAX_CREDENTIALS: usize = 64;

/// A ceremony's outcome: the credential, as WebAuthn's JSON, or WebAuthn's error.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Credential(Value),
    Error { name: &'static str, reason: String },
}

impl Outcome {
    pub fn to_json(&self) -> Value {
        match self {
            Outcome::Credential(credential) => json!({ "credential": credential }),
            Outcome::Error { name, reason } => json!({"error": {"name": name, "reason": reason}}),
        }
    }
}

fn type_error(reason: &str) -> Outcome {
    Outcome::Error {
        name: "TypeError",
        reason: reason.into(),
    }
}

/// WebAuthn's error for a refusal of the service's.
fn refused(reason: &str) -> Outcome {
    let name = match reason {
        "invalidState" => "InvalidStateError",
        "notSupported" => "NotSupportedError",
        "invalidRequest" => "TypeError",
        "offline" | "failed" => "UnknownError",
        // `notAllowed`, `locked`, `notPermitted` (not an application the session launched), and
        // anything else: what WebAuthn says of a ceremony that did not happen.
        _ => "NotAllowedError",
    };
    Outcome::Error {
        name,
        reason: reason.into(),
    }
}

/// Whether `text` is base64url without padding (what WebAuthn's JSON carries).
fn base64url(text: &str) -> bool {
    !text.is_empty()
        && text.len() % 4 != 1
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A string field of `value`.
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// The ids of a list of `PublicKeyCredentialDescriptorJSON`s (those of type `public-key`).
fn descriptors(value: &Value, key: &str) -> Result<Vec<String>, Outcome> {
    let Some(list) = value.get(key) else {
        return Ok(Vec::new());
    };
    let list = list
        .as_array()
        .ok_or_else(|| type_error(&format!("{key} is a list")))?;
    if list.len() > MAX_CREDENTIALS {
        return Err(type_error(&format!("{key} names too many credentials")));
    }
    let mut ids = Vec::new();
    for descriptor in list {
        if text(descriptor, "type") != Some("public-key") {
            continue;
        }
        match text(descriptor, "id") {
            Some(id) if base64url(id) => ids.push(id.to_owned()),
            _ => return Err(type_error(&format!("{key}: an id is base64url"))),
        }
    }
    Ok(ids)
}

/// The client data's JSON in WebAuthn's serialization, for `kind` (`webauthn.create` or
/// `webauthn.get`).
pub fn client_data(kind: &str, challenge: &str, rp_id: &str) -> String {
    let string = |text: &str| serde_json::to_string(text).unwrap_or_default();
    format!(
        "{{\"type\":{},\"challenge\":{},\"origin\":{},\"crossOrigin\":false}}",
        string(kind),
        string(challenge),
        string(&format!("https://{rp_id}"))
    )
}

fn hash(client_data: &str) -> String {
    URL_SAFE_NO_PAD.encode(ring::digest::digest(
        &ring::digest::SHA256,
        client_data.as_bytes(),
    ))
}

/// The challenge and the relying party's id every ceremony takes.
fn challenge_of(options: &Value) -> Result<&str, Outcome> {
    match text(options, "challenge") {
        Some(challenge) if base64url(challenge) => Ok(challenge),
        _ => Err(type_error("the challenge is base64url")),
    }
}

/// `navigator.credentials.create()`'s ceremony, from `PublicKeyCredentialCreationOptionsJSON`.
pub fn create_at(socket: &Path, options: &Value) -> Result<Outcome, SessionError> {
    let request = match create_request(options) {
        Ok(request) => request,
        Err(outcome) => return Ok(outcome),
    };
    let (client_data, create) = request;
    match ask(socket, &session::Request::PasskeyCreate(create))? {
        Err(outcome) => Ok(outcome),
        Ok(session::Answer::PasskeyCreated(created))
            if base64url(&created.credential_id)
                && base64url(&created.attestation_object)
                && base64url(&created.authenticator_data)
                && base64url(&created.public_key) =>
        {
            Ok(Outcome::Credential(json!({
                "id": created.credential_id,
                "rawId": created.credential_id,
                "type": "public-key",
                "authenticatorAttachment": "platform",
                "response": {
                    "clientDataJSON": URL_SAFE_NO_PAD.encode(client_data.as_bytes()),
                    "attestationObject": created.attestation_object,
                    "authenticatorData": created.authenticator_data,
                    "publicKey": created.public_key,
                    "publicKeyAlgorithm": created.public_key_algorithm,
                    "transports": created.transports,
                },
                "clientExtensionResults": {},
            })))
        }
        Ok(_) => Err(failed(socket, "not a passkey's registration")),
    }
}

fn create_request(options: &Value) -> Result<(String, PasskeyCreate), Outcome> {
    if !options.is_object() {
        return Err(type_error("the options are an object"));
    }
    let rp = options
        .get("rp")
        .ok_or_else(|| type_error("the relying party (rp)"))?;
    let rp_id = text(rp, "id").ok_or_else(|| type_error("the relying party's id (rp.id)"))?;
    let rp_name = text(rp, "name").ok_or_else(|| type_error("the relying party's name"))?;
    let user = options.get("user").ok_or_else(|| type_error("the user"))?;
    let (Some(id), Some(name), Some(display_name)) = (
        text(user, "id"),
        text(user, "name"),
        text(user, "displayName"),
    ) else {
        return Err(type_error("the user's id, name and displayName"));
    };
    if !base64url(id) {
        return Err(type_error("the user's id is base64url"));
    }
    let challenge = challenge_of(options)?;
    // The algorithms asked, in order; none asked is ES256 and RS256 (WebAuthn §5.4).
    let params = options
        .get("pubKeyCredParams")
        .and_then(Value::as_array)
        .ok_or_else(|| type_error("pubKeyCredParams is a list"))?;
    let mut algorithms: Vec<i64> = params
        .iter()
        .filter(|param| text(param, "type") == Some("public-key"))
        .filter_map(|param| param.get("alg").and_then(Value::as_i64))
        .collect();
    if params.is_empty() {
        algorithms = vec![-7, -257];
    }
    if algorithms.is_empty() {
        return Err(Outcome::Error {
            name: "NotSupportedError",
            reason: "notSupported".into(),
        });
    }
    let selection = options.get("authenticatorSelection");
    if selection.and_then(|s| text(s, "authenticatorAttachment")) == Some("cross-platform") {
        return Err(refused("crossPlatform"));
    }
    let exclude = descriptors(options, "excludeCredentials")?;
    let client_data = client_data("webauthn.create", challenge, rp_id);
    let create = PasskeyCreate {
        rp_id: rp_id.to_owned(),
        rp_name: Secret::new(rp_name),
        user: PasskeyUser {
            id: id.to_owned(),
            name: Secret::new(name),
            display_name: Secret::new(display_name),
        },
        client_data_hash: hash(&client_data),
        exclude_credentials: exclude,
        algorithms,
    };
    Ok((client_data, create))
}

/// `navigator.credentials.get()`'s ceremony, from `PublicKeyCredentialRequestOptionsJSON`.
pub fn get_at(socket: &Path, options: &Value) -> Result<Outcome, SessionError> {
    let (client_data, get) = match get_request(options) {
        Ok(request) => request,
        Err(outcome) => return Ok(outcome),
    };
    match ask(socket, &session::Request::PasskeyGet(get))? {
        Err(outcome) => Ok(outcome),
        Ok(session::Answer::PasskeyAssertion(assertion))
            if base64url(&assertion.credential_id)
                && base64url(&assertion.authenticator_data)
                && base64url(&assertion.signature)
                && base64url(&assertion.user_handle) =>
        {
            Ok(Outcome::Credential(json!({
                "id": assertion.credential_id,
                "rawId": assertion.credential_id,
                "type": "public-key",
                "authenticatorAttachment": "platform",
                "response": {
                    "clientDataJSON": URL_SAFE_NO_PAD.encode(client_data.as_bytes()),
                    "authenticatorData": assertion.authenticator_data,
                    "signature": assertion.signature,
                    "userHandle": assertion.user_handle,
                },
                "clientExtensionResults": {},
            })))
        }
        Ok(_) => Err(failed(socket, "not a passkey's assertion")),
    }
}

fn get_request(options: &Value) -> Result<(String, PasskeyGet), Outcome> {
    if !options.is_object() {
        return Err(type_error("the options are an object"));
    }
    let rp_id = text(options, "rpId").ok_or_else(|| type_error("the relying party's id (rpId)"))?;
    let challenge = challenge_of(options)?;
    let allow = descriptors(options, "allowCredentials")?;
    let client_data = client_data("webauthn.get", challenge, rp_id);
    let get = PasskeyGet {
        rp_id: rp_id.to_owned(),
        client_data_hash: hash(&client_data),
        allow_credentials: allow,
    };
    Ok((client_data, get))
}

/// One ceremony on a connection of its own: the service's answer, or WebAuthn's error for its
/// refusal.
fn ask(
    socket: &Path,
    request: &session::Request,
) -> Result<Result<session::Answer, Outcome>, SessionError> {
    let connection = Connection::connect(socket).map_err(|error| failed(socket, error))?;
    connection
        .set_timeout(PASSKEY_WAIT)
        .map_err(|error| failed(socket, error))?;
    connection
        .send(request)
        .map_err(|error| failed(socket, error))?;
    match connection.receive::<session::Answer>() {
        Ok(Some(Some(session::Answer::Refused { reason, .. }))) => {
            Ok(Err(refused(&reason_name(&reason))))
        }
        Ok(Some(Some(answer))) => Ok(Ok(answer)),
        Ok(Some(None)) => Err(failed(socket, "an answer that is not one")),
        Ok(None) => Err(failed(socket, "it closed the connection")),
        Err(error) => Err(failed(socket, error)),
    }
}

fn reason_name(reason: &Reason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "failed".into())
}

/// A ceremony at the session's socket: `ceremony` `create` or `get`.
pub fn ceremony(ceremony: &str, options: &Value) -> Result<Outcome, SessionError> {
    let socket = session_socket();
    match ceremony {
        "create" => create_at(&socket, options),
        "get" => get_at(&socket, options),
        _ => Ok(type_error("a ceremony is create or get")),
    }
}

/// A passkey ceremony for the calling application (docs/adr/0050-keychain-autofill-and-passkeys.mdx,
/// decision 7): `ceremony` "create" with `PublicKeyCredentialCreationOptionsJSON`, or "get" with
/// `PublicKeyCredentialRequestOptionsJSON`, as JSON text. Blocks while the person answers the
/// system's sheet (up to six and a half minutes): never on the interface's thread. Into `out`,
/// JSON: `{"credential": RegistrationResponseJSON | AuthenticationResponseJSON}` or
/// `{"error": {"name" (WebAuthn's DOMException name), "reason"}}`. Its length (larger than
/// `capacity`: nothing written, give OXYOS_ACCOUNT_MESSAGE_MAX bytes), or OXYOS_ERROR
/// (`oxyos_last_error`: the service could not be asked).
///
/// # Safety
/// `ceremony` and `options` are NUL-terminated strings valid for the call; `out` points to
/// `capacity` writable bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_account_session_passkey(
    ceremony: *const std::ffi::c_char,
    options: *const std::ffi::c_char,
    out: *mut std::ffi::c_char,
    capacity: usize,
) -> i64 {
    if out.is_null() && capacity != 0 {
        return i64::from(crate::fail("null buffer"));
    }
    // SAFETY: the caller's contract.
    let (Some(kind), Some(options)) = (unsafe { super::text_arg(ceremony) }, unsafe {
        super::text_arg(options)
    }) else {
        return i64::from(crate::fail("a ceremony and its options, as text"));
    };
    let outcome = match serde_json::from_str::<Value>(options) {
        Ok(options) => self::ceremony(kind, &options),
        Err(_) => Ok(type_error("the options are JSON")),
    };
    match outcome {
        Ok(outcome) => {
            let json = outcome.to_json().to_string();
            // SAFETY: the caller's contract.
            unsafe { super::write_out(json.as_bytes(), out, capacity) }
        }
        Err(error) => super::session_fail(error),
    }
}

#[cfg(test)]
mod tests;
