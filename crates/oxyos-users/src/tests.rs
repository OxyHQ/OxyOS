//! NSS answers as the files the copies this crate replaced read by hand (`/etc/passwd`,
//! `/etc/group`), on every machine where those files are what NSS reads first: a developer's,
//! CI's, the image's (`passwd: files systemd`).

use super::*;

/// `/etc/passwd`'s entries, its seven fields each, as the replaced readers split them.
fn passwd_file() -> Vec<Vec<String>> {
    std::fs::read_to_string("/etc/passwd")
        .expect("/etc/passwd")
        .lines()
        .map(|line| line.split(':').map(str::to_owned).collect::<Vec<_>>())
        .filter(|fields| fields.len() == 7)
        .collect()
}

fn as_user(fields: &[String]) -> User {
    User {
        name: fields[0].clone(),
        uid: fields[2].parse().unwrap(),
        gid: fields[3].parse().unwrap(),
        gecos: fields[4].clone(),
        home: PathBuf::from(&fields[5]),
        shell: PathBuf::from(&fields[6]),
    }
}

#[test]
fn root_is_uid_0() {
    let root = user_named("root").unwrap().expect("root");
    assert_eq!((root.uid, root.gid), (0, 0));
    assert_eq!(user_by_uid(0).unwrap(), Some(root));
}

#[test]
fn this_process_s_user_is_found_by_its_uid_and_its_name() {
    let uid = rustix::process::getuid().as_raw();
    let user = user_by_uid(uid).unwrap().expect("this process's user");
    assert_eq!(user.uid, uid);
    assert_eq!(user_named(&user.name).unwrap(), Some(user));
}

#[test]
fn an_unknown_name_or_uid_is_nobody() {
    assert_eq!(user_named("oxyos-no-such-user").unwrap(), None);
    assert_eq!(user_named("").unwrap(), None);
    assert_eq!(user_named("ro\0ot").unwrap(), None);
    let unused = (60_000..4_000_000_000u32)
        .step_by(7919)
        .find(|uid| !passwd_file().iter().any(|f| f[2] == uid.to_string()))
        .unwrap();
    assert_eq!(user_by_uid(unused).unwrap(), None);
    assert_eq!(group_members("oxyos-no-such-group").unwrap(), None);
}

#[test]
fn every_entry_of_etc_passwd_is_found_as_the_file_says() {
    let file = passwd_file();
    assert!(!file.is_empty());
    for fields in &file {
        let user = as_user(fields);
        // A name or a uid listed twice is answered by its first line, as by the readers this
        // replaced (`find`).
        let first_named = file.iter().find(|f| f[0] == fields[0]).unwrap();
        assert_eq!(
            user_named(&user.name).unwrap(),
            Some(as_user(first_named)),
            "{}",
            user.name
        );
        let first_with_uid = file.iter().find(|f| f[2] == fields[2]).unwrap();
        assert_eq!(
            user_by_uid(user.uid).unwrap(),
            Some(as_user(first_with_uid)),
            "{}",
            user.uid
        );
    }
}

#[test]
fn a_gecos_field_is_whole_commas_and_all() {
    // Every GECOS field as the file holds it, commas and all (`Full Name,Room,Phone,...`).
    for fields in passwd_file() {
        let user = user_named(&fields[0]).unwrap().unwrap();
        if user.name == fields[0] {
            assert_eq!(user.gecos, fields[4]);
        }
    }
}

#[test]
fn every_group_s_members_are_found_as_etc_group_lists_them() {
    let text = std::fs::read_to_string("/etc/group").expect("/etc/group");
    let mut seen = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() != 4 || seen.contains(&fields[0]) {
            continue;
        }
        seen.push(fields[0]);
        let members: Vec<String> = fields[3]
            .split(',')
            .filter(|member| !member.is_empty())
            .map(str::to_owned)
            .collect();
        assert_eq!(
            group_members(fields[0]).unwrap(),
            Some(members),
            "{}",
            fields[0]
        );
    }
}

#[test]
fn a_user_s_groups_start_with_its_own() {
    let root = CString::new("root").unwrap();
    let groups = group_list(&root, 0).unwrap();
    assert_eq!(groups.first(), Some(&0));
}
