//! OxyOS as a platform authenticator (OxyOS ADR 0050 decision 7): WebAuthn's two ceremonies with
//! CTAP2's semantics over passkey items (Oxy ADR 0063 D7). The client data (origin, challenge) is
//! the caller's to make and hash, as an authenticator never sees it: this module is given its
//! SHA-256 and the relying party's id.
//!
//! - **The key** is ES256 (COSE -7): P-256 from RustCrypto's `p256`, kept in the item as PKCS #8
//!   (base64url). Signatures are ECDSA over SHA-256, DER-encoded as WebAuthn's assertions carry
//!   them, with RFC 6979's deterministic nonces.
//! - **Authenticator data** (WebAuthn L3 §6.1): `SHA-256(rpId) ‖ flags ‖ signCount (4 bytes,
//!   big-endian)`, and at registration the attested credential data `AAGUID ‖ L (2 bytes) ‖
//!   credentialId ‖ COSE_Key`. The flags are UP and UV (the person verified on the system's own
//!   sheet), BE and BS (the passkey is synced with the account's keychain, as iCloud Keychain's
//!   are), AT at registration. The signature counter is always 0: a synced credential has no one
//!   counter (WebAuthn L3 §6.1.1).
//! - **Attestation** is `none` (§8.7): `{"fmt": "none", "attStmt": {}, "authData"}`, in CTAP2's
//!   canonical CBOR (ciborium writes the maps in the order built, which is that order).
//! - **The AAGUID** is OxyOS's own, [`AAGUID`]: 16 random bytes fixed once, so a relying party can
//!   name the authenticator; with `none` attestation nothing vouches for it.
//! - **Which credentials** ([`matches`], [`excluded`]): passkeys are discoverable credentials of
//!   one relying party; an empty `allowCredentials` asks for any of the relying party's, a list
//!   for those named. An `excludeCredentials` naming one the keychain holds for that relying party
//!   ends a registration (`InvalidStateError`, once the person consented).
//!
//! **Secrets.** The private key is decoded into buffers overwritten when dropped, and the signing
//! key RustCrypto holds zeroes itself when dropped; neither appears in `Debug` or an error.

use crate::item::{Content, Item, Passkey, ES256};
use crate::secret::SecretString;
use crate::wire::{b64, b64_secret, getrandom, unb64, ItemId};
use crate::{Error, Result};
use ciborium::value::{Integer, Value};
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey};
use p256::SecretKey;
use sha2::{Digest, Sha256};
use std::fmt;
use zeroize::Zeroizing;

/// OxyOS's AAGUID, `7c6ab314-ebcd-3fa1-5109-5ada4bb93e0b`: 16 bytes drawn once from a CSPRNG
/// (OxyOS ADR 0050, step 5), the same on every OxyOS machine and never changed.
pub const AAGUID: [u8; 16] = [
    0x7c, 0x6a, 0xb3, 0x14, 0xeb, 0xcd, 0x3f, 0xa1, 0x51, 0x09, 0x5a, 0xda, 0x4b, 0xb9, 0x3e, 0x0b,
];

/// User present.
pub const FLAG_UP: u8 = 0x01;
/// User verified.
pub const FLAG_UV: u8 = 0x04;
/// Backup eligible.
pub const FLAG_BE: u8 = 0x08;
/// Backed up.
pub const FLAG_BS: u8 = 0x10;
/// Attested credential data included.
pub const FLAG_AT: u8 = 0x40;
/// Every assertion's flags: the person present and verified, the credential synced.
pub const FLAGS: u8 = FLAG_UP | FLAG_UV | FLAG_BE | FLAG_BS;

/// A credential id's length: 16 random bytes (Oxy ADR 0063 D7).
pub const CREDENTIAL_ID_LENGTH: usize = 16;
/// The client data's hash: SHA-256.
pub const CLIENT_DATA_HASH_LENGTH: usize = 32;
/// The longest user handle, in bytes (WebAuthn L3 §5.4.3).
pub const MAX_USER_HANDLE: usize = 64;
/// The relying party's and the user's names are kept to this many bytes, cut at a character's
/// edge, as CTAP 2.1 lets an authenticator keep them (§6.1.2, "truncated to 64 bytes").
pub const MAX_NAME: usize = 64;
/// The longest relying party id (a domain name).
pub const MAX_RP_ID: usize = 253;
/// At most this many credential ids in `allowCredentials` or `excludeCredentials`.
pub const MAX_CREDENTIAL_LIST: usize = 64;

/// The client data's SHA-256.
pub type ClientDataHash = [u8; CLIENT_DATA_HASH_LENGTH];

/// Whether `rp_id` is a relying party id OxyOS takes: a domain name as WebAuthn's RP ID is (L3
/// §5.1.3, a valid domain string), in lowercase ASCII (an internationalized one as its A-labels),
/// labels of 1 to 63 letters, digits and hyphens neither first nor last, no trailing dot, not an
/// IP address. Matching a relying party is comparing this text exactly, so no other spelling of a
/// domain is taken.
pub fn valid_rp_id(rp_id: &str) -> bool {
    if rp_id.is_empty() || rp_id.len() > MAX_RP_ID {
        return false;
    }
    let labels: Vec<&str> = rp_id.split('.').collect();
    let well_formed = labels.iter().all(|label| {
        (1..=63).contains(&label.len())
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    });
    // A last label of digits alone is an IPv4 address's, never a domain's.
    let numeric = labels
        .last()
        .is_some_and(|last| last.bytes().all(|b| b.is_ascii_digit()));
    well_formed && !numeric
}

/// Whether the caller's `algorithms` (COSE identifiers, in its order of preference) admit ES256,
/// the one OxyOS makes (CTAP2 `pubKeyCredParams`).
pub fn supports(algorithms: &[i64]) -> bool {
    algorithms.contains(&ES256)
}

/// `text` kept to [`MAX_NAME`] bytes, cut at a character's edge.
fn name(text: &str) -> SecretString {
    let mut end = text.len().min(MAX_NAME);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    SecretString::new(&text[..end])
}

/// What a registration is asked to make (CTAP2 authenticatorMakeCredential, the parts OxyOS
/// keeps).
pub struct NewPasskey<'a> {
    pub rp_id: &'a str,
    pub rp_name: &'a str,
    /// The relying party's user handle, 1 to [`MAX_USER_HANDLE`] bytes.
    pub user_handle: &'a [u8],
    pub user_name: &'a str,
    pub user_display_name: &'a str,
    /// The desktop entry ids of the applications it was made for (none from the browser).
    pub apps: Vec<SecretString>,
}

impl fmt::Debug for NewPasskey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewPasskey").finish_non_exhaustive()
    }
}

/// A registration's answer, as WebAuthn's `AuthenticatorAttestationResponse` carries it.
#[derive(Clone, PartialEq, Eq)]
pub struct Registration {
    pub credential_id: [u8; CREDENTIAL_ID_LENGTH],
    /// The authenticator data, with the attested credential data.
    pub authenticator_data: Vec<u8>,
    /// `none` attestation's CBOR object.
    pub attestation_object: Vec<u8>,
    /// The public key as `getPublicKey()` gives it: SubjectPublicKeyInfo, DER.
    pub public_key: Vec<u8>,
}

impl fmt::Debug for Registration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registration")
            .field("credential_id", &b64(&self.credential_id))
            .finish_non_exhaustive()
    }
}

/// An assertion, as WebAuthn's `AuthenticatorAssertionResponse` carries it.
#[derive(Clone, PartialEq, Eq)]
pub struct Assertion {
    pub credential_id: Vec<u8>,
    pub authenticator_data: Vec<u8>,
    /// ECDSA over SHA-256 of `authenticatorData ‖ clientDataHash`, DER.
    pub signature: Vec<u8>,
    pub user_handle: Vec<u8>,
}

impl fmt::Debug for Assertion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Assertion")
            .field("credential_id", &b64(&self.credential_id))
            .finish_non_exhaustive()
    }
}

/// The authenticator data (WebAuthn L3 §6.1): the relying party id's hash, `flags`, the
/// signature counter (0), and `attested` credential data when given.
pub fn authenticator_data(rp_id: &str, flags: u8, attested: Option<&[u8]>) -> Vec<u8> {
    let mut data = Vec::with_capacity(37 + attested.map_or(0, <[u8]>::len));
    data.extend_from_slice(&Sha256::digest(rp_id.as_bytes()));
    data.push(flags);
    data.extend_from_slice(&0u32.to_be_bytes());
    if let Some(attested) = attested {
        data.extend_from_slice(attested);
    }
    data
}

fn cbor(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).map_err(|_| Error::Malformed("CBOR"))?;
    Ok(bytes)
}

fn int(value: i64) -> Value {
    Value::Integer(Integer::from(value))
}

/// The public key as a COSE_Key (RFC 9053, WebAuthn L3 §6.5.1.1): `{1: 2 (EC2), 3: -7 (ES256),
/// -1: 1 (P-256), -2: x, -3: y}`, its keys in CTAP2's canonical order.
pub fn cose_key(key: &VerifyingKey) -> Result<Vec<u8>> {
    let point = key.to_encoded_point(false);
    let (Some(x), Some(y)) = (point.x(), point.y()) else {
        return Err(Error::Malformed("public key"));
    };
    cbor(&Value::Map(vec![
        (int(1), int(2)),
        (int(3), int(ES256)),
        (int(-1), int(1)),
        (int(-2), Value::Bytes(x.to_vec())),
        (int(-3), Value::Bytes(y.to_vec())),
    ]))
}

/// `none` attestation's object (WebAuthn L3 §8.7), its keys in CTAP2's canonical order.
pub fn attestation_object(authenticator_data: &[u8]) -> Result<Vec<u8>> {
    cbor(&Value::Map(vec![
        (Value::Text("fmt".into()), Value::Text("none".into())),
        (Value::Text("attStmt".into()), Value::Map(Vec::new())),
        (
            Value::Text("authData".into()),
            Value::Bytes(authenticator_data.to_vec()),
        ),
    ]))
}

/// Makes a passkey (CTAP2 authenticatorMakeCredential, the person having verified): a new P-256
/// key and credential id from the system's CSPRNG, the item to keep (`id`, made at `now`) and the
/// registration to answer.
pub fn create(new: &NewPasskey<'_>, id: ItemId, now: &str) -> Result<(Item, Registration)> {
    if !valid_rp_id(new.rp_id) {
        return Err(Error::Malformed("relying party id"));
    }
    if new.user_handle.is_empty() || new.user_handle.len() > MAX_USER_HANDLE {
        return Err(Error::Malformed("user handle"));
    }
    let secret = SecretKey::random(&mut rand_core::OsRng);
    let mut credential_id = [0u8; CREDENTIAL_ID_LENGTH];
    getrandom(&mut credential_id)?;
    let public = secret.public_key();
    let verifying = VerifyingKey::from(&public);
    let cose = cose_key(&verifying)?;
    let mut attested = Vec::with_capacity(AAGUID.len() + 2 + CREDENTIAL_ID_LENGTH + cose.len());
    attested.extend_from_slice(&AAGUID);
    attested.extend_from_slice(&(CREDENTIAL_ID_LENGTH as u16).to_be_bytes());
    attested.extend_from_slice(&credential_id);
    attested.extend_from_slice(&cose);
    let authenticator_data = authenticator_data(new.rp_id, FLAGS | FLAG_AT, Some(&attested));
    let attestation_object = attestation_object(&authenticator_data)?;
    let public_key = public
        .to_public_key_der()
        .map_err(|_| Error::Malformed("public key"))?
        .into_vec();
    let private_key = secret
        .to_pkcs8_der()
        .map_err(|_| Error::Malformed("private key"))?;
    let item = Item {
        id,
        created_at: now.to_owned(),
        modified_at: now.to_owned(),
        counter: 1,
        content: Content::Passkey(Passkey {
            rp_id: SecretString::new(new.rp_id),
            rp_name: name(new.rp_name),
            user_handle: SecretString::new(b64(new.user_handle)),
            user_name: name(new.user_name),
            user_display_name: name(new.user_display_name),
            credential_id: SecretString::new(b64(&credential_id)),
            algorithm: ES256,
            private_key: SecretString::from(b64_secret(private_key.as_bytes())),
            apps: new.apps.clone(),
        }),
    };
    item.validate()?;
    Ok((
        item,
        Registration {
            credential_id,
            authenticator_data,
            attestation_object,
            public_key,
        },
    ))
}

/// Whether `passkey` answers a request of `rp_id` naming `allow` (any of the relying party's when
/// empty).
pub fn matches(passkey: &Passkey, rp_id: &str, allow: &[Vec<u8>]) -> bool {
    passkey.rp_id.expose() == rp_id && (allow.is_empty() || named(passkey, allow))
}

/// Whether a registration for `rp_id` excluding `exclude` is ended by `passkey`.
pub fn excluded(passkey: &Passkey, rp_id: &str, exclude: &[Vec<u8>]) -> bool {
    passkey.rp_id.expose() == rp_id && named(passkey, exclude)
}

fn named(passkey: &Passkey, ids: &[Vec<u8>]) -> bool {
    unb64(passkey.credential_id.expose(), "credential id").is_ok_and(|own| ids.contains(&own))
}

/// Signs an assertion with `passkey` for `rp_id` (CTAP2 authenticatorGetAssertion, the person
/// having verified): the authenticator data with [`FLAGS`], and ECDSA over `authenticatorData ‖
/// clientDataHash`.
pub fn assert(
    passkey: &Passkey,
    rp_id: &str,
    client_data_hash: &ClientDataHash,
) -> Result<Assertion> {
    if passkey.rp_id.expose() != rp_id || passkey.algorithm != ES256 {
        return Err(Error::Malformed("passkey (another relying party)"));
    }
    let der = Zeroizing::new(unb64(passkey.private_key.expose(), "private key")?);
    let secret = SecretKey::from_pkcs8_der(&der).map_err(|_| Error::Malformed("private key"))?;
    let signing = SigningKey::from(&secret);
    let authenticator_data = authenticator_data(rp_id, FLAGS, None);
    let mut signed = Vec::with_capacity(authenticator_data.len() + CLIENT_DATA_HASH_LENGTH);
    signed.extend_from_slice(&authenticator_data);
    signed.extend_from_slice(client_data_hash);
    let signature: Signature = signing.sign(&signed);
    Ok(Assertion {
        credential_id: unb64(passkey.credential_id.expose(), "credential id")?,
        authenticator_data,
        signature: signature.to_der().as_bytes().to_vec(),
        user_handle: unb64(passkey.user_handle.expose(), "user handle")?,
    })
}

#[cfg(test)]
mod tests;
