//! The vault as a member device holds it (Oxy ADR 0063; OxyOS ADR 0050 decisions 1 to 5): what
//! the service does with Oxy's answers, without the network. Each operation is in two halves: what
//! to send (entries, wraps, ciphertexts made and signed here), and the answer checked before
//! anything of it is kept. The service makes the calls between them.
//!
//! - **What is pinned.** The vault's id and genesis, and the roster's head this device verified:
//!   every roster Oxy sends must be that vault and extend that head ([`VerifiedRoster::verify`]),
//!   so a fork, a rewrite or a shorter roster is refused. A device joining trusts the genesis it
//!   first sees (the person's comparison of the six-digit code, or the recovery key, binds it).
//! - **Items** come with Oxy's revisions, which only go up: a page whose revision is below the one
//!   synced, or items out of order, are refused; an item whose counter went back, or that does not
//!   open, is not taken (the copy kept stays) and is counted.
//! - **A rotation** (removing a member) is made here: `remove` and `generation` signed by this
//!   member, a new keyring generation sealed to every member left and to the recovery key. It is
//!   this device's keyring only once Oxy accepted it.
//! - **What an application may be offered** ([`associated`]): logins whose `apps` name its desktop
//!   entry id.

use crate::enroll;
use crate::item::{Content, Item, SealedItem};
use crate::keyring::Keyring;
use crate::keys::KeyPairs;
use crate::recovery::RecoveryKey;
use crate::roster::{Entry, EntryBody, Member, Op, Platform, Principal, VerifiedRoster};
use crate::server::{Enrollment, EnrollmentStatus, ItemsPage, ServerItem, State};
use crate::store::{StoredItem, StoredItems, Unlocked};
use crate::wire::{EnrollmentId, ItemId, MemberId, VaultId};
use crate::wrap::Wrap;
use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// A member device's vault: its member and keys, the keyring, the verified roster, the items'
/// ciphertext.
pub struct Vault {
    id: VaultId,
    me: MemberId,
    keys: KeyPairs,
    keyring: Keyring,
    roster: VerifiedRoster,
    entries: Vec<Entry>,
    items: StoredItems,
    pinned: bool,
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vault")
            .field("id", &self.id)
            .field("me", &self.me)
            .field("generation", &self.keyring.generation())
            .field("revision", &self.items.revision)
            .finish_non_exhaustive()
    }
}

/// What Oxy's state says of this member, once checked.
pub enum Look {
    /// The account has no vault any more (it was reset).
    Gone,
    /// This member was removed from the roster.
    Removed,
    /// The roster still has this member, but Oxy binds this device session to none (the device
    /// was set up again): another device removes it, and this machine joins again.
    Unbound,
    /// Still a member: the roster verified, and the members Oxy says are orphaned.
    Member(Box<Checked>),
}

/// A roster that verified as an extension of the head kept.
pub struct Checked {
    roster: VerifiedRoster,
    entries: Vec<Entry>,
    /// Current members other than this one whose device session ended.
    pub orphaned: Vec<MemberId>,
}

/// What a page of items came to.
#[derive(Debug, PartialEq, Eq)]
pub struct Applied {
    /// Items not taken: their counter went back, or they did not open.
    pub refused: usize,
    /// More follow: ask again from this revision.
    pub next: Option<u64>,
}

/// An item sealed to be written: its base revision and the ciphertext sent.
pub struct Prepared {
    pub id: ItemId,
    pub base: Option<u64>,
    pub sealed: SealedItem,
    counter: u64,
}

/// A removal with its rotation, to be appended.
pub struct Rotation {
    pub entries: Vec<Entry>,
    pub wraps: Vec<Wrap>,
    keyring: Keyring,
    roster: VerifiedRoster,
}

/// A vault made here, before Oxy has it: the recovery key to show once, and what is sent.
pub struct Created {
    pub vault: Vault,
    pub recovery: RecoveryKey,
    pub genesis: Entry,
    pub wraps: Vec<Wrap>,
}

/// The six-digit code: an approval's, as both devices show it.
pub type Code = Zeroizing<String>;

/// Whether `item` may be offered to the application `desktop_entry_id` (decision 5): a login
/// whose `apps` name it. (Domains associated with an application are a later step: Oxy's
/// registry's `webOrigins` for its own applications, a domain's proof for any other.)
pub fn associated(item: &Item, desktop_entry_id: &str) -> bool {
    match &item.content {
        Content::Login(login) => login
            .apps
            .iter()
            .any(|app| app.expose() == desktop_entry_id),
        _ => false,
    }
}

/// The member a device of this platform is, named `name`.
fn new_member(name: &str) -> Result<(Member, KeyPairs)> {
    let keys = KeyPairs::generate()?;
    let member = Member {
        box_key: keys.box_key(),
        id: MemberId::random()?,
        name: name.to_owned(),
        platform: Platform::Oxyos,
        signing_key: keys.signing_key(),
    };
    Ok((member, keys))
}

/// Verifies Oxy's roster from its genesis, as entries of `vault_id`, extending `known` if given.
fn verify(
    vault_id: &VaultId,
    state: &State,
    known: Option<&crate::roster::Head>,
) -> Result<VerifiedRoster> {
    let summary = state.vault.as_ref().ok_or(Error::Answer("no vault"))?;
    if summary.id != *vault_id {
        return Err(Error::Roster(crate::roster::RosterError::GenesisMismatch));
    }
    let roster = VerifiedRoster::verify(vault_id, &state.roster, known)?;
    let head = roster.head();
    if summary.genesis_hash != head.genesis_hash
        || summary.roster_head != head.hash
        || summary.roster_seq != head.seq
        || summary.generation != roster.generation()
    {
        return Err(Error::Answer("the vault's counters are not its roster's"));
    }
    Ok(roster)
}

impl Vault {
    /// The vault kept in `store`, if this machine is a member.
    pub fn load(store: &Unlocked) -> Result<Option<Self>> {
        let member = store.member()?;
        let keyring = store.keyring()?;
        let roster = store.stored_roster()?;
        let (member, keyring, (roster, entries)) = match (member, keyring, roster) {
            (None, None, None) => return Ok(None),
            (Some(member), Some(keyring), Some(roster)) => (member, keyring, roster),
            _ => return Err(Error::Malformed("store (a vault half kept)")),
        };
        let known = roster
            .member(&member.id)
            .ok_or(Error::Malformed("store (not a member of its roster)"))?;
        if known.signing_key != member.keys.signing_key() || known.box_key != member.keys.box_key()
        {
            return Err(Error::Malformed("store (the roster names other keys)"));
        }
        Ok(Some(Self {
            id: roster.vault_id().clone(),
            me: member.id,
            keys: member.keys,
            keyring,
            roster,
            entries,
            items: store.items()?,
            pinned: false,
        }))
    }

    /// Locks the member's keys and the keyring in memory ([`crate::memory`]).
    pub fn pin(&mut self) -> Result<()> {
        self.keys.pin()?;
        self.keyring.pin()?;
        self.pinned = true;
        Ok(())
    }

    /// Keeps everything in `store`.
    pub fn save(&self, store: &Unlocked) -> Result<()> {
        store.put_member(&self.me, &self.keys)?;
        store.put_keyring(&self.keyring)?;
        store.put_roster(&self.roster, &self.entries)?;
        store.put_items(&self.items)
    }

    /// Keeps the roster and the keyring.
    pub fn save_roster(&self, store: &Unlocked) -> Result<()> {
        store.put_keyring(&self.keyring)?;
        store.put_roster(&self.roster, &self.entries)
    }

    pub fn save_items(&self, store: &Unlocked) -> Result<()> {
        store.put_items(&self.items)
    }

    pub fn id(&self) -> &VaultId {
        &self.id
    }

    pub fn me(&self) -> &MemberId {
        &self.me
    }

    pub fn roster(&self) -> &VerifiedRoster {
        &self.roster
    }

    pub fn generation(&self) -> u32 {
        self.keyring.generation()
    }

    /// The items' revision synced.
    pub fn revision(&self) -> u64 {
        self.items.revision
    }

    /// Checks Oxy's state against the head kept.
    pub fn look(&self, state: &State) -> Result<Look> {
        self.look_from(state, &self.roster.head())
    }

    fn look_from(&self, state: &State, known: &crate::roster::Head) -> Result<Look> {
        if state.vault.is_none() {
            return Ok(Look::Gone);
        }
        let roster = verify(&self.id, state, Some(known))?;
        let Some(member) = roster.member(&self.me) else {
            return Ok(Look::Removed);
        };
        if member.signing_key != self.keys.signing_key() || member.box_key != self.keys.box_key() {
            return Err(Error::Answer("the roster names other keys for this member"));
        }
        if state.own.as_ref() != Some(&self.me) {
            return Ok(Look::Unbound);
        }
        let orphaned = state
            .members
            .iter()
            .filter(|status| status.orphaned && !status.removed && status.id != self.me)
            .filter(|status| roster.member(&status.id).is_some())
            .map(|status| status.id.clone())
            .collect();
        Ok(Look::Member(Box::new(Checked {
            roster,
            entries: state.roster.clone(),
            orphaned,
        })))
    }

    /// Takes a checked roster.
    pub fn adopt(&mut self, checked: Box<Checked>) {
        self.roster = checked.roster;
        self.entries = checked.entries;
    }

    /// Whether the roster moved to a generation this keyring lacks: this member's wrap is read.
    pub fn needs_wrap(&self) -> bool {
        self.roster.generation() > self.keyring.generation()
    }

    /// Takes this member's wrap at the roster's generation: it must be signed by a current member,
    /// addressed to this one, and hold the generations already known unchanged.
    pub fn adopt_wrap(&mut self, wrap: &Wrap) -> Result<()> {
        let mut keyring = wrap.open(
            &self.roster,
            &Principal::Member(self.me.clone()),
            &self.keys,
        )?;
        for generation in 1..=self.keyring.generation() {
            let (Some(old), Some(new)) = (self.keyring.get(generation), keyring.get(generation))
            else {
                return Err(Error::Answer("a keyring without an earlier generation"));
            };
            if !bool::from(old.bytes().ct_eq(new.bytes())) {
                return Err(Error::Answer(
                    "a keyring that changed an earlier generation",
                ));
            }
        }
        if self.pinned {
            keyring.pin()?;
        }
        self.keyring = keyring;
        Ok(())
    }

    /// Takes a page of items Oxy sent after revision `since`.
    pub fn apply_items(&mut self, since: u64, page: &ItemsPage) -> Result<Applied> {
        if page.revision < self.items.revision || since > page.revision {
            return Err(Error::Rollback);
        }
        // More to come must move the cursor on, or a sync would ask for the same page forever.
        if page.more && page.items.is_empty() {
            return Err(Error::Answer("more items promised, none given"));
        }
        let mut last = since;
        for item in &page.items {
            if item.revision <= last || item.revision > page.revision {
                return Err(Error::Answer("items out of order"));
            }
            last = item.revision;
        }
        let mut refused = 0;
        for item in &page.items {
            if self
                .items
                .items
                .get(&item.id)
                .is_some_and(|kept| kept.revision >= item.revision)
            {
                continue;
            }
            match self.take(item) {
                Ok(()) => {}
                Err(_) => refused += 1,
            }
        }
        let next = page.more.then_some(last);
        self.items.revision = if page.more { last } else { page.revision };
        Ok(Applied { refused, next })
    }

    fn take(&mut self, item: &ServerItem) -> Result<()> {
        let sealed = item.sealed()?;
        if let Some(sealed) = &sealed {
            let opened = Item::open(
                &self.id,
                &item.id,
                sealed,
                &self.keyring,
                self.items.counters.get(&item.id).copied(),
            )?;
            let counter = self.items.counters.entry(item.id.clone()).or_insert(0);
            *counter = (*counter).max(opened.counter);
        }
        self.items.items.insert(
            item.id.clone(),
            StoredItem {
                revision: item.revision,
                sealed,
            },
        );
        Ok(())
    }

    /// The live items, each opened (or why it does not open).
    pub fn open_items(&self) -> impl Iterator<Item = (&ItemId, Result<Item>)> {
        self.items.items.iter().filter_map(|(id, kept)| {
            let sealed = kept.sealed.as_ref()?;
            Some((
                id,
                Item::open(
                    &self.id,
                    id,
                    sealed,
                    &self.keyring,
                    self.items.counters.get(id).copied(),
                ),
            ))
        })
    }

    /// One live item, opened.
    pub fn open_item(&self, id: &ItemId) -> Result<Option<Item>> {
        let Some(sealed) = self
            .items
            .items
            .get(id)
            .and_then(|kept| kept.sealed.as_ref())
        else {
            return Ok(None);
        };
        Item::open(
            &self.id,
            id,
            sealed,
            &self.keyring,
            self.items.counters.get(id).copied(),
        )
        .map(Some)
    }

    /// The counter the next write of `id` takes.
    pub fn next_counter(&self, id: &ItemId) -> u64 {
        self.items.counters.get(id).map_or(1, |seen| seen + 1)
    }

    /// The revision Oxy holds `id` at (a tombstone's too), if it has it.
    pub fn base(&self, id: &ItemId) -> Option<u64> {
        self.items.items.get(id).map(|kept| kept.revision)
    }

    /// Seals `item` to be written over what Oxy holds.
    pub fn prepare(&self, item: &Item) -> Result<Prepared> {
        if item.counter < self.next_counter(&item.id) {
            return Err(Error::Rollback);
        }
        Ok(Prepared {
            id: item.id.clone(),
            base: self.base(&item.id),
            sealed: item.seal(&self.id, &self.keyring)?,
            counter: item.counter,
        })
    }

    /// Takes Oxy's answer to a write: the item it holds must be the one sent, at a later revision.
    pub fn written(&mut self, prepared: Prepared, answer: &ServerItem) -> Result<()> {
        if answer.id != prepared.id || answer.revision <= prepared.base.unwrap_or(0) {
            return Err(Error::Answer("another item, or an earlier revision"));
        }
        if answer.sealed()?.as_ref() != Some(&prepared.sealed) {
            return Err(Error::Answer("not the ciphertext sent"));
        }
        let counter = self.items.counters.entry(prepared.id.clone()).or_insert(0);
        *counter = (*counter).max(prepared.counter);
        self.items.items.insert(
            prepared.id,
            StoredItem {
                revision: answer.revision,
                sealed: Some(prepared.sealed),
            },
        );
        Ok(())
    }

    /// Takes Oxy's answer to a deletion of `id` from revision `base`.
    pub fn deleted(&mut self, id: &ItemId, base: u64, answer: &ServerItem) -> Result<()> {
        if answer.id != *id || answer.revision <= base || answer.sealed()?.is_some() {
            return Err(Error::Answer("not the tombstone of the item deleted"));
        }
        self.items.items.insert(
            id.clone(),
            StoredItem {
                revision: answer.revision,
                sealed: None,
            },
        );
        Ok(())
    }

    /// Removes `member` with the vault key's rotation (D3, D4): `remove` and the next
    /// `generation`, signed by this member, and the new keyring sealed to every member left and to
    /// the recovery key.
    pub fn removal(&self, member: &MemberId, created_at: &str) -> Result<Rotation> {
        if *member == self.me || self.roster.member(member).is_none() {
            return Err(Error::Malformed("member (not another current member)"));
        }
        let me = Principal::Member(self.me.clone());
        let remove = self
            .roster
            .next(
                Op::Remove {
                    member_id: member.clone(),
                },
                created_at.to_owned(),
            )
            .sign(&[(me.clone(), &self.keys)])?;
        let head = self.roster.head();
        let generation = EntryBody {
            vault_id: self.id.clone(),
            seq: head.seq + 2,
            prev: Some(remove.hash()),
            created_at: created_at.to_owned(),
            op: Op::Generation {
                generation: self.roster.generation() + 1,
            },
        }
        .sign(&[(me.clone(), &self.keys)])?;
        let entries = vec![remove, generation];
        let roster = self.roster.extend(&entries)?;
        let mut keyring = self.keyring.duplicate()?;
        keyring.rotate()?;
        let mut wraps = Vec::new();
        for member in roster.members() {
            wraps.push(Wrap::create(
                &self.id,
                Principal::Member(member.id.clone()),
                &member.box_key,
                &keyring,
                me.clone(),
                &self.keys,
            )?);
        }
        wraps.push(Wrap::create(
            &self.id,
            Principal::Recovery,
            &roster.recovery().box_key,
            &keyring,
            me,
            &self.keys,
        )?);
        Ok(Rotation {
            entries,
            wraps,
            keyring,
            roster,
        })
    }

    /// Takes Oxy's answer to a rotation appended: its roster must extend the rotation's head, and
    /// the rotated keyring is then this member's.
    pub fn rotated(&mut self, rotation: Rotation, state: &State) -> Result<()> {
        match self.look_from(state, &rotation.roster.head())? {
            Look::Member(checked) => {
                self.keyring = rotation.keyring;
                self.adopt(checked);
                Ok(())
            }
            _ => Err(Error::Answer("this device is no member after its rotation")),
        }
    }

    /// Starts approving `enrollment` (D5): this member's nonce, sent before the device reveals its
    /// own.
    pub fn approving(&self, enrollment: &Enrollment) -> Result<Approving> {
        if enrollment.vault_id != self.id
            || enrollment.status != EnrollmentStatus::Pending
            || enrollment.approver_nonce.is_some()
        {
            return Err(Error::Answer(
                "not an enrollment of this vault waiting for an approver",
            ));
        }
        if self.roster.member(&enrollment.member.id).is_some() {
            return Err(Error::Answer("the device is a member already"));
        }
        Ok(Approving {
            id: enrollment.id.clone(),
            member: enrollment.member.clone(),
            commitment: enrollment.commitment()?,
            nonce: enroll::nonce()?,
            code: None,
        })
    }

    /// The code once the device revealed its nonce, and it opens the commitment it made first.
    pub fn approval_code(
        &self,
        approving: &mut Approving,
        enrollment: &Enrollment,
    ) -> Result<Option<Code>> {
        if enrollment.id != approving.id
            || enrollment.member != approving.member
            || enrollment.commitment()? != approving.commitment
            || enrollment.vault_id != self.id
        {
            return Err(Error::Answer("another enrollment"));
        }
        if enrollment.status != EnrollmentStatus::Pending {
            return Err(Error::Answer("the enrollment was decided"));
        }
        let ours = enrollment
            .approver_nonce()?
            .is_some_and(|nonce| bool::from(nonce.ct_eq(&*approving.nonce)));
        if !ours || enrollment.approver_member_id.as_ref() != Some(&self.me) {
            return Err(Error::Answer("another approver's nonce"));
        }
        let Some(device) = enrollment.device_nonce()? else {
            return Ok(None);
        };
        if !enroll::opens(&approving.commitment, &self.id, &approving.member, &device)? {
            return Err(Error::Answer(
                "the device's nonce does not open its commitment",
            ));
        }
        let code = enroll::code(&self.id, &approving.member, &approving.nonce, &device)?;
        approving.code = Some(code.clone());
        Ok(Some(code))
    }

    /// The `add` entry and the new member's wrap, once the code was shown (the person confirms it
    /// matches before this is sent).
    pub fn approval(&self, approving: &Approving, created_at: &str) -> Result<(Entry, Wrap)> {
        if approving.code.is_none() {
            return Err(Error::Answer("no code shown yet"));
        }
        let me = Principal::Member(self.me.clone());
        let entry = self
            .roster
            .next(
                Op::Add {
                    member: approving.member.clone(),
                },
                created_at.to_owned(),
            )
            .sign(&[(me.clone(), &self.keys)])?;
        let roster = self.roster.extend(std::slice::from_ref(&entry))?;
        let wrap = Wrap::create(
            &self.id,
            Principal::Member(approving.member.id.clone()),
            &approving.member.box_key,
            &self.keyring,
            me,
            &self.keys,
        )?;
        drop(roster);
        Ok((entry, wrap))
    }

    /// Takes Oxy's answer to an approval: the roster must hold the `add` of the device approved.
    pub fn approved(&mut self, approving: &Approving, state: &State) -> Result<()> {
        match self.look(state)? {
            Look::Member(checked) => {
                if checked.roster.member(&approving.member.id) != Some(&approving.member) {
                    return Err(Error::Answer(
                        "the roster does not hold the device approved",
                    ));
                }
                self.adopt(checked);
                Ok(())
            }
            _ => Err(Error::Answer("this device is no member after approving")),
        }
    }

    /// Reused passwords (logins sharing one) and weak ones (zxcvbn's estimate under 10⁸ guesses),
    /// by item id. Items that do not open are left out.
    pub fn audit(&self) -> Audit {
        let mut by_password: HashMap<[u8; 32], Vec<ItemId>> = HashMap::new();
        let mut weak = Vec::new();
        for (id, item) in self.open_items() {
            let Ok(item) = item else { continue };
            let Content::Login(login) = &item.content else {
                continue;
            };
            if login.password.is_empty() {
                continue;
            }
            let digest: [u8; 32] = Sha256::digest(login.password.expose().as_bytes()).into();
            by_password.entry(digest).or_default().push(id.clone());
            let estimate = zxcvbn::zxcvbn(
                login.password.expose(),
                &[login.username.expose(), login.title.expose()],
            );
            if estimate.guesses() < WEAK_GUESSES {
                weak.push(id.clone());
            }
        }
        let mut reused: Vec<Vec<ItemId>> = by_password
            .into_values()
            .filter(|ids| ids.len() > 1)
            .collect();
        reused.sort();
        Audit { reused, weak }
    }
}

/// A password zxcvbn estimates under this many guesses is weak (decision 8).
pub const WEAK_GUESSES: u64 = 100_000_000;

/// What the audit found.
#[derive(Debug, PartialEq, Eq)]
pub struct Audit {
    pub reused: Vec<Vec<ItemId>>,
    pub weak: Vec<ItemId>,
}

/// An approval in progress, on the approving device.
pub struct Approving {
    pub id: EnrollmentId,
    member: Member,
    commitment: [u8; 32],
    nonce: Zeroizing<[u8; 32]>,
    code: Option<Code>,
}

impl fmt::Debug for Approving {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Approving")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Approving {
    /// The nonce this member sends.
    pub fn nonce(&self) -> &[u8; 32] {
        &self.nonce
    }

    pub fn member(&self) -> &Member {
        &self.member
    }

    pub fn code(&self) -> Option<&str> {
        self.code.as_deref().map(String::as_str)
    }
}

/// Makes a new vault (D2, D6): its first member (this machine, named `name`), its recovery key and
/// keyring, the genesis signed by both, and the member's and the recovery wraps.
pub fn create(name: &str, created_at: &str) -> Result<Created> {
    let id = VaultId::random()?;
    let (member, keys) = new_member(name)?;
    let recovery = RecoveryKey::generate()?;
    let recovery_keys = recovery.keys()?;
    let keyring = Keyring::generate()?;
    let genesis = EntryBody::genesis(
        id.clone(),
        member.clone(),
        recovery.public()?,
        created_at.to_owned(),
    )
    .sign(&[
        (Principal::Member(member.id.clone()), &keys),
        (Principal::Recovery, &recovery_keys),
    ])?;
    let roster = VerifiedRoster::verify(&id, std::slice::from_ref(&genesis), None)?;
    let me = Principal::Member(member.id.clone());
    let wraps = vec![
        Wrap::create(
            &id,
            me.clone(),
            &member.box_key,
            &keyring,
            me.clone(),
            &keys,
        )?,
        Wrap::create(
            &id,
            Principal::Recovery,
            &roster.recovery().box_key,
            &keyring,
            me,
            &keys,
        )?,
    ];
    Ok(Created {
        vault: Vault {
            id,
            me: member.id,
            keys,
            keyring,
            roster,
            entries: vec![genesis.clone()],
            items: StoredItems::default(),
            pinned: false,
        },
        recovery,
        genesis,
        wraps,
    })
}

impl Created {
    /// Takes Oxy's answer to the vault made: the same genesis, and this member bound to this
    /// device. The vault is then the machine's.
    pub fn accepted(self, state: &State) -> Result<Vault> {
        match self.vault.look_from(state, &self.vault.roster.head())? {
            Look::Member(checked) => {
                let mut vault = self.vault;
                vault.adopt(checked);
                Ok(vault)
            }
            _ => Err(Error::Answer("the vault made is not this member's")),
        }
    }
}

/// Joining an existing vault by another device's approval (D5), on the joining device.
pub struct Joining {
    vault_id: VaultId,
    roster: VerifiedRoster,
    member: Member,
    keys: KeyPairs,
    device_nonce: Zeroizing<[u8; 32]>,
    commitment: [u8; 32],
    enrollment: Option<EnrollmentId>,
    approver_nonce: Option<[u8; 32]>,
    code: Option<Code>,
}

impl fmt::Debug for Joining {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Joining")
            .field("vault_id", &self.vault_id)
            .field("enrollment", &self.enrollment)
            .finish_non_exhaustive()
    }
}

/// Where a join by approval stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinStep {
    /// No member has started approving yet.
    Waiting,
    /// An approver sent its nonce: this device reveals its own.
    Reveal,
    /// Both nonces are known: the code is shown.
    Code,
    Approved,
    Denied,
}

/// Starts joining the vault Oxy holds, this machine named `name`.
pub fn join(state: &State, name: &str) -> Result<Joining> {
    let summary = state.vault.as_ref().ok_or(Error::Answer("no vault"))?;
    let roster = verify(&summary.id, state, None)?;
    let (member, keys) = new_member(name)?;
    let device_nonce = enroll::nonce()?;
    let commitment = enroll::commitment(&summary.id, &member, &device_nonce)?;
    Ok(Joining {
        vault_id: summary.id.clone(),
        roster,
        member,
        keys,
        device_nonce,
        commitment,
        enrollment: None,
        approver_nonce: None,
        code: None,
    })
}

impl Joining {
    pub fn member(&self) -> &Member {
        &self.member
    }

    pub fn commitment(&self) -> &[u8; 32] {
        &self.commitment
    }

    pub fn device_nonce(&self) -> &[u8; 32] {
        &self.device_nonce
    }

    pub fn enrollment(&self) -> Option<&EnrollmentId> {
        self.enrollment.as_ref()
    }

    pub fn code(&self) -> Option<&str> {
        self.code.as_deref().map(String::as_str)
    }

    /// Locks the new member's keys in memory.
    pub fn pin(&mut self) -> Result<()> {
        self.keys.pin()
    }

    fn check(&self, enrollment: &Enrollment) -> Result<()> {
        let id_matches = self
            .enrollment
            .as_ref()
            .is_none_or(|id| *id == enrollment.id);
        if !id_matches
            || enrollment.vault_id != self.vault_id
            || enrollment.member != self.member
            || enrollment.commitment()? != self.commitment
        {
            return Err(Error::Answer("not this device's enrollment"));
        }
        Ok(())
    }

    /// Takes Oxy's answer to the enrollment asked for.
    pub fn requested(&mut self, enrollment: &Enrollment) -> Result<()> {
        self.check(enrollment)?;
        if enrollment.status != EnrollmentStatus::Pending {
            return Err(Error::Answer("an enrollment already decided"));
        }
        self.enrollment = Some(enrollment.id.clone());
        Ok(())
    }

    /// Where the enrollment stands, as Oxy says. The approver's nonce is taken once, before this
    /// device reveals its own; the code is computed from it once the reveal is Oxy's.
    pub fn progress(&mut self, enrollment: &Enrollment) -> Result<JoinStep> {
        self.check(enrollment)?;
        match enrollment.status {
            EnrollmentStatus::Denied => return Ok(JoinStep::Denied),
            EnrollmentStatus::Approved => return Ok(JoinStep::Approved),
            EnrollmentStatus::Pending => {}
        }
        let Some(approver) = enrollment.approver_nonce()? else {
            return Ok(JoinStep::Waiting);
        };
        match self.approver_nonce {
            Some(taken) if taken != approver => {
                return Err(Error::Answer("the approver's nonce changed"))
            }
            Some(_) => {}
            None => self.approver_nonce = Some(approver),
        }
        match enrollment.device_nonce()? {
            None => Ok(JoinStep::Reveal),
            Some(revealed) if bool::from(revealed.ct_eq(&*self.device_nonce)) => {
                if self.code.is_none() {
                    self.code = Some(enroll::code(
                        &self.vault_id,
                        &self.member,
                        &approver,
                        &self.device_nonce,
                    )?);
                }
                Ok(JoinStep::Code)
            }
            Some(_) => Err(Error::Answer("not this device's nonce")),
        }
    }

    /// Takes the vault once approved: the roster extends the one first seen and holds this member,
    /// bound to this device, and its wrap opens.
    pub fn finish(self, state: &State, wrap: &Wrap) -> Result<Vault> {
        if self.code.is_none() {
            return Err(Error::Answer("approved without a code shown"));
        }
        let roster = verify(&self.vault_id, state, Some(&self.roster.head()))?;
        if roster.member(&self.member.id) != Some(&self.member)
            || state.own.as_ref() != Some(&self.member.id)
        {
            return Err(Error::Answer("the roster does not hold this device"));
        }
        let keyring = wrap.open(
            &roster,
            &Principal::Member(self.member.id.clone()),
            &self.keys,
        )?;
        Ok(Vault {
            id: self.vault_id,
            me: self.member.id,
            keys: self.keys,
            keyring,
            roster,
            entries: state.roster.clone(),
            items: StoredItems::default(),
            pinned: false,
        })
    }
}

/// Joining with the recovery key (D6): this device's `add` and wrap, both signed by the recovery
/// key, once its wrap of the keyring opened.
pub struct RecoveryJoin {
    vault: Vault,
    pub entry: Entry,
    pub wrap: Wrap,
}

impl fmt::Debug for RecoveryJoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecoveryJoin").finish_non_exhaustive()
    }
}

/// Prepares joining the vault Oxy holds with `recovery`, its recovery wrap `recovery_wrap`.
pub fn join_by_recovery(
    state: &State,
    recovery: &RecoveryKey,
    recovery_wrap: &Wrap,
    name: &str,
    created_at: &str,
) -> Result<RecoveryJoin> {
    let summary = state.vault.as_ref().ok_or(Error::Answer("no vault"))?;
    let roster = verify(&summary.id, state, None)?;
    if *roster.recovery() != recovery.public()? {
        return Err(Error::NotThisRecoveryKey);
    }
    let recovery_keys = recovery.keys()?;
    let keyring = recovery_wrap.open(&roster, &Principal::Recovery, &recovery_keys)?;
    let (member, keys) = new_member(name)?;
    let entry = roster
        .next(
            Op::Add {
                member: member.clone(),
            },
            created_at.to_owned(),
        )
        .sign(&[(Principal::Recovery, &recovery_keys)])?;
    let roster = roster.extend(std::slice::from_ref(&entry))?;
    let me = Principal::Member(member.id.clone());
    let wrap = Wrap::create(
        &summary.id,
        me.clone(),
        &member.box_key,
        &keyring,
        Principal::Recovery,
        &recovery_keys,
    )?;
    // What is sent must open as every device will open it.
    wrap.open(&roster, &me, &keys)?;
    let mut entries = state.roster.clone();
    entries.push(entry.clone());
    Ok(RecoveryJoin {
        vault: Vault {
            id: summary.id.clone(),
            me: member.id,
            keys,
            keyring,
            roster,
            entries,
            items: StoredItems::default(),
            pinned: false,
        },
        entry,
        wrap,
    })
}

impl RecoveryJoin {
    /// Locks the new member's keys and the keyring in memory.
    pub fn pin(&mut self) -> Result<()> {
        self.vault.pin()
    }

    /// Takes Oxy's answer to the append: the roster holds this member, bound to this device.
    pub fn accepted(self, state: &State) -> Result<Vault> {
        match self.vault.look(state)? {
            Look::Member(checked) => {
                let mut vault = self.vault;
                vault.adopt(checked);
                Ok(vault)
            }
            _ => Err(Error::Answer("the roster does not hold this device")),
        }
    }
}

impl Created {
    /// Locks the new member's keys, the keyring and the recovery key in memory.
    pub fn pin(&mut self) -> Result<()> {
        self.vault.pin()?;
        self.recovery.pin()
    }
}
