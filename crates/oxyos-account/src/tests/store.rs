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

#[test]
fn the_login_screen_is_listed_each_account_without_its_credential() {
    let dir = TempDir::new("store-list");
    let pictures = dir.path().join("pictures");
    fs::create_dir(&pictures).unwrap();
    let store = Store::open(&dir.path().join("accounts"))
        .unwrap()
        .with_pictures(&pictures);
    assert_eq!(store.list().unwrap(), (Vec::new(), Vec::new()));
    store.put(&enrollment()).unwrap();
    let (listed, broken) = store.list().unwrap();
    assert!(broken.is_empty());
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].account, account());
    assert_eq!(listed[0].user, account().user_name());
    assert_eq!(listed[0].uid, 1001);
    assert_eq!(listed[0].name, "Ada Lovelace");
    assert_eq!(listed[0].picture, None);
    assert!(!format!("{listed:?}").contains(DEVICE_SECRET));
    // The picture is the user's file of the pictures' directory, never a link.
    let picture = pictures.join(account().user_name().as_str());
    std::os::unix::fs::symlink("/etc/shadow", &picture).unwrap();
    assert_eq!(store.list().unwrap().0[0].picture, None);
    fs::remove_file(&picture).unwrap();
    fs::write(&picture, b"png").unwrap();
    assert_eq!(store.list().unwrap().0[0].picture, Some(picture));
    // Without a display name, the username; a file that is no account's is left out and said.
    let mut nameless = enrollment();
    nameless.profile.display_name = Some("  ".into());
    store.put(&nameless).unwrap();
    fs::write(store.directory().join("oxy-other.json"), "{}").unwrap();
    let (listed, broken) = store.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "ada");
    assert_eq!(broken.len(), 1);
    assert!(broken[0].starts_with("oxy-other"));
}
