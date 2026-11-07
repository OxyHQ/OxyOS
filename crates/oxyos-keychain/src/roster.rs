//! The roster (Oxy ADR 0063 D3): the vault's membership as an append-only log signed by its
//! members, verified from its first entry by every device, because Oxy, which relays it, cannot
//! be trusted to say who the members are.
//!
//! An entry on the wire is `{ body, signatures: [{ signer, signature }] }`, `body` the JCS of
//! `{ v: 1, vaultId, seq, prev, op, …, createdAt }`. Signatures are Ed25519 over
//! `"oxy.keychain.roster.v1\n" ‖ body`, and the entry's hash is the SHA-256 of that, in hex.
//!
//! [`VerifiedRoster`] accepts a roster only if every rule holds, and refuses anything it does not
//! know: an unknown field, operation, platform or signer, a body not in its canonical form.
//!
//! - `genesis` at `seq` 0 alone, signed by its member and by the recovery key it names.
//! - Each entry continues the one before (`seq` + 1, `prev` its hash) in the same vault.
//! - `add` a member never seen before, signed by a current member or the current recovery key
//!   (the recovery key signs nothing else).
//! - `remove` a current member, signed by a current member (itself included), and the very next
//!   entry is `generation`.
//! - `generation` the current generation + 1, signed by a current member.
//! - `recovery` new recovery keys, signed by a current member.
//!
//! A device keeps the [`Head`] it verified; a later roster must have the same genesis and contain
//! that head at its `seq` (an extension). A fork, a rewrite or a shorter roster is refused.

use crate::keys::{verify, KeyPairs};
use crate::recovery::RecoveryPublic;
use crate::wire::{
    b64, box_key, hex, is_box_key, is_signing_key, is_timestamp, jcs, signing_key, unb64_fixed,
    MemberId, VaultId, MAX_SAFE_INTEGER,
};
use crate::{Error, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

const DOMAIN: &[u8] = b"oxy.keychain.roster.v1\n";
const VERSION: u32 = 1;

/// The platforms a member may name (display only; a closed set).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Oxyos,
    Ios,
    Android,
    Macos,
    Windows,
    Linux,
    Web,
}

/// A member device: `{ id, name, platform, signingKey, boxKey }`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Member {
    #[serde(with = "box_key")]
    pub box_key: [u8; 32],
    pub id: MemberId,
    /// 1 to [`MAX_NAME_BYTES`] bytes.
    pub name: String,
    pub platform: Platform,
    #[serde(with = "signing_key")]
    pub signing_key: [u8; 32],
}

/// The longest a member's name may be, in UTF-8 bytes.
pub const MAX_NAME_BYTES: usize = 256;

impl Member {
    fn check(&self) -> Result<()> {
        if self.name.is_empty() || self.name.len() > MAX_NAME_BYTES {
            return Err(Error::Malformed("member (name)"));
        }
        if !is_signing_key(&self.signing_key) || !is_box_key(&self.box_key) {
            return Err(Error::Malformed("member (keys)"));
        }
        Ok(())
    }
}

impl fmt::Debug for Member {
    /// Its name is the person's (a device's name often is theirs): left out.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Member")
            .field("id", &self.id)
            .field("platform", &self.platform)
            .finish_non_exhaustive()
    }
}

/// Who signed or receives something: a member, or the recovery key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Principal {
    Member(MemberId),
    Recovery,
}

impl Principal {
    pub fn as_str(&self) -> &str {
        match self {
            Principal::Member(id) => id.as_str(),
            Principal::Recovery => "recovery",
        }
    }
}

impl Serialize for Principal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Principal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text == "recovery" {
            return Ok(Principal::Recovery);
        }
        MemberId::parse(&text)
            .map(Principal::Member)
            .map_err(serde::de::Error::custom)
    }
}

/// A 64-byte signature on the wire.
pub(crate) mod sig64 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::b64(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 64], D::Error> {
        let text = String::deserialize(deserializer)?;
        super::unb64_fixed::<64>(&text, "signature").map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntrySignature {
    pub signer: Principal,
    #[serde(with = "sig64")]
    pub signature: [u8; 64],
}

/// An entry as Oxy stores and relays it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    pub body: String,
    pub signatures: Vec<EntrySignature>,
}

impl Entry {
    /// `hex(SHA-256("oxy.keychain.roster.v1\n" ‖ body))`.
    pub fn hash(&self) -> String {
        hash(&self.body)
    }
}

fn signing_input(body: &str) -> Vec<u8> {
    [DOMAIN, body.as_bytes()].concat()
}

fn hash(body: &str) -> String {
    hex(&Sha256::digest(signing_input(body)))
}

/// What an entry does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Genesis {
        member: Member,
        recovery: RecoveryPublic,
    },
    Add {
        member: Member,
    },
    Remove {
        member_id: MemberId,
    },
    Generation {
        generation: u32,
    },
    Recovery {
        recovery: RecoveryPublic,
    },
}

/// An entry's body, parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryBody {
    pub vault_id: VaultId,
    pub seq: u64,
    /// The previous entry's hash; none at `seq` 0.
    pub prev: Option<String>,
    pub created_at: String,
    pub op: Op,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum OpName {
    Genesis,
    Add,
    Remove,
    Generation,
    Recovery,
}

/// The body on the wire: every operation's fields, those of the others absent. Fields are in
/// RFC 8785's order (the canonicalizer sorts them anyway).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireBody {
    created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generation: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    member: Option<Member>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    member_id: Option<MemberId>,
    op: OpName,
    prev: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery: Option<RecoveryPublic>,
    seq: u64,
    v: u32,
    vault_id: VaultId,
}

impl EntryBody {
    /// The body's JCS, what is signed and stored.
    pub fn to_jcs(&self) -> Result<String> {
        let (mut generation, mut member, mut member_id, mut recovery) = (None, None, None, None);
        let op = match &self.op {
            Op::Genesis {
                member: m,
                recovery: r,
            } => {
                member = Some(m.clone());
                recovery = Some(r.clone());
                OpName::Genesis
            }
            Op::Add { member: m } => {
                member = Some(m.clone());
                OpName::Add
            }
            Op::Remove { member_id: m } => {
                member_id = Some(m.clone());
                OpName::Remove
            }
            Op::Generation { generation: g } => {
                generation = Some(*g);
                OpName::Generation
            }
            Op::Recovery { recovery: r } => {
                recovery = Some(r.clone());
                OpName::Recovery
            }
        };
        let wire = WireBody {
            created_at: self.created_at.clone(),
            generation,
            member,
            member_id,
            op,
            prev: self.prev.clone(),
            recovery,
            seq: self.seq,
            v: VERSION,
            vault_id: self.vault_id.clone(),
        };
        let bytes = jcs(&wire)?;
        String::from_utf8(bytes).map_err(|_| Error::Malformed("canonical JSON"))
    }

    /// Parses a body: its schema exactly, and nothing but its canonical form.
    pub fn parse(body: &str) -> Result<Self> {
        let malformed = || Error::Malformed("roster entry");
        let wire: WireBody = serde_json::from_str(body).map_err(|_| malformed())?;
        if wire.v != VERSION || wire.seq > MAX_SAFE_INTEGER || !is_timestamp(&wire.created_at) {
            return Err(malformed());
        }
        // `prev` is null at `seq` 0 and a hash after.
        if (wire.seq == 0) != wire.prev.is_none() {
            return Err(malformed());
        }
        if let Some(prev) = &wire.prev {
            let is_hash = prev.len() == 64
                && prev
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !is_hash {
                return Err(malformed());
            }
        }
        let op = match (
            wire.op,
            wire.member,
            wire.member_id,
            wire.generation,
            wire.recovery,
        ) {
            (OpName::Genesis, Some(member), None, None, Some(recovery)) if wire.seq == 0 => {
                Op::Genesis { member, recovery }
            }
            (OpName::Add, Some(member), None, None, None) => Op::Add { member },
            (OpName::Remove, None, Some(member_id), None, None) => Op::Remove { member_id },
            (OpName::Generation, None, None, Some(generation), None) if generation >= 2 => {
                Op::Generation { generation }
            }
            (OpName::Recovery, None, None, None, Some(recovery)) => Op::Recovery { recovery },
            _ => return Err(malformed()),
        };
        if let Op::Genesis { member, .. } | Op::Add { member } = &op {
            member.check()?;
        }
        let parsed = EntryBody {
            vault_id: wire.vault_id,
            seq: wire.seq,
            prev: wire.prev,
            created_at: wire.created_at,
            op,
        };
        // A second spelling of the same body (another order, spaces, escapes, a missing `prev`)
        // would hash differently from the one signed: only the canonical one is accepted.
        if parsed.to_jcs()? != body {
            return Err(Error::Malformed("roster entry (not canonical)"));
        }
        Ok(parsed)
    }

    /// The signed entry: each signer's signature over the body, in the order given.
    pub fn sign(&self, signers: &[(Principal, &KeyPairs)]) -> Result<Entry> {
        let body = self.to_jcs()?;
        let input = signing_input(&body);
        let signatures = signers
            .iter()
            .map(|(signer, keys)| EntrySignature {
                signer: signer.clone(),
                signature: keys.sign(&input),
            })
            .collect();
        Ok(Entry { body, signatures })
    }

    /// The first entry of a new vault: to be signed by `member` and by the recovery key.
    pub fn genesis(
        vault_id: VaultId,
        member: Member,
        recovery: RecoveryPublic,
        created_at: String,
    ) -> Self {
        Self {
            vault_id,
            seq: 0,
            prev: None,
            created_at,
            op: Op::Genesis { member, recovery },
        }
    }
}

/// Why a roster was refused, as the SDK's `KeychainRosterError` reasons (shown by
/// [`RosterError::reason`]). `at` is the entry's position.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RosterError {
    /// No entries, or a first entry that is not a genesis.
    Genesis { at: usize },
    /// An entry or a body not of its schema, or not in its canonical form.
    Format { at: usize },
    /// An entry that does not continue the one before (its `seq` or `prev`).
    Continuity { at: usize },
    /// An entry of another vault.
    VaultMismatch { at: usize },
    /// A signer the rules do not allow for this entry: not a current member, the recovery key
    /// where it may not sign, a required signer missing.
    Signer { at: usize },
    /// Signatures that do not verify, or not as many as the entry takes, or one signer twice.
    Signature { at: usize },
    /// An `add` of a member id already used, or a `remove` of one who is not a member.
    Membership { at: usize },
    /// A `generation` that is not the next.
    Generation { at: usize },
    /// A `remove` not followed by its `generation`.
    Incomplete { at: usize },
    /// Not the vault this device knows (another genesis).
    GenesisMismatch,
    /// Not an extension of the head this device verified: a fork, a rewrite, a shorter roster.
    NotExtension,
}

impl RosterError {
    /// The SDK's name for the reason.
    pub fn reason(&self) -> &'static str {
        match self {
            RosterError::Genesis { .. } => "genesis",
            RosterError::Format { .. } => "format",
            RosterError::Continuity { .. } => "continuity",
            RosterError::VaultMismatch { .. } => "vault_mismatch",
            RosterError::Signer { .. } => "signer",
            RosterError::Signature { .. } => "signature",
            RosterError::Membership { .. } => "membership",
            RosterError::Generation { .. } => "generation",
            RosterError::Incomplete { .. } => "incomplete",
            RosterError::GenesisMismatch => "genesis_mismatch",
            RosterError::NotExtension => "not_extension",
        }
    }

    fn at(&self) -> Option<usize> {
        match *self {
            RosterError::Genesis { at }
            | RosterError::Format { at }
            | RosterError::Continuity { at }
            | RosterError::VaultMismatch { at }
            | RosterError::Signer { at }
            | RosterError::Signature { at }
            | RosterError::Membership { at }
            | RosterError::Generation { at }
            | RosterError::Incomplete { at } => Some(at),
            RosterError::GenesisMismatch | RosterError::NotExtension => None,
        }
    }
}

impl fmt::Display for RosterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())?;
        match self.at() {
            Some(at) => write!(f, " at entry {at}"),
            None => Ok(()),
        }
    }
}

impl std::error::Error for RosterError {}

/// What a device keeps of the roster it verified: the vault's identity and the newest head.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Head {
    pub genesis_hash: String,
    pub seq: u64,
    pub hash: String,
}

#[derive(Clone, Debug)]
struct Current {
    member: Member,
    /// Admitted with the recovery key: its wrap may be signed by the recovery key (D6).
    by_recovery: bool,
}

/// A roster that verified: its members, its recovery keys and its generation now.
#[derive(Clone, Debug)]
pub struct VerifiedRoster {
    vault_id: VaultId,
    hashes: Vec<String>,
    members: BTreeMap<MemberId, Current>,
    /// Every member id ever added: an id is never used again.
    used: BTreeSet<MemberId>,
    removed: Vec<MemberId>,
    recovery: RecoveryPublic,
    generation: u32,
    /// The last entry is a `remove`, which must be followed by `generation`.
    pending_generation: bool,
}

impl VerifiedRoster {
    /// Verifies `entries` from the genesis, as entries of `vault_id`. With `known`, the head this
    /// device verified before, they must also be that vault (the same genesis) and extend that
    /// head.
    pub fn verify(
        vault_id: &VaultId,
        entries: &[Entry],
        known: Option<&Head>,
    ) -> Result<Self, RosterError> {
        let (first, rest) = entries
            .split_first()
            .ok_or(RosterError::Genesis { at: 0 })?;
        let mut roster = Self::from_genesis(vault_id, first)?;
        for entry in rest {
            roster.apply(entry)?;
        }
        roster.check_complete()?;
        if let Some(known) = known {
            if roster.hashes[0] != known.genesis_hash {
                return Err(RosterError::GenesisMismatch);
            }
            let extends = usize::try_from(known.seq)
                .ok()
                .and_then(|at| roster.hashes.get(at))
                .is_some_and(|hash| *hash == known.hash);
            if !extends {
                return Err(RosterError::NotExtension);
            }
        }
        Ok(roster)
    }

    /// This roster with `entries` appended, if they verify (an append about to be sent, or the
    /// entries after a head already verified).
    pub fn extend(&self, entries: &[Entry]) -> Result<Self, RosterError> {
        let mut roster = self.clone();
        for entry in entries {
            roster.apply(entry)?;
        }
        roster.check_complete()?;
        Ok(roster)
    }

    fn parse(entry: &Entry, at: usize) -> Result<EntryBody, RosterError> {
        let body = EntryBody::parse(&entry.body).map_err(|_| RosterError::Format { at })?;
        if body.seq != at as u64 {
            return Err(RosterError::Continuity { at });
        }
        Ok(body)
    }

    fn from_genesis(vault_id: &VaultId, entry: &Entry) -> Result<Self, RosterError> {
        let at = 0;
        let body = Self::parse(entry, at)?;
        let Op::Genesis { member, recovery } = body.op else {
            return Err(RosterError::Genesis { at });
        };
        if body.vault_id != *vault_id {
            return Err(RosterError::VaultMismatch { at });
        }
        // Two signatures: the member's and those of the recovery key the genesis introduces,
        // which proves the person held the recovery key when the vault was made.
        let mut roster = Self {
            vault_id: vault_id.clone(),
            hashes: Vec::new(),
            used: BTreeSet::from([member.id.clone()]),
            members: BTreeMap::from([(
                member.id.clone(),
                Current {
                    member: member.clone(),
                    by_recovery: false,
                },
            )]),
            removed: Vec::new(),
            recovery,
            generation: 1,
            pending_generation: false,
        };
        let signers = roster.check_signatures(entry, at, 2, true)?;
        if !signers.contains(&Principal::Member(member.id.clone()))
            || !signers.contains(&Principal::Recovery)
        {
            return Err(RosterError::Signer { at });
        }
        roster.hashes.push(entry.hash());
        Ok(roster)
    }

    fn apply(&mut self, entry: &Entry) -> Result<(), RosterError> {
        let at = self.hashes.len();
        let body = Self::parse(entry, at)?;
        if body.prev.as_ref() != self.hashes.last() {
            return Err(RosterError::Continuity { at });
        }
        if body.vault_id != self.vault_id {
            return Err(RosterError::VaultMismatch { at });
        }
        if self.pending_generation && !matches!(body.op, Op::Generation { .. }) {
            return Err(RosterError::Incomplete { at });
        }
        match body.op {
            Op::Genesis { .. } => return Err(RosterError::Genesis { at }),
            Op::Add { member } => {
                let signers = self.check_signatures(entry, at, 1, true)?;
                if self.used.contains(&member.id) {
                    return Err(RosterError::Membership { at });
                }
                self.used.insert(member.id.clone());
                self.members.insert(
                    member.id.clone(),
                    Current {
                        member,
                        by_recovery: signers.contains(&Principal::Recovery),
                    },
                );
            }
            Op::Remove { member_id } => {
                self.check_signatures(entry, at, 1, false)?;
                if self.members.remove(&member_id).is_none() {
                    return Err(RosterError::Membership { at });
                }
                self.removed.push(member_id);
                self.pending_generation = true;
            }
            Op::Generation { generation } => {
                self.check_signatures(entry, at, 1, false)?;
                if Some(generation) != self.generation.checked_add(1) {
                    return Err(RosterError::Generation { at });
                }
                self.generation = generation;
                self.pending_generation = false;
            }
            Op::Recovery { recovery } => {
                self.check_signatures(entry, at, 1, false)?;
                self.recovery = recovery;
            }
        }
        self.hashes.push(entry.hash());
        Ok(())
    }

    /// Checks an entry's signatures: exactly `expected`; no signer twice; each a current member,
    /// or the current recovery key where `recovery_may_sign`; each verifying. Says who signed.
    fn check_signatures(
        &self,
        entry: &Entry,
        at: usize,
        expected: usize,
        recovery_may_sign: bool,
    ) -> Result<BTreeSet<Principal>, RosterError> {
        if entry.signatures.len() != expected {
            return Err(RosterError::Signature { at });
        }
        let input = signing_input(&entry.body);
        let mut signers = BTreeSet::new();
        for signature in &entry.signatures {
            if !signers.insert(signature.signer.clone()) {
                return Err(RosterError::Signature { at });
            }
            let key = match &signature.signer {
                Principal::Recovery if recovery_may_sign => &self.recovery.signing_key,
                Principal::Recovery => return Err(RosterError::Signer { at }),
                Principal::Member(id) => match self.members.get(id) {
                    Some(current) => &current.member.signing_key,
                    None => return Err(RosterError::Signer { at }),
                },
            };
            if !verify(key, &input, &signature.signature) {
                return Err(RosterError::Signature { at });
            }
        }
        Ok(signers)
    }

    /// A roster may not end on a `remove`: its `generation` comes in the same request.
    fn check_complete(&self) -> Result<(), RosterError> {
        if self.pending_generation {
            return Err(RosterError::Incomplete {
                at: self.hashes.len() - 1,
            });
        }
        Ok(())
    }

    pub fn vault_id(&self) -> &VaultId {
        &self.vault_id
    }

    /// The vault's identity: its genesis entry's hash.
    pub fn genesis_hash(&self) -> &str {
        &self.hashes[0]
    }

    /// The head to keep, and to check the next roster against.
    pub fn head(&self) -> Head {
        Head {
            genesis_hash: self.hashes[0].clone(),
            seq: self.hashes.len() as u64 - 1,
            hash: self.hashes[self.hashes.len() - 1].clone(),
        }
    }

    /// The vault key's generation now.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn recovery(&self) -> &RecoveryPublic {
        &self.recovery
    }

    /// A current member.
    pub fn member(&self, id: &MemberId) -> Option<&Member> {
        self.members.get(id).map(|current| &current.member)
    }

    /// Whether a current member was admitted with the recovery key.
    pub(crate) fn admitted_by_recovery(&self, id: &MemberId) -> bool {
        self.members
            .get(id)
            .is_some_and(|current| current.by_recovery)
    }

    /// The current members, by id.
    pub fn members(&self) -> impl Iterator<Item = &Member> {
        self.members.values().map(|current| &current.member)
    }

    /// The members removed, in the roster's order.
    pub fn removed(&self) -> &[MemberId] {
        &self.removed
    }

    /// The body of the next entry, continuing the head.
    pub fn next(&self, op: Op, created_at: String) -> EntryBody {
        let head = self.head();
        EntryBody {
            vault_id: self.vault_id.clone(),
            seq: head.seq + 1,
            prev: Some(head.hash),
            created_at,
            op,
        }
    }
}
