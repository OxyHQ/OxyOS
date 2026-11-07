//! The wire's encodings (Oxy ADR 0063 D8): binary as base64url without padding, decoded
//! strictly; ids as lowercase uuids; JCS (RFC 8785) of what is signed.

use crate::{Error, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use zeroize::Zeroizing;

/// The largest integer JavaScript holds exactly, the most a counter or `seq` may be.
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// Base64url without padding, as the wire writes binary.
pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Base64url without padding of exactly `N` bytes; anything else (padding, another alphabet,
/// stray bits, another length) is refused.
pub(crate) fn unb64_fixed<const N: usize>(text: &str, what: &'static str) -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    if text.len() != N.div_ceil(3) * 4 - (3 - N % 3) % 3 {
        return Err(Error::Malformed(what));
    }
    match URL_SAFE_NO_PAD.decode_slice(text, &mut bytes) {
        Ok(n) if n == N => Ok(bytes),
        _ => Err(Error::Malformed(what)),
    }
}

/// Base64url without padding of a secret of exactly `N` bytes, decoded into a buffer that is
/// overwritten when dropped.
pub(crate) fn unb64_secret<const N: usize>(
    text: &str,
    what: &'static str,
) -> Result<Zeroizing<[u8; N]>> {
    let mut bytes = Zeroizing::new([0u8; N]);
    if text.len() != N.div_ceil(3) * 4 - (3 - N % 3) % 3 {
        return Err(Error::Malformed(what));
    }
    match URL_SAFE_NO_PAD.decode_slice(text, &mut *bytes) {
        Ok(n) if n == N => Ok(bytes),
        _ => Err(Error::Malformed(what)),
    }
}

/// Base64url without padding of a secret of exactly `N` bytes, written into a buffer that is
/// overwritten when dropped.
pub(crate) fn b64_secret(bytes: &[u8]) -> Zeroizing<String> {
    let mut text = Zeroizing::new(String::with_capacity(bytes.len().div_ceil(3) * 4));
    URL_SAFE_NO_PAD.encode_string(bytes, &mut text);
    text
}

/// Base64url without padding of any length.
pub(crate) fn unb64(text: &str, what: &'static str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| Error::Malformed(what))
}

/// JCS (RFC 8785) of public data: what is signed or hashed.
pub(crate) fn jcs(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json_canonicalizer::to_vec(value).map_err(|_| Error::Malformed("canonical JSON"))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

fn is_uuid(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(b),
        })
}

macro_rules! uuid_id {
    ($name:ident, $what:literal) => {
        #[doc = concat!("A ", $what, ": a lowercase uuid.")]
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(text: &str) -> Result<Self> {
                if is_uuid(text) {
                    Ok(Self(text.to_owned()))
                } else {
                    Err(Error::Malformed($what))
                }
            }

            /// A new random (version 4) id.
            pub fn random() -> Result<Self> {
                let mut bytes = [0u8; 16];
                getrandom(&mut bytes)?;
                bytes[6] = (bytes[6] & 0x0f) | 0x40;
                bytes[8] = (bytes[8] & 0x3f) | 0x80;
                let hex = hex(&bytes);
                Ok(Self(format!(
                    "{}-{}-{}-{}-{}",
                    &hex[0..8],
                    &hex[8..12],
                    &hex[12..16],
                    &hex[16..20],
                    &hex[20..32]
                )))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                Self::parse(&text).map_err(serde::de::Error::custom)
            }
        }
    };
}

uuid_id!(VaultId, "vault id");
uuid_id!(MemberId, "member id");
uuid_id!(ItemId, "item id");
uuid_id!(EnrollmentId, "enrollment id");

/// Fills `bytes` from the system's CSPRNG.
pub(crate) fn getrandom(bytes: &mut [u8]) -> Result<()> {
    use rand_core::RngCore;
    rand_core::OsRng
        .try_fill_bytes(bytes)
        .map_err(|_| Error::Random)
}

/// An Ed25519 public key the keychain accepts: a point in its one canonical encoding, not of
/// small order (as the SDK's `decodeSigningKey`).
pub(crate) fn is_signing_key(bytes: &[u8; 32]) -> bool {
    match ed25519_dalek::VerifyingKey::from_bytes(bytes) {
        Ok(key) => !key.is_weak() && key.to_edwards().compress().to_bytes() == *bytes,
        Err(_) => false,
    }
}

/// An X25519 public key the keychain accepts: not of low order, which X25519 of any secret
/// shows as an all-zero result (as the SDK's `decodeBoxKey`).
pub(crate) fn is_box_key(bytes: &[u8; 32]) -> bool {
    let probe = x25519_dalek::StaticSecret::from([0x55; 32]);
    probe
        .diffie_hellman(&x25519_dalek::PublicKey::from(*bytes))
        .was_contributory()
}

/// A timestamp as the keychain writes them: ISO 8601 in UTC with milliseconds
/// (`2026-11-07T12:00:00.000Z`), of a day that exists (JavaScript's `toISOString`).
pub fn is_timestamp(text: &str) -> bool {
    let b = text.as_bytes();
    let shape = b.len() == 24
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            19 => *c == b'.',
            23 => *c == b'Z',
            _ => c.is_ascii_digit(),
        });
    if !shape {
        return false;
    }
    let number = |from: usize, to: usize| text[from..to].parse::<u32>().unwrap_or(u32::MAX);
    let (year, month, day) = (number(0, 4), number(5, 7), number(8, 10));
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day) && number(11, 13) < 24 && number(14, 16) < 60 && number(17, 19) < 60
}

/// `at` as the keychain writes timestamps (JavaScript's `toISOString`): UTC, milliseconds.
pub fn timestamp(at: std::time::SystemTime) -> String {
    let since = at.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let seconds = since.as_secs();
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Days to the civil date (Howard Hinnant's `civil_from_days`).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60,
        since.subsec_millis()
    )
}

/// Now, as [`timestamp`] writes it.
pub fn now() -> String {
    timestamp(std::time::SystemTime::now())
}

/// A 32-byte key on the wire, of any value (a public key checked where it is used, an
/// ephemeral key).
pub(crate) mod key32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::b64(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 32], D::Error> {
        let text = String::deserialize(deserializer)?;
        super::unb64_fixed::<32>(&text, "key").map_err(serde::de::Error::custom)
    }
}

/// An Ed25519 public key on the wire, refused unless [`is_signing_key`].
pub(crate) mod signing_key {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error> {
        super::key32::serialize(bytes, serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 32], D::Error> {
        let bytes = super::key32::deserialize(deserializer)?;
        if super::is_signing_key(&bytes) {
            Ok(bytes)
        } else {
            Err(serde::de::Error::custom("not a usable Ed25519 public key"))
        }
    }
}

/// An X25519 public key on the wire, refused unless [`is_box_key`].
pub(crate) mod box_key {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error> {
        super::key32::serialize(bytes, serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 32], D::Error> {
        let bytes = super::key32::deserialize(deserializer)?;
        if super::is_box_key(&bytes) {
            Ok(bytes)
        } else {
            Err(serde::de::Error::custom("not a usable X25519 public key"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_is_strict() {
        let key = [7u8; 32];
        let text = b64(&key);
        assert_eq!(text.len(), 43);
        assert_eq!(unb64_fixed::<32>(&text, "k").unwrap(), key);
        assert!(unb64_fixed::<32>(&format!("{text}="), "k").is_err());
        assert!(unb64_fixed::<32>(&text[..42], "k").is_err());
        let ones = b64(&[0xff; 32]);
        assert!(ones.contains('_'));
        assert!(unb64_fixed::<32>(&ones, "k").is_ok());
        assert!(unb64_fixed::<32>(&ones.replace('_', "/"), "k").is_err());
        // Stray bits in the last character: a second spelling of the same bytes.
        let mut stray = text.clone().into_bytes();
        let last = stray.len() - 1;
        stray[last] = if stray[last] == b'c' { b'd' } else { b'c' };
        assert!(unb64_fixed::<32>(std::str::from_utf8(&stray).unwrap(), "k").is_err());
        assert_eq!(unb64_fixed::<24>(&b64(&[1u8; 24]), "n").unwrap(), [1u8; 24]);
        assert_eq!(unb64_fixed::<64>(&b64(&[2u8; 64]), "s").unwrap(), [2u8; 64]);
        assert_eq!(unb64_fixed::<16>(&b64(&[3u8; 16]), "c").unwrap(), [3u8; 16]);
    }

    #[test]
    fn timestamps_are_javascripts_iso_strings() {
        for good in [
            "2026-11-07T12:00:00.000Z",
            "2024-02-29T23:59:59.999Z",
            "2000-02-29T00:00:00.000Z",
        ] {
            assert!(is_timestamp(good), "{good}");
        }
        for bad in [
            "2026-11-07T12:00:00Z",
            "2026-11-07T12:00:00.000+00:00",
            "2026-11-07 12:00:00.000Z",
            "2026-13-07T12:00:00.000Z",
            "2026-02-29T12:00:00.000Z",
            "1900-02-29T12:00:00.000Z",
            "2026-04-31T12:00:00.000Z",
            "2026-11-07T24:00:00.000Z",
            "2026-11-07T12:60:00.000Z",
            "2026-11-07T12:00:60.000Z",
            "2026-11-00T12:00:00.000Z",
            "２026-11-07T12:00:00.000Z",
        ] {
            assert!(!is_timestamp(bad), "{bad}");
        }
    }

    #[test]
    fn public_keys_of_low_order_are_refused() {
        let mut identity = [0u8; 32];
        identity[0] = 1;
        assert!(!is_signing_key(&identity));
        assert!(!is_box_key(&[0; 32]));
        assert!(!is_box_key(&identity));
        let keys = crate::keys::KeyPairs::generate().unwrap();
        assert!(is_signing_key(&keys.signing_key()));
        assert!(is_box_key(&keys.box_key()));
        // y + p for a small y is a second encoding of the point whose y it is: refused, though
        // the point itself is accepted in its canonical encoding.
        let mut found = 0;
        for y in 2u8..=18 {
            let mut canonical = [0u8; 32];
            canonical[0] = y;
            if !is_signing_key(&canonical) {
                continue;
            }
            let mut second = [0xffu8; 32];
            second[0] = 0xed + y;
            second[31] = 0x7f;
            assert!(ed25519_dalek::VerifyingKey::from_bytes(&second).is_ok());
            assert!(!is_signing_key(&second), "y = {y}");
            found += 1;
        }
        assert!(found > 0);
    }

    #[test]
    fn timestamps_are_written_as_javascript_writes_them() {
        use std::time::{Duration, UNIX_EPOCH};
        for (seconds, millis, text) in [
            (0, 0, "1970-01-01T00:00:00.000Z"),
            (1_709_164_799, 999, "2024-02-28T23:59:59.999Z"),
            (1_709_251_200, 5, "2024-03-01T00:00:00.005Z"),
            (1_793_966_400, 120, "2026-11-06T12:00:00.120Z"),
            (951_782_400, 0, "2000-02-29T00:00:00.000Z"),
        ] {
            let at = UNIX_EPOCH + Duration::from_secs(seconds) + Duration::from_millis(millis);
            assert_eq!(timestamp(at), text);
            assert!(is_timestamp(text));
        }
        assert!(is_timestamp(&now()));
    }

    #[test]
    fn ids_are_lowercase_uuids() {
        assert!(VaultId::parse("0f8fad5b-d9cb-469f-a165-70867728950e").is_ok());
        assert!(VaultId::parse("0F8FAD5B-D9CB-469F-A165-70867728950E").is_err());
        assert!(VaultId::parse("0f8fad5bd9cb469fa16570867728950e").is_err());
        assert!(VaultId::parse("recovery").is_err());
        let id = MemberId::random().unwrap();
        assert!(MemberId::parse(id.as_str()).is_ok());
        assert_eq!(&id.as_str()[14..15], "4");
    }
}
