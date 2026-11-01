//! Key repeat through real sockets (ADR 0036): a key held is pressed again by the compositor's
//! turn when its time comes, the turn waking for it, and stops at its release.

use oxyos_client::Client;
use oxyos_compositor::Compositor;
use oxyos_compositor_core::{Output, RenderItem, Scene, REPEAT_DELAY, REPEAT_INTERVAL};
use oxyos_ipc::{Connection, Listener};
use oxyos_protocol::{Event, Rect, Request, Size};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const KEY_A: u32 = 30;

#[test]
fn a_held_key_is_pressed_again_by_the_turn_until_it_is_released() {
    let directory = std::env::temp_dir().join(format!("oxyos-key-repeat-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: Rect::new(0, 0, 1280, 720),
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    compositor.idle_wait = Duration::from_millis(20);

    let (keys, keys_seen) = mpsc::channel();
    let path = socket.clone();
    let client = std::thread::spawn(move || {
        let mut client = Client::handshake(Connection::connect(&path).unwrap()).unwrap();
        client
            .send(&Request::CreateToplevel {
                surface: 1,
                size: Size::new(200, 100),
            })
            .unwrap();
        loop {
            match client
                .next_event()
                .unwrap()
                .expect("the compositor is there")
            {
                Event::Configure { serial, rect, .. } => {
                    let pixels = vec![0xff; (rect.width * rect.height * 4) as usize];
                    client
                        .present(1, serial, Size::new(rect.width, rect.height), &pixels)
                        .unwrap();
                }
                Event::Key {
                    key,
                    pressed,
                    time_ns,
                    ..
                } => {
                    keys.send((key, pressed, time_ns)).unwrap();
                    if !pressed {
                        return;
                    }
                }
                _ => {}
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !compositor
        .scene
        .render_list()
        .iter()
        .any(|item| matches!(item, RenderItem::Surface { .. }))
        || compositor.scene.focus().is_none()
    {
        assert!(Instant::now() < deadline, "the window was never shown");
        compositor.serve(&listener).unwrap();
    }
    // The key held, at a time the compositor's clock says (ahead of the system's, which the turns
    // so far told the scene: its clock never goes back).
    let start = oxyos_compositor::monotonic() + Duration::from_secs(3600);
    compositor.clock = Some(start);
    compositor.scene.set_clock(start);
    compositor.scene.set_input_time(start.as_nanos() as u64);
    compositor
        .scene
        .keyboard_repeating(KEY_A, true, Some("a"), true);
    let mut seen = Vec::new();
    let mut take = |compositor: &mut Compositor, count: usize| {
        while seen.len() < count {
            assert!(Instant::now() < deadline, "keys missing: {seen:?}");
            compositor.serve(&listener).unwrap();
            seen.extend(keys_seen.try_iter());
        }
        seen.clone()
    };
    assert_eq!(
        take(&mut compositor, 1),
        [(KEY_A, true, start.as_nanos() as u64)]
    );
    // Before the delay the turn presses nothing again; at it, and an interval later, it does.
    compositor.clock = Some(start + REPEAT_DELAY - Duration::from_millis(1));
    for _ in 0..3 {
        compositor.serve(&listener).unwrap();
    }
    assert_eq!(compositor.scene.next_repeat(), Some(start + REPEAT_DELAY));
    let first = start + REPEAT_DELAY;
    compositor.clock = Some(first);
    let second = first + REPEAT_INTERVAL;
    take(&mut compositor, 2);
    compositor.clock = Some(second);
    let presses = take(&mut compositor, 3);
    assert_eq!(
        presses,
        [
            (KEY_A, true, start.as_nanos() as u64),
            (KEY_A, true, first.as_nanos() as u64),
            (KEY_A, true, second.as_nanos() as u64),
        ]
    );
    // Released: the release, and nothing more however long the turns go on.
    compositor
        .scene
        .keyboard_repeating(KEY_A, false, None, false);
    assert_eq!(compositor.scene.next_repeat(), None);
    compositor.clock = Some(second + Duration::from_secs(5));
    let all = take(&mut compositor, 4);
    assert!(!all[3].1);
    client.join().unwrap();
    assert!(keys_seen.try_iter().next().is_none());
}
