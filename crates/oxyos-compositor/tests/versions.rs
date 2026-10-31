//! A client of an older protocol version against this compositor, through real sockets (ADR 0033):
//! it is served, its events come as its version defines them, and a request of a newer version is
//! refused from it.

use oxyos_client::Client;
use oxyos_compositor::Compositor;
use oxyos_compositor_core::{Output, RenderItem, Scene};
use oxyos_ipc::{Connection, Listener};
use oxyos_protocol::{ClientConnection, Event, PointerKind, Rect, Request, Size};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn compositor(name: &str) -> (Compositor, Listener, PathBuf) {
    let directory =
        std::env::temp_dir().join(format!("oxyos-versions-{name}-{}", std::process::id()));
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
    (Compositor::new(scene, uid), listener, socket)
}

/// A client that speaks only version 2, as an engine built before version 3 does.
fn version_2_client(path: &std::path::Path) -> Connection {
    let connection = Connection::connect(path).unwrap();
    let (mut state, hello) = ClientConnection::new(2, 2);
    connection.send_request(&hello, None).unwrap();
    state
        .receive(&connection.recv_event_for(2).unwrap().0)
        .unwrap();
    assert_eq!(state.version(), Some(2));
    connection
}

#[test]
fn a_version_2_client_is_served_with_version_2_events() {
    let (mut compositor, listener, socket) = compositor("served");
    let (pressed, press_seen) = mpsc::channel();
    let path = socket.clone();
    let client = std::thread::spawn(move || {
        let mut client =
            Client::handshake_versions(Connection::connect(&path).unwrap(), 2, 2).unwrap();
        assert_eq!(client.version(), 2);
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
                Event::Pointer {
                    kind: PointerKind::Press,
                    time_ns,
                    ..
                } => {
                    pressed.send(time_ns).unwrap();
                    return;
                }
                _ => {}
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let rect = loop {
        assert!(Instant::now() < deadline, "the window was never shown");
        compositor.serve(&listener).unwrap();
        if let Some(rect) = compositor
            .scene
            .render_list()
            .iter()
            .find_map(|item| match item {
                RenderItem::Surface { rect, .. } => Some(*rect),
                _ => None,
            })
        {
            break rect;
        }
    };
    // The press has a device time, which a version 3 client is told and version 2 has no place for.
    compositor.scene.set_input_time(5_000_000_000);
    compositor.scene.pointer_motion(rect.x + 10, rect.y + 10);
    compositor.scene.pointer_button(272, true);
    loop {
        assert!(
            Instant::now() < deadline,
            "the press never reached the client"
        );
        compositor.serve(&listener).unwrap();
        if let Ok(time_ns) = press_seen.try_recv() {
            assert_eq!(
                time_ns, 0,
                "read as version 2 lays it out, with no device time"
            );
            break;
        }
    }
    client.join().unwrap();
}

#[test]
fn a_request_of_a_newer_version_is_refused_from_an_older_client() {
    let (mut compositor, listener, socket) = compositor("refused");
    let path = socket.clone();
    let client = std::thread::spawn(move || {
        let connection = version_2_client(&path);
        connection
            .send_request(&Request::DestroyTimeline { timeline: 1 }, None)
            .unwrap();
        loop {
            match connection.recv_event_for(2) {
                Ok((Event::ProtocolError { .. }, _)) | Err(_) => return,
                Ok(_) => {}
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !client.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the older client was not refused"
        );
        compositor.serve(&listener).unwrap();
    }
    client.join().unwrap();
}
