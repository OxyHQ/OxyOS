//! A long answer of the session's vocabulary through real sockets (SystemData, protocol version
//! 32, docs/adr/0060-activity-monitor.mdx): one event carrying a sealed memory file, which the
//! client library reads whole, where SystemText's 32-byte chunks would be thousands of events and
//! a client that did not read them at once would be dropped.

use oxyos_client::Client;
use oxyos_compositor::Compositor;
use oxyos_compositor_core::{Grants, Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{Event, Rect};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn a_long_answer_arrives_whole_in_one_sealed_file() {
    let directory = std::env::temp_dir().join(format!("oxyos-system-data-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: Rect::new(0, 0, 800, 600),
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);

    // Two hundred kilobytes: 6,400 SystemText chunks, past what a client may leave unread.
    let text: String = format!(
        "{{\"activity\":{{\"processes\":[{}]}}}}",
        (0..4000)
            .map(|pid| format!("{{\"pid\":{pid},\"name\":\"process number {pid}\"}}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(text.len() > 150_000);
    let (connected, ready) = mpsc::channel();
    let (received, answer) = mpsc::channel();
    let path = socket.clone();
    let client = std::thread::spawn(move || {
        let mut client = Client::connect(&path).unwrap();
        connected.send(()).unwrap();
        loop {
            match client.next_event().unwrap() {
                Some(Event::SystemData { serial, size }) => {
                    received
                        .send((serial, size, client.system_data().map(str::to_owned)))
                        .unwrap();
                    return;
                }
                Some(Event::SystemText { .. }) => panic!("sent in chunks"),
                Some(_) => {}
                None => panic!("the compositor dropped the client"),
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut ready_seen = false;
    let mut sent = false;
    loop {
        assert!(Instant::now() < deadline, "no answer in time");
        compositor.serve(&listener).unwrap();
        ready_seen |= ready.try_recv().is_ok();
        let client = compositor.peers.keys().next().copied();
        if let (true, false, Some(client)) = (ready_seen, sent, client) {
            // The client has said Hello: its version is known. Granted what asks the session, as
            // Activity Monitor is.
            if compositor.scene.grants_of(client).is_some() {
                compositor
                    .scene
                    .grant(
                        client,
                        Grants {
                            activity: true,
                            ..Grants::default()
                        },
                    )
                    .unwrap();
                assert!(compositor.system_data(client, 0, &text));
                sent = true;
            }
        }
        if let Ok((serial, size, data)) = answer.try_recv() {
            assert_eq!(serial, 0);
            assert_eq!(size as usize, text.len());
            assert_eq!(data.as_deref(), Some(text.as_str()));
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    client.join().unwrap();
}
