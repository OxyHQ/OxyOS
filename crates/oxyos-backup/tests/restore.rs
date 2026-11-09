//! Restoring a whole computer (docs/adr/0059-backups.mdx, decision 7), end to end on roots of the
//! test's own: a machine's homes, pictures, Wi-Fi networks and manifest backed up with Debian's
//! restic onto a played disk, its computer found by its backup password, and restored into another
//! root, whose users are made again with the system's own passwd tools (`--prefix`) and whose
//! accounts the login screen then lists as `signInAgain`.
//!
//! restic must be installed, and the passwd tools must work on a root of the tests' own (Debian
//! trixie's shadow, as the account crate's tests need): without either these tests say so and
//! pass, unless `OXYOS_BACKUP_ENGINE=required` or `OXYOS_ACCOUNT_PASSWD_TOOLS=required` (the Debian
//! job of CI), where their absence fails them. Run as root (CI's container, a developer's
//! `docker run`), the files' owners are checked too: the backup's files are given the manifest's
//! uids, and must come back with them.

use oxyos_account::store::{State, Store as Accounts, ADMINISTRATOR_MADE};
use oxyos_account::AccountId;
use oxyos_backup::disks::Played;
use oxyos_backup::engine::{Backup, Engine, Location, Repository, Stop};
use oxyos_backup::machine::{self, Manifest};
use oxyos_backup::migrate::Migration;
use oxyos_backup::restore::{Refusal, Restorer, Source, Step};
use oxyos_service_socket::Connection;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use zeroize::Zeroizing;

const ADA: &str = "65a1b2c3d4e5f60718293a4b";
const BEA: &str = "019a5b6c-7d8e-7f90-a1b2-c3d4e5f60718";
const PASSWORD: &str = "correct horse battery";
const UUID: &str = "1234-ABCD";

fn engine(cache: &Path) -> Option<Engine> {
    match Engine::from_path(cache) {
        Some(engine) => Some(engine),
        None if std::env::var("OXYOS_BACKUP_ENGINE").as_deref() == Ok("required") => {
            panic!("restic is not installed, and OXYOS_BACKUP_ENGINE=required")
        }
        None => {
            eprintln!("skipped: restic is not installed (apt install restic)");
            None
        }
    }
}

/// A root for the passwd tools, as Debian's base system has its account database.
fn system_root(path: &Path) {
    let etc = path.join("etc");
    fs::create_dir_all(&etc).unwrap();
    fs::create_dir_all(path.join("home")).unwrap();
    fs::write(etc.join("passwd"), "root:x:0:0:root:/root:/bin/sh\n").unwrap();
    fs::write(etc.join("shadow"), "root:*:20000:0:99999:7:::\n").unwrap();
    fs::write(etc.join("group"), "root:x:0:\nsudo:x:27:\n").unwrap();
    fs::write(etc.join("gshadow"), "root:*::\nsudo:*::\n").unwrap();
    fs::write(
        etc.join("login.defs"),
        "UID_MIN 1000\nUID_MAX 60000\nGID_MIN 1000\nGID_MAX 60000\nHOME_MODE 0700\n\
         USERGROUPS_ENAB yes\nENCRYPT_METHOD YESCRYPT\n",
    )
    .unwrap();
}

/// Whether the system's groupadd and useradd make a group and a user on a root of the test's
/// own, as Debian trixie's do.
fn passwd_tools(dir: &Path) -> bool {
    let root = dir.join("probe");
    system_root(&root);
    let ran = |tool: &str, arguments: &[&str]| {
        Command::new(tool)
            .env_clear()
            .arg("--prefix")
            .arg(&root)
            .args(arguments)
            .output()
            .map_err(|e| format!("{tool}: {e}"))
            .and_then(|o| {
                let said = String::from_utf8_lossy(&o.stderr).into_owned();
                if !o.status.success() || said.contains("lastlog") {
                    Err(format!("{tool} ({}): {said}", o.status))
                } else {
                    Ok(())
                }
            })
    };
    let probe = ran("/usr/sbin/groupadd", &["--gid", "1500", "--", "oxy-probe"]).and_then(|()| {
        ran(
            "/usr/sbin/useradd",
            &[
                "--no-create-home",
                "--uid",
                "1500",
                "--gid",
                "1500",
                "--",
                "oxy-probe",
            ],
        )
    });
    match probe {
        Ok(()) => true,
        Err(why) if std::env::var("OXYOS_ACCOUNT_PASSWD_TOOLS").as_deref() == Ok("required") => {
            panic!("OXYOS_ACCOUNT_PASSWD_TOOLS=required, and the passwd tools cannot run on a root of the tests' own: {why}")
        }
        Err(why) => {
            eprintln!("skipped: the passwd tools cannot run on a root of the tests' own: {why}");
            false
        }
    }
}

fn as_root() -> bool {
    rustix::process::geteuid().is_root()
}

fn user(account: &str) -> String {
    AccountId::parse(account)
        .unwrap()
        .user_name()
        .as_str()
        .to_owned()
}

/// What a person's home holds: folders, a link, an empty file, a private file, other scripts.
fn home(path: &Path, words: &str) {
    fs::create_dir_all(path.join("Documents/Taxes 2026")).unwrap();
    fs::create_dir_all(path.join("Pictures/Ñandú")).unwrap();
    fs::write(path.join("Documents/notes.txt"), words).unwrap();
    fs::write(
        path.join("Documents/Taxes 2026/return.pdf"),
        b"%PDF-1.7 \x00\x01",
    )
    .unwrap();
    fs::write(path.join("Pictures/Ñandú/写真.jpg"), [7u8; 4096]).unwrap();
    fs::write(path.join("empty"), b"").unwrap();
    std::os::unix::fs::symlink("Documents/notes.txt", path.join("notes")).unwrap();
    fs::write(path.join("Documents/private"), "mine").unwrap();
    fs::set_permissions(
        path.join("Documents/private"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut items = Vec::new();
    for entry in fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        items.push(path.clone());
        if entry.file_type().unwrap().is_dir() {
            items.extend(walk(&path));
        }
    }
    items
}

/// Whether two trees hold the same items, bytes, links, modes and (as root) owners.
fn same(a: &Path, b: &Path) {
    let relative = |root: &Path| {
        let mut items: Vec<PathBuf> = walk(root)
            .into_iter()
            .map(|p| p.strip_prefix(root).unwrap().to_owned())
            .collect();
        items.sort();
        items
    };
    let items = relative(a);
    assert_eq!(
        items,
        relative(b),
        "{} and {} differ",
        a.display(),
        b.display()
    );
    for item in std::iter::once(PathBuf::new()).chain(items) {
        let (x, y) = (a.join(&item), b.join(&item));
        let (mx, my) = (
            fs::symlink_metadata(&x).unwrap(),
            fs::symlink_metadata(&y).unwrap(),
        );
        assert_eq!(mx.file_type(), my.file_type(), "{}", item.display());
        if !mx.file_type().is_symlink() {
            assert_eq!(mx.mode(), my.mode(), "{}", item.display());
        }
        assert_eq!(
            (mx.uid(), mx.gid()),
            (my.uid(), my.gid()),
            "{}",
            item.display()
        );
        if mx.file_type().is_symlink() {
            assert_eq!(fs::read_link(&x).unwrap(), fs::read_link(&y).unwrap());
        } else if mx.is_file() {
            assert!(
                fs::read(&x).unwrap() == fs::read(&y).unwrap(),
                "{}",
                item.display()
            );
        }
    }
}

fn chown_tree(path: &Path, uid: u32, gid: u32) {
    for item in std::iter::once(path.to_owned()).chain(walk(path)) {
        std::os::unix::fs::lchown(&item, Some(uid), Some(gid)).unwrap();
    }
}

/// The accounts of the machine backed up: Ada, its administrator, uid 1001 in group 1001; Bea, uid
/// 1002 in group 1005 (a group whose number is not the uid's).
fn accounts() -> Vec<machine::Account> {
    vec![
        machine::Account {
            account_id: ADA.into(),
            user: user(ADA),
            uid: 1001,
            gid: Some(1001),
            name: "Ada Lovelace".into(),
            administrator: true,
        },
        machine::Account {
            account_id: BEA.into(),
            user: user(BEA),
            uid: 1002,
            gid: Some(1005),
            name: "Bea Ramos".into(),
            administrator: false,
        },
    ]
}

/// A machine's root: the accounts' homes, a home that is no account's, their pictures, a Wi-Fi
/// network and the manifest.
fn machine(root: &Path, bea_home: bool) {
    let [ada, bea] = &accounts()[..] else {
        unreachable!()
    };
    home(&root.join("home").join(&ada.user), "Ada's\n");
    if bea_home {
        home(&root.join("home").join(&bea.user), "Bea's\n");
    }
    home(&root.join("home/stranger"), "nobody's\n");
    let pictures = root.join("var/lib/oxyos/pictures");
    fs::create_dir_all(&pictures).unwrap();
    fs::write(pictures.join(&ada.user), b"\x89PNG ada").unwrap();
    let networks = root.join("etc/NetworkManager/system-connections");
    fs::create_dir_all(&networks).unwrap();
    fs::write(networks.join("Casa.nmconnection"), "[wifi]\nssid=Casa\n").unwrap();
    fs::set_permissions(
        networks.join("Casa.nmconnection"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    fs::create_dir_all(root.join("var/lib/oxyos/backup")).unwrap();
    Manifest {
        version: 1,
        computer: "ada-laptop".into(),
        written: 0,
        accounts: accounts(),
    }
    .write(&root.join("var/lib/oxyos/backup/manifest"))
    .unwrap();
    if as_root() {
        chown_tree(&root.join("home").join(&ada.user), 1001, 1001);
        if bea_home {
            chown_tree(&root.join("home").join(&bea.user), 1002, 1005);
        }
        chown_tree(&root.join("home/stranger"), 1003, 1003);
    }
}

fn sources(root: &Path) -> Vec<PathBuf> {
    [
        "home",
        "var/lib/oxyos/backup/manifest",
        "var/lib/oxyos/pictures",
        "etc/NetworkManager/system-connections",
    ]
    .iter()
    .map(|p| root.join(p))
    .collect()
}

/// A computer's repository in the disk's `OxyOS Backups`, opened by a machine key and the backup
/// password, with backups of `root` at each of `times`.
fn back_up(
    engine: &Engine,
    disk: &Path,
    folder: &str,
    password: &str,
    host: &str,
    root: &Path,
    times: &[&str],
) {
    let path = disk.join("OxyOS Backups").join(folder);
    fs::create_dir_all(&path).unwrap();
    let repository = Repository {
        location: Location::directory(&path),
        password: Zeroizing::new("the machine's key".into()),
    };
    engine.init(&repository, Some(password)).unwrap();
    for time in times {
        let backup = Backup {
            paths: sources(root),
            excludes: Vec::new(),
            host: host.into(),
            tags: Vec::new(),
            time: Some((*time).into()),
        };
        engine
            .backup(&repository, &backup, &Stop::new(), |_| {})
            .unwrap();
    }
}

struct World {
    _dir: tempfile::TempDir,
    base: PathBuf,
    machine: PathBuf,
    restorer: Restorer,
}

fn world(bea_home: bool) -> Option<World> {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_owned();
    let engine = engine(&base.join("cache"))?;
    if !passwd_tools(&base) {
        return None;
    }
    let machine_root = base.join("machine");
    machine(&machine_root, bea_home);
    let disks = base.join("disks");
    let disk = disks.join(UUID);
    fs::create_dir_all(&disk).unwrap();
    back_up(
        &engine,
        &disk,
        "0123456789abcdef",
        PASSWORD,
        "ada-laptop",
        &machine_root,
        &["2026-11-01 09:00:00", "2026-11-07 18:30:00"],
    );
    // Another person's computer on the same disk, with its own password.
    back_up(
        &engine,
        &disk,
        "fedcba9876543210",
        "another password",
        "bea-desktop",
        &machine_root,
        &["2026-11-02 10:00:00"],
    );
    let played = Arc::new(Played::open(&disks).unwrap());
    Some(World {
        _dir: dir,
        base,
        machine: machine_root.clone(),
        restorer: Restorer::new(engine, played).with_root(&machine_root),
    })
}

fn source() -> Source {
    Source::Disk { uuid: UUID.into() }
}

#[test]
fn a_computer_is_found_by_its_backup_password_and_restored_into_another_root() {
    let Some(world) = world(true) else { return };
    let restorer = &world.restorer;
    assert_eq!(restorer.disks().len(), 1);
    let folder = restorer.open(&source(), None).unwrap();

    // The password opens one computer of the two; its backups newest first, its accounts.
    let listing = restorer.computers(&folder, PASSWORD).unwrap();
    assert_eq!(listing.unopened, 1);
    let [computer] = &listing.computers[..] else {
        panic!("{listing:?}")
    };
    assert_eq!(computer.name, "ada-laptop");
    assert_eq!(computer.repository, "0123456789abcdef");
    assert_eq!(computer.backups.len(), 2);
    assert!(computer.backups[0].time > computer.backups[1].time);
    let names: Vec<&str> = computer.accounts.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Ada Lovelace", "Bea Ramos"]);
    assert!(computer.accounts[0].administrator);
    // A wrong password opens neither.
    let none = restorer.computers(&folder, "wrong").unwrap();
    assert!(none.computers.is_empty());
    assert_eq!(none.unopened, 2);

    // The newest backup, restored into a root with no account.
    let target = world.base.join("target");
    system_root(&target);
    let repository = folder.repository(&computer.repository, PASSWORD).unwrap();
    let mut steps = Vec::new();
    let restored = restorer
        .restore(
            &repository,
            &computer.backups[0].snapshot,
            &target,
            &Stop::new(),
            |step| {
                steps.push(match step {
                    Step::Accounts => "accounts",
                    Step::Files(_) => "files",
                    Step::Finishing => "finishing",
                })
            },
        )
        .unwrap();
    steps.dedup();
    assert_eq!(steps.first(), Some(&"accounts"));
    assert_eq!(steps.last(), Some(&"finishing"));
    assert_eq!(restored.computer, "ada-laptop");
    assert_eq!(restored.accounts.len(), 2);

    // The users, with their numbers, locked; Ada an administrator.
    let (ada, bea) = (user(ADA), user(BEA));
    let passwd = fs::read_to_string(target.join("etc/passwd")).unwrap();
    assert!(passwd.contains(&format!("{ada}:x:1001:1001:")), "{passwd}");
    assert!(passwd.contains(&format!("{bea}:x:1002:1005:")), "{passwd}");
    let group = fs::read_to_string(target.join("etc/group")).unwrap();
    assert!(group.contains(&format!("{ada}:x:1001:")), "{group}");
    assert!(group.contains(&format!("{bea}:x:1005:")), "{group}");
    assert!(group.contains(&format!("sudo:x:27:{ada}\n")), "{group}");
    let shadow = fs::read_to_string(target.join("etc/shadow")).unwrap();
    for name in [&ada, &bea] {
        let verifier = shadow
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{name}:")))
            .unwrap();
        assert!(
            verifier.starts_with('!'),
            "{name} is not locked: {verifier}"
        );
    }

    // Their homes as they were, nobody else's; their pictures; the Wi-Fi network.
    same(
        &world.machine.join("home").join(&ada),
        &target.join("home").join(&ada),
    );
    same(
        &world.machine.join("home").join(&bea),
        &target.join("home").join(&bea),
    );
    assert!(!target.join("home/stranger").exists());
    assert_eq!(
        fs::read(target.join("var/lib/oxyos/pictures").join(&ada)).unwrap(),
        b"\x89PNG ada"
    );
    let network = target.join("etc/NetworkManager/system-connections/Casa.nmconnection");
    assert_eq!(fs::read_to_string(&network).unwrap(), "[wifi]\nssid=Casa\n");
    assert_eq!(fs::metadata(&network).unwrap().mode() & 0o777, 0o600);
    // Not the manifest: the service writes the new machine's.
    assert!(!target.join("var/lib/oxyos/backup").exists());

    // The login screen lists both, to sign in again; the machine has its administrator.
    let store = Accounts::open(&target.join("var/lib/oxyos/accounts")).unwrap();
    let (listed, broken) = store.list().unwrap();
    assert!(broken.is_empty(), "{broken:?}");
    let listed: Vec<(String, &str, u32, State)> = listed
        .iter()
        .map(|a| (a.user.as_str().to_owned(), a.name.as_str(), a.uid, a.state))
        .collect();
    assert!(listed.contains(&(ada.clone(), "Ada Lovelace", 1001, State::SignInAgain)));
    assert!(listed.contains(&(bea.clone(), "Bea Ramos", 1002, State::SignInAgain)));
    assert!(store.fact(ADMINISTRATOR_MADE));

    // A computer with accounts takes no restore.
    assert!(matches!(
        restorer.restore(
            &repository,
            &computer.backups[0].snapshot,
            &target,
            &Stop::new(),
            |_| {}
        ),
        Err(Refusal::AlreadySetUp)
    ));
}

#[test]
fn a_clash_with_the_targets_users_is_refused_before_anything_is_made() {
    let Some(world) = world(true) else { return };
    let restorer = &world.restorer;
    let folder = restorer.open(&source(), None).unwrap();
    let listing = restorer.computers(&folder, PASSWORD).unwrap();
    let computer = &listing.computers[0];
    let repository = folder.repository(&computer.repository, PASSWORD).unwrap();
    // A developer's user has Bea's uid.
    let target = world.base.join("target");
    system_root(&target);
    let passwd = target.join("etc/passwd");
    fs::write(
        &passwd,
        "root:x:0:0:root:/root:/bin/sh\nnate:x:1002:1002::/home/nate:/bin/bash\n",
    )
    .unwrap();
    let before = fs::read_to_string(&passwd).unwrap();
    match restorer.restore(
        &repository,
        &computer.backups[0].snapshot,
        &target,
        &Stop::new(),
        |_| {},
    ) {
        Err(Refusal::Clash(why)) => assert!(why.contains("uid 1002 is nate's"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(fs::read_to_string(&passwd).unwrap(), before);
    assert!(!target.join("home").join(user(ADA)).exists());
    assert!(fs::read_to_string(target.join("etc/group"))
        .unwrap()
        .ends_with("sudo:x:27:\n"));
}

#[test]
fn a_restore_cut_short_is_finished_by_the_next() {
    // As root, Bea's home is not in the backup: it is made empty, hers.
    let Some(world) = world(!as_root()) else {
        return;
    };
    let restorer = &world.restorer;
    let folder = restorer.open(&source(), None).unwrap();
    let listing = restorer.computers(&folder, PASSWORD).unwrap();
    let computer = &listing.computers[0];
    let repository = folder.repository(&computer.repository, PASSWORD).unwrap();
    let target = world.base.join("target");
    system_root(&target);
    // Stopped once the users are made, before their files.
    let stop = Stop::new();
    let stopped = restorer.restore(
        &repository,
        &computer.backups[1].snapshot,
        &target,
        &stop,
        |step| {
            if step == Step::Accounts {
                stop.stop();
            }
        },
    );
    assert!(matches!(stopped, Err(Refusal::Stopped)), "{stopped:?}");
    assert!(fs::read_to_string(target.join("etc/passwd"))
        .unwrap()
        .contains(&user(ADA)));
    let store = Accounts::open(&target.join("var/lib/oxyos/accounts")).unwrap();
    assert!(
        store.list().unwrap().0.is_empty(),
        "no account is listed half restored"
    );

    // Again: the users it made are kept, the rest done.
    let restored = restorer
        .restore(
            &repository,
            &computer.backups[1].snapshot,
            &target,
            &Stop::new(),
            |_| {},
        )
        .unwrap();
    assert_eq!(restored.accounts.len(), 2);
    assert_eq!(store.list().unwrap().0.len(), 2);
    let passwd = fs::read_to_string(target.join("etc/passwd")).unwrap();
    let ada = format!("{}:", user(ADA));
    assert_eq!(
        passwd.lines().filter(|l| l.starts_with(&ada)).count(),
        1,
        "{passwd}"
    );
    same(
        &world.machine.join("home").join(user(ADA)),
        &target.join("home").join(user(ADA)),
    );
    let bea = target.join("home").join(user(BEA));
    if as_root() {
        let metadata = fs::metadata(&bea).unwrap();
        assert_eq!(
            (metadata.uid(), metadata.gid(), metadata.mode() & 0o777),
            (1002, 1005, 0o700)
        );
        assert_eq!(fs::read_dir(&bea).unwrap().count(), 0);
    } else {
        assert!(bea.join("Documents/notes.txt").is_file());
    }
}

#[test]
fn the_login_screen_migrates_a_backup_once_and_only_before_any_account() {
    let Some(world) = world(true) else { return };
    let target = world.base.join("target");
    system_root(&target);
    let me = rustix::process::geteuid().as_raw();
    let migration = Arc::new(Migration {
        restorer: world.restorer,
        target: target.clone(),
        login: Some(me),
        reload_networks: false,
    });
    let (client, server) = Connection::pair().unwrap();
    let serving = {
        let migration = migration.clone();
        std::thread::spawn(move || migration.serve(&server))
    };
    let ask = |request: Value| {
        client.send(&request).unwrap();
        client.receive::<Value>().unwrap().unwrap().unwrap()
    };
    let disks = ask(json!({"disks": {}}));
    assert_eq!(disks["disks"][0]["uuid"], UUID);
    // A migrate before a source is opened.
    assert_eq!(
        ask(json!({"migrate": {"repository": "0123456789abcdef", "snapshot": "0123abcd"}})),
        json!({"refused": {"reason": "notOpened"}})
    );
    assert_eq!(
        ask(json!({"backups": {"source": {"disk": {"uuid": UUID}}, "password": "wrong"}})),
        json!({"refused": {"reason": "wrongPassword"}})
    );
    assert_eq!(
        ask(json!({"backups": {"source": {"disk": {"uuid": "FFFF-0000"}}, "password": PASSWORD}})),
        json!({"refused": {"reason": "notConnected"}})
    );
    let backups =
        ask(json!({"backups": {"source": {"disk": {"uuid": UUID}}, "password": PASSWORD}}));
    let computer = &backups["backups"]["computers"][0];
    assert_eq!(computer["name"], "ada-laptop");
    assert_eq!(backups["backups"]["unopened"], 1);
    client
        .send(&json!({"migrate": {"repository": computer["repository"], "snapshot": computer["backups"][0]["snapshot"]}}))
        .unwrap();
    let mut steps = Vec::new();
    let migrated = loop {
        let answer: Value = client.receive().unwrap().unwrap().unwrap();
        match answer.get("migrating") {
            Some(step) => steps.push(step["step"].as_str().unwrap().to_owned()),
            None => break answer,
        }
    };
    steps.dedup();
    assert_eq!(steps.first().map(String::as_str), Some("accounts"));
    assert_eq!(steps.last().map(String::as_str), Some("finishing"));
    assert_eq!(migrated["migrated"]["computer"], "ada-laptop");
    assert_eq!(migrated["migrated"]["accounts"][0]["name"], "Ada Lovelace");
    // Set up now: nothing more is done.
    assert_eq!(
        ask(json!({"disks": {}})),
        json!({"refused": {"reason": "alreadySetUp"}})
    );
    drop(client);
    serving.join().unwrap().unwrap();

    // Anyone but root and the login screen's user is refused.
    let stranger = Migration {
        login: None,
        ..Arc::into_inner(migration).unwrap()
    };
    if me != 0 {
        let (client, server) = Connection::pair().unwrap();
        std::thread::spawn(move || stranger.serve(&server));
        client.send(&json!({"disks": {}})).unwrap();
        let answer: Value = client.receive().unwrap().unwrap().unwrap();
        assert_eq!(answer, json!({"refused": {"reason": "notPermitted"}}));
    }
}
