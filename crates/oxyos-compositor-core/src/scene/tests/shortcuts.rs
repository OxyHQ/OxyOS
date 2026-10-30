//! The system shortcuts (ADR 0024): Super and every key pressed while it is held are the
//! system's, the compositor's window actions, the shell's switcher and search, and what an
//! application is.

use super::*;
use crate::scene::AppearanceChange;

const RIGHT_SUPER: u32 = 126;
const SHIFT: u32 = 42;
const TAB: u32 = 15;
const Q: u32 = 16;
const W: u32 = 17;
const F: u32 = 33;
const H: u32 = 35;
const GRAVE: u32 = 41;
const M: u32 = 50;
/// Another shell, on a second output.
const SHELL_2: ClientId = 9;

/// The shell's dock, so it has a surface to be the shell of output 1 by.
fn dock(h: &mut Harness) {
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Bottom,
            output: 1,
            size: Size::new(1, 48),
            exclusive: 48,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
}

/// Keys pressed in order and released in reverse, through the keyboard's path.
fn keys(h: &mut Harness, codes: &[u32]) {
    for code in codes {
        h.scene.keyboard(*code, true, None);
    }
    for code in codes.iter().rev() {
        h.scene.keyboard(*code, false, None);
    }
}

/// Super with `codes`, pressed and released.
fn chord(h: &mut Harness, codes: &[u32]) {
    let mut all = vec![SUPER];
    all.extend_from_slice(codes);
    keys(h, &all);
}

fn shortcuts_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<(SystemShortcut, u32)> {
    to(events, client)
        .iter()
        .filter_map(|event| match event {
            Event::SystemShortcut {
                shortcut, output, ..
            } => Some((*shortcut, *output)),
            _ => None,
        })
        .collect()
}

fn close_requests(events: &[(ClientId, Event)]) -> Vec<SurfaceKey> {
    events
        .iter()
        .filter_map(|(client, event)| match event {
            Event::CloseRequested { surface } => Some(key(*client, *surface)),
            _ => None,
        })
        .collect()
}

/// Everything sent to `client` that is input: keys, text and composition.
fn input_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<Event> {
    to(events, client)
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                Event::Key { .. }
                    | Event::Text { .. }
                    | Event::Preedit { .. }
                    | Event::InputMethodKey { .. }
            )
        })
        .collect()
}

#[test]
fn super_chords_never_reach_applications_and_their_ctrl_chords_do() {
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    // Every shortcut and a chord that is none: nothing of any of them, press or release.
    for codes in [
        &[TAB][..],
        &[SHIFT, TAB],
        &[GRAVE],
        &[SPACE],
        &[CTRL, F],
        &[KEY_A],
        &[SHIFT, KEY_A],
    ] {
        chord(&mut h, codes);
    }
    let events = logged(&mut h);
    assert_eq!(input_to(&events, APP), []);
    assert!(close_requests(&events).is_empty());
    // An application's own chord (Bloom's Mod is Ctrl on OxyOS) is the application's, exactly.
    keys(&mut h, &[CTRL, W]);
    assert_eq!(
        keys_to(&logged(&mut h), APP),
        [(CTRL, true), (W, true), (W, false), (CTRL, false)]
    );
    // A modifier pressed before Super reaches it, press and release; what is pressed with Super
    // held does not, even released after Super.
    h.scene.keyboard(CTRL, true, None);
    h.scene.keyboard(RIGHT_SUPER, true, None);
    h.scene.keyboard(KEY_A, true, Some("a"));
    h.scene.keyboard(RIGHT_SUPER, false, None);
    h.scene.keyboard(KEY_A, false, None);
    h.scene.keyboard(CTRL, false, None);
    let events = logged(&mut h);
    assert_eq!(keys_to(&events, APP), [(CTRL, true), (CTRL, false)]);
    assert_eq!(input_to(&events, APP).len(), 2, "no text either");
    // With Super up again, keys are the application's.
    keys(&mut h, &[KEY_A]);
    assert_eq!(
        keys_to(&logged(&mut h), APP),
        [(KEY_A, true), (KEY_A, false)]
    );
}

#[test]
fn super_w_asks_the_active_window_alone_to_close() {
    let mut h = Harness::new();
    let other = h.toplevel(OTHER, 1, 400, 300);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    chord(&mut h, &[W]);
    let events = logged(&mut h);
    assert_eq!(close_requests(&events), [window]);
    assert_eq!(input_to(&events, APP), []);
    // The window is the application's to close (it may ask first): it stays until it goes.
    assert_eq!(h.scene.active_window(), Some(window));
    h.request(APP, Request::Destroy { surface: 1 }).unwrap();
    assert_eq!(h.scene.active_window(), Some(other));
    // With no window active (none left), nothing.
    h.request(OTHER, Request::Destroy { surface: 1 }).unwrap();
    logged(&mut h);
    chord(&mut h, &[W]);
    chord(&mut h, &[Q]);
    chord(&mut h, &[M]);
    chord(&mut h, &[H]);
    assert!(close_requests(&logged(&mut h)).is_empty());
}

#[test]
fn super_q_asks_every_window_of_the_active_application() {
    let mut h = Harness::new();
    let first = h.toplevel(APP, 1, 400, 300);
    let second = h.toplevel(APP, 2, 400, 300);
    let other = h.toplevel(OTHER, 1, 400, 300);
    // Windows claiming an id are that application's, whichever client; without one, its
    // client's alone.
    name(&mut h, first, "Uno", "so.oxy.allo");
    name(&mut h, second, "Dos", "so.oxy.allo");
    h.activate(first);
    logged(&mut h);
    chord(&mut h, &[Q]);
    let mut asked = close_requests(&logged(&mut h));
    asked.sort();
    assert_eq!(asked, [first, second]);
    name(&mut h, other, "Tres", "so.oxy.allo");
    chord(&mut h, &[Q]);
    let mut asked = close_requests(&logged(&mut h));
    asked.sort();
    assert_eq!(asked, [first, second, other]);
    // A launched process's windows are its registry application's, whatever they claim.
    h.scene.set_application(OTHER, "so.oxy.inbox");
    chord(&mut h, &[Q]);
    let mut asked = close_requests(&logged(&mut h));
    asked.sort();
    assert_eq!(asked, [first, second]);
    // A window that claims no id is its client's application alone.
    let unnamed = h.toplevel(OTHER, 2, 300, 200);
    let lone = h.toplevel(LOCKER, 1, 300, 200);
    h.activate(lone);
    logged(&mut h);
    chord(&mut h, &[Q]);
    assert_eq!(close_requests(&logged(&mut h)), [lone]);
    let _ = unnamed;
}

#[test]
fn super_m_minimizes_the_active_window_into_the_dock() {
    let mut h = Harness::new();
    dock(&mut h);
    let other = h.toplevel(OTHER, 1, 400, 300);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    h.scene.take_appearances();
    logged(&mut h);
    chord(&mut h, &[M]);
    let events = logged(&mut h);
    assert!(!h.rendered(window));
    assert_eq!(h.scene.focus(), Some(other));
    assert!(to(&events, SHELL).contains(&Event::WindowState {
        window: h.scene.window_number(window).unwrap(),
        workspace: 0,
        minimized: true,
        focused: false,
        launched: false,
    }));
    let changes: Vec<AppearanceChange> = h
        .scene
        .take_appearances()
        .iter()
        .map(|appearance| appearance.change)
        .collect();
    assert_eq!(changes, [AppearanceChange::Minimized]);
    assert_eq!(input_to(&events, APP), []);
}

#[test]
fn super_h_hides_the_application_until_it_is_activated() {
    let mut h = Harness::new();
    dock(&mut h);
    let other = h.toplevel(OTHER, 1, 400, 300);
    let first = h.toplevel(APP, 1, 400, 300);
    let second = h.toplevel(APP, 2, 400, 300);
    let minimized = h.toplevel(APP, 3, 400, 300);
    h.scene.minimize(minimized);
    h.activate(second);
    h.scene.take_appearances();
    logged(&mut h);
    chord(&mut h, &[H]);
    let events = logged(&mut h);
    assert!(!h.rendered(first) && !h.rendered(second));
    assert_eq!(h.scene.focus(), Some(other));
    let number = |h: &Harness, window| h.scene.window_number(window).unwrap();
    // The shell is told they are hidden with their application, not minimized into the dock;
    // the window minimized before stays as it was.
    for window in [first, second] {
        assert!(to(&events, SHELL).contains(&Event::WindowUse {
            window: number(&h, window),
            used: h.scene.window_use(window).0,
            hidden: true,
        }));
    }
    assert!(!to(&events, SHELL).iter().any(
        |event| matches!(event, Event::WindowUse { window, .. } if *window == number(&h, minimized))
    ));
    let gone: Vec<AppearanceChange> = h
        .scene
        .take_appearances()
        .iter()
        .map(|appearance| appearance.change)
        .collect();
    assert_eq!(gone, [AppearanceChange::Gone, AppearanceChange::Gone]);
    // Activated again (the shell focuses one of its windows, as the switcher and the dock do):
    // both come back, that one in front; the minimized one stays in the dock.
    h.request(
        SHELL,
        Request::FocusWindow {
            window: number(&h, first),
        },
    )
    .unwrap();
    let events = logged(&mut h);
    assert!(h.rendered(first) && h.rendered(second) && !h.rendered(minimized));
    assert_eq!(h.scene.focus(), Some(first));
    for window in [first, second] {
        assert!(to(&events, SHELL).iter().any(|event| matches!(
            event,
            Event::WindowUse { window: w, hidden: false, .. } if *w == number(&h, window)
        )));
    }
    let back: Vec<AppearanceChange> = h
        .scene
        .take_appearances()
        .iter()
        .map(|appearance| appearance.change)
        .collect();
    assert_eq!(back, [AppearanceChange::Arrived, AppearanceChange::Arrived]);
}

#[test]
fn super_backtick_goes_through_the_applications_windows() {
    let mut h = Harness::new();
    let a = h.toplevel(APP, 1, 400, 300);
    let b = h.toplevel(APP, 2, 400, 300);
    let other = h.toplevel(OTHER, 1, 400, 300);
    let c = h.toplevel(APP, 3, 400, 300);
    h.activate(c);
    logged(&mut h);
    // Forward: each press the next, the one left going behind the others, so every window comes.
    let mut seen = Vec::new();
    for _ in 0..3 {
        chord(&mut h, &[GRAVE]);
        seen.push(h.scene.focus().unwrap());
    }
    assert_eq!(seen, [b, a, c]);
    // Backwards: the one furthest behind.
    chord(&mut h, &[SHIFT, GRAVE]);
    assert_eq!(h.scene.focus(), Some(a));
    // Another application's window is never one of them; nor is a minimized one.
    h.scene.minimize(b);
    h.activate(a);
    chord(&mut h, &[GRAVE]);
    assert_eq!(h.scene.focus(), Some(c));
    assert_ne!(h.scene.focus(), Some(other));
    // One window alone: nothing.
    h.activate(other);
    chord(&mut h, &[GRAVE]);
    assert_eq!(h.scene.focus(), Some(other));
    assert_eq!(input_to(&logged(&mut h), APP), []);
}

#[test]
fn super_ctrl_f_toggles_full_screen() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let windowed = h.rect(window);
    h.activate(window);
    chord(&mut h, &[CTRL, F]);
    h.present(window);
    assert_eq!(h.rect(window), SCREEN);
    chord(&mut h, &[CTRL, F]);
    h.present(window);
    assert_eq!(h.rect(window), windowed);
    // Super+F alone is no shortcut.
    chord(&mut h, &[F]);
    h.present(window);
    assert_eq!(h.rect(window), windowed);
    assert_eq!(input_to(&logged(&mut h), APP), []);
}

#[test]
fn the_switcher_is_the_shells_while_super_is_held() {
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    h.scene.keyboard(SUPER, true, None);
    for codes in [
        &[TAB][..],
        &[TAB],
        &[SHIFT, TAB],
        &[GRAVE],
        &[RIGHT],
        &[LEFT],
        &[Q],
        &[H],
        &[W],
        &[M],
        &[ESCAPE],
    ] {
        keys(&mut h, codes);
    }
    // Cancelled, Super still held: Super+Shift+Tab opens it again, going backwards.
    keys(&mut h, &[SHIFT, TAB]);
    assert!(h.scene.switching());
    h.scene.keyboard(SUPER, false, None);
    assert!(!h.scene.switching());
    let events = logged(&mut h);
    use SystemShortcut::*;
    assert_eq!(
        shortcuts_to(&events, SHELL),
        [
            (SwitcherNext, 1),
            (SwitcherNext, 1),
            (SwitcherPrevious, 1),
            (SwitcherPrevious, 1),
            (SwitcherNext, 1),
            (SwitcherPrevious, 1),
            (SwitcherQuit, 1),
            (SwitcherHide, 1),
            (SwitcherCancel, 1),
            (SwitcherPrevious, 1),
            (SwitcherDone, 1),
        ]
    );
    // In the switcher, Q, H, W and M are its or nothing: no window was acted on.
    assert!(close_requests(&events).is_empty());
    assert!(h.rendered(window));
    assert_eq!(input_to(&events, APP), []);
    assert!(
        input_to(&events, SHELL).is_empty(),
        "the shell gets commands, not keys"
    );
    // Super released with no switcher: nothing. Each shortcut's serial is the shell's input.
    keys(&mut h, &[SUPER]);
    assert!(shortcuts_to(&logged(&mut h), SHELL).is_empty());
    keys(&mut h, &[SUPER, SPACE]);
    let events = logged(&mut h);
    let serial = to(&events, SHELL)
        .iter()
        .find_map(|event| match event {
            Event::SystemShortcut {
                shortcut: SystemShortcut::Search,
                serial,
                ..
            } => Some(*serial),
            _ => None,
        })
        .expect("search");
    assert!(h.scene.inputs[&SHELL].contains(&serial));
}

#[test]
fn the_switcher_opens_with_no_window_active_and_dismisses_a_menu() {
    let mut h = Harness::new();
    dock(&mut h);
    // No window at all: the shell is still told, and decides what to show.
    chord(&mut h, &[TAB]);
    assert_eq!(
        shortcuts_to(&logged(&mut h), SHELL),
        [
            (SystemShortcut::SwitcherNext, 1),
            (SystemShortcut::SwitcherDone, 1)
        ]
    );
    // A menu open in the window is dismissed, as a press elsewhere dismisses it, so the
    // switcher's choice can take the focus.
    let window = h.toplevel(APP, 1, 400, 300);
    let at = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (at.x + 100, at.y + 100),
        menu(Rect::new(90, 90, 20, 20), 200, 200),
    );
    logged(&mut h);
    h.scene.keyboard(SUPER, true, None);
    h.scene.keyboard(TAB, true, None);
    let events = logged(&mut h);
    assert!(to(&events, APP).contains(&Event::PopupDone {
        surface: popup.id,
        reason: DismissReason::OutsidePress,
    }));
    assert!(h.scene.grab_chain().is_empty());
}

#[test]
fn shortcuts_go_to_the_shell_of_the_active_windows_output() {
    let mut h = Harness::new();
    dock(&mut h);
    h.scene
        .add_output(Output {
            id: 2,
            rect: Rect::new(1920, 0, 1280, 720),
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    h.scene
        .connect(
            SHELL_2,
            Grants {
                shell: true,
                shortcuts: true,
                ..Grants::default()
            },
        )
        .unwrap();
    h.request(
        SHELL_2,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Background,
            edge: PanelEdge::None,
            output: 2,
            size: Size::new(1, 1),
            exclusive: 0,
        },
    )
    .unwrap();
    h.present(key(SHELL_2, 1));
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    chord(&mut h, &[SPACE]);
    let events = logged(&mut h);
    assert_eq!(shortcuts_to(&events, SHELL), [(SystemShortcut::Search, 1)]);
    assert!(shortcuts_to(&events, SHELL_2).is_empty());
    // Moved onto the second output, its shell shows them, the switcher to its end.
    h.scene.move_window_to(window, (2000, 100));
    h.present(window);
    logged(&mut h);
    chord(&mut h, &[SPACE]);
    chord(&mut h, &[TAB]);
    let events = logged(&mut h);
    assert!(shortcuts_to(&events, SHELL).is_empty());
    assert_eq!(
        shortcuts_to(&events, SHELL_2),
        [
            (SystemShortcut::Search, 2),
            (SystemShortcut::SwitcherNext, 2),
            (SystemShortcut::SwitcherDone, 2)
        ]
    );
    // A shell without the `shortcuts` grant is never told.
    h.scene.revoke(
        SHELL_2,
        Grants {
            shortcuts: true,
            ..Grants::default()
        },
    );
    chord(&mut h, &[SPACE]);
    let events = logged(&mut h);
    assert!(shortcuts_to(&events, SHELL_2).is_empty());
    assert_eq!(shortcuts_to(&events, SHELL), [(SystemShortcut::Search, 2)]);
}

#[test]
fn locked_no_shortcut_acts_and_their_keys_reach_nobody() {
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    // The switcher open as the session locks: the shell is told it was cancelled.
    h.scene.keyboard(SUPER, true, None);
    keys(&mut h, &[TAB]);
    logged(&mut h);
    h.scene.lock();
    assert_eq!(
        shortcuts_to(&logged(&mut h), SHELL),
        [(SystemShortcut::SwitcherCancel, 1)]
    );
    h.scene.keyboard(SUPER, false, None);
    h.request(
        LOCKER,
        Request::CreateLockSurface {
            surface: 1,
            output: 1,
        },
    )
    .unwrap();
    h.present(key(LOCKER, 1));
    assert_eq!(h.scene.focus(), Some(key(LOCKER, 1)));
    logged(&mut h);
    for codes in [
        &[W][..],
        &[Q],
        &[M],
        &[H],
        &[TAB],
        &[SPACE],
        &[GRAVE],
        &[CTRL, F],
        &[KEY_A],
    ] {
        chord(&mut h, codes);
    }
    let events = logged(&mut h);
    assert!(close_requests(&events).is_empty());
    assert!(shortcuts_to(&events, SHELL).is_empty());
    assert_eq!(
        input_to(&events, LOCKER),
        [],
        "not a letter into the password"
    );
    assert_eq!(input_to(&events, APP), []);
    assert!(h.scene.is_locked() && !h.scene.switching());
    // The lock's own keys are its: what is typed without Super.
    keys(&mut h, &[KEY_A]);
    assert_eq!(
        keys_to(&logged(&mut h), LOCKER),
        [(KEY_A, true), (KEY_A, false)]
    );
}

#[test]
fn the_input_method_never_sees_super_or_its_chords() {
    let (mut h, _) = served("");
    dock(&mut h);
    logged(&mut h);
    h.scene.keyboard(SUPER, true, None);
    h.scene.keyboard(SPACE, true, Some(" "));
    h.scene.keyboard(SPACE, false, None);
    h.scene.keyboard(KEY_A, true, Some("a"));
    h.scene.keyboard(KEY_A, false, None);
    h.scene.keyboard(SUPER, false, None);
    let events = logged(&mut h);
    assert_eq!(input_to(&events, IME), []);
    assert_eq!(input_to(&events, APP), []);
    assert_eq!(shortcuts_to(&events, SHELL), [(SystemShortcut::Search, 1)]);
    // Without Super, the input method has the key first, as before.
    h.scene.keyboard(KEY_A, true, Some("a"));
    assert_eq!(input_to(&logged(&mut h), IME).len(), 1);
}

#[test]
fn the_shell_focuses_its_own_surface_only_with_an_input_it_received() {
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 2,
            kind: ShellKind::Overlay,
            edge: PanelEdge::None,
            output: 1,
            size: Size::new(400, 200),
            exclusive: 0,
        },
    )
    .unwrap();
    h.present(key(SHELL, 2));
    // An overlay shown takes no focus by itself, nor with a serial the shell was never given.
    assert_eq!(h.scene.focus(), Some(window));
    h.request(
        SHELL,
        Request::FocusShellSurface {
            surface: 2,
            serial: h.scene.serial + 100,
        },
    )
    .unwrap();
    assert_eq!(h.scene.focus(), Some(window));
    // Nor with the application's own input.
    keys(&mut h, &[KEY_A]);
    let theirs = h.scene.serial - 1;
    h.request(
        SHELL,
        Request::FocusShellSurface {
            surface: 2,
            serial: theirs,
        },
    )
    .unwrap();
    assert_eq!(h.scene.focus(), Some(window));
    // With Super+Space's, it does: the launcher's search takes the keys.
    chord(&mut h, &[SPACE]);
    let serial = to(&logged(&mut h), SHELL)
        .iter()
        .find_map(|event| match event {
            Event::SystemShortcut { serial, .. } => Some(*serial),
            _ => None,
        })
        .unwrap();
    h.request(SHELL, Request::FocusShellSurface { surface: 2, serial })
        .unwrap();
    assert_eq!(h.scene.focus(), Some(key(SHELL, 2)));
    keys(&mut h, &[KEY_A]);
    assert_eq!(
        keys_to(&logged(&mut h), SHELL),
        [(KEY_A, true), (KEY_A, false)]
    );
    // Only the shell's own panels and overlays; only the shell.
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 3,
            kind: ShellKind::Background,
            edge: PanelEdge::None,
            output: 1,
            size: Size::new(1, 1),
            exclusive: 0,
        },
    )
    .unwrap();
    assert_eq!(
        h.request(SHELL, Request::FocusShellSurface { surface: 3, serial }),
        Err(ErrorCode::InvalidRole)
    );
    assert_eq!(
        h.request(APP, Request::FocusShellSurface { surface: 1, serial }),
        Err(ErrorCode::Unauthorized)
    );
}

#[test]
fn the_shell_hides_and_quits_applications_by_a_window_and_is_told_their_use() {
    let mut h = Harness::new();
    let first = h.toplevel(APP, 1, 400, 300);
    let second = h.toplevel(APP, 2, 400, 300);
    let other = h.toplevel(OTHER, 1, 400, 300);
    h.activate(first);
    h.activate(other);
    let number = |h: &Harness, window| h.scene.window_number(window).unwrap();
    // Each activation counts: the latest is the highest.
    let (first_used, _) = h.scene.window_use(first);
    let (other_used, _) = h.scene.window_use(other);
    assert!(other_used > first_used);
    assert!(h.scene.window_use(second).0 < first_used);
    logged(&mut h);
    h.request(
        SHELL,
        Request::QuitApplication {
            window: number(&h, second),
        },
    )
    .unwrap();
    let mut asked = close_requests(&logged(&mut h));
    asked.sort();
    assert_eq!(asked, [first, second]);
    h.request(
        SHELL,
        Request::HideApplication {
            window: number(&h, first),
        },
    )
    .unwrap();
    assert!(!h.rendered(first) && !h.rendered(second) && h.rendered(other));
    assert_eq!(h.scene.focus(), Some(other));
    // The application only through the shell.
    for request in [
        Request::HideApplication { window: 1 },
        Request::QuitApplication { window: 1 },
    ] {
        assert_eq!(h.request(APP, request), Err(ErrorCode::Unauthorized));
    }
    // A window gone takes its use with it; the shell is told it closed.
    h.request(APP, Request::Destroy { surface: 2 }).unwrap();
    assert_eq!(h.scene.window_use(second), (0, false));
}
