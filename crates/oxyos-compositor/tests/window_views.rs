//! The shell's overview showing windows live (docs/adr/0022-window-views.mdx), through real
//! sockets on the compositor's clock set by the test: a window's own pixels drawn where the
//! shell's overlay places it, scaled down and box-filtered; its next frame there in the frame
//! that first shows it; nothing composed when nothing changed; nothing of it under the lock; and
//! the windows going with the overview as it fades out.

use oxyos_client::Client;
use oxyos_compositor::{Compositor, DESKTOP};
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{Event, Format, PanelEdge, Rect, Request, ShellKind, Size};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const RED: u32 = 0x00ff_0000;
const BLUE: u32 = 0x0000_00ff;
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
    /// Tells the application to show another frame, all of this colour.
    repaint: mpsc::Sender<u32>,
    /// Tells the shell to close its overview.
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
            assert!(Instant::now() < deadline, "{what} did not happen in time");
            self.at_ms(ms);
        }
    }

    fn pixel(&self, at: (i32, i32)) -> u32 {
        self.compositor.pixel(at.0, at.1)
    }
}

/// A compositor on its test clock with an application's window and the shell's overview, a
/// transparent overlay showing the window in `CARD`; served until the overview shows it, at
/// 300 ms (its arrival done).
fn session(name: &str) -> Session {
    let directory =
        std::env::temp_dir().join(format!("oxyos-window-views-{name}-{}", std::process::id()));
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
        repaint,
        close,
        _directory: TempDir(directory),
    };
    // Its first frame arrives at the clock's start; whole by 300 ms.
    session.until(0, "the overview showing the window", |c| {
        c.scene.render_list().len() == 3
    });
    session.until(300, "the overview arrived", |c| c.pixel(60, 340) == RED);
    session
}

/// In the card, the window's own pixels at half their size: its red half red, and its
/// checkerboard grey, each output pixel the mean of the four it covers. Nearest neighbour would
/// show black or white there.
#[test]
fn the_overview_shows_the_windows_own_pixels_scaled_and_filtered() {
    let s = session("pixels");
    for x in [41, 60, 89] {
        assert_eq!(s.pixel((x, 340)), RED, "the red half at {x}");
    }
    for (x, y) in [(90, 325), (110, 350), (139, 374)] {
        assert_eq!(s.pixel((x, y)), 0x0080_8080, "the checkerboard at {x}, {y}");
    }
    // Fitted in its proportions: above and below it, the desktop.
    assert_eq!(s.pixel((60, 320)), DESKTOP);
    assert_eq!(s.pixel((60, 380)), DESKTOP);
    // The window itself is where it is too.
    let window = s.compositor.surface_rect(1, 1).unwrap();
    assert_eq!(
        s.pixel((window.x + 50, window.y + window.height as i32 / 2)),
        RED
    );
}

/// Live: the window's next frame is in the card in the very frame that first shows it on
/// screen, and that frame composes only where the window and the card are.
#[test]
fn a_windows_next_frame_is_in_the_overview_in_the_frame_that_shows_it() {
    let mut s = session("live");
    let window = s.compositor.surface_rect(1, 1).unwrap();
    let inside = (window.x + 150, window.y + 50);
    let frames = s.compositor.frames;
    s.repaint.send(BLUE).unwrap();
    s.until(300, "the window's blue frame", |c| {
        c.pixel(inside.0, inside.1) == BLUE
    });
    assert_eq!(s.pixel((60, 340)), BLUE, "the card shows the same frame");
    assert_eq!(s.pixel((110, 350)), BLUE);
    assert_eq!(s.compositor.frames, frames + 1, "in one frame");
    // Composed: the window (with its shadow) and the card, not the output.
    let damage = s.compositor.screens[0]
        .last_damage
        .clone()
        .expect("from its damage");
    let card = oxyos_gpu::Rect {
        x: 40,
        y: 325,
        width: 100,
        height: 50,
    };
    assert!(damage.contains(&card), "{damage:?}");
    let area: u64 = damage
        .iter()
        .map(|rect| u64::from(rect.width) * u64::from(rect.height))
        .sum();
    assert!(area < 640 * 480 / 3, "{area} pixels composed");
}

/// Nothing changed, nothing is composed: an open overview costs no frame of its own.
#[test]
fn an_open_overview_composes_nothing_when_nothing_changed() {
    let mut s = session("idle");
    let frames = s.compositor.frames;
    for ms in [310, 320, 330, 400, 500] {
        s.at_ms(ms);
    }
    assert_eq!(s.compositor.frames, frames);
}

/// Locked, nothing of the window is drawn in the overview, or anywhere.
#[test]
fn nothing_of_a_window_in_the_overview_shows_under_the_lock() {
    let mut s = session("lock");
    s.compositor.scene.lock();
    s.at_ms(310);
    for at in [(60, 340), (110, 350)] {
        assert_ne!(s.pixel(at), RED);
        assert_ne!(s.pixel(at), 0x0080_8080);
    }
    assert!(s.compositor.screens[0].framebuffer.iter().all(|&p| p == 0));
    // Unlocked, the overview shows it again.
    s.compositor.scene.unlock();
    s.until(400, "the overview again", |c| c.pixel(60, 340) == RED);
}

/// Closed, the overview fades out with the windows it showed, as one: halfway, the card is red
/// faded over the desktop; at the end, the desktop. Without them going with it the card would be
/// empty at once.
#[test]
fn the_windows_in_the_overview_go_with_it() {
    let mut s = session("going");
    s.close.send(()).unwrap();
    let start = 300;
    s.until(start, "the overview closing", |c| {
        c.scene.render_list().len() == 1
    });
    s.at_ms(start + 75);
    let faded = s.pixel((60, 350));
    assert_ne!(faded, DESKTOP, "the card's window fading with it");
    assert_ne!(faded, RED);
    assert!(
        faded >> 16 > DESKTOP >> 16,
        "{faded:06x}: red over the desktop"
    );
    s.at_ms(start + 200);
    assert_eq!(s.pixel((60, 350)), DESKTOP);
}
