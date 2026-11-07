//! A drag that vanishes (docs/adr/0035-drag-and-drop.mdx, 6 November 2026), through real sockets on
//! the compositor's clock set by the test: dropped on nothing, its image never slides back; a
//! cloud puffs out where it was, at the output's scale, and is gone in three tenths of a second,
//! the poof's sound played once.

use oxyos_client::{Client, DragOptions};
use oxyos_compositor::{Compositor, DESKTOP};
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{
    DragActions, DragCancelReason, Event, PointerKind, Rect, Request, Size, TextChunk,
};
use oxyos_sound::{Recorder, Sound};
use std::time::{Duration, Instant};

/// The drag's image: a 40 × 40 square of this colour, gripped at its middle.
const ICON: u32 = 0x00d0_2040;
const ICON_SIDE: u32 = 40;
const START: Duration = Duration::from_secs(100);
/// When the drag is dropped, on the test's clock: the window long arrived.
const DROPPED: Duration = Duration::from_secs(101);

/// Whether `pixel` is the cloud's: light and nearly grey (its white, or its shading over the
/// desktop), never the image's red nor the desktop.
fn cloudy(pixel: u32) -> bool {
    let [b, g, r, _] = pixel.to_le_bytes();
    let (lo, hi) = (r.min(g).min(b), r.max(g).max(b));
    pixel != DESKTOP && lo > 0x60 && hi - lo < 0x30
}

/// A surface's pixels at its configured scale, as the compositor rounds them.
fn pixels_of(rect: Rect, scale_120: u32) -> Size {
    Size::new(
        oxyos_compositor::scaled(rect.width, scale_120),
        oxyos_compositor::scaled(rect.height, scale_120),
    )
}

/// The output's points around `drop` that are cloud.
fn clouded(compositor: &Compositor, drop: (i32, i32)) -> usize {
    (-40..40)
        .flat_map(|dy| (-40..40).map(move |dx| (dx, dy)))
        .filter(|(dx, dy)| cloudy(compositor.pixel(drop.0 + dx, drop.1 + dy)))
        .count()
}

fn run(scale_120: u32) {
    let name = format!("oxyos-poof-{scale_120}-{}", std::process::id());
    let directory = std::env::temp_dir().join(name);
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: Rect::new(0, 0, 800, 600),
            scale_120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    compositor.clock = Some(START);
    let recorder = Recorder::default();
    compositor.sounds = Some(oxyos_compositor::sounds::Sounds::new(Box::new(
        recorder.clone(),
    )));
    let path = socket.clone();
    let source = std::thread::spawn(move || {
        let mut client = Client::connect(&path).unwrap();
        client
            .send(&Request::CreateToplevel {
                surface: 1,
                size: Size::new(200, 100),
            })
            .unwrap();
        let mut press = None;
        loop {
            match client
                .next_event()
                .unwrap()
                .expect("the compositor is there")
            {
                Event::Configure {
                    surface: 1,
                    serial,
                    rect,
                    scale_120,
                    ..
                } => {
                    let extent = pixels_of(rect, scale_120);
                    let pixels = vec![0xff; (extent.width * extent.height * 4) as usize];
                    client.present(1, serial, extent, &pixels).unwrap();
                }
                // A press: the image is made, then the drag starts from it once it is drawn.
                Event::Pointer {
                    kind: PointerKind::Press,
                    serial,
                    ..
                } => {
                    press = Some(serial);
                    client
                        .create_drag_icon(9, Size::new(ICON_SIDE, ICON_SIDE), (20, 20))
                        .unwrap();
                }
                Event::Configure {
                    surface: 9,
                    serial,
                    rect,
                    scale_120,
                    ..
                } => {
                    let extent = pixels_of(rect, scale_120);
                    let pixels: Vec<u8> = (0..extent.width * extent.height)
                        .flat_map(|_| (ICON | 0xff00_0000).to_le_bytes())
                        .collect();
                    client.present(9, serial, extent, &pixels).unwrap();
                    client
                        .start_drag_with(
                            1,
                            9,
                            press.expect("pressed"),
                            DragActions::MOVE,
                            vec![(TextChunk::new("text/uri-list").unwrap(), b"x\r\n".to_vec())],
                            DragOptions {
                                vanish: true,
                                ..DragOptions::default()
                            },
                        )
                        .unwrap();
                }
                Event::DragCancelled { reason, .. } => return (reason, client),
                _ => {}
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let serve = |compositor: &mut Compositor, done: &mut dyn FnMut(&Compositor) -> bool| {
        while !done(compositor) {
            assert!(Instant::now() < deadline, "not in time");
            compositor.serve(&listener).unwrap();
        }
    };
    serve(&mut compositor, &mut |c| c.surface_rect(1, 1).is_some());
    let window = compositor.surface_rect(1, 1).unwrap();
    // The window whole on the test's clock: its arrival over.
    compositor.clock = Some(DROPPED);
    compositor.serve(&listener).unwrap();
    compositor
        .scene
        .pointer_motion(window.x + 100, window.y + 50);
    compositor.scene.pointer_button(272, true);
    serve(&mut compositor, &mut |c| c.scene.dragging());
    // Shown at the pointer, over the desktop far from the window.
    let drop = (600, 450);
    compositor.scene.pointer_motion(drop.0, drop.1);
    serve(&mut compositor, &mut |c| {
        c.scene
            .drag_icons()
            .iter()
            .any(|(_, shown)| shown.is_some_and(|r| r.x == drop.0 - 20))
    });
    // The output's pixels are looked at by the global space's logical points.
    let physical = |v: i32| v;
    let centre = (physical(drop.0), physical(drop.1));
    // The image's own pixel just inside its corner, while it is dragged.
    compositor.serve(&listener).unwrap();
    let corner = (physical(drop.0 - 18), physical(drop.1 - 18));
    assert_eq!(compositor.pixel(corner.0, corner.1) & 0x00ff_ffff, ICON);
    // A point between the drop and the window the drag came from, as it is while dragged.
    let way_back = (physical(drop.0 - 150), physical(drop.1 - 120));
    let behind = compositor.pixel(way_back.0, way_back.1);
    // Released over nothing: the drag is over, nothing took it.
    compositor.scene.pointer_button(272, false);
    let at = |compositor: &mut Compositor, ms: u64| {
        compositor.clock = Some(DROPPED + Duration::from_millis(ms));
        compositor.serve(&listener).unwrap();
    };
    at(&mut compositor, 0);
    let (reason, _client) = source.join().unwrap();
    assert_eq!(reason, DragCancelReason::NotAccepted);
    // The cloud at once where it was dropped; never the image going back toward the window.
    assert!(
        cloudy(compositor.pixel(centre.0, centre.1)),
        "{:08x}",
        compositor.pixel(centre.0, centre.1)
    );
    let start = clouded(&compositor, drop);
    for ms in [30, 60, 90, 120] {
        at(&mut compositor, ms);
        assert!(cloudy(compositor.pixel(centre.0, centre.1)), "{ms} ms");
        assert_eq!(
            compositor.pixel(way_back.0, way_back.1),
            behind,
            "{ms} ms: window {window:?}"
        );
        // The image itself is gone under it within a tenth of a second.
        if ms >= 90 {
            assert_ne!(
                compositor.pixel(corner.0, corner.1) & 0x00ff_ffff,
                ICON,
                "{ms} ms"
            );
        }
    }
    // It bursts out as it goes: more of the output is cloud a third of the way than at its start.
    at(&mut compositor, 130);
    let burst = clouded(&compositor, drop);
    assert!(burst > start, "{start} then {burst}");
    // Beyond the image's own square (40 × 40 about the drop), the cloud's: drawn there too.
    let beyond = (drop.0, drop.1 + 23);
    assert!(
        cloudy(compositor.pixel(beyond.0, beyond.1)),
        "{:08x}",
        compositor.pixel(beyond.0, beyond.1)
    );
    // Gone at its end, and the sound once.
    at(&mut compositor, 300);
    at(&mut compositor, 320);
    assert_eq!(compositor.pixel(centre.0, centre.1), DESKTOP);
    assert_eq!(compositor.pixel(beyond.0, beyond.1), DESKTOP);
    assert!(compositor.transitions.is_empty());
    assert_eq!(recorder.take(), [Sound::Poof]);
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_vanishing_drag_dropped_on_nothing_puffs_away_where_it_is_with_the_poof_sound_once() {
    run(120);
}

#[test]
fn the_poof_is_drawn_at_a_fractional_scale_too() {
    run(150);
}
