//! Frames paced by the display's refresh, never an assumed 60 Hz
//! (docs/adr/0018-several-displays.mdx), through real sockets: a client is told its window's
//! refresh before its first configure, and a client drawing a frame each time its last one is shown
//! (as an animation's requestAnimationFrame does) gets one shown each vblank: 8.33 ms apart at
//! 120 Hz, twice as many as at 60 Hz in the same time. The window's arrival, a compositor
//! transition on springs of time, lasts as long at either rate with twice the frames at 120 Hz.
//! The output is paced as a display without one (`Screen::paced`): each frame completes at its next
//! vblank, as on a display at its flip.

use oxyos_client::Client;
use oxyos_compositor::Compositor;
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{Event, Outcome, Rect, Request, Size};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What the client heard: the refresh it was told (and whether before its first configure), and
/// when each of its frames was shown (the feedback's timestamps, CLOCK_MONOTONIC, in ns).
struct Heard {
    refresh_mhz: u32,
    before_configure: bool,
    shown_ns: Vec<u64>,
    /// The window's arrival: the frames the compositor drew in it, and its last one's time (ms).
    arrival: (u32, u32),
}

/// A compositor with one output paced at `refresh_mhz`, and a client drawing a new frame of its
/// window as soon as its last is shown, for `lasting`.
fn run(name: &str, refresh_mhz: u32, lasting: Duration) -> Heard {
    let directory =
        std::env::temp_dir().join(format!("oxyos-refresh-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let _cleanup = TempDir(directory.clone());
    let socket = directory.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: Rect::new(0, 0, 128, 96),
            scale_120: 120,
            refresh_mhz,
        })
        .unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    compositor.screen_mut(1).unwrap().paced = true;
    let (told, heard) = mpsc::channel::<Event>();
    let client = std::thread::spawn(move || {
        let mut client = Client::connect(&socket).unwrap();
        client
            .send(&Request::CreateToplevel {
                surface: 1,
                size: Size::new(48, 32),
            })
            .unwrap();
        let mut configure = None;
        // The window's pixels change each frame: each is a frame of something moving.
        let mut shade = 0u8;
        while let Ok(Some(event)) = client.next_event() {
            let draw = match event {
                Event::Configure { serial, rect, .. } => {
                    configure = Some((serial, Size::new(rect.width, rect.height)));
                    true
                }
                Event::Feedback {
                    outcome: Outcome::Presented,
                    ..
                } => true,
                _ => false,
            };
            if told.send(event).is_err() {
                break;
            }
            if let (true, Some((serial, size))) = (draw, configure) {
                shade = shade.wrapping_add(1);
                let pixels = vec![shade; (size.width * size.height * 4) as usize];
                if client.present(1, serial, size, &pixels).is_err() {
                    break;
                }
            }
        }
    });
    let start = Instant::now();
    while start.elapsed() < lasting {
        compositor.serve(&listener).unwrap();
    }
    let seen = compositor.transitions.take_seen();
    let arrived = seen
        .iter()
        .find(|seen| seen.arriving())
        .expect("the window arrived");
    let arrival = (
        arrived.frames,
        arrived.path.last().map_or(0, |step| step.ms),
    );
    drop(compositor);
    drop(listener);
    client.join().unwrap();
    let events: Vec<Event> = heard.try_iter().collect();
    let refresh = events
        .iter()
        .position(|event| matches!(event, Event::Refresh { surface: 1, .. }));
    let configure = events
        .iter()
        .position(|event| matches!(event, Event::Configure { surface: 1, .. }));
    Heard {
        refresh_mhz: refresh.map_or(0, |at| match events[at] {
            Event::Refresh { refresh_mhz, .. } => refresh_mhz,
            _ => unreachable!(),
        }),
        before_configure: matches!((refresh, configure), (Some(r), Some(c)) if r < c),
        shown_ns: events
            .iter()
            .filter_map(|event| match event {
                Event::Feedback {
                    outcome: Outcome::Presented,
                    timestamp_ns,
                    ..
                } => Some(*timestamp_ns),
                _ => None,
            })
            .collect(),
        arrival,
    }
}

/// Once the window's arrival is at rest (its springs within 600 ms, ADR 0011: while it runs, the
/// compositor composes its frame at each vblank before the client's next frame comes): the frames
/// shown, and the time between them, in ns.
fn steady(shown_ns: &[u64]) -> Vec<u64> {
    let from = shown_ns.first().copied().unwrap_or(0) + 700_000_000;
    shown_ns.iter().copied().filter(|&at| at >= from).collect()
}

fn gaps(shown_ns: &[u64]) -> Vec<u64> {
    steady(shown_ns)
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect()
}

fn median(mut values: Vec<u64>) -> u64 {
    values.sort_unstable();
    values[values.len() / 2]
}

#[test]
fn a_client_drawing_each_frame_shown_gets_a_frame_each_vblank_at_120_hz_as_at_60() {
    let lasting = Duration::from_millis(1200);
    let fast = run("120", 120_000, lasting);
    let slow = run("60", 60_000, lasting);
    // Told before its window's first configure: its first frame is paced by it already.
    assert_eq!((fast.refresh_mhz, fast.before_configure), (120_000, true));
    assert_eq!((slow.refresh_mhz, slow.before_configure), (60_000, true));
    for (heard, period_ns) in [(&fast, 8_333_333), (&slow, 16_666_666)] {
        let gaps = gaps(&heard.shown_ns);
        assert!(
            gaps.len() > 10,
            "too few frames shown: {:?}",
            heard.shown_ns
        );
        // Each on a vblank: a whole number of refreshes apart, never sooner than one.
        assert!(
            gaps.iter().all(|&gap| {
                let refreshes = (gap + period_ns / 2) / period_ns;
                refreshes >= 1 && gap.abs_diff(refreshes * period_ns) <= 1_000
            }),
            "frames off the vblanks of a {period_ns} ns refresh: {gaps:?}"
        );
        // And a frame on each one, mostly.
        assert!(
            median(gaps.clone()).abs_diff(period_ns) <= 1_000,
            "{gaps:?}"
        );
    }
    // The window's arrival is as long at either rate (its springs are time's: at rest by 441 ms,
    // its last frame within a refresh of that), with about twice the frames at 120 Hz.
    let ((fast_frames, fast_last), (slow_frames, slow_last)) = (fast.arrival, slow.arrival);
    assert!(
        fast_last.abs_diff(slow_last) <= 17,
        "{fast_last} ms, {slow_last} ms"
    );
    assert!((420..=460).contains(&fast_last), "{fast_last} ms");
    let ratio = f64::from(fast_frames) / f64::from(slow_frames);
    assert!(
        (1.6..=2.4).contains(&ratio),
        "{fast_frames} frames arriving at 120 Hz, {slow_frames} at 60"
    );
    // The same time shows about twice the frames at 120 Hz.
    let (fast, slow) = (steady(&fast.shown_ns).len(), steady(&slow.shown_ns).len());
    let ratio = fast as f64 / slow as f64;
    assert!(
        (1.6..=2.4).contains(&ratio),
        "{fast} frames at 120 Hz, {slow} at 60"
    );
}
