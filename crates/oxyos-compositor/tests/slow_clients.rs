//! A client slow to read never holds the compositor's turn (ADR 0001: the compositor never waits
//! on a client). A client that leaves its events unread fills its socket, and the compositor keeps
//! the rest in its outbox and polls the socket for room as well as for requests. When the client
//! reads, the socket has room: the turn sends the outbox, and reads nothing from a client that sent
//! nothing. A browser starting (Astro) or an application under load does exactly this: it reads
//! its events in a burst, then waits for the next one without sending anything.

use oxyos_client::{Client, Poll};
use oxyos_compositor::Compositor;
use oxyos_compositor_core::{Output, RenderItem, Scene};
use oxyos_ipc::{Connection, Listener};
use oxyos_protocol::{Event, PointerShape, Rect, Request, Size};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const KEY_A: u32 = 30;
/// Key events sent while the client reads none: more than its socket holds, fewer than the
/// outbox's limit beyond it (the client is slow, not gone).
const PRESSES: usize = 400;
/// How long the client stays silent after reading: a turn that waited for it would take this.
const SILENCE: Duration = Duration::from_secs(3);

#[test]
fn a_client_that_reads_its_events_and_sends_nothing_never_holds_the_turn() {
    let directory = std::env::temp_dir().join(format!("oxyos-slow-clients-{}", std::process::id()));
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

    let (shown, until_shown) = mpsc::channel();
    let (read, until_read) = mpsc::channel::<()>();
    let (drained, until_drained) = mpsc::channel();
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
        let mut presses = 0;
        let press = |event: &Event| usize::from(matches!(event, Event::Key { pressed: true, .. }));
        loop {
            let event = client
                .next_event()
                .unwrap()
                .expect("the compositor is there");
            if let Event::Configure { serial, rect, .. } = event {
                let pixels = vec![0xff; (rect.width * rect.height * 4) as usize];
                client
                    .present(1, serial, Size::new(rect.width, rect.height), &pixels)
                    .unwrap();
                break;
            }
        }
        shown.send(()).unwrap();
        // Busy: nothing read while the compositor sends more than the socket holds. Then, twice,
        // everything the socket holds read at once: the first time it answers the Ping the input
        // brought (ADR 0034), the second time it sends nothing.
        for _ in 0..2 {
            until_read.recv().unwrap();
            loop {
                match client.poll().unwrap() {
                    Poll::Event(event) => presses += press(&event),
                    Poll::Empty => break,
                    Poll::Closed => panic!("the compositor closed the connection"),
                }
            }
            drained.send(()).unwrap();
        }
        std::thread::sleep(SILENCE);
        // Speaking again, which is all a compositor waiting on it would have waited for.
        client
            .send(&Request::SetPointerShape {
                surface: 1,
                shape: PointerShape::Hand,
            })
            .unwrap();
        while presses < PRESSES {
            let event = client
                .next_event()
                .unwrap()
                .expect("the compositor is there");
            presses += press(&event);
        }
        keys.send(presses).unwrap();
    });

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut presented = false;
    while {
        presented |= until_shown.try_recv().is_ok();
        !presented
    } || !compositor
        .scene
        .render_list()
        .iter()
        .any(|item| matches!(item, RenderItem::Surface { .. }))
        || compositor.scene.focus().is_none()
    {
        assert!(Instant::now() < deadline, "the window was never shown");
        compositor.serve(&listener).unwrap();
    }
    // Everything the client sent read (it sends nothing more until it is woken).
    for _ in 0..5 {
        compositor.serve(&listener).unwrap();
    }
    // The client's socket fills; the rest waits in its outbox.
    for _ in 0..PRESSES {
        compositor.scene.key(KEY_A, true);
        compositor.scene.key(KEY_A, false);
    }
    compositor.serve(&listener).unwrap();
    read.send(()).unwrap();
    until_drained.recv_timeout(Duration::from_secs(10)).unwrap();
    // The Pong read, and the socket filled again from the outbox.
    for _ in 0..3 {
        compositor.serve(&listener).unwrap();
    }
    read.send(()).unwrap();
    until_drained.recv_timeout(Duration::from_secs(10)).unwrap();

    // The socket has room and the client sent nothing: each turn sends what it can and goes on.
    for turn in 0..5 {
        let started = Instant::now();
        compositor.serve(&listener).unwrap();
        let took = started.elapsed();
        assert!(
            took < Duration::from_millis(500),
            "turn {turn} waited {took:?} for a client that sent nothing"
        );
    }
    while keys_seen.try_recv().is_err() {
        assert!(Instant::now() < deadline, "the client never got its events");
        compositor.serve(&listener).unwrap();
    }
    client.join().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
