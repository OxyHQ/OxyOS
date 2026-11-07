//! The recovery key (Oxy ADR 0063 D2, D6, D8): 20 random bytes, shown once as 32 Crockford
//! base32 characters in eight groups of four, and the Ed25519 and X25519 keys derived from it:
//!
//! ```text
//! seed_s = HKDF-SHA256(ikm = recoveryKey, salt = "oxy.keychain.recovery.v1", info = "ed25519", 32)
//! seed_b = HKDF-SHA256(ikm = recoveryKey, salt = "oxy.keychain.recovery.v1", info = "x25519", 32)
//! ```
//!
//! Reading it ignores case, hyphens and spaces (tabs and line breaks too, as the SDK does), and
//! reads `I` and `L` as 1 and `O` as 0.

use crate::keys::KeyPairs;
use crate::wire::{box_key, getrandom, signing_key};
use crate::{Error, Result};
use data_encoding::{Encoding, Specification};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fmt;
use std::sync::OnceLock;
use zeroize::Zeroizing;

pub const LENGTH: usize = 20;
const TEXT_LENGTH: usize = 32;
const SALT: &[u8] = b"oxy.keychain.recovery.v1";

/// The recovery key's public halves, as the roster names them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryPublic {
    #[serde(with = "box_key")]
    pub box_key: [u8; 32],
    #[serde(with = "signing_key")]
    pub signing_key: [u8; 32],
}

pub struct RecoveryKey(Box<Zeroizing<[u8; LENGTH]>>);

/// Crockford's base32: its 32 symbols, lowercase read as uppercase, `I`/`L` as 1, `O` as 0,
/// hyphens and white space passed over.
fn crockford() -> &'static Encoding {
    static ENCODING: OnceLock<Encoding> = OnceLock::new();
    ENCODING.get_or_init(|| {
        let mut spec = Specification::new();
        spec.symbols.push_str("0123456789ABCDEFGHJKMNPQRSTVWXYZ");
        spec.translate.from.push_str("abcdefghjkmnpqrstvwxyzIiLlOo");
        spec.translate.to.push_str("ABCDEFGHJKMNPQRSTVWXYZ111100");
        spec.ignore.push_str("- \t\n\r");
        spec.encoding()
            .expect("Crockford's base32 is a valid specification")
    })
}

impl RecoveryKey {
    /// A new recovery key from the system's CSPRNG.
    pub fn generate() -> Result<Self> {
        let mut bytes = Box::new(Zeroizing::new([0u8; LENGTH]));
        getrandom(&mut **bytes)?;
        Ok(Self(bytes))
    }

    pub fn from_bytes(bytes: &[u8; LENGTH]) -> Self {
        Self(Box::new(Zeroizing::new(*bytes)))
    }

    /// Reads the recovery key as the person typed it.
    pub fn parse(text: &str) -> Result<Self> {
        let malformed = || Error::Malformed("recovery key");
        let symbols = text
            .bytes()
            .filter(|byte| !matches!(byte, b'-' | b' ' | b'\t' | b'\n' | b'\r'))
            .count();
        if symbols != TEXT_LENGTH {
            return Err(malformed());
        }
        let encoding = crockford();
        // With characters to pass over, `decode_len` is a bound; what was decoded is counted.
        let length = encoding.decode_len(text.len()).map_err(|_| malformed())?;
        let mut buffer = Zeroizing::new(vec![0u8; length]);
        let decoded = encoding
            .decode_mut(text.as_bytes(), &mut buffer)
            .map_err(|_| malformed())?;
        if decoded != LENGTH {
            return Err(malformed());
        }
        let mut bytes = Box::new(Zeroizing::new([0u8; LENGTH]));
        bytes.copy_from_slice(&buffer[..LENGTH]);
        Ok(Self(bytes))
    }

    /// The text shown to the person: eight groups of four, joined by hyphens.
    pub fn to_text(&self) -> Zeroizing<String> {
        let mut symbols = Zeroizing::new([0u8; TEXT_LENGTH]);
        crockford().encode_mut(&**self.0, &mut *symbols);
        let mut text = Zeroizing::new(String::with_capacity(TEXT_LENGTH + 7));
        for (i, group) in symbols.chunks(4).enumerate() {
            if i > 0 {
                text.push('-');
            }
            text.extend(group.iter().map(|byte| *byte as char));
        }
        text
    }

    /// The recovery signing and box keys.
    pub fn keys(&self) -> Result<KeyPairs> {
        let hkdf = Hkdf::<Sha256>::new(Some(SALT), &**self.0);
        let mut signing = Zeroizing::new([0u8; 32]);
        let mut boxing = Zeroizing::new([0u8; 32]);
        hkdf.expand(b"ed25519", &mut *signing)
            .and_then(|()| hkdf.expand(b"x25519", &mut *boxing))
            .map_err(|_| Error::Malformed("key length"))?;
        Ok(KeyPairs::from_secrets(&signing, &boxing))
    }

    /// The public halves the roster holds.
    pub fn public(&self) -> Result<RecoveryPublic> {
        let keys = self.keys()?;
        Ok(RecoveryPublic {
            box_key: keys.box_key(),
            signing_key: keys.signing_key(),
        })
    }

    /// Whether the person typed back the key they were shown (D6: before the vault exists).
    pub fn matches(&self, other: &RecoveryKey) -> bool {
        use subtle::ConstantTimeEq;
        bool::from(self.0.ct_eq(&**other.0))
    }
}

impl fmt::Debug for RecoveryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoveryKey(…)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips() {
        let key = RecoveryKey::from_bytes(&[0u8; LENGTH]);
        assert_eq!(&*key.to_text(), "0000-0000-0000-0000-0000-0000-0000-0000");
        let key = RecoveryKey::from_bytes(&[0xff; LENGTH]);
        assert_eq!(&*key.to_text(), "ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ");
        let key = RecoveryKey::generate().unwrap();
        let text = key.to_text();
        assert_eq!(text.len(), 39);
        assert!(RecoveryKey::parse(&text).unwrap().matches(&key));
    }

    #[test]
    fn reading_forgives_what_the_adr_says() {
        let bytes: [u8; LENGTH] = core::array::from_fn(|i| (i * 13 + 7) as u8);
        let key = RecoveryKey::from_bytes(&bytes);
        let text = key.to_text().replace('-', "");
        let lower = text.to_lowercase();
        let spaced: String = text
            .chars()
            .enumerate()
            .flat_map(|(i, c)| (i % 4 == 0 && i > 0).then_some(' ').into_iter().chain([c]))
            .collect();
        let confused = text.replace('1', "l").replace('0', "O");
        for typed in [&text, &lower, &spaced, &confused, &format!("{lower}-")] {
            assert!(RecoveryKey::parse(typed).unwrap().matches(&key), "{typed}");
        }
        let with_i = text.replace('1', "I");
        assert!(RecoveryKey::parse(&with_i).unwrap().matches(&key));
    }

    #[test]
    fn reading_refuses_the_rest() {
        let text = RecoveryKey::from_bytes(&[0x42; LENGTH]).to_text();
        assert!(RecoveryKey::parse(&text[..text.len() - 1]).is_err());
        assert!(RecoveryKey::parse(&format!("{}0", &*text)).is_err());
        assert!(RecoveryKey::parse(&text.replacen('2', "U", 1)).is_err());
        assert!(RecoveryKey::parse(&text.replacen('-', "_", 1)).is_err());
        assert!(RecoveryKey::parse("").is_err());
    }

    #[test]
    fn derives_two_different_keys() {
        let key = RecoveryKey::from_bytes(&[1; LENGTH]);
        let public = key.public().unwrap();
        assert_ne!(public.signing_key, public.box_key);
        assert_eq!(
            public,
            RecoveryKey::from_bytes(&[1; LENGTH]).public().unwrap()
        );
        assert_ne!(
            public,
            RecoveryKey::from_bytes(&[2; LENGTH]).public().unwrap()
        );
    }

    #[test]
    fn debug_shows_no_key() {
        let key = RecoveryKey::from_bytes(&[0xff; LENGTH]);
        assert_eq!(format!("{key:?}"), "RecoveryKey(…)");
    }
}
