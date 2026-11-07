//! What Oxy answers (`docs/identity/keychain.md`, `@oxy.so/contracts`' `keychain.ts`): the shapes
//! of its `/keychain` routes, read as Oxy writes them. Nothing here is trusted for what it says:
//! [`crate::vault`] checks every answer against what the machine verified before it uses it.

use crate::item::SealedItem;
use crate::roster::{Entry, Member};
use crate::seal::NONCE_LENGTH;
use crate::wire::{unb64, unb64_fixed, EnrollmentId, ItemId, MemberId, VaultId};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// The vault's counters (`KeychainVaultSummary`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSummary {
    pub id: VaultId,
    pub genesis_hash: String,
    pub roster_seq: u64,
    pub roster_head: String,
    pub generation: u32,
    pub revision: u64,
}

/// What Oxy says of a member (`KeychainMemberStatus`): its device session ended (`orphaned`), it
/// was removed.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberStatus {
    pub id: MemberId,
    pub orphaned: bool,
    pub removed: bool,
}

/// `GET /keychain`, and the answer of every roster write (`KeychainStateResponse`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub vault: Option<VaultSummary>,
    pub roster: Vec<Entry>,
    pub members: Vec<MemberStatus>,
    /// The live member bound to the caller's device session.
    #[serde(rename = "self")]
    pub own: Option<MemberId>,
}

/// An item as Oxy holds it (`KeychainItem`); a tombstone has no nonce or ciphertext.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerItem {
    pub id: ItemId,
    pub revision: u64,
    pub generation: u32,
    pub nonce: Option<String>,
    pub ciphertext: Option<String>,
    pub deleted_at: Option<String>,
    pub updated_at: String,
}

impl fmt::Debug for ServerItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerItem")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .field("generation", &self.generation)
            .field("deleted", &self.deleted_at.is_some())
            .finish_non_exhaustive()
    }
}

impl ServerItem {
    /// Its ciphertext; none for a tombstone. A half tombstone is refused.
    pub fn sealed(&self) -> Result<Option<SealedItem>> {
        match (&self.nonce, &self.ciphertext, &self.deleted_at) {
            (None, None, Some(_)) => Ok(None),
            (Some(nonce), Some(ciphertext), None) => Ok(Some(SealedItem {
                generation: self.generation,
                nonce: unb64_fixed::<NONCE_LENGTH>(nonce, "item")?,
                ciphertext: unb64(ciphertext, "item")?,
            })),
            _ => Err(Error::Malformed("item (neither an item nor a tombstone)")),
        }
    }
}

/// `GET /keychain/items` (`KeychainItemsResponse`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemsPage {
    pub items: Vec<ServerItem>,
    pub revision: u64,
    pub more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnrollmentStatus {
    Pending,
    Approved,
    Denied,
}

/// An enrollment (`KeychainEnrollment`): a device asking to join, and its approver's progress.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Enrollment {
    pub id: EnrollmentId,
    pub vault_id: VaultId,
    pub member: Member,
    pub commitment: String,
    pub approver_nonce: Option<String>,
    pub approver_member_id: Option<MemberId>,
    pub device_nonce: Option<String>,
    pub status: EnrollmentStatus,
    pub expires_at: String,
    pub created_at: String,
}

impl fmt::Debug for Enrollment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Enrollment")
            .field("id", &self.id)
            .field("member", &self.member)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl Enrollment {
    pub fn commitment(&self) -> Result<[u8; 32]> {
        unb64_fixed::<32>(&self.commitment, "enrollment")
    }

    pub fn approver_nonce(&self) -> Result<Option<[u8; 32]>> {
        self.approver_nonce
            .as_deref()
            .map(|nonce| unb64_fixed::<32>(nonce, "enrollment"))
            .transpose()
    }

    pub fn device_nonce(&self) -> Result<Option<[u8; 32]>> {
        self.device_nonce
            .as_deref()
            .map(|nonce| unb64_fixed::<32>(nonce, "enrollment"))
            .transpose()
    }
}

/// The keychain's error codes Oxy answers with (`KEYCHAIN_ERROR_CODES`).
pub mod codes {
    pub const NOT_AVAILABLE: &str = "KEYCHAIN_NOT_AVAILABLE";
    pub const DEVICE_REQUIRED: &str = "KEYCHAIN_DEVICE_REQUIRED";
    pub const EXISTS: &str = "KEYCHAIN_EXISTS";
    pub const NOT_FOUND: &str = "KEYCHAIN_NOT_FOUND";
    pub const NOT_MEMBER: &str = "KEYCHAIN_NOT_MEMBER";
    pub const ALREADY_MEMBER: &str = "KEYCHAIN_ALREADY_MEMBER";
    pub const ROSTER_CONFLICT: &str = "KEYCHAIN_ROSTER_CONFLICT";
    pub const ROSTER_INVALID: &str = "KEYCHAIN_ROSTER_INVALID";
    pub const WRAP_INVALID: &str = "KEYCHAIN_WRAP_INVALID";
    pub const ROTATION_INCOMPLETE: &str = "KEYCHAIN_ROTATION_INCOMPLETE";
    pub const LIMIT_REACHED: &str = "KEYCHAIN_LIMIT_REACHED";
    pub const ENROLLMENT_NOT_FOUND: &str = "KEYCHAIN_ENROLLMENT_NOT_FOUND";
    pub const ENROLLMENT_EXPIRED: &str = "KEYCHAIN_ENROLLMENT_EXPIRED";
    pub const ENROLLMENT_STATE: &str = "KEYCHAIN_ENROLLMENT_STATE";
    pub const COMMITMENT_MISMATCH: &str = "KEYCHAIN_COMMITMENT_MISMATCH";
    pub const ITEM_NOT_FOUND: &str = "KEYCHAIN_ITEM_NOT_FOUND";
    pub const ITEM_CONFLICT: &str = "KEYCHAIN_ITEM_CONFLICT";
    pub const GENERATION_INVALID: &str = "KEYCHAIN_GENERATION_INVALID";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_oxys_state_and_items() {
        let state: State =
            serde_json::from_str(r#"{"vault":null,"roster":[],"members":[],"self":null}"#).unwrap();
        assert!(state.vault.is_none() && state.own.is_none());
        let tombstone: ServerItem = serde_json::from_str(
            r#"{"id":"0f8fad5b-d9cb-469f-a165-70867728950e","revision":3,"generation":1,
                "nonce":null,"ciphertext":null,"deletedAt":"2026-11-07T12:00:00.000Z",
                "updatedAt":"2026-11-07T12:00:00.000Z"}"#,
        )
        .unwrap();
        assert!(tombstone.sealed().unwrap().is_none());
        let mut half = tombstone.clone();
        half.nonce = Some(crate::wire::b64(&[1; 24]));
        assert!(half.sealed().is_err());
        assert!(!format!("{half:?}").contains(half.nonce.as_deref().unwrap()));
    }
}
