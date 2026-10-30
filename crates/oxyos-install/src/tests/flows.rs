//! Installing, a second account, updating, holding, rolling back, consent, rotation, removing,
//! what a crash leaves: each account its own, the bytes the machine's.

use super::*;
use crate::state::{Consent, Staged};

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
    assert!(committed.switched && !committed.at_next_launch);
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
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(version.join("icon.svg")).unwrap();
    assert_eq!((meta.nlink(), meta.mode() & 0o777), (2, 0o444));
    assert_eq!(world.objects().len(), 3);

    let entry = world.entry("alice", ID);
    assert!(
        entry.contains("Name=Notes\n") && entry.contains("Name[es]=Notas\n"),
        "{entry}"
    );
    assert!(entry.contains(&format!(
        "X-OxyOS-Bloom-Bundle={}\n",
        version.join("index.hbc").display()
    )));
    assert!(entry.contains("X-OxyOS-Bloom-Module=Notes\n"), "{entry}");
    assert!(
        entry.contains("X-OxyOS-Runtime=1\n") && entry.contains("X-OxyOS-Package-Version=12\n")
    );
    assert!(entry.contains(&format!("Icon={}\n", version.join("icon.svg").display())));
    assert!(
        !entry.contains("X-OxyOS-Grants"),
        "an entry never asks for a grant"
    );

    // The machine's record is the pinned key alone; the account's is its version and decisions.
    let record = world.installed(ID).unwrap();
    assert_eq!(record.pinned_key, key(DEVELOPER).public_key().encode());
    assert_eq!(world.current("alice", ID), Some(12));
    assert_eq!(
        decision(&world, "alice", "network"),
        Some(Decision::Granted)
    );
    assert_eq!(
        decision(&world, "alice", "notifications"),
        Some(Decision::Undecided)
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let layout = world.layout();
    assert_eq!(
        mode(&layout.user("alice").join(format!("{ID}.json"))),
        0o640
    );
    assert_eq!(mode(&layout.user("alice")), 0o750);
    assert_eq!(mode(&layout.user_applications("alice")), 0o750);
    assert_eq!(
        mode(
            &layout
                .user_applications("alice")
                .join(format!("{ID}.desktop"))
        ),
        0o640
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
    let fetched_once = asked(&installer).len();
    let objects = world.objects();
    let prepared = installer.prepare(BOB, ID).unwrap();
    assert_eq!(prepared.plan.download, 0, "the machine has every file");
    installer.commit(&prepared, &mut |_| {}).unwrap();
    assert_eq!(world.objects(), objects);
    let objects_asked: Vec<String> = asked(&installer)[fetched_once..]
        .iter()
        .filter(|p| p.starts_with("objects/"))
        .cloned()
        .collect();
    assert!(objects_asked.is_empty(), "{objects_asked:?}");
    assert_eq!(
        (world.current("alice", ID), world.current("bob", ID)),
        (Some(12), Some(12))
    );
    assert!(world.entry("bob", ID).contains("/12/index.hbc"));
}

#[test]
fn a_second_account_gets_the_pinned_key_and_no_other() {
    let mut world = World::new("pinned-for-all");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    // The store now names another key for 13, with no rotation from the pinned one: Bob, who
    // installs for the first time, is not handed it.
    let mut v13 = Pkg::notes(13);
    v13.signer = STRANGER;
    v13.published_key = STRANGER;
    world.repo.add(&v13);
    world.repo.publish();
    assert_eq!(
        installer.prepare(BOB, ID).err(),
        Some(InstallError::Package(oxyos_package::Error::NotCurrentKey))
    );
    assert_eq!(
        world.installed(ID).unwrap().pinned_key,
        key(DEVELOPER).public_key().encode()
    );
}

fn asked(installer: &TestInstaller<Counting<FetcherClient>>) -> Vec<String> {
    installer.fetch_for_tests().asked.lock().unwrap().clone()
}

#[test]
fn an_update_fetches_what_changed_and_keeps_the_version_before_until_shown() {
    let mut world = World::new("update");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.counting();
    install(&installer, ALICE, ID).unwrap();
    let before = asked(&installer).len();
    let v13 = Pkg::notes(13).file("icon.svg", b"<svg version='13'/>");
    world.repo.add(&v13);
    world.repo.publish();
    installer.check().unwrap();
    // The state counts only the objects the machine lacks: the bundle and the icon changed.
    let state = installer.update_state(ALICE).unwrap();
    let changed: u64 = ["bundle/index.js", "icon.svg"]
        .iter()
        .map(|p| v13.files.iter().find(|(q, _)| q == p).unwrap().1.len() as u64)
        .sum();
    assert_eq!(state.updates.len(), 1);
    let update = &state.updates[0];
    assert_eq!(
        (
            update.from_version_code,
            update.to_version_code,
            update.download,
            update.status.as_str()
        ),
        (12, 13, changed, "available")
    );
    assert_eq!(update.release_notes["es"], "Más rápida.");
    let updated = installer.update(ALICE, None).unwrap();
    assert_eq!(updated[0].outcome, "switched");
    let objects_asked: Vec<String> = asked(&installer)[before..]
        .iter()
        .filter(|p| p.starts_with("objects/"))
        .cloned()
        .collect();
    assert_eq!(objects_asked.len(), 2, "{objects_asked:?}");
    let alice = world.user("alice", ID).unwrap();
    assert_eq!((alice.current, alice.previous), (13, Some(12)));
    assert!(world.layout().version(ID, 12).is_dir());
    assert!(world.entry("alice", ID).contains("/13/index.hbc"));
    assert!(installer.update_state(ALICE).unwrap().updates.is_empty());
    // Shown: the version before goes, and the objects only it had.
    installer.shown(ALICE, ID, 13).unwrap();
    assert!(!world.layout().version(ID, 12).exists());
    assert_eq!(world.objects().len(), 3);
}

#[test]
fn an_account_never_holds_another_back() {
    let mut world = World::new("consent-per-account");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    install(&installer, BOB, ID).unwrap();
    // 13 asks for the account (shown at install: consent) and the camera (asked at first use:
    // none).
    let mut v13 = Pkg::notes(13);
    v13.capabilities = vec!["camera", "network", "notifications", "oxy-account"];
    world.repo.add(&v13);
    world.repo.publish();
    installer.check().unwrap();
    let wanted = Some(Consent {
        capabilities: vec!["oxy-account".into()],
        key_rotation: false,
    });
    for uid in [ALICE, BOB] {
        let state = installer.update_state(uid).unwrap();
        assert_eq!(state.updates[0].status, "awaitingConsent");
        assert_eq!(state.updates[0].consent, wanted);
    }
    // Update All stages it for each; neither switches without its own consent.
    assert_eq!(
        installer.update(ALICE, None).unwrap()[0].outcome,
        "awaitingConsent"
    );
    assert_eq!(
        installer.update(BOB, None).unwrap()[0].outcome,
        "awaitingConsent"
    );
    assert_eq!(
        world.user("alice", ID).unwrap().staged,
        Some(Staged {
            version_code: 13,
            consent: wanted.clone()
        })
    );
    // Alice consents: she switches, Bob stays on 12, and 12's tree stays while he names it.
    assert_eq!(
        installer.consent(ALICE, ID, 13).unwrap().outcome,
        "switched"
    );
    assert_eq!(
        (world.current("alice", ID), world.current("bob", ID)),
        (Some(13), Some(12))
    );
    assert!(world.entry("alice", ID).contains("/13/") && world.entry("bob", ID).contains("/12/"));
    assert_eq!(
        decision(&world, "alice", "oxy-account"),
        Some(Decision::Granted)
    );
    assert_eq!(
        decision(&world, "alice", "camera"),
        Some(Decision::Undecided)
    );
    assert_eq!(decision(&world, "bob", "oxy-account"), None);
    installer.shown(ALICE, ID, 13).unwrap();
    assert!(
        world.layout().version(ID, 12).is_dir(),
        "Bob's current version stays"
    );
    assert_eq!(
        installer.update_state(BOB).unwrap().updates[0].status,
        "awaitingConsent"
    );
    // Bob consents: switched; once he shows a window of it, 12 is named by nobody, and goes.
    installer.consent(BOB, ID, 13).unwrap();
    installer.shown(BOB, ID, 13).unwrap();
    assert!(!world.layout().version(ID, 12).exists());
    // A consent to something not staged is stale.
    assert_eq!(
        installer.consent(BOB, ID, 13).err(),
        Some(InstallError::Stale)
    );
}

#[test]
fn a_hold_defers_only_its_own_accounts_switch() {
    let mut world = World::new("hold");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    install(&installer, BOB, ID).unwrap();
    installer.shown(ALICE, ID, 12).unwrap();
    installer.shown(BOB, ID, 12).unwrap();
    installer.hold(ALICE, ID, 12).unwrap();
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    installer.check().unwrap();
    assert_eq!(
        installer.update_state(ALICE).unwrap().updates[0].status,
        "atNextLaunch"
    );
    assert_eq!(
        installer.update_state(BOB).unwrap().updates[0].status,
        "available"
    );
    assert_eq!(
        installer.update(ALICE, None).unwrap()[0].outcome,
        "atNextLaunch"
    );
    assert_eq!(installer.update(BOB, None).unwrap()[0].outcome, "switched");
    assert_eq!(
        (world.current("alice", ID), world.current("bob", ID)),
        (Some(12), Some(13))
    );
    // Alice's session ends: she is switched.
    installer.release(ALICE, ID, 12).unwrap();
    assert_eq!(world.current("alice", ID), Some(13));
    // A held version is kept past being needed, and goes with the hold.
    installer.hold(ALICE, ID, 12).unwrap();
    installer.shown(ALICE, ID, 13).unwrap();
    installer.shown(BOB, ID, 13).unwrap();
    assert!(
        world.layout().version(ID, 12).is_dir(),
        "a held version stays"
    );
    installer.release(ALICE, ID, 12).unwrap();
    assert!(!world.layout().version(ID, 12).exists());
    assert!(
        installer.release(ALICE, ID, 12).is_err(),
        "nothing left to release"
    );
    assert!(
        installer.hold(ALICE, ID, 12).is_err(),
        "no such version for her"
    );
    assert!(installer.hold(ALICE, "so.oxy.other", 1).is_err());
}

#[test]
fn a_rollback_is_its_accounts_alone() {
    let mut world = World::new("rollback");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    install(&installer, BOB, ID).unwrap();
    installer.shown(ALICE, ID, 12).unwrap();
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    installer.check().unwrap();
    installer.update(ALICE, None).unwrap();
    installer.update(BOB, None).unwrap();
    installer.shown(BOB, ID, 13).unwrap();
    assert!(
        !installer.exited(ALICE, ID, 13, false).unwrap(),
        "once is not enough"
    );
    assert!(installer.exited(ALICE, ID, 13, false).unwrap(), "twice is");
    let alice = world.user("alice", ID).unwrap();
    assert_eq!((alice.current, alice.bad.clone()), (12, vec![13]));
    assert!(world.entry("alice", ID).contains("/12/index.hbc"));
    assert_eq!(
        world.current("bob", ID),
        Some(13),
        "Bob stays on the new version"
    );
    assert!(world.user("bob", ID).unwrap().bad.is_empty());
    // 13 is never offered to Alice again; 12 is what the store has for her.
    assert!(installer.update_state(ALICE).unwrap().updates.is_empty());
    assert_eq!(installer.prepare(ALICE, ID).unwrap().plan.version_code, 12);
    // A version that showed a window is never rolled back.
    assert!(!installer.exited(BOB, ID, 13, false).unwrap());
    assert!(!installer.exited(BOB, ID, 13, false).unwrap());
    assert_eq!(world.current("bob", ID), Some(13));
}

#[test]
fn nothing_goes_down() {
    let mut world = World::new("downgrade");
    world.repo.add(&Pkg::notes(12));
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    assert_eq!(world.current("alice", ID), Some(13), "the newest");
    world.repo.remove(&Pkg::notes(13));
    world.repo.publish();
    assert_eq!(
        installer.prepare(ALICE, ID).err(),
        Some(InstallError::Downgrade)
    );
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
    installer.check().unwrap();
    let state = installer.update_state(ALICE).unwrap();
    assert_eq!(
        state.updates[0].consent.as_ref().map(|c| c.key_rotation),
        Some(true)
    );
    let prepared = installer.prepare(ALICE, ID).unwrap();
    assert!(prepared.plan.key_rotation);
    assert!(installer.commit(&prepared, &mut |_| {}).unwrap().switched);
    assert_eq!(
        world.installed(ID).unwrap().pinned_key,
        key(DEVELOPER_NEXT).public_key().encode()
    );
}

#[test]
fn checks_join_and_watchers_are_told() {
    let mut world = World::new("checks");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = std::sync::Arc::new(world.counting());
    install(&*installer, ALICE, ID).unwrap();
    install(&*installer, BOB, ID).unwrap();
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    // Alice watches; Bob's update leaves her list alone, her own leaves it at once.
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<UpdateStateSeen>::new()));
    let log = seen.clone();
    installer
        .watch(
            ALICE,
            Box::new(move |state| {
                log.lock().unwrap().push(UpdateStateSeen {
                    checking: state.checking,
                    updates: state.updates.iter().map(|u| u.to_version_code).collect(),
                });
                true
            }),
        )
        .unwrap();
    // Two asking at once make one check: with a store that answers slowly, the second asks
    // while the first's check runs, joins it, and the timestamp is read once.
    assert_eq!(joined_checks(&world), 1, "two callers at once, one check");
    installer.check().unwrap();
    {
        let seen = seen.lock().unwrap();
        assert!(seen.iter().any(|s| s.checking), "told the check started");
        assert_eq!(seen.last().unwrap().updates, [13], "and what it found");
    }
    installer.update(BOB, None).unwrap();
    assert_eq!(seen.lock().unwrap().last().unwrap().updates, [13]);
    installer.update(ALICE, None).unwrap();
    assert!(seen.lock().unwrap().last().unwrap().updates.is_empty());
}

#[derive(Debug)]
struct UpdateStateSeen {
    checking: bool,
    updates: Vec<u64>,
}

/// A fetch whose timestamp waits until the test lets it go: how many times it was asked.
struct Slow {
    inner: FetcherClient,
    gate: std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    timestamps: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Fetch for Slow {
    fn fetch(
        &self,
        path: &str,
        limit: u64,
        out: rustix::fd::BorrowedFd,
    ) -> Result<u64, crate::fetch::FetchError> {
        if path.ends_with("timestamp.json") {
            self.timestamps
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (open, opened) = &*self.gate;
            let mut open = open.lock().unwrap();
            while !*open {
                open = opened.wait(open).unwrap();
            }
        }
        self.inner.fetch(path, limit, out)
    }
}

fn joined_checks(world: &World) -> usize {
    let gate = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let timestamps = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let installer = std::sync::Arc::new(
        world
            .installer_with(
                Slow {
                    inner: world.fetcher.client(),
                    gate: gate.clone(),
                    timestamps: timestamps.clone(),
                },
                FakeCompiler::Works,
                now,
            )
            .unwrap(),
    );
    let first = {
        let installer = installer.clone();
        std::thread::spawn(move || installer.check())
    };
    while timestamps.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        std::thread::yield_now();
    }
    // The first check is running and waits on the store; the second joins it.
    let second = {
        let installer = installer.clone();
        std::thread::spawn(move || installer.check())
    };
    std::thread::sleep(std::time::Duration::from_millis(50));
    {
        let (open, opened) = &*gate;
        *open.lock().unwrap() = true;
        opened.notify_all();
    }
    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
    timestamps.load(std::sync::atomic::Ordering::SeqCst)
}

#[test]
fn automatic_updates_are_each_accounts_option() {
    let mut world = World::new("automatic");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    install(&installer, ALICE, ID).unwrap();
    install(&installer, BOB, ID).unwrap();
    assert!(
        installer.update_state(BOB).unwrap().automatic,
        "on by default"
    );
    installer.set_automatic(BOB, false).unwrap();
    assert!(!installer.update_state(BOB).unwrap().automatic);
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    let done = installer.automatic().unwrap();
    assert_eq!(
        done.iter().map(|(uid, _)| *uid).collect::<Vec<_>>(),
        [ALICE]
    );
    assert_eq!(
        (world.current("alice", ID), world.current("bob", ID)),
        (Some(13), Some(12))
    );
}

#[test]
fn removing_one_account_keeps_the_others() {
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
    assert!(!world
        .layout()
        .user_applications("alice")
        .join(format!("{ID}.desktop"))
        .exists());
    assert!(
        world.layout().version(ID, 12).is_dir(),
        "Bob's version stays"
    );
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
fn a_crash_leaves_each_account_on_the_old_version_or_the_new() {
    let mut world = World::new("crash");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    {
        let installer = world.installer();
        install(&installer, ALICE, ID).unwrap();
        install(&installer, BOB, ID).unwrap();
    }
    let layout = world.layout();
    let before = world.files();
    // A crash while fetching and assembling, and before renaming each kind of record.
    fs::write(layout.staging().join("object-ab-1"), b"half").unwrap();
    fs::create_dir_all(layout.packages().join(ID).join(".13.new-0001/bundle")).unwrap();
    fs::write(layout.installed().join(format!(".{ID}.json.new")), b"{").unwrap();
    fs::write(layout.user("alice").join(format!(".{ID}.json.new")), b"{").unwrap();
    fs::write(
        layout
            .user_applications("bob")
            .join(format!(".{ID}.desktop.new")),
        b"[",
    )
    .unwrap();
    // A crash after a record, before its entry.
    fs::remove_file(
        layout
            .user_applications("alice")
            .join(format!("{ID}.desktop")),
    )
    .unwrap();
    // A crash after a version was renamed into place, before any record named it.
    fs::create_dir_all(layout.version(ID, 13)).unwrap();
    fs::write(layout.version(ID, 13).join("manifest.json"), b"{}").unwrap();
    let installer = world.installer();
    assert_eq!(
        world.files(),
        before,
        "opened again, the machine is as it was"
    );
    // Alice switches to 13; her record is renamed, her entry left stale by a crash: written
    // again from the record when the service starts. Bob is on 12 throughout.
    drop(installer);
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    let installer = world.installer();
    installer.check().unwrap();
    installer.update(ALICE, None).unwrap();
    let alice_entry = layout
        .user_applications("alice")
        .join(format!("{ID}.desktop"));
    fs::write(&alice_entry, b"stale").unwrap();
    drop(installer);
    let _installer = world.installer();
    assert!(read(&alice_entry).contains("/13/index.hbc"));
    assert!(world.entry("bob", ID).contains("/12/index.hbc"));
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

/// Writes a store repository for trying the service on a machine, as root, by hand: run with
/// `OXYOS_WRITE_REPOSITORY=DIRECTORY cargo test -p oxyos-install write_a_repository --
/// --ignored`. DIRECTORY gets the repository (`metadata/`, `targets/`, `objects/`), whose
/// `metadata/1.root.json` is the anchor to trust: Notes 12, and 13 asking for the account's
/// capability, so an update needs consent. Its metadata expires within the year after
/// 2026-10-30.
#[test]
#[ignore]
fn write_a_repository() {
    let Some(directory) = std::env::var_os("OXYOS_WRITE_REPOSITORY").map(PathBuf::from) else {
        panic!("OXYOS_WRITE_REPOSITORY names no directory");
    };
    let mut repo = Repo::new(&directory);
    repo.add(&Pkg::notes(12));
    let mut v13 = Pkg::notes(13);
    v13.capabilities = vec!["network", "notifications", "oxy-account"];
    repo.add(&v13);
    repo.publish();
    eprintln!(
        "wrote {}; the anchor is {}",
        directory.display(),
        repo.anchor().display()
    );
}
