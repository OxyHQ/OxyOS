//! The session's side of Activity Monitor over this machine's `/proc`: who may ask, a sample told
//! at once and every two seconds while the follower's window is shown and never while it is not
//! (its thread not waking at all), processes quit and inspected, and an application quit.

use super::*;
use oxyos_compositor_core::{Grants, Output};
use oxyos_protocol::{Clock, Event, Format, Rect, Request, Size};
use serde_json::Value;

const MONITOR: ClientId = 1;
const OTHER: ClientId = 2;

struct Session {
    compositor: Compositor,
    activity: Activity,
    told: Vec<(ClientId, u32, String)>,
    partial: HashMap<(ClientId, u32), String>,
    serial: u32,
    configure: Option<(u64, Size)>,
}

impl Session {
    fn new() -> Self {
        let mut scene = Scene::new();
        scene
            .add_output(Output {
                id: 1,
                rect: Rect::new(0, 0, 1280, 800),
                scale_120: 120,
                refresh_mhz: 60_000,
            })
            .unwrap();
        scene
            .connect(
                MONITOR,
                Grants {
                    activity: true,
                    ..Grants::default()
                },
            )
            .unwrap();
        scene
            .connect(
                OTHER,
                Grants {
                    settings: true,
                    ..Grants::default()
                },
            )
            .unwrap();
        Self {
            compositor: Compositor::new(scene, 1000),
            activity: Activity::new().unwrap(),
            told: Vec::new(),
            partial: HashMap::new(),
            serial: 0,
            configure: None,
        }
    }

    fn drain(&mut self) {
        for (client, event) in self.compositor.scene.drain_events() {
            match event {
                Event::SystemText {
                    serial, text, last, ..
                } => {
                    let whole = self.partial.entry((client, serial)).or_default();
                    whole.push_str(text.as_str());
                    if last {
                        let whole = self.partial.remove(&(client, serial)).unwrap();
                        self.told.push((client, serial, whole));
                    }
                }
                Event::Configure { serial, rect, .. } if client == MONITOR => {
                    self.configure = Some((serial, Size::new(rect.width, rect.height)));
                }
                _ => {}
            }
        }
    }

    /// Activity Monitor's window, drawn once: shown.
    fn window(&mut self) {
        self.compositor
            .scene
            .handle(
                MONITOR,
                Request::CreateToplevel {
                    surface: 1,
                    size: Size::new(400, 300),
                },
            )
            .unwrap();
        self.drain();
        let (serial, extent) = self.configure.unwrap();
        self.compositor
            .scene
            .handle(
                MONITOR,
                Request::AttachBuffer {
                    buffer: 1,
                    format: Format::Xrgb8888,
                    extent,
                    stride: extent.width * 4,
                    offset: 0,
                },
            )
            .unwrap();
        self.compositor
            .scene
            .handle(
                MONITOR,
                Request::Submit {
                    surface: 1,
                    sequence: 1,
                    buffer: 1,
                    configure: serial,
                    extent,
                },
            )
            .unwrap();
        self.compositor.scene.begin_frame();
        self.compositor.scene.end_frame(1, Clock::Synthetic);
        self.drain();
        assert!(self.compositor.scene.client_shown(MONITOR));
    }

    fn turn(&mut self) {
        self.activity.serve(&mut self.compositor, &[], None);
        self.drain();
    }

    fn ask(&mut self, client: ClientId, text: Value, with_input: bool) -> u32 {
        self.serial += 1;
        self.activity.handle(
            &mut self.compositor.scene,
            SystemRequest {
                client,
                serial: self.serial,
                with_input,
                text: text.to_string(),
            },
        );
        self.turn();
        self.serial
    }

    /// The answer to `serial`, waiting for it at most five seconds.
    fn answer(&mut self, client: ClientId, serial: u32) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some((_, _, text)) = self
                .told
                .iter()
                .find(|(c, s, _)| *c == client && *s == serial)
            {
                return serde_json::from_str(text).unwrap();
            }
            assert!(Instant::now() < deadline, "no answer to {serial}");
            std::thread::sleep(Duration::from_millis(10));
            self.turn();
        }
    }

    /// The samples `client` was told, unasked.
    fn samples(&self, client: ClientId) -> Vec<Value> {
        self.told
            .iter()
            .filter(|(c, s, _)| *c == client && *s == 0)
            .map(|(_, _, text)| serde_json::from_str::<Value>(text).unwrap()["activity"].clone())
            .collect()
    }

    /// Turns until `client` was told `count` samples, at most `within`; when it was.
    fn until_samples(&mut self, client: ClientId, count: usize, within: Duration) -> Instant {
        let deadline = Instant::now() + within;
        loop {
            self.turn();
            if self.samples(client).len() >= count {
                return Instant::now();
            }
            assert!(
                Instant::now() < deadline,
                "not {count} samples in {within:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Turns for `time`.
    fn run(&mut self, time: Duration) {
        let until = Instant::now() + time;
        while Instant::now() < until {
            self.turn();
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// How many times the part's thread was switched out since it started: each time it woke and
/// slept again.
fn thread_switches(activity: &Activity) -> u64 {
    let status =
        std::fs::read_to_string(format!("/proc/self/task/{}/status", activity.thread_id()))
            .unwrap();
    status
        .lines()
        .filter(|line| line.contains("ctxt_switches"))
        .filter_map(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
        .sum()
}

#[test]
fn only_activity_monitor_asks_and_strictly() {
    let mut session = Session::new();
    let serial = session.ask(OTHER, json!({"activityFollow": {"view": "cpu"}}), false);
    assert_eq!(
        session.answer(OTHER, serial),
        json!({"refused": {"reason": "notPermitted"}})
    );
    let serial = session.ask(MONITOR, json!({"activityFollow": {"view": "gpu"}}), false);
    assert_eq!(
        session.answer(MONITOR, serial),
        json!({"refused": {"reason": "invalidRequest"}})
    );
    let serial = session.ask(
        MONITOR,
        json!({"activityQuit": {"pid": 1, "started": 1, "force": false, "extra": 1}}),
        true,
    );
    assert_eq!(
        session.answer(MONITOR, serial),
        json!({"refused": {"reason": "invalidRequest"}})
    );
    // Quitting cites an input.
    let serial = session.ask(
        MONITOR,
        json!({"activityQuit": {"pid": 1, "started": 1, "force": false}}),
        false,
    );
    assert_eq!(
        session.answer(MONITOR, serial),
        json!({"refused": {"reason": "needsInput"}})
    );
}

#[test]
fn a_shown_follower_is_sampled_every_two_seconds_and_a_hidden_one_never() {
    let mut session = Session::new();
    // Following without a window shown: answered, nothing sampled.
    let serial = session.ask(MONITOR, json!({"activityFollow": {"view": "cpu"}}), false);
    assert_eq!(
        session.answer(MONITOR, serial),
        json!({"activityFollow": {}})
    );
    session.run(Duration::from_millis(800));
    assert!(session.samples(MONITOR).is_empty());
    assert_eq!(session.activity.counts(), (0, 0));

    // Shown: a sample soon (rates over half a second), then one every two seconds.
    session.window();
    let shown = Instant::now();
    let first_at = session.until_samples(MONITOR, 1, Duration::from_secs(5));
    assert!(first_at - shown < Duration::from_millis(1500));
    let samples = session.samples(MONITOR);
    let first = &samples[0];
    assert_eq!(first["view"], "cpu");
    assert!(first["interval"].as_f64().unwrap() >= 0.45);
    let me = first["processes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pid"] == std::process::id())
        .expect("this process listed")
        .clone();
    assert_eq!(me["own"], true);
    assert!(me["threads"].as_u64().unwrap() >= 2);
    assert!(first["system"]["memory"]["total"].as_u64().unwrap() > 0);
    let second_at = session.until_samples(MONITOR, 2, Duration::from_secs(5));
    assert!(second_at - first_at >= Duration::from_millis(1900));
    assert!((session.samples(MONITOR)[1]["interval"].as_f64().unwrap() - 2.0).abs() < 0.5);
    // Another application is told none of it.
    assert!(session.samples(OTHER).is_empty());

    // Minimized: no sample, and the thread does not wake at all.
    session
        .compositor
        .scene
        .minimize(oxyos_compositor_core::SurfaceKey {
            client: MONITOR,
            id: 1,
        });
    session.run(Duration::from_millis(100));
    let (taken, sent) = session.activity.counts();
    let switches = thread_switches(&session.activity);
    session.run(Duration::from_secs(5));
    assert_eq!(session.activity.counts(), (taken, sent));
    assert_eq!(
        thread_switches(&session.activity),
        switches,
        "the thread woke while hidden"
    );
    assert_eq!(session.samples(MONITOR).len(), 2);

    // Shown again: sampled again; stopped: never again.
    session
        .compositor
        .scene
        .restore(oxyos_compositor_core::SurfaceKey {
            client: MONITOR,
            id: 1,
        });
    session.compositor.scene.begin_frame();
    session.compositor.scene.end_frame(2, Clock::Synthetic);
    session.until_samples(MONITOR, 3, Duration::from_secs(5));
    let serial = session.ask(MONITOR, json!({"activityFollow": {"view": null}}), false);
    assert_eq!(
        session.answer(MONITOR, serial),
        json!({"activityFollow": {}})
    );
    // The thread is told once, and then waits on nothing but its jobs.
    session.run(Duration::from_millis(100));
    let switches = thread_switches(&session.activity);
    session.run(Duration::from_millis(2500));
    assert_eq!(session.samples(MONITOR).len(), 3);
    assert_eq!(thread_switches(&session.activity), switches);
}

#[test]
fn the_displays_off_sample_nothing() {
    let mut session = Session::new();
    session.window();
    session.ask(
        MONITOR,
        json!({"activityFollow": {"view": "memory"}}),
        false,
    );
    session.compositor.set_displays_on(false);
    session.run(Duration::from_millis(1500));
    assert!(session.samples(MONITOR).is_empty());
}

#[test]
fn a_process_of_the_persons_is_inspected_and_quit_and_the_session_never() {
    let mut session = Session::new();
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let pid = child.id();
    let started = {
        let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let after = &text[text.rfind(')').unwrap() + 1..];
        after
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    let serial = session.ask(
        MONITOR,
        json!({"activityInspect": {"pid": pid, "started": started}}),
        false,
    );
    let inspected = session.answer(MONITOR, serial)["activityInspect"].clone();
    assert_eq!(inspected["name"], "sleep");
    assert_eq!(inspected["own"], true);
    assert_eq!(inspected["arguments"], json!(["sleep", "30"]));
    // The session itself: never.
    let me = std::process::id();
    let my_start = {
        let text = std::fs::read_to_string("/proc/self/stat").unwrap();
        let after = &text[text.rfind(')').unwrap() + 1..];
        after
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    let serial = session.ask(
        MONITOR,
        json!({"activityQuit": {"pid": me, "started": my_start, "force": true}}),
        true,
    );
    assert_eq!(
        session.answer(MONITOR, serial),
        json!({"failed": {"reason": "notPermitted"}})
    );
    // Quit: it ends by SIGTERM, once.
    let serial = session.ask(
        MONITOR,
        json!({"activityQuit": {"pid": pid, "started": started, "force": false}}),
        true,
    );
    assert_eq!(session.answer(MONITOR, serial), json!({"activityQuit": {}}));
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(child.wait().unwrap().signal(), Some(15));
    let serial = session.ask(
        MONITOR,
        json!({"activityInspect": {"pid": pid, "started": started}}),
        false,
    );
    assert_eq!(
        session.answer(MONITOR, serial),
        json!({"failed": {"reason": "notFound"}})
    );
}

#[test]
fn an_application_not_running_is_not_found() {
    let mut session = Session::new();
    let serial = session.ask(
        MONITOR,
        json!({"activityQuitApp": {"id": "so.oxy.inbox", "force": false}}),
        true,
    );
    assert_eq!(
        session.answer(MONITOR, serial),
        json!({"failed": {"reason": "notFound"}})
    );
}
