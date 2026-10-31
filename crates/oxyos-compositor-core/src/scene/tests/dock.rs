//! The dock's application menu (ADR 0034): pings and whether a window's process responds, force
//! quitting by a window, the person's lists of applications and URLs the shell opens.

use super::*;
use crate::scene::{ApplicationList, PING_TIMEOUT};
use oxyos_protocol::{ApplicationListKind, MODIFIER_ALT, MODIFIER_CONTROL};
use std::time::Duration;

fn pings_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<u32> {
    to(events, client)
        .iter()
        .filter_map(|event| match event {
            Event::Ping { serial } => Some(*serial),
            _ => None,
        })
        .collect()
}

/// Answers every Ping the log holds, as each client's library does, and clears the log.
fn answer_pings(h: &mut Harness) {
    for (client, event) in logged(h) {
        if let Event::Ping { serial } = event {
            h.request(client, Request::Pong { serial }).unwrap();
        }
    }
    logged(h);
}

/// A press and release in the middle of `window`.
fn click(h: &mut Harness, window: SurfaceKey) {
    let rect = h.rect(window);
    h.press(
        rect.x + rect.width as i32 / 2,
        rect.y + rect.height as i32 / 2,
    );
    h.scene.pointer_button(BUTTON, false);
    h.pump();
}

fn responding_told(events: &[(ClientId, Event)], window: u32) -> Vec<bool> {
    to(events, SHELL)
        .iter()
        .filter_map(|event| match event {
            Event::WindowState {
                window: w,
                responding,
                ..
            } if *w == window => Some(*responding),
            _ => None,
        })
        .collect()
}

#[test]
fn a_client_given_a_press_is_pinged_once_and_is_not_responding_until_it_answers() {
    let mut h = Harness::new();
    h.scene.set_version(APP, 8);
    let window = h.toplevel(APP, 1, 400, 300);
    let number = h.scene.window_number(window).unwrap();
    h.scene.set_clock(Duration::from_secs(10));
    // Mapped, it was activated, and pinged for it.
    answer_pings(&mut h);
    click(&mut h, window);
    let events = logged(&mut h);
    let pings = pings_to(&events, APP);
    assert_eq!(pings.len(), 1, "one ping for the press");
    // Another press while it is unanswered pings no more.
    click(&mut h, window);
    assert!(pings_to(&logged(&mut h), APP).is_empty());
    // Within the time it is still responding.
    h.scene
        .set_clock(Duration::from_secs(10) + PING_TIMEOUT - Duration::from_millis(1));
    h.scene.check_responding();
    assert!(responding_told(&logged(&mut h), number).is_empty());
    assert_eq!(h.scene.next_ping_deadline(), Some(Duration::from_millis(1)));
    // Late: the shell is told its window is not responding.
    h.scene.set_clock(Duration::from_secs(10) + PING_TIMEOUT);
    h.scene.check_responding();
    assert_eq!(responding_told(&logged(&mut h), number), [false]);
    assert!(!h.scene.responding(APP));
    assert_eq!(h.scene.next_ping_deadline(), None);
    // A Pong naming another serial changes nothing; its own does.
    h.request(
        APP,
        Request::Pong {
            serial: pings[0] + 7,
        },
    )
    .unwrap();
    assert!(!h.scene.responding(APP));
    h.request(APP, Request::Pong { serial: pings[0] }).unwrap();
    assert_eq!(responding_told(&logged(&mut h), number), [true]);
    // Answered, the next press pings again.
    click(&mut h, window);
    assert_eq!(pings_to(&logged(&mut h), APP).len(), 1);
}

#[test]
fn a_client_of_an_older_version_is_never_pinged_and_always_responding() {
    let mut h = Harness::new();
    h.scene.set_version(APP, 7);
    let window = h.toplevel(APP, 1, 400, 300);
    logged(&mut h);
    click(&mut h, window);
    h.request(
        SHELL,
        Request::CloseWindow {
            window: h.scene.window_number(window).unwrap(),
        },
    )
    .unwrap();
    assert!(pings_to(&logged(&mut h), APP).is_empty());
    h.scene.set_clock(Duration::from_secs(3600));
    h.scene.check_responding();
    assert!(h.scene.responding(APP));
}

#[test]
fn a_close_request_and_an_activation_ping_and_a_disconnect_forgets_the_ping() {
    let mut h = Harness::new();
    h.scene.set_version(APP, 8);
    h.scene.set_version(OTHER, 8);
    let window = h.toplevel(APP, 1, 400, 300);
    let other = h.toplevel(OTHER, 1, 400, 300);
    answer_pings(&mut h);
    // The other window was mapped last and is the active one: activating this one pings it.
    h.activate(window);
    assert_eq!(pings_to(&logged(&mut h), APP).len(), 1);
    h.request(
        SHELL,
        Request::CloseWindow {
            window: h.scene.window_number(other).unwrap(),
        },
    )
    .unwrap();
    assert_eq!(pings_to(&logged(&mut h), OTHER).len(), 1);
    h.scene.disconnect(APP);
    h.scene.set_clock(Duration::from_secs(3600));
    h.scene.check_responding();
    assert!(h.scene.responding(APP));
    assert!(!h.scene.responding(OTHER));
}

#[test]
fn force_quit_names_the_launched_clients_of_the_windows_application_and_no_other() {
    let mut h = Harness::new();
    let first = h.toplevel(APP, 1, 400, 300);
    let other = h.toplevel(OTHER, 1, 400, 300);
    // Only a window whose process the session started may be force-quit.
    h.request(
        SHELL,
        Request::ForceQuitApplication {
            window: h.scene.window_number(first).unwrap(),
        },
    )
    .unwrap();
    assert!(h.scene.take_force_quits().is_empty());
    h.scene.set_application(APP, "so.oxy.inbox");
    h.scene.set_application(OTHER, "so.oxy.allo");
    h.request(
        SHELL,
        Request::ForceQuitApplication {
            window: h.scene.window_number(first).unwrap(),
        },
    )
    .unwrap();
    assert_eq!(h.scene.take_force_quits(), [vec![APP]]);
    // The same application in two processes: both.
    h.scene.set_application(OTHER, "so.oxy.inbox");
    h.request(
        SHELL,
        Request::ForceQuitApplication {
            window: h.scene.window_number(other).unwrap(),
        },
    )
    .unwrap();
    assert_eq!(h.scene.take_force_quits(), [vec![APP, OTHER]]);
    // Only the shell asks; an unknown window is nothing.
    assert_eq!(
        h.request(APP, Request::ForceQuitApplication { window: 1 }),
        Err(ErrorCode::Unauthorized)
    );
    h.request(SHELL, Request::ForceQuitApplication { window: 999 })
        .unwrap();
    assert!(h.scene.take_force_quits().is_empty());
}

#[test]
fn the_shells_list_is_handed_whole_and_every_shell_is_told_the_programs() {
    let mut h = Harness::new();
    for (index, (last, text)) in [(false, "so.oxy.stash\n"), (true, "so.oxy.inbox")]
        .into_iter()
        .enumerate()
    {
        h.request(
            SHELL,
            Request::SetApplicationList {
                serial: 4,
                list: ApplicationListKind::Dock,
                index: index as u16,
                last,
                text: PreeditText::new(text).unwrap(),
            },
        )
        .unwrap();
    }
    let changes = h.scene.take_application_list_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].serial, 4);
    assert_eq!(changes[0].list, ApplicationListKind::Dock);
    assert_eq!(changes[0].text, "so.oxy.stash\nso.oxy.inbox");
    // Only the shell changes them.
    assert_eq!(
        h.request(
            APP,
            Request::SetApplicationList {
                serial: 1,
                list: ApplicationListKind::Dock,
                index: 0,
                last: true,
                text: PreeditText::default(),
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
    logged(&mut h);
    let list = ApplicationList {
        chosen: true,
        ids: vec!["so.oxy.stash".into(), "so.oxy.inbox".into()],
    };
    h.scene
        .set_application_list(ApplicationListKind::Dock, list.clone());
    let told: String = to(&logged(&mut h), SHELL)
        .iter()
        .filter_map(|event| match event {
            Event::ApplicationList {
                list: ApplicationListKind::Dock,
                chosen: true,
                text,
                ..
            } => Some(text.as_str().to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(told, "so.oxy.stash\nso.oxy.inbox");
    // The same again tells nothing; a shell connecting later is told it.
    h.scene
        .set_application_list(ApplicationListKind::Dock, list);
    assert!(to(&logged(&mut h), SHELL).is_empty());
    h.scene
        .connect(
            9,
            Grants {
                shell: true,
                ..Grants::default()
            },
        )
        .unwrap();
    assert!(to(&logged(&mut h), 9)
        .iter()
        .any(|event| matches!(event, Event::ApplicationList { last: true, .. })));
}

#[test]
fn a_url_is_handed_to_the_program_with_whether_it_cited_an_input() {
    let mut h = Harness::new();
    h.request(
        SHELL,
        Request::OpenUrl {
            serial: 2,
            input: 0,
            index: 0,
            last: true,
            text: PreeditText::new("stash://trash").unwrap(),
        },
    )
    .unwrap();
    let asked = h.scene.take_url_requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].url, "stash://trash");
    assert!(!asked[0].with_input);
    assert_eq!(
        h.request(
            APP,
            Request::OpenUrl {
                serial: 2,
                input: 0,
                index: 0,
                last: true,
                text: PreeditText::new("stash://trash").unwrap(),
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
}

#[test]
fn a_pointer_event_carries_the_modifiers_held_whoever_has_the_keyboard() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let other = h.toplevel(OTHER, 1, 400, 300);
    h.activate(other);
    logged(&mut h);
    h.scene.keyboard(29, true, None);
    h.scene.keyboard(56, true, None);
    click(&mut h, window);
    let modifiers: Vec<u8> = logged(&mut h)
        .iter()
        .filter_map(|(_, event)| match event {
            Event::Pointer {
                kind: PointerKind::Press,
                modifiers,
                ..
            } => Some(*modifiers),
            _ => None,
        })
        .collect();
    assert_eq!(modifiers, [MODIFIER_CONTROL | MODIFIER_ALT]);
    // Left and right are each their own key: one released, the other still holds.
    h.scene.keyboard(97, true, None);
    h.scene.keyboard(29, false, None);
    h.scene.keyboard(56, false, None);
    click(&mut h, window);
    let modifiers: Vec<u8> = logged(&mut h)
        .iter()
        .filter_map(|(_, event)| match event {
            Event::Pointer {
                kind: PointerKind::Press,
                modifiers,
                ..
            } => Some(*modifiers),
            _ => None,
        })
        .collect();
    assert_eq!(modifiers, [MODIFIER_CONTROL]);
}
