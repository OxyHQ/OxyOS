//! A member device's keys (Oxy ADR 0063 D2): Ed25519 to sign, X25519 to be sealed to. Made on
//! the machine, kept in its store sealed, never sent anywhere.

use crate::wire::getrandom;
use crate::Result;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use std::fmt;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

/// An Ed25519 and an X25519 secret key, boxed so that moving them leaves no copy.
pub struct KeyPairs {
    signing: Box<SigningKey>,
    boxing: Box<StaticSecret>,
}

impl KeyPairs {
    /// New keys from the system's CSPRNG.
    pub fn generate() -> Result<Self> {
        let mut signing = Zeroizing::new([0u8; 32]);
        let mut boxing = Zeroizing::new([0u8; 32]);
        getrandom(&mut *signing)?;
        getrandom(&mut *boxing)?;
        Ok(Self::from_secrets(&signing, &boxing))
    }

    /// The keys of an Ed25519 seed and an X25519 secret (as the store keeps them).
    pub fn from_secrets(signing: &[u8; 32], boxing: &[u8; 32]) -> Self {
        Self {
            signing: Box::new(SigningKey::from_bytes(signing)),
            boxing: Box::new(StaticSecret::from(*boxing)),
        }
    }

    /// The Ed25519 public key.
    pub fn signing_key(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }

    /// The X25519 public key.
    pub fn box_key(&self) -> [u8; 32] {
        PublicKey::from(&*self.boxing).to_bytes()
    }

    pub(crate) fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing.sign(message).to_bytes()
    }

    pub(crate) fn box_secret(&self) -> &StaticSecret {
        &self.boxing
    }

    /// The Ed25519 seed and the X25519 secret, for the store alone.
    pub(crate) fn secrets(&self) -> (Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>) {
        (
            Zeroizing::new(self.signing.to_bytes()),
            Zeroizing::new(self.boxing.to_bytes()),
        )
    }
}

impl fmt::Debug for KeyPairs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyPairs").finish_non_exhaustive()
    }
}

/// Verifies an Ed25519 signature strictly (RFC 8032, no small-order keys, canonical `S`).
pub(crate) fn verify(public: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public) else {
        return false;
    };
    if key.is_weak() {
        return false;
    }
    key.verify_strict(message, &Signature::from_bytes(signature))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32(text: &str) -> [u8; 32] {
        hex::decode(text).unwrap().try_into().unwrap()
    }

    #[test]
    fn ed25519_rfc8032_vectors() {
        // RFC 8032 § 7.1, tests 1 and 2.
        let one = KeyPairs::from_secrets(
            &hex32("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"),
            &[1; 32],
        );
        assert_eq!(
            hex::encode(one.signing_key()),
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        let signature = one.sign(b"");
        assert_eq!(
            hex::encode(signature),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
             5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
        assert!(verify(&one.signing_key(), b"", &signature));
        let two = KeyPairs::from_secrets(
            &hex32("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb"),
            &[1; 32],
        );
        assert_eq!(
            hex::encode(two.signing_key()),
            "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c"
        );
        let signature = two.sign(&[0x72]);
        assert_eq!(
            hex::encode(signature),
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da\
             085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00"
        );
        assert!(verify(&two.signing_key(), &[0x72], &signature));
        assert!(!verify(&two.signing_key(), &[0x73], &signature));
        assert!(!verify(&one.signing_key(), &[0x72], &signature));
    }

    #[test]
    fn refuses_weak_keys_and_malleable_signatures() {
        // The identity point as a public key verifies anything under a lax check.
        let mut identity = [0u8; 32];
        identity[0] = 1;
        let mut signature = [0u8; 64];
        signature[0] = 1;
        assert!(!verify(&identity, b"anything", &signature));
        // S + ℓ: the same signature spelled again, refused.
        let keys = KeyPairs::generate().unwrap();
        let mut signature = keys.sign(b"m");
        let order = hex32("edd3f55c1a631258d69cf7a2def9de1400000000000000000000000000000010");
        let mut carry = 0u16;
        for i in 0..32 {
            let sum = signature[32 + i] as u16 + order[i] as u16 + carry;
            signature[32 + i] = sum as u8;
            carry = sum >> 8;
        }
        assert!(!verify(&keys.signing_key(), b"m", &signature));
    }

    #[test]
    fn debug_shows_no_key() {
        let keys = KeyPairs::from_secrets(&[0xab; 32], &[0xcd; 32]);
        let shown = format!("{keys:?}");
        assert_eq!(shown, "KeyPairs { .. }");
    }
}
