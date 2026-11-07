//! The keyring (Oxy ADR 0063 D2, D8): every generation of the vault key, sealed whole to each
//! member and to the recovery key. Its plaintext is
//! `JCS({ "generations": { "<g>": "<VK_g base64url>", … } })`.

use crate::secret;
use crate::wire::{b64_secret, getrandom, unb64_secret};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use zeroize::{Zeroize, Zeroizing};

/// One generation of the vault key: 32 bytes, boxed, overwritten when dropped.
pub struct VaultKey(Box<[u8; 32]>);

impl VaultKey {
    fn generate() -> Result<Self> {
        let mut key = Box::new([0u8; 32]);
        getrandom(&mut *key)?;
        Ok(Self(key))
    }

    pub(crate) fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Drop for VaultKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Every generation of the vault key, from 1 to the newest, none missing.
pub struct Keyring {
    generations: BTreeMap<u32, VaultKey>,
}

/// The keyring as written: `generations` keyed by the generation in decimal. A `BTreeMap` of
/// ASCII keys is in RFC 8785's order.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plain {
    generations: BTreeMap<String, crate::SecretString>,
}

impl Keyring {
    /// A new keyring: generation 1, random.
    pub fn generate() -> Result<Self> {
        let mut generations = BTreeMap::new();
        generations.insert(1, VaultKey::generate()?);
        Ok(Self { generations })
    }

    /// The newest generation and its key.
    pub fn newest(&self) -> (u32, &VaultKey) {
        let (generation, key) = self
            .generations
            .last_key_value()
            .expect("a keyring always holds generation 1");
        (*generation, key)
    }

    pub fn generation(&self) -> u32 {
        self.newest().0
    }

    /// The key of `generation`, if the keyring has it.
    pub fn get(&self, generation: u32) -> Option<&VaultKey> {
        self.generations.get(&generation)
    }

    /// Adds the next generation, random (a rotation, D4), and says which it is.
    pub fn rotate(&mut self) -> Result<u32> {
        let next = self
            .generation()
            .checked_add(1)
            .ok_or(Error::Malformed("generation"))?;
        self.generations.insert(next, VaultKey::generate()?);
        Ok(next)
    }

    /// The keyring's plaintext, its JCS form, in a buffer overwritten when dropped.
    pub fn to_plaintext(&self) -> Result<Zeroizing<Vec<u8>>> {
        let plain = Plain {
            generations: self
                .generations
                .iter()
                .map(|(generation, key)| {
                    (
                        generation.to_string(),
                        crate::SecretString::from(b64_secret(key.bytes())),
                    )
                })
                .collect(),
        };
        secret::json(&plain)
    }

    /// Reads a keyring's plaintext. Only its canonical form is accepted, with generations 1 to
    /// the newest, each a 32-byte key.
    pub fn from_plaintext(plaintext: &[u8]) -> Result<Self> {
        let malformed = || Error::Malformed("keyring");
        let plain: Plain = serde_json::from_slice(plaintext).map_err(|_| malformed())?;
        let mut generations = BTreeMap::new();
        for (text, key) in &plain.generations {
            // `[1-9][0-9]{0,8}`, as the SDK reads them.
            let generation: u32 = text.parse().map_err(|_| malformed())?;
            if generation == 0 || text.len() > 9 || generation.to_string() != *text {
                return Err(malformed());
            }
            let bytes = unb64_secret::<32>(key.expose(), "keyring")?;
            generations.insert(generation, VaultKey(Box::new(*bytes)));
        }
        let contiguous = generations.keys().copied().eq(1..=generations.len() as u32);
        if generations.is_empty() || !contiguous {
            return Err(malformed());
        }
        let keyring = Self { generations };
        // Not a second spelling of the same keyring (a duplicate key, spaces, another order).
        if *keyring.to_plaintext()? != *plaintext {
            return Err(malformed());
        }
        Ok(keyring)
    }

    #[cfg(test)]
    pub(crate) fn from_keys(keys: &[[u8; 32]]) -> Self {
        Self {
            generations: keys
                .iter()
                .enumerate()
                .map(|(i, key)| (i as u32 + 1, VaultKey(Box::new(*key))))
                .collect(),
        }
    }
}

impl fmt::Debug for Keyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keyring")
            .field("generation", &self.generation())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VaultKey(…)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_is_jcs_and_round_trips() {
        let keys: Vec<[u8; 32]> = (1..=11u8).map(|g| [g; 32]).collect();
        let keyring = Keyring::from_keys(&keys);
        let plaintext = keyring.to_plaintext().unwrap();
        let value: serde_json::Value = serde_json::from_slice(&plaintext).unwrap();
        assert_eq!(
            *plaintext,
            serde_json_canonicalizer::to_vec(&value).unwrap(),
            "the keyring is written in its JCS form"
        );
        assert!(std::str::from_utf8(&plaintext)
            .unwrap()
            .starts_with("{\"generations\":{\"1\":\"AQEB"));
        let read = Keyring::from_plaintext(&plaintext).unwrap();
        assert_eq!(read.generation(), 11);
        assert_eq!(read.get(10).unwrap().bytes(), &[10; 32]);
    }

    #[test]
    fn refuses_anything_but_its_canonical_form() {
        let key = crate::wire::b64(&[1; 32]);
        for bad in [
            r#"{"generations":{}}"#.to_string(),
            format!(r#"{{"generations":{{"0":"{key}"}}}}"#),
            format!(r#"{{"generations":{{"01":"{key}"}}}}"#),
            format!(r#"{{"generations":{{"2":"{key}"}}}}"#),
            format!(r#"{{"generations":{{"1":"{key}","3":"{key}"}}}}"#),
            format!(r#"{{"generations":{{"1":"{key}","1":"{key}"}}}}"#),
            format!(r#"{{"generations": {{"1":"{key}"}}}}"#),
            format!(r#"{{"generations":{{"1":"{key}"}},"extra":1}}"#),
            format!(r#"{{"generations":{{"1":"{}"}}}}"#, &key[..42]),
            format!(r#"{{"generations":{{"1":"{key}"}}}} "#),
            format!(r#"{{"generations":{{"-1":"{key}"}}}}"#),
        ] {
            assert!(Keyring::from_plaintext(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(Keyring::from_plaintext(
            format!(r#"{{"generations":{{"1":"{key}"}}}}"#).as_bytes()
        )
        .is_ok());
    }

    #[test]
    fn rotation_keeps_every_generation() {
        let mut keyring = Keyring::generate().unwrap();
        let first = *keyring.get(1).unwrap().bytes();
        assert_eq!(keyring.rotate().unwrap(), 2);
        assert_eq!(keyring.get(1).unwrap().bytes(), &first);
        assert_ne!(keyring.newest().1.bytes(), &first);
    }

    #[test]
    fn debug_shows_no_key() {
        let keyring = Keyring::from_keys(&[[0x5a; 32]]);
        let shown = format!("{keyring:?} {:?}", keyring.get(1));
        assert_eq!(shown, "Keyring { generation: 1, .. } Some(VaultKey(…))");
    }
}
