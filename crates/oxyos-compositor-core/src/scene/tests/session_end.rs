//! Sleep, restart, shut down and log out (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx):
//! the power key and Super+Shift+Q are the session's, and applications are asked to quit.

use super::*;
use oxyos_protocol::{SessionEnd, QUIT_REQUESTS_VERSION};

const CTRL: u32 = 29;
const SHIFT: u32 = 42;
const ALT: u32 = 56;
const Q: u32 = 16;
const IME_CLIENT: ClientId = 6;

fn press(h: &mut Harness, codes: &[u32]) {
    for code in codes {
        h.scene.keyboard(*code, true, None);
    }
    for code in codes.iter().rev() {
        h.scene.keyboard(*code, false, None);
    }
}

/// Every key, text or composition `client` was sent.
fn input_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<Event> {
    to(events, client)
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                Event::Key { .. }
                    | Event::KeyText { .. }
                    | Event::Text { .. }
                    | Event::Preedit { .. }
                    | Event::InputMethodKey { .. }
            )
        })
        .collect()
}

#[test]
fn the_power_key_reaches_no_one_locked_or_not_and_each_press_goes_to_the_program() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    press(&mut h, &[KEY_POWER]);
    // With modifiers held, Super's included, and through the path without a layout.
    press(&mut h, &[CTRL, KEY_POWER]);
    press(&mut h, &[SUPER, KEY_POWER]);
    h.scene.key(KEY_POWER, true);
    h.scene.key(KEY_POWER, false);
    let events = logged(&mut h);
    // Ctrl's and Super's own press and release are the application's; nothing of the power key.
    assert!(keys_to(&events, APP)
        .iter()
        .all(|(key, _)| *key != KEY_POWER));
    assert_eq!(h.scene.take_session_keys(), [SessionKey::Power; 4]);
    assert!(h.scene.take_session_keys().is_empty(), "taken once");
    // Locked: still the session's, and the lock screen is sent nothing of it.
    h.scene.lock();
    logged(&mut h);
    press(&mut h, &[KEY_POWER]);
    let events = logged(&mut h);
    for client in [APP, OTHER, SHELL, LOCKER] {
        assert_eq!(input_to(&events, client), [], "client {client}");
    }
    assert_eq!(h.scene.take_session_keys(), [SessionKey::Power]);
}

#[test]
fn the_input_method_never_sees_the_power_key() {
    let (mut h, window) = served("");
    press(&mut h, &[KEY_POWER]);
    let events = logged(&mut h);
    assert_eq!(input_to(&events, IME), []);
    assert_eq!(input_to(&events, window.client), []);
    assert_eq!(h.scene.take_session_keys(), [SessionKey::Power]);
}

#[test]
fn super_shift_q_logs_out_asked_and_with_alt_at_once_never_while_locked() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    press(&mut h, &[SUPER, SHIFT, Q]);
    press(&mut h, &[SUPER, ALT, SHIFT, Q]);
    let events = logged(&mut h);
    // Super's chords reach no application, and the window is not asked to close.
    assert!(keys_to(&events, APP).iter().all(|(key, _)| *key != Q));
    assert!(!to(&events, APP)
        .iter()
        .any(|event| matches!(event, Event::CloseRequested { .. })));
    assert_eq!(
        h.scene.take_session_keys(),
        [
            SessionKey::LogOut { confirm: true },
            SessionKey::LogOut { confirm: false }
        ]
    );
    // Ctrl with it is no log out; Super+Q alone quits the application, as before.
    press(&mut h, &[SUPER, CTRL, SHIFT, Q]);
    assert!(h.scene.take_session_keys().is_empty());
    press(&mut h, &[SUPER, Q]);
    assert!(h.scene.take_session_keys().is_empty());
    assert!(to(&logged(&mut h), APP)
        .iter()
        .any(|event| matches!(event, Event::CloseRequested { .. })));
    // Locked, no system shortcut acts.
    h.scene.lock();
    press(&mut h, &[SUPER, SHIFT, Q]);
    assert!(h.scene.take_session_keys().is_empty());
}

#[test]
fn applications_are_the_clients_without_the_sessions_own_grants() {
    let mut h = Harness::new();
    h.scene.connect(IME_CLIENT, input_method()).unwrap();
    let settings = 7;
    h.scene
        .connect(
            settings,
            Grants {
                settings: true,
                ..Grants::default()
            },
        )
        .unwrap();
    assert_eq!(h.scene.applications(), [APP, OTHER, settings]);
    assert!(h.scene.connected(APP));
    h.scene.disconnect(APP);
    assert!(!h.scene.connected(APP));
    assert_eq!(h.scene.applications(), [OTHER, settings]);
}

#[test]
fn a_new_application_is_asked_to_quit_and_an_older_ones_windows_to_close() {
    let mut h = Harness::new();
    let new = h.toplevel(APP, 1, 400, 300);
    let old = h.toplevel(OTHER, 1, 300, 200);
    let second = h.toplevel(OTHER, 2, 300, 200);
    h.scene.set_version(APP, QUIT_REQUESTS_VERSION);
    h.scene.set_version(OTHER, QUIT_REQUESTS_VERSION - 1);
    logged(&mut h);
    assert!(h.scene.ask_to_quit(APP, 5, SessionEnd::Restart));
    assert!(!h.scene.ask_to_quit(OTHER, 5, SessionEnd::Restart));
    let events = logged(&mut h);
    assert_eq!(
        to(&events, APP)
            .into_iter()
            .filter(|event| matches!(
                event,
                Event::QuitRequested { .. } | Event::CloseRequested { .. }
            ))
            .collect::<Vec<_>>(),
        [Event::QuitRequested {
            serial: 5,
            reason: SessionEnd::Restart
        }]
    );
    let closes: Vec<u32> = to(&events, OTHER)
        .into_iter()
        .filter_map(|event| match event {
            Event::CloseRequested { surface } => Some(surface),
            Event::QuitRequested { .. } => panic!("an older client is never sent it"),
            _ => None,
        })
        .collect();
    assert_eq!(closes, [old.id, second.id]);
    let _ = new;
    // A client that is gone is asked nothing.
    assert!(!h.scene.ask_to_quit(42, 5, SessionEnd::Restart));
}

#[test]
fn an_answer_is_taken_once_only_from_the_client_asked_with_its_serial() {
    let mut h = Harness::new();
    for client in [APP, OTHER] {
        h.scene.set_version(client, QUIT_REQUESTS_VERSION);
        assert!(h.scene.ask_to_quit(client, 9, SessionEnd::ShutDown));
    }
    // Another serial, or a client never asked (the shell), is ignored, never an error.
    h.request(
        APP,
        Request::QuitReply {
            serial: 8,
            quit: true,
        },
    )
    .unwrap();
    h.request(
        SHELL,
        Request::QuitReply {
            serial: 9,
            quit: true,
        },
    )
    .unwrap();
    assert!(h.scene.take_quit_replies().is_empty());
    h.request(
        APP,
        Request::QuitReply {
            serial: 9,
            quit: false,
        },
    )
    .unwrap();
    h.request(
        APP,
        Request::QuitReply {
            serial: 9,
            quit: true,
        },
    )
    .unwrap();
    assert_eq!(
        h.scene.take_quit_replies(),
        [QuitReplied {
            client: APP,
            serial: 9,
            quit: false
        }]
    );
    assert!(h.scene.take_quit_replies().is_empty(), "taken once");
    // Its answer stays though it goes at once; what was forgotten is answered by nobody.
    h.request(
        OTHER,
        Request::QuitReply {
            serial: 9,
            quit: true,
        },
    )
    .unwrap();
    h.scene.disconnect(OTHER);
    assert_eq!(h.scene.take_quit_replies().len(), 1);
    h.scene.set_version(APP, QUIT_REQUESTS_VERSION);
    assert!(h.scene.ask_to_quit(APP, 10, SessionEnd::LogOut));
    h.scene.forget_quit_requests();
    h.request(
        APP,
        Request::QuitReply {
            serial: 10,
            quit: true,
        },
    )
    .unwrap();
    assert!(h.scene.take_quit_replies().is_empty());
}

#[test]
fn the_power_key_held_is_never_pressed_again() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    h.scene.set_clock(std::time::Duration::from_secs(1));
    h.scene.keyboard_repeating(KEY_POWER, true, None, true);
    assert_eq!(h.scene.repeating_key(), None);
    assert_eq!(h.scene.next_repeat(), None);
    h.scene.set_clock(std::time::Duration::from_secs(5));
    h.scene.repeat_keys();
    assert_eq!(h.scene.take_session_keys(), [SessionKey::Power]);
}

#[test]
fn the_keyboards_surface_is_told_the_modifiers_as_they_go_down_and_up() {
    use oxyos_protocol::{MODIFIER_ALT, MODIFIER_SHIFT};
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.scene.set_version(APP, QUIT_REQUESTS_VERSION);
    h.activate(window);
    logged(&mut h);
    let flags = |events: &[(ClientId, Event)], client| -> Vec<u8> {
        to(events, client)
            .into_iter()
            .filter_map(|event| match event {
                Event::Modifiers { modifiers, .. } => Some(modifiers),
                _ => None,
            })
            .collect()
    };
    h.scene.key(ALT, true);
    h.scene.key(SHIFT, true);
    h.scene.key(ALT, false);
    // Another key changes no modifier: told nothing.
    h.scene.key(Q, true);
    h.scene.key(Q, false);
    h.scene.key(SHIFT, false);
    let events = logged(&mut h);
    assert_eq!(
        flags(&events, APP),
        [
            MODIFIER_ALT,
            MODIFIER_ALT | MODIFIER_SHIFT,
            MODIFIER_SHIFT,
            0
        ]
    );
    // Held as another surface takes the keyboard: it is told at once; a client of an older
    // version never is.
    let other = h.toplevel(OTHER, 1, 300, 200);
    h.scene.set_version(OTHER, QUIT_REQUESTS_VERSION);
    h.scene.key(ALT, true);
    logged(&mut h);
    h.activate(other);
    assert_eq!(flags(&logged(&mut h), OTHER), [MODIFIER_ALT]);
    h.scene.set_version(OTHER, QUIT_REQUESTS_VERSION - 1);
    h.scene.key(ALT, false);
    assert!(flags(&logged(&mut h), OTHER).is_empty());
}
