//! The pointer moving over what does not change composes nothing (docs/adr/0005-damage.mdx): a
//! display shows it on its cursor plane, committed alone (`Display::present_again`), so a frame
//! is composed only when something shown changes. Through real sockets, on the compositor's
//! clock set by the test: a window shown and settled, the pointer moved across it, onto the
//! desktop and back, and no frame composed, no client's pixels read.

use oxyos_client::Client;
use oxyos_compositor::Compositor;
use oxyos_compositor_core::{Output, Scene};
use oxyos_input::Action;
use oxyos_ipc::Listener;
use oxyos_protocol::{Event, Rect, Request, Size};
use std::time::{Duration, Instant};

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_pointer_moving_over_what_does_not_change_composes_nothing() {
    let directory =
        TempDir(std::env::temp_dir().join(format!("oxyos-pointer-motion-{}", std::process::id())));
    std::fs::create_dir_all(&directory.0).unwrap();
    let socket = directory.0.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: Rect::new(0, 0, 640, 480),
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory.0).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    compositor.clock = Some(Duration::from_secs(100));
    // A window that draws once and then only listens: what the pointer does to it changes
    // nothing it shows.
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
        let pixels: Vec<u8> = (0..200 * 100)
            .flat_map(|_| 0xff20_4080u32.to_le_bytes())
            .collect();
        client
            .present(1, configure, Size::new(200, 100), &pixels)
            .unwrap();
        while client.next_event().is_ok() {}
    });
    let serve_at = |compositor: &mut Compositor, ms: u64| {
        compositor.clock = Some(Duration::from_secs(100) + Duration::from_millis(ms));
        compositor.serve(&listener).unwrap();
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while compositor.surface_sequence(1, 1).is_none() {
        assert!(Instant::now() < deadline, "the window was never shown");
        serve_at(&mut compositor, 0);
    }
    // Its arrival over, nothing more to compose.
    for ms in (1000..=3000).step_by(100) {
        serve_at(&mut compositor, ms);
    }
    let window = compositor.surface_rect(1, 1).unwrap();
    let read = |compositor: &Compositor| compositor.bytes_read.values().sum::<u64>();
    let (frames, bytes) = (compositor.frames, read(&compositor));
    let mut ms = 3000;
    // Across the window, onto the desktop and back, a motion each turn.
    let (from, to) = (
        (window.x - 50, window.y + window.height as i32 / 2),
        (window.x + window.width as i32 + 50, window.y + 10),
    );
    for step in 0..=40 {
        let share = f64::from(step) / 40.0;
        let x = f64::from(from.0) + f64::from(to.0 - from.0) * share;
        let y = f64::from(from.1) + f64::from(to.1 - from.1) * share;
        compositor.apply_input(Action::Motion { x, y });
        ms += 8;
        serve_at(&mut compositor, ms);
    }
    assert_eq!(
        compositor.frames - frames,
        0,
        "frames composed while only the pointer moved"
    );
    assert_eq!(
        read(&compositor) - bytes,
        0,
        "a client's pixels read while only the pointer moved"
    );
    // Something shown changing still composes: the window gone.
    compositor.disconnect(1);
    for step in 1..=20 {
        serve_at(&mut compositor, ms + step * 50);
    }
    assert!(
        compositor.frames > frames,
        "the window going composed nothing"
    );
}
