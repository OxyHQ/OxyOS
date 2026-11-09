//! The store: its files and their modes, the keybag, the sealed records. Without root: the
//! service's own user is the test's.

use super::{Device, TempDir, Vault};
use crate::item::{Content, Item, Note};
use crate::keyring::Keyring;
use crate::store::{Store, StoredItem, StoredItems};
use crate::wire::ItemId;
use crate::{Error, Secret};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

const PASSWORD: &str = "correct horse battery staple";

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().mode() & 0o7777
}

fn open_store(dir: &TempDir) -> Store {
    Store::open(&dir.path().join("keychain")).unwrap()
}

#[test]
fn the_directory_is_made_the_services_alone() {
    let dir = TempDir::new("made");
    let store = open_store(&dir);
    assert_eq!(mode(store.directory()), 0o700);
    assert_eq!(
        fs::metadata(store.directory()).unwrap().uid(),
        rustix::process::geteuid().as_raw()
    );
    assert!(!store.has_keybag().unwrap());
}

#[test]
fn everything_kept_is_sealed_and_read_back() {
    let dir = TempDir::new("records");
    let store = open_store(&dir);
    let unlocked = store.create(PASSWORD).unwrap();
    assert!(matches!(store.create(PASSWORD), Err(Error::Exists)));
    assert!(unlocked.member().unwrap().is_none());
    assert!(unlocked.keyring().unwrap().is_none());
    assert!(unlocked.roster().unwrap().is_none());
    assert_eq!(unlocked.items().unwrap(), StoredItems::default());

    let mut vault = Vault::new();
    vault.add(&Device::new("b"));
    let roster = vault.roster();
    let mut keyring = Keyring::generate().unwrap();
    keyring.rotate().unwrap();
    let item = Item {
        id: ItemId::random().unwrap(),
        created_at: super::AT.into(),
        modified_at: super::AT.into(),
        counter: 3,
        content: Content::Note(Note {
            title: Secret::new("door"),
            body: Secret::new("4711"),
        }),
    };
    let sealed = item.seal(&vault.id, &keyring).unwrap();
    let mut items = StoredItems {
        revision: 9,
        ..StoredItems::default()
    };
    items.counters.insert(item.id.clone(), 3);
    items.items.insert(
        item.id.clone(),
        StoredItem {
            revision: 9,
            sealed: Some(sealed.clone()),
        },
    );

    unlocked.put_member(&vault.a.id, &vault.a.keys).unwrap();
    unlocked.put_keyring(&keyring).unwrap();
    unlocked.put_roster(&roster, &vault.entries).unwrap();
    unlocked.put_items(&items).unwrap();

    // Every file is the service's alone, and none holds anything in the clear.
    for name in ["keybag", "member", "keyring", "roster", "items"] {
        let path = store.directory().join(name);
        assert_eq!(mode(&path), 0o600, "{name}");
        let bytes = fs::read_to_string(&path).unwrap();
        assert!(!bytes.contains(vault.id.as_str()), "{name}");
        assert!(!bytes.contains(PASSWORD), "{name}");
    }

    // Read back after locking and unlocking again.
    drop(unlocked);
    let unlocked = store.unlock(PASSWORD).unwrap();
    let member = unlocked.member().unwrap().unwrap();
    assert_eq!(member.id, vault.a.id);
    assert_eq!(member.keys.signing_key(), vault.a.keys.signing_key());
    assert_eq!(member.keys.box_key(), vault.a.keys.box_key());
    let read = unlocked.keyring().unwrap().unwrap();
    assert_eq!(read.generation(), 2);
    assert_eq!(
        read.get(1).unwrap().bytes(),
        keyring.get(1).unwrap().bytes()
    );
    let kept = unlocked.roster().unwrap().unwrap();
    assert_eq!(kept.head(), roster.head());
    assert_eq!(unlocked.items().unwrap(), items);
    let opened = Item::open(&vault.id, &item.id, &sealed, &read, Some(3)).unwrap();
    assert_eq!(opened.counter, 3);
}

#[test]
fn the_keybag_opens_with_the_password_alone_and_follows_it() {
    let dir = TempDir::new("password");
    let store = open_store(&dir);
    let unlocked = store.create(PASSWORD).unwrap();
    let keyring = Keyring::generate().unwrap();
    unlocked.put_keyring(&keyring).unwrap();
    assert!(matches!(store.unlock("wrong"), Err(Error::WrongPassword)));
    assert!(matches!(store.unlock(""), Err(Error::WrongPassword)));
    assert!(matches!(
        store.change_password("wrong", "new password"),
        Err(Error::WrongPassword)
    ));
    store.change_password(PASSWORD, "new password").unwrap();
    assert!(matches!(store.unlock(PASSWORD), Err(Error::WrongPassword)));
    let unlocked = store.unlock("new password").unwrap();
    // The records are under the same keybag key: still readable.
    let read = unlocked.keyring().unwrap().unwrap();
    assert_eq!(
        read.get(1).unwrap().bytes(),
        keyring.get(1).unwrap().bytes()
    );
    assert_eq!(mode(&store.directory().join("keybag")), 0o600);
}

#[test]
fn a_record_cannot_stand_in_for_another_or_be_changed() {
    let dir = TempDir::new("swap");
    let store = open_store(&dir);
    let unlocked = store.create(PASSWORD).unwrap();
    unlocked.put_keyring(&Keyring::generate().unwrap()).unwrap();
    unlocked.put_items(&StoredItems::default()).unwrap();
    let directory = store.directory();
    fs::copy(directory.join("keyring"), directory.join("items.copy")).unwrap();
    fs::rename(directory.join("items.copy"), directory.join("items")).unwrap();
    fs::set_permissions(directory.join("items"), fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(unlocked.items(), Err(Error::Decrypt)));
    // A byte of the ciphertext changed.
    let text = fs::read_to_string(directory.join("keyring")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut ciphertext = crate::wire::unb64(value["ciphertext"].as_str().unwrap(), "t").unwrap();
    ciphertext[0] ^= 1;
    let changed = serde_json::json!({
        "ciphertext": crate::wire::b64(&ciphertext),
        "nonce": value["nonce"],
        "v": 1,
    });
    fs::write(directory.join("keyring"), changed.to_string()).unwrap();
    assert!(matches!(unlocked.keyring(), Err(Error::Decrypt)));
    // Another store's keybag key opens none of this one's records.
    let other = TempDir::new("swap-other");
    let other = open_store(&other);
    let foreign = other.create(PASSWORD).unwrap();
    foreign.put_items(&StoredItems::default()).unwrap();
    fs::copy(other.directory().join("items"), directory.join("items")).unwrap();
    assert!(matches!(unlocked.items(), Err(Error::Decrypt)));
}

#[test]
fn a_roster_is_kept_only_as_verified() {
    let dir = TempDir::new("roster");
    let store = open_store(&dir);
    let unlocked = store.create(PASSWORD).unwrap();
    let mut vault = Vault::new();
    let roster = vault.roster();
    vault.add(&Device::new("b"));
    // Entries that are not the verified roster's.
    assert!(unlocked.put_roster(&roster, &vault.entries).is_err());
    assert!(unlocked.put_roster(&roster, &vault.entries[..1]).is_ok());
}

#[test]
fn refuses_a_directory_others_can_reach_or_a_link() {
    let dir = TempDir::new("unsafe-dir");
    let path = dir.path().join("keychain");
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(matches!(Store::open(&path), Err(Error::Unsafe(_))));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o701)).unwrap();
    assert!(matches!(Store::open(&path), Err(Error::Unsafe(_))));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(Store::open(&path).is_ok());
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(matches!(Store::open(&link), Err(Error::Unsafe(_))));
    let file = dir.path().join("file");
    fs::write(&file, b"").unwrap();
    assert!(Store::open(&file).is_err());
}

#[test]
fn refuses_a_file_others_can_read_a_link_or_a_second_name() {
    let dir = TempDir::new("unsafe-file");
    let store = open_store(&dir);
    store.create(PASSWORD).unwrap();
    let keybag = store.directory().join("keybag");
    for unsafe_mode in [0o640, 0o604, 0o644] {
        fs::set_permissions(&keybag, fs::Permissions::from_mode(unsafe_mode)).unwrap();
        assert!(matches!(store.unlock(PASSWORD), Err(Error::Unsafe(_))));
    }
    fs::set_permissions(&keybag, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(store.unlock(PASSWORD).is_ok());
    // A second name for it, elsewhere.
    let second = dir.path().join("second");
    fs::hard_link(&keybag, &second).unwrap();
    assert!(matches!(store.unlock(PASSWORD), Err(Error::Unsafe(_))));
    fs::remove_file(&second).unwrap();
    // A link in its place.
    let moved = dir.path().join("moved");
    fs::rename(&keybag, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &keybag).unwrap();
    assert!(matches!(store.unlock(PASSWORD), Err(Error::Unsafe(_))));
    assert!(matches!(store.has_keybag(), Err(Error::Unsafe(_))));
}

#[test]
fn a_half_written_file_is_cleared_and_erasing_forgets_everything() {
    let dir = TempDir::new("erase");
    let store = open_store(&dir);
    let unlocked = store.create(PASSWORD).unwrap();
    unlocked.put_keyring(&Keyring::generate().unwrap()).unwrap();
    fs::write(store.directory().join("keyring.new"), b"half").unwrap();
    let store = Store::open(store.directory()).unwrap();
    assert!(!store.directory().join("keyring.new").exists());
    // Records left from another keybag: no new keybag over them until they are erased.
    fs::remove_file(store.directory().join("keybag")).unwrap();
    assert!(matches!(store.create(PASSWORD), Err(Error::Exists)));
    store.erase().unwrap();
    assert_eq!(fs::read_dir(store.directory()).unwrap().count(), 0);
    assert!(!store.has_keybag().unwrap());
    assert!(store.unlock(PASSWORD).is_err());
    assert!(store.create(PASSWORD).is_ok());
}

#[test]
fn debug_shows_no_key() {
    let dir = TempDir::new("debug");
    let store = open_store(&dir);
    let unlocked = store.create(PASSWORD).unwrap();
    let vault = Vault::new();
    unlocked.put_member(&vault.a.id, &vault.a.keys).unwrap();
    let member = unlocked.member().unwrap().unwrap();
    let (signing, boxing) = member.keys.secrets();
    let shown = format!("{store:?} {unlocked:?} {member:?}");
    assert!(!shown.contains(PASSWORD));
    for secret in [&*signing, &*boxing] {
        assert!(!shown.contains(&crate::wire::b64(secret)));
        assert!(!shown.contains(&format!("{:?}", &secret[..4])));
    }
}
