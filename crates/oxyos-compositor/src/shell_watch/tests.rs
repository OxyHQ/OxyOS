//! The shell watched (docs/adr/0061, decision 6, amended): pinged whatever the person does, armed
//! on its first answer, hung after eight seconds, its report asked for and then its end.

use super::*;
use oxyos_compositor_core::Grants;
use oxyos_protocol::{Event, Request};

const SHELL: ClientId = 1;
const PID: u32 = 4321;

/// A scene with the shell connected, speaking a version that answers Pings, and the clock at 0.
fn scene() -> Scene {
    let mut scene = Scene::new();
    scene
        .connect(
            SHELL,
            Grants {
                shell: true,
                ..Grants::default()
            },
        )
        .unwrap();
    scene.set_version(SHELL, oxyos_protocol::VERSION);
    scene.set_clock(Duration::ZERO);
    scene
}

/// The serial of the Ping the shell was sent last, if any since the last call.
fn pinged(scene: &mut Scene) -> Option<u32> {
    scene
        .drain_events()
        .into_iter()
        .rev()
        .find_map(|(client, event)| match event {
            Event::Ping { serial } if client == SHELL => Some(serial),
            _ => None,
        })
}

fn answer(scene: &mut Scene, serial: u32) {
    scene.handle(SHELL, Request::Pong { serial }).unwrap();
}

struct Clock {
    start: Instant,
    at: Duration,
}

impl Clock {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            at: Duration::ZERO,
        }
    }

    /// The time passes for the scene and the program alike.
    fn pass(&mut self, scene: &mut Scene, by: Duration) -> Instant {
        self.at += by;
        scene.set_clock(self.at);
        self.start + self.at
    }

    fn now(&self) -> Instant {
        self.start + self.at
    }
}

fn shells() -> Vec<(u32, Vec<ClientId>)> {
    vec![(PID, vec![SHELL])]
}

#[test]
fn the_shell_is_pinged_every_two_seconds_whatever_the_person_does() {
    let mut scene = scene();
    let mut clock = Clock::new();
    let mut watch = ShellWatch::default();
    assert!(watch
        .serve_scene(&mut scene, &shells(), clock.now())
        .is_empty());
    let first = pinged(&mut scene).expect("pinged at once");
    answer(&mut scene, first);
    assert_eq!(watch.deadline(), Some(clock.now() + KEEP_ALIVE));
    // Not again before its time.
    let now = clock.pass(&mut scene, Duration::from_millis(1500));
    watch.serve_scene(&mut scene, &shells(), now);
    assert_eq!(pinged(&mut scene), None);
    let now = clock.pass(&mut scene, Duration::from_millis(500));
    watch.serve_scene(&mut scene, &shells(), now);
    assert!(pinged(&mut scene).is_some());
}

#[test]
fn a_shell_silent_for_eight_seconds_is_asked_for_its_report_then_ended() {
    let mut scene = scene();
    let mut clock = Clock::new();
    let mut watch = ShellWatch::default();
    watch.serve_scene(&mut scene, &shells(), clock.now());
    let first = pinged(&mut scene).unwrap();
    // It answers once: watched from now on.
    let now = clock.pass(&mut scene, Duration::from_millis(100));
    answer(&mut scene, first);
    watch.serve_scene(&mut scene, &shells(), now);
    let now = clock.pass(&mut scene, KEEP_ALIVE);
    watch.serve_scene(&mut scene, &shells(), now);
    assert!(pinged(&mut scene).is_some());
    // Then nothing: until 7.9 s nothing is done, at 8 s its report is asked for.
    let mut signals = Vec::new();
    for _ in 0..39 {
        let now = clock.pass(&mut scene, Duration::from_millis(200));
        signals.extend(watch.serve_scene(&mut scene, &shells(), now));
    }
    let now = clock.pass(&mut scene, Duration::from_millis(100));
    signals.extend(watch.serve_scene(&mut scene, &shells(), now));
    assert!(signals.is_empty(), "{signals:?}");
    let now = clock.pass(&mut scene, Duration::from_millis(100));
    assert_eq!(
        watch.serve_scene(&mut scene, &shells(), now),
        vec![(PID, ShellSignal::Report)]
    );
    // Its end once its report had its time, never before and only once.
    assert_eq!(watch.deadline(), Some(now + REPORT_WAIT));
    let now = clock.pass(&mut scene, REPORT_WAIT - Duration::from_millis(1));
    assert!(watch.serve_scene(&mut scene, &shells(), now).is_empty());
    let now = clock.pass(&mut scene, Duration::from_millis(1));
    assert_eq!(
        watch.serve_scene(&mut scene, &shells(), now),
        vec![(PID, ShellSignal::End)]
    );
    let now = clock.pass(&mut scene, Duration::from_secs(1));
    assert!(watch.serve_scene(&mut scene, &shells(), now).is_empty());
    // Ended: how long it had not answered, for its report.
    assert_eq!(watch.ended(PID), Some(Duration::from_millis(8000)));
    assert_eq!(watch.ended(PID), None);
}

#[test]
fn a_shell_that_never_answered_is_starting_and_never_held_hung() {
    let mut scene = scene();
    let mut clock = Clock::new();
    let mut watch = ShellWatch::default();
    watch.serve_scene(&mut scene, &shells(), clock.now());
    assert!(pinged(&mut scene).is_some());
    for _ in 0..30 {
        let now = clock.pass(&mut scene, Duration::from_secs(1));
        assert!(watch.serve_scene(&mut scene, &shells(), now).is_empty());
    }
}

#[test]
fn a_shell_that_answers_late_but_within_eight_seconds_is_never_ended() {
    let mut scene = scene();
    let mut clock = Clock::new();
    let mut watch = ShellWatch::default();
    watch.serve_scene(&mut scene, &shells(), clock.now());
    let first = pinged(&mut scene).unwrap();
    answer(&mut scene, first);
    let now = clock.pass(&mut scene, KEEP_ALIVE);
    watch.serve_scene(&mut scene, &shells(), now);
    let second = pinged(&mut scene).unwrap();
    // A long stall of 7.5 s (a display's modeset, a loaded machine), then its answer.
    for _ in 0..15 {
        let now = clock.pass(&mut scene, Duration::from_millis(500));
        assert!(watch.serve_scene(&mut scene, &shells(), now).is_empty());
    }
    answer(&mut scene, second);
    let now = clock.pass(&mut scene, Duration::from_millis(500));
    assert!(watch.serve_scene(&mut scene, &shells(), now).is_empty());
    assert!(pinged(&mut scene).is_some());
}

#[test]
fn a_shell_started_again_is_watched_afresh_and_one_gone_is_forgotten() {
    let mut scene = scene();
    let mut clock = Clock::new();
    let mut watch = ShellWatch::default();
    watch.serve_scene(&mut scene, &shells(), clock.now());
    let first = pinged(&mut scene).unwrap();
    answer(&mut scene, first);
    let now = clock.pass(&mut scene, Duration::from_millis(100));
    watch.serve_scene(&mut scene, &shells(), now);
    // It ended on its own (a crash): nothing is said of a hang, and nothing is due for it.
    assert_eq!(watch.ended(PID), None);
    assert!(watch.serve_scene(&mut scene, &[], now).is_empty());
    assert_eq!(watch.deadline(), None);
}
