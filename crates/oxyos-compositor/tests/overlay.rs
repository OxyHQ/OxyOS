//! A maximized window on a display's overlay plane (docs/adr/0031-gpu-buffers.mdx, decision 15),
//! on a real KMS driver, through real sockets: the window's buffer goes on the plane, the shell's
//! panel beside it composed on the primary plane; a frame of the window alone composes no pixel;
//! a menu of its own over it has it composed again, all of it, and once the menu goes the window
//! is on the plane again. At each
//! step what the display shows (the primary plane's buffer, read back, with the window's buffer
//! where the plane is) is the CPU reference's frame, pixel for pixel.
//!
//! Needs a KMS card with an overlay plane this process can be DRM master of, and a Vulkan device
//! exporting DMA-BUF: `tools/gpu-vm/gpu-vm --vkms` has them (vkms, composing on lavapipe);
//! elsewhere it says so and skips.

use oxyos_compositor::{Compositor, Gpu};
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::{Connection, Listener};
use oxyos_protocol::{
    Adjust, Anchor, Event, Format, Gravity, Outcome, PanelEdge, Positioner, Rect, Request,
    ShellKind, Size,
};
use rustix::fd::AsFd;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The panel's height, at the bottom: the maximized window is the output above it.
const PANEL: u32 = 40;
const RED: u32 = 0xffff_0000;
const GREEN: u32 = 0xff00_ff00;
/// The colour of the window's menu over it.
const BLUE: u32 = 0xff00_00ff;

/// A display of a card that has an overlay plane and can be ours.
fn display() -> Option<oxyos_kms::Display> {
    let cards = oxyos_gpu_device::display_cards(Path::new(oxyos_gpu_device::SYSTEM_ROOT));
    cards.into_iter().find_map(|path| {
        let card = oxyos_kms::Card::open(&path).ok()?;
        let display = oxyos_kms::Display::open(card).ok()?;
        (!display.overlay_formats().is_empty()).then_some(display)
    })
}

/// A buffer of the card's memory in one colour, as a DMA-BUF, and its row pitch.
fn filled(
    card: &oxyos_kms::Card,
    width: u32,
    height: u32,
    colour: u32,
) -> (std::os::fd::OwnedFd, u32) {
    use drm::buffer::Buffer as _;
    use drm::control::Device as _;
    let row: Vec<u8> = (0..width).flat_map(|_| colour.to_le_bytes()).collect();
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

/// Connects and says hello.
fn connected(path: &Path) -> (Connection, Vec<Event>) {
    let connection = Connection::connect(path).unwrap();
    connection.set_timeouts(Duration::from_secs(30)).unwrap();
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
    (connection, seen)
}

/// A surface's configure: its serial and rectangle.
fn configured(connection: &Connection, seen: &mut Vec<Event>, surface: u32) -> (u64, Rect) {
    match until(
        connection,
        seen,
        |event| matches!(event, Event::Configure { surface: s, .. } if *s == surface),
    ) {
        Event::Configure { serial, rect, .. } => (serial, rect),
        _ => unreachable!(),
    }
}

/// Attaches DMA-BUF `buffer` (linear, in `format`) of `size`.
fn attach(
    connection: &Connection,
    id: u64,
    (fd, stride): &(std::os::fd::OwnedFd, u32),
    size: Size,
    format: Format,
) {
    connection
        .send_request(
            &Request::AttachDmabufPlane {
                buffer: id,
                plane: 0,
                offset: 0,
                stride: *stride,
            },
            Some(fd.as_fd()),
        )
        .unwrap();
    connection
        .send_request(
            &Request::AttachDmabuf {
                buffer: id,
                format,
                modifier: 0,
                extent: size,
                planes: 1,
            },
            None,
        )
        .unwrap();
}

/// Submits `buffer` as frame `sequence` of `surface` and waits until it is presented.
fn shown(
    connection: &Connection,
    seen: &mut Vec<Event>,
    (surface, serial, size): (u32, u64, Size),
    sequence: u64,
    buffer: u64,
) {
    connection
        .send_request(
            &Request::Submit {
                surface,
                sequence,
                buffer,
                configure: serial,
                extent: size,
            },
            None,
        )
        .unwrap();
    until(
        connection,
        seen,
        |event| matches!(event, Event::Feedback { sequence: s, outcome: Outcome::Presented, .. } if *s == sequence),
    );
}

/// What the client did, for the test to look at.
#[derive(Debug)]
enum Step {
    /// Its window shows `colour`, at that place.
    Shows(u32, Rect),
    /// A menu of the window's shows over it.
    Covered,
    /// The menu is gone.
    Uncovered,
}

/// What the display shows: the primary plane's buffer with `window`'s colour where the overlay
/// plane has it, if it does; and whether it is the CPU reference's frame (which the compositor
/// composes too, `gpu_only` off).
fn checked(compositor: &mut Compositor, window: Option<(Rect, u32)>, what: &str) {
    let screen = &compositor.screens[0];
    let (width, height) = (screen.width, screen.height);
    let reference = screen.framebuffer.clone();
    let front = screen
        .front_target()
        .expect("a frame composed for the display");
    let mut shown = compositor
        .gpu
        .as_mut()
        .unwrap()
        .compositor
        .read_scanout(front)
        .unwrap();
    if let Some((rect, colour)) = window {
        for y in rect.y..rect.y + rect.height as i32 {
            for x in rect.x..rect.x + rect.width as i32 {
                shown[(y as u32 * width + x as u32) as usize] = colour & 0x00ff_ffff;
            }
        }
    }
    assert_eq!(reference.len(), (width * height) as usize);
    let differing = (0..shown.len())
        .filter(|&at| shown[at] != reference[at])
        .count();
    if let Some(at) = (0..shown.len()).find(|&at| shown[at] != reference[at]) {
        panic!(
            "{what}: {differing} pixels shown are not the reference's, the first at ({}, {}): {:#08x}, not {:#08x}",
            at as u32 % width,
            at as u32 / width,
            shown[at],
            reference[at]
        );
    }
}

#[test]
fn a_maximized_window_is_on_the_overlay_plane_and_its_frames_compose_nothing() {
    let Some(display) = display() else {
        eprintln!("no display with an overlay plane this process can drive here, skipped; tools/gpu-vm/gpu-vm --vkms runs it");
        return;
    };
    let mut vulkan = match oxyos_gpu::VulkanCompositor::on_device(0) {
        Ok(vulkan) if vulkan.exports_dma_buf() => vulkan,
        _ => {
            eprintln!("no Vulkan device exporting DMA-BUF here, skipped");
            return;
        }
    };
    let (width, height) = display.size();
    let window = Size::new(width, height - PANEL);
    let buffers =
        [RED, GREEN].map(|colour| filled(display.card(), window.width, window.height, colour));
    let covering = filled(display.card(), 300, 200, BLUE);
    let mut scanout = Vec::new();
    for (index, target) in vulkan.add_scanout_targets(2, width, height).unwrap() {
        let framebuffer = display
            .card()
            .import_dmabuf(target.fd.as_fd(), target.width, target.height, target.pitch)
            .unwrap();
        scanout.push((index, framebuffer));
    }

    let directory = std::env::temp_dir().join(format!("oxyos-overlay-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let shell_socket = directory.join("shell");
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
    compositor.shell_listener = Some(Listener::bind(&shell_socket).unwrap());
    compositor.screens[0].display = Some(display);
    compositor.screens[0].scanout = scanout;
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
    // The CPU reference composes every frame too.
    compositor.gpu_only = false;
    compositor.offer_gpu_buffers();

    // The shell: a translucent panel along the bottom.
    let shell_path = shell_socket.clone();
    std::thread::spawn(move || {
        let mut shell = oxyos_client::Client::connect(&shell_path).unwrap();
        shell
            .send(&Request::CreateShellSurface {
                surface: 1,
                kind: ShellKind::Panel,
                edge: PanelEdge::Bottom,
                output: 1,
                size: Size::new(width, PANEL),
                exclusive: PANEL,
            })
            .unwrap();
        loop {
            match shell.next_event() {
                Ok(Some(Event::Configure {
                    surface: 1, serial, ..
                })) => {
                    let pixels = vec![0x80; (width * PANEL * 4) as usize];
                    shell
                        .present(1, serial, Size::new(width, PANEL), &pixels)
                        .unwrap();
                }
                Ok(_) => {}
                Err(_) => return,
            }
        }
    });

    let (to_test, steps) = mpsc::channel();
    let (to_client, go) = mpsc::channel::<()>();
    let path = socket.clone();
    std::thread::spawn(move || {
        // Once the panel shows: the window's maximized place is above it.
        go.recv().unwrap();
        let (connection, mut seen) = connected(&path);
        for (surface, size) in [(1, Size::new(800, 500)), (2, Size::new(300, 200))] {
            if surface == 2 {
                go.recv().unwrap();
            }
            let created = if surface == 1 {
                Request::CreateToplevel { surface, size }
            } else {
                // A menu of the window's: it keeps the focus, and looks the same, under it. (An
                // opaque square one would go on the plane itself, the window composed.)
                Request::CreatePopup {
                    surface,
                    parent: 1,
                    positioner: Positioner {
                        anchor_rect: Rect::new(100, 100, 10, 10),
                        size,
                        anchor: Anchor::BottomLeft,
                        gravity: Gravity::BottomRight,
                        adjust: Adjust::NONE,
                        offset_x: 0,
                        offset_y: 0,
                    },
                }
            };
            connection.send_request(&created, None).unwrap();
            if surface == 1 {
                connection
                    .send_request(
                        &Request::SetMaximized {
                            surface,
                            enabled: true,
                        },
                        None,
                    )
                    .unwrap();
                let (serial, rect) = loop {
                    let (serial, rect) = configured(&connection, &mut seen, 1);
                    if (rect.width, rect.height) == (window.width, window.height) {
                        break (serial, rect);
                    }
                };
                for (id, buffer) in (1..).zip(&buffers) {
                    attach(&connection, id, buffer, window, Format::Xrgb8888);
                }
                shown(&connection, &mut seen, (1, serial, window), 1, 1);
                to_test.send(Step::Shows(RED, rect)).unwrap();
                go.recv().unwrap();
                shown(&connection, &mut seen, (1, serial, window), 2, 2);
                to_test.send(Step::Shows(GREEN, rect)).unwrap();
            } else {
                let (serial, _) = configured(&connection, &mut seen, 2);
                // Translucent (premultiplied, opaque pixels): no plane shows it as it is.
                attach(&connection, 3, &covering, size, Format::Argb8888);
                shown(&connection, &mut seen, (2, serial, size), 1, 3);
                to_test.send(Step::Covered).unwrap();
                go.recv().unwrap();
                connection
                    .send_request(&Request::Destroy { surface: 2 }, None)
                    .unwrap();
                to_test.send(Step::Uncovered).unwrap();
            }
        }
        go.recv().ok();
    });

    let deadline = Instant::now() + Duration::from_secs(60);
    let serve_until =
        |compositor: &mut Compositor, what: &str, done: &dyn Fn(&Compositor) -> bool| {
            while !done(compositor) {
                assert!(Instant::now() < deadline, "{what}: not in time");
                compositor.serve(&listener).unwrap();
            }
        };
    let next = |compositor: &mut Compositor| loop {
        assert!(
            Instant::now() < deadline,
            "the client did not go on in time"
        );
        if let Ok(step) = steps.try_recv() {
            return step;
        }
        compositor.serve(&listener).unwrap();
    };
    let on_plane = |compositor: &Compositor| {
        compositor.screens[0]
            .display
            .as_ref()
            .is_some_and(|display| display.shows_overlay())
            && compositor.screens[0].pending_flip.is_none()
    };

    serve_until(&mut compositor, "the panel shown", &|c| {
        !c.scene.render_list().is_empty() && c.screens[0].pending_flip.is_none()
    });
    to_client.send(()).unwrap();
    // Red, maximized, its arrival over: on the overlay plane, the panel composed beside it.
    let Step::Shows(RED, rect) = next(&mut compositor) else {
        panic!("expected the window shown");
    };
    serve_until(
        &mut compositor,
        "the window on the overlay plane",
        &on_plane,
    );
    assert!(compositor.overlay_frames > 0);
    checked(&mut compositor, Some((rect, RED)), "red on the plane");

    // Green: a frame of the window alone composes no pixel.
    let (pixels, overlaid) = (compositor.display_pixels, compositor.overlay_frames);
    to_client.send(()).unwrap();
    let Step::Shows(GREEN, _) = next(&mut compositor) else {
        panic!("expected green shown");
    };
    serve_until(&mut compositor, "green's flip", &|c| {
        c.screens[0].pending_flip.is_none()
    });
    assert_eq!(
        compositor.display_pixels - pixels,
        0,
        "pixels composed for a frame of the window on the plane"
    );
    assert!(compositor.overlay_frames > overlaid);
    checked(&mut compositor, Some((rect, GREEN)), "green on the plane");

    // A menu of its own over it (the window keeping the focus, looking the same): the window is
    // composed again, all of its place, though nothing of it changed.
    to_client.send(()).unwrap();
    let Step::Covered = next(&mut compositor) else {
        panic!("expected the menu shown");
    };
    serve_until(&mut compositor, "the plane off", &|c| {
        !c.screens[0].display.as_ref().unwrap().shows_overlay()
            && c.screens[0].pending_flip.is_none()
    });
    serve_until(&mut compositor, "the menu settled", &|c| {
        c.transitions.is_empty()
    });
    serve_until(&mut compositor, "its flip", &|c| {
        c.screens[0].pending_flip.is_none()
    });
    checked(&mut compositor, None, "a menu over it, composed");

    // Gone: the window on the plane again, what the menu covered composed.
    to_client.send(()).unwrap();
    let Step::Uncovered = next(&mut compositor) else {
        panic!("expected the menu gone");
    };
    serve_until(&mut compositor, "the window on the plane again", &|c| {
        c.transitions.is_empty() && on_plane(c)
    });
    checked(&mut compositor, Some((rect, GREEN)), "on the plane again");
    to_client.send(()).ok();
    let _ = std::fs::remove_dir_all(directory);
}
