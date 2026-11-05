//! Mission Control and App Exposé (ADR 0045): Super+↑ and Super+↓ and the touchpad's three- and
//! four-finger swipes are the system's, told to the shell; a shell surface's windows moved into
//! its views.

use super::*;

const UP: u32 = 103;
const DOWN: u32 = 108;

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

fn shortcuts_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<SystemShortcut> {
    to(events, client)
        .iter()
        .filter_map(|event| match event {
            Event::SystemShortcut { shortcut, .. } => Some(*shortcut),
            _ => None,
        })
        .collect()
}

/// A swipe of `fingers` in `steps` equal steps by (dx, dy) logical pixels each, then lifted.
fn swipe(h: &mut Harness, fingers: u8, steps: u32, (dx, dy): (i32, i32)) {
    h.scene.pointer_swipe(GesturePhase::Begin, fingers, 0, 0);
    for _ in 0..steps {
        h.scene
            .pointer_swipe(GesturePhase::Update, fingers, dx * 256, dy * 256);
    }
    h.scene.pointer_swipe(GesturePhase::End, fingers, 0, 0);
}

#[test]
fn super_up_and_down_are_mission_control_and_app_expose_and_reach_no_application() {
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    let at = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (at.x + 100, at.y + 100),
        menu(Rect::new(90, 90, 20, 20), 200, 200),
    );
    logged(&mut h);
    for code in [UP, DOWN] {
        h.scene.keyboard(SUPER, true, None);
        h.scene.keyboard(code, true, None);
        h.scene.keyboard(code, false, None);
        h.scene.keyboard(SUPER, false, None);
    }
    let events = logged(&mut h);
    assert_eq!(
        shortcuts_to(&events, SHELL),
        [SystemShortcut::MissionControl, SystemShortcut::AppExpose]
    );
    // A menu open in the window closes first; the window hears no key of either.
    assert!(to(&events, APP).contains(&Event::PopupDone {
        surface: popup.id,
        reason: DismissReason::OutsidePress,
    }));
    assert!(!to(&events, APP)
        .iter()
        .any(|event| matches!(event, Event::Key { .. } | Event::Text { .. })));
    // The arrows alone are the window's.
    h.scene.keyboard(UP, true, None);
    h.scene.keyboard(UP, false, None);
    let events = logged(&mut h);
    assert!(shortcuts_to(&events, SHELL).is_empty());
    assert_eq!(keys_to(&events, APP), [(UP, true), (UP, false)]);
}

#[test]
fn three_or_four_fingers_swiped_up_or_down_tell_the_shell_once_and_no_application_anything() {
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    let at = h.rect(window);
    h.scene.pointer_motion(at.x + 50, at.y + 50);
    logged(&mut h);
    // Up, with three fingers: Mission Control, once however far the fingers go.
    swipe(&mut h, 3, 12, (1, -10));
    // Down, with four: App Exposé.
    swipe(&mut h, 4, 12, (-1, 10));
    let events = logged(&mut h);
    assert_eq!(
        shortcuts_to(&events, SHELL),
        [SystemShortcut::MissionControl, SystemShortcut::AppExpose]
    );
    // The window under the fingers is told nothing of them.
    assert!(to(&events, APP).is_empty(), "{:?}", to(&events, APP));
    // Not far enough (39 of the 40 pixels), across rather than along, or cancelled before the
    // distance: nothing.
    swipe(&mut h, 3, 3, (0, 13));
    swipe(&mut h, 3, 10, (12, 10));
    h.scene.pointer_swipe(GesturePhase::Begin, 3, 0, 0);
    h.scene.pointer_swipe(GesturePhase::Update, 3, 0, 20 * 256);
    h.scene.pointer_swipe(GesturePhase::Cancel, 3, 0, 0);
    h.scene.pointer_swipe(GesturePhase::Update, 3, 0, 30 * 256);
    let events = logged(&mut h);
    assert!(shortcuts_to(&events, SHELL).is_empty());
    assert!(to(&events, APP).is_empty());
    // Exactly the distance acts.
    swipe(&mut h, 3, 4, (0, 10));
    assert_eq!(
        shortcuts_to(&logged(&mut h), SHELL),
        [SystemShortcut::AppExpose]
    );
    // While locked, nothing.
    h.scene.lock();
    logged(&mut h);
    swipe(&mut h, 3, 12, (0, -10));
    assert!(shortcuts_to(&logged(&mut h), SHELL).is_empty());
}

#[test]
fn only_the_shell_moves_windows_into_its_own_surfaces_views() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 640, 400);
    let number = h.scene.window_number(window).unwrap();
    let overlay = overview(&mut h);
    let card = Rect::new(100, 100, 320, 200);
    let moved = |scene: &Scene| {
        scene
            .frame_list()
            .into_iter()
            .filter_map(|item| match item {
                RenderItem::View { moved, .. } => Some(moved),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    place_views(&mut h, &[(number, card, card)]).unwrap();
    h.present(overlay);
    // A surface's views are copies until the shell moves its windows into them.
    assert_eq!(moved(&h.scene), [false]);
    h.request(
        SHELL,
        Request::MoveWindowsToViews {
            surface: 9,
            moved: true,
        },
    )
    .unwrap();
    assert_eq!(moved(&h.scene), [true]);
    // The window itself is still the scene's, where it is: the compositor leaves it out of its
    // place while its view is shown.
    assert!(h.rendered(window));
    h.request(
        SHELL,
        Request::MoveWindowsToViews {
            surface: 9,
            moved: false,
        },
    )
    .unwrap();
    assert_eq!(moved(&h.scene), [false]);
    // An application may not, even on its own window; nor the shell on what is no shell surface.
    assert_eq!(
        h.request(
            APP,
            Request::MoveWindowsToViews {
                surface: window.id,
                moved: true,
            },
        ),
        Err(ErrorCode::Unauthorized)
    );
    h.request(
        SHELL,
        Request::CreateToplevel {
            surface: 20,
            size: Size::new(100, 100),
        },
    )
    .ok();
    assert!(matches!(
        h.request(
            SHELL,
            Request::MoveWindowsToViews {
                surface: 20,
                moved: true,
            },
        ),
        Err(ErrorCode::InvalidRole | ErrorCode::UnknownSurface)
    ));
}
