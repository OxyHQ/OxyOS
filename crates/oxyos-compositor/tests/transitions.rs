//! Surfaces arriving and going, animated (docs/adr/0011-transitions.mdx), through real sockets
//! on the compositor's clock set by the test: what each composed frame shows of a window as it
//! arrives and goes, and that locking shows nothing of it going.

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
    let mut compositor = Compositor::new(scene, uid, output);
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
