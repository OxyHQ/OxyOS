//! Tests across the modules, and the fixtures they share.

mod roster;
mod store;
mod vectors;

use crate::keys::KeyPairs;
use crate::recovery::RecoveryKey;
use crate::roster::{Entry, EntryBody, Member, Op, Platform, Principal, VerifiedRoster};
use crate::wire::{MemberId, VaultId};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) const AT: &str = "2026-11-07T12:00:00.000Z";

/// A member device of a test vault.
pub(crate) struct Device {
    pub id: MemberId,
    pub keys: KeyPairs,
    pub member: Member,
}

impl Device {
    pub fn new(name: &str) -> Self {
        let id = MemberId::random().unwrap();
        let keys = KeyPairs::generate().unwrap();
        let member = Member {
            box_key: keys.box_key(),
            id: id.clone(),
            name: name.into(),
            platform: Platform::Oxyos,
            signing_key: keys.signing_key(),
        };
        Self { id, keys, member }
    }

    pub fn principal(&self) -> Principal {
        Principal::Member(self.id.clone())
    }
}

/// A vault made by `a`, with its recovery key, and the entries appended so far.
pub(crate) struct Vault {
    pub id: VaultId,
    pub a: Device,
    pub recovery: KeyPairs,
    pub entries: Vec<Entry>,
}

impl Vault {
    pub fn new() -> Self {
        let id = VaultId::random().unwrap();
        let a = Device::new("a");
        let recovery = RecoveryKey::generate().unwrap().keys().unwrap();
        let public = crate::recovery::RecoveryPublic {
            box_key: recovery.box_key(),
            signing_key: recovery.signing_key(),
        };
        let genesis = EntryBody::genesis(id.clone(), a.member.clone(), public, AT.into())
            .sign(&[(a.principal(), &a.keys), (Principal::Recovery, &recovery)])
            .unwrap();
        Self {
            id,
            a,
            recovery,
            entries: vec![genesis],
        }
    }

    pub fn roster(&self) -> VerifiedRoster {
        VerifiedRoster::verify(&self.id, &self.entries, None).unwrap()
    }

    /// The next entry for `op`, signed by `signer`.
    pub fn next(&self, op: Op, signer: (Principal, &KeyPairs)) -> Entry {
        self.roster().next(op, AT.into()).sign(&[signer]).unwrap()
    }

    /// Appends the next entry for `op`, signed by `a`, which must verify.
    pub fn append(&mut self, op: Op) {
        let entry = self.next(op, (self.a.principal(), &self.a.keys));
        self.entries.push(entry);
        self.roster();
    }

    /// Adds `device`, approved by `a`.
    pub fn add(&mut self, device: &Device) {
        let a = (self.a.principal(), &self.a.keys);
        let entry = self.next(
            Op::Add {
                member: device.member.clone(),
            },
            a,
        );
        self.entries.push(entry);
    }
}

/// A directory of the test's own, removed when dropped.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "oxyos-keychain-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
