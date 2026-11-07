//! Items (Oxy ADR 0063 D7, D8): one ciphertext each, under a generation of the vault key. The
//! plaintext is a JSON object padded with trailing spaces to a multiple of 256 bytes, so lengths
//! leak only coarsely; the type and every field are inside it.
//!
//! ```text
//! aad        = "oxy.keychain.item.v1" ‖ 0x00 ‖ vaultId ‖ 0x00 ‖ itemId ‖ 0x00 ‖ g
//! ciphertext = XChaCha20-Poly1305(VK_g, random 24-byte nonce, paddedJson, aad)
//! ```
//!
//! Every item carries its `id` (the one Oxy sees, repeated inside, so a ciphertext cannot be
//! moved to another item), `v` 1, `createdAt`, `modifiedAt` and `counter`, which its writer
//! increments: a device refuses an item whose counter is below the highest it has seen, so Oxy
//! cannot roll an item back to a password the person already changed.
//!
//! The schema is closed and every field is present (as the SDK's `validateItem`): `id` a
//! lowercase uuid, `v` 1, timestamps as JavaScript's `toISOString`, `modifiedAt` not before
//! `createdAt`, `counter` at least 1. The plaintext is the item's JCS, and only that spelling is
//! read back; padded, it is at most [`MAX_PLAINTEXT`] bytes.
//!
//! Every text of an item is a [`SecretString`]: overwritten when dropped, never in `Debug`.

use crate::keyring::Keyring;
use crate::seal::{self, NONCE_LENGTH};
use crate::secret::{self, SecretString};
use crate::wire::{is_timestamp, unb64, ItemId, VaultId, MAX_SAFE_INTEGER};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroizing;

const DOMAIN: &str = "oxy.keychain.item.v1";
const VERSION: u32 = 1;
/// Plaintexts are padded to a multiple of this.
pub const PADDING: usize = 256;
/// A login keeps at most this many previous passwords.
pub const PASSWORD_HISTORY: usize = 10;
/// ES256, the one passkey algorithm (COSE -7).
pub const ES256: i64 = -7;
/// The largest padded plaintext.
pub const MAX_PLAINTEXT: usize = 48_896;
/// The longest SSID, in bytes (IEEE 802.11).
pub const MAX_SSID_BYTES: usize = 32;

/// An item, decrypted.
pub struct Item {
    pub id: ItemId,
    pub created_at: String,
    pub modified_at: String,
    pub counter: u64,
    pub content: Content,
}

impl fmt::Debug for Item {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Item")
            .field("id", &self.id)
            .field("type", &self.content.kind())
            .field("counter", &self.counter)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum Content {
    Login(Login),
    Passkey(Passkey),
    Totp(Totp),
    Note(Note),
    Wifi(Wifi),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Login,
    Passkey,
    Totp,
    Note,
    Wifi,
}

impl Content {
    pub fn kind(&self) -> Kind {
        match self {
            Content::Login(_) => Kind::Login,
            Content::Passkey(_) => Kind::Passkey,
            Content::Totp(_) => Kind::Totp,
            Content::Note(_) => Kind::Note,
            Content::Wifi(_) => Kind::Wifi,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Url {
    pub url: SecretString,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviousPassword {
    pub changed_at: String,
    pub password: SecretString,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Login {
    pub title: SecretString,
    pub username: SecretString,
    pub password: SecretString,
    /// Matched by registrable domain.
    pub urls: Vec<Url>,
    /// OxyOS desktop entry ids, iOS and Android package ids, none empty.
    pub apps: Vec<SecretString>,
    /// An `otpauth://totp/` URI, or none (`null`).
    pub totp: Option<SecretString>,
    pub notes: SecretString,
    /// Newest first, at most [`PASSWORD_HISTORY`].
    pub password_history: Vec<PreviousPassword>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Passkey {
    pub rp_id: SecretString,
    pub rp_name: SecretString,
    /// base64url of 1 to 64 bytes.
    pub user_handle: SecretString,
    pub user_name: SecretString,
    pub user_display_name: SecretString,
    /// base64url of 16 random bytes.
    pub credential_id: SecretString,
    /// [`ES256`].
    pub algorithm: i64,
    /// PKCS #8, base64url, at most 1024 bytes.
    pub private_key: SecretString,
    pub apps: Vec<SecretString>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Totp {
    pub title: SecretString,
    pub issuer: SecretString,
    pub account: SecretString,
    /// An `otpauth://totp/` URI.
    pub uri: SecretString,
    pub notes: SecretString,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Note {
    pub title: SecretString,
    pub body: SecretString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WifiSecurity {
    #[serde(rename = "wpa2-psk")]
    Wpa2Psk,
    #[serde(rename = "wpa3-sae")]
    Wpa3Sae,
    #[serde(rename = "wep")]
    Wep,
    #[serde(rename = "open")]
    Open,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Wifi {
    /// 1 to [`MAX_SSID_BYTES`] bytes.
    pub ssid: SecretString,
    /// Empty for an open network, and only for one.
    pub password: SecretString,
    pub security: WifiSecurity,
    pub hidden: bool,
}

/// Every type's fields in one object, those of the other types absent, declared in RFC 8785's
/// order so that what is written is canonical (tested against the canonicalizer).
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Wire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    algorithm: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    apps: Option<Vec<SecretString>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<SecretString>,
    counter: u64,
    created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential_id: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hidden: Option<bool>,
    id: Option<ItemId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    issuer: Option<SecretString>,
    modified_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notes: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password_history: Option<Vec<PreviousPassword>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    private_key: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rp_id: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rp_name: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    security: Option<WifiSecurity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssid: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<SecretString>,
    /// A login's: present as a URI or `null`; absent from the other types.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    totp: Option<Option<SecretString>>,
    #[serde(rename = "type")]
    kind: Option<Kind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    uri: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    urls: Option<Vec<Url>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_display_name: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_handle: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_name: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    username: Option<SecretString>,
    v: u32,
}

/// An item's ciphertext as Oxy holds it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SealedItem {
    pub generation: u32,
    #[serde(with = "crate::wrap::nonce24")]
    pub nonce: [u8; NONCE_LENGTH],
    #[serde(with = "crate::wrap::bytes")]
    pub ciphertext: Vec<u8>,
}

impl fmt::Debug for SealedItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SealedItem")
            .field("generation", &self.generation)
            .field("ciphertext_len", &self.ciphertext.len())
            .finish_non_exhaustive()
    }
}

/// `"oxy.keychain.item.v1" ‖ 0x00 ‖ vaultId ‖ 0x00 ‖ itemId ‖ 0x00 ‖ g`.
pub fn aad(vault_id: &VaultId, item_id: &ItemId, generation: u32) -> Vec<u8> {
    [
        DOMAIN.as_bytes(),
        &[0],
        vault_id.as_str().as_bytes(),
        &[0],
        item_id.as_str().as_bytes(),
        &[0],
        generation.to_string().as_bytes(),
    ]
    .concat()
}

/// A field present, even as `null`: `Some(None)`, not the `None` of a field absent.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

fn need<T>(field: Option<T>) -> Result<T> {
    field.ok_or(Error::Malformed("item (a field missing)"))
}

fn check(condition: bool, what: &'static str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Malformed(what))
    }
}

/// A base64url field of `range` bytes.
fn check_b64(text: &SecretString, range: std::ops::RangeInclusive<usize>) -> Result<()> {
    let bytes = Zeroizing::new(unb64(text.expose(), "item")?);
    check(range.contains(&bytes.len()), "item (a field's length)")
}

fn check_otpauth(uri: &SecretString) -> Result<()> {
    crate::totp::Totp::parse(uri.expose())
        .map(drop)
        .map_err(|_| Error::Malformed("item (not an otpauth://totp/ URI)"))
}

fn check_texts(texts: &[SecretString]) -> Result<()> {
    check(
        texts.iter().all(|text| !text.is_empty()),
        "item (an empty entry)",
    )
}

impl Item {
    fn to_wire(&self) -> Wire {
        let mut wire = Wire {
            counter: self.counter,
            created_at: self.created_at.clone(),
            id: Some(self.id.clone()),
            kind: Some(self.content.kind()),
            modified_at: self.modified_at.clone(),
            v: VERSION,
            ..Wire::default()
        };
        match &self.content {
            Content::Login(login) => {
                wire.title = Some(login.title.clone());
                wire.username = Some(login.username.clone());
                wire.password = Some(login.password.clone());
                wire.urls = Some(login.urls.clone());
                wire.apps = Some(login.apps.clone());
                wire.totp = Some(login.totp.clone());
                wire.notes = Some(login.notes.clone());
                wire.password_history = Some(login.password_history.clone());
            }
            Content::Passkey(passkey) => {
                wire.rp_id = Some(passkey.rp_id.clone());
                wire.rp_name = Some(passkey.rp_name.clone());
                wire.user_handle = Some(passkey.user_handle.clone());
                wire.user_name = Some(passkey.user_name.clone());
                wire.user_display_name = Some(passkey.user_display_name.clone());
                wire.credential_id = Some(passkey.credential_id.clone());
                wire.algorithm = Some(passkey.algorithm);
                wire.private_key = Some(passkey.private_key.clone());
                wire.apps = Some(passkey.apps.clone());
            }
            Content::Totp(totp) => {
                wire.title = Some(totp.title.clone());
                wire.issuer = Some(totp.issuer.clone());
                wire.account = Some(totp.account.clone());
                wire.uri = Some(totp.uri.clone());
                wire.notes = Some(totp.notes.clone());
            }
            Content::Note(note) => {
                wire.title = Some(note.title.clone());
                wire.body = Some(note.body.clone());
            }
            Content::Wifi(wifi) => {
                wire.ssid = Some(wifi.ssid.clone());
                wire.password = Some(wifi.password.clone());
                wire.security = Some(wifi.security);
                wire.hidden = Some(wifi.hidden);
            }
        }
        wire
    }

    fn from_wire(mut wire: Wire) -> Result<Self> {
        let id = need(wire.id.take())?;
        // Take each type's fields; whatever is left belongs to another type, and is refused.
        let content = match need(wire.kind.take())? {
            Kind::Login => Content::Login(Login {
                title: need(wire.title.take())?,
                username: need(wire.username.take())?,
                password: need(wire.password.take())?,
                urls: need(wire.urls.take())?,
                apps: need(wire.apps.take())?,
                totp: need(wire.totp.take())?,
                notes: need(wire.notes.take())?,
                password_history: need(wire.password_history.take())?,
            }),
            Kind::Passkey => Content::Passkey(Passkey {
                rp_id: need(wire.rp_id.take())?,
                rp_name: need(wire.rp_name.take())?,
                user_handle: need(wire.user_handle.take())?,
                user_name: need(wire.user_name.take())?,
                user_display_name: need(wire.user_display_name.take())?,
                credential_id: need(wire.credential_id.take())?,
                algorithm: need(wire.algorithm.take())?,
                private_key: need(wire.private_key.take())?,
                apps: need(wire.apps.take())?,
            }),
            Kind::Totp => Content::Totp(Totp {
                title: need(wire.title.take())?,
                issuer: need(wire.issuer.take())?,
                account: need(wire.account.take())?,
                uri: need(wire.uri.take())?,
                notes: need(wire.notes.take())?,
            }),
            Kind::Note => Content::Note(Note {
                title: need(wire.title.take())?,
                body: need(wire.body.take())?,
            }),
            Kind::Wifi => Content::Wifi(Wifi {
                ssid: need(wire.ssid.take())?,
                password: need(wire.password.take())?,
                security: need(wire.security.take())?,
                hidden: need(wire.hidden.take())?,
            }),
        };
        let leftover = wire.account.is_some()
            || wire.algorithm.is_some()
            || wire.apps.is_some()
            || wire.body.is_some()
            || wire.credential_id.is_some()
            || wire.hidden.is_some()
            || wire.issuer.is_some()
            || wire.notes.is_some()
            || wire.password.is_some()
            || wire.password_history.is_some()
            || wire.private_key.is_some()
            || wire.rp_id.is_some()
            || wire.rp_name.is_some()
            || wire.security.is_some()
            || wire.ssid.is_some()
            || wire.title.is_some()
            || wire.totp.is_some()
            || wire.uri.is_some()
            || wire.urls.is_some()
            || wire.user_display_name.is_some()
            || wire.user_handle.is_some()
            || wire.user_name.is_some()
            || wire.username.is_some();
        check(
            !leftover && wire.v == VERSION,
            "item (a field of another type)",
        )?;
        let item = Item {
            id,
            created_at: std::mem::take(&mut wire.created_at),
            modified_at: std::mem::take(&mut wire.modified_at),
            counter: wire.counter,
            content,
        };
        item.validate()?;
        Ok(item)
    }

    /// The rules of the SDK's `validateItem`, checked before writing and after reading.
    pub fn validate(&self) -> Result<()> {
        check(
            is_timestamp(&self.created_at) && is_timestamp(&self.modified_at),
            "item (timestamps)",
        )?;
        // Both are fixed-width UTC: their text orders as their times.
        check(
            self.modified_at >= self.created_at,
            "item (modified before created)",
        )?;
        check(
            (1..=MAX_SAFE_INTEGER).contains(&self.counter),
            "item (counter)",
        )?;
        match &self.content {
            Content::Login(login) => {
                check(
                    login.urls.iter().all(|url| !url.url.is_empty()),
                    "item (an empty URL)",
                )?;
                check_texts(&login.apps)?;
                if let Some(totp) = &login.totp {
                    check_otpauth(totp)?;
                }
                let history = &login.password_history;
                check(history.len() <= PASSWORD_HISTORY, "item (password history)")?;
                check(
                    history.iter().all(|old| is_timestamp(&old.changed_at))
                        && history
                            .windows(2)
                            .all(|w| w[1].changed_at <= w[0].changed_at),
                    "item (password history, newest first)",
                )?;
            }
            Content::Passkey(passkey) => {
                check(
                    passkey.algorithm == ES256 && !passkey.rp_id.is_empty(),
                    "item (passkey)",
                )?;
                // WebAuthn: a user handle of 1 to 64 bytes; the credential id is 16 random bytes.
                check_b64(&passkey.user_handle, 1..=64)?;
                check_b64(&passkey.credential_id, 16..=16)?;
                check_b64(&passkey.private_key, 1..=1024)?;
                check_texts(&passkey.apps)?;
            }
            Content::Totp(totp) => check_otpauth(&totp.uri)?,
            Content::Note(_) => {}
            Content::Wifi(wifi) => {
                check(
                    !wifi.ssid.is_empty() && wifi.ssid.len() <= MAX_SSID_BYTES,
                    "item (SSID)",
                )?;
                check(
                    (wifi.security == WifiSecurity::Open) == wifi.password.is_empty(),
                    "item (an open network has no password, any other one has)",
                )?;
            }
        }
        Ok(())
    }

    /// The item's JCS.
    fn to_json(&self) -> Result<Zeroizing<Vec<u8>>> {
        self.validate()?;
        secret::json(&self.to_wire())
    }

    /// The padded plaintext: the item's JCS and trailing spaces to a multiple of 256 bytes (at
    /// least 256).
    pub fn to_plaintext(&self) -> Result<Zeroizing<Vec<u8>>> {
        let json = self.to_json()?;
        let padded = json.len().div_ceil(PADDING).max(1) * PADDING;
        check(padded <= MAX_PLAINTEXT, "item (too large)")?;
        let mut plaintext = Zeroizing::new(Vec::with_capacity(padded));
        plaintext.extend_from_slice(&json);
        plaintext.resize(padded, b' ');
        Ok(plaintext)
    }

    /// Reads a padded plaintext: only the item's JCS, then spaces.
    pub fn from_plaintext(plaintext: &[u8]) -> Result<Self> {
        check(
            !plaintext.is_empty()
                && plaintext.len() % PADDING == 0
                && plaintext.len() <= MAX_PLAINTEXT,
            "item (padding)",
        )?;
        // JCS never ends in a space, so the padding is exactly the trailing spaces.
        let end = plaintext
            .iter()
            .rposition(|byte| *byte != b' ')
            .map_or(0, |last| last + 1);
        let json = &plaintext[..end];
        let wire: Wire = serde_json::from_slice(json).map_err(|_| Error::Malformed("item"))?;
        let item = Self::from_wire(wire)?;
        // Another spelling (an order, an escape, white space, a duplicate) is not the item's.
        check(*item.to_json()? == *json, "item (not canonical)")?;
        Ok(item)
    }

    /// Seals the item under the keyring's newest generation.
    pub fn seal(&self, vault_id: &VaultId, keyring: &Keyring) -> Result<SealedItem> {
        self.seal_at(&seal::nonce()?, keyring.generation(), vault_id, keyring)
    }

    /// [`Item::seal`] with the nonce and the generation given; outside this module, for the
    /// vectors only.
    pub(crate) fn seal_at(
        &self,
        nonce: &[u8; NONCE_LENGTH],
        generation: u32,
        vault_id: &VaultId,
        keyring: &Keyring,
    ) -> Result<SealedItem> {
        let key = keyring
            .get(generation)
            .ok_or(Error::Malformed("item (a generation the keyring lacks)"))?;
        let plaintext = self.to_plaintext()?;
        let ciphertext = seal::encrypt(
            key.bytes(),
            nonce,
            &plaintext,
            &aad(vault_id, &self.id, generation),
        )?;
        Ok(SealedItem {
            generation,
            nonce: *nonce,
            ciphertext,
        })
    }

    /// Opens item `item_id`, refusing it if its counter is below `highest_seen`, the highest
    /// counter this device has seen for it.
    pub fn open(
        vault_id: &VaultId,
        item_id: &ItemId,
        sealed: &SealedItem,
        keyring: &Keyring,
        highest_seen: Option<u64>,
    ) -> Result<Self> {
        let key = keyring
            .get(sealed.generation)
            .ok_or(Error::Malformed("item (a generation the keyring lacks)"))?;
        let plaintext = seal::decrypt(
            key.bytes(),
            &sealed.nonce,
            &sealed.ciphertext,
            &aad(vault_id, item_id, sealed.generation),
        )?;
        let item = Self::from_plaintext(&plaintext)?;
        if item.id != *item_id {
            return Err(Error::Malformed("item (another item's id inside)"));
        }
        if highest_seen.is_some_and(|seen| item.counter < seen) {
            return Err(Error::Rollback);
        }
        Ok(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::AT;

    const LATER: &str = "2026-11-07T12:30:00.000Z";
    const TOTP_URI: &str = "otpauth://totp/Ex:ana?secret=JBSWY3DPEHPK3PXP&issuer=Ex";

    fn t(text: &str) -> SecretString {
        SecretString::new(text)
    }

    fn item(content: Content) -> Item {
        Item {
            id: ItemId::random().unwrap(),
            created_at: AT.into(),
            modified_at: LATER.into(),
            counter: 1,
            content,
        }
    }

    fn login() -> Login {
        Login {
            title: t("Mail"),
            username: t("ana@example.com"),
            password: t("abcde-FGHIJ-k1m2n-OPq3r"),
            urls: vec![Url {
                url: t("https://mail.example.com"),
            }],
            apps: vec![t("so.oxy.mail")],
            totp: Some(t(TOTP_URI)),
            notes: t(""),
            password_history: vec![
                PreviousPassword {
                    changed_at: "2026-11-07T12:20:00.000Z".into(),
                    password: t("second"),
                },
                PreviousPassword {
                    changed_at: "2026-11-07T12:10:00.000Z".into(),
                    password: t("first"),
                },
            ],
        }
    }

    fn every_type() -> Vec<Item> {
        vec![
            item(Content::Login(login())),
            item(Content::Login(Login {
                totp: None,
                ..login()
            })),
            item(Content::Passkey(Passkey {
                rp_id: t("example.com"),
                rp_name: t("Example"),
                user_handle: t(&crate::wire::b64(&[9; 32])),
                user_name: t("ana"),
                user_display_name: t("Ana"),
                credential_id: t(&crate::wire::b64(&[7; 16])),
                algorithm: ES256,
                private_key: t(&crate::wire::b64(&[5; 138])),
                apps: vec![],
            })),
            item(Content::Totp(Totp {
                title: t("Ex"),
                issuer: t("Ex"),
                account: t("ana"),
                uri: t(TOTP_URI),
                notes: t("recovery codes in the safe"),
            })),
            item(Content::Note(Note {
                title: t("Door"),
                body: t("4711 \u{0} \\ \"q\" 😀"),
            })),
            item(Content::Wifi(Wifi {
                ssid: t("Casa 🏠"),
                password: t("correct horse"),
                security: WifiSecurity::Wpa3Sae,
                hidden: true,
            })),
            item(Content::Wifi(Wifi {
                ssid: t("Café"),
                password: t(""),
                security: WifiSecurity::Open,
                hidden: false,
            })),
        ]
    }

    fn keyring() -> Keyring {
        Keyring::from_keys(&[[1; 32], [2; 32]])
    }

    fn vault() -> VaultId {
        VaultId::random().unwrap()
    }

    #[test]
    fn every_type_round_trips_padded_and_canonical() {
        let vault = vault();
        let keyring = keyring();
        for item in every_type() {
            let plaintext = item.to_plaintext().unwrap();
            assert_eq!(plaintext.len() % PADDING, 0);
            let json = std::str::from_utf8(&plaintext).unwrap().trim_end();
            let value: serde_json::Value = serde_json::from_str(json).unwrap();
            assert_eq!(
                serde_json_canonicalizer::to_string(&value).unwrap(),
                json,
                "{:?} is written in its JCS form",
                item.content.kind()
            );
            let sealed = item.seal(&vault, &keyring).unwrap();
            assert_eq!(sealed.generation, 2);
            assert_eq!(sealed.ciphertext.len(), plaintext.len() + 16);
            let opened = Item::open(&vault, &item.id, &sealed, &keyring, None).unwrap();
            assert_eq!(*opened.to_plaintext().unwrap(), *plaintext);
            assert_eq!(opened.content.kind(), item.content.kind());
        }
    }

    #[test]
    fn a_ciphertext_is_bound_to_its_vault_item_and_generation() {
        let vault = vault();
        let keyring = keyring();
        let item = item(Content::Login(login()));
        let sealed = item.seal(&vault, &keyring).unwrap();
        let open = |vault: &VaultId, id: &ItemId, sealed: &SealedItem| {
            Item::open(vault, id, sealed, &keyring, None)
        };
        assert!(matches!(
            open(&self::vault(), &item.id, &sealed),
            Err(Error::Decrypt)
        ));
        assert!(matches!(
            open(&vault, &ItemId::random().unwrap(), &sealed),
            Err(Error::Decrypt)
        ));
        let mut older = sealed.clone();
        older.generation = 1;
        assert!(matches!(
            open(&vault, &item.id, &older),
            Err(Error::Decrypt)
        ));
        let mut unknown = sealed.clone();
        unknown.generation = 3;
        assert!(matches!(
            open(&vault, &item.id, &unknown),
            Err(Error::Malformed(_))
        ));
        let mut changed = sealed.clone();
        changed.ciphertext[0] ^= 1;
        assert!(matches!(
            open(&vault, &item.id, &changed),
            Err(Error::Decrypt)
        ));
        // An item sealed under another item's id (by a writer holding the key): the id inside
        // says it is not the one asked for.
        let other = ItemId::random().unwrap();
        let ciphertext = seal::encrypt(
            keyring.get(2).unwrap().bytes(),
            &[4; 24],
            &item.to_plaintext().unwrap(),
            &aad(&vault, &other, 2),
        )
        .unwrap();
        let moved = SealedItem {
            generation: 2,
            nonce: [4; 24],
            ciphertext,
        };
        assert!(matches!(
            open(&vault, &other, &moved),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn a_counter_never_goes_back() {
        let vault = vault();
        let keyring = keyring();
        let mut item = item(Content::Note(Note {
            title: t("n"),
            body: t("v1"),
        }));
        item.counter = 5;
        let sealed = item.seal(&vault, &keyring).unwrap();
        for seen in [None, Some(1), Some(5)] {
            assert!(Item::open(&vault, &item.id, &sealed, &keyring, seen).is_ok());
        }
        assert!(matches!(
            Item::open(&vault, &item.id, &sealed, &keyring, Some(6)),
            Err(Error::Rollback)
        ));
    }

    fn refused(edit: impl Fn(&mut Item)) -> bool {
        let mut changed = item(Content::Login(login()));
        edit(&mut changed);
        changed.to_plaintext().is_err()
    }

    #[test]
    fn writing_checks_the_sdks_rules() {
        assert!(!refused(|_| {}));
        assert!(refused(|i| i.counter = 0));
        assert!(refused(|i| i.counter = MAX_SAFE_INTEGER + 1));
        assert!(refused(|i| i.created_at = "2026-11-07T12:00:00Z".into()));
        assert!(refused(
            |i| i.modified_at = "2026-11-07T11:00:00.000Z".into()
        ));
        let login = |edit: fn(&mut Login)| {
            move |i: &mut Item| {
                if let Content::Login(login) = &mut i.content {
                    edit(login)
                }
            }
        };
        assert!(refused(login(|l| l.urls.push(Url { url: t("") }))));
        assert!(refused(login(|l| l.apps.push(t("")))));
        assert!(refused(login(
            |l| l.totp = Some(t("otpauth://hotp/x?secret=AA"))
        )));
        assert!(refused(login(|l| l.password_history.reverse())));
        assert!(refused(login(|l| {
            let one = l.password_history[1].clone();
            l.password_history = vec![one; 11];
        })));
        let wifi = |security, password: &str, ssid: &str| {
            item(Content::Wifi(Wifi {
                ssid: t(ssid),
                password: t(password),
                security,
                hidden: false,
            }))
            .to_plaintext()
            .is_err()
        };
        assert!(wifi(WifiSecurity::Open, "secret", "Net"));
        assert!(wifi(WifiSecurity::Wpa2Psk, "", "Net"));
        assert!(wifi(WifiSecurity::Wpa2Psk, "pw", ""));
        assert!(wifi(WifiSecurity::Wpa2Psk, "pw", &"x".repeat(33)));
        assert!(!wifi(WifiSecurity::Wpa2Psk, "pw", &"x".repeat(32)));
        let passkey = |algorithm, credential: &[u8], handle: &[u8]| {
            item(Content::Passkey(Passkey {
                rp_id: t("example.com"),
                rp_name: t(""),
                user_handle: t(&crate::wire::b64(handle)),
                user_name: t(""),
                user_display_name: t(""),
                credential_id: t(&crate::wire::b64(credential)),
                algorithm,
                private_key: t(&crate::wire::b64(&[1; 100])),
                apps: vec![],
            }))
            .to_plaintext()
            .is_err()
        };
        assert!(!passkey(ES256, &[0; 16], &[1]));
        assert!(passkey(-8, &[0; 16], &[1]));
        assert!(passkey(ES256, &[0; 15], &[1]));
        assert!(passkey(ES256, &[0; 16], &[]));
        assert!(passkey(ES256, &[0; 16], &[1; 65]));
        // Too large once padded.
        let big = item(Content::Note(Note {
            title: t("big"),
            body: t(&"x".repeat(MAX_PLAINTEXT)),
        }));
        assert!(big.to_plaintext().is_err());
    }

    #[test]
    fn reading_takes_nothing_but_the_canonical_closed_schema() {
        let item = item(Content::Login(login()));
        let plaintext = item.to_plaintext().unwrap();
        let json = std::str::from_utf8(&plaintext)
            .unwrap()
            .trim_end()
            .to_owned();
        let pad = |json: &str| {
            let mut bytes = json.as_bytes().to_vec();
            bytes.resize(json.len().div_ceil(PADDING).max(1) * PADDING, b' ');
            bytes
        };
        assert!(Item::from_plaintext(&pad(&json)).is_ok());
        for (bad, why) in [
            (json.replacen(':', ": ", 1), "white space"),
            (json.replacen('{', "{\"extra\":1,", 1), "an unknown field"),
            (
                json.replacen('{', "{\"body\":\"x\",", 1),
                "another type's field",
            ),
            (json.replace("\"notes\":\"\",", ""), "a field missing"),
            (
                json.replace("\"type\":\"login\"", "\"type\":\"secret\""),
                "an unknown type",
            ),
            (json.replace("\"v\":1", "\"v\":2"), "another version"),
            (
                json.replace("\"counter\":1", "\"counter\":1,\"counter\":1"),
                "a duplicate",
            ),
            (
                json.replace("\"apps\":[", "\"apps\":[ "),
                "white space in a list",
            ),
            (
                json.replace("ana@", "ana\\u0040"),
                "an escape where none is needed",
            ),
        ] {
            assert!(Item::from_plaintext(&pad(&bad)).is_err(), "{why}");
        }
        // Padding: not a multiple of 256, something other than spaces after the JSON, empty.
        assert!(Item::from_plaintext(&plaintext[..plaintext.len() - 1]).is_err());
        let mut tail = plaintext.to_vec();
        let last = tail.len() - 1;
        tail[last] = b'\n';
        assert!(Item::from_plaintext(&tail).is_err());
        assert!(Item::from_plaintext(&[]).is_err());
        assert!(Item::from_plaintext(&[b' '; 256]).is_err());
    }

    #[test]
    fn debug_shows_nothing_of_an_item() {
        for item in every_type() {
            let sealed = item.seal(&vault(), &keyring()).unwrap();
            let shown = format!("{item:?} {:?} {sealed:?}", item.content);
            // The random id is shown, and may happen to hold one of the texts below.
            let shown = shown.replace(item.id.as_str(), "<id>");
            for secret in [
                "Mail",
                "ana@example.com",
                "abcde",
                "mail.example.com",
                "so.oxy.mail",
                "JBSWY3DP",
                "second",
                "example.com",
                "Door",
                "4711",
                "Casa",
                "correct horse",
                "recovery codes",
            ] {
                assert!(!shown.contains(secret), "{secret} in {shown}");
            }
        }
    }
}
