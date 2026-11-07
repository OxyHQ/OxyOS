//! The keybag (OxyOS ADR 0050 decision 2): 32 random bytes, the key everything the machine keeps
//! of the keychain is sealed under, kept only wrapped by a key derived from the account's
//! password, as a Mac's login keychain is sealed by the login password.
//!
//! ```text
//! wrapping key = Argon2id(password, 16-byte random salt, m = 64 MiB, t = 3, p = 1, 32 bytes)
//! wrapped      = XChaCha20-Poly1305(wrapping key, random 24-byte nonce, keybag key, "oxyos.keychain.keybag.v1")
//! ```
//!
//! A keybag of other parameters is refused, never opened with weaker ones. A new password
//! re-seals the same keybag key (new salt, new nonce) given the old password; without it, the
//! keybag cannot be opened, by design.

use crate::seal::{self, NONCE_LENGTH};
use crate::wire::{b64, getrandom, unb64_fixed};
use crate::{Error, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroizing;

const AAD: &[u8] = b"oxyos.keychain.keybag.v1";
const VERSION: u32 = 1;
const KDF: &str = "argon2id";
/// 64 MiB, in KiB.
pub const MEMORY_KIB: u32 = 64 * 1024;
pub const ITERATIONS: u32 = 3;
pub const PARALLELISM: u32 = 1;
pub const SALT_LENGTH: usize = 16;

/// The keybag key, in memory only while the session is unlocked.
pub struct KeybagKey(Box<Zeroizing<[u8; 32]>>);

impl KeybagKey {
    pub(crate) fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for KeybagKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeybagKey(…)")
    }
}

/// The keybag key wrapped by the password, as its file holds it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Keybag {
    ciphertext: String,
    iterations: u32,
    kdf: String,
    memory_kib: u32,
    nonce: String,
    parallelism: u32,
    salt: String,
    v: u32,
}

impl fmt::Debug for Keybag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keybag")
            .field("kdf", &self.kdf)
            .field("memory_kib", &self.memory_kib)
            .field("iterations", &self.iterations)
            .field("parallelism", &self.parallelism)
            .finish_non_exhaustive()
    }
}

/// Argon2id of `password` and `salt` at the keybag's parameters.
fn wrapping_key(password: &str, salt: &[u8; SALT_LENGTH]) -> Result<Zeroizing<[u8; 32]>> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, PARALLELISM, Some(32))
        .map_err(|_| Error::Malformed("keybag parameters"))?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut *key)
        .map_err(|_| Error::Malformed("keybag parameters"))?;
    Ok(key)
}

impl Keybag {
    /// A new keybag key, wrapped by `password`.
    pub fn create(password: &str) -> Result<(Self, KeybagKey)> {
        let mut key = Box::new(Zeroizing::new([0u8; 32]));
        getrandom(&mut **key)?;
        let key = KeybagKey(key);
        Ok((Self::wrap(&key, password)?, key))
    }

    fn wrap(key: &KeybagKey, password: &str) -> Result<Self> {
        if password.is_empty() {
            return Err(Error::Malformed("password (empty)"));
        }
        let mut salt = [0u8; SALT_LENGTH];
        getrandom(&mut salt)?;
        let nonce = seal::nonce()?;
        let wrapping = wrapping_key(password, &salt)?;
        let ciphertext = seal::encrypt(&wrapping, &nonce, key.bytes(), AAD)?;
        Ok(Self {
            ciphertext: b64(&ciphertext),
            iterations: ITERATIONS,
            kdf: KDF.into(),
            memory_kib: MEMORY_KIB,
            nonce: b64(&nonce),
            parallelism: PARALLELISM,
            salt: b64(&salt),
            v: VERSION,
        })
    }

    /// The keybag key, if `password` opens it.
    pub fn unlock(&self, password: &str) -> Result<KeybagKey> {
        if self.v != VERSION
            || self.kdf != KDF
            || self.memory_kib != MEMORY_KIB
            || self.iterations != ITERATIONS
            || self.parallelism != PARALLELISM
        {
            return Err(Error::Malformed(
                "keybag (not of this version's parameters)",
            ));
        }
        let salt = unb64_fixed::<SALT_LENGTH>(&self.salt, "keybag")?;
        let nonce = unb64_fixed::<NONCE_LENGTH>(&self.nonce, "keybag")?;
        let ciphertext = unb64_fixed::<48>(&self.ciphertext, "keybag")?;
        let wrapping = wrapping_key(password, &salt)?;
        let opened =
            seal::decrypt(&wrapping, &nonce, &ciphertext, AAD).map_err(|error| match error {
                Error::Decrypt => Error::WrongPassword,
                other => other,
            })?;
        let mut key = Box::new(Zeroizing::new([0u8; 32]));
        key.copy_from_slice(&opened);
        Ok(KeybagKey(key))
    }

    /// The same keybag key wrapped by `new`, given the `old` password that opens it.
    pub fn reseal(&self, old: &str, new: &str) -> Result<Self> {
        let key = self.unlock(old)?;
        Self::wrap(&key, new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_with_its_password_alone() {
        let (keybag, key) = Keybag::create("correct horse battery staple").unwrap();
        let opened = keybag.unlock("correct horse battery staple").unwrap();
        assert_eq!(opened.bytes(), key.bytes());
        assert!(matches!(
            keybag.unlock("correct horse battery stapler"),
            Err(Error::WrongPassword)
        ));
        assert!(matches!(keybag.unlock(""), Err(Error::WrongPassword)));
        assert!(Keybag::create("").is_err());
    }

    #[test]
    fn reseals_under_a_new_password() {
        let (keybag, key) = Keybag::create("old password").unwrap();
        assert!(matches!(
            keybag.reseal("wrong", "new password"),
            Err(Error::WrongPassword)
        ));
        let resealed = keybag.reseal("old password", "new password").unwrap();
        assert_ne!(resealed.salt, keybag.salt);
        assert_eq!(
            resealed.unlock("new password").unwrap().bytes(),
            key.bytes()
        );
        assert!(matches!(
            resealed.unlock("old password"),
            Err(Error::WrongPassword)
        ));
    }

    #[test]
    fn refuses_weaker_parameters_and_changes() {
        let (keybag, _) = Keybag::create("pw").unwrap();
        let mut weaker = keybag.clone();
        weaker.memory_kib = 8;
        assert!(matches!(weaker.unlock("pw"), Err(Error::Malformed(_))));
        let mut weaker = keybag.clone();
        weaker.iterations = 1;
        assert!(matches!(weaker.unlock("pw"), Err(Error::Malformed(_))));
        let mut other = keybag.clone();
        other.kdf = "argon2i".into();
        assert!(matches!(other.unlock("pw"), Err(Error::Malformed(_))));
        // Another salt derives another key: the ciphertext does not open.
        let mut salted = keybag.clone();
        salted.salt = b64(&[9; SALT_LENGTH]);
        assert!(matches!(salted.unlock("pw"), Err(Error::WrongPassword)));
        let json = serde_json::to_string(&keybag).unwrap();
        let extra = json.replacen('{', "{\"hint\":\"x\",", 1);
        assert!(serde_json::from_str::<Keybag>(&extra).is_err());
    }

    #[test]
    fn argon2id_rfc9106_vector() {
        // RFC 9106 § 5.3: Argon2id with a secret and associated data, through the same crate.
        let params = argon2::ParamsBuilder::new()
            .m_cost(32)
            .t_cost(3)
            .p_cost(4)
            .data(argon2::AssociatedData::new(&[4; 12]).unwrap())
            .output_len(32)
            .build()
            .unwrap();
        let argon2 =
            Argon2::new_with_secret(&[3; 8], Algorithm::Argon2id, Version::V0x13, params).unwrap();
        let mut out = [0u8; 32];
        argon2
            .hash_password_into(&[1; 32], &[2; 16], &mut out)
            .unwrap();
        assert_eq!(
            hex::encode(out),
            "0d640df58d78766c08c037a34a8b53c9d01ef0452d75b65eb52520e96b01e659"
        );
    }

    #[test]
    fn debug_shows_no_key() {
        let (keybag, key) = Keybag::create("pw").unwrap();
        let shown = format!("{keybag:?} {key:?}");
        assert!(!shown.contains(&keybag.ciphertext) && !shown.contains(&keybag.salt));
        assert!(shown.ends_with("KeybagKey(…)"));
    }
}
