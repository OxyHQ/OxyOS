//! Drag and drop through real sockets (docs/adr/0035-drag-and-drop.mdx): one application drags a
//! file from a press it holds, another accepts it as the pointer crosses its window and, dropped
//! on, receives exactly the offered bytes once through the pipe the compositor passes to the
//! source, whose library writes them.

use oxyos_client::Client;
use oxyos_compositor::Compositor;
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{DragActions, Event, PointerKind, Rect, Request, Size, TextChunk};
use std::io::Read;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const URIS: &str = "text/uri-list";
const PLAIN: &str = "text/plain;charset=utf-8";
const FILE: &[u8] = b"file:///home/ana/informe.pdf\r\n";

/// A window of the client's own, shown: its configure answered with one frame.
fn window(client: &mut Client) {
    client
        .send(&Request::CreateToplevel {
            surface: 1,
            size: Size::new(200, 100),
        })
        .unwrap();
    loop {
        if let Event::Configure {
            surface: 1,
            serial,
            rect,
            ..
        } = client
            .next_event()
            .unwrap()
            .expect("the compositor is there")
        {
            let pixels = vec![0xff; (rect.width * rect.height * 4) as usize];
            client
                .present(1, serial, Size::new(rect.width, rect.height), &pixels)
                .unwrap();
            return;
        }
    }
}

#[test]
fn a_file_dragged_to_another_application_reaches_it_once_through_a_pipe() {
    let directory = std::env::temp_dir().join(format!("oxyos-drag-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: Rect::new(0, 0, 1920, 1080),
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    let deadline = Instant::now() + Duration::from_secs(20);
    let serve_until = |compositor: &mut Compositor,
                       done: &mut dyn FnMut(&mut Compositor) -> bool| {
        while !done(compositor) {
            assert!(
                Instant::now() < deadline,
                "the drag did not get there in time"
            );
            compositor.serve(&listener).unwrap();
        }
    };

    // The source drags its file, offered as a URI list and as text, from the press it is given;
    // it hears what came of the drag.
    let (source_ready, source_shown) = mpsc::channel();
    let path = socket.clone();
    let source = std::thread::spawn(move || {
        let mut client = Client::connect(&path).unwrap();
        window(&mut client);
        source_ready.send(()).unwrap();
        let mut heard = Vec::new();
        loop {
            match client
                .next_event()
                .unwrap()
                .expect("the compositor is there")
            {
                Event::Pointer {
                    kind: PointerKind::Press,
                    serial,
                    ..
                } => {
                    client
                        .start_drag(
                            1,
                            0,
                            serial,
                            DragActions::COPY.union(DragActions::MOVE),
                            vec![
                                (TextChunk::new(URIS).unwrap(), FILE.to_vec()),
                                (
                                    TextChunk::new(PLAIN).unwrap(),
                                    b"/home/ana/informe.pdf".to_vec(),
                                ),
                            ],
                        )
                        .unwrap();
                }
                event @ (Event::DragSourceAction { .. } | Event::DragDropped { .. }) => {
                    heard.push(event)
                }
                event @ (Event::DragFinished { .. } | Event::DragCancelled { .. }) => {
                    heard.push(event);
                    return heard;
                }
                _ => {}
            }
        }
    });
    let mut shown = false;
    serve_until(&mut compositor, &mut |c| {
        shown |= source_shown.try_recv().is_ok();
        shown && c.scene.focus().is_some()
    });
    let source_window = compositor.scene.focus().unwrap();

    // The target takes URI lists, preferring a move, and reads each type it receives to its end.
    let (target_ready, target_shown) = mpsc::channel();
    let path = socket.clone();
    let target = std::thread::spawn(move || {
        let mut client = Client::connect(&path).unwrap();
        window(&mut client);
        target_ready.send(()).unwrap();
        let mut entered = None;
        loop {
            match client
                .next_event()
                .unwrap()
                .expect("the compositor is there")
            {
                Event::DragEnter { offer, serial, .. } => entered = Some((offer, serial)),
                Event::DragOfferType { offer, mime } if mime.as_str() == URIS => {
                    let (entered_offer, serial) = entered.expect("entered before its types");
                    assert_eq!(offer, entered_offer);
                    client
                        .accept_drag(
                            offer,
                            serial,
                            Some(mime),
                            DragActions::COPY.union(DragActions::MOVE),
                            DragActions::MOVE,
                        )
                        .unwrap();
                }
                Event::DragDrop { offer, action, .. } => {
                    let mime = TextChunk::new(URIS).unwrap();
                    let read = |pipe| {
                        let mut bytes = Vec::new();
                        std::fs::File::from(pipe).read_to_end(&mut bytes).unwrap();
                        bytes
                    };
                    let first = read(client.receive_drop(offer, mime).unwrap());
                    // The same type again is refused: its pipe ends at once, empty.
                    let again = read(client.receive_drop(offer, mime).unwrap());
                    client.finish_drop(offer).unwrap();
                    // The connection stays until the test ends: a client gone at once could be
                    // forgotten before its finish is read.
                    return (action, first, again, client);
                }
                _ => {}
            }
        }
    });
    let mut shown = false;
    serve_until(&mut compositor, &mut |c| {
        shown |= target_shown.try_recv().is_ok();
        shown
            && c.scene
                .focus()
                .is_some_and(|key| key.client != source_window.client)
    });
    let target_window = compositor.scene.focus().unwrap();
    compositor.scene.move_window_to(source_window, (0, 0));
    compositor.scene.move_window_to(target_window, (600, 0));

    // A press on the source's window starts the drag; the pointer crosses onto the target's, which
    // accepts; the release drops.
    compositor.scene.pointer_motion(100, 50);
    compositor.scene.pointer_button(272, true);
    serve_until(&mut compositor, &mut |c| c.scene.dragging());
    compositor.scene.pointer_motion(400, 50);
    compositor.scene.pointer_motion(700, 50);
    // Until the target's answer has been taken: a few turns of the loop.
    let settle = Instant::now() + Duration::from_millis(300);
    serve_until(&mut compositor, &mut |_| Instant::now() > settle);
    compositor.scene.pointer_button(272, false);
    serve_until(&mut compositor, &mut |_| {
        source.is_finished() && target.is_finished()
    });
    let (action, first, again, _client) = target.join().unwrap();
    assert_eq!(action, DragActions::MOVE);
    assert_eq!(first, FILE, "exactly the offered bytes");
    assert!(again.is_empty(), "and only once");
    let heard = source.join().unwrap();
    assert_eq!(
        heard,
        [
            Event::DragSourceAction {
                drag: 1,
                action: DragActions::MOVE
            },
            Event::DragDropped {
                drag: 1,
                action: DragActions::MOVE
            },
            Event::DragFinished {
                drag: 1,
                action: DragActions::MOVE
            },
        ]
    );
    std::fs::remove_dir_all(&directory).unwrap();
}
