//! Sealing to an X25519 public key (Oxy ADR 0063 D8), the construction of age's X25519
//! recipient:
//!
//! ```text
//! (e, E) ephemeral;  shared = X25519(e, R), refused if all zero
//! key = HKDF-SHA256(ikm = shared, salt = E ‖ R, info = "oxy.keychain.seal.v1", 32)
//! ciphertext = XChaCha20-Poly1305(key, random 24-byte nonce, plaintext, aad)
//! ```

use crate::wire::getrandom;
use crate::{Error, Result};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;
use std::fmt;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

const INFO: &[u8] = b"oxy.keychain.seal.v1";

pub const NONCE_LENGTH: usize = 24;
pub const KEY_LENGTH: usize = 32;

/// A sealed message: `{ epk, nonce, ciphertext }`.
#[derive(Clone, PartialEq, Eq)]
pub struct Sealed {
    pub epk: [u8; 32],
    pub nonce: [u8; NONCE_LENGTH],
    /// The ciphertext with its 16-byte Poly1305 tag.
    pub ciphertext: Vec<u8>,
}

impl fmt::Debug for Sealed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sealed")
            .field("ciphertext_len", &self.ciphertext.len())
            .finish_non_exhaustive()
    }
}

/// Seals `plaintext` to `recipient`, bound to `aad`.
pub fn seal(recipient: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Result<Sealed> {
    let mut ephemeral = Zeroizing::new([0u8; 32]);
    getrandom(&mut *ephemeral)?;
    let mut nonce = [0u8; NONCE_LENGTH];
    getrandom(&mut nonce)?;
    seal_with(&ephemeral, &nonce, recipient, plaintext, aad)
}

/// [`seal`] with the ephemeral secret and the nonce given: for the interoperability vectors only.
pub(crate) fn seal_with(
    ephemeral: &[u8; 32],
    nonce: &[u8; NONCE_LENGTH],
    recipient: &[u8; 32],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Sealed> {
    let ephemeral = Box::new(StaticSecret::from(*ephemeral));
    let epk = PublicKey::from(&*ephemeral).to_bytes();
    let key = derive(&ephemeral, &PublicKey::from(*recipient), &epk, recipient)?;
    let ciphertext = encrypt(&key, nonce, plaintext, aad)?;
    Ok(Sealed {
        epk,
        nonce: *nonce,
        ciphertext,
    })
}

/// Opens what was sealed to `secret`'s public key with `aad`.
pub fn open(secret: &StaticSecret, sealed: &Sealed, aad: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let recipient = PublicKey::from(secret).to_bytes();
    let key = derive(
        secret,
        &PublicKey::from(sealed.epk),
        &sealed.epk,
        &recipient,
    )?;
    decrypt(&key, &sealed.nonce, &sealed.ciphertext, aad)
}

/// HKDF-SHA256 of the shared secret, salted with `E ‖ R`.
fn derive(
    own: &StaticSecret,
    other: &PublicKey,
    epk: &[u8; 32],
    recipient: &[u8; 32],
) -> Result<Zeroizing<[u8; KEY_LENGTH]>> {
    let shared = own.diffie_hellman(other);
    // An all-zero result: the other key is of low order, and the "secret" is public.
    if !shared.was_contributory() {
        return Err(Error::Malformed("public key (low order)"));
    }
    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(epk);
    salt[32..].copy_from_slice(recipient);
    let mut key = Zeroizing::new([0u8; KEY_LENGTH]);
    Hkdf::<Sha256>::new(Some(&salt), shared.as_bytes())
        .expand(INFO, &mut *key)
        .map_err(|_| Error::Malformed("key length"))?;
    Ok(key)
}

/// XChaCha20-Poly1305 of `plaintext` under `key`, bound to `aad`.
pub(crate) fn encrypt(
    key: &[u8; KEY_LENGTH],
    nonce: &[u8; NONCE_LENGTH],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>> {
    XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| Error::Malformed("plaintext too long"))
}

/// Opens XChaCha20-Poly1305 of `ciphertext` under `key`, bound to `aad`.
pub(crate) fn decrypt(
    key: &[u8; KEY_LENGTH],
    nonce: &[u8; NONCE_LENGTH],
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| Error::Decrypt)
}

/// A fresh 24-byte nonce.
pub(crate) fn nonce() -> Result<[u8; NONCE_LENGTH]> {
    let mut nonce = [0u8; NONCE_LENGTH];
    getrandom(&mut nonce)?;
    Ok(nonce)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(seed: u8) -> (StaticSecret, [u8; 32]) {
        let secret = StaticSecret::from([seed; 32]);
        let public = PublicKey::from(&secret).to_bytes();
        (secret, public)
    }

    #[test]
    fn seals_and_opens() {
        let (secret, public) = pair(9);
        let sealed = seal(&public, b"the keyring", b"aad").unwrap();
        assert_eq!(sealed.ciphertext.len(), 11 + 16);
        assert_eq!(&*open(&secret, &sealed, b"aad").unwrap(), b"the keyring");
    }

    #[test]
    fn refuses_every_change() {
        let (secret, public) = pair(9);
        let sealed = seal(&public, b"the keyring", b"aad").unwrap();
        assert!(matches!(
            open(&secret, &sealed, b"aaD"),
            Err(Error::Decrypt)
        ));
        let (other, _) = pair(10);
        assert!(matches!(open(&other, &sealed, b"aad"), Err(Error::Decrypt)));
        for flip in 0..3 {
            let mut changed = sealed.clone();
            match flip {
                0 => changed.epk[0] ^= 1,
                1 => changed.nonce[0] ^= 1,
                _ => changed.ciphertext[0] ^= 1,
            }
            assert!(open(&secret, &changed, b"aad").is_err());
        }
        let mut short = sealed.clone();
        short.ciphertext.truncate(15);
        assert!(open(&secret, &short, b"aad").is_err());
    }

    #[test]
    fn refuses_a_low_order_key() {
        // u = 0 and u = 1, points of low order: X25519 with them is all zero (RFC 7748 § 6.1).
        for bad in [[0u8; 32], {
            let mut one = [0u8; 32];
            one[0] = 1;
            one
        }] {
            assert!(matches!(seal(&bad, b"x", b""), Err(Error::Malformed(_))));
        }
        let (secret, _) = pair(1);
        let sealed = Sealed {
            epk: [0u8; 32],
            nonce: [0; 24],
            ciphertext: vec![0; 32],
        };
        assert!(matches!(
            open(&secret, &sealed, b""),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn x25519_rfc7748_vectors() {
        // RFC 7748 § 6.1.
        let alice: [u8; 32] =
            hex::decode("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a")
                .unwrap()
                .try_into()
                .unwrap();
        let bob: [u8; 32] =
            hex::decode("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb")
                .unwrap()
                .try_into()
                .unwrap();
        let alice = StaticSecret::from(alice);
        let bob = StaticSecret::from(bob);
        assert_eq!(
            hex::encode(PublicKey::from(&alice).as_bytes()),
            "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
        );
        assert_eq!(
            hex::encode(PublicKey::from(&bob).as_bytes()),
            "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"
        );
        let shared = alice.diffie_hellman(&PublicKey::from(&bob));
        assert_eq!(
            hex::encode(shared.as_bytes()),
            "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742"
        );
    }

    #[test]
    fn debug_shows_no_ciphertext() {
        let (_, public) = pair(3);
        let sealed = seal(&public, b"secret", b"").unwrap();
        let shown = format!("{sealed:?}");
        assert!(!shown.contains(&format!("{:?}", sealed.ciphertext[0..4].to_vec())));
        assert!(shown.contains("ciphertext_len"));
    }
}
