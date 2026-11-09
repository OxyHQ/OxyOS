//! Whether the screen reader runs (docs/adr/0014-screen-reader.mdx, decision 8): every client is
//! told as it connects while one does, and whenever one connects, is granted it, loses it or goes;
//! never while it is as it was.

use super::*;

/// The screen reader, as the session grants it on its own socket.
const SPEAKER: ClientId = 8;

fn speaker() -> Grants {
    Grants {
        accessibility: true,
        screen_reader: true,
        ..Grants::default()
    }
}

/// What `client` was told of the screen reader in `events`, in order.
fn told(events: &[(ClientId, Event)], client: ClientId) -> Vec<bool> {
    to(events, client)
        .iter()
        .filter_map(|event| match event {
            Event::ScreenReaderState { running } => Some(*running),
            _ => None,
        })
        .collect()
}

#[test]
fn every_client_is_told_once_as_the_screen_reader_starts_and_stops() {
    let mut h = Harness::new();
    // None runs: nothing is told, as connecting clients read none from not being told.
    let events = logged(&mut h);
    for client in [APP, OTHER, SHELL, LOCKER] {
        assert_eq!(told(&events, client), [] as [bool; 0]);
    }
    assert!(!h.scene.screen_reader_running());
    // An assistive technology that is no screen reader (the development compositor's record of
    // the trees) changes nothing.
    h.scene
        .connect(
            READER,
            Grants {
                accessibility: true,
                ..Grants::default()
            },
        )
        .unwrap();
    let events = logged(&mut h);
    for client in [APP, OTHER, SHELL, LOCKER, READER] {
        assert_eq!(told(&events, client), [] as [bool; 0]);
    }
    // Super+F5 starts it: every client is told, once, the reader too.
    h.scene.connect(SPEAKER, speaker()).unwrap();
    assert!(h.scene.screen_reader_running());
    let events = logged(&mut h);
    for client in [APP, OTHER, SHELL, LOCKER, READER, SPEAKER] {
        assert_eq!(told(&events, client), [true], "client {client}");
    }
    // A second reader is refused, and nothing is told again.
    assert_eq!(h.scene.connect(9, speaker()), Err(ErrorCode::Unauthorized));
    let events = logged(&mut h);
    assert!(events
        .iter()
        .all(|(_, event)| !matches!(event, Event::ScreenReaderState { .. })));
    // Super+F5 again stops it (or it quits, or crashes): every client still connected is told,
    // once.
    h.scene.disconnect(SPEAKER);
    assert!(!h.scene.screen_reader_running());
    let events = logged(&mut h);
    for client in [APP, OTHER, SHELL, LOCKER, READER] {
        assert_eq!(told(&events, client), [false], "client {client}");
    }
    assert_eq!(told(&events, SPEAKER), [] as [bool; 0]);
    // Another client going changes nothing.
    h.scene.disconnect(OTHER);
    let events = logged(&mut h);
    assert!(events
        .iter()
        .all(|(_, event)| !matches!(event, Event::ScreenReaderState { .. })));
}

#[test]
fn a_client_that_connects_while_the_reader_runs_is_told_at_once_and_only_it() {
    let mut h = Harness::new();
    h.scene.connect(SPEAKER, speaker()).unwrap();
    logged(&mut h);
    // An application started now is told as it connects, before it has a window; nobody else is
    // told again, nor it as its window comes.
    h.scene.connect(20, Grants::default()).unwrap();
    let events = logged(&mut h);
    assert_eq!(told(&events, 20), [true]);
    for client in [APP, OTHER, SHELL, LOCKER, SPEAKER] {
        assert_eq!(told(&events, client), [] as [bool; 0], "client {client}");
    }
    let window = h.toplevel(20, 1, 400, 300);
    h.activate(window);
    assert_eq!(told(&logged(&mut h), 20), [] as [bool; 0]);
    // Once it stopped, one that connects is told nothing: it reads that none runs.
    h.scene.disconnect(SPEAKER);
    logged(&mut h);
    h.scene.connect(21, Grants::default()).unwrap();
    assert_eq!(told(&logged(&mut h), 21), [] as [bool; 0]);
}

#[test]
fn the_screen_reader_granted_and_revoked_is_told_as_a_start_and_a_stop() {
    let mut h = Harness::new();
    h.scene.connect(SPEAKER, Grants::default()).unwrap();
    assert_eq!(told(&logged(&mut h), APP), [] as [bool; 0]);
    h.scene.grant(SPEAKER, speaker()).unwrap();
    let events = logged(&mut h);
    assert_eq!(told(&events, APP), [true]);
    assert_eq!(told(&events, SPEAKER), [true]);
    // Granted again: nothing new, nothing told.
    h.scene.grant(SPEAKER, speaker()).unwrap();
    assert_eq!(told(&logged(&mut h), APP), [] as [bool; 0]);
    // Its accessibility alone revoked: still the screen reader.
    h.scene.revoke(
        SPEAKER,
        Grants {
            accessibility: true,
            ..Grants::default()
        },
    );
    assert_eq!(told(&logged(&mut h), APP), [] as [bool; 0]);
    h.scene.revoke(SPEAKER, speaker());
    let events = logged(&mut h);
    assert_eq!(told(&events, APP), [false]);
    assert_eq!(told(&events, SPEAKER), [false]);
    assert!(!h.scene.screen_reader_running());
    // Gone with nothing to say: nobody is told again.
    h.scene.disconnect(SPEAKER);
    assert_eq!(told(&logged(&mut h), APP), [] as [bool; 0]);
}
