//! Wraps (Oxy ADR 0063 D4, D8): the keyring sealed to one recipient (a member, or `recovery`)
//! and signed by the member that sealed it (or, for a device admitted with the recovery key, by
//! the recovery key).
//!
//! ```text
//! aad       = "oxy.keychain.wrap.v1" ‖ 0x00 ‖ vaultId ‖ 0x00 ‖ recipient ‖ 0x00 ‖ g
//! signature = Ed25519("oxy.keychain.wrap.v1\n" ‖ JCS({ vaultId, recipient, generation, epk, nonce, ciphertext, signer }))
//! ```
//!
//! A wrap is opened only if it is addressed to the one opening it, as the verified roster knows
//! it; its signer is a current member of that roster (or its current recovery key, for a member
//! the recovery key admitted); it is of the roster's generation; and the keyring inside is that
//! generation's. Oxy therefore cannot hand a device a keyring of its own making.

use crate::keyring::Keyring;
use crate::keys::{verify, KeyPairs};
use crate::roster::{sig64, Principal, VerifiedRoster};
use crate::seal::{self, Sealed, NONCE_LENGTH};
use crate::wire::{b64, jcs, VaultId};
use crate::{Error, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

const DOMAIN: &str = "oxy.keychain.wrap.v1";

/// A wrap as Oxy stores and relays it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Wrap {
    pub recipient: Principal,
    pub generation: u32,
    #[serde(with = "crate::wire::key32")]
    pub epk: [u8; 32],
    #[serde(with = "nonce24")]
    pub nonce: [u8; NONCE_LENGTH],
    #[serde(with = "bytes")]
    pub ciphertext: Vec<u8>,
    pub signer: Principal,
    #[serde(with = "sig64")]
    pub signature: [u8; 64],
}

impl fmt::Debug for Wrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Wrap")
            .field("recipient", &self.recipient)
            .field("generation", &self.generation)
            .field("signer", &self.signer)
            .finish_non_exhaustive()
    }
}

/// What a wrap's signature covers, in RFC 8785's order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Signed<'a> {
    ciphertext: String,
    epk: String,
    generation: u32,
    nonce: String,
    recipient: &'a Principal,
    signer: &'a Principal,
    vault_id: &'a VaultId,
}

/// `"oxy.keychain.wrap.v1" ‖ 0x00 ‖ vaultId ‖ 0x00 ‖ recipient ‖ 0x00 ‖ g`.
pub fn aad(vault_id: &VaultId, recipient: &Principal, generation: u32) -> Vec<u8> {
    [
        DOMAIN.as_bytes(),
        &[0],
        vault_id.as_str().as_bytes(),
        &[0],
        recipient.as_str().as_bytes(),
        &[0],
        generation.to_string().as_bytes(),
    ]
    .concat()
}

impl Wrap {
    /// Seals `keyring` (at its newest generation) to `recipient`'s box key and signs it.
    pub fn create(
        vault_id: &VaultId,
        recipient: Principal,
        recipient_box_key: &[u8; 32],
        keyring: &Keyring,
        signer: Principal,
        signing: &KeyPairs,
    ) -> Result<Self> {
        let generation = keyring.generation();
        let plaintext = keyring.to_plaintext()?;
        let sealed = seal::seal(
            recipient_box_key,
            &plaintext,
            &aad(vault_id, &recipient, generation),
        )?;
        Self::signed(vault_id, recipient, generation, sealed, signer, signing)
    }

    /// [`Wrap::create`] with the ephemeral secret and the nonce given: for the vectors only.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create_with(
        ephemeral: &[u8; 32],
        nonce: &[u8; NONCE_LENGTH],
        vault_id: &VaultId,
        recipient: Principal,
        recipient_box_key: &[u8; 32],
        keyring: &Keyring,
        signer: Principal,
        signing: &KeyPairs,
    ) -> Result<Self> {
        let generation = keyring.generation();
        let plaintext = keyring.to_plaintext()?;
        let sealed = seal::seal_with(
            ephemeral,
            nonce,
            recipient_box_key,
            &plaintext,
            &aad(vault_id, &recipient, generation),
        )?;
        Self::signed(vault_id, recipient, generation, sealed, signer, signing)
    }

    fn signed(
        vault_id: &VaultId,
        recipient: Principal,
        generation: u32,
        sealed: Sealed,
        signer: Principal,
        signing: &KeyPairs,
    ) -> Result<Self> {
        let mut wrap = Self {
            recipient,
            generation,
            epk: sealed.epk,
            nonce: sealed.nonce,
            ciphertext: sealed.ciphertext,
            signer,
            signature: [0; 64],
        };
        wrap.signature = signing.sign(&wrap.signing_input(vault_id)?);
        Ok(wrap)
    }

    /// `"oxy.keychain.wrap.v1\n" ‖ JCS({ vaultId, recipient, generation, epk, nonce, ciphertext, signer })`.
    pub fn signing_input(&self, vault_id: &VaultId) -> Result<Vec<u8>> {
        let signed = Signed {
            ciphertext: b64(&self.ciphertext),
            epk: b64(&self.epk),
            generation: self.generation,
            nonce: b64(&self.nonce),
            recipient: &self.recipient,
            signer: &self.signer,
            vault_id,
        };
        let mut input = format!("{DOMAIN}\n").into_bytes();
        input.extend(jcs(&signed)?);
        Ok(input)
    }

    /// Opens the wrap addressed to `me`, whose keys are `keys`, checked against `roster`.
    pub fn open(
        &self,
        roster: &VerifiedRoster,
        me: &Principal,
        keys: &KeyPairs,
    ) -> Result<Keyring> {
        if self.recipient != *me {
            return Err(Error::Signature(
                "the wrap is not addressed to this recipient",
            ));
        }
        // The roster must know the recipient by these keys: a wrap sealed to a member the roster
        // does not have, or to keys it does not name, is not this vault's.
        let own_box = match me {
            Principal::Member(id) => roster
                .member(id)
                .map(|member| member.box_key)
                .ok_or(Error::Signature("the recipient is not a member"))?,
            Principal::Recovery => roster.recovery().box_key,
        };
        if own_box != keys.box_key() {
            return Err(Error::Signature(
                "the roster names other keys for the recipient",
            ));
        }
        let signer_key = match &self.signer {
            Principal::Member(id) => {
                roster
                    .member(id)
                    .ok_or(Error::Signature("the signer is not a current member"))?
                    .signing_key
            }
            // The recovery key signs the wrap of a member it admitted, and no other (D4, D6).
            Principal::Recovery => match &self.recipient {
                Principal::Member(id) if roster.admitted_by_recovery(id) => {
                    roster.recovery().signing_key
                }
                _ => return Err(Error::Signature("the recovery key may not sign this wrap")),
            },
        };
        if !verify(
            &signer_key,
            &self.signing_input(roster.vault_id())?,
            &self.signature,
        ) {
            return Err(Error::Signature("the wrap's signature does not verify"));
        }
        if self.generation != roster.generation() {
            return Err(Error::Signature(
                "the wrap is not of the roster's generation",
            ));
        }
        let sealed = Sealed {
            epk: self.epk,
            nonce: self.nonce,
            ciphertext: self.ciphertext.clone(),
        };
        let plaintext = seal::open(
            keys.box_secret(),
            &sealed,
            &aad(roster.vault_id(), &self.recipient, self.generation),
        )?;
        let keyring = Keyring::from_plaintext(&plaintext)?;
        if keyring.generation() != self.generation {
            return Err(Error::Malformed("keyring (not of the wrap's generation)"));
        }
        Ok(keyring)
    }
}

pub(crate) mod nonce24 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 24], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&crate::wire::b64(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 24], D::Error> {
        let text = String::deserialize(deserializer)?;
        crate::wire::unb64_fixed::<24>(&text, "nonce").map_err(serde::de::Error::custom)
    }
}

pub(crate) mod bytes {
    use super::*;

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&b64(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        crate::wire::unb64(&text, "ciphertext").map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roster::Op;
    use crate::tests::{Device, Vault};

    fn wrap_for(vault: &Vault, to: &Device, keyring: &Keyring, signer: &Device) -> Wrap {
        Wrap::create(
            &vault.id,
            to.principal(),
            &to.keys.box_key(),
            keyring,
            signer.principal(),
            &signer.keys,
        )
        .unwrap()
    }

    #[test]
    fn opens_for_its_recipient_from_a_member() {
        let mut vault = Vault::new();
        let b = Device::new("b");
        vault.add(&b);
        let roster = vault.roster();
        let keyring = Keyring::generate().unwrap();
        let wrap = wrap_for(&vault, &b, &keyring, &vault.a);
        let opened = wrap.open(&roster, &b.principal(), &b.keys).unwrap();
        assert_eq!(opened.newest().1.bytes(), keyring.newest().1.bytes());
        // The recovery wrap, opened with the recovery key.
        let to_recovery = Wrap::create(
            &vault.id,
            Principal::Recovery,
            &vault.recovery.box_key(),
            &keyring,
            vault.a.principal(),
            &vault.a.keys,
        )
        .unwrap();
        assert!(to_recovery
            .open(&roster, &Principal::Recovery, &vault.recovery)
            .is_ok());
        // On the wire and back.
        let json = serde_json::to_string(&wrap).unwrap();
        let read: Wrap = serde_json::from_str(&json).unwrap();
        assert!(read == wrap);
        let extra = json.replacen('{', "{\"x\":1,", 1);
        assert!(serde_json::from_str::<Wrap>(&extra).is_err());
    }

    #[test]
    fn refuses_every_change_and_every_wrong_party() {
        let mut vault = Vault::new();
        let b = Device::new("b");
        vault.add(&b);
        let roster = vault.roster();
        let keyring = Keyring::generate().unwrap();
        let wrap = wrap_for(&vault, &b, &keyring, &vault.a);
        let refused = |wrap: &Wrap| wrap.open(&roster, &b.principal(), &b.keys).is_err();
        for change in 0..6 {
            let mut changed = wrap.clone();
            match change {
                0 => changed.ciphertext[3] ^= 1,
                1 => changed.nonce[0] ^= 1,
                2 => changed.epk[0] ^= 1,
                3 => changed.signature[0] ^= 1,
                4 => changed.generation = 2,
                _ => changed.signer = Principal::Member(b.id.clone()),
            }
            assert!(refused(&changed), "change {change}");
        }
        // Addressed to another, or opened by another.
        assert!(wrap
            .open(&roster, &vault.a.principal(), &vault.a.keys)
            .is_err());
        // Sealed to b's key but named for a: refused before any decryption.
        let misnamed = Wrap::create(
            &vault.id,
            vault.a.principal(),
            &b.keys.box_key(),
            &keyring,
            vault.a.principal(),
            &vault.a.keys,
        )
        .unwrap();
        assert!(misnamed
            .open(&roster, &vault.a.principal(), &b.keys)
            .is_err());
        // Signed by a stranger, or by a member of another vault.
        let stranger = Device::new("s");
        assert!(refused(&wrap_for(&vault, &b, &keyring, &stranger)));
        let mut elsewhere = Vault::new();
        elsewhere.add(&b);
        let from_elsewhere = Wrap::create(
            &elsewhere.id,
            b.principal(),
            &b.keys.box_key(),
            &keyring,
            elsewhere.a.principal(),
            &elsewhere.a.keys,
        )
        .unwrap();
        assert!(refused(&from_elsewhere));
    }

    #[test]
    fn the_recovery_key_signs_only_the_wraps_of_members_it_admitted() {
        let mut vault = Vault::new();
        let b = Device::new("b");
        vault.add(&b);
        let c = Device::new("c");
        vault.entries.push(vault.next(
            Op::Add {
                member: c.member.clone(),
            },
            (Principal::Recovery, &vault.recovery),
        ));
        let roster = vault.roster();
        let keyring = Keyring::generate().unwrap();
        let by_recovery = |to: &Device| {
            Wrap::create(
                &vault.id,
                to.principal(),
                &to.keys.box_key(),
                &keyring,
                Principal::Recovery,
                &vault.recovery,
            )
            .unwrap()
        };
        assert!(by_recovery(&c)
            .open(&roster, &c.principal(), &c.keys)
            .is_ok());
        assert!(by_recovery(&b)
            .open(&roster, &b.principal(), &b.keys)
            .is_err());
        let recovery_by_recovery = Wrap::create(
            &vault.id,
            Principal::Recovery,
            &vault.recovery.box_key(),
            &keyring,
            Principal::Recovery,
            &vault.recovery,
        )
        .unwrap();
        assert!(recovery_by_recovery
            .open(&roster, &Principal::Recovery, &vault.recovery)
            .is_err());
    }

    #[test]
    fn a_removed_member_signs_and_receives_nothing() {
        let mut vault = Vault::new();
        let b = Device::new("b");
        vault.add(&b);
        let keyring = Keyring::generate().unwrap();
        let from_b = wrap_for(&vault, &vault.a, &keyring, &b);
        let to_b = wrap_for(&vault, &b, &keyring, &vault.a);
        let before = vault.roster();
        assert!(from_b
            .open(&before, &vault.a.principal(), &vault.a.keys)
            .is_ok());
        let remove = vault.next(
            Op::Remove {
                member_id: b.id.clone(),
            },
            (vault.a.principal(), &vault.a.keys),
        );
        let rotation = crate::roster::EntryBody {
            vault_id: vault.id.clone(),
            seq: 3,
            prev: Some(remove.hash()),
            created_at: crate::tests::AT.into(),
            op: Op::Generation { generation: 2 },
        }
        .sign(&[(vault.a.principal(), &vault.a.keys)])
        .unwrap();
        vault.entries.extend([remove, rotation]);
        let after = vault.roster();
        assert!(from_b
            .open(&after, &vault.a.principal(), &vault.a.keys)
            .is_err());
        assert!(to_b.open(&after, &b.principal(), &b.keys).is_err());
        // The rotated keyring, to the remaining member: generation 2, and generation 1 still.
        let mut rotated = keyring;
        rotated.rotate().unwrap();
        let to_a = wrap_for(&vault, &vault.a, &rotated, &vault.a);
        let opened = to_a
            .open(&after, &vault.a.principal(), &vault.a.keys)
            .unwrap();
        assert_eq!(opened.generation(), 2);
        assert!(opened.get(1).is_some());
        // An old wrap at generation 1: not the roster's generation now.
        let stale = wrap_for(&vault, &vault.a, &Keyring::generate().unwrap(), &vault.a);
        assert!(stale
            .open(&after, &vault.a.principal(), &vault.a.keys)
            .is_err());
    }

    #[test]
    fn the_keyring_inside_is_of_the_wraps_generation() {
        let mut vault = Vault::new();
        vault.append(Op::Generation { generation: 2 });
        let roster = vault.roster();
        // A keyring of generation 1 sealed and signed as generation 2.
        let keyring = Keyring::generate().unwrap();
        let me = vault.a.principal();
        let sealed = seal::seal(
            &vault.a.keys.box_key(),
            &keyring.to_plaintext().unwrap(),
            &aad(&vault.id, &me, 2),
        )
        .unwrap();
        let wrap =
            Wrap::signed(&vault.id, me.clone(), 2, sealed, me.clone(), &vault.a.keys).unwrap();
        assert!(matches!(
            wrap.open(&roster, &me, &vault.a.keys),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn debug_shows_no_ciphertext() {
        let vault = Vault::new();
        let keyring = Keyring::generate().unwrap();
        let wrap = wrap_for(&vault, &vault.a, &keyring, &vault.a);
        let shown = format!("{wrap:?}");
        assert!(!shown.contains(&b64(&wrap.ciphertext)[..16]), "{shown}");
        assert!(!shown.contains(&b64(&wrap.signature)[..16]));
    }
}
