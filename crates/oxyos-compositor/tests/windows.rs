//! Windows drawn as windows (docs/adr/0016-windows.mdx), through real sockets: a window's corners
//! show what is under them, its shadow darkens what is below it, the active window's deeper; a
//! change of the active window is drawn from its damage; a full-screen window has neither.

use oxyos_client::{Client, Poll};
use oxyos_compositor::{Compositor, DESKTOP};
use oxyos_compositor_core::{Output, Scene, SurfaceKey};
use oxyos_ipc::Listener;
use oxyos_protocol::{Event, Rect, Request, Size, WINDOW_RADIUS};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const WHITE: u32 = 0x00ff_ffff;
const OUTPUT: Rect = Rect::new(0, 0, 800, 600);

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Session {
    compositor: Compositor,
    listener: Listener,
    /// Tells the application to make its first window full-screen, or not.
    fullscreen: mpsc::Sender<bool>,
    ms: u64,
    _directory: TempDir,
}

/// A compositor on its test clock with one application of two white 200 × 100 windows, shown
/// whole.
fn session(name: &str) -> Session {
    let directory =
        std::env::temp_dir().join(format!("oxyos-windows-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: OUTPUT,
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    compositor.clock = Some(Duration::from_secs(100));
    compositor.keep_frame = true;
    let (fullscreen, asked) = mpsc::channel::<bool>();
    std::thread::spawn(move || {
        let mut client = Client::connect(&socket).unwrap();
        for surface in [1, 2] {
            client
                .send(&Request::CreateToplevel {
                    surface,
                    size: Size::new(200, 100),
                })
                .unwrap();
        }
        // Every configure answered with a white frame of its size.
        loop {
            match client.poll().unwrap() {
                Poll::Event(Event::Configure {
                    surface,
                    serial,
                    rect,
                    ..
                }) => {
                    let pixels = vec![0xff; (rect.width * rect.height * 4) as usize];
                    client
                        .present(surface, serial, Size::new(rect.width, rect.height), &pixels)
                        .unwrap();
                }
                Poll::Event(_) => {}
                Poll::Closed => return,
                Poll::Empty => match asked.try_recv() {
                    Ok(enabled) => client
                        .send(&Request::SetFullscreen {
                            surface: 1,
                            enabled,
                        })
                        .unwrap(),
                    Err(mpsc::TryRecvError::Disconnected) => return,
                    Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(1)),
                },
            }
        }
    });
    let mut s = Session {
        compositor,
        listener,
        fullscreen,
        ms: 0,
        _directory: TempDir(directory),
    };
    s.until("both windows were not shown", |s| {
        s.compositor.surface_rect(1, 1).is_some() && s.compositor.surface_rect(1, 2).is_some()
    });
    // Past their arrival.
    s.ms = 1000;
    s.turn();
    s
}

const FIRST: SurfaceKey = SurfaceKey { client: 1, id: 1 };
const SECOND: SurfaceKey = SurfaceKey { client: 1, id: 2 };

impl Session {
    /// A turn of the compositor at its clock's time.
    fn turn(&mut self) {
        self.compositor.clock = Some(Duration::from_secs(100) + Duration::from_millis(self.ms));
        self.compositor.serve(&self.listener).unwrap();
    }

    fn until(&mut self, what: &str, done: impl Fn(&Session) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(self) {
            assert!(Instant::now() < deadline, "{what} in time");
            self.turn();
        }
    }

    /// Composes the frame 20 ms later and checks that everything it changed on the output is
    /// within its damage, and that it was composed from its damage, not whole.
    fn composed_from_damage(&mut self) {
        let before = self.compositor.screens[0].framebuffer.clone();
        let frames = self.compositor.frames;
        self.ms += 20;
        self.turn();
        assert!(self.compositor.frames > frames, "a frame was composed");
        let damage = self.compositor.screens[0]
            .last_damage
            .clone()
            .expect("composed from its damage");
        for (index, (old, new)) in before
            .iter()
            .zip(&self.compositor.screens[0].framebuffer)
            .enumerate()
        {
            if old != new {
                let (x, y) = ((index % 800) as i32, (index / 800) as i32);
                assert!(
                    damage.iter().any(|r| x >= r.x
                        && y >= r.y
                        && x < r.x + r.width as i32
                        && y < r.y + r.height as i32),
                    "({x}, {y}) changed outside the damage {damage:?}"
                );
            }
        }
    }

    fn pixel(&self, at: (i32, i32)) -> u32 {
        self.compositor.pixel(at.0, at.1)
    }

    /// The last frame's layers: how many are shadows, and each layer's radius.
    fn layers(&self) -> (usize, Vec<u32>) {
        let frame =
            oxyos_gpu::frame_file::read(&mut self.compositor.last_frame.as_slice()).unwrap();
        let layers = frame.layers();
        let shadows = layers
            .iter()
            .filter(|layer| matches!(layer.paint, oxyos_gpu::Paint::Shadow(..)))
            .count();
        (shadows, layers.iter().map(|layer| layer.radius).collect())
    }
}

/// Each channel of `pixel` at most the desktop's, and some below it: the desktop, shadowed.
fn shadowed(pixel: u32) -> bool {
    let channels = |p: u32| [(p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff];
    let (pixel, desktop) = (channels(pixel), channels(DESKTOP));
    pixel.iter().zip(&desktop).all(|(p, d)| p <= d) && pixel != desktop
}

#[test]
fn a_window_has_rounded_corners_and_a_shadow_below_it_deeper_when_active() {
    let mut s = session("look");
    // Apart: the first at the top left, the second at the bottom right.
    s.compositor.scene.move_window_to(FIRST, (60, 60));
    s.compositor.scene.move_window_to(SECOND, (500, 380));
    s.composed_from_damage();
    assert_eq!(
        s.compositor.surface_rect(1, 1),
        Some(Rect::new(60, 60, 200, 100))
    );
    // Its corner pixels are not drawn: the desktop shows there, under the shadow's faint edge;
    // inside the corner's curve and along its sides it is white.
    let r = WINDOW_RADIUS as i32;
    for corner in [(60, 60), (259, 60), (60, 159), (259, 159)] {
        assert!(
            shadowed(s.pixel(corner)),
            "{corner:?}: {:#08x}",
            s.pixel(corner)
        );
    }
    for inside in [(60 + r, 60 + r), (160, 60), (60, 110), (259 - r, 159 - r)] {
        assert_eq!(s.pixel(inside), WHITE, "{inside:?}");
    }
    // The shadow: below the window, darker than above it (it falls 18 px), and gone far away.
    let (below, above) = (s.pixel((160, 172)), s.pixel((160, 48)));
    assert!(
        shadowed(below) && shadowed(above),
        "{below:#08x} {above:#08x}"
    );
    assert!(below & 0xff < above & 0xff, "{below:#08x} {above:#08x}");
    assert_eq!(s.pixel((400, 10)), DESKTOP);
    let (shadows, radii) = s.layers();
    assert_eq!((shadows, radii), (2, vec![0, 16, 0, 16]));
    // The second window was shown last: it is active, its shadow deeper than the first's at the
    // same place below it.
    assert_eq!(s.compositor.scene.focus(), Some(SECOND));
    let second_below = s.pixel((600, 492));
    assert!(
        second_below & 0xff < below & 0xff,
        "{second_below:#08x} {below:#08x}"
    );
    // The first made active: the shadows change places, drawn from their damage alone.
    s.compositor.scene.activate(FIRST);
    s.composed_from_damage();
    assert_eq!(s.pixel((160, 172)), second_below);
    assert_eq!(s.pixel((600, 492)), below);
}

#[test]
fn a_full_screen_window_has_neither_corners_nor_shadow() {
    let mut s = session("fullscreen");
    s.fullscreen.send(true).unwrap();
    s.until("the window was not full-screen", |s| {
        s.compositor.surface_rect(1, 1) == Some(OUTPUT)
    });
    s.ms += 20;
    s.turn();
    for corner in [(0, 0), (799, 0), (0, 599), (799, 599)] {
        assert_eq!(s.pixel(corner), WHITE, "{corner:?}");
    }
    // Only the other window keeps its own.
    let (shadows, radii) = s.layers();
    assert_eq!(shadows, 1);
    assert_eq!(radii.iter().filter(|&&radius| radius == 16).count(), 1);
    // Back from full-screen: a window again.
    s.fullscreen.send(false).unwrap();
    s.until("the window was not restored", |s| {
        s.compositor
            .surface_rect(1, 1)
            .is_some_and(|r| r.width == 200)
    });
    s.ms += 20;
    s.turn();
    let rect = s.compositor.surface_rect(1, 1).unwrap();
    assert!(shadowed(s.pixel((rect.x, rect.y))));
    assert_eq!(s.layers().0, 2);
}
