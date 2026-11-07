//! Adding a device by approval (Oxy ADR 0063 D5, D8): the new device commits to its member
//! before it sees the approver's nonce, and both show the same six-digit code, which Oxy, relaying
//! everything, can match only by chance (1 in 10⁶).
//!
//! ```text
//! commitment = SHA-256("oxy.keychain.enroll.commit.v1\n" ‖ JCS({ vaultId, member }) ‖ 0x00 ‖ deviceNonce)
//! h          = SHA-256("oxy.keychain.enroll.code.v1\n" ‖ JCS({ vaultId, member }) ‖ 0x00 ‖ approverNonce ‖ deviceNonce)
//! code       = u32::from_be_bytes(h[0..4]) mod 10⁶, six digits
//! ```

use crate::roster::Member;
use crate::wire::{getrandom, jcs, VaultId};
use crate::Result;
use serde::Serialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

pub const NONCE_LENGTH: usize = 32;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Subject<'a> {
    member: &'a Member,
    vault_id: &'a VaultId,
}

fn subject(vault_id: &VaultId, member: &Member) -> Result<Vec<u8>> {
    jcs(&Subject { member, vault_id })
}

/// A fresh 32-byte nonce (the device's or the approver's).
pub fn nonce() -> Result<Zeroizing<[u8; NONCE_LENGTH]>> {
    let mut nonce = Zeroizing::new([0u8; NONCE_LENGTH]);
    getrandom(&mut *nonce)?;
    Ok(nonce)
}

/// `"oxy.keychain.enroll.commit.v1\n" ‖ JCS({ vaultId, member }) ‖ 0x00 ‖ deviceNonce`.
pub(crate) fn commit_input(
    vault_id: &VaultId,
    member: &Member,
    device_nonce: &[u8; NONCE_LENGTH],
) -> Result<Vec<u8>> {
    Ok([
        b"oxy.keychain.enroll.commit.v1\n".as_slice(),
        &subject(vault_id, member)?,
        &[0],
        device_nonce,
    ]
    .concat())
}

/// `"oxy.keychain.enroll.code.v1\n" ‖ JCS({ vaultId, member }) ‖ 0x00 ‖ approverNonce ‖ deviceNonce`.
pub(crate) fn code_input(
    vault_id: &VaultId,
    member: &Member,
    approver_nonce: &[u8; NONCE_LENGTH],
    device_nonce: &[u8; NONCE_LENGTH],
) -> Result<Vec<u8>> {
    Ok([
        b"oxy.keychain.enroll.code.v1\n".as_slice(),
        &subject(vault_id, member)?,
        &[0],
        approver_nonce,
        device_nonce,
    ]
    .concat())
}

/// The new device's commitment to its member and its nonce.
pub fn commitment(
    vault_id: &VaultId,
    member: &Member,
    device_nonce: &[u8; NONCE_LENGTH],
) -> Result<[u8; 32]> {
    Ok(Sha256::digest(commit_input(vault_id, member, device_nonce)?).into())
}

/// Whether a revealed nonce opens the commitment (the approver checks it too).
pub fn opens(
    commitment_given: &[u8; 32],
    vault_id: &VaultId,
    member: &Member,
    device_nonce: &[u8; NONCE_LENGTH],
) -> Result<bool> {
    let computed = commitment(vault_id, member, device_nonce)?;
    Ok(bool::from(computed.ct_eq(commitment_given)))
}

/// The six-digit code both devices show.
pub fn code(
    vault_id: &VaultId,
    member: &Member,
    approver_nonce: &[u8; NONCE_LENGTH],
    device_nonce: &[u8; NONCE_LENGTH],
) -> Result<Zeroizing<String>> {
    let h = Sha256::digest(code_input(vault_id, member, approver_nonce, device_nonce)?);
    let number = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000;
    Ok(Zeroizing::new(format!("{number:06}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roster::Platform;
    use crate::wire::MemberId;

    fn member(name: &str) -> Member {
        Member {
            box_key: [1; 32],
            id: MemberId::parse("11111111-1111-4111-8111-111111111111").unwrap(),
            name: name.into(),
            platform: Platform::Oxyos,
            signing_key: [2; 32],
        }
    }

    #[test]
    fn commitment_opens_only_with_its_nonce_and_member() {
        let vault = VaultId::parse("22222222-2222-4222-8222-222222222222").unwrap();
        let device = nonce().unwrap();
        let committed = commitment(&vault, &member("ThinkPad"), &device).unwrap();
        assert!(opens(&committed, &vault, &member("ThinkPad"), &device).unwrap());
        assert!(!opens(&committed, &vault, &member("ThinkPa"), &device).unwrap());
        assert!(!opens(&committed, &vault, &member("ThinkPad"), &nonce().unwrap()).unwrap());
        let mut other = member("ThinkPad");
        other.box_key[0] ^= 1;
        assert!(!opens(&committed, &vault, &other, &device).unwrap());
    }

    #[test]
    fn code_is_six_digits_and_bound_to_everything() {
        let vault = VaultId::parse("22222222-2222-4222-8222-222222222222").unwrap();
        let (approver, device) = ([3u8; 32], [4u8; 32]);
        let code1 = code(&vault, &member("a"), &approver, &device).unwrap();
        assert_eq!(code1.len(), 6);
        assert!(code1.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(
            code1,
            code(&vault, &member("a"), &approver, &device).unwrap()
        );
        assert_ne!(
            code1,
            code(&vault, &member("a"), &device, &approver).unwrap()
        );
        assert_ne!(
            code1,
            code(&vault, &member("b"), &approver, &device).unwrap()
        );
    }

    #[test]
    fn subject_is_jcs() {
        let vault = VaultId::parse("22222222-2222-4222-8222-222222222222").unwrap();
        let text = String::from_utf8(subject(&vault, &member("Mac \"mini\"")).unwrap()).unwrap();
        assert_eq!(
            text,
            format!(
                "{{\"member\":{{\"boxKey\":\"{}\",\"id\":\"11111111-1111-4111-8111-111111111111\",\
                 \"name\":\"Mac \\\"mini\\\"\",\"platform\":\"oxyos\",\"signingKey\":\"{}\"}},\
                 \"vaultId\":\"22222222-2222-4222-8222-222222222222\"}}",
                crate::wire::b64(&[1; 32]),
                crate::wire::b64(&[2; 32])
            )
        );
    }
}
