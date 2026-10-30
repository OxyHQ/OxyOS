//! Installing, a second account, updating, holding, rolling back, consent, rotation, removing,
//! and what a crash leaves.

use super::*;
use crate::state::Consent;

const ID: &str = "so.oxy.notes";

#[test]
fn an_application_is_installed_whole_and_the_entry_asks_for_nothing() {
    let mut world = World::new("install");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    let prepared = installer.prepare(ALICE, ID).unwrap();
    let plan = &prepared.plan;
    assert_eq!(
        (plan.version_code, plan.update, plan.key_rotation),
        (12, false, false)
    );
    assert_eq!(
        plan.download,
        Pkg::notes(12)
            .files
            .iter()
            .map(|(_, b)| b.len() as u64)
            .sum::<u64>()
    );
    assert_eq!(
        plan.capabilities
            .iter()
            .map(|c| (c.name.as_str(), c.asked_at_first_use))
            .collect::<Vec<_>>(),
        [("network", false), ("notifications", true)]
    );
    let mut said = Vec::new();
    let committed = installer.commit(&prepared, &mut |p| said.push(p)).unwrap();
    assert!(committed.switched && committed.awaiting_consent.is_none());
    assert!(said.contains(&crate::install::Progress::Compiling));

    let version = world.layout().version(ID, 12);
    assert_eq!(
        fs::read(version.join("bundle/index.js")).unwrap(),
        Pkg::notes(12).files[1].1
    );
    let hbc = fs::read(version.join("index.hbc")).unwrap();
    assert_eq!(hbc[..8], HERMES_BYTECODE_MAGIC);
    assert_eq!(
        fs::read(version.join("manifest.json")).unwrap(),
        Pkg::notes(12).manifest()
    );
    // The files are the objects' (hard links), read-only.
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(version.join("icon.svg")).unwrap();
    assert_eq!((meta.nlink(), meta.mode() & 0o777), (2, 0o444));
    assert_eq!(world.objects().len(), 3);

    let entry = read(&world.layout().applications().join(format!("{ID}.desktop")));
    assert!(
        entry.contains("Name=Notes\n") && entry.contains("Name[es]=Notas\n"),
        "{entry}"
    );
    assert!(entry.contains(&format!(
        "X-OxyOS-Bloom-Bundle={}\n",
        version.join("index.hbc").display()
    )));
    assert!(entry.contains("X-OxyOS-Bloom-Module=Notes\n"), "{entry}");
    assert!(entry.contains("X-OxyOS-Runtime=1\n"), "{entry}");
    assert!(entry.contains(&format!("Icon={}\n", version.join("icon.svg").display())));
    assert!(
        !entry.contains("X-OxyOS-Grants"),
        "an entry never asks for a grant"
    );

    let record = world.installed(ID).unwrap();
    assert_eq!(record.current.version_code, 12);
    assert_eq!(record.pinned_key, key(DEVELOPER).public_key().encode());
    assert_eq!(
        decision(&world, "alice", "network"),
        Some(Decision::Granted)
    );
    assert_eq!(
        decision(&world, "alice", "notifications"),
        Some(Decision::Undecided)
    );
    use std::os::unix::fs::PermissionsExt;
    let user_file = world
        .layout()
        .users()
        .join("alice")
        .join(format!("{ID}.json"));
    assert_eq!(
        fs::metadata(&user_file).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        fs::metadata(world.layout().users().join("alice"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
    // Bob does not have it until he installs it.
    assert!(installer.list(BOB).unwrap().is_empty());
    assert_eq!(installer.list(ALICE).unwrap().len(), 1);
    assert!(!installer.has(BOB, ID) && installer.has(ALICE, ID));
}

#[test]
fn a_second_account_shares_the_bytes_and_keeps_its_own_record() {
    let mut world = World::new("second-account");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.counting();
    install(&installer, ALICE, ID).unwrap();
    let fetched_once = installer_asked(&installer);
    let objects = world.objects();
    let prepared = installer.prepare(BOB, ID).unwrap();
    assert_eq!(prepared.plan.download, 0, "the machine has every file");
    installer.commit(&prepared, &mut |_| {}).unwrap();
    assert_eq!(world.objects(), objects);
    let objects_asked: Vec<String> = installer_asked(&installer)[fetched_once.len()..]
        .iter()
        .filter(|p| p.starts_with("objects/"))
        .cloned()
        .collect();
    assert!(objects_asked.is_empty(), "{objects_asked:?}");
    assert!(world.user("bob", ID).is_some() && world.user("alice", ID).is_some());
    assert_eq!(installer.list(BOB).unwrap()[0].version_code, 12);
}

fn installer_asked(installer: &TestInstaller<Counting<FetcherClient>>) -> Vec<String> {
    installer_fetch(installer).asked.lock().unwrap().clone()
}

fn installer_fetch(installer: &TestInstaller<Counting<FetcherClient>>) -> &Counting<FetcherClient> {
    installer.fetch_for_tests()
}

#[test]
fn an_update_fetches_what_changed_and_keeps_the_version_before_until_shown() {
    let mut world = World::new("update");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.counting();
    install(&installer, ALICE, ID).unwrap();
    let before = installer_asked(&installer).len();
    let v13 = Pkg::notes(13).file("icon.svg", b"<svg version='13'/>");
    world.repo.add(&v13);
    world.repo.publish();
    let committed = install(&installer, ALICE, ID).unwrap();
    assert!(committed.switched);
    let asked: Vec<String> = installer_asked(&installer)[before..]
        .iter()
        .filter(|p| p.starts_with("objects/"))
        .cloned()
        .collect();
    // The bundle (its comment names the version) and the icon changed; the logo did not.
    let expected: Vec<String> = ["bundle/index.js", "icon.svg"]
        .iter()
        .map(|p| {
            let bytes = &v13.files.iter().find(|(q, _)| q == p).unwrap().1;
            let hex = Sha256::of(bytes).hex();
            format!("objects/{}/{hex}", &hex[..2])
        })
        .collect();
    let mut asked_sorted = asked.clone();
    asked_sorted.sort();
    let mut expected_sorted = expected;
    expected_sorted.sort();
    assert_eq!(asked_sorted, expected_sorted);

    let record = world.installed(ID).unwrap();
    assert_eq!(record.current.version_code, 13);
    assert_eq!(record.previous.as_ref().map(|p| p.version_code), Some(12));
    assert!(world.layout().version(ID, 12).is_dir());
    let entry = read(&world.layout().applications().join(format!("{ID}.desktop")));
    assert!(entry.contains("/13/index.hbc"), "{entry}");
    // Shown: the version before goes, and the objects only it had.
    installer.shown(ID, 13).unwrap();
    assert!(!world.layout().version(ID, 12).exists());
    assert!(world.installed(ID).unwrap().previous.is_none());
    assert_eq!(world.objects().len(), 3);
}

#[test]
fn a_held_version_is_kept_and_the_switch_waits_for_its_release() {
    let mut world = World::new("hold");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    installer.shown(ID, 12).unwrap();
    installer.hold(ID, 12).unwrap();
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    let updates = installer.check_updates().unwrap();
    assert_eq!(updates[0].outcome, "staged", "{updates:?}");
    assert_eq!(world.installed(ID).unwrap().current.version_code, 12);
    // Released: switched to.
    installer.release(ID, 12).unwrap();
    let record = world.installed(ID).unwrap();
    assert_eq!(record.current.version_code, 13);
    // 12 is the version before, kept until 13 shows; held again, it stays past that too.
    installer.hold(ID, 12).unwrap();
    installer.shown(ID, 13).unwrap();
    assert!(
        world.layout().version(ID, 12).is_dir(),
        "a held version stays"
    );
    installer.release(ID, 12).unwrap();
    assert!(!world.layout().version(ID, 12).exists());
    assert!(
        installer.release(ID, 12).is_err(),
        "nothing left to release"
    );
    assert!(installer.hold(ID, 12).is_err(), "no such version");
}

#[test]
fn a_version_that_never_shows_a_window_is_rolled_back_and_not_tried_again() {
    let mut world = World::new("rollback");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    installer.shown(ID, 12).unwrap();
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    install(&installer, ALICE, ID).unwrap();
    assert!(
        !installer.exited(ID, 13, false).unwrap(),
        "once is not enough"
    );
    assert!(installer.exited(ID, 13, false).unwrap(), "twice is");
    let record = world.installed(ID).unwrap();
    assert_eq!(
        (record.current.version_code, record.bad.clone()),
        (12, vec![13])
    );
    let entry = read(&world.layout().applications().join(format!("{ID}.desktop")));
    assert!(entry.contains("/12/index.hbc"), "{entry}");
    assert!(!world.layout().version(ID, 13).exists());
    // 13 is never offered again here; 12 is what the store has for this machine.
    assert_eq!(installer.prepare(ALICE, ID).unwrap().plan.version_code, 12);
    assert_eq!(installer.check_updates().unwrap()[0].outcome, "upToDate");
    // A version that showed a window is never rolled back.
    world.repo.add(&Pkg::notes(14));
    world.repo.publish();
    install(&installer, ALICE, ID).unwrap();
    installer.shown(ID, 14).unwrap();
    assert!(!installer.exited(ID, 14, false).unwrap());
    assert!(!installer.exited(ID, 14, false).unwrap());
    assert_eq!(world.installed(ID).unwrap().current.version_code, 14);
}

#[test]
fn nothing_goes_down() {
    let mut world = World::new("downgrade");
    world.repo.add(&Pkg::notes(12));
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    assert_eq!(
        world.installed(ID).unwrap().current.version_code,
        13,
        "the newest"
    );
    // The store then offers only 12: older than what is installed.
    world.repo.remove(&Pkg::notes(13));
    world.repo.publish();
    assert_eq!(
        installer.prepare(ALICE, ID).err(),
        Some(InstallError::Downgrade)
    );
    assert_eq!(
        installer.prepare(BOB, ID).err(),
        Some(InstallError::Downgrade)
    );
}

#[test]
fn a_new_capability_shown_at_install_waits_for_every_account() {
    let mut world = World::new("consent");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    install(&installer, BOB, ID).unwrap();
    // 13 asks for the camera (asked at first use: no consent) and the account (shown at
    // install: consent).
    let mut v13 = Pkg::notes(13);
    v13.capabilities = vec!["camera", "network", "notifications", "oxy-account"];
    world.repo.add(&v13);
    world.repo.publish();
    let updates = installer.check_updates().unwrap();
    assert_eq!(updates[0].outcome, "awaitingConsent");
    let staged = world.installed(ID).unwrap().staged.unwrap();
    assert_eq!(
        staged.consent,
        Some(Consent {
            capabilities: vec!["oxy-account".into()],
            key_rotation: false,
            approved: vec![]
        })
    );
    let plan = installer.prepare(ALICE, ID).unwrap();
    assert_eq!(plan.plan.new_capabilities, ["oxy-account"]);
    let committed = installer.commit(&plan, &mut |_| {}).unwrap();
    assert!(!committed.switched);
    assert_eq!(committed.awaiting_consent.unwrap().approved, [ALICE]);
    assert_eq!(world.installed(ID).unwrap().current.version_code, 12);
    install(&installer, BOB, ID).unwrap();
    assert_eq!(world.installed(ID).unwrap().current.version_code, 13);
    for user in ["alice", "bob"] {
        assert_eq!(
            decision(&world, user, "oxy-account"),
            Some(Decision::Granted)
        );
        assert_eq!(decision(&world, user, "camera"), Some(Decision::Undecided));
    }
}

#[test]
fn a_rotated_key_is_followed_through_its_lineage_with_consent() {
    let mut world = World::new("rotation");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    let rotation = key(DEVELOPER)
        .rotate_to(&key(DEVELOPER_NEXT).public_key(), None)
        .unwrap();
    let mut v13 = Pkg::notes(13);
    v13.signer = DEVELOPER_NEXT;
    v13.published_key = DEVELOPER_NEXT;
    v13.lineage = Lineage(vec![rotation]);
    world.repo.add(&v13);
    world.repo.publish();
    let prepared = installer.prepare(ALICE, ID).unwrap();
    assert!(prepared.plan.key_rotation);
    assert!(installer.commit(&prepared, &mut |_| {}).unwrap().switched);
    assert_eq!(
        world.installed(ID).unwrap().pinned_key,
        key(DEVELOPER_NEXT).public_key().encode()
    );
}

#[test]
fn removing_the_last_account_removes_the_application() {
    let mut world = World::new("uninstall");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    install(&installer, BOB, ID).unwrap();
    assert_eq!(
        installer.uninstall(ALICE, ID).unwrap(),
        [
            ".local/share/oxyos/apps/so.oxy.notes",
            ".cache/oxyos/apps/so.oxy.notes"
        ]
    );
    assert!(world.user("alice", ID).is_none() && world.installed(ID).is_some());
    assert_eq!(
        installer.uninstall(ALICE, ID).err(),
        Some(InstallError::NotInstalled)
    );
    installer.uninstall(BOB, ID).unwrap();
    let left = world.files();
    assert!(left.is_empty(), "{left:?}");
    assert!(!world.layout().packages().join(ID).exists());
}

#[test]
fn a_crash_leaves_the_old_version_or_the_new_one() {
    let mut world = World::new("crash");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    {
        let installer = world.installer();
        install(&installer, ALICE, ID).unwrap();
    }
    let layout = world.layout();
    let before = world.files();
    // A crash while fetching and assembling: a staged object, a half-assembled version, a new
    // record not yet renamed.
    fs::write(layout.staging().join("object-ab-1"), b"half").unwrap();
    fs::create_dir_all(layout.packages().join(ID).join(".13.new-0001/bundle")).unwrap();
    fs::write(layout.installed().join(format!(".{ID}.json.new")), b"{").unwrap();
    // A crash after the record, before the entry.
    fs::remove_file(layout.applications().join(format!("{ID}.desktop"))).unwrap();
    // A crash after a version was renamed into place, before its record: nothing names it.
    fs::create_dir_all(layout.version(ID, 13)).unwrap();
    fs::write(layout.version(ID, 13).join("manifest.json"), b"{}").unwrap();
    let installer = world.installer();
    assert_eq!(
        world.files(),
        before,
        "opened again, the machine is as it was"
    );
    assert_eq!(installer.list(ALICE).unwrap()[0].version_code, 12);
    // And a crash between switching the record and writing the entry leaves the new record,
    // whose entry is written again when the service starts.
    drop(installer);
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    let entry_path = layout.applications().join(format!("{ID}.desktop"));
    fs::write(&entry_path, b"stale").unwrap();
    drop(installer);
    let _installer = world.installer();
    assert!(read(&entry_path).contains("/13/index.hbc"));
}

#[test]
fn hermes_compiles_the_bundle_when_the_tests_have_it() {
    // OXYOS_TEST_HERMESC: the hermesc of the runtime's Hermes (tools/rn-host-spike's build).
    let Some(hermesc) = std::env::var_os("OXYOS_TEST_HERMESC").map(PathBuf::from) else {
        eprintln!("skipped: OXYOS_TEST_HERMESC names no hermesc, so the real compiler is not run");
        return;
    };
    let mut world = World::new("hermesc");
    let pkg = Pkg::notes(12).file(
        "bundle/index.js",
        b"var notes = [1, 2, 3]; print(notes.length);",
    );
    world.repo.add(&pkg);
    world.repo.publish();
    let installer = Installer::open(
        Layout::new(&world.state),
        &world.repo.anchor(),
        world.fetcher.client(),
        crate::compile::Hermesc { confined: false },
        accounts(),
        machine(&world.scratch.0, Some(&hermesc)),
        now,
    )
    .unwrap();
    let prepared = installer.prepare(ALICE, ID).unwrap();
    installer.commit(&prepared, &mut |_| {}).unwrap();
    let hbc = fs::read(world.layout().version(ID, 12).join("index.hbc")).unwrap();
    assert_eq!(hbc[..8], HERMES_BYTECODE_MAGIC);
    assert!(hbc.len() > 64);
    // A bundle that does not compile installs nothing.
    let bad = Pkg::notes(13).file("bundle/index.js", b"var = ;");
    world.repo.add(&bad);
    world.repo.publish();
    let before = world.files();
    let prepared = installer.prepare(ALICE, ID).unwrap();
    assert!(matches!(
        installer.commit(&prepared, &mut |_| {}),
        Err(InstallError::Compile(_))
    ));
    assert_eq!(world.files(), before);
}
