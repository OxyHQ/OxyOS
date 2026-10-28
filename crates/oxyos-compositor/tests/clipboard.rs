//! The clipboard through real sockets (docs/adr/0007-clipboard.mdx): one application copies with
//! a key typed into it, another pastes with a key of its own, and the content goes from the
//! owner to the receiver through the pipe the compositor passes from one to the other.

use oxyos_client::Client;
use oxyos_compositor::Compositor;
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{Event, Rect, Request, Size, TextChunk};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const PLAIN: &str = "text/plain;charset=utf-8";

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

/// A key press and its release, into whichever window has focus.
fn type_key(compositor: &mut Compositor) {
    compositor.scene.key(30, true);
    compositor.scene.key(30, false);
}

#[test]
fn a_copy_in_one_application_is_pasted_in_another_through_a_pipe() {
    let directory = std::env::temp_dir().join(format!("oxyos-clipboard-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket: PathBuf = directory.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    let output = Rect::new(0, 0, 1920, 1080);
    scene
        .add_output(Output {
            id: 1,
            rect: output,
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    // The test's own user, whom the compositor accepts: the owner of the directory it made.
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid, output);

    // The owner copies plain text with the first key it is given, and writes it when asked.
    let (owner_ready, owner_shown) = mpsc::channel();
    let (owner_copied, copied) = mpsc::channel();
    let path = socket.clone();
    let owner = std::thread::spawn(move || {
        let mut client = Client::connect(&path).unwrap();
        window(&mut client);
        owner_ready.send(()).unwrap();
        loop {
            match client
                .next_event()
                .unwrap()
                .expect("the compositor is there")
            {
                Event::Key {
                    pressed: true,
                    serial,
                    ..
                } => {
                    for request in [
                        Request::ClipboardOffer { offer: 1, serial },
                        Request::ClipboardType {
                            offer: 1,
                            mime: TextChunk::new(PLAIN).unwrap(),
                        },
                        Request::ClipboardCommit { offer: 1 },
                    ] {
                        client.send(&request).unwrap();
                    }
                    owner_copied.send(()).unwrap();
                }
                Event::ClipboardSend { offer: 1, mime } => {
                    assert_eq!(mime.as_str(), PLAIN);
                    let pipe = client
                        .take_descriptor()
                        .expect("the pipe comes with the event");
                    std::fs::File::from(pipe)
                        .write_all("hola OxyOS".as_bytes())
                        .unwrap();
                    return;
                }
                _ => {}
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(20);
    let serve_until = |compositor: &mut Compositor,
                       done: &mut dyn FnMut(&mut Compositor) -> bool| {
        while !done(compositor) {
            assert!(
                Instant::now() < deadline,
                "the clipboard did not get there in time"
            );
            compositor.serve(&listener).unwrap();
        }
    };
    // Once the owner's window has focus, a key: it copies.
    // A message is taken once: whether it came is kept.
    let mut shown = false;
    serve_until(&mut compositor, &mut |c| {
        shown |= owner_shown.try_recv().is_ok();
        shown && c.scene.focus().is_some()
    });
    let owner_client = compositor.scene.focus().unwrap().client;
    type_key(&mut compositor);
    serve_until(&mut compositor, &mut |_| copied.try_recv().is_ok());

    // The receiver is told what the clipboard holds when its window takes focus, and pastes
    // with the first key it is given.
    let (receiver_ready, receiver_shown) = mpsc::channel();
    let path = socket.clone();
    let receiver = std::thread::spawn(move || {
        let mut client = Client::connect(&path).unwrap();
        window(&mut client);
        receiver_ready.send(()).unwrap();
        let mut selection = None;
        loop {
            match client
                .next_event()
                .unwrap()
                .expect("the compositor is there")
            {
                Event::ClipboardType {
                    selection: told,
                    mime,
                } if mime.as_str() == PLAIN => {
                    selection = Some(told);
                }
                Event::Key {
                    pressed: true,
                    serial,
                    ..
                } => {
                    let selection =
                        selection.expect("told what the clipboard holds before the key");
                    let pipe = client
                        .receive_clipboard(selection, TextChunk::new(PLAIN).unwrap(), serial)
                        .unwrap();
                    let mut text = String::new();
                    std::fs::File::from(pipe).read_to_string(&mut text).unwrap();
                    return text;
                }
                _ => {}
            }
        }
    });
    let mut shown = false;
    serve_until(&mut compositor, &mut |c| {
        shown |= receiver_shown.try_recv().is_ok();
        shown
            && c.scene
                .focus()
                .is_some_and(|key| key.client != owner_client)
    });
    type_key(&mut compositor);
    serve_until(&mut compositor, &mut |_| {
        receiver.is_finished() && owner.is_finished()
    });
    owner.join().unwrap();
    assert_eq!(receiver.join().unwrap(), "hola OxyOS");
    std::fs::remove_dir_all(&directory).unwrap();
}
