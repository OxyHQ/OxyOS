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
    for (index, (last, text)) in [(false, "so.oxy.dropsea\n"), (true, "so.oxy.inbox")]
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
    assert_eq!(changes[0].text, "so.oxy.dropsea\nso.oxy.inbox");
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
        ids: vec!["so.oxy.dropsea".into(), "so.oxy.inbox".into()],
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
    assert_eq!(told, "so.oxy.dropsea\nso.oxy.inbox");
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
            text: PreeditText::new("dropsea://trash").unwrap(),
        },
    )
    .unwrap();
    let asked = h.scene.take_url_requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].url, "dropsea://trash");
    assert!(!asked[0].with_input);
    // An application's too (protocol version 34): the program opens it, for the person's input.
    h.request(
        APP,
        Request::OpenUrl {
            serial: 3,
            input: 0,
            index: 0,
            last: true,
            text: PreeditText::new("dropsea://trash").unwrap(),
        },
    )
    .unwrap();
    let asked = h.scene.take_url_requests();
    assert_eq!((asked.len(), asked[0].serial), (1, 3));
}

#[test]
fn an_application_opens_a_url_by_its_scheme_and_never_a_file() {
    let mut h = Harness::new();
    let chunk = |text: &str| PreeditText::new(text).unwrap();
    let open = |serial, input, index, last, text| Request::OpenUrl {
        serial,
        input,
        index,
        last,
        text,
    };
    // In two chunks, the scheme in any case, citing no input the scene saw.
    h.request(APP, open(3, 7, 0, false, chunk("OxyOS-Settings://oxy-")))
        .unwrap();
    h.request(APP, open(3, 7, 1, true, chunk("account")))
        .unwrap();
    let asked = h.scene.take_url_requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].url, "OxyOS-Settings://oxy-account");
    assert_eq!(asked[0].serial, 3);
    assert!(!asked[0].with_input);
    // Any scheme an application's entry may declare (the program says whether one does).
    for url in ["dropsea://trash", "https://oxy.so", "x-oxy+mail.v2:to=ada"] {
        h.request(APP, open(4, 7, 0, true, chunk(url))).unwrap();
        assert_eq!(h.scene.take_url_requests()[0].url, url);
    }
    // Never a file (opened by its type), nor text that names no scheme.
    for url in [
        "file:///home/ada/notas.md",
        "FILE:/etc/passwd",
        "oxyos-settings",
        "/home/ada",
        "2fa://code",
        "://x",
    ] {
        assert_eq!(
            h.request(APP, open(4, 0, 0, true, chunk(url))),
            Err(ErrorCode::Unauthorized),
            "{url}"
        );
    }
    // A chunk that does not follow a first one naming a scheme is out of order.
    assert_eq!(
        h.request(APP, open(5, 0, 1, true, chunk("x://y"))),
        Err(ErrorCode::InvalidOrder)
    );
    // One URL at a time: beginning another drops the one unfinished.
    h.request(APP, open(6, 0, 0, false, chunk("oxyos-settings://")))
        .unwrap();
    h.request(APP, open(7, 0, 0, true, chunk("oxyos-settings://about")))
        .unwrap();
    assert_eq!(
        h.request(APP, open(6, 0, 1, true, chunk("about"))),
        Err(ErrorCode::InvalidOrder)
    );
    let asked = h.scene.take_url_requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].url, "oxyos-settings://about");
    assert!(crate::scene::dock::opens_by_scheme("oxyos-settings:"));
    assert!(!crate::scene::dock::opens_by_scheme("oxyos-settings"));
    assert!(!crate::scene::dock::opens_by_scheme("File:x"));
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

/// What the shell was told of each output's cover since the last call, in order.
fn covers_told(h: &mut Harness) -> Vec<(u32, bool)> {
    to(&logged(h), SHELL)
        .iter()
        .filter_map(|event| match event {
            Event::OutputCovered { output, covered } => Some((*output, *covered)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_shell_is_told_each_output_a_zoomed_or_full_screen_window_covers() {
    let mut h = Harness::new();
    h.scene
        .add_output(Output {
            id: 2,
            rect: Rect::new(1920, 0, 1280, 720),
            scale_120: 150,
            refresh_mhz: 60_000,
        })
        .unwrap();
    // Each output is told uncovered after its Output, once.
    assert_eq!(covers_told(&mut h), [(1, false), (2, false)]);
    let window = h.toplevel(APP, 1, 400, 300);
    let number = h.scene.window_number(window).unwrap();
    assert!(
        covers_told(&mut h).is_empty(),
        "a window not zoomed covers nothing"
    );
    let request = |h: &mut Harness, client, request| {
        h.request(client, request).unwrap();
        h.present(window);
    };
    let zoom = |h: &mut Harness, enabled| {
        request(
            h,
            APP,
            Request::SetMaximized {
                surface: 1,
                enabled,
            },
        );
    };
    // Zoomed, its output alone is covered; told once, however many frames follow.
    zoom(&mut h, true);
    assert_eq!(covers_told(&mut h), [(1, true)]);
    h.present(window);
    assert!(covers_told(&mut h).is_empty());
    // Minimized it covers nothing, and restored it covers its output again.
    request(&mut h, SHELL, Request::MinimizeWindow { window: number });
    assert_eq!(covers_told(&mut h), [(1, false)]);
    request(&mut h, SHELL, Request::FocusWindow { window: number });
    assert_eq!(covers_told(&mut h), [(1, true)]);
    // On a workspace not shown it covers nothing.
    request(&mut h, SHELL, Request::AddWorkspace {});
    request(&mut h, SHELL, Request::ActivateWorkspace { workspace: 1 });
    assert_eq!(covers_told(&mut h), [(1, false)]);
    request(&mut h, SHELL, Request::ActivateWorkspace { workspace: 0 });
    assert_eq!(covers_told(&mut h), [(1, true)]);
    // Hidden with its application, nor; activated again, it does.
    request(&mut h, SHELL, Request::HideApplication { window: number });
    assert_eq!(covers_told(&mut h), [(1, false)]);
    request(&mut h, SHELL, Request::FocusWindow { window: number });
    assert_eq!(covers_told(&mut h), [(1, true)]);
    // The lock leaves the desktop as it is beneath.
    h.scene.lock();
    h.pump();
    assert!(covers_told(&mut h).is_empty());
    h.scene.unlock();
    h.pump();
    assert!(covers_told(&mut h).is_empty());
    // Restored, uncovered.
    zoom(&mut h, false);
    assert_eq!(covers_told(&mut h), [(1, false)]);
    // Full screen covers it too, until it leaves full screen.
    let fullscreen = |h: &mut Harness, enabled| {
        request(
            h,
            APP,
            Request::SetFullscreen {
                surface: 1,
                enabled,
            },
        );
    };
    fullscreen(&mut h, true);
    assert_eq!(covers_told(&mut h), [(1, true)]);
    fullscreen(&mut h, false);
    assert_eq!(covers_told(&mut h), [(1, false)]);
    // A window on the other output covers that one alone; closed, it covers nothing.
    h.scene.pointer_motion(2000, 100);
    let right = h.toplevel(OTHER, 1, 400, 300);
    h.request(
        OTHER,
        Request::SetMaximized {
            surface: 1,
            enabled: true,
        },
    )
    .unwrap();
    h.present(right);
    assert_eq!(covers_told(&mut h), [(2, true)]);
    h.request(OTHER, Request::Destroy { surface: 1 }).unwrap();
    assert_eq!(covers_told(&mut h), [(2, false)]);
    // A shell that connects is told each output's cover as it is now.
    zoom(&mut h, true);
    logged(&mut h);
    h.scene.disconnect(SHELL);
    h.scene
        .connect(
            SHELL,
            Grants {
                shell: true,
                ..Grants::default()
            },
        )
        .unwrap();
    assert_eq!(covers_told(&mut h), [(1, true), (2, false)]);
}
