//! Verification codes (OxyOS ADR 0050 decision 6): RFC 6238's TOTP over RustCrypto's HMAC
//! (SHA-1, SHA-256 or SHA-512), from the `otpauth://totp/…` URI a site gives.
//!
//! The URI is `otpauth://totp/[issuer:]account?secret=BASE32[&issuer=…][&algorithm=SHA1|SHA256|SHA512][&digits=6|7|8][&period=1…3600]`,
//! read as the SDK's `parseOtpauthUri` reads it: the scheme in any case; the label
//! percent-decoded, the query's names and values too with `+` as a space; a parameter given twice,
//! malformed percent-encoding, a fragment, an unknown algorithm or another type (`hotp`) refused;
//! other parameters (`image`, …) passed over. The secret is base32 in either case, spaces and
//! hyphens passed over, trailing `=` dropped, 1 to 128 bytes.

use crate::{Error, Result, SecretString};
use data_encoding::{Encoding, Specification};
use hmac::{Hmac, Mac};
use percent_encoding::percent_decode_str;
use std::fmt;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

pub struct Totp {
    pub(crate) secret: Zeroizing<Vec<u8>>,
    algorithm: Algorithm,
    digits: u32,
    period: u64,
    issuer: SecretString,
    account: SecretString,
}

const SCHEME: &str = "otpauth://totp/";
const MAX_URI: usize = 4096;
const MAX_SECRET_BYTES: usize = 128;

impl fmt::Debug for Totp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Totp")
            .field("algorithm", &self.algorithm)
            .field("digits", &self.digits)
            .field("period", &self.period)
            .finish_non_exhaustive()
    }
}

/// RFC 4648's base32 without padding; the bits after the last whole byte are not checked, as
/// sites' secrets do not always end on a byte.
fn base32() -> &'static Encoding {
    static ENCODING: OnceLock<Encoding> = OnceLock::new();
    ENCODING.get_or_init(|| {
        let mut spec = Specification::new();
        spec.symbols.push_str("ABCDEFGHIJKLMNOPQRSTUVWXYZ234567");
        spec.check_trailing_bits = false;
        spec.encoding().expect("base32 is a valid specification")
    })
}

/// Percent-decoding as JavaScript's `decodeURIComponent`: every `%` begins two hex digits, and
/// what is decoded is UTF-8; with `plus_is_space`, `+` is a space first.
fn decode(text: &str, plus_is_space: bool) -> Result<Zeroizing<String>> {
    let malformed = || Error::Malformed("otpauth URI (percent-encoding)");
    let bytes = text.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && !(bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
        {
            return Err(malformed());
        }
    }
    let spaced;
    let text = if plus_is_space && text.contains('+') {
        spaced = Zeroizing::new(text.replace('+', " "));
        spaced.as_str()
    } else {
        text
    };
    let decoded = Zeroizing::new(percent_decode_str(text).collect::<Vec<u8>>());
    match std::str::from_utf8(&decoded) {
        Ok(text) => Ok(Zeroizing::new(text.to_owned())),
        Err(_) => Err(malformed()),
    }
}

/// A TOTP secret in base32.
fn decode_secret(text: &str) -> Result<Zeroizing<Vec<u8>>> {
    let malformed = || Error::Malformed("TOTP secret");
    let mut symbols = Zeroizing::new(Vec::with_capacity(text.len()));
    for byte in text.bytes() {
        match byte {
            b' ' | b'-' => {}
            b'a'..=b'z' => symbols.push(byte.to_ascii_uppercase()),
            b'A'..=b'Z' | b'2'..=b'7' | b'=' => symbols.push(byte),
            _ => return Err(malformed()),
        }
    }
    while symbols.last() == Some(&b'=') {
        symbols.pop();
    }
    if symbols.contains(&b'=') {
        return Err(malformed());
    }
    // Whole bytes only: the symbols past them hold bits that are dropped, as the SDK drops them.
    let length = symbols.len() * 5 / 8;
    if length == 0 || length > MAX_SECRET_BYTES {
        return Err(malformed());
    }
    let used = (length * 8).div_ceil(5);
    let mut secret = Zeroizing::new(vec![0u8; length]);
    base32()
        .decode_mut(&symbols[..used], &mut secret)
        .map_err(|_| malformed())?;
    Ok(secret)
}

impl Totp {
    pub fn new(secret: &[u8], algorithm: Algorithm, digits: u32, period: u64) -> Result<Self> {
        if secret.is_empty() || !(6..=8).contains(&digits) || !(1..=86_400).contains(&period) {
            return Err(Error::Malformed("verification code's parameters"));
        }
        Ok(Self {
            secret: Zeroizing::new(secret.to_vec()),
            algorithm,
            digits,
            period,
            issuer: SecretString::default(),
            account: SecretString::default(),
        })
    }

    /// Reads an `otpauth://totp/` URI.
    pub fn parse(uri: &str) -> Result<Self> {
        let malformed = || Error::Malformed("otpauth URI");
        if uri.len() > MAX_URI
            || !uri
                .get(..SCHEME.len())
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case(SCHEME))
            || uri.contains('#')
        {
            return Err(malformed());
        }
        let rest = &uri[SCHEME.len()..];
        let (label, query) = rest.split_once('?').unwrap_or((rest, ""));
        let label = decode(label, false)?;
        let (label_issuer, account) = match label.split_once(':') {
            Some((issuer, account)) => (issuer, account),
            None => ("", label.as_str()),
        };
        let account = account.trim_start_matches(' ');
        let mut parameters: Vec<(Zeroizing<String>, Zeroizing<String>)> = Vec::new();
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let key = Zeroizing::new(decode(key, true)?.to_lowercase());
            if parameters.iter().any(|(known, _)| *known == key) {
                return Err(malformed());
            }
            parameters.push((key, decode(value, true)?));
        }
        let get = |name: &str| {
            parameters
                .iter()
                .find(|(key, _)| key.as_str() == name)
                .map(|(_, value)| value.as_str())
        };
        let secret = decode_secret(get("secret").ok_or_else(malformed)?)?;
        let algorithm = match get("algorithm").unwrap_or("SHA1").to_uppercase().as_str() {
            "SHA1" => Algorithm::Sha1,
            "SHA256" => Algorithm::Sha256,
            "SHA512" => Algorithm::Sha512,
            _ => return Err(malformed()),
        };
        let digits = match get("digits").unwrap_or("6") {
            "6" => 6,
            "7" => 7,
            "8" => 8,
            _ => return Err(malformed()),
        };
        let period_text = get("period").unwrap_or("30");
        let period = match period_text.parse::<u64>() {
            Ok(period)
                if (1..=3600).contains(&period)
                    && period_text.len() <= 4
                    && !period_text.starts_with('0')
                    && period_text.bytes().all(|b| b.is_ascii_digit()) =>
            {
                period
            }
            _ => return Err(malformed()),
        };
        let mut totp = Self::new(&secret, algorithm, digits, period)?;
        totp.issuer = SecretString::new(get("issuer").unwrap_or(label_issuer));
        totp.account = SecretString::new(account);
        Ok(totp)
    }

    /// The code at `unix_seconds`.
    pub fn code_at(&self, unix_seconds: u64) -> Zeroizing<String> {
        let counter = (unix_seconds / self.period).to_be_bytes();
        let digest: Zeroizing<Vec<u8>> = Zeroizing::new(match self.algorithm {
            Algorithm::Sha1 => mac!(sha1::Sha1, &self.secret, &counter),
            Algorithm::Sha256 => mac!(sha2::Sha256, &self.secret, &counter),
            Algorithm::Sha512 => mac!(sha2::Sha512, &self.secret, &counter),
        });
        // RFC 4226 § 5.3, dynamic truncation.
        let offset = (digest[digest.len() - 1] & 0x0f) as usize;
        let binary = u32::from_be_bytes([
            digest[offset] & 0x7f,
            digest[offset + 1],
            digest[offset + 2],
            digest[offset + 3],
        ]);
        let code = binary % 10u32.pow(self.digits);
        Zeroizing::new(format!("{code:0width$}", width = self.digits as usize))
    }

    /// The code now.
    pub fn code_now(&self) -> Zeroizing<String> {
        self.code_at(now())
    }

    /// Seconds until the code at `unix_seconds` changes.
    pub fn remaining(&self, unix_seconds: u64) -> u64 {
        self.period - unix_seconds % self.period
    }

    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }

    pub fn digits(&self) -> u32 {
        self.digits
    }

    pub fn period(&self) -> u64 {
        self.period
    }

    /// The `issuer` parameter, else the label's issuer, else empty.
    pub fn issuer(&self) -> &SecretString {
        &self.issuer
    }

    pub fn account(&self) -> &SecretString {
        &self.account
    }
}

/// HMAC of `message` under `key` with the digest `$digest`.
macro_rules! mac {
    ($digest:ty, $key:expr, $message:expr) => {{
        let mut mac =
            Hmac::<$digest>::new_from_slice($key).expect("HMAC takes a key of any length");
        mac.update($message);
        mac.finalize().into_bytes().to_vec()
    }};
}
use mac;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_vectors() {
        // RFC 6238, appendix B: eight digits, a period of 30 seconds.
        let sha1 = Totp::new(b"12345678901234567890", Algorithm::Sha1, 8, 30).unwrap();
        let sha256 = Totp::new(
            b"12345678901234567890123456789012",
            Algorithm::Sha256,
            8,
            30,
        )
        .unwrap();
        let sha512 = Totp::new(
            b"1234567890123456789012345678901234567890123456789012345678901234",
            Algorithm::Sha512,
            8,
            30,
        )
        .unwrap();
        for (time, one, two, five) in [
            (59, "94287082", "46119246", "90693936"),
            (1111111109, "07081804", "68084774", "25091201"),
            (1111111111, "14050471", "67062674", "99943326"),
            (1234567890, "89005924", "91819424", "93441116"),
            (2000000000, "69279037", "90698825", "38618901"),
            (20000000000, "65353130", "77737706", "47863826"),
        ] {
            assert_eq!(*sha1.code_at(time), one, "SHA-1 at {time}");
            assert_eq!(*sha256.code_at(time), two, "SHA-256 at {time}");
            assert_eq!(*sha512.code_at(time), five, "SHA-512 at {time}");
        }
    }

    #[test]
    fn rfc4226_vectors_as_six_digits() {
        // RFC 4226 appendix D: counters 0 to 9, which a period of 1 second makes times.
        let hotp = Totp::new(b"12345678901234567890", Algorithm::Sha1, 6, 1).unwrap();
        let expected = [
            "755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583",
            "399871", "520489",
        ];
        for (counter, code) in expected.iter().enumerate() {
            assert_eq!(*hotp.code_at(counter as u64), *code);
        }
    }

    #[test]
    fn reads_otpauth_uris() {
        // "12345678901234567890" in base32.
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let totp = Totp::parse(&format!(
            "otpauth://totp/Example:alice%40example.com?secret={secret}&issuer=Example&period=30"
        ))
        .unwrap();
        assert_eq!(totp.issuer().expose(), "Example");
        assert_eq!(totp.account().expose(), "alice@example.com");
        assert_eq!(totp.algorithm(), Algorithm::Sha1);
        assert_eq!(totp.digits(), 6);
        assert_eq!(*totp.code_at(59), "287082");
        let totp = Totp::parse(&format!(
            "OTPAUTH://TOTP/ACME%20Co:john?secret={}&algorithm=sha256&digits=8&period=60&image=x",
            secret.to_lowercase()
        ))
        .unwrap();
        assert_eq!(totp.issuer().expose(), "ACME Co");
        assert_eq!(totp.algorithm(), Algorithm::Sha256);
        assert_eq!((totp.digits(), totp.period()), (8, 60));
        assert_eq!(totp.remaining(61), 59);
        // Padding, as some sites write it.
        assert!(Totp::parse("otpauth://totp/a?secret=JBSWY3DPEHPK3PXP====").is_ok());
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        for bad in [
            "otpauth://hotp/a?secret=JBSWY3DPEHPK3PXP&counter=1",
            "otpauth://totp/a?issuer=x",
            "otpauth://totp/a?secret=JBSWY3DPEHPK3PXP&secret=JBSWY3DPEHPK3PXP",
            "otpauth://totp/a?secret=JBSWY3DPEHPK3PX1",
            "otpauth://totp/a?secret=JBSWY3DPEHPK3PXP&algorithm=MD5",
            "otpauth://totp/a?secret=JBSWY3DPEHPK3PXP&digits=10",
            "otpauth://totp/a?secret=JBSWY3DPEHPK3PXP&digits=5",
            "otpauth://totp/a?secret=JBSWY3DPEHPK3PXP&period=0",
            "otpauth://totp/a?secret=",
            "otpauth://totp/a",
            "https://totp/a?secret=JBSWY3DPEHPK3PXP",
        ] {
            assert!(Totp::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn debug_shows_no_secret() {
        let totp = Totp::parse("otpauth://totp/Bank:me?secret=JBSWY3DPEHPK3PXP").unwrap();
        let shown = format!("{totp:?}");
        assert!(
            !shown.contains("Bank") && !shown.contains("me\""),
            "{shown}"
        );
        assert_eq!(shown, "Totp { algorithm: Sha1, digits: 6, period: 30, .. }");
    }
}
