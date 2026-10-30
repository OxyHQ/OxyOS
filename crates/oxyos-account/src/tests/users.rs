//! The system's real `useradd`, `chpasswd` and `userdel`, on a root of the tests' own.

use super::{account, crypt_verifies, Root, TempDir, PASSWORD};
use crate::users::{passwd_entry, verifier_password, SystemUsers, Users, UsersError};
use crate::Secret;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

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

/// Tools that write down how they were run, in place of the system's.
fn recording_tools(dir: &Path) -> SystemUsers {
    let mut users = SystemUsers::new();
    for tool in ["useradd", "chpasswd", "userdel"] {
        let script = dir.join(tool);
        let record = dir.join(format!("{tool}.record"));
        fs::write(
            &script,
            format!(
                "#!/bin/sh\n{{ echo \"args: $*\"; echo \"env: $(env | sort | tr '\\n' ' ')\"; \
                 echo \"stdin: $(cat)\"; }} > '{}'\n",
                record.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
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
