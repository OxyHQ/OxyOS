//! A client's burst of requests is read in a few turns, not one turn a request: a window's menus
//! or an accessibility snapshot are hundreds of packets, and what the client sends after them (a
//! frame) must not wait a turn for each of them.

use oxyos_client::Client;
use oxyos_compositor::Compositor;
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{PointerShape, Rect, Request, Size};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[test]
fn a_burst_of_requests_is_read_in_a_few_turns() {
    let directory = std::env::temp_dir().join(format!("oxyos-requests-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket: PathBuf = directory.join("compositor");
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
    compositor.idle_wait = Duration::from_millis(5);

    // The client sends its whole burst while the compositor is not serving, so every request is
    // waiting on the socket when the turns are counted, however slowly the client wrote them. A
    // burst the socket holds whole (each packet is accounted at its kernel buffer's size), and
    // fewer requests than a turn reads: all of it is read in one turn.
    let (connected, until_connected) = std::sync::mpsc::channel();
    let (go, until_go) = std::sync::mpsc::channel();
    let (sent, until_sent) = std::sync::mpsc::channel();
    let path = socket.clone();
    let client = std::thread::spawn(move || {
        let client = Client::connect(&path).unwrap();
        connected.send(()).unwrap();
        until_go.recv().unwrap();
        client
            .send(&Request::CreateToplevel {
                surface: 1,
                size: Size::new(200, 100),
            })
            .unwrap();
        for _ in 0..100 {
            client
                .send(&Request::SetPointerShape {
                    surface: 1,
                    shape: PointerShape::Hand,
                })
                .unwrap();
        }
        client
            .send(&Request::CreateToplevel {
                surface: 2,
                size: Size::new(200, 100),
            })
            .unwrap();
        sent.send(()).unwrap();
        client
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    while until_connected.try_recv().is_err() {
        assert!(Instant::now() < deadline);
        compositor.serve(&listener).unwrap();
    }
    go.send(()).unwrap();
    until_sent.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut turns = 0;
    while compositor.scene.stats().surfaces < 2 {
        assert!(Instant::now() < deadline);
        compositor.serve(&listener).unwrap();
        turns += 1;
    }
    assert_eq!(turns, 1, "102 requests took {turns} turns");
    drop(client.join().unwrap());
    std::fs::remove_dir_all(directory).unwrap();
}
