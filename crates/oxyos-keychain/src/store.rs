//! The machine's copy of an account's keychain (OxyOS ADR 0050 decision 1), in a directory of
//! the service's alone (`/var/lib/oxyos/keychain/<user>/`), so the keychain works offline.
//!
//! - `keybag`: the keybag key wrapped by the account's password ([`Keybag`]).
//! - `member`, `keyring`, `roster`, `items`: the member's keys, the keyring, the verified roster
//!   (its entries and its head) and the items' ciphertext as Oxy holds them, each sealed under the
//!   keybag key (XChaCha20-Poly1305, AAD `"oxyos.keychain.store.v1" ‖ 0x00 ‖ name`, so a file
//!   cannot stand in for another).
//!
//! The directory must be the service's own, mode 0700, not a link; each file the service's own,
//! mode 0600, a regular file with one link, opened without following a link. Anything else is
//! refused, never repaired. A file is written whole: to a new file beside it, flushed, renamed
//! over the old one, the directory flushed; a crash leaves the old file or the new one.

use crate::item::SealedItem;
use crate::keybag::{Keybag, KeybagKey};
use crate::keyring::Keyring;
use crate::keys::KeyPairs;
use crate::roster::{Entry, Head};
use crate::seal::{self, NONCE_LENGTH};
use crate::secret;
use crate::wire::{b64_secret, unb64_fixed, unb64_secret, ItemId, MemberId, VaultId};
use crate::{Error, Result, SecretString};
use rustix::fs::OFlags;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Where the service keeps each user's keychain: a directory per user under it.
pub const STATE_DIRECTORY: &str = "/var/lib/oxyos/keychain";

const KEYBAG: &str = "keybag";
const NEW: &str = ".new";
const DOMAIN: &[u8] = b"oxyos.keychain.store.v1";
const VERSION: u32 = 1;

/// The sealed records, by file name.
#[derive(Clone, Copy)]
enum Record {
    Member,
    Keyring,
    Roster,
    Items,
}

impl Record {
    const ALL: [Record; 4] = [
        Record::Member,
        Record::Keyring,
        Record::Roster,
        Record::Items,
    ];

    fn name(self) -> &'static str {
        match self {
            Record::Member => "member",
            Record::Keyring => "keyring",
            Record::Roster => "roster",
            Record::Items => "items",
        }
    }
}

/// A sealed record's file.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sealed {
    ciphertext: String,
    nonce: String,
    v: u32,
}

/// The member's own keys as the store keeps them.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MemberFile {
    box_secret: SecretString,
    id: MemberId,
    signing_seed: SecretString,
}

/// This machine's member: its id in the vault and its keys.
#[derive(Debug)]
pub struct OwnMember {
    pub id: MemberId,
    pub keys: KeyPairs,
}

/// The roster as verified: its entries (re-verified when read) and the head kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredRoster {
    pub entries: Vec<Entry>,
    pub head: Head,
    pub vault_id: VaultId,
}

/// An item as Oxy last gave it: its ciphertext, or none once deleted (a tombstone).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredItem {
    pub revision: u64,
    pub sealed: Option<SealedItem>,
}

/// The items' ciphertext, the vault's revision they are at, and the highest counter seen of
/// each item (kept after a deletion, so an old copy cannot come back).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredItems {
    pub counters: BTreeMap<ItemId, u64>,
    pub items: BTreeMap<ItemId, StoredItem>,
    pub revision: u64,
}

pub struct Store {
    directory: PathBuf,
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("directory", &self.directory)
            .finish()
    }
}

/// The store opened with its keybag key, while the session is unlocked.
pub struct Unlocked {
    directory: PathBuf,
    key: KeybagKey,
}

impl fmt::Debug for Unlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Unlocked")
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

fn euid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// The service's own and no one else's: owned by it, nothing for group or others.
fn private(metadata: &fs::Metadata) -> bool {
    metadata.uid() == euid() && metadata.mode() & 0o077 == 0
}

impl Store {
    /// The store in `directory`, made (mode 0700) if it does not exist; its parent must. Files a
    /// crash left half written are removed.
    pub fn open(directory: &Path) -> Result<Self> {
        match DirBuilder::new().mode(0o700).create(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.is_dir() || !private(&metadata) {
            return Err(Error::Unsafe(directory.to_owned()));
        }
        for entry in fs::read_dir(directory)? {
            let name = entry?.file_name();
            if name.to_string_lossy().ends_with(NEW) {
                fs::remove_file(directory.join(name))?;
            }
        }
        Ok(Self {
            directory: directory.to_owned(),
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Whether the store has a keybag (the keychain was set up on this machine).
    pub fn has_keybag(&self) -> Result<bool> {
        Ok(read(&self.directory, KEYBAG)?.is_some())
    }

    fn keybag(&self) -> Result<Keybag> {
        let bytes = read(&self.directory, KEYBAG)?.ok_or(Error::Malformed("store (no keybag)"))?;
        serde_json::from_slice(&bytes).map_err(|_| Error::Malformed("keybag"))
    }

    /// Makes the keybag, wrapped by `password`, and opens the store with it. Refused if the
    /// store holds anything (a keybag, or records sealed under another one: [`Store::erase`]
    /// first).
    pub fn create(&self, password: &str) -> Result<Unlocked> {
        for name in Record::ALL.map(Record::name).into_iter().chain([KEYBAG]) {
            if fs::symlink_metadata(self.directory.join(name)).is_ok() {
                return Err(Error::Exists);
            }
        }
        let (keybag, key) = Keybag::create(password)?;
        write(&self.directory, KEYBAG, &secret::json(&keybag)?)?;
        Ok(Unlocked {
            directory: self.directory.clone(),
            key,
        })
    }

    /// Opens the store with the account's password.
    pub fn unlock(&self, password: &str) -> Result<Unlocked> {
        let key = self.keybag()?.unlock(password)?;
        Ok(Unlocked {
            directory: self.directory.clone(),
            key,
        })
    }

    /// Re-seals the keybag under `new`, given the `old` password that opens it.
    pub fn change_password(&self, old: &str, new: &str) -> Result<()> {
        let keybag = self.keybag()?.reseal(old, new)?;
        write(&self.directory, KEYBAG, &secret::json(&keybag)?)
    }

    /// Forgets the vault on this machine and keeps the keybag: the member's keys, the keyring,
    /// the roster and the items (the member was removed, or the person turns the keychain off).
    pub fn forget_vault(&self) -> Result<()> {
        for record in Record::ALL {
            remove(&self.directory, record.name())?;
        }
        Ok(())
    }

    /// Forgets the keychain on this machine (it leaves the vault): every file, the keybag last.
    pub fn erase(&self) -> Result<()> {
        for record in Record::ALL {
            remove(&self.directory, record.name())?;
        }
        remove(&self.directory, KEYBAG)
    }
}

fn record_aad(record: Record) -> Vec<u8> {
    [DOMAIN, &[0], record.name().as_bytes()].concat()
}

impl Unlocked {
    /// Forgets the vault and keeps the keybag ([`Store::forget_vault`]).
    pub fn forget_vault(&self) -> Result<()> {
        for record in Record::ALL {
            remove(&self.directory, record.name())?;
        }
        Ok(())
    }

    /// Locks the keybag key in memory ([`crate::memory`]).
    pub fn pin(&mut self) -> Result<()> {
        self.key.pin()
    }

    fn get(&self, record: Record) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let Some(bytes) = read(&self.directory, record.name())? else {
            return Ok(None);
        };
        let sealed: Sealed =
            serde_json::from_slice(&bytes).map_err(|_| Error::Malformed("store record"))?;
        if sealed.v != VERSION {
            return Err(Error::Malformed("store record (version)"));
        }
        let nonce = unb64_fixed::<NONCE_LENGTH>(&sealed.nonce, "store record")?;
        let ciphertext = crate::wire::unb64(&sealed.ciphertext, "store record")?;
        seal::decrypt(self.key.bytes(), &nonce, &ciphertext, &record_aad(record)).map(Some)
    }

    fn put(&self, record: Record, plaintext: &[u8]) -> Result<()> {
        let nonce = seal::nonce()?;
        let ciphertext = seal::encrypt(self.key.bytes(), &nonce, plaintext, &record_aad(record))?;
        let sealed = Sealed {
            ciphertext: crate::wire::b64(&ciphertext),
            nonce: crate::wire::b64(&nonce),
            v: VERSION,
        };
        write(&self.directory, record.name(), &secret::json(&sealed)?)
    }

    /// This machine's member keys, if it has made them.
    pub fn member(&self) -> Result<Option<OwnMember>> {
        let Some(plaintext) = self.get(Record::Member)? else {
            return Ok(None);
        };
        let file: MemberFile =
            serde_json::from_slice(&plaintext).map_err(|_| Error::Malformed("member keys"))?;
        let signing = unb64_secret::<32>(file.signing_seed.expose(), "member keys")?;
        let boxing = unb64_secret::<32>(file.box_secret.expose(), "member keys")?;
        Ok(Some(OwnMember {
            id: file.id,
            keys: KeyPairs::from_secrets(&signing, &boxing),
        }))
    }

    pub fn put_member(&self, id: &MemberId, keys: &KeyPairs) -> Result<()> {
        let (signing, boxing) = keys.secrets();
        let file = MemberFile {
            box_secret: SecretString::from(b64_secret(&*boxing)),
            id: id.clone(),
            signing_seed: SecretString::from(b64_secret(&*signing)),
        };
        self.put(Record::Member, &secret::json(&file)?)
    }

    pub fn keyring(&self) -> Result<Option<Keyring>> {
        self.get(Record::Keyring)?
            .map(|plaintext| Keyring::from_plaintext(&plaintext))
            .transpose()
    }

    pub fn put_keyring(&self, keyring: &Keyring) -> Result<()> {
        self.put(Record::Keyring, &keyring.to_plaintext()?)
    }

    /// The roster kept, its entries verified again from the genesis up to the head kept.
    pub fn roster(&self) -> Result<Option<crate::roster::VerifiedRoster>> {
        let Some(plaintext) = self.get(Record::Roster)? else {
            return Ok(None);
        };
        let stored: StoredRoster =
            serde_json::from_slice(&plaintext).map_err(|_| Error::Malformed("roster"))?;
        let roster = crate::roster::VerifiedRoster::verify(
            &stored.vault_id,
            &stored.entries,
            Some(&stored.head),
        )?;
        if roster.head() != stored.head {
            return Err(Error::Malformed("roster (not at the head kept)"));
        }
        Ok(Some(roster))
    }

    /// The roster kept and its entries, verified again from the genesis up to the head kept.
    pub fn stored_roster(&self) -> Result<Option<(crate::roster::VerifiedRoster, Vec<Entry>)>> {
        let Some(plaintext) = self.get(Record::Roster)? else {
            return Ok(None);
        };
        let stored: StoredRoster =
            serde_json::from_slice(&plaintext).map_err(|_| Error::Malformed("roster"))?;
        let roster = crate::roster::VerifiedRoster::verify(
            &stored.vault_id,
            &stored.entries,
            Some(&stored.head),
        )?;
        if roster.head() != stored.head {
            return Err(Error::Malformed("roster (not at the head kept)"));
        }
        Ok(Some((roster, stored.entries)))
    }

    /// Keeps a verified roster's entries and head.
    pub fn put_roster(
        &self,
        roster: &crate::roster::VerifiedRoster,
        entries: &[Entry],
    ) -> Result<()> {
        let stored = StoredRoster {
            entries: entries.to_vec(),
            head: roster.head(),
            vault_id: roster.vault_id().clone(),
        };
        // What is kept must be what was verified.
        let again = crate::roster::VerifiedRoster::verify(&stored.vault_id, &stored.entries, None)?;
        if again.head() != stored.head {
            return Err(Error::Malformed(
                "roster (entries are not the verified roster's)",
            ));
        }
        self.put(Record::Roster, &secret::json(&stored)?)
    }

    pub fn items(&self) -> Result<StoredItems> {
        match self.get(Record::Items)? {
            Some(plaintext) => {
                serde_json::from_slice(&plaintext).map_err(|_| Error::Malformed("items"))
            }
            None => Ok(StoredItems::default()),
        }
    }

    pub fn put_items(&self, items: &StoredItems) -> Result<()> {
        self.put(Record::Items, &secret::json(items)?)
    }
}

/// Reads one of the store's files, if it exists, refusing one anyone else could reach.
fn read(directory: &Path, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
    let path = directory.join(name);
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::NOFOLLOW | OFlags::CLOEXEC).bits() as i32)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        // ELOOP: the name is a symbolic link.
        Err(error) if error.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error()) => {
            return Err(Error::Unsafe(path))
        }
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 || !private(&metadata) {
        return Err(Error::Unsafe(path));
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(metadata.len() as usize + 1));
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Writes one of the store's files whole, mode 0600, replacing the one it had.
fn write(directory: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let path = directory.join(name);
    let temporary = directory.join(format!("{name}{NEW}"));
    let written = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            // O_EXCL: never through a link, never into a file someone else made.
            .create_new(true)
            .mode(0o600)
            .custom_flags(OFlags::CLOEXEC.bits() as i32)
            .open(&temporary)?;
        // The mode asked for, whatever the umask took from it.
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

fn remove(directory: &Path, name: &str) -> Result<()> {
    match fs::remove_file(directory.join(name)) {
        Ok(()) => Ok(File::open(directory)?.sync_all()?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
