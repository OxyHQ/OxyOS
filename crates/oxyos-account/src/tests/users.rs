//! The system's real `useradd`, `chpasswd` and `userdel`, on a root of the tests' own.

use super::{account, crypt_verifies, Root, TempDir, PASSWORD};
use crate::users::{
    gecos_full_name, group_members, passwd_entry, passwd_gecos, verifier_password, SystemUsers,
    UnixUser, Users, UsersError, ADMINISTRATORS, MAX_FULL_NAME_CHARS,
};
use crate::Secret;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[test]
fn a_user_is_made_with_its_own_group_and_a_home_only_it_opens() {
    if !super::passwd_tools() {
        return;
    }
    let root = Root::new("users-create");
    let users = SystemUsers::with_prefix(root.path());
    let name = account().user_name();
    assert_eq!(users.lookup(&name), Ok(None));
    let user = users.create(&name).unwrap();
    assert!(user.uid >= 1000, "an ordinary user: {}", user.uid);
    assert_eq!(users.lookup(&name), Ok(Some(user.clone())));
    assert!(root
        .read("group")
        .lines()
        .any(|line| line.starts_with(&format!("{name}:x:{}:", user.gid))));
    assert_eq!(user.home, Path::new("/home").join(name.as_str()));
    let home = root.path().join("home").join(name.as_str());
    assert_eq!(fs::metadata(&home).unwrap().mode() & 0o777, 0o700);
    // Locked until its verifier is set: nothing signs in as it.
    assert!(root.shadow(name.as_str()).unwrap().starts_with('!'));
}

#[test]
fn the_verifier_is_a_yescrypt_hash_of_the_password_never_the_password() {
    if !super::passwd_tools() {
        return;
    }
    let root = Root::new("users-verifier");
    let users = SystemUsers::with_prefix(root.path());
    let name = account().user_name();
    users.create(&name).unwrap();
    users.set_verifier(&name, &Secret::new(PASSWORD)).unwrap();
    let verifier = root.shadow(name.as_str()).unwrap();
    assert!(verifier.starts_with("$y$"), "{verifier}");
    assert!(crypt_verifies(PASSWORD, &verifier));
    assert!(!crypt_verifies("correct horse battery", &verifier));
    for file in ["passwd", "shadow", "group", "gshadow"] {
        assert!(!root.read(file).contains(PASSWORD));
    }
    // A new one replaces it.
    users
        .set_verifier(&name, &Secret::new("another password: with a colon"))
        .unwrap();
    let replaced = root.shadow(name.as_str()).unwrap();
    assert!(crypt_verifies("another password: with a colon", &replaced));
    assert!(!crypt_verifies(PASSWORD, &replaced));
}

#[test]
fn a_name_in_use_is_not_made_again() {
    if !super::passwd_tools() {
        return;
    }
    let root = Root::new("users-twice");
    let users = SystemUsers::with_prefix(root.path());
    let name = account().user_name();
    users.create(&name).unwrap();
    assert_eq!(users.create(&name), Err(UsersError::Exists));
}

#[test]
fn removing_a_user_takes_its_group_and_home() {
    if !super::passwd_tools() {
        return;
    }
    let root = Root::new("users-remove");
    let users = SystemUsers::with_prefix(root.path());
    let name = account().user_name();
    users.create(&name).unwrap();
    users.set_verifier(&name, &Secret::new(PASSWORD)).unwrap();
    users.remove(&name).unwrap();
    assert_eq!(users.lookup(&name), Ok(None));
    for file in ["passwd", "shadow", "group", "gshadow"] {
        assert!(!root.read(file).contains(name.as_str()), "{file}");
    }
    assert!(!root.path().join("home").join(name.as_str()).exists());
    assert_eq!(users.remove(&name), Err(UsersError::Missing));
}

#[test]
fn only_a_password_pam_can_check_becomes_a_verifier() {
    assert!(verifier_password(&Secret::new(PASSWORD)));
    assert!(verifier_password(&Secret::new("ñandú: contraseña 🔑")));
    assert!(verifier_password(&Secret::new("x".repeat(512))));
    assert!(!verifier_password(&Secret::new("")));
    assert!(!verifier_password(&Secret::new("x".repeat(513))));
    assert!(!verifier_password(&Secret::new("two\nlines")));
    assert!(!verifier_password(&Secret::new("carriage\rreturn")));
    assert!(!verifier_password(&Secret::new("nul\0byte")));
}

/// Tools that write down how they were run, in place of the system's: links to the committed
/// `record-tool`. A script written by the test itself could not be run reliably: a process another
/// test thread forks while the file is open for writing keeps it open until it execs, and running
/// the script meanwhile fails (`ETXTBSY`). A link is made without opening anything.
fn recording_tools(dir: &Path) -> SystemUsers {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tests/record-tool");
    let mut users = SystemUsers::new();
    for tool in ["useradd", "chpasswd", "userdel"] {
        std::os::unix::fs::symlink(&fixture, dir.join(tool)).unwrap();
    }
    users.useradd = dir.join("useradd");
    users.chpasswd = dir.join("chpasswd");
    users.userdel = dir.join("userdel");
    users
}

#[test]
fn the_password_goes_on_standard_input_never_in_the_arguments_or_environment() {
    let dir = TempDir::new("users-argv");
    let users = recording_tools(dir.path());
    let name = account().user_name();
    users.set_verifier(&name, &Secret::new(PASSWORD)).unwrap();
    let record = fs::read_to_string(dir.path().join("chpasswd.record")).unwrap();
    let mut lines = record.lines();
    let args = lines.next().unwrap();
    let env = lines.next().unwrap();
    let stdin = lines.next().unwrap();
    assert_eq!(args, "args: ");
    assert!(!args.contains(PASSWORD) && !env.contains(PASSWORD));
    assert_eq!(stdin, format!("stdin: {name}:{PASSWORD}"));
    // A clean environment: nothing the service inherited reaches the tool.
    assert!(env.contains("LC_ALL=C"), "{env}");
    assert!(!env.contains("HOME="), "{env}");
}

#[test]
fn the_tools_are_given_an_argument_vector() {
    let dir = TempDir::new("users-args");
    let users = recording_tools(dir.path());
    let name = account().user_name();
    // The recording useradd makes no user: the lookup after it says so.
    assert!(users.create(&name).is_err());
    let useradd = fs::read_to_string(dir.path().join("useradd.record")).unwrap();
    assert!(useradd.starts_with(&format!("args: --create-home --user-group -- {name}\n")));
    users.remove(&name).unwrap();
    let userdel = fs::read_to_string(dir.path().join("userdel.record")).unwrap();
    assert!(userdel.starts_with(&format!("args: --remove -- {name}\n")));
}

#[test]
fn a_passwd_entry_is_read_as_passwd_5_says() {
    let dir = TempDir::new("users-passwd");
    let passwd = dir.path().join("passwd");
    fs::write(
        &passwd,
        "root:x:0:0:root:/root:/bin/sh\noxyos-login:x:990:990::/nonexistent:/usr/sbin/nologin\n\
         broken:x:nope:1::/:/bin/sh\n",
    )
    .unwrap();
    let login = passwd_entry(&passwd, "oxyos-login").unwrap().unwrap();
    assert_eq!((login.uid, login.gid), (990, 990));
    assert_eq!(passwd_entry(&passwd, "oxyos").unwrap(), None);
    assert_eq!(passwd_entry(&passwd, "oxyos-logi").unwrap(), None);
    assert!(passwd_entry(&passwd, "broken").is_err());
    assert!(passwd_entry(&dir.path().join("missing"), "root").is_err());
}

#[test]
fn an_administrator_is_a_member_of_sudo_until_it_is_removed() {
    if !super::passwd_tools() {
        return;
    }
    let root = Root::new("users-administrator");
    let users = SystemUsers::with_prefix(root.path());
    let name = account().user_name();
    users.create(&name).unwrap();
    assert_eq!(users.is_administrator(&name), Ok(false));
    users.make_administrator(&name).unwrap();
    assert_eq!(users.is_administrator(&name), Ok(true));
    assert!(root
        .read("group")
        .lines()
        .any(|line| line == format!("{ADMINISTRATORS}:x:27:{name}")));
    // userdel takes it out of the group.
    users.remove(&name).unwrap();
    assert_eq!(users.is_administrator(&name), Ok(false));
    assert_eq!(users.make_administrator(&name), Err(UsersError::Missing));
}

#[test]
fn a_groups_members_are_read_from_its_fourth_field() {
    let dir = TempDir::new("group-file");
    let group = dir.path().join("group");
    fs::write(
        &group,
        "root:x:0:\nsudo:x:27:oxy-a,oxy-b\nusers:x:100:\nbroken:x\n",
    )
    .unwrap();
    assert_eq!(
        group_members(&group, "sudo"),
        Ok(Some(vec!["oxy-a".into(), "oxy-b".into()]))
    );
    assert_eq!(group_members(&group, "users"), Ok(Some(vec![])));
    assert_eq!(group_members(&group, "wheel"), Ok(None));
    assert!(group_members(&group, "broken").is_err());
    assert!(group_members(&dir.path().join("missing"), "sudo").is_err());
}

#[test]
fn a_new_homes_file_is_written_inside_it_and_never_through_a_link() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new("seed");
    let home = root.path().join("home/oxy-seed");
    std::fs::create_dir_all(&home).unwrap();
    let users = SystemUsers::with_prefix(root.path());
    let user = UnixUser {
        uid: 1000,
        gid: 1000,
        home: PathBuf::from("/home/oxy-seed"),
    };
    let file = Path::new(".config/oxyos/preferences");
    users
        .seed_home(&user, file, b"appearance = dark\n")
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(home.join(file)).unwrap(),
        "appearance = dark\n"
    );
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&home.join(".config")), 0o755);
    assert_eq!(mode(&home.join(".config/oxyos")), 0o755);
    assert_eq!(mode(&home.join(file)), 0o644);
    // A link where a directory is expected is not followed out of the home.
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let linked = root.path().join("home/oxy-linked");
    std::fs::create_dir_all(&linked).unwrap();
    std::os::unix::fs::symlink(&elsewhere, linked.join(".config")).unwrap();
    let user = UnixUser {
        home: PathBuf::from("/home/oxy-linked"),
        ..user
    };
    assert!(users
        .seed_home(&user, file, b"appearance = dark\n")
        .is_err());
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
    // Nothing outside it is named.
    assert!(users.seed_home(&user, Path::new("../escape"), b"").is_err());
    assert!(users.seed_home(&user, Path::new("/etc/x"), b"").is_err());
}

#[test]
fn a_full_name_is_what_gecos_can_hold() {
    assert_eq!(gecos_full_name("Ada Lovelace"), "Ada Lovelace");
    assert_eq!(
        gecos_full_name("  Ada,  Countess:\nof\tLovelace "),
        "Ada Countess of Lovelace"
    );
    assert_eq!(gecos_full_name("José Núñez 李"), "José Núñez 李");
    assert_eq!(gecos_full_name(" , : "), "");
    let long = "é".repeat(MAX_FULL_NAME_CHARS + 10);
    assert_eq!(gecos_full_name(&long).chars().count(), MAX_FULL_NAME_CHARS);
}

#[test]
fn a_users_full_name_is_written_with_usermod_once() {
    if !super::passwd_tools() {
        return;
    }
    let root = Root::new("users-full-name");
    let users = SystemUsers::with_prefix(root.path());
    let name = account().user_name();
    assert_eq!(users.set_full_name(&name, "Ada"), Err(UsersError::Missing));
    let made = users.create(&name).unwrap();
    let passwd = root.path().join("etc/passwd");
    assert_eq!(
        passwd_gecos(&passwd, name.as_str()),
        Ok(Some(String::new()))
    );
    assert_eq!(
        users.set_full_name(&name, "Ada, Countess: Lovelace"),
        Ok(true)
    );
    assert_eq!(
        passwd_gecos(&passwd, name.as_str()),
        Ok(Some("Ada Countess Lovelace".into()))
    );
    // The entry is otherwise as it was.
    assert_eq!(passwd_entry(&passwd, name.as_str()), Ok(Some(made)));
    // The same name is not written again; another is.
    assert_eq!(
        users.set_full_name(&name, "Ada Countess Lovelace"),
        Ok(false)
    );
    assert_eq!(users.set_full_name(&name, "José Núñez"), Ok(true));
    assert_eq!(
        passwd_gecos(&passwd, name.as_str()),
        Ok(Some("José Núñez".into()))
    );
}
