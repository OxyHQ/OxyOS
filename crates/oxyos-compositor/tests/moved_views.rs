//! Windows moved into the shell's views (docs/adr/0045-app-expose.mdx), through real sockets on
//! the compositor's clock set by the test, as App Exposé moves them: arriving, the window flies
//! from where it is to its view on the overlay's spring, whole, and is drawn nowhere else; going,
//! it flies back and is drawn where it is again once there. With reduce motion nothing flies: the
//! view is a copy that fades in with its overlay, the window staying where it is.

use oxyos_client::Client;
use oxyos_compositor::{Compositor, DESKTOP};
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{Event, Format, PanelEdge, Rect, Request, ShellKind, Size};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const RED: u32 = 0x00ff_0000;
/// Where the shell places the window on its overlay: a 200 × 100 window fitted into it is
/// 100 × 50 at (40, 325).
const CARD: Rect = Rect::new(40, 300, 100, 100);

/// The application's window, 200 × 100: its left half red, its right half a one-pixel
/// checkerboard of black and white (B, G, R, X).
fn first_frame() -> Vec<u8> {
    let mut pixels = Vec::with_capacity(200 * 100 * 4);
    for y in 0..100u32 {
        for x in 0..200u32 {
            let colour = if x < 100 {
                RED
            } else if (x + y) % 2 == 0 {
                0x00ff_ffff
            } else {
                0
            };
            pixels.extend((0xff00_0000 | colour).to_le_bytes());
        }
    }
    pixels
}

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Session {
    compositor: Compositor,
    listener: Listener,
    /// Keeps the application's window: dropped, it closes.
    _repaint: mpsc::Sender<u32>,
    /// Tells the shell to open its overview, then to close it.
    close: mpsc::Sender<()>,
    _directory: TempDir,
}

impl Session {
    fn at_ms(&mut self, ms: u64) {
        self.compositor.clock = Some(Duration::from_secs(100) + Duration::from_millis(ms));
        self.compositor.serve(&self.listener).unwrap();
    }

    /// Serves, at `ms` on the test's clock, until `done` holds.
    fn until(&mut self, ms: u64, what: &str, done: impl Fn(&Compositor) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(&self.compositor) {
            assert!(
                Instant::now() < deadline,
                "{what} did not happen in time: {:?}",
                self.compositor.scene.render_list()
            );
            self.at_ms(ms);
        }
    }

    fn pixel(&self, at: (i32, i32)) -> u32 {
        self.compositor.pixel(at.0, at.1)
    }
}

/// A compositor on its test clock with an application's window, settled; the shell's overview, a
/// transparent overlay showing the window in `CARD` with the window moved into it, opens when
/// told (`close` once), and closes when told again.
fn session(name: &str, reduce_motion: bool) -> Session {
    let directory =
        std::env::temp_dir().join(format!("oxyos-moved-views-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let shell_socket = directory.join("shell");
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
    compositor.transitions.set_reduce_motion(reduce_motion);
    compositor.shell_listener = Some(Listener::bind(&shell_socket).unwrap());
    compositor.clock = Some(Duration::from_secs(100));
    let (repaint, repaints) = mpsc::channel::<u32>();
    std::thread::spawn(move || {
        let mut client = Client::connect(&socket).unwrap();
        client
            .send(&Request::CreateToplevel {
                surface: 1,
                size: Size::new(200, 100),
            })
            .unwrap();
        let configure = loop {
            if let Some(Event::Configure { serial, .. }) = client.next_event().unwrap() {
                break serial;
            }
        };
        client
            .present(1, configure, Size::new(200, 100), &first_frame())
            .unwrap();
        while let Ok(colour) = repaints.recv() {
            let pixels: Vec<u8> = (0..200 * 100)
                .flat_map(|_| (0xff00_0000 | colour).to_le_bytes())
                .collect();
            // Its buffers come back as frames are shown.
            while matches!(client.poll(), Ok(oxyos_client::Poll::Event(_))) {}
            client
                .present(1, configure, Size::new(200, 100), &pixels)
                .unwrap();
        }
    });
    let (close, closes) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut shell = Client::connect(&shell_socket).unwrap();
        if closes.recv().is_err() {
            return;
        }
        shell
            .send(&Request::CreateShellSurface {
                surface: 1,
                kind: ShellKind::Overlay,
                edge: PanelEdge::None,
                output: 1,
                size: Size::new(640, 480),
                exclusive: 0,
            })
            .unwrap();
        // The window's number, and the overlay's configure.
        let (mut window, mut configure) = (None, None);
        while window.is_none() || configure.is_none() {
            match shell.next_event().unwrap() {
                Some(Event::WindowState { window: number, .. }) => window = Some(number),
                Some(Event::Configure { serial, .. }) => configure = Some(serial),
                _ => {}
            }
        }
        shell
            .send(&Request::ClearWindowViews { surface: 1 })
            .unwrap();
        shell
            .send(&Request::PlaceWindowView {
                surface: 1,
                window: window.unwrap(),
                rect: CARD,
                clip: CARD,
                radius: 0,
                opacity: 255,
            })
            .unwrap();
        shell
            .send(&Request::MoveWindowsToViews {
                surface: 1,
                moved: true,
            })
            .unwrap();
        // Transparent: what shows is the window, and the desktop around it.
        shell
            .present_format(
                1,
                configure.unwrap(),
                Size::new(640, 480),
                Format::Argb8888,
                &vec![0; 640 * 480 * 4],
            )
            .unwrap();
        if closes.recv().is_ok() {
            shell.send(&Request::Destroy { surface: 1 }).unwrap();
        }
        let _ = closes.recv();
    });
    let mut session = Session {
        compositor,
        listener,
        _repaint: repaint,
        close,
        _directory: TempDir(directory),
    };
    // The window arrives at the clock's start, whole by 600 ms; the shell is connected.
    session.until(600, "the window", |c| c.scene.render_list().len() == 1);
    session.at_ms(1000);
    session
}

const OPEN: u64 = 2000;

impl Session {
    /// The window's own rectangle.
    fn window(&self) -> Rect {
        self.compositor.surface_rect(1, 1).unwrap()
    }

    /// Opens the overview at `OPEN`, served until its first frame is composed.
    fn open(&mut self) {
        self.close.send(()).unwrap();
        self.until(OPEN, "the overview showing the window", |c| {
            c.scene.render_list().len() == 3
        });
    }
}

/// Where the window's view is at its end: 200 × 100 fitted into the card.
const VIEW: Rect = Rect::new(40, 325, 100, 50);

/// Arriving, the window flies from where it is to its view, whole, on the overlay's spring: at
/// its first frame it is exactly where it was, halfway it is between, and at the end it is in
/// the card; where it was, only the desktop shows once it has left. Going, it flies back, and is
/// drawn where it is again.
#[test]
fn a_window_moved_into_its_view_flies_there_and_back() {
    let mut s = session("flies", false);
    let window = s.window();
    let left = (window.x + 20, window.y + 50);
    s.open();
    // Its first frame: the view is the window, where it is.
    assert_eq!(s.pixel(left), RED);
    let flights = s.compositor.take_flights();
    assert_eq!(flights.len(), 1, "{flights:?}");
    assert!(flights[0].arriving);
    assert_eq!(flights[0].path[0], window);
    // On its way: between, nearer the card than linear at a quarter of its time (eased out).
    s.at_ms(OPEN + 80);
    let flights = s.compositor.take_flights();
    let at = *flights[0].path.last().unwrap();
    assert!(
        at.x < window.x && at.x > VIEW.x,
        "{at:?} from {window:?} to {VIEW:?}"
    );
    let travelled = f64::from(window.x - at.x) / f64::from(window.x - VIEW.x);
    assert!(travelled > 0.4, "{travelled}");
    // Arrived: in the card, and where it was the desktop, nothing of the window there.
    s.at_ms(OPEN + 600);
    assert_eq!(s.pixel((60, 340)), RED);
    assert_eq!(s.pixel(left), DESKTOP);
    assert_eq!(
        s.pixel((window.x + 150, window.y + 50)),
        DESKTOP,
        "nothing of the window where it is"
    );
    // The overview closes: back, from the card toward the window.
    s.close.send(()).unwrap();
    let close = OPEN + 1000;
    s.until(close, "the overview closing", |c| {
        c.scene.render_list().len() == 1
    });
    let flights = s.compositor.take_flights();
    let back = flights.iter().find(|f| !f.arriving).expect("flying back");
    assert_eq!(back.path[0], VIEW);
    s.at_ms(close + 80);
    s.at_ms(close + 600);
    let flights = s.compositor.take_flights();
    let back = flights.iter().find(|f| !f.arriving).expect("flying back");
    let path = &back.path;
    assert!(
        path.windows(2)
            .all(|w| w[1].x >= w[0].x && w[1].width >= w[0].width),
        "{path:?}"
    );
    // Home: where it is, whole, and the card is the desktop.
    assert_eq!(s.pixel(left), RED);
    assert_eq!(
        s.pixel((window.x + 150, window.y + 50)) & 0xff_0000,
        0xff_0000
    );
    assert_eq!(s.pixel((60, 340)), DESKTOP);
    assert_eq!(s.window(), window, "the window never moved");
}

/// With reduce motion nothing flies: the window stays where it is while its view, a copy, fades
/// in with the overlay in the card.
#[test]
fn with_reduce_motion_a_moved_window_does_not_fly() {
    let mut s = session("reduced", true);
    let window = s.window();
    let left = (window.x + 20, window.y + 50);
    s.open();
    s.at_ms(OPEN + 600);
    assert!(s.compositor.take_flights().is_empty());
    assert_eq!(s.pixel((60, 340)), RED);
    assert_eq!(s.pixel(left), RED);
}
