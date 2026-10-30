//! Clients' GPU buffers through real sockets (docs/adr/0031-gpu-buffers.mdx): the compositor's
//! offer after Welcome, DMA-BUFs attached plane by plane and checked, a frame held until its
//! acquire point signals, its buffer released with its release point, a frame superseded while
//! it waits, and what is refused. The DMA-BUFs are udmabuf's and the timelines a render node's:
//! where this machine has neither, the tests that need them say so and skip
//! (`tools/gpu-vm/gpu-vm` runs them).

use oxyos_compositor::{Compositor, Gpu};
use oxyos_compositor_core::{Output, Scene};
use oxyos_dmabuf::{RenderNode, Timeline};
use oxyos_ipc::{Connection, Listener};
use oxyos_protocol::{ErrorCode, Event, Format, Outcome, Rect, Request, Size};
use rustix::fd::AsFd;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;
const STRIDE: u32 = 256;

/// A compositor on a socket of its own, with one 1280×720 output.
fn compositor(name: &str) -> (Compositor, Listener, PathBuf) {
    let directory = std::env::temp_dir().join(format!("oxyos-gpu-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let _ = std::fs::remove_file(&socket);
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

/// Serves until `done`, or fails after 20 s.
fn serve_until(
    compositor: &mut Compositor,
    listener: &Listener,
    mut done: impl FnMut(&mut Compositor) -> bool,
    what: impl Fn(&Compositor) -> String,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done(compositor) {
        assert!(
            Instant::now() < deadline,
            "not in time: {}",
            what(compositor)
        );
        compositor.serve(listener).unwrap();
    }
}

/// A connection past its handshake, and the events that came with Welcome.
fn connect(socket: &Path) -> (Connection, Vec<Event>) {
    let connection = Connection::connect(socket).unwrap();
    connection.set_timeouts(Duration::from_secs(20)).unwrap();
    connection
        .send_request(
            &Request::Hello {
                min_version: 2,
                max_version: 2,
            },
            None,
        )
        .unwrap();
    let (welcome, _) = connection.recv_event().unwrap();
    assert!(matches!(welcome, Event::Welcome { .. }), "{welcome:?}");
    (connection, Vec::new())
}

/// The GPU offer that follows Welcome: the device and its formats.
fn offer(connection: &Connection) -> (u64, Vec<(Format, u64)>) {
    let (event, _) = connection.recv_event().unwrap();
    let Event::GpuDevice { device, formats } = event else {
        panic!("expected GpuDevice after Welcome, got {event:?}");
    };
    let table = (0..formats)
        .map(|index| match connection.recv_event().unwrap().0 {
            Event::GpuFormat {
                index: told,
                format,
                modifier,
            } if told == index => (format, modifier),
            other => panic!("expected format {index}, got {other:?}"),
        })
        .collect();
    (device, table)
}

/// Events until one `want` takes, the others kept.
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

/// A linear DMA-BUF of shared memory filled with one premultiplied colour, rows `STRIDE` apart.
fn filled(colour: u32) -> Option<oxyos_dmabuf::DmaBuf> {
    let (memfd, buffer) = oxyos_dmabuf::shared_dmabuf(u64::from(STRIDE * HEIGHT)).ok()?;
    let row: Vec<u8> = (0..STRIDE / 4).flat_map(|_| colour.to_le_bytes()).collect();
    for y in 0..HEIGHT {
        rustix::io::pwrite(&memfd, &row, u64::from(y * STRIDE)).unwrap();
    }
    Some(buffer)
}

fn attach(connection: &Connection, id: u64, buffer: &oxyos_dmabuf::DmaBuf) {
    connection
        .send_request(
            &Request::AttachDmabufPlane {
                buffer: id,
                plane: 0,
                offset: 0,
                stride: STRIDE,
            },
            Some(buffer.as_fd()),
        )
        .unwrap();
    connection
        .send_request(
            &Request::AttachDmabuf {
                buffer: id,
                format: Format::Argb8888,
                modifier: 0,
                extent: Size::new(WIDTH, HEIGHT),
                planes: 1,
            },
            None,
        )
        .unwrap();
}

/// Whether this machine has what the GPU path needs; says why not.
fn gpu_path() -> bool {
    if RenderNode::with_timelines(None).is_none() {
        eprintln!("no render node with timeline syncobjs here, skipped; tools/gpu-vm runs it");
        return false;
    }
    if oxyos_dmabuf::shared_dmabuf(4096).is_err() {
        eprintln!("no udmabuf here, skipped; tools/gpu-vm runs it");
        return false;
    }
    true
}

#[test]
fn without_a_render_node_a_plane_is_refused_and_no_gpu_is_offered() {
    let (mut compositor, listener, socket) = compositor("refused");
    if compositor.offers_gpu_buffers() {
        unreachable!("nothing offered before offer_gpu_buffers");
    }
    let client = std::thread::spawn(move || {
        let (connection, _) = connect(&socket);
        // Not a DMA-BUF (a memfd) and, with no offer, no plane at all: the connection fails.
        let memfd = rustix::fs::memfd_create("plane", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
        rustix::fs::ftruncate(&memfd, 4096).unwrap();
        connection
            .send_request(
                &Request::AttachDmabufPlane {
                    buffer: 1,
                    plane: 0,
                    offset: 0,
                    stride: 256,
                },
                Some(memfd.as_fd()),
            )
            .unwrap();
        let (event, _) = connection.recv_event().unwrap();
        event
    });
    serve_until(
        &mut compositor,
        &listener,
        |_| client.is_finished(),
        |_| "the client finishing".into(),
    );
    assert_eq!(
        client.join().unwrap(),
        Event::ProtocolError {
            code: ErrorCode::InvalidBuffer
        }
    );
}

/// What the client thread tells the test, and the test it.
#[derive(Debug)]
enum Step {
    Submitted,
    Signalled,
    Shown,
    Done,
}

fn frames_wait_for_their_acquire_points(gpu: Option<oxyos_gpu::VulkanCompositor>) {
    let checks = gpu.is_some();
    let (mut compositor, listener, socket) =
        compositor(if checks { "sync-gpu" } else { "sync-cpu" });
    if let Some(vulkan) = gpu {
        compositor.gpu = Some(Gpu {
            compositor: vulkan,
            frames: 0,
            mismatches: 0,
            device_ms: Vec::new(),
            readback_ms: Vec::new(),
            host_ms: Vec::new(),
            uploaded: Vec::new(),
            phases_ms: Vec::new(),
        });
    }
    compositor.offer_gpu_buffers();
    assert!(compositor.offers_gpu_buffers());
    let (to_test, steps) = mpsc::channel();
    let (to_client, go) = mpsc::channel::<()>();
    let client = std::thread::spawn(move || {
        let (connection, _) = connect(&socket);
        let (_, table) = offer(&connection);
        assert!(table.contains(&(Format::Argb8888, 0)), "{table:?}");
        let mut seen = Vec::new();
        connection
            .send_request(
                &Request::CreateToplevel {
                    surface: 1,
                    size: Size::new(WIDTH, HEIGHT),
                },
                None,
            )
            .unwrap();
        let Event::Configure { serial, .. } = until(&connection, &mut seen, |event| {
            matches!(event, Event::Configure { surface: 1, .. })
        }) else {
            unreachable!()
        };
        let red = filled(0xffff_0000).unwrap();
        let green = filled(0xff00_ff00).unwrap();
        let blue = filled(0xff00_00ff).unwrap();
        attach(&connection, 1, &red);
        attach(&connection, 2, &green);
        attach(&connection, 3, &blue);
        let node = Arc::new(RenderNode::with_timelines(None).unwrap());
        let timeline = Timeline::create(&node).unwrap();
        connection
            .send_request(
                &Request::ImportTimeline { timeline: 1 },
                Some(timeline.export().unwrap().as_fd()),
            )
            .unwrap();
        let submit = |sequence: u64, buffer: u64, sync: Option<(u64, u64)>| {
            if let Some((acquire, release)) = sync {
                connection
                    .send_request(
                        &Request::SetSync {
                            surface: 1,
                            acquire_timeline: 1,
                            acquire_point: acquire,
                            release_timeline: 1,
                            release_point: release,
                        },
                        None,
                    )
                    .unwrap();
            }
            connection
                .send_request(
                    &Request::Submit {
                        surface: 1,
                        sequence,
                        buffer,
                        configure: serial,
                        extent: Size::new(WIDTH, HEIGHT),
                    },
                    None,
                )
                .unwrap();
        };
        // Red, once point 1 signals; released at point 2.
        submit(1, 1, Some((1, 2)));
        to_test.send(Step::Submitted).unwrap();
        go.recv().unwrap();
        timeline.signal(1).unwrap();
        to_test.send(Step::Signalled).unwrap();
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
        to_test.send(Step::Shown).unwrap();
        go.recv().unwrap();
        // Green, ready now: red is released, and its release point with it.
        timeline.signal(3).unwrap();
        submit(2, 2, Some((3, 4)));
        until(&connection, &mut seen, |event| {
            matches!(event, Event::Released { buffer: 1, .. })
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while timeline.signalled().unwrap() < 2 {
            assert!(
                Instant::now() < deadline,
                "red's release point never signalled"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        // Red again, waiting for point 5, superseded by blue (ready): red is discarded and
        // released, its release point signalled only once its acquire point has.
        submit(3, 1, Some((5, 6)));
        submit(4, 3, None);
        until(&connection, &mut seen, |event| {
            matches!(
                event,
                Event::Feedback {
                    sequence: 3,
                    outcome: Outcome::Discarded,
                    ..
                }
            )
        });
        until(&connection, &mut seen, |event| {
            matches!(event, Event::Released { buffer: 1, .. })
        });
        assert!(
            timeline.signalled().unwrap() < 6,
            "released before its acquire point"
        );
        timeline.signal(5).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while timeline.signalled().unwrap() < 6 {
            assert!(
                Instant::now() < deadline,
                "the superseded release point never signalled"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        until(&connection, &mut seen, |event| {
            matches!(
                event,
                Event::Feedback {
                    sequence: 4,
                    outcome: Outcome::Presented,
                    ..
                }
            )
        });
        to_test.send(Step::Done).unwrap();
        // Kept until the test has looked at the frame.
        go.recv().ok();
    });
    let mut step = None;
    let mut next = |compositor: &mut Compositor, want: &str| {
        serve_until(
            compositor,
            &listener,
            |_| {
                if step.is_none() {
                    step = steps.try_recv().ok();
                }
                step.as_ref().is_some_and(|got| format!("{got:?}") == want)
            },
            |_| format!("the client's step {want}"),
        );
        step = None;
    };
    let centre = |compositor: &Compositor| {
        let rect = compositor
            .scene
            .frame_list()
            .into_iter()
            .find_map(|item| match item {
                oxyos_compositor_core::RenderItem::Surface { rect, .. } => Some(rect),
                _ => None,
            });
        rect.map(|rect| {
            compositor.pixel(
                rect.x + rect.width as i32 / 2,
                rect.y + rect.height as i32 / 2,
            )
        })
    };
    next(&mut compositor, "Submitted");
    // Turns pass; the frame waits for its point and nothing of it is shown.
    for _ in 0..5 {
        compositor.serve(&listener).unwrap();
    }
    assert_eq!(centre(&compositor), None, "shown before its acquire point");
    to_client.send(()).unwrap();
    next(&mut compositor, "Signalled");
    next(&mut compositor, "Shown");
    serve_until(
        &mut compositor,
        &listener,
        |c| centre(c) == Some(0x00ff_0000),
        |c| format!("red, the centre is {:x?}", centre(c)),
    );
    to_client.send(()).unwrap();
    next(&mut compositor, "Done");
    serve_until(
        &mut compositor,
        &listener,
        |c| centre(c) == Some(0x0000_00ff),
        |c| format!("blue, the centre is {:x?}", centre(c)),
    );
    if checks {
        let gpu = compositor.gpu.as_ref().unwrap();
        assert!(gpu.frames > 0);
        assert_eq!(gpu.mismatches, 0, "the GPU's frames differ from the CPU's");
    }
    to_client.send(()).unwrap();
    serve_until(
        &mut compositor,
        &listener,
        |_| client.is_finished(),
        |_| "the client finishing".into(),
    );
    client.join().unwrap();
}

#[test]
fn a_frame_waits_for_its_acquire_point_composed_on_the_cpu() {
    if gpu_path() {
        frames_wait_for_their_acquire_points(None);
    }
}

#[test]
fn a_frame_waits_for_its_acquire_point_composed_on_the_gpu_too() {
    if !gpu_path() {
        return;
    }
    match oxyos_gpu::VulkanCompositor::new() {
        Ok(gpu) if !gpu.dma_buf_formats().is_empty() => {
            frames_wait_for_their_acquire_points(Some(gpu))
        }
        Ok(gpu) => eprintln!("{} imports no DMA-BUF, skipped", gpu.device_name()),
        Err(error) => eprintln!("no Vulkan device ({error}), skipped"),
    }
}

/// What a refused client sends, given a DMA-BUF it may use.
type Sends = fn(&Connection, &oxyos_dmabuf::DmaBuf);

#[test]
fn what_the_compositor_does_not_take_fails_the_connection() {
    if !gpu_path() {
        return;
    }
    let cases: Vec<(&str, Sends, ErrorCode)> = vec![
        (
            "a memfd as a plane",
            |connection, _| {
                let memfd =
                    rustix::fs::memfd_create("plane", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
                rustix::fs::ftruncate(&memfd, 65536).unwrap();
                connection
                    .send_request(
                        &Request::AttachDmabufPlane {
                            buffer: 1,
                            plane: 0,
                            offset: 0,
                            stride: STRIDE,
                        },
                        Some(memfd.as_fd()),
                    )
                    .unwrap();
            },
            ErrorCode::InvalidBuffer,
        ),
        (
            "a modifier not offered",
            |connection, buffer| {
                connection
                    .send_request(
                        &Request::AttachDmabufPlane {
                            buffer: 1,
                            plane: 0,
                            offset: 0,
                            stride: STRIDE,
                        },
                        Some(buffer.as_fd()),
                    )
                    .unwrap();
                connection
                    .send_request(
                        &Request::AttachDmabuf {
                            buffer: 1,
                            format: Format::Argb8888,
                            modifier: 0x0100_0000_0000_0001,
                            extent: Size::new(WIDTH, HEIGHT),
                            planes: 1,
                        },
                        None,
                    )
                    .unwrap();
            },
            ErrorCode::InvalidBuffer,
        ),
        (
            "a plane beyond its DMA-BUF",
            |connection, buffer| {
                connection
                    .send_request(
                        &Request::AttachDmabufPlane {
                            buffer: 1,
                            plane: 0,
                            offset: 4096,
                            stride: STRIDE,
                        },
                        Some(buffer.as_fd()),
                    )
                    .unwrap();
                connection
                    .send_request(
                        &Request::AttachDmabuf {
                            buffer: 1,
                            format: Format::Argb8888,
                            modifier: 0,
                            extent: Size::new(WIDTH, HEIGHT),
                            planes: 1,
                        },
                        None,
                    )
                    .unwrap();
            },
            ErrorCode::InvalidBuffer,
        ),
        (
            "planes missing",
            |connection, _| {
                connection
                    .send_request(
                        &Request::AttachDmabuf {
                            buffer: 1,
                            format: Format::Argb8888,
                            modifier: 0,
                            extent: Size::new(WIDTH, HEIGHT),
                            planes: 1,
                        },
                        None,
                    )
                    .unwrap();
            },
            ErrorCode::InvalidBuffer,
        ),
        (
            "a release point not after its acquire point",
            |connection, _| {
                let node = Arc::new(RenderNode::with_timelines(None).unwrap());
                let timeline = Timeline::create(&node).unwrap();
                connection
                    .send_request(
                        &Request::ImportTimeline { timeline: 1 },
                        Some(timeline.export().unwrap().as_fd()),
                    )
                    .unwrap();
                connection
                    .send_request(
                        &Request::SetSync {
                            surface: 1,
                            acquire_timeline: 1,
                            acquire_point: 4,
                            release_timeline: 1,
                            release_point: 4,
                        },
                        None,
                    )
                    .unwrap();
            },
            ErrorCode::InvalidMessage,
        ),
        (
            "a timeline that is not a syncobj",
            |connection, buffer| {
                connection
                    .send_request(
                        &Request::ImportTimeline { timeline: 1 },
                        Some(buffer.as_fd()),
                    )
                    .unwrap();
            },
            ErrorCode::InvalidMessage,
        ),
    ];
    for (what, send, code) in cases {
        let (mut compositor, listener, socket) = compositor("refusals");
        compositor.offer_gpu_buffers();
        let client = std::thread::spawn(move || {
            let (connection, _) = connect(&socket);
            offer(&connection);
            let buffer = filled(0xffff_ffff).unwrap();
            send(&connection, &buffer);
            connection.recv_event().unwrap().0
        });
        serve_until(
            &mut compositor,
            &listener,
            |_| client.is_finished(),
            |_| "the client finishing".into(),
        );
        assert_eq!(
            client.join().unwrap(),
            Event::ProtocolError { code },
            "{what}"
        );
    }
}
