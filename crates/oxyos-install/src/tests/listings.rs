//! A store names an application by its listing (its slug); the machine resolves the listing to a
//! package from the store's signed metadata, for the releases it can run.

use super::*;
use crate::ipc::{self as service, Answer, Reason, Request};
use crate::socket::Connection;
use std::collections::BTreeMap;

fn listings(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn resolved(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
    pairs
        .iter()
        .map(|(l, id)| (l.to_string(), id.map(str::to_string)))
        .collect()
}

#[test]
fn a_listing_resolves_to_its_package_and_an_unknown_one_to_none() {
    let mut world = World::new("listing");
    world.repo.add(&Pkg::notes(12));
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    let installer = world.installer();
    assert_eq!(
        installer
            .resolve(ALICE, &listings(&["notes", "mention"]))
            .unwrap(),
        resolved(&[("notes", Some("so.oxy.notes")), ("mention", None)])
    );
    // Resolving installs nothing.
    assert!(world.files().is_empty(), "{:?}", world.files());
}

#[test]
fn a_release_the_machine_cannot_run_does_not_resolve() {
    let mut world = World::new("listing-runtime");
    let mut pkg = Pkg::notes(12);
    pkg.runtime = (2, 1);
    world.repo.add(&pkg);
    world.repo.publish();
    assert_eq!(
        world
            .installer()
            .resolve(ALICE, &listings(&["notes"]))
            .unwrap(),
        resolved(&[("notes", None)])
    );
}

#[test]
fn a_listing_two_packages_claim_resolves_to_none() {
    let mut world = World::new("listing-two");
    let mut other = Pkg::notes(3);
    other.id = "so.oxy.other".into();
    world.repo.add(&Pkg::notes(12));
    world.repo.add(&other);
    world.repo.publish();
    assert_eq!(
        world
            .installer()
            .resolve(ALICE, &listings(&["notes"]))
            .unwrap(),
        resolved(&[("notes", None)])
    );
}

#[test]
fn a_release_whose_listing_is_not_a_slug_is_ignored() {
    for bad in ["Notes", "-notes", "notes-", "no--tes", "notes!", ""] {
        let mut world = World::new("listing-bad");
        let mut pkg = Pkg::notes(12);
        pkg.listing = Some(bad);
        world.repo.add(&pkg);
        world.repo.publish();
        let installer = world.installer();
        // The release itself is not offered: its custom data is not the store's.
        assert_eq!(
            installer.prepare(ALICE, "so.oxy.notes").err(),
            Some(InstallError::NoRelease),
            "{bad:?}"
        );
        assert_eq!(
            installer.resolve(ALICE, &listings(&["notes"])).unwrap(),
            resolved(&[("notes", None)]),
            "{bad:?}"
        );
    }
    // Without a listing, a release is offered and resolves from none.
    let mut world = World::new("listing-none");
    let mut pkg = Pkg::notes(12);
    pkg.listing = None;
    world.repo.add(&pkg);
    world.repo.publish();
    let installer = world.installer();
    assert!(installer.prepare(ALICE, "so.oxy.notes").is_ok());
    assert_eq!(
        installer.resolve(ALICE, &listings(&["notes"])).unwrap(),
        resolved(&[("notes", None)])
    );
}

#[test]
fn a_request_naming_what_is_not_a_listing_is_refused() {
    let mut world = World::new("listing-request");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    for bad in ["Notes", "a/b", "-x", "x".repeat(65).as_str()] {
        assert!(
            matches!(
                installer.resolve(ALICE, &listings(&["notes", bad])),
                Err(InstallError::InvalidRequest(_))
            ),
            "{bad:?}"
        );
    }
    let many: Vec<String> = (0..=crate::install::MAX_LISTINGS)
        .map(|n| format!("app-{n}"))
        .collect();
    assert!(matches!(
        installer.resolve(ALICE, &many),
        Err(InstallError::InvalidRequest(_))
    ));
    assert_eq!(
        installer.resolve(4_000_000, &listings(&["notes"])),
        Err(InstallError::NotPermitted)
    );
}

#[test]
fn resolving_reads_the_store_again() {
    let mut world = World::new("listing-refresh");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = world.installer();
    assert_eq!(
        installer.resolve(ALICE, &listings(&["other"])).unwrap(),
        resolved(&[("other", None)])
    );
    let mut other = Pkg::notes(1);
    other.id = "so.oxy.other".into();
    other.listing = Some("other");
    world.repo.add(&other);
    world.repo.publish();
    assert_eq!(
        installer.resolve(ALICE, &listings(&["other"])).unwrap(),
        resolved(&[("other", Some("so.oxy.other"))])
    );
}

#[test]
fn the_socket_resolves_for_the_peer_alone() {
    let mut world = World::new("listing-socket");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = std::sync::Arc::new(world.installer());
    let (client, server) = Connection::pair().unwrap();
    let served = installer.clone();
    let thread = std::thread::spawn(move || {
        let _ = service::serve(&server, &served, |_, _| {});
    });
    let ask = |request: &Request| {
        client.send(request).unwrap();
        match client.receive::<Answer>().unwrap() {
            Some(Some(answer)) => answer,
            other => panic!("no answer: {other:?}"),
        }
    };
    // As JSON on the wire: a listing and the package it is, or null.
    let wire: serde_json::Value =
        serde_json::from_str(r#"{"resolve":{"listings":["notes","mention"]}}"#).unwrap();
    client.send(&wire).unwrap();
    let answer = client
        .receive::<serde_json::Value>()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        answer.to_string(),
        r#"{"resolved":{"mention":null,"notes":"so.oxy.notes"}}"#
    );
    assert_eq!(
        ask(&Request::Resolve {
            listings: listings(&["notes"]),
            uid: Some(ALICE),
        }),
        Answer::refused(Reason::NotPermitted)
    );
    assert!(matches!(
        ask(&Request::Resolve {
            listings: listings(&["Not a slug"]),
            uid: None,
        }),
        Answer::Refused {
            reason: Reason::InvalidRequest,
            ..
        }
    ));
    drop(client);
    thread.join().unwrap();
}
