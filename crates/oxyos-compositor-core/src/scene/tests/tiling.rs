//! Window tiling (docs/adr/0016-windows.mdx, decision 12): a window dragged to a display's edges,
//! the preview beneath it, Alt anywhere, a drag away giving back its size, Super+Alt's keys, and
//! tiles kept through zoom, resizes, displays and their scale.

use super::*;
use crate::scene::tiling::{Layout, Tile, TILE_MARGIN};

const ALT: u32 = 56;
const SHIFT: u32 = 42;
const LEFT: u32 = 105;
const RIGHT: u32 = 106;
const UP: u32 = 103;
const DOWN: u32 = 108;
const F: u32 = 33;
const C: u32 = 46;
const R: u32 = 19;

/// The shell's menu bar (32 px) and dock (48 px), whose exclusive zones the work area leaves out.
fn bars(h: &mut Harness) {
    for (surface, edge, height) in [(1, PanelEdge::Top, 32), (2, PanelEdge::Bottom, 48)] {
        h.request(
            SHELL,
            Request::CreateShellSurface {
                surface,
                kind: ShellKind::Panel,
                edge,
                output: 1,
                size: Size::new(1, height),
                exclusive: height,
            },
        )
        .unwrap();
        h.present(key(SHELL, surface));
    }
}

/// The work area between the bars.
const AREA: Rect = Rect::new(0, 32, 1920, 1000);

/// A press on the window's top band (40 px in from its left, 20 down) that its client answers
/// with `BeginMove`, as Bloom's drag region does.
fn grab(h: &mut Harness, window: SurfaceKey) {
    let at = h.rect(window);
    let serial = h.press(at.x + 40, at.y + 20);
    h.request(
        window.client,
        Request::BeginMove {
            surface: window.id,
            serial,
        },
    )
    .unwrap();
    assert_eq!(h.scene.moving_window(), Some(window));
}

fn release(h: &mut Harness) {
    h.scene.pointer_button(BUTTON, false);
    h.pump();
}

fn preview(h: &Harness) -> Option<(usize, Rect)> {
    h.scene
        .render_list()
        .into_iter()
        .enumerate()
        .find_map(|(index, item)| match item {
            RenderItem::TilePreview { rect } => Some((index, rect)),
            _ => None,
        })
}

fn index_of(h: &Harness, surface: SurfaceKey) -> usize {
    h.scene
        .render_list()
        .iter()
        .position(|item| matches!(item, RenderItem::Surface { key, .. } if *key == surface))
        .expect("rendered")
}

/// Super with `codes`, pressed in order and released in reverse.
fn chord(h: &mut Harness, codes: &[u32]) {
    let mut all = vec![SUPER];
    all.extend_from_slice(codes);
    for code in &all {
        h.scene.keyboard(*code, true, None);
    }
    for code in all.iter().rev() {
        h.scene.keyboard(*code, false, None);
    }
    h.pump();
}

fn keys_to(h: &Harness, client: ClientId) -> usize {
    h.events_for(client)
        .iter()
        .filter(|event| matches!(event, Event::Key { .. } | Event::Text { .. }))
        .count()
}

#[test]
fn a_window_dragged_to_an_edge_shows_the_tile_and_takes_it_once_released() {
    let mut h = Harness::new();
    bars(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    let before = h.rect(window);
    grab(&mut h, window);
    // Moved, but not to an edge: no tile.
    h.scene.pointer_motion(900, 400);
    assert_eq!(preview(&h), None);
    // At the left edge: the left half, previewed just beneath the window, which stays as it is
    // and is told nothing.
    let configures = h.scene.configures_sent(window);
    h.scene.pointer_motion(0, 500);
    let (at, rect) = preview(&h).expect("a preview");
    assert_eq!(rect, Tile::Left.rect(AREA));
    assert_eq!(rect, Rect::new(8, 40, 948, 984));
    assert_eq!(at + 1, index_of(&h, window), "just beneath the window");
    assert_eq!(h.rect(window).width, before.width);
    // Away from the edge again, gone.
    h.scene.pointer_motion(5, 500);
    assert_eq!(preview(&h), None);
    h.scene.pointer_motion(3, 500);
    assert!(preview(&h).is_some());
    h.clear_log();
    release(&mut h);
    assert_eq!(preview(&h), None);
    assert_eq!(h.scene.configures_sent(window), configures + 1, "told once");
    h.present(window);
    assert_eq!(h.rect(window), Rect::new(8, 40, 948, 984));
    assert_eq!(h.scene.tile_of(window), Some(Some(Tile::Left)));
    assert_eq!(h.scene.active_window(), Some(window));
    assert_eq!(h.scene.focus, Some(window));
    // Its client heard nothing of the drag: no press, motion or release after the grab.
    assert_eq!(pointer_presses(&h.events_for(APP)), 0);
}

#[test]
fn every_edge_and_corner_tiles_and_the_docks_edge_does_not() {
    let mut h = Harness::new();
    bars(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    for ((x, y), tile) in [
        ((0, 600), Some(Tile::Left)),
        ((1919, 600), Some(Tile::Right)),
        ((0, 40), Some(Tile::TopLeft)),
        ((1919, 40), Some(Tile::TopRight)),
        ((0, 1079), Some(Tile::BottomLeft)),
        ((1919, 1060), Some(Tile::BottomRight)),
        // The menu bar, or the very top, fills.
        ((960, 10), Some(Tile::Fill)),
        ((960, 0), Some(Tile::Fill)),
        ((100, 0), Some(Tile::TopLeft)),
        ((1900, 0), Some(Tile::TopRight)),
        ((100, 1079), Some(Tile::BottomLeft)),
        // The bottom edge's middle is the dock's.
        ((960, 1079), None),
    ] {
        let at = h.rect(window);
        grab(&mut h, window);
        h.scene.pointer_motion(at.x + 50, at.y + 50);
        h.scene.pointer_motion(x, y);
        assert_eq!(
            preview(&h).map(|(_, rect)| rect),
            tile.map(|tile| tile.rect(AREA)),
            "at ({x}, {y})"
        );
        release(&mut h);
        h.present(window);
        if let Some(tile) = tile {
            assert_eq!(h.rect(window), tile.rect(AREA), "at ({x}, {y})");
        }
        // Back to the middle for the next.
        h.scene.lay_out_window(window, Layout::ReturnToPreviousSize);
        h.present(window);
        assert_eq!(h.rect(window).width, 400);
    }
}

#[test]
fn with_alt_held_a_drag_previews_a_tile_anywhere() {
    let mut h = Harness::new();
    bars(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    grab(&mut h, window);
    h.scene.pointer_motion(900, 500);
    assert_eq!(preview(&h), None);
    // Alt down while dragging: the middle fills, a side's quarter is its half, a corner its
    // quarter; up again, none.
    h.scene.keyboard(ALT, true, None);
    assert_eq!(preview(&h).map(|(_, r)| r), Some(Tile::Fill.rect(AREA)));
    h.scene.pointer_motion(200, 500);
    assert_eq!(preview(&h).map(|(_, r)| r), Some(Tile::Left.rect(AREA)));
    h.scene.pointer_motion(1800, 100);
    assert_eq!(preview(&h).map(|(_, r)| r), Some(Tile::TopRight.rect(AREA)));
    h.scene.keyboard(ALT, false, None);
    assert_eq!(preview(&h), None);
    h.scene.keyboard(ALT, true, None);
    release(&mut h);
    h.scene.keyboard(ALT, false, None);
    h.present(window);
    assert_eq!(h.rect(window), Tile::TopRight.rect(AREA));
}

#[test]
fn a_tiled_window_dragged_away_takes_back_its_size_under_the_pointer() {
    let mut h = Harness::new();
    bars(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    let before = h.rect(window);
    h.scene.lay_out_window(window, Layout::Tile(Tile::Left));
    h.present(window);
    let tiled = h.rect(window);
    assert_eq!(tiled, Rect::new(8, 40, 948, 984));
    // Grabbed at 237 px across (a quarter of its width) and 20 down.
    let serial = h.press(tiled.x + 237, tiled.y + 20);
    h.request(APP, Request::BeginMove { surface: 1, serial })
        .unwrap();
    let configures = h.scene.configures_sent(window);
    // A press alone changes nothing.
    assert_eq!(h.scene.configures_sent(window), configures);
    h.scene
        .pointer_motion(tiled.x + 237 + 10, tiled.y + 20 + 10);
    assert_eq!(h.scene.configures_sent(window), configures + 1, "told once");
    assert_eq!(h.scene.tile_of(window), None);
    h.scene
        .pointer_motion(tiled.x + 237 + 30, tiled.y + 20 + 40);
    h.present(window);
    release(&mut h);
    // Its own size again, the pointer a quarter across it and 20 down, as when it was grabbed.
    let at = h.rect(window);
    assert_eq!((at.width, at.height), (before.width, before.height));
    assert_eq!(at.x, tiled.x + 237 + 30 - 100);
    assert_eq!(at.y, tiled.y + 20 + 40 - 20);
    assert_eq!(h.scene.configures_sent(window), configures + 1);
    // An ordinary window dragged is told nothing.
    grab(&mut h, window);
    h.scene.pointer_motion(800, 600);
    release(&mut h);
    assert_eq!(h.scene.configures_sent(window), configures + 1);
}

#[test]
fn super_alt_tiles_fills_centres_and_returns_the_active_window() {
    let mut h = Harness::new();
    bars(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.click(h.rect(window).x + 10, h.rect(window).y + 10);
    h.scene.move_window_to(window, (100, 100));
    let before = h.rect(window);
    h.clear_log();
    let area = AREA;
    for (codes, rect) in [
        (vec![ALT, LEFT], Tile::Left.rect(area)),
        (vec![ALT, RIGHT], Tile::Right.rect(area)),
        (vec![ALT, UP], Tile::Top.rect(area)),
        (vec![ALT, DOWN], Tile::Bottom.rect(area)),
        (vec![ALT, F], Tile::Fill.rect(area)),
    ] {
        let configures = h.scene.configures_sent(window);
        chord(&mut h, &codes);
        assert_eq!(h.scene.configures_sent(window), configures + 1, "{codes:?}");
        h.present(window);
        assert_eq!(h.rect(window), rect, "{codes:?}");
    }
    // Again: already there, so its client is told nothing.
    let configures = h.scene.configures_sent(window);
    chord(&mut h, &[ALT, F]);
    assert_eq!(h.scene.configures_sent(window), configures);
    // Return to Previous Size: the frame before the first tile, not the one before the last.
    chord(&mut h, &[ALT, R]);
    h.present(window);
    assert_eq!(h.rect(window), before);
    assert_eq!(h.scene.tile_of(window), None);
    // Nothing more to return to.
    let configures = h.scene.configures_sent(window);
    chord(&mut h, &[ALT, R]);
    assert_eq!(h.scene.configures_sent(window), configures);
    // Center: in the middle of the work area, at its size.
    chord(&mut h, &[ALT, C]);
    h.present(window);
    assert_eq!(h.rect(window), Rect::new(760, 382, 400, 300));
    chord(&mut h, &[ALT, R]);
    h.present(window);
    assert_eq!(h.rect(window), before);
    // The keys are the system's: nothing of them reached the application, which kept the
    // keyboard throughout.
    assert_eq!(keys_to(&h, APP), 0);
    assert_eq!(h.scene.focus, Some(window));
    assert_eq!(h.scene.active_window(), Some(window));
}

#[test]
fn super_alt_shift_arranges_the_active_window_and_the_one_behind() {
    let mut h = Harness::new();
    bars(&mut h);
    let back = h.toplevel(OTHER, 1, 500, 400);
    let front = h.toplevel(APP, 1, 400, 300);
    h.click(h.rect(front).x + 10, h.rect(front).y + 10);
    assert_eq!(h.scene.active_window(), Some(front));
    h.clear_log();
    for (codes, first, second) in [
        (vec![ALT, SHIFT, LEFT], Tile::Left, Tile::Right),
        (vec![ALT, SHIFT, RIGHT], Tile::Right, Tile::Left),
        (vec![ALT, SHIFT, UP], Tile::Top, Tile::Bottom),
        (vec![ALT, SHIFT, DOWN], Tile::Bottom, Tile::Top),
    ] {
        chord(&mut h, &codes);
        h.present(front);
        h.present(back);
        assert_eq!(h.rect(front), first.rect(AREA), "{codes:?}");
        assert_eq!(h.rect(back), second.rect(AREA), "{codes:?}");
        // The active window stays the active one, in front, with the keyboard.
        assert_eq!(h.scene.active_window(), Some(front));
        assert_eq!(h.scene.focus, Some(front));
        assert!(index_of(&h, front) > index_of(&h, back));
    }
    // The halves keep the margin between them.
    assert_eq!(
        i64::from(h.rect(front).y) - h.rect(back).bottom(),
        i64::from(TILE_MARGIN)
    );
    assert_eq!(keys_to(&h, APP) + keys_to(&h, OTHER), 0);
    // Each window returns to its own frame.
    h.scene.lay_out_window(back, Layout::ReturnToPreviousSize);
    h.present(back);
    assert_eq!((h.rect(back).width, h.rect(back).height), (500, 400));
}

#[test]
fn a_zoomed_window_tiled_returns_to_its_frame_before_the_zoom_and_a_resize_ends_the_tile() {
    let mut h = Harness::new();
    bars(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    let before = h.rect(window);
    h.request(
        APP,
        Request::SetMaximized {
            surface: 1,
            enabled: true,
        },
    )
    .unwrap();
    h.present(window);
    assert!(h.scene.is_maximized(window));
    h.scene.lay_out_window(window, Layout::Tile(Tile::Right));
    h.present(window);
    assert!(!h.scene.is_maximized(window));
    assert_eq!(last_configure_maximized(&h, window), Some(false));
    assert_eq!(h.rect(window), Tile::Right.rect(AREA));
    h.scene.lay_out_window(window, Layout::ReturnToPreviousSize);
    h.present(window);
    assert_eq!(h.rect(window), before);

    // Tiled, then resized by hand from its left edge: no longer tiled, nothing to return to.
    h.scene.lay_out_window(window, Layout::Tile(Tile::Right));
    h.present(window);
    let tiled = h.rect(window);
    h.scene.pointer_motion(tiled.x - 3, tiled.y + 300);
    h.scene.pointer_button(BUTTON, true);
    h.scene.pointer_motion(tiled.x + 97, tiled.y + 300);
    h.scene.pointer_button(BUTTON, false);
    h.present(window);
    assert_eq!(h.scene.tile_of(window), None);
    let configures = h.scene.configures_sent(window);
    h.scene.lay_out_window(window, Layout::ReturnToPreviousSize);
    assert_eq!(h.scene.configures_sent(window), configures);

    // A window's limits hold: one that cannot be narrower than 1000 is centred on its tile,
    // within the work area.
    h.request(
        APP,
        Request::SetSizeLimits {
            surface: 1,
            min_width: 1000,
            min_height: 0,
            max_width: 0,
            max_height: 0,
        },
    )
    .unwrap();
    h.present(window);
    h.scene.lay_out_window(window, Layout::Tile(Tile::Left));
    h.present(window);
    assert_eq!(h.rect(window), Rect::new(0, 40, 1000, 984));
}

#[test]
fn nothing_tiles_while_locked_or_full_screen() {
    let mut h = Harness::new();
    bars(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.click(h.rect(window).x + 10, h.rect(window).y + 10);
    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: true,
        },
    )
    .unwrap();
    h.present(window);
    let configures = h.scene.configures_sent(window);
    chord(&mut h, &[ALT, LEFT]);
    assert_eq!(h.scene.configures_sent(window), configures);
    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: false,
        },
    )
    .unwrap();
    h.present(window);
    h.scene.lock();
    let configures = h.scene.configures_sent(window);
    chord(&mut h, &[ALT, LEFT]);
    h.scene.lay_out_window(window, Layout::Tile(Tile::Fill));
    assert_eq!(h.scene.configures_sent(window), configures);
}

#[test]
fn a_shared_edge_is_crossed_and_each_display_tiles_on_its_own_at_its_scale() {
    let mut h = Harness::new();
    // A second display right of the first, at 1.25: 1600 × 1000 pixels, 1280 × 800 logical.
    h.scene
        .add_output(Output {
            id: 2,
            rect: Rect::new(1920, 0, 1280, 800),
            scale_120: 150,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let window = h.toplevel(APP, 1, 400, 300);
    grab(&mut h, window);
    let at = h.rect(window);
    h.scene.pointer_motion(at.x + 60, at.y + 30);
    // The first display's right edge goes on into the second: no tile there.
    h.scene.pointer_motion(1919, 500);
    assert_eq!(preview(&h), None);
    // The second's left edge is the first's: none either; its right edge is its own.
    h.scene.pointer_motion(1920, 500);
    assert_eq!(preview(&h), None);
    h.scene.pointer_motion(3199, 400);
    let second = Rect::new(1920, 0, 1280, 800);
    assert_eq!(preview(&h).map(|(_, r)| r), Some(Tile::Right.rect(second)));
    release(&mut h);
    h.present(window);
    // On the second display, at its scale: 628 × 784 logical, 785 × 980 pixels.
    assert_eq!(h.rect(window), Rect::new(2564, 8, 628, 784));
    let (_, rect, scale) = h.configures[&window];
    assert_eq!((rect.width, rect.height, scale), (628, 784, 150));
    assert_eq!(physical(Size::new(628, 784), 150), Size::new(785, 980));

    // The display at another mode: the window takes its tile there.
    h.scene
        .set_output_geometry(2, Rect::new(1920, 0, 1024, 640), 150)
        .unwrap();
    h.present(window);
    assert_eq!(
        h.rect(window),
        Tile::Right.rect(Rect::new(1920, 0, 1024, 640))
    );
    // The display gone: the window takes its tile on the one left.
    h.scene.remove_output(2);
    h.present(window);
    assert_eq!(h.rect(window), Tile::Right.rect(SCREEN));
    assert_eq!(h.configures[&window].2, 120);
}

#[test]
fn the_shells_window_menu_lays_out_and_zooms_a_window_by_its_number() {
    use oxyos_protocol::WindowLayout;
    let mut h = Harness::new();
    bars(&mut h);
    let back = h.toplevel(OTHER, 1, 500, 400);
    let window = h.toplevel(APP, 1, 400, 300);
    h.click(h.rect(window).x + 10, h.rect(window).y + 10);
    h.scene.move_window_to(window, (100, 100));
    let before = h.rect(window);
    let number = h.scene.window_number(window).unwrap();
    let ask = |h: &mut Harness, layout| {
        h.request(
            SHELL,
            Request::LayOutWindow {
                window: number,
                layout,
            },
        )
    };
    h.clear_log();
    // Each tile as Super+Alt's keys give it, and the quarters, which no key gives.
    for (layout, tile) in [
        (WindowLayout::Left, Tile::Left),
        (WindowLayout::Right, Tile::Right),
        (WindowLayout::Top, Tile::Top),
        (WindowLayout::Bottom, Tile::Bottom),
        (WindowLayout::TopLeft, Tile::TopLeft),
        (WindowLayout::TopRight, Tile::TopRight),
        (WindowLayout::BottomLeft, Tile::BottomLeft),
        (WindowLayout::BottomRight, Tile::BottomRight),
        (WindowLayout::Fill, Tile::Fill),
    ] {
        ask(&mut h, layout).unwrap();
        h.present(window);
        assert_eq!(h.rect(window), tile.rect(AREA), "{layout:?}");
        assert_eq!(h.scene.tile_of(window), Some(Some(tile)), "{layout:?}");
    }
    // Return to Previous Size: the frame before the first tile; Center: the middle at its size.
    ask(&mut h, WindowLayout::ReturnToPreviousSize).unwrap();
    h.present(window);
    assert_eq!(h.rect(window), before);
    ask(&mut h, WindowLayout::Center).unwrap();
    h.present(window);
    assert_eq!(h.rect(window), Rect::new(760, 382, 400, 300));
    ask(&mut h, WindowLayout::ReturnToPreviousSize).unwrap();
    h.present(window);
    assert_eq!(h.rect(window), before);
    // An arrangement: the window and the one behind it, the window kept in front with the
    // keyboard.
    ask(&mut h, WindowLayout::LeftAndRight).unwrap();
    h.present(window);
    h.present(back);
    assert_eq!(h.rect(window), Tile::Left.rect(AREA));
    assert_eq!(h.rect(back), Tile::Right.rect(AREA));
    assert_eq!(h.scene.focus, Some(window));
    ask(&mut h, WindowLayout::ReturnToPreviousSize).unwrap();
    h.present(window);
    // Zoom, as the green bubble: zoomed, and back to where it was.
    ask(&mut h, WindowLayout::Zoom).unwrap();
    h.present(window);
    assert!(h.scene.is_maximized(window));
    ask(&mut h, WindowLayout::Zoom).unwrap();
    h.present(window);
    assert!(!h.scene.is_maximized(window));
    assert_eq!(h.rect(window), before);
    // Nothing of it reached the application as a key.
    assert_eq!(keys_to(&h, APP), 0);
    // Only the shell; a window gone is ignored; nothing while locked.
    assert_eq!(
        h.request(
            APP,
            Request::LayOutWindow {
                window: number,
                layout: WindowLayout::Fill,
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
    assert_eq!(
        h.request(
            SHELL,
            Request::LayOutWindow {
                window: 999,
                layout: WindowLayout::Fill,
            }
        ),
        Ok(())
    );
    h.scene.lock();
    ask(&mut h, WindowLayout::Fill).unwrap();
    h.scene.unlock();
    h.present(window);
    assert_eq!(h.rect(window), before);
    assert_eq!(h.scene.tile_of(window), None);
}

#[test]
fn a_window_lays_itself_out_from_its_zoom_control_as_the_shell_does() {
    use oxyos_protocol::WindowLayout;
    let mut h = Harness::new();
    bars(&mut h);
    let back = h.toplevel(OTHER, 1, 500, 400);
    let window = h.toplevel(APP, 1, 400, 300);
    h.click(h.rect(window).x + 10, h.rect(window).y + 10);
    h.scene.move_window_to(window, (100, 100));
    let before = h.rect(window);
    // Its own toplevel, by its own surface id: no grant.
    let ask = |h: &mut Harness, layout| {
        h.request(
            APP,
            Request::LayOutSurface {
                surface: window.id,
                layout,
            },
        )
    };
    for (layout, tile) in [
        (WindowLayout::Left, Tile::Left),
        (WindowLayout::Right, Tile::Right),
        (WindowLayout::Top, Tile::Top),
        (WindowLayout::Bottom, Tile::Bottom),
        (WindowLayout::TopLeft, Tile::TopLeft),
        (WindowLayout::TopRight, Tile::TopRight),
        (WindowLayout::BottomLeft, Tile::BottomLeft),
        (WindowLayout::BottomRight, Tile::BottomRight),
        (WindowLayout::Fill, Tile::Fill),
    ] {
        ask(&mut h, layout).unwrap();
        h.present(window);
        assert_eq!(h.rect(window), tile.rect(AREA), "{layout:?}");
        assert_eq!(h.scene.tile_of(window), Some(Some(tile)), "{layout:?}");
    }
    ask(&mut h, WindowLayout::ReturnToPreviousSize).unwrap();
    h.present(window);
    assert_eq!(h.rect(window), before);
    ask(&mut h, WindowLayout::Center).unwrap();
    h.present(window);
    assert_eq!(h.rect(window), Rect::new(760, 382, 400, 300));
    ask(&mut h, WindowLayout::ReturnToPreviousSize).unwrap();
    h.present(window);
    // An arrangement takes the window behind it too, as the shell's does.
    ask(&mut h, WindowLayout::RightAndLeft).unwrap();
    h.present(window);
    h.present(back);
    assert_eq!(h.rect(window), Tile::Right.rect(AREA));
    assert_eq!(h.rect(back), Tile::Left.rect(AREA));
    assert_eq!(h.scene.focus, Some(window));
    ask(&mut h, WindowLayout::ReturnToPreviousSize).unwrap();
    h.present(window);
    // Zoom, as its zoom control: zoomed, and back.
    ask(&mut h, WindowLayout::Zoom).unwrap();
    h.present(window);
    assert!(h.scene.is_maximized(window));
    ask(&mut h, WindowLayout::Zoom).unwrap();
    h.present(window);
    assert!(!h.scene.is_maximized(window));
    assert_eq!(h.rect(window), before);
    // Nothing while locked.
    h.scene.lock();
    ask(&mut h, WindowLayout::Fill).unwrap();
    h.scene.unlock();
    h.present(window);
    assert_eq!(h.rect(window), before);
}

#[test]
fn a_window_lays_out_only_its_own_toplevels() {
    use oxyos_protocol::WindowLayout;
    let mut h = Harness::new();
    bars(&mut h);
    let theirs = h.toplevel(OTHER, 1, 500, 400);
    let mine = h.toplevel(APP, 2, 400, 300);
    let before = h.rect(theirs);
    // The other application's window by its surface id is no surface of this client's.
    assert_eq!(
        h.request(
            APP,
            Request::LayOutSurface {
                surface: theirs.id,
                layout: WindowLayout::Fill,
            }
        ),
        Err(ErrorCode::UnknownSurface)
    );
    assert_eq!(
        h.request(
            APP,
            Request::MoveSurfaceToWorkspace {
                surface: theirs.id,
                workspace: 0,
            }
        ),
        Err(ErrorCode::UnknownSurface)
    );
    h.present(theirs);
    assert_eq!(h.rect(theirs), before);
    // A popup of its own is no window.
    let popup = h.popup(mine, 9, menu(Rect::new(10, 10, 20, 20), 120, 80));
    assert_eq!(
        h.request(
            APP,
            Request::LayOutSurface {
                surface: popup.id,
                layout: WindowLayout::Fill,
            }
        ),
        Err(ErrorCode::InvalidRole)
    );
    assert_eq!(
        h.request(
            APP,
            Request::MoveSurfaceToWorkspace {
                surface: popup.id,
                workspace: 0,
            }
        ),
        Err(ErrorCode::InvalidRole)
    );
}

#[test]
fn a_window_moves_itself_to_another_workspace_and_is_told_the_workspaces() {
    let mut h = Harness::new();
    bars(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    let told = |h: &Harness| -> Vec<(u32, u32, u32)> {
        h.events_for(APP)
            .into_iter()
            .filter_map(|event| match event {
                Event::SurfaceWorkspaces {
                    surface,
                    count,
                    workspace,
                } => Some((surface, count, workspace)),
                _ => None,
            })
            .collect()
    };
    // Told as it gets its role: one workspace, on it.
    h.scene.drain_events();
    assert_eq!(told(&h), vec![(window.id, 1, 0)]);
    h.clear_log();
    // Told again only when something changed: a workspace added.
    h.scene.drain_events();
    assert_eq!(told(&h), vec![]);
    h.request(SHELL, Request::AddWorkspace {}).unwrap();
    h.scene.drain_events();
    assert_eq!(told(&h), vec![(window.id, 2, 0)]);
    h.clear_log();
    // Moved by its own request: gone from the workspace shown, and told where it is.
    h.request(
        APP,
        Request::MoveSurfaceToWorkspace {
            surface: window.id,
            workspace: 1,
        },
    )
    .unwrap();
    assert_eq!(h.scene.window_workspace(window), Some(1));
    assert!(!h.rendered(window));
    h.scene.drain_events();
    assert_eq!(told(&h), vec![(window.id, 2, 1)]);
    // A workspace there is not: nothing.
    h.request(
        APP,
        Request::MoveSurfaceToWorkspace {
            surface: window.id,
            workspace: 7,
        },
    )
    .unwrap();
    assert_eq!(h.scene.window_workspace(window), Some(1));
    // Nothing while locked.
    h.scene.lock();
    h.request(
        APP,
        Request::MoveSurfaceToWorkspace {
            surface: window.id,
            workspace: 0,
        },
    )
    .unwrap();
    h.scene.unlock();
    assert_eq!(h.scene.window_workspace(window), Some(1));
}
