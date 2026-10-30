use super::{account, credential, TempDir, DEVICE_SECRET};
use crate::api::Profile;
use crate::store::{Enrollment, Store};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

fn enrollment() -> Enrollment {
    Enrollment {
        account: account(),
        user: account().user_name(),
        uid: 1001,
        profile: Profile {
            id: account().as_str().into(),
            username: Some("ada".into()),
            display_name: Some("Ada Lovelace".into()),
            avatar: None,
        },
        credential: credential(),
    }
}

fn mode(path: &std::path::Path) -> u32 {
    fs::symlink_metadata(path).unwrap().mode() & 0o7777
}

#[test]
fn the_directory_is_made_the_services_alone() {
    let dir = TempDir::new("store-made");
    let path = dir.path().join("accounts");
    Store::open(&path).unwrap();
    assert_eq!(mode(&path), 0o700);
    assert_eq!(
        fs::metadata(&path).unwrap().uid(),
        rustix::process::geteuid().as_raw()
    );
}

#[test]
fn an_enrollment_is_kept_in_a_file_only_the_service_reads() {
    let dir = TempDir::new("store-file");
    let store = Store::open(&dir.path().join("accounts")).unwrap();
    assert_eq!(store.get(&account()).unwrap(), None);
    store.put(&enrollment()).unwrap();
    assert_eq!(store.get(&account()).unwrap(), Some(enrollment()));
    let file = store
        .directory()
        .join(format!("{}.json", account().user_name()));
    assert_eq!(mode(&file), 0o600);
    // Nothing but the file: no temporary left beside it.
    assert_eq!(fs::read_dir(store.directory()).unwrap().count(), 1);
    assert_eq!(
        store.users().unwrap(),
        vec![account().user_name().to_string()]
    );
}

#[test]
fn a_file_is_replaced_whole() {
    let dir = TempDir::new("store-replace");
    let store = Store::open(&dir.path().join("accounts")).unwrap();
    store.put(&enrollment()).unwrap();
    let mut changed = enrollment();
    changed.credential.device_secret = crate::Secret::new("another-secret");
    store.put(&changed).unwrap();
    assert_eq!(store.get(&account()).unwrap(), Some(changed));
    assert_eq!(fs::read_dir(store.directory()).unwrap().count(), 1);
    store.remove(&account().user_name()).unwrap();
    assert_eq!(store.get(&account()).unwrap(), None);
    assert_eq!(fs::read_dir(store.directory()).unwrap().count(), 0);
}

#[test]
fn a_half_written_file_from_a_crash_is_removed() {
    let dir = TempDir::new("store-crash");
    let path = dir.path().join("accounts");
    let store = Store::open(&path).unwrap();
    store.put(&enrollment()).unwrap();
    let half = path.join(format!("{}.json.new", account().user_name()));
    fs::write(&half, "{\"version\":").unwrap();
    let store = Store::open(&path).unwrap();
    assert!(!half.exists());
    assert_eq!(store.get(&account()).unwrap(), Some(enrollment()));
}

#[test]
fn a_directory_others_can_read_is_refused_not_repaired() {
    let dir = TempDir::new("store-open");
    let path = dir.path().join("accounts");
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(mode(&path), 0o755);
}

#[test]
fn a_link_is_no_directory_of_the_services() {
    let dir = TempDir::new("store-link");
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).unwrap();
    let link = dir.path().join("accounts");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(Store::open(&link).is_err());
}

#[test]
fn another_accounts_file_is_not_this_ones() {
    let dir = TempDir::new("store-other");
    let store = Store::open(&dir.path().join("accounts")).unwrap();
    store.put(&enrollment()).unwrap();
    let other = crate::AccountId::parse(super::OTHER_ACCOUNT).unwrap();
    fs::rename(
        store
            .directory()
            .join(format!("{}.json", account().user_name())),
        store
            .directory()
            .join(format!("{}.json", other.user_name())),
    )
    .unwrap();
    assert!(store.get(&other).is_err());
}

#[test]
fn the_secret_is_never_printed() {
    let printed = format!("{:?}", enrollment());
    assert!(!printed.contains(DEVICE_SECRET), "{printed}");
    assert!(printed.contains("Secret(…)"));
}
