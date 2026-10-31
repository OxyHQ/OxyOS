//! Direct scanout of a full-screen window on a real display (docs/adr/0031-gpu-buffers.mdx,
//! decision 8), through real sockets: the surface is told the primary plane's formats once it
//! is all the output shows, its buffer goes on the plane instead of a composition (or, where the
//! card refuses a foreign DMA-BUF, is composed, never shown black or stale), and a buffer on the
//! display is not released before a later flip replaced it, even once its surface is gone.
//! Needs a KMS card this process can be DRM master of, udmabuf and a render node with timeline
//! syncobjs: `tools/gpu-vm/gpu-vm` has them (virtio-gpu); elsewhere it says so and skips.

use oxyos_compositor::Compositor;
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::{Connection, Listener};
use oxyos_protocol::{Event, Format, Outcome, Rect, Request, Size};
use rustix::fd::AsFd;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A display of the first card that has one and can be ours.
fn display() -> Option<oxyos_kms::Display> {
    let mut cards: Vec<_> = std::fs::read_dir("/dev/dri")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("card"))
        })
        .collect();
    cards.sort();
    cards.into_iter().find_map(|path| {
        let card = oxyos_kms::Card::open(&path).ok()?;
        oxyos_kms::Display::open(card).ok()
    })
}

/// Where the client's buffers are made: shared memory (udmabuf), which a display's card may
/// refuse as a framebuffer, or the display's card itself, as a client drawing on the GPU that
/// drives the display allocates.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    SharedMemory,
    TheCard,
}

/// A buffer of one colour and its row pitch.
fn filled(
    source: Source,
    card: &oxyos_kms::Card,
    width: u32,
    height: u32,
    colour: u32,
) -> (std::os::fd::OwnedFd, u32) {
    let row: Vec<u8> = (0..width).flat_map(|_| colour.to_le_bytes()).collect();
    match source {
        Source::SharedMemory => {
            let stride = width * 4;
            let (memfd, buffer) = oxyos_dmabuf::shared_dmabuf(u64::from(stride * height)).unwrap();
            for y in 0..height {
                rustix::io::pwrite(&memfd, &row, u64::from(y * stride)).unwrap();
            }
            (buffer.into_fd(), stride)
        }
        Source::TheCard => {
            use drm::buffer::Buffer as _;
            use drm::control::Device as _;
            let mut buffer = card
                .create_dumb_buffer((width, height), drm::buffer::DrmFourcc::Xrgb8888, 32)
                .unwrap();
            let pitch = buffer.pitch();
            {
                let mut mapping = card.map_dumb_buffer(&mut buffer).unwrap();
                for y in 0..height as usize {
                    let start = y * pitch as usize;
                    mapping[start..start + row.len()].copy_from_slice(&row);
                }
            }
            // DRM_CLOEXEC | DRM_RDWR.
            let fd = card
                .buffer_to_prime_fd(buffer.handle(), 0x8_0000 | 2)
                .unwrap();
            (fd, pitch)
        }
    }
}

fn until(connection: &Connection, seen: &mut Vec<Event>, want: impl Fn(&Event) -> bool) -> Event {
    if let Some(index) = seen.iter().position(&want) {
        return seen.remove(index);
    }
    loop {
        let (event, _) = connection.recv_event().unwrap();
        if want(&event) {
            return event;
        }
        seen.push(event);
    }
}

#[derive(Debug, PartialEq)]
enum Step {
    Shown(Vec<(Format, u64)>),
    Destroyed,
    Released(Instant),
}

#[test]
fn a_full_screen_window_in_shared_memory_is_scanned_out_or_composed() {
    scanned_out(Source::SharedMemory);
}

#[test]
fn a_full_screen_window_in_the_cards_memory_is_scanned_out_and_held_until_a_later_flip() {
    scanned_out(Source::TheCard);
}

fn scanned_out(source: Source) {
    if oxyos_dmabuf::shared_dmabuf(4096).is_err()
        || oxyos_dmabuf::RenderNode::with_timelines(None).is_none()
    {
        eprintln!("no udmabuf or timeline syncobjs here, skipped; tools/gpu-vm runs it");
        return;
    }
    let Some(display) = display() else {
        eprintln!("no display this process can drive here, skipped; tools/gpu-vm runs it");
        return;
    };
    let (width, height) = display.size();
    let buffers = [0xffff_0000, 0xff00_ff00]
        .map(|colour| filled(source, display.card(), width, height, colour));
    let directory = std::env::temp_dir().join(format!(
        "oxyos-scanout-{}-{}",
        std::process::id(),
        source == Source::TheCard
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let _ = std::fs::remove_file(&socket);
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: Rect::new(0, 0, width, height),
            scale_120: 120,
            refresh_mhz: display.mode().refresh_mhz,
        })
        .unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    compositor.screens[0].display = Some(display);
    compositor.offer_gpu_buffers();
    let (to_test, steps) = mpsc::channel();
    let (to_client, go) = mpsc::channel::<()>();
    let path = socket.clone();
    let client = std::thread::spawn(move || {
        let connection = Connection::connect(Path::new(&path)).unwrap();
        connection.set_timeouts(Duration::from_secs(20)).unwrap();
        connection
            .send_request(
                &Request::Hello {
                    min_version: oxyos_protocol::MIN_VERSION,
                    max_version: oxyos_protocol::VERSION,
                },
                None,
            )
            .unwrap();
        let mut seen = Vec::new();
        until(&connection, &mut seen, |event| {
            matches!(event, Event::Welcome { .. })
        });
        connection
            .send_request(
                &Request::CreateToplevel {
                    surface: 1,
                    size: Size::new(640, 400),
                },
                None,
            )
            .unwrap();
        connection
            .send_request(
                &Request::SetFullscreen {
                    surface: 1,
                    enabled: true,
                },
                None,
            )
            .unwrap();
        let serial = loop {
            if let Event::Configure {
                surface: 1,
                serial,
                rect,
                fullscreen: true,
                ..
            } = until(&connection, &mut seen, |event| {
                matches!(event, Event::Configure { surface: 1, .. })
            }) {
                assert_eq!((rect.width, rect.height), (width, height));
                break serial;
            }
        };
        for (id, (buffer, stride)) in (1..).zip(&buffers) {
            connection
                .send_request(
                    &Request::AttachDmabufPlane {
                        buffer: id,
                        plane: 0,
                        offset: 0,
                        stride: *stride,
                    },
                    Some(buffer.as_fd()),
                )
                .unwrap();
            connection
                .send_request(
                    &Request::AttachDmabuf {
                        buffer: id,
                        format: Format::Xrgb8888,
                        modifier: 0,
                        extent: Size::new(width, height),
                        planes: 1,
                    },
                    None,
                )
                .unwrap();
        }
        let submit = |sequence: u64, buffer: u64| {
            connection
                .send_request(
                    &Request::Submit {
                        surface: 1,
                        sequence,
                        buffer,
                        configure: serial,
                        extent: Size::new(width, height),
                    },
                    None,
                )
                .unwrap();
        };
        submit(1, 1);
        // Told its scanout formats once it is all the output shows.
        let Event::GpuScanout { formats, .. } = until(
            &connection,
            &mut seen,
            |event| matches!(event, Event::GpuScanout { surface: 1, formats } if *formats > 0),
        ) else {
            unreachable!()
        };
        let told: Vec<(Format, u64)> = (0..formats)
            .map(|index| match until(&connection, &mut seen, |event| {
                matches!(event, Event::GpuScanoutFormat { surface: 1, index: told, .. } if *told == index)
            }) {
                Event::GpuScanoutFormat {
                    format, modifier, ..
                } => (format, modifier),
                _ => unreachable!(),
            })
            .collect();
        until(&connection, &mut seen, |event| {
            matches!(
                event,
                Event::Feedback {
                    sequence: 1,
                    outcome: Outcome::Presented,
                    ..
                }
            )
        });
        submit(2, 2);
        until(&connection, &mut seen, |event| {
            matches!(
                event,
                Event::Feedback {
                    sequence: 2,
                    outcome: Outcome::Presented,
                    ..
                }
            )
        });
        to_test.send(Step::Shown(told)).unwrap();
        go.recv().unwrap();
        // The surface goes while green is on the display: green is released only once the frame
        // without it has flipped.
        connection
            .send_request(&Request::Destroy { surface: 1 }, None)
            .unwrap();
        to_test.send(Step::Destroyed).unwrap();
        until(&connection, &mut seen, |event| {
            matches!(event, Event::Released { buffer: 2, .. })
        });
        to_test.send(Step::Released(Instant::now())).unwrap();
        go.recv().ok();
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let next = |compositor: &mut Compositor| loop {
        assert!(Instant::now() < deadline, "not in time");
        if let Ok(step) = steps.try_recv() {
            return step;
        }
        compositor.serve(&listener).unwrap();
    };
    let Step::Shown(told) = next(&mut compositor) else {
        panic!("expected the window shown");
    };
    assert!(told.contains(&(Format::Xrgb8888, 0)), "{told:?}");
    let direct = compositor.direct_frames > 0;
    if source == Source::TheCard {
        assert!(direct, "a buffer of the display's own card is scanned out");
    }
    if direct {
        assert!(compositor.shows_client_buffer(0));
    } else {
        // The card refused the buffer (virtio-gpu takes no foreign DMA-BUF without blob
        // resources): it is composed, and shows what the client drew.
        eprintln!("the card refused the client's buffer: composed instead");
        assert!(!compositor.shows_client_buffer(0));
    }
    let (x, y) = (width as i32 / 2, height as i32 / 2);
    assert_eq!(compositor.pixel(x, y), 0x0000_ff00, "green shown");
    if direct {
        // Something over the window (the session's splash): composed, the window under it; then
        // gone, the window's buffer on the plane again.
        let before = compositor.direct_frames;
        compositor.show_splash();
        let deadline = Instant::now() + Duration::from_secs(10);
        while compositor.shows_client_buffer(0) {
            assert!(Instant::now() < deadline, "the splash was never composed");
            compositor.serve(&listener).unwrap();
        }
        assert_eq!(
            compositor.direct_frames, before,
            "scanned out under the splash"
        );
        assert_ne!(compositor.pixel(x, y), 0x0000_ff00, "the splash shown");
        compositor.end_splash();
        while !compositor.shows_client_buffer(0) {
            assert!(Instant::now() < deadline, "never scanned out again");
            compositor.serve(&listener).unwrap();
        }
        assert_eq!(compositor.pixel(x, y), 0x0000_ff00, "green shown again");
    }
    to_client.send(()).unwrap();
    assert_eq!(next(&mut compositor), Step::Destroyed);
    let mut cleared = None;
    let released = loop {
        assert!(Instant::now() < deadline, "not in time");
        if let Ok(Step::Released(at)) = steps.try_recv() {
            break at;
        }
        compositor.serve(&listener).unwrap();
        if cleared.is_none() && !compositor.shows_client_buffer(0) {
            cleared = Some(Instant::now());
        }
    };
    let cleared = cleared.expect("the display let go of the buffer");
    assert!(
        !direct || released >= cleared,
        "released while the display still showed it"
    );
    to_client.send(()).unwrap();
    while !client.is_finished() {
        compositor.serve(&listener).unwrap();
    }
    client.join().unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
}
