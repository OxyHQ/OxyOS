//! Every refusal leaves the machine as it found it: no object, no version, no record, no entry.

use super::*;

const ID: &str = "so.oxy.notes";

/// Publishes `pkg` (after `before`, if given, is installed) and tries to install it for Alice:
/// the error, and that nothing changed.
fn refused(
    name: &str,
    before: Option<Pkg>,
    pkg: Pkg,
    change: impl FnOnce(&mut World),
) -> InstallError {
    refused_with(name, before, pkg, FakeCompiler::Works, change)
}

fn refused_with(
    name: &str,
    before: Option<Pkg>,
    pkg: Pkg,
    compiler: FakeCompiler,
    change: impl FnOnce(&mut World),
) -> InstallError {
    let mut world = World::new(name);
    if let Some(before) = &before {
        world.repo.add(before);
        world.repo.publish();
        install(&world.installer(), ALICE, ID).unwrap();
    }
    world.repo.add(&pkg);
    world.repo.publish();
    change(&mut world);
    let files = world.files();
    let installer = world
        .installer_with(world.fetcher.client(), compiler, now)
        .unwrap();
    let error = install(&installer, ALICE, ID).unwrap_err();
    assert_eq!(world.files(), files, "{name}: the refusal left something");
    let staging: Vec<_> = fs::read_dir(world.layout().staging()).unwrap().collect();
    assert!(staging.is_empty(), "{name}: staging not empty");
    error
}

#[test]
fn a_file_of_other_bytes_than_the_manifest_lists() {
    let pkg = Pkg::notes(12);
    let error = refused("bad-hash", None, pkg.clone(), |world| {
        let hex = Sha256::of(&pkg.files[0].1).hex();
        fs::write(
            world.repo.dir.join("objects").join(&hex[..2]).join(&hex),
            b"\x89PNG another logo",
        )
        .unwrap();
    });
    assert!(
        matches!(
            error,
            InstallError::Package(oxyos_package::Error::SizeMismatch(_)) | InstallError::Fetch(_)
        ),
        "{error:?}"
    );
    let error = refused("bad-digest", None, pkg.clone(), |world| {
        let hex = Sha256::of(&pkg.files[0].1).hex();
        fs::write(
            world.repo.dir.join("objects").join(&hex[..2]).join(&hex),
            b"\x89PNG lagt",
        )
        .unwrap();
    });
    assert!(
        matches!(
            error,
            InstallError::Package(oxyos_package::Error::DigestMismatch(_))
        ),
        "{error:?}"
    );
}

#[test]
fn a_file_missing_from_the_repository() {
    let pkg = Pkg::notes(12);
    let error = refused("missing", None, pkg.clone(), |world| {
        let hex = Sha256::of(&pkg.files[2].1).hex();
        fs::remove_file(world.repo.dir.join("objects").join(&hex[..2]).join(&hex)).unwrap();
    });
    assert!(matches!(error, InstallError::Fetch(_)), "{error:?}");
}

#[test]
fn a_manifest_other_than_the_store_signed() {
    let pkg = Pkg::notes(12);
    let error = refused("manifest-changed", None, pkg.clone(), |world| {
        // The manifest's bytes in the repository changed after the targets were signed.
        let manifest = pkg.manifest();
        let location = world
            .repo
            .dir
            .join(write::target_location(&pkg.target_path(), &manifest));
        let mut changed = manifest.clone();
        changed[10] ^= 1;
        fs::write(location, changed).unwrap();
    });
    assert_eq!(error, InstallError::Store(TufError::Mismatch("a target")));
}

#[test]
fn a_signature_by_another_key_than_the_developers() {
    let mut pkg = Pkg::notes(12);
    pkg.signer = STRANGER;
    let error = refused("wrong-developer", None, pkg, |_| {});
    assert!(
        matches!(error, InstallError::Release(_) | InstallError::Package(_)),
        "{error:?}"
    );
}

#[test]
fn an_update_by_a_key_outside_the_lineage() {
    // 12 pinned the developer's key; 13 is signed by another, which the store names, with no
    // rotation from the pinned key to it.
    let mut v13 = Pkg::notes(13);
    v13.signer = DEVELOPER_NEXT;
    v13.published_key = DEVELOPER_NEXT;
    let error = refused("no-lineage", Some(Pkg::notes(12)), v13.clone(), |_| {});
    assert_eq!(
        error,
        InstallError::Package(oxyos_package::Error::NotCurrentKey)
    );
    // A rotation signed by a key that is not the pinned one is no lineage either.
    v13.lineage = Lineage(vec![key(STRANGER)
        .rotate_to(&key(DEVELOPER_NEXT).public_key(), None)
        .unwrap()]);
    let error = refused("broken-lineage", Some(Pkg::notes(12)), v13, |_| {});
    assert_eq!(
        error,
        InstallError::Package(oxyos_package::Error::NotInLineage)
    );
}

#[test]
fn a_statement_signed_by_a_key_the_root_does_not_name_for_targets() {
    let mut pkg = Pkg::notes(12);
    pkg.store = STRANGER;
    assert_eq!(
        refused("untrusted-store", None, pkg, |_| {}),
        InstallError::NoRelease
    );
}

#[test]
fn expired_or_older_metadata() {
    let error = refused("expired", None, Pkg::notes(12), |world| {
        world.repo.expires[TIMESTAMP] = NOW - 1;
        world.repo.publish();
    });
    assert_eq!(error, InstallError::Store(TufError::Expired("timestamp")));
    let error = refused(
        "rolled-back",
        Some(Pkg::notes(12)),
        Pkg::notes(13),
        |world| {
            // The machine sees the repository's second timestamp…
            let mut client = Client::open(&world.layout().store(), &world.repo.anchor()).unwrap();
            client
                .refresh(&LocalRepository::new(&world.repo.dir), NOW)
                .unwrap();
            // …then the repository goes back to its first, which the machine has seen past.
            let mut older = Repo {
                dir: world.repo.dir.clone(),
                targets: world.repo.targets.clone(),
                versions: [0; 3],
                expires: world.repo.expires,
            };
            older.publish();
        },
    );
    assert_eq!(error, InstallError::Store(TufError::Rollback("timestamp")));
}

#[test]
fn a_runtime_the_machine_does_not_have() {
    let mut pkg = Pkg::notes(12);
    pkg.runtime = (2, 1);
    assert_eq!(
        refused("runtime-abi", None, pkg.clone(), |_| {}),
        InstallError::NoRelease
    );
    pkg.runtime = (1, 4);
    assert_eq!(
        refused("runtime-api", None, pkg, |_| {}),
        InstallError::NoRelease
    );
}

#[test]
fn a_bundle_that_does_not_compile() {
    let error = refused_with("compile", None, Pkg::notes(12), FakeCompiler::Fails, |_| {});
    assert!(matches!(error, InstallError::Compile(_)), "{error:?}");
    let error = refused_with(
        "not-bytecode",
        None,
        Pkg::notes(12),
        FakeCompiler::WritesText,
        |_| {},
    );
    assert!(matches!(error, InstallError::Compile(_)), "{error:?}");
}

#[test]
fn native_code_and_system_grants() {
    let pkg = Pkg::notes(12).file("native/x86_64/libnotes.so", b"\x7fELF");
    assert_eq!(
        refused("native", None, pkg, |_| {}),
        InstallError::Package(oxyos_package::Error::NativeCode)
    );
    // A store that signed a package asking for `settings` still has it refused here.
    let mut pkg = Pkg::notes(12);
    pkg.capabilities.push("settings");
    assert!(matches!(
        refused("system-grant", None, pkg, |_| {}),
        InstallError::Package(oxyos_package::Error::SystemGrant(_))
    ));
}

#[test]
fn bytecode_as_the_entry() {
    let mut bytes = HERMES_BYTECODE_MAGIC.to_vec();
    bytes.extend_from_slice(b"compiled elsewhere");
    let pkg = Pkg::notes(12).file("bundle/index.js", &bytes);
    assert_eq!(
        refused("bytecode", None, pkg, |_| {}),
        InstallError::Package(oxyos_package::Error::Bytecode)
    );
}
