//! Several outputs (docs/adr/0018-several-displays.mdx): each output's frame presents and
//! completes on its own, releases wait only for the frames that may read the buffer, an output
//! moves with what is on it, the shell is told the outputs and which is the primary, and every
//! surface's client the refresh of its output.

use super::*;

/// The second output, right of the first, top-aligned.
const RIGHT: Rect = Rect::new(1920, 0, 1280, 720);

fn two_outputs() -> Harness {
    let mut h = Harness::new();
    h.scene
        .add_output(Output {
            id: 2,
            rect: RIGHT,
            scale_120: 180,
            refresh_mhz: 144_000,
        })
        .unwrap();
    h
}

/// A toplevel opened with the pointer at (x, y), so on the output there.
fn window_at(h: &mut Harness, client: ClientId, id: u32, (x, y): (i32, i32)) -> SurfaceKey {
    h.scene.pointer_motion(x, y);
    h.toplevel(client, id, 400, 300)
}

fn presented(h: &Harness, surface: SurfaceKey) -> Vec<u64> {
    h.events_for(surface.client)
        .into_iter()
        .filter_map(|event| match event {
            Event::Feedback {
                surface: id,
                outcome: Outcome::Presented,
                timestamp_ns,
                ..
            } if id == surface.id => Some(timestamp_ns),
            _ => None,
        })
        .collect()
}

#[test]
fn each_output_presents_and_completes_its_own_frame() {
    let mut h = two_outputs();
    let left = window_at(&mut h, APP, 1, (100, 100));
    let right = window_at(&mut h, OTHER, 1, (2000, 100));
    assert!(RIGHT.contains(h.rect(right)), "{:?}", h.rect(right));
    h.clear_log();
    h.submit(left);
    h.submit(right);
    // The right output's frame takes its window's frame alone, and completes at its own flip.
    assert_eq!(h.scene.begin_frame_on(2), 1);
    assert_eq!(h.scene.begin_frame_on(1), 1);
    h.scene.end_frame_on(2, 222, Clock::Monotonic);
    h.pump();
    assert_eq!(presented(&h, right), [222]);
    assert!(
        presented(&h, left).is_empty(),
        "the left output has not flipped"
    );
    // Nothing more begins on the right while its window has nothing new.
    assert_eq!(h.scene.begin_frame_on(2), 0);
    h.scene.end_frame_on(2, 0, Clock::Synthetic);
    h.scene.end_frame_on(1, 111, Clock::Monotonic);
    h.pump();
    assert_eq!(presented(&h, left), [111]);
    assert_eq!(presented(&h, right), [222]);
}

#[test]
fn a_release_waits_only_for_the_frames_that_may_read_the_buffer() {
    let mut h = two_outputs();
    let left = window_at(&mut h, APP, 1, (100, 100));
    let right = window_at(&mut h, OTHER, 1, (2000, 100));
    h.submit(left);
    h.submit(right);
    // The right output's frame is in flight, a long refresh; the left window's old buffer is
    // released at the left output's flip, as nothing on the right shows it.
    h.scene.begin_frame_on(2);
    h.scene.begin_frame_on(1);
    h.clear_log();
    h.scene.end_frame_on(1, 1, Clock::Monotonic);
    h.pump();
    let released = |h: &Harness, client| {
        h.events_for(client)
            .iter()
            .any(|event| matches!(event, Event::Released { .. }))
    };
    assert!(released(&h, APP));
    assert_eq!(h.scene.stats().deferred_releases, 0);
    h.scene.end_frame_on(2, 2, Clock::Monotonic);
    h.pump();
    assert!(released(&h, OTHER));

    // A window across both outputs is read by both frames: its old buffer waits for the other.
    h.scene.pointer_motion(100, 100);
    let across = h.toplevel(APP, 2, 400, 300);
    let at = h.rect(across);
    let serial = h.press(at.x + 5, at.y + 5);
    h.request(APP, Request::BeginMove { surface: 2, serial })
        .unwrap();
    // Dragged until a quarter of it is on the right output.
    h.scene.pointer_motion(1920 - 300 + 5, at.y + 5);
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    let at = h.rect(across);
    assert!(at.x < 1920 && at.right() > 1920, "{at:?}");
    let output = h.scene.nodes[&across].output;
    let other = if output == 1 { 2 } else { 1 };
    h.submit(across);
    h.scene.begin_frame_on(other);
    h.scene.begin_frame_on(output);
    h.clear_log();
    h.scene.end_frame_on(output, 3, Clock::Monotonic);
    h.pump();
    assert!(
        !released(&h, APP),
        "the other output's frame may still read it"
    );
    assert_eq!(h.scene.stats().deferred_releases, 1);
    h.scene.end_frame_on(other, 4, Clock::Monotonic);
    h.pump();
    assert!(released(&h, APP));
    assert_eq!(h.scene.stats().deferred_releases, 0);
}

#[test]
fn an_output_moved_takes_what_is_on_it() {
    let mut h = two_outputs();
    let left = window_at(&mut h, APP, 1, (100, 100));
    let right = window_at(&mut h, OTHER, 1, (2000, 100));
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Top,
            output: 2,
            size: Size::new(1, 28),
            exclusive: 28,
        },
    )
    .unwrap();
    let panel = key(SHELL, 1);
    h.present(panel);
    let (before_left, before_right) = (h.rect(left), h.rect(right));
    // The output on the left went: the right one takes its place at the origin.
    h.scene.set_output_origin(2, (0, 0)).unwrap();
    h.pump();
    assert_eq!(h.scene.outputs()[1].rect, Rect::new(0, 0, 1280, 720));
    assert_eq!(h.rect(left), before_left, "another output's window stays");
    let moved = h.rect(right);
    assert_eq!(
        (moved.x, moved.y),
        (before_right.x - 1920, before_right.y),
        "its window keeps its place on it"
    );
    assert_eq!(h.rect(panel), Rect::new(0, 0, 1280, 28));
    assert_eq!(h.scene.work_area(2), Some(Rect::new(0, 28, 1280, 692)));
    assert_eq!(
        h.scene.set_output_origin(9, (0, 0)),
        Err(ErrorCode::InvalidGeometry)
    );
}

#[test]
fn the_shell_is_told_the_outputs_and_the_primary() {
    let mut h = two_outputs();
    let outputs = |h: &Harness| -> Vec<Event> {
        h.events_for(SHELL)
            .into_iter()
            .filter(|event| matches!(event, Event::Output { .. } | Event::OutputRemoved { .. }))
            .collect()
    };
    h.pump();
    assert_eq!(
        outputs(&h),
        [
            Event::Output {
                output: 1,
                rect: SCREEN,
                scale_120: 120,
                refresh_mhz: 60_000,
                primary: true,
            },
            Event::Output {
                output: 2,
                rect: RIGHT,
                scale_120: 180,
                refresh_mhz: 144_000,
                primary: false,
            },
        ]
    );
    // Told once: nothing changed.
    h.clear_log();
    h.pump();
    assert!(outputs(&h).is_empty());
    // The primary goes: the other is told it is the primary now, at its new place.
    h.scene.remove_output(1);
    h.scene.set_output_origin(2, (0, 0)).unwrap();
    h.pump();
    assert_eq!(
        outputs(&h),
        [
            Event::OutputRemoved { output: 1 },
            Event::Output {
                output: 2,
                rect: Rect::new(0, 0, 1280, 720),
                scale_120: 180,
                refresh_mhz: 144_000,
                primary: true,
            },
        ]
    );
    // Applications are never told.
    assert!(h
        .log
        .iter()
        .all(|(client, event)| *client == SHELL || !matches!(event, Event::Output { .. })));
}

#[test]
fn windows_left_on_no_output_go_to_the_next_one_added() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    // The last output goes (its monitor unplugged, nothing else connected).
    h.scene.remove_output(1);
    h.pump();
    assert!(h.scene.outputs().is_empty());
    let monitor = Rect::new(0, 0, 2560, 1440);
    h.scene
        .add_output(Output {
            id: 7,
            rect: monitor,
            scale_120: 180,
            refresh_mhz: 60_000,
        })
        .unwrap();
    h.pump();
    // Asked for a frame at the new output's scale, centred on it.
    let (_, rect, scale) = h.configures[&window];
    assert_eq!((rect.width, rect.height, scale), (400, 300, 180));
    h.present(window);
    let placed = h.rect(window);
    assert!(monitor.contains(placed), "{placed:?}");
    assert_eq!(h.scene.nodes[&window].output, 7);
}

/// What a client was told of its surfaces' refresh: (surface, mHz), in order.
fn refreshes(h: &Harness, client: ClientId) -> Vec<(u32, u32)> {
    h.events_for(client)
        .into_iter()
        .filter_map(|event| match event {
            Event::Refresh {
                surface,
                refresh_mhz,
            } => Some((surface, refresh_mhz)),
            _ => None,
        })
        .collect()
}

#[test]
fn every_surface_is_told_its_outputs_refresh_and_paces_by_it() {
    let mut h = two_outputs();
    h.clear_log();
    // A window opened on the 60 Hz output is told its refresh before its first configure.
    let window = window_at(&mut h, APP, 1, (100, 100));
    let told = h.events_for(APP);
    let refresh = told
        .iter()
        .position(|event| matches!(event, Event::Refresh { surface: 1, .. }))
        .expect("told its refresh");
    let configure = told
        .iter()
        .position(|event| matches!(event, Event::Configure { surface: 1, .. }))
        .expect("configured");
    assert!(refresh < configure, "{told:?}");
    assert_eq!(refreshes(&h, APP), [(1, 60_000)]);
    // One on the 144 Hz output, its own.
    window_at(&mut h, OTHER, 1, (2000, 100));
    assert_eq!(refreshes(&h, OTHER), [(1, 144_000)]);
    // Told once: nothing changed.
    h.clear_log();
    h.pump();
    assert!(refreshes(&h, APP).is_empty());
    // Dragged onto the 144 Hz output, its client is told the refresh there.
    let at = h.rect(window);
    let serial = h.press(at.x + 5, at.y + 5);
    h.request(APP, Request::BeginMove { surface: 1, serial })
        .unwrap();
    h.scene.pointer_motion(2100, at.y + 5);
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    assert_eq!(h.scene.nodes[&window].output, 2);
    assert_eq!(refreshes(&h, APP), [(1, 144_000)]);
    // The display driven at another rate (Settings' refresh, or its fastest at the start): every
    // surface on it is told, and the shell the output again; nothing else changes.
    h.clear_log();
    h.scene.set_output_refresh(2, 119_998).unwrap();
    h.pump();
    assert_eq!(refreshes(&h, APP), [(1, 119_998)]);
    assert_eq!(refreshes(&h, OTHER), [(1, 119_998)]);
    assert!(h.events_for(SHELL).iter().any(|event| matches!(
        event,
        Event::Output {
            output: 2,
            refresh_mhz: 119_998,
            ..
        }
    )));
    assert!(h
        .events_for(APP)
        .iter()
        .all(|event| !matches!(event, Event::Configure { .. })));
    // A refresh no display has is refused, as an output with none.
    assert_eq!(
        h.scene.set_output_refresh(2, 0),
        Err(ErrorCode::InvalidGeometry)
    );
    assert_eq!(
        h.scene.add_output(Output {
            id: 3,
            rect: Rect::new(3200, 0, 800, 600),
            scale_120: 120,
            refresh_mhz: 0,
        }),
        Err(ErrorCode::InvalidGeometry)
    );
}
