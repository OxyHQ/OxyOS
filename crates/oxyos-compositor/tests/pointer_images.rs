//! A client's pointer image through real sockets (ADR 0029): the client library puts the pixels
//! in a buffer of their own, the compositor reads them when the request arrives and keeps them
//! for the surface, and the image is the pointer's while it is over that surface, until a shape
//! replaces it.

use oxyos_client::Client;
use oxyos_compositor::{pointer_image_at, Compositor};
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{Event, PointerShape, Rect, Request, Size};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn a_clients_pointer_image_is_read_at_once_and_shown_over_its_surface() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-pointer-images-{}", std::process::id()));
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

    let (set, image_set) = mpsc::channel();
    let (go_on, shape_wanted) = mpsc::channel::<()>();
    let path = socket.clone();
    let client = std::thread::spawn(move || {
        let mut client = Client::connect(&path).unwrap();
        client
            .send(&Request::CreateToplevel {
                surface: 1,
                size: Size::new(200, 100),
            })
            .unwrap();
        loop {
            if let Event::Configure { serial, rect, .. } = client.next_event().unwrap().unwrap() {
                let pixels = vec![0xff; (rect.width * rect.height * 4) as usize];
                client
                    .present(1, serial, Size::new(rect.width, rect.height), &pixels)
                    .unwrap();
                break;
            }
        }
        let mut pixels = Vec::new();
        for index in 0..6u32 {
            pixels.extend_from_slice(&(0xff00_0000 | index).to_le_bytes());
        }
        assert!(client
            .set_pointer_image(1, Size::new(3, 2), &pixels, (5, 0), 240)
            .is_err());
        assert!(client
            .set_pointer_image(1, Size::new(3, 2), &pixels[..20], (1, 1), 240)
            .is_err());
        client
            .set_pointer_image(1, Size::new(3, 2), &pixels, (1, 1), 240)
            .unwrap();
        set.send(()).unwrap();
        shape_wanted.recv().unwrap();
        client
            .send(&Request::SetPointerShape {
                surface: 1,
                shape: PointerShape::Hand,
            })
            .unwrap();
        set.send(()).unwrap();
        shape_wanted.recv().unwrap();
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let serve_until = |compositor: &mut Compositor, done: &dyn Fn(&Compositor) -> bool| {
        while !done(compositor) {
            assert!(Instant::now() < deadline, "the compositor never got there");
            compositor.serve(&listener).unwrap();
        }
    };
    serve_until(&mut compositor, &|c| c.scene.render_list().len() == 1);
    image_set.recv_timeout(Duration::from_secs(10)).unwrap();
    serve_until(&mut compositor, &|c| !c.client_pointers.is_empty());
    let (&key, pointer) = compositor.client_pointers.iter().next().unwrap();
    assert_eq!((pointer.image.width, pointer.image.height), (3, 2));
    assert_eq!(pointer.image.hotspot, (1, 1));
    assert_eq!(pointer.scale_120, 240);
    assert_eq!(
        pointer.image.pixels,
        (0..6u32)
            .map(|index| 0xff00_0000 | index)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        compositor.scene.pointer_image(),
        None,
        "the pointer is elsewhere"
    );
    let rect = compositor.surface_rect(key.client, key.id).unwrap();
    compositor.scene.pointer_motion(rect.x + 10, rect.y + 10);
    assert_eq!(compositor.scene.pointer_image(), Some((key, pointer.id)));

    go_on.send(()).unwrap();
    image_set.recv_timeout(Duration::from_secs(10)).unwrap();
    serve_until(&mut compositor, &|c| c.scene.pointer_image().is_none());
    assert_eq!(compositor.scene.pointer_shape(), PointerShape::Hand);
    go_on.send(()).unwrap();
    client.join().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_pointer_image_is_shown_at_the_displays_scale() {
    let image = oxyos_kms::PointerImage {
        width: 4,
        height: 2,
        pixels: (0..8).collect(),
        hotspot: (2, 1),
    };
    assert_eq!(pointer_image_at(&image, 240, 240), image);
    let half = pointer_image_at(&image, 240, 120);
    assert_eq!((half.width, half.height), (2, 1));
    assert_eq!(half.pixels, [0, 2]);
    assert_eq!(half.hotspot, (1, 0));
    let larger = pointer_image_at(&image, 120, 180);
    assert_eq!((larger.width, larger.height), (6, 3));
    assert_eq!(larger.hotspot, (3, 1));
}
