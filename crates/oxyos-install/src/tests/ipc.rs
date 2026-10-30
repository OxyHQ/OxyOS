//! The service's socket, end to end over a socket pair (the peer's credentials are the kernel's:
//! this process's), and the fetcher's.

use super::*;
use crate::fetch::{self as fetching, FetchError};
use crate::ipc::{self as service, Answer, Reason, Request};
use crate::socket::Connection;
use std::sync::Arc;

const ID: &str = "so.oxy.notes";

fn me() -> u32 {
    rustix::process::getuid().as_raw()
}

/// A served connection to an installer, and the installer.
fn serve(
    world: &World,
) -> (
    Connection,
    Arc<TestInstaller<FetcherClient>>,
    std::thread::JoinHandle<()>,
) {
    let installer = Arc::new(world.installer());
    let (client, server) = Connection::pair().unwrap();
    let served = installer.clone();
    let thread = std::thread::spawn(move || {
        let _ = service::serve(&server, &served, |_, _| {});
    });
    (client, installer, thread)
}

fn ask(connection: &Connection, request: &Request) -> Answer {
    connection.send(request).unwrap();
    loop {
        match connection.receive::<Answer>().unwrap() {
            Some(Some(Answer::Progress(_))) => continue,
            Some(Some(answer)) => return answer,
            other => panic!("no answer: {other:?}"),
        }
    }
}

#[test]
fn an_account_installs_lists_and_removes_its_own_only() {
    let mut world = World::new("socket");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let (connection, installer, thread) = serve(&world);
    // Another account's, named: refused, whatever it asks.
    for request in [
        Request::Prepare {
            id: ID.into(),
            uid: Some(ALICE),
        },
        Request::Uninstall {
            id: ID.into(),
            uid: Some(ALICE),
        },
        Request::List { uid: Some(ALICE) },
    ] {
        assert_eq!(
            ask(&connection, &request),
            Answer::refused(Reason::NotPermitted)
        );
    }
    // A plan that is not this connection's last.
    assert_eq!(
        ask(
            &connection,
            &Request::Commit {
                plan_id: "0".into()
            }
        ),
        Answer::refused(Reason::NoPlan)
    );
    let Answer::Plan(plan) = ask(
        &connection,
        &Request::Prepare {
            id: ID.into(),
            uid: None,
        },
    ) else {
        panic!("no plan")
    };
    assert_eq!(plan.plan.version_code, 12);
    let Answer::Committed(committed) = ask(
        &connection,
        &Request::Commit {
            plan_id: plan.plan_id.clone(),
        },
    ) else {
        panic!("not committed")
    };
    assert!(committed.switched);
    // A plan is committed once.
    assert_eq!(
        ask(
            &connection,
            &Request::Commit {
                plan_id: plan.plan_id
            }
        ),
        Answer::refused(Reason::NoPlan)
    );
    let Answer::Applications(listed) = ask(&connection, &Request::List { uid: None }) else {
        panic!("no list")
    };
    assert_eq!(listed[0].id, ID);
    // Holding what the account has, and saying what its versions did.
    assert_eq!(
        ask(
            &connection,
            &Request::Hold {
                id: ID.into(),
                version_code: 12,
                uid: None,
            }
        ),
        Answer::Held
    );
    assert_eq!(
        ask(
            &connection,
            &Request::Shown {
                id: ID.into(),
                version_code: 12,
                uid: None,
            }
        ),
        Answer::Noted
    );
    // Not what it does not have.
    assert_eq!(
        ask(
            &connection,
            &Request::Hold {
                id: "so.oxy.other".into(),
                version_code: 1,
                uid: None,
            }
        ),
        Answer::Refused {
            reason: Reason::NotInstalled,
            detail: Some("not installed for this account".into())
        }
    );
    // An update staged while held; the connection closing releases the hold, and it is switched.
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    let Answer::UpdateState(state) = ask(&connection, &Request::CheckUpdates { uid: None }) else {
        panic!("no state")
    };
    assert_eq!(state.updates[0].status, "atNextLaunch");
    let Answer::Updated(updated) = ask(&connection, &Request::UpdateAll { uid: None }) else {
        panic!("not updated")
    };
    assert_eq!(updated[0].outcome, "atNextLaunch");
    // Something that is not a request is answered, and the conversation goes on.
    connection
        .send(&serde_json::json!({"format": "c:"}))
        .unwrap();
    assert_eq!(
        connection.receive::<Answer>().unwrap(),
        Some(Some(Answer::refused(Reason::InvalidRequest)))
    );
    drop(connection);
    thread.join().unwrap();
    assert_eq!(world.current("me", ID), Some(13));
    let _ = installer;
}

#[test]
fn a_watcher_is_told_what_another_connection_did() {
    let mut world = World::new("socket-watch");
    world.repo.add(&Pkg::notes(12));
    world.repo.publish();
    let installer = Arc::new(world.installer());
    install(&*installer, me(), ID).unwrap();
    world.repo.add(&Pkg::notes(13));
    world.repo.publish();
    let connect = || {
        let (client, server) = Connection::pair().unwrap();
        let served = installer.clone();
        let thread = std::thread::spawn(move || {
            let _ = service::serve(&server, &served, |_, _| {});
        });
        (client, thread)
    };
    let (settings, settings_thread) = connect();
    let (atlas, atlas_thread) = connect();
    // Settings watches: "watching", then the state now (nothing checked yet).
    settings.send(&Request::WatchUpdates { uid: None }).unwrap();
    let next = |c: &Connection| c.receive::<Answer>().unwrap().unwrap().unwrap();
    assert_eq!(next(&settings), Answer::Watching);
    let Answer::UpdateState(first) = next(&settings) else {
        panic!("no state")
    };
    assert!(first.updates.is_empty() && first.checked_at.is_none());
    // Atlas checks: Settings is told the check started, then what it found.
    let Answer::UpdateState(found) = ask(&atlas, &Request::CheckUpdates { uid: None }) else {
        panic!("no state")
    };
    assert_eq!(found.updates[0].to_version_code, 13);
    let mut told = Vec::new();
    loop {
        let Answer::UpdateState(state) = next(&settings) else {
            panic!("not a state")
        };
        let done = !state.checking && !state.updates.is_empty();
        told.push(state);
        if done {
            break;
        }
    }
    assert!(told[0].checking, "told the check started");
    // Atlas updates it: Settings' list empties at once.
    ask(
        &atlas,
        &Request::Update {
            ids: vec![ID.into()],
            uid: None,
        },
    );
    let Answer::UpdateState(after) = next(&settings) else {
        panic!("not a state")
    };
    assert!(after.updates.is_empty(), "{after:?}");
    drop((settings, atlas));
    settings_thread.join().unwrap();
    atlas_thread.join().unwrap();
}

#[test]
fn the_fetcher_writes_only_for_its_peer_and_only_what_it_may() {
    let world = World::new("fetcher");
    fs::write(world.repo.dir.join("objects/file"), b"0123456789").unwrap();
    let client = world.fetcher.client();
    assert_eq!(
        client.fetch_to_vec("objects/file", 10).unwrap(),
        b"0123456789"
    );
    assert_eq!(
        client.fetch_to_vec("objects/file", 9),
        Err(FetchError::TooLarge)
    );
    assert_eq!(
        client.fetch_to_vec("objects/none", 9),
        Err(FetchError::NotFound)
    );
    for bad in [
        "../secret",
        "objects/../file",
        "/etc/passwd",
        "objects//file",
        "objects/a b",
    ] {
        assert!(
            matches!(client.fetch_to_vec(bad, 9), Err(FetchError::Failed(_))),
            "{bad}"
        );
    }
    // A link in the repository is not followed.
    std::os::unix::fs::symlink("/etc/hostname", world.repo.dir.join("objects/link")).unwrap();
    assert!(matches!(
        client.fetch_to_vec("objects/link", 1 << 20),
        Err(FetchError::Failed(_))
    ));
    // A peer it does not serve gets nothing written.
    let (ours, theirs) = Connection::pair().unwrap();
    let repository = LocalRepository::new(&world.repo.dir);
    let thread = std::thread::spawn(move || {
        let _ = fetching::serve(&theirs, &repository, |peer| peer.uid != me());
    });
    let memory = rustix::fs::memfd_create("test", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
    ours.send_with_fd(
        &serde_json::json!({"fetch": {"path": "objects/file", "limit": 10}}),
        rustix::fd::AsFd::as_fd(&memory),
    )
    .unwrap();
    let answer: serde_json::Value = ours.receive().unwrap().unwrap().unwrap();
    assert_eq!(answer["failed"]["reason"], "not permitted");
    assert_eq!(rustix::fs::fstat(&memory).unwrap().st_size, 0);
    thread.join().unwrap();
}
