//! The TUF client: python-tuf's repository read as it wrote it, and each attack the specification
//! names refused.

use super::*;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tuf-fixture")
}

#[test]
fn a_repository_python_tuf_wrote_is_read_as_it_wrote_it() {
    // tests/data/make-tuf-fixture.py: python-tuf 6.0.0, root 1 (three keys, two to sign), root 2
    // rotating one of them out, consistent snapshots, one target with custom data.
    let scratch = Scratch::new("python-tuf");
    let store = scratch.0.join("store");
    fs::create_dir_all(&store).unwrap();
    let repository = LocalRepository::new(&fixture());
    let mut client = Client::open(&store, &fixture().join("metadata/1.root.json")).unwrap();
    client.refresh(&repository, NOW).unwrap();
    assert_eq!(
        client.root().version,
        2,
        "the rotated root is the trusted one"
    );
    let path = "so.oxy.fixture/1/any/manifest.json";
    let target = &client.targets().unwrap().targets[path];
    assert_eq!(
        json::canonical(target.custom.as_ref().unwrap()),
        br#"{"oxyos":{"note":"custom data travels with the target"}}"#
    );
    assert_eq!(
        client.fetch_target(&repository, path).unwrap(),
        b"oxyos fixture target\n"
    );
    // What was verified is kept, and a client opened again trusts the newest root.
    let again = Client::open(&store, &fixture().join("metadata/1.root.json")).unwrap();
    assert_eq!(again.root().version, 2);
    assert!(again.targets().is_some());
}

fn client(world: &World) -> Client {
    let store = world.scratch.0.join("store");
    fs::create_dir_all(&store).unwrap();
    Client::open(&store, &world.repo.anchor()).unwrap()
}

fn refresh(world: &World, client: &mut Client, now: i64) -> Result<(), TufError> {
    client.refresh(&LocalRepository::new(&world.repo.dir), now)
}

#[test]
fn metadata_is_refused_when_expired_older_or_mismatched() {
    let mut world = World::new("tuf-attacks");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let mut c = client(&world);
    refresh(&world, &mut c, NOW).unwrap();

    // Freeze: a repository stuck at a moment serves documents that expire.
    assert_eq!(
        refresh(&world, &mut c, LATER),
        Err(TufError::Expired("timestamp"))
    );
    world.repo.expires = [LATER + 86_400, NOW + 1, LATER + 86_400];
    world.repo.publish();
    assert_eq!(
        refresh(&world, &mut c, NOW + 2),
        Err(TufError::Expired("snapshot"))
    );
    world.repo.expires = [NOW + 300 * 86_400; 3];
    world.repo.publish();
    refresh(&world, &mut c, NOW).unwrap();

    // Rollback: an older timestamp than the machine has.
    let current = fs::read(world.repo.dir.join("metadata/timestamp.json")).unwrap();
    let mut older = Repo {
        dir: world.repo.dir.clone(),
        targets: world.repo.targets.clone(),
        versions: [0; 3],
        expires: world.repo.expires,
    };
    older.publish();
    assert_eq!(
        refresh(&world, &mut c, NOW),
        Err(TufError::Rollback("timestamp"))
    );
    fs::write(world.repo.dir.join("metadata/timestamp.json"), &current).unwrap();

    // Mix and match: a targets of another version than the snapshot names, under its name.
    let versions = world.repo.versions;
    world.repo.publish();
    let newer_targets = fs::read(world.repo.dir.join(format!(
        "metadata/{}.targets.json",
        world.repo.versions[TARGETS]
    )))
    .unwrap();
    fs::write(
        world.repo.dir.join(format!(
            "metadata/{}.targets.json",
            world.repo.versions[TARGETS]
        )),
        fs::read(
            world
                .repo
                .dir
                .join(format!("metadata/{}.targets.json", versions[TARGETS])),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        refresh(&world, &mut c, NOW),
        Err(TufError::Mismatch("targets"))
    );
    fs::write(
        world.repo.dir.join(format!(
            "metadata/{}.targets.json",
            world.repo.versions[TARGETS]
        )),
        newer_targets,
    )
    .unwrap();
    refresh(&world, &mut c, NOW).unwrap();

    // A timestamp signed by a key the root does not name for it.
    world.repo.publish_with(STRANGER);
    assert_eq!(
        refresh(&world, &mut c, NOW),
        Err(TufError::Threshold("timestamp"))
    );
}

#[test]
fn a_root_is_trusted_only_signed_by_the_old_threshold_and_its_own() {
    let mut world = World::new("tuf-root");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let mut c = client(&world);
    // Root 2 signed by one old key alone: below the old threshold.
    world.repo.write_root(2, &ROOT_KEYS, &[1], TIMESTAMP_KEY);
    assert_eq!(
        refresh(&world, &mut c, NOW),
        Err(TufError::Threshold("root"))
    );
    // Root 2 naming new root keys, signed by the old two but not by two of its own.
    world.repo.write_root(2, &[1, 7, 8], &[1, 2], TIMESTAMP_KEY);
    assert_eq!(
        refresh(&world, &mut c, NOW),
        Err(TufError::Threshold("root"))
    );
    // Signed by both thresholds: trusted, and its rotated timestamp key used from then on.
    world.repo.write_root(2, &[1, 2, 8], &[1, 2, 8], STRANGER);
    world.repo.publish_with(STRANGER);
    refresh(&world, &mut c, NOW).unwrap();
    assert_eq!(c.root().version, 2);
    // The old timestamp key no longer signs.
    world.repo.publish_with(TIMESTAMP_KEY);
    assert_eq!(
        refresh(&world, &mut c, NOW),
        Err(TufError::Threshold("timestamp"))
    );
    // A root named 3 that says it is 4 is refused.
    world.repo.write_root(3, &[1, 2, 8], &[1, 2], STRANGER);
    let three = world.repo.dir.join("metadata/3.root.json");
    let four = Repo::root_value(4, &[1, 2, 8], STRANGER);
    fs::write(&three, write::sign(four, &[&key(1), &key(2)])).unwrap();
    assert!(matches!(
        refresh(&world, &mut c, NOW),
        Err(TufError::Version { role: "root", .. })
    ));
}

#[test]
fn a_key_counts_once_however_many_times_it_signs() {
    let world = World::new("tuf-threshold");
    let mut c = client(&world);
    // Root 2 signed twice by key 1 (once under its id, once again): one signature, not two.
    let value = Repo::root_value(2, &ROOT_KEYS, TIMESTAMP_KEY);
    let once = write::sign(value.clone(), &[&key(1)]);
    let doubled = {
        let parsed = json::parse(&once).unwrap();
        let Value::Object(members) = parsed else {
            unreachable!()
        };
        let signatures = members
            .iter()
            .find(|(k, _)| k == "signatures")
            .map(|(_, v)| v.clone())
            .unwrap();
        let Value::Array(mut items) = signatures else {
            unreachable!()
        };
        items.push(items[0].clone());
        tuf::canonical(&json::object(vec![
            ("signatures", Value::Array(items)),
            ("signed", value),
        ]))
    };
    fs::write(world.repo.dir.join("metadata/2.root.json"), doubled).unwrap();
    assert_eq!(
        refresh(&world, &mut c, NOW),
        Err(TufError::Threshold("root"))
    );
}
