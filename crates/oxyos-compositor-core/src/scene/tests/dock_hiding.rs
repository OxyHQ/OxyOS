//! The dock hiding automatically (docs/adr/0034-the-dock.mdx, 10 November 2026): the preference
//! the program applies, the strip it no longer keeps, the bottom edge bringing it up, what holds it
//! there, and Super+Alt+D asking the program to turn it over.

use super::*;
use crate::scene::AppearanceChange;
use std::time::Duration;

const ALT: u32 = 56;
const D: u32 = 32;

/// The shell's dock on output 1 (1920 × 1080): 64 tall at the bottom, keeping its strip.
fn dock(h: &mut Harness) -> SurfaceKey {
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Bottom,
            output: 1,
            size: Size::new(1, 64),
            exclusive: 64,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
    key(SHELL, 1)
}

fn changes(h: &mut Harness, of: SurfaceKey) -> Vec<AppearanceChange> {
    h.scene
        .take_appearances()
        .iter()
        .filter(|appearance| appearance.key == of)
        .map(|appearance| appearance.change)
        .collect()
}

#[test]
fn on_the_dock_keeps_no_strip_and_slides_off_until_the_pointer_reaches_the_bottom_edge() {
    let mut h = Harness::new();
    let dock = dock(&mut h);
    assert_eq!(h.scene.work_area(1), Some(Rect::new(0, 0, 1920, 1016)));
    h.scene.pointer_motion(900, 500);
    h.scene.take_appearances();
    h.scene.set_dock_hiding(true);
    // No strip kept: a window zoomed fills the screen to its bottom; the dock goes down.
    assert_eq!(h.scene.work_area(1), Some(Rect::new(0, 0, 1920, 1080)));
    assert!(h.scene.dock_hidden(dock) && !h.rendered(dock));
    assert_eq!(changes(&mut h, dock), [AppearanceChange::EdgeHidden]);
    // Near the edge is not at it.
    h.scene.pointer_motion(900, 1078);
    assert!(h.scene.dock_hidden(dock));
    // The last row of pixels brings it up, under the pointer.
    h.scene.pointer_motion(900, 1079);
    assert!(!h.scene.dock_hidden(dock) && h.rendered(dock));
    assert_eq!(changes(&mut h, dock), [AppearanceChange::EdgeShown]);
    assert_eq!(h.scene.pointer_focus(), Some(dock));
    // Over it, it stays; it keeps no strip even shown, coming over the windows.
    h.scene.pointer_motion(300, 1020);
    assert!(!h.scene.dock_hidden(dock));
    assert_eq!(h.scene.work_area(1), Some(Rect::new(0, 0, 1920, 1080)));
    // Off it, it goes down again, the pointer no longer over it.
    h.scene.pointer_motion(300, 1000);
    assert!(h.scene.dock_hidden(dock));
    assert_eq!(changes(&mut h, dock), [AppearanceChange::EdgeHidden]);
    assert_ne!(h.scene.pointer_focus(), Some(dock));
    // Off: shown again, keeping its strip.
    h.scene.set_dock_hiding(false);
    assert!(!h.scene.dock_hidden(dock) && h.rendered(dock));
    assert_eq!(changes(&mut h, dock), [AppearanceChange::EdgeShown]);
    assert_eq!(h.scene.work_area(1), Some(Rect::new(0, 0, 1920, 1016)));
    // Off, the edge and the pointer move nothing.
    h.scene.pointer_motion(300, 500);
    assert!(!h.scene.dock_hidden(dock));
    assert!(changes(&mut h, dock).is_empty());
}

#[test]
fn a_window_minimized_brings_the_dock_up_for_a_moment() {
    let mut h = Harness::new();
    let dock = dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    h.scene.pointer_motion(900, 100);
    h.scene.set_clock(Duration::from_secs(10));
    h.scene.set_dock_hiding(true);
    h.scene.take_appearances();
    h.scene.minimize(window);
    // Up for the genie, which goes into it.
    assert!(!h.scene.dock_hidden(dock));
    assert_eq!(changes(&mut h, dock), [AppearanceChange::EdgeShown]);
    assert_eq!(h.scene.next_dock_deadline(), Some(Duration::from_secs(1)));
    // Not yet down within its moment, then down once it passed.
    h.scene.set_clock(Duration::from_millis(10_900));
    h.scene.follow_dock_hiding();
    assert!(!h.scene.dock_hidden(dock));
    h.scene.set_clock(Duration::from_secs(11));
    h.scene.follow_dock_hiding();
    assert!(h.scene.dock_hidden(dock));
    assert_eq!(h.scene.next_dock_deadline(), None);
}

#[test]
fn super_alt_d_asks_the_program_to_turn_it_over_and_reaches_no_application() {
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    for code in [SUPER, ALT, D] {
        h.scene.keyboard(code, true, None);
    }
    for code in [D, ALT, SUPER] {
        h.scene.keyboard(code, false, None);
    }
    assert_eq!(h.scene.take_dock_hiding_toggles(), 1);
    assert_eq!(h.scene.take_dock_hiding_toggles(), 0);
    assert!(!keys_to(&logged(&mut h), APP)
        .iter()
        .any(|(code, _)| *code == D));
    // The scene changes nothing by itself: the program keeps the preference and applies it.
    assert!(!h.scene.dock_hiding());
    // Super+D alone, or with Ctrl, is no shortcut.
    for codes in [&[SUPER, D][..], &[SUPER, ALT, CTRL, D]] {
        for code in codes {
            h.scene.keyboard(*code, true, None);
        }
        for code in codes.iter().rev() {
            h.scene.keyboard(*code, false, None);
        }
    }
    assert_eq!(h.scene.take_dock_hiding_toggles(), 0);
}

#[test]
fn locked_nothing_moves_beneath_the_lock() {
    let mut h = Harness::new();
    let dock = dock(&mut h);
    h.scene.pointer_motion(900, 500);
    h.scene.set_dock_hiding(true);
    assert!(h.scene.dock_hidden(dock));
    h.scene.lock();
    h.scene.pointer_motion(900, 1079);
    assert!(h.scene.dock_hidden(dock));
}

#[test]
fn super_f3_brings_it_up_with_the_keyboard_and_it_stays_while_it_has_it() {
    const F3: u32 = 61;
    let mut h = Harness::new();
    let dock = dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    h.scene.pointer_motion(900, 100);
    h.scene.set_dock_hiding(true);
    assert!(h.scene.dock_hidden(dock));
    logged(&mut h);
    for code in [SUPER, F3] {
        h.scene.keyboard(code, true, None);
    }
    for code in [F3, SUPER] {
        h.scene.keyboard(code, false, None);
    }
    let serial = to(&logged(&mut h), SHELL)
        .iter()
        .find_map(|event| match event {
            Event::SystemShortcut { serial, .. } => Some(*serial),
            _ => None,
        })
        .unwrap();
    h.scene.take_appearances();
    h.request(SHELL, Request::FocusShellSurface { surface: 1, serial })
        .unwrap();
    assert!(!h.scene.dock_hidden(dock));
    assert_eq!(h.scene.focus(), Some(dock));
    assert_eq!(changes(&mut h, dock), [AppearanceChange::EdgeShown]);
    // A turn, the pointer far from it: it stays while it has the keys.
    h.scene.follow_dock_hiding();
    h.scene.pointer_motion(900, 200);
    assert!(!h.scene.dock_hidden(dock));
    // The keys back with the window: it goes down.
    h.activate(window);
    h.scene.follow_dock_hiding();
    assert!(h.scene.dock_hidden(dock));
}
