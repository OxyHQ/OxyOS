//! Mission Control and App Exposé (ADR 0045): Super+↑ and Super+↓ and the touchpad's three- and
//! four-finger swipes are the system's, told to the shell; a shell surface's windows moved into
//! its views.

use super::*;
use crate::scene::swipes::{FLICK, OVERLAY_DISTANCE};
use crate::scene::{DrawingPreferences, SwipeDrive, SwipeRelease, SwipeRest};
use oxyos_protocol::{GestureOverlay, ShortcutIntent};

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

/// What the shell was told of each shortcut, with its intent.
fn intents_to(
    events: &[(ClientId, Event)],
    client: ClientId,
) -> Vec<(SystemShortcut, ShortcutIntent)> {
    to(events, client)
        .iter()
        .filter_map(|event| match event {
            Event::SystemShortcut {
                shortcut, intent, ..
            } => Some((*shortcut, *intent)),
            _ => None,
        })
        .collect()
}

const MS: u64 = 1_000_000;

/// A swipe of `fingers` in `steps` equal steps by (dx, dy) logical pixels each, one every
/// `interval` milliseconds from `at` (ms), then lifted at once after the last.
fn swipe_at(
    h: &mut Harness,
    fingers: u8,
    steps: u32,
    (dx, dy): (i32, i32),
    at: u64,
    interval: u64,
) {
    begin(h, fingers, at);
    for step in 1..=steps {
        h.scene
            .set_input_time((at + u64::from(step) * interval) * MS);
        h.scene
            .pointer_swipe(GesturePhase::Update, fingers, dx * 256, dy * 256);
    }
    h.scene.pointer_swipe(GesturePhase::End, fingers, 0, 0);
}

fn begin(h: &mut Harness, fingers: u8, at: u64) {
    h.scene.set_input_time(at * MS);
    h.scene.pointer_swipe(GesturePhase::Begin, fingers, 0, 0);
}

fn update(h: &mut Harness, fingers: u8, (dx, dy): (i32, i32), at: u64) {
    h.scene.set_input_time(at * MS);
    h.scene
        .pointer_swipe(GesturePhase::Update, fingers, dx * 256, dy * 256);
}

/// A slow swipe (a step of 10 pixels each 100 ms, slower than any flick), as `swipe_at`.
fn swipe(h: &mut Harness, fingers: u8, steps: u32, (dx, dy): (i32, i32)) {
    swipe_at(h, fingers, steps, (dx, dy), 0, 100);
}

fn openness(h: &Harness) -> Option<(GestureOverlay, f64, Option<SwipeRelease>)> {
    match h.scene.swipe_drive()? {
        SwipeDrive::Overlay {
            overlay,
            openness,
            release,
            ..
        } => Some((overlay, openness, release)),
        SwipeDrive::Spaces { .. } => None,
    }
}

/// The shell's overlay (surface 9) shown, said to be `overlay`'s.
fn shown_overlay(h: &mut Harness, overlay: GestureOverlay) -> SurfaceKey {
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 9,
            kind: ShellKind::Overlay,
            edge: PanelEdge::None,
            output: 1,
            size: Size::new(SCREEN.width, SCREEN.height),
            exclusive: 0,
        },
    )
    .unwrap();
    h.request(
        SHELL,
        Request::SetGestureOverlay {
            surface: 9,
            overlay,
        },
    )
    .unwrap();
    h.present(key(SHELL, 9));
    key(SHELL, 9)
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
fn a_swipe_opens_as_the_fingers_go_and_the_window_under_them_hears_nothing() {
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    let at = h.rect(window);
    h.scene.pointer_motion(at.x + 50, at.y + 50);
    logged(&mut h);
    // Up, with three fingers: the shell is told to open Mission Control once the fingers chose up
    // (8 pixels), and the drive follows them, from where they began: 75 pixels is a quarter.
    begin(&mut h, 3, 0);
    update(&mut h, 3, (0, -5), 10);
    assert!(intents_to(&logged(&mut h), SHELL).is_empty());
    assert_eq!(h.scene.swipe_drive(), None);
    update(&mut h, 3, (1, -5), 20);
    assert_eq!(
        intents_to(&logged(&mut h), SHELL),
        [(SystemShortcut::MissionControl, ShortcutIntent::Open)]
    );
    for (step, quarter) in [(1, 0.25), (2, 0.5), (3, 0.75)] {
        // Each 75 pixels more, slowly.
        let travelled = if step == 1 { 65 } else { 75 };
        update(&mut h, 3, (0, -travelled), 20 + step * 200);
        let (overlay, open, release) = openness(&h).unwrap();
        assert_eq!((overlay, release), (GestureOverlay::MissionControl, None));
        assert!((open - quarter).abs() < 1e-9, "{open} at step {step}");
    }
    // Back down, and let go at a quarter, slowly: back, the shell told to close it.
    update(&mut h, 3, (0, 150), 1000);
    h.scene.set_input_time(1100 * MS);
    h.scene.pointer_swipe(GesturePhase::End, 3, 0, 0);
    let (_, open, release) = openness(&h).unwrap();
    assert!((open - 0.25).abs() < 1e-9);
    assert_eq!(
        release,
        Some(SwipeRelease {
            to: 0.0,
            velocity: 0.0
        })
    );
    let events = logged(&mut h);
    assert_eq!(
        intents_to(&events, SHELL),
        [(SystemShortcut::MissionControl, ShortcutIntent::Close)]
    );
    // The window under the fingers is told nothing of them.
    assert!(to(&events, APP).is_empty(), "{:?}", to(&events, APP));
    // Down past halfway, slowly: App Exposé opens and stays, its spring from where the fingers
    // were, at their speed.
    swipe(&mut h, 4, 18, (0, 10));
    let (overlay, open, release) = openness(&h).unwrap();
    assert_eq!(overlay, GestureOverlay::AppExpose);
    assert!((open - 0.6).abs() < 1e-9);
    let release = release.unwrap();
    assert_eq!(release.to, 1.0);
    // 10 pixels each 100 ms: a third of openness a second.
    assert!((release.velocity - 1.0 / 3.0).abs() < 1e-9, "{release:?}");
    assert_eq!(
        intents_to(&logged(&mut h), SHELL),
        [(SystemShortcut::AppExpose, ShortcutIntent::Open)]
    );
    // A fast, short flick (30 pixels in 30 ms, a sixth of the way): it goes through.
    swipe_at(&mut h, 3, 3, (0, -10), 5000, 10);
    let (_, open, release) = openness(&h).unwrap();
    assert!(open < 0.15);
    assert_eq!(release.unwrap().to, 1.0);
    assert!(release.unwrap().velocity >= FLICK / OVERLAY_DISTANCE);
    assert_eq!(
        intents_to(&logged(&mut h), SHELL),
        [(SystemShortcut::MissionControl, ShortcutIntent::Open)]
    );
    // Cancelled by libinput (a finger more): back, whatever how far.
    begin(&mut h, 3, 9000);
    update(&mut h, 3, (0, -200), 9100);
    h.scene.pointer_swipe(GesturePhase::Cancel, 3, 0, 0);
    assert_eq!(openness(&h).unwrap().2.unwrap().to, 0.0);
    assert_eq!(
        intents_to(&logged(&mut h), SHELL),
        [
            (SystemShortcut::MissionControl, ShortcutIntent::Open),
            (SystemShortcut::MissionControl, ShortcutIntent::Close)
        ]
    );
    // Two fingers are no swipe; and while locked, nothing.
    swipe(&mut h, 2, 20, (0, -10));
    assert!(intents_to(&logged(&mut h), SHELL).is_empty());
    h.scene.lock();
    logged(&mut h);
    swipe(&mut h, 3, 30, (0, -10));
    assert!(intents_to(&logged(&mut h), SHELL).is_empty());
}

#[test]
fn the_swipe_back_closes_what_is_open_and_the_same_way_again_does_nothing() {
    let mut h = Harness::new();
    dock(&mut h);
    shown_overlay(&mut h, GestureOverlay::MissionControl);
    logged(&mut h);
    // Up over Mission Control: nothing.
    swipe(&mut h, 3, 30, (0, -10));
    assert!(intents_to(&logged(&mut h), SHELL).is_empty());
    assert_eq!(h.scene.swipe_drive(), None);
    // Down, a third of the way, slowly: nothing told as it goes, and nothing when it goes back
    // open.
    begin(&mut h, 3, 0);
    update(&mut h, 3, (0, 100), 100);
    let (overlay, open, _) = openness(&h).unwrap();
    assert_eq!(overlay, GestureOverlay::MissionControl);
    assert!((open - 2.0 / 3.0).abs() < 1e-9);
    assert!(intents_to(&logged(&mut h), SHELL).is_empty());
    h.scene.set_input_time(400 * MS);
    h.scene.pointer_swipe(GesturePhase::End, 3, 0, 0);
    assert_eq!(openness(&h).unwrap().2.unwrap().to, 1.0);
    assert!(intents_to(&logged(&mut h), SHELL).is_empty());
    // Past halfway: closed, the shell told once the fingers let it go.
    swipe(&mut h, 3, 20, (0, 10));
    assert_eq!(openness(&h).unwrap().2.unwrap().to, 0.0);
    assert_eq!(
        intents_to(&logged(&mut h), SHELL),
        [(SystemShortcut::MissionControl, ShortcutIntent::Close)]
    );
    // Caught by the fingers as the compositor draws it closing (at 0.4): from there, not from
    // open, and up opens it again, the shell told so.
    h.scene.swipe_from(SwipeRest {
        openness: Some((GestureOverlay::MissionControl, 0.4)),
        position: None,
    });
    begin(&mut h, 3, 10_000);
    update(&mut h, 3, (0, -30), 10_100);
    assert!((openness(&h).unwrap().1 - 0.5).abs() < 1e-9);
    assert_eq!(
        intents_to(&logged(&mut h), SHELL),
        [(SystemShortcut::MissionControl, ShortcutIntent::Open)]
    );
}

#[test]
fn with_reduce_motion_nothing_follows_the_fingers_and_their_release_acts() {
    let mut h = Harness::new();
    dock(&mut h);
    h.scene.set_preferences(
        DrawingPreferences {
            dark: false,
            reduce_motion: true,
            reduce_transparency: false,
            accent: PreeditText::new("oxy").unwrap(),
            caret_blink: true,
        },
        &[],
    );
    logged(&mut h);
    // Short of halfway: nothing at all.
    swipe(&mut h, 3, 10, (0, -10));
    assert_eq!(h.scene.swipe_drive(), None);
    assert!(intents_to(&logged(&mut h), SHELL).is_empty());
    // Past it: told to open once the fingers lift, never as they go.
    begin(&mut h, 3, 0);
    update(&mut h, 3, (0, -200), 100);
    assert!(intents_to(&logged(&mut h), SHELL).is_empty());
    h.scene.set_input_time(400 * MS);
    h.scene.pointer_swipe(GesturePhase::End, 3, 0, 0);
    assert_eq!(h.scene.swipe_drive(), None);
    assert_eq!(
        intents_to(&logged(&mut h), SHELL),
        [(SystemShortcut::MissionControl, ShortcutIntent::Open)]
    );
}

#[test]
fn a_swipe_across_slides_the_desktops_and_brings_the_one_beside_in() {
    let mut h = Harness::new();
    dock(&mut h);
    let first = h.toplevel(APP, 1, 400, 300);
    h.scene.add_workspace();
    h.scene.add_workspace();
    h.scene.move_to_workspace(first, 0);
    let position = |h: &Harness| match h.scene.swipe_drive() {
        Some(SwipeDrive::Spaces {
            position, release, ..
        }) => (position, release),
        other => panic!("{other:?}"),
    };
    h.scene.take_appearances();
    // Fingers to the left bring the next desktop in from the right: a quarter of 400 pixels.
    begin(&mut h, 3, 0);
    update(&mut h, 3, (-100, 2), 100);
    assert!((position(&h).0 - 0.25).abs() < 1e-9);
    // The scene draws the next one's windows beside, never the others'.
    h.scene.show_beside(Some(1));
    h.scene.show_beside(Some(7));
    assert_eq!(h.scene.beside, None);
    // Let go past halfway, slowly: the next desktop is the active one, its windows not sliding
    // of their own (the compositor's motion takes them on), and the drive goes on to 1.
    update(&mut h, 3, (-150, 0), 300);
    h.scene.set_input_time(600 * MS);
    h.scene.pointer_swipe(GesturePhase::End, 3, 0, 0);
    assert_eq!(h.scene.workspaces(), (3, 1));
    assert_eq!(position(&h).1.unwrap().to, 1.0);
    assert!(h.scene.take_appearances().is_empty());
    // A flick back, short: the first again.
    swipe_at(&mut h, 3, 3, (12, 0), 1000, 10);
    assert_eq!(h.scene.workspaces(), (3, 0));
    // Past the first, it pulls against it: never past a seventh of a desktop, and back.
    begin(&mut h, 3, 2000);
    update(&mut h, 3, (300, 0), 2100);
    let (pulled, _) = position(&h);
    assert!((-0.15..-0.05).contains(&pulled), "{pulled}");
    h.scene.set_input_time(2500 * MS);
    h.scene.pointer_swipe(GesturePhase::End, 3, 0, 0);
    assert_eq!(position(&h).1.unwrap().to, 0.0);
    assert_eq!(h.scene.workspaces(), (3, 0));
    // Across over Mission Control: nothing.
    shown_overlay(&mut h, GestureOverlay::MissionControl);
    swipe(&mut h, 3, 30, (-10, 0));
    assert_eq!(h.scene.workspaces(), (3, 0));
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
    // Which view a swipe drives it is the shell's to say too (ADR 0046).
    h.request(
        SHELL,
        Request::SetGestureOverlay {
            surface: 9,
            overlay: GestureOverlay::AppExpose,
        },
    )
    .unwrap();
    assert_eq!(h.scene.gesture_overlay(overlay), GestureOverlay::AppExpose);
    assert_eq!(
        h.request(
            APP,
            Request::SetGestureOverlay {
                surface: window.id,
                overlay: GestureOverlay::MissionControl,
            },
        ),
        Err(ErrorCode::Unauthorized)
    );
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
