//! Surfaces arriving and going, animated (docs/adr/0011-transitions.mdx), through real sockets
//! on the compositor's clock set by the test: what each composed frame shows of a window as it
//! arrives and goes, and that locking shows nothing of it going; and a window's frame zooming
//! (docs/adr/0016-windows.mdx).

use oxyos_client::Client;
use oxyos_compositor::{Compositor, DESKTOP};
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{Event, Rect, Request, Size};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const WHITE: u32 = 0x00ff_ffff;

/// White over the desktop at `opacity`, as composition blends it.
fn white_at(opacity: u8) -> u32 {
    let white = [0xffu8; 4];
    let layer = oxyos_gpu::Layer {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        extent: (1, 1),
        paint: oxyos_gpu::Paint::Opaque(&white),
        source: None,
        changed: None,
        opacity,
        radius: 0,
        filter: oxyos_gpu::Filter::Nearest,
        clip: None,
    };
    oxyos_gpu::compose_cpu(1, 1, DESKTOP, &[layer])[0]
}

struct Session {
    compositor: Compositor,
    listener: Listener,
    /// Tells the application to destroy its window.
    destroy: mpsc::Sender<()>,
    _directory: TempDir,
}

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A compositor on its test clock with one application whose white window is shown; the
/// window's centre on the output.
fn session(name: &str) -> (Session, (i32, i32)) {
    let directory =
        std::env::temp_dir().join(format!("oxyos-transitions-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let output = Rect::new(0, 0, 640, 480);
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: output,
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    compositor.clock = Some(Duration::from_secs(100));
    let (destroy, destroyed) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut client = Client::connect(&socket).unwrap();
        client
            .send(&Request::CreateToplevel {
                surface: 1,
                size: Size::new(200, 100),
            })
            .unwrap();
        loop {
            if let Some(Event::Configure { serial, rect, .. }) = client.next_event().unwrap() {
                let pixels = vec![0xff; (rect.width * rect.height * 4) as usize];
                client
                    .present(1, serial, Size::new(rect.width, rect.height), &pixels)
                    .unwrap();
                break;
            }
        }
        if destroyed.recv().is_ok() {
            client.send(&Request::Destroy { surface: 1 }).unwrap();
        }
        // Stays connected until the test ends.
        let _ = destroyed.recv();
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let rect = loop {
        assert!(
            Instant::now() < deadline,
            "the window was not shown in time"
        );
        compositor.serve(&listener).unwrap();
        if let Some(rect) = compositor.surface_rect(1, 1) {
            break rect;
        }
    };
    let centre = (
        rect.x + rect.width as i32 / 2,
        rect.y + rect.height as i32 / 2,
    );
    (
        Session {
            compositor,
            listener,
            destroy,
            _directory: TempDir(directory),
        },
        centre,
    )
}

impl Session {
    /// The output's pixel at `at` in the frame composed at `ms` on the test's clock.
    fn at(&mut self, ms: u64, at: (i32, i32)) -> u32 {
        self.compositor.clock = Some(Duration::from_secs(100) + Duration::from_millis(ms));
        self.compositor.serve(&self.listener).unwrap();
        self.compositor.pixel(at.0, at.1)
    }
}

#[test]
fn a_window_fades_in_as_it_arrives_and_out_from_its_last_frame_as_it_goes() {
    let (mut s, centre) = session("window");
    // Its first frame, composed at the clock's start: not seen yet.
    assert_eq!(s.compositor.pixel(centre.0, centre.1), DESKTOP);
    // Halfway in time, 87.5 % of the way; then whole.
    assert_eq!(s.at(90, centre), white_at(223));
    assert_eq!(s.at(180, centre), WHITE);
    assert!(s.compositor.transitions.is_empty());
    // Destroyed, it goes: drawn from its last frame, fading.
    s.destroy.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while s.compositor.surface_rect(1, 1).is_some() {
        assert!(
            Instant::now() < deadline,
            "the window was not destroyed in time"
        );
        s.compositor.clock = Some(Duration::from_millis(100_200));
        s.compositor.serve(&s.listener).unwrap();
    }
    assert_eq!(s.compositor.pixel(centre.0, centre.1), WHITE);
    assert_eq!(s.at(275, centre), white_at(32));
    assert_eq!(s.at(350, centre), DESKTOP);
    assert!(s.compositor.transitions.is_empty());
}

#[test]
fn locking_shows_nothing_of_the_session_going() {
    let (mut s, centre) = session("lock");
    assert_eq!(s.at(200, centre), WHITE);
    // The window starts going (minimized) as the session locks, in the same turn: the cover at
    // once, and nothing of the window drawn going over it.
    let window = oxyos_compositor_core::SurfaceKey { client: 1, id: 1 };
    s.compositor.scene.minimize(window);
    s.compositor.scene.lock();
    assert_eq!(s.at(210, centre), 0);
    assert_eq!(s.at(260, centre), 0);
    assert!(s.compositor.transitions.is_empty());
    // A window destroyed under the lock has no frame to go from.
    s.compositor.scene.restore(window);
    s.destroy.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while s.compositor.scene.stats().surfaces > 0 {
        assert!(
            Instant::now() < deadline,
            "the window was not destroyed in time"
        );
        assert_eq!(s.at(270, centre), 0);
    }
    assert_eq!(s.at(280, centre), 0);
    assert!(s.compositor.transitions.is_empty());
}

#[test]
fn the_splash_shows_the_mark_until_the_session_is_on_screen_then_fades_out() {
    let (mut s, centre) = session("splash");
    // Shown once nothing changes any more: the next frame has it.
    assert_eq!(s.at(200, centre), WHITE);
    assert_eq!(s.at(300, centre), WHITE);
    s.compositor.show_splash();
    // Over everything: the window under it is not seen; the mark is white at the centre, black
    // around.
    let mark = (320, 240);
    assert_eq!(s.at(300, mark), WHITE);
    assert_eq!(s.at(201, centre), WHITE);
    assert_eq!(s.at(202, (5, 5)), 0);
    // The session is on screen: it fades out over 400 ms, the desktop seen through it.
    s.compositor.end_splash();
    assert_eq!(s.at(203, (5, 5)), 0);
    assert_ne!(s.at(400, (5, 5)), 0);
    assert_eq!(s.at(603, (5, 5)), DESKTOP);
    assert!(s.compositor.transitions.is_empty());
    assert!(!s.compositor.showing_splash());
}

impl Session {
    /// Composes the frame at `ms` and checks that everything it changed on the output is within
    /// its damage: a display composing only the damage shows the same frame as one composing it
    /// whole. True if the frame was composed from its damage (not whole).
    fn damage_covers_changes(&mut self, ms: u64) -> bool {
        let before = self.compositor.screens[0].framebuffer.clone();
        let frames = self.compositor.frames;
        self.compositor.clock = Some(Duration::from_secs(100) + Duration::from_millis(ms));
        self.compositor.serve(&self.listener).unwrap();
        if self.compositor.frames == frames {
            assert_eq!(
                self.compositor.screens[0].framebuffer, before,
                "no frame, no change"
            );
            return true;
        }
        let Some(damage) = self.compositor.screens[0].last_damage.clone() else {
            return false;
        };
        let width = 640;
        for (index, (old, new)) in before
            .iter()
            .zip(&self.compositor.screens[0].framebuffer)
            .enumerate()
        {
            if old == new {
                continue;
            }
            let (x, y) = ((index % width) as i32, (index / width) as i32);
            assert!(
                damage.iter().any(|r| x >= r.x
                    && y >= r.y
                    && x < r.x + r.width as i32
                    && y < r.y + r.height as i32),
                "({x}, {y}) changed at {ms} ms outside the damage {damage:?}"
            );
        }
        true
    }
}

#[test]
fn a_transition_is_composed_from_its_damage_alone() {
    let (mut s, _) = session("damage");
    // Arriving: every frame of the window's fade and growth from its damage, the last (whole
    // again) included.
    for ms in [20, 45, 90, 135, 170, 180, 200] {
        assert!(s.damage_covers_changes(ms), "arriving, at {ms} ms");
    }
    // Going: from its last frame, shrinking and fading, to nothing.
    s.destroy.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while s.compositor.surface_rect(1, 1).is_some() {
        assert!(
            Instant::now() < deadline,
            "the window was not destroyed in time"
        );
        assert!(s.damage_covers_changes(200));
    }
    for ms in [220, 260, 300, 340, 360] {
        assert!(s.damage_covers_changes(ms), "going, at {ms} ms");
    }
    assert!(s.compositor.transitions.is_empty());
    // Damaged, not whole: a transition damages its surface, not the output.
    let damaged = s
        .compositor
        .damaged
        .iter()
        .skip(1)
        .filter(|&&share| share < 1.0)
        .count();
    assert!(
        damaged >= 10,
        "frames composed from their damage: {damaged}"
    );
}

#[test]
fn a_window_minimized_shrinks_into_the_dock_and_a_workspace_slides_aside() {
    let (mut s, centre) = session("minimize");
    let window = oxyos_compositor_core::SurfaceKey { client: 1, id: 1 };
    assert_eq!(s.at(200, centre), WHITE);
    // Minimized: it shrinks toward the dock (without a shell, the output's bottom edge's centre)
    // as it fades, from its damage alone, and is gone at 300 ms.
    s.compositor.scene.minimize(window);
    for ms in [200, 230, 280, 350, 420] {
        assert!(s.damage_covers_changes(ms), "minimizing, at {ms} ms");
    }
    assert!(s.damage_covers_changes(500));
    assert_eq!(s.compositor.pixel(centre.0, centre.1), DESKTOP);
    assert!(s.compositor.transitions.is_empty());
    // Restored: back out of the dock to where it was.
    s.compositor.scene.restore(window);
    for ms in [600, 640, 700, 780, 900] {
        assert!(s.damage_covers_changes(ms), "restoring, at {ms} ms");
    }
    assert_eq!(s.compositor.pixel(centre.0, centre.1), WHITE);
    // Another workspace: the window slides out to the left, whole (never faded), and is gone.
    let workspace = s.compositor.scene.add_workspace().unwrap();
    assert!(s.compositor.scene.activate_workspace(workspace));
    assert!(s.damage_covers_changes(1000));
    assert_eq!(
        s.compositor.pixel(centre.0, centre.1),
        WHITE,
        "not moved yet"
    );
    assert!(s.damage_covers_changes(1030));
    let moved_left = (0..640)
        .filter(|&x| s.compositor.pixel(x, centre.1) == WHITE)
        .max()
        .unwrap();
    assert!(moved_left < centre.0, "{moved_left}");
    assert!(s.damage_covers_changes(1400));
    assert_eq!(s.compositor.pixel(centre.0, centre.1), DESKTOP);
    // Back: it slides in from the left to where it was.
    assert!(s.compositor.scene.activate_workspace(0));
    for ms in [1500, 1560, 1650, 1900] {
        assert!(s.damage_covers_changes(ms), "sliding in, at {ms} ms");
    }
    assert_eq!(s.compositor.pixel(centre.0, centre.1), WHITE);
    assert!(s.compositor.transitions.is_empty());
}

/// What the zooming test tells its application: ask to zoom (or restore), or answer the latest
/// configure with a frame of that size.
enum Ask {
    Zoom(bool),
    Answer,
}

/// The application's frames after its first are this colour (XRGB8888 bytes B, G, R, X).
const NEW: u32 = 0x0030_6090;

/// A compositor on its test clock with a white 200×100 window whose application zooms and answers
/// configures only when told.
fn zoom_session(name: &str) -> (Session, mpsc::Sender<Ask>) {
    let directory =
        std::env::temp_dir().join(format!("oxyos-transitions-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let output = Rect::new(0, 0, 640, 480);
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: output,
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    compositor.clock = Some(Duration::from_secs(100));
    let (ask, asked) = mpsc::channel::<Ask>();
    std::thread::spawn(move || {
        let mut client = Client::connect(&socket).unwrap();
        client
            .send(&Request::CreateToplevel {
                surface: 1,
                size: Size::new(200, 100),
            })
            .unwrap();
        let mut colour = [0xff; 4];
        let mut latest = None;
        let mut answer = true;
        loop {
            if answer {
                while latest.is_none() {
                    match client.next_event().unwrap() {
                        Some(Event::Configure { serial, rect, .. }) => {
                            latest = Some((serial, rect))
                        }
                        Some(_) => {}
                        None => return,
                    }
                }
                let (serial, rect) = latest.take().unwrap();
                let pixels = colour.repeat((rect.width * rect.height) as usize);
                client
                    .present(1, serial, Size::new(rect.width, rect.height), &pixels)
                    .unwrap();
                colour = [0x90, 0x60, 0x30, 0xff];
            }
            match asked.recv() {
                Ok(Ask::Zoom(enabled)) => {
                    client
                        .send(&Request::SetMaximized {
                            surface: 1,
                            enabled,
                        })
                        .unwrap();
                    answer = false;
                }
                Ok(Ask::Answer) => answer = true,
                Err(_) => return,
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while compositor.surface_rect(1, 1).is_none() {
        assert!(
            Instant::now() < deadline,
            "the window was not shown in time"
        );
        compositor.serve(&listener).unwrap();
    }
    let (destroy, _) = mpsc::channel();
    (
        Session {
            compositor,
            listener,
            destroy,
            _directory: TempDir(directory),
        },
        ask,
    )
}

impl Session {
    /// Where the frame composed last shows `colour`: its leftmost, topmost, rightmost and
    /// bottommost pixels along the output's middle row and column, as a rectangle.
    fn drawn(&self, colour: u32) -> Option<Rect> {
        let row: Vec<i32> = (0..640)
            .filter(|&x| self.compositor.pixel(x, 240) == colour)
            .collect();
        let column: Vec<i32> = (0..480)
            .filter(|&y| self.compositor.pixel(320, y) == colour)
            .collect();
        let (left, right) = (*row.first()?, *row.last()?);
        let (top, bottom) = (*column.first()?, *column.last()?);
        Some(Rect::new(
            left,
            top,
            (right - left + 1) as u32,
            (bottom - top + 1) as u32,
        ))
    }

    /// Serves at `ms` on the test's clock, every frame composed within its damage, until `done`
    /// (the application's request handled, its frame shown).
    fn serve_until(&mut self, ms: u64, done: &dyn Fn(&Compositor) -> bool) {
        self.compositor.clock = Some(Duration::from_secs(100) + Duration::from_millis(ms));
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(&self.compositor) {
            assert!(
                Instant::now() < deadline,
                "the application did not answer in time"
            );
            assert!(self.damage_covers_changes(ms), "at {ms} ms");
        }
    }
}

#[test]
fn a_zoom_draws_the_frame_between_and_ends_exactly_at_the_work_area() {
    let (mut s, ask) = zoom_session("zoom");
    let window = oxyos_compositor_core::SurfaceKey { client: 1, id: 1 };
    let from = Rect::new(220, 190, 200, 100);
    // Arrived and whole, white where it is.
    assert!(s.damage_covers_changes(200));
    assert!(s.compositor.transitions.is_empty());
    assert_eq!(s.drawn(WHITE), Some(from));
    // Zoomed: the configure goes, and its old frame is drawn stretched toward the work area (the
    // whole output: there are no panels), each edge by its own distance.
    let serial = s.compositor.scene.configure_serial(window).unwrap();
    ask.send(Ask::Zoom(true)).unwrap();
    s.serve_until(200, &|c| c.scene.configure_serial(window) > Some(serial));
    assert_eq!(s.drawn(WHITE), Some(from), "it starts where it was");
    let between = |s: &Session, ms: u64| {
        s.compositor
            .transitions
            .zooming(window, Duration::from_secs(100) + Duration::from_millis(ms))
            .unwrap()
    };
    assert!(s.damage_covers_changes(260));
    let at = s.drawn(WHITE).unwrap();
    assert_eq!(at, between(&s, 260));
    assert!(
        at.x < from.x && at.width > from.width && at.width < 640,
        "{at:?}"
    );
    assert!(
        at.y < from.y && at.height > from.height && at.height < 480,
        "{at:?}"
    );
    // Its client shows the new size: that frame, drawn in the rectangle between.
    ask.send(Ask::Answer).unwrap();
    s.serve_until(260, &|c| {
        c.surface_rect(1, 1) == Some(Rect::new(0, 0, 640, 480))
    });
    assert!(s.damage_covers_changes(320));
    let at = s.drawn(NEW).unwrap();
    assert_eq!(at, between(&s, 320));
    assert!(at.width < 640 && at.height < 480, "{at:?}");
    assert_eq!(s.drawn(WHITE), None);
    // It ends exactly at the work area, and the transition with it.
    for ms in [380, 450] {
        assert!(s.damage_covers_changes(ms), "at {ms} ms");
    }
    assert_eq!(s.drawn(NEW), Some(Rect::new(0, 0, 640, 480)));
    assert!(s.damage_covers_changes(480));
    assert!(s.compositor.transitions.is_empty());
    let seen = s.compositor.transitions.take_seen();
    let zoom = seen
        .iter()
        .find(|seen| seen.change == oxyos_compositor_core::AppearanceChange::Zoomed)
        .unwrap();
    assert!(zoom.ended && zoom.frames >= 4, "{zoom:?}");
    // With reduce motion, restored at once: drawn where it was until the client answers, then
    // where it goes.
    s.compositor.transitions.set_reduce_motion(true);
    let serial = s.compositor.scene.configure_serial(window).unwrap();
    ask.send(Ask::Zoom(false)).unwrap();
    s.serve_until(600, &|c| c.scene.configure_serial(window) > Some(serial));
    assert!(s.compositor.transitions.is_empty());
    assert_eq!(s.drawn(NEW), Some(Rect::new(0, 0, 640, 480)));
    ask.send(Ask::Answer).unwrap();
    s.serve_until(600, &|c| c.surface_rect(1, 1) == Some(from));
    assert_eq!(s.drawn(NEW), Some(from));
}
