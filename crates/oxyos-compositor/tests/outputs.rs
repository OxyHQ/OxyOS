//! Several outputs (docs/adr/0018-several-displays.mdx), through real sockets: each screen
//! composes its own part of the global space at its own scale, a window's new frame composes
//! only the screen it is on, the splash is on every screen, and a window on an output removed
//! moves to the primary and is drawn there at the primary's scale.

use oxyos_client::Client;
use oxyos_compositor::{Compositor, DESKTOP};
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{Event, Rect, Request, Size};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The primary, and right of it an output of 400x300 logical pixels at 1.5 (600x450 pixels).
const LEFT: Rect = Rect::new(0, 0, 640, 480);
const RIGHT: Rect = Rect::new(640, 0, 400, 300);

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Session {
    compositor: Compositor,
    listener: Listener,
    socket: std::path::PathBuf,
    _directory: TempDir,
}

fn session(name: &str) -> Session {
    let directory =
        std::env::temp_dir().join(format!("oxyos-outputs-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    for (id, rect, scale_120) in [(1, LEFT, 120), (2, RIGHT, 180)] {
        scene
            .add_output(Output {
                id,
                rect,
                scale_120,
                refresh_mhz: 60_000,
            })
            .unwrap();
    }
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    // On the test's clock: what arrives is shown whole once it is moved on (`until`).
    compositor.clock = Some(Duration::from_secs(100));
    Session {
        compositor,
        listener,
        socket,
        _directory: TempDir(directory),
    }
}

/// An application whose 100x60 window is filled with `colour` (0xRRGGBB) at the scale each
/// configure asks for, and again with the next colour `next` sends.
fn application(socket: std::path::PathBuf, colour: u32, next: mpsc::Receiver<u32>) {
    std::thread::spawn(move || {
        let mut client = Client::connect(&socket).unwrap();
        client
            .send(&Request::CreateToplevel {
                surface: 1,
                size: Size::new(100, 60),
            })
            .unwrap();
        let mut colour = colour;
        // The latest configure, and whether a frame for it is to be drawn.
        let mut configure = None;
        let mut draw = false;
        loop {
            match client.poll().unwrap() {
                oxyos_client::Poll::Event(Event::Configure {
                    serial,
                    rect,
                    scale_120,
                    ..
                }) => {
                    let extent = |length: u32| (length * scale_120).div_ceil(120);
                    configure = Some((serial, Size::new(extent(rect.width), extent(rect.height))));
                    draw = true;
                }
                oxyos_client::Poll::Event(_) => continue,
                oxyos_client::Poll::Closed => return,
                oxyos_client::Poll::Empty => match next.try_recv() {
                    Ok(next) => {
                        colour = next;
                        draw = true;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => return,
                    Err(mpsc::TryRecvError::Empty) => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                },
            }
            if let (Some((serial, extent)), true) = (configure, std::mem::take(&mut draw)) {
                let pixels: Vec<u8> = std::iter::repeat_n(colour.to_le_bytes(), {
                    (extent.width * extent.height) as usize
                })
                .flatten()
                .collect();
                client.present(1, serial, extent, &pixels).unwrap();
            }
        }
    });
}

impl Session {
    /// Serves until `done` holds, then until what arrived has finished arriving (its
    /// transition, a second on the test's clock).
    fn until(&mut self, what: &str, done: impl Fn(&Compositor) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(&self.compositor) {
            assert!(Instant::now() < deadline, "{what} not in time");
            self.compositor.serve(&self.listener).unwrap();
        }
        let clock = self.compositor.clock.expect("the test's clock");
        self.compositor.clock = Some(clock + Duration::from_secs(1));
        while !self.compositor.transitions.is_empty() {
            assert!(Instant::now() < deadline, "{what}'s transition not in time");
            self.compositor.serve(&self.listener).unwrap();
        }
        self.compositor.serve(&self.listener).unwrap();
    }
}

#[test]
fn each_screen_composes_its_part_at_its_own_scale() {
    let mut s = session("scale");
    assert_eq!(
        s.compositor
            .screens
            .iter()
            .map(|screen| (screen.width, screen.height))
            .collect::<Vec<_>>(),
        [(640, 480), (600, 450)]
    );
    // A window opened with the pointer on the right output is placed there.
    s.compositor.scene.pointer_motion(800, 100);
    let (_next, frames) = mpsc::channel();
    application(s.socket.clone(), 0x00c0_4020, frames);
    s.until("the window", |c| c.surface_rect(1, 1).is_some());
    let window = s.compositor.surface_rect(1, 1).unwrap();
    assert!(RIGHT.contains(window), "{window:?}");
    // On the right screen, at its scale: the window's 100x60 logical pixels are 150x90, each
    // edge's middle its own (its corners are rounded), the next pixel past it not (its shadow,
    // or the desktop).
    let right = &s.compositor.screens[1];
    let (x, y) = (
        ((window.x - RIGHT.x) * 3 / 2) as u32,
        ((window.y - RIGHT.y) * 3 / 2) as u32,
    );
    for (px, py) in [
        (x, y + 45),
        (x + 149, y + 45),
        (x + 75, y),
        (x + 75, y + 89),
    ] {
        assert_eq!(right.pixel(px, py), 0x00c0_4020, "({px}, {py})");
    }
    for (px, py) in [
        (x - 1, y + 45),
        (x + 150, y + 45),
        (x + 75, y - 1),
        (x + 75, y + 90),
    ] {
        assert_ne!(right.pixel(px, py), 0x00c0_4020, "({px}, {py})");
    }
    // Nothing of it on the left screen; the global space's pixel is the right screen's.
    assert!(s.compositor.screens[0]
        .framebuffer
        .iter()
        .all(|&pixel| pixel == DESKTOP));
    assert_eq!(
        s.compositor.pixel(window.x + 50, window.y + 30),
        0x00c0_4020
    );
}

#[test]
fn a_new_frame_composes_only_the_screen_it_is_on() {
    let mut s = session("damage");
    s.compositor.scene.pointer_motion(800, 100);
    let (next, frames) = mpsc::channel();
    application(s.socket.clone(), 0x0010_8040, frames);
    s.until("the window", |c| c.surface_rect(1, 1).is_some());
    let window = s.compositor.surface_rect(1, 1).unwrap();
    let left_damage = s.compositor.screens[0].last_damage.clone();
    let composed = s.compositor.frames;
    next.send(0x0040_1080).unwrap();
    s.until("the new frame", |c| {
        c.pixel(window.x + 50, window.y + 30) == 0x0040_1080
    });
    assert!(s.compositor.frames > composed);
    // The right screen composed the window's damage alone; the left composed nothing.
    let right = s.compositor.screens[1]
        .last_damage
        .clone()
        .expect("damaged");
    assert_eq!(
        right,
        [oxyos_gpu::Rect {
            x: (window.x - RIGHT.x) * 3 / 2,
            y: (window.y - RIGHT.y) * 3 / 2,
            width: 150,
            height: 90,
        }]
    );
    assert_eq!(s.compositor.screens[0].last_damage, left_damage);
}

#[test]
fn the_splash_is_centred_on_every_screen() {
    let mut s = session("splash");
    s.compositor.show_splash();
    s.compositor.serve(&s.listener).unwrap();
    for (screen, (centre_x, centre_y)) in [(0, (320, 240)), (1, (300, 225))] {
        let screen = &s.compositor.screens[screen];
        assert_eq!(screen.pixel(centre_x, centre_y), 0x00ff_ffff);
        assert_eq!(screen.pixel(5, 5), 0);
        assert_eq!(screen.pixel(screen.width - 5, screen.height - 5), 0);
    }
}

#[test]
fn a_window_on_an_output_removed_moves_to_the_primary_at_its_scale() {
    let mut s = session("removed");
    s.compositor.scene.pointer_motion(800, 100);
    let (_next, frames) = mpsc::channel();
    application(s.socket.clone(), 0x0020_a0f0, frames);
    s.until("the window", |c| c.surface_rect(1, 1).is_some());
    s.compositor.remove_output(2);
    assert_eq!(s.compositor.screens.len(), 1);
    assert_eq!(s.compositor.scene.outputs().len(), 1);
    // Asked for a frame at the primary's scale, it is drawn there, whole.
    s.until("the window on the primary", |c| {
        c.surface_rect(1, 1).is_some_and(|rect| LEFT.contains(rect))
            && c.screens[0].pixel(320, 240) == 0x0020_a0f0
    });
    let window = s.compositor.surface_rect(1, 1).unwrap();
    assert_eq!((window.width, window.height), (100, 60));
    let left = &s.compositor.screens[0];
    let (x, y) = (window.x as u32, window.y as u32);
    for (px, py) in [(x, y + 30), (x + 99, y + 30), (x + 50, y), (x + 50, y + 59)] {
        assert_eq!(left.pixel(px, py), 0x0020_a0f0, "({px}, {py})");
    }
    assert_ne!(left.pixel(x + 100, y + 30), 0x0020_a0f0);
    assert_ne!(left.pixel(x + 50, y + 60), 0x0020_a0f0);
}
