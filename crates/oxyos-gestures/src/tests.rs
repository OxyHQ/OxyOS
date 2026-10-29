//! Each test drives the orchestrator as the host does and checks what JavaScript would be sent,
//! in order, against the Android code it ports.

use super::*;
use crate::handler::Event;
use crate::velocity::Tracker;
use std::collections::BTreeMap;

const ROOT: ViewTag = 1;
const PARENT: ViewTag = 5;
const VIEW: ViewTag = 10;

struct World {
    o: Orchestrator,
    frames: BTreeMap<ViewTag, Frame>,
    down: Vec<Pointer>,
    time: u64,
    hits: Vec<ViewTag>,
    /// What the last event sent, for its data.
    last: Vec<Output>,
}

impl World {
    fn new() -> Self {
        let frame = |x, y, width, height| Frame {
            x,
            y,
            width,
            height,
        };
        let frames = BTreeMap::from([
            (ROOT, frame(0.0, 0.0, 400.0, 400.0)),
            (PARENT, frame(0.0, 0.0, 200.0, 200.0)),
            (VIEW, frame(50.0, 50.0, 100.0, 100.0)),
        ]);
        Self {
            o: Orchestrator::new(),
            frames,
            down: Vec::new(),
            time: 1000,
            hits: vec![VIEW, PARENT, ROOT],
            last: Vec::new(),
        }
    }

    fn handler(&mut self, name: &str, tag: Tag, view: ViewTag, config: &[(&str, Value)]) {
        assert_eq!(self.o.create(name, tag, config), Ok(Vec::new()));
        self.o.attach(tag, view).expect("attached");
    }

    fn after(&mut self, ms: u64) -> &mut Self {
        self.time += ms;
        self
    }

    fn send(&mut self, action: Action, index: usize) -> Vec<String> {
        let touch = Touch {
            action,
            index,
            pointers: self.down.clone(),
            time: self.time,
            pointer_type: PointerType::Mouse,
        };
        let frames = self.frames.clone();
        let out = self
            .o
            .touch(&touch, &self.hits, &|view| frames.get(&view).copied());
        self.last = out.clone();
        summary(&out)
    }

    fn press(&mut self, id: u32, x: f32, y: f32) -> Vec<String> {
        let action = if self.down.is_empty() {
            Action::Down
        } else {
            Action::PointerDown
        };
        self.down.push(Pointer { id, x, y });
        self.send(action, self.down.len() - 1)
    }

    fn drag(&mut self, id: u32, x: f32, y: f32) -> Vec<String> {
        let pointer = self.down.iter_mut().find(|p| p.id == id).expect("down");
        (pointer.x, pointer.y) = (x, y);
        let index = self.down.iter().position(|p| p.id == id).expect("down");
        self.send(Action::Move, index)
    }

    fn lift(&mut self, id: u32) -> Vec<String> {
        let index = self.down.iter().position(|p| p.id == id).expect("down");
        let action = if self.down.len() == 1 {
            Action::Up
        } else {
            Action::PointerUp
        };
        let out = self.send(action, index);
        self.down.remove(index);
        out
    }

    fn tick(&mut self) -> Vec<String> {
        let out = self.o.tick(self.time);
        self.last = out.clone();
        summary(&out)
    }

    /// The gesture is over and nothing of it is left.
    fn settled(&self) {
        assert!(self.o.recorded().is_empty(), "{:?}", self.o.recorded());
        assert_eq!(self.o.next_deadline(), None);
        assert!(!self.o.intercepting());
    }
}

fn summary(out: &[Output]) -> Vec<String> {
    out.iter()
        .map(|o| match o {
            Output::StateChange {
                tag,
                state,
                old_state,
                ..
            } => format!("{tag} {old_state:?}>{state:?}"),
            Output::Update { tag, .. } => format!("{tag} update"),
            Output::Intercept => "intercept".into(),
            Output::CancelResponder { tag } => format!("{tag} cancel-responder"),
        })
        .collect()
}

fn data(out: &Output) -> &Data {
    match out {
        Output::StateChange { data, .. } | Output::Update { data, .. } => data,
        _ => panic!("no data: {out:?}"),
    }
}

fn n(value: f64) -> Value {
    Value::Number(value)
}

const ACTIVATED: [&str; 2] = ["intercept", "1 cancel-responder"];

fn activated(tail: &[&str]) -> Vec<String> {
    ACTIVATED
        .iter()
        .chain(tail)
        .map(|s| s.to_string())
        .collect()
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_pan_activates_past_the_touch_slop_and_reports_translation_from_there() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    assert_eq!(w.press(0, 60.0, 60.0), strings(&["1 Undetermined>Began"]));
    assert_eq!(w.after(10).drag(0, 65.0, 60.0), strings(&[]));
    assert_eq!(
        w.after(10).drag(0, 69.0, 60.0),
        activated(&["1 Began>Active", "1 update"])
    );
    // Translation counts from where it activated, as on Android.
    assert_eq!(data(&w.last[3]).translation, Some((0.0, 0.0)));
    assert_eq!(w.after(10).drag(0, 79.0, 62.0), strings(&["1 update"]));
    let update = data(&w.last[0]).clone();
    assert_eq!(update.translation, Some((10.0, 2.0)));
    assert_eq!(
        update.position,
        Some(Position {
            x: 29.0,
            y: 12.0,
            absolute_x: 79.0,
            absolute_y: 62.0
        })
    );
    assert_eq!(update.number_of_pointers, 1);
    assert_eq!(update.pointer_type, PointerType::Mouse);
    assert_eq!(w.after(10).lift(0), strings(&["1 Active>End"]));
    w.settled();
}

#[test]
fn a_pan_lifted_before_it_activates_fails() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    w.press(0, 60.0, 60.0);
    w.after(10).drag(0, 64.0, 60.0);
    assert_eq!(w.after(10).lift(0), strings(&["1 Began>Failed"]));
    w.settled();
}

#[test]
fn the_same_gesture_twice_sends_the_same_events() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    let mut runs = Vec::new();
    for _ in 0..2 {
        let mut run = w.press(0, 60.0, 60.0);
        run.extend(w.after(10).drag(0, 80.0, 60.0));
        run.extend(w.after(10).drag(0, 90.0, 60.0));
        run.extend(w.after(10).lift(0));
        w.settled();
        runs.push(run);
    }
    assert_eq!(runs[0], runs[1]);
    assert_eq!(runs[0].iter().filter(|e| *e == "intercept").count(), 1);
}

#[test]
fn a_pans_velocity_is_its_pointers() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    w.press(0, 60.0, 60.0);
    for step in 1..=6 {
        w.after(10).drag(0, 60.0 + 10.0 * step as f32, 60.0);
    }
    // Ten pixels every ten milliseconds.
    let (vx, vy) = data(w.last.last().expect("update"))
        .velocity
        .expect("a pan's");
    assert!((vx - 1000.0).abs() < 1.0, "{vx}");
    assert!(vy.abs() < 1.0, "{vy}");
    // Still for more than 40 ms: it has stopped.
    w.after(50).lift(0);
    let end = data(&w.last[0]).clone();
    assert_eq!(end.velocity, Some((0.0, 0.0)));
}

#[test]
fn a_second_pointer_keeps_a_pans_translation() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    w.press(0, 60.0, 60.0);
    w.after(10).drag(0, 75.0, 60.0);
    w.after(10).drag(0, 85.0, 60.0);
    assert_eq!(data(&w.last[0]).translation, Some((10.0, 0.0)));
    // The pan follows the last pointer down from where it is, its translation kept.
    assert_eq!(w.after(10).press(1, 100.0, 100.0), strings(&["1 update"]));
    assert_eq!(data(&w.last[0]).translation, Some((10.0, 0.0)));
    assert_eq!(data(&w.last[0]).number_of_pointers, 2);
    w.after(10).drag(1, 110.0, 100.0);
    assert_eq!(data(&w.last[0]).translation, Some((20.0, 0.0)));
    assert_eq!(w.after(10).lift(1), strings(&["1 update"]));
    assert_eq!(data(&w.last[0]).number_of_pointers, 1);
    assert_eq!(w.after(10).lift(0), strings(&["1 Active>End"]));
    w.settled();
}

#[test]
fn custom_activation_criteria_drop_the_default_distance() {
    let mut w = World::new();
    w.handler(
        "PanGestureHandler",
        1,
        VIEW,
        &[("activeOffsetXEnd", n(20.0))],
    );
    w.press(0, 60.0, 60.0);
    // Far, but not to the right: no activation.
    assert_eq!(w.after(10).drag(0, 60.0, 100.0), strings(&[]));
    assert_eq!(
        w.after(10).drag(0, 81.0, 100.0),
        activated(&["1 Began>Active", "1 update"])
    );
    // An explicit distance stays.
    let mut w = World::new();
    w.handler(
        "PanGestureHandler",
        1,
        VIEW,
        &[("activeOffsetXEnd", n(20.0)), ("minDist", n(30.0))],
    );
    w.press(0, 60.0, 60.0);
    assert_eq!(
        w.after(10).drag(0, 60.0, 91.0),
        activated(&["1 Began>Active", "1 update"])
    );
}

#[test]
fn a_fail_offset_fails_a_pan() {
    let mut w = World::new();
    w.handler(
        "PanGestureHandler",
        1,
        VIEW,
        &[("activeOffsetXEnd", n(10.0)), ("failOffsetYEnd", n(5.0))],
    );
    w.press(0, 60.0, 60.0);
    assert_eq!(
        w.after(10).drag(0, 62.0, 66.0),
        strings(&["1 Began>Failed"])
    );
    w.after(10).lift(0);
    w.settled();
}

#[test]
fn a_pan_after_a_long_press_activates_on_its_timer() {
    let mut w = World::new();
    w.handler(
        "PanGestureHandler",
        1,
        VIEW,
        &[("activateAfterLongPress", n(300.0))],
    );
    w.press(0, 60.0, 60.0);
    assert_eq!(w.o.next_deadline(), Some(1300));
    assert_eq!(w.after(299).tick(), strings(&[]));
    assert_eq!(w.after(1).tick(), activated(&["1 Began>Active"]));
    // Moving too far before then fails it.
    let mut w = World::new();
    w.handler(
        "PanGestureHandler",
        1,
        VIEW,
        &[("activateAfterLongPress", n(300.0))],
    );
    w.press(0, 60.0, 60.0);
    assert_eq!(
        w.after(100).drag(0, 70.0, 60.0),
        strings(&["1 Began>Failed"])
    );
    assert_eq!(w.o.next_deadline(), None);
}

#[test]
fn a_tap_ends_on_release() {
    let mut w = World::new();
    w.handler("TapGestureHandler", 1, VIEW, &[]);
    assert_eq!(w.press(0, 60.0, 60.0), strings(&["1 Undetermined>Began"]));
    assert_eq!(
        w.after(100).lift(0),
        activated(&["1 Began>Active", "1 Active>End"])
    );
    w.settled();
}

#[test]
fn a_tap_held_too_long_fails() {
    let mut w = World::new();
    w.handler("TapGestureHandler", 1, VIEW, &[]);
    w.press(0, 60.0, 60.0);
    assert_eq!(w.after(499).tick(), strings(&[]));
    assert_eq!(w.after(1).tick(), strings(&["1 Began>Failed"]));
    // Its release then sends nothing.
    assert_eq!(w.after(10).lift(0), strings(&[]));
    w.settled();
}

#[test]
fn a_tap_leaving_its_view_fails() {
    let mut w = World::new();
    w.handler("TapGestureHandler", 1, VIEW, &[]);
    w.press(0, 60.0, 60.0);
    assert_eq!(
        w.after(10).drag(0, 40.0, 60.0),
        strings(&["1 Began>Failed"])
    );
    w.after(10).lift(0);
    w.settled();
}

#[test]
fn a_double_tap_cancels_the_single_tap_waiting_for_it() {
    let mut w = World::new();
    w.handler("TapGestureHandler", 1, VIEW, &[]);
    w.handler("TapGestureHandler", 2, VIEW, &[("numberOfTaps", n(2.0))]);
    w.o.relate(
        1,
        Relations {
            wait_for: vec![2],
            ..Relations::default()
        },
    )
    .expect("related");
    assert_eq!(
        w.press(0, 60.0, 60.0),
        strings(&["1 Undetermined>Began", "2 Undetermined>Began"])
    );
    // The single tap is recognized but waits.
    assert_eq!(w.after(50).lift(0), strings(&[]));
    assert_eq!(w.after(100).press(1, 62.0, 60.0), strings(&[]));
    assert_eq!(
        w.after(50).lift(1),
        strings(&[
            "intercept",
            "2 cancel-responder",
            "2 Began>Active",
            "1 Began>Cancelled",
            "2 Active>End"
        ])
    );
    w.settled();
}

#[test]
fn a_single_tap_waiting_for_a_double_one_ends_when_it_fails() {
    let mut w = World::new();
    w.handler("TapGestureHandler", 1, VIEW, &[]);
    w.handler("TapGestureHandler", 2, VIEW, &[("numberOfTaps", n(2.0))]);
    w.o.relate(
        1,
        Relations {
            wait_for: vec![2],
            ..Relations::default()
        },
    )
    .expect("related");
    w.press(0, 60.0, 60.0);
    assert_eq!(w.after(50).lift(0), strings(&[]));
    assert_eq!(w.o.next_deadline(), Some(1250));
    // Every pointer is up: there is no gesture to take from React Native.
    assert_eq!(
        w.after(200).tick(),
        strings(&[
            "1 cancel-responder",
            "1 Began>Active",
            "1 Active>End",
            "2 Began>Failed"
        ])
    );
    w.settled();
}

#[test]
fn a_long_press_activates_on_its_timer() {
    let mut w = World::new();
    w.handler("LongPressGestureHandler", 1, VIEW, &[]);
    assert_eq!(w.press(0, 60.0, 60.0), strings(&["1 Undetermined>Began"]));
    // Within ten pixels it holds.
    assert_eq!(w.after(100).drag(0, 66.0, 66.0), strings(&[]));
    assert_eq!(w.after(400).tick(), activated(&["1 Began>Active"]));
    assert_eq!(data(&w.last[2]).duration, Some(500));
    assert_eq!(w.after(100).lift(0), strings(&["1 Active>End"]));
    assert_eq!(data(&w.last[0]).duration, Some(600));
    w.settled();
}

#[test]
fn a_long_press_moved_or_released_early_fails() {
    let mut w = World::new();
    w.handler("LongPressGestureHandler", 1, VIEW, &[]);
    w.press(0, 60.0, 60.0);
    assert_eq!(
        w.after(100).drag(0, 71.0, 60.0),
        strings(&["1 Began>Failed"])
    );
    w.after(10).lift(0);
    w.settled();
    w.press(0, 60.0, 60.0);
    assert_eq!(w.after(100).lift(0), strings(&["1 Began>Failed"]));
    assert_eq!(w.after(500).tick(), strings(&[]));
    w.settled();
}

#[test]
fn a_fast_move_flings() {
    let mut w = World::new();
    w.handler("FlingGestureHandler", 1, VIEW, &[]);
    assert_eq!(w.press(0, 60.0, 60.0), strings(&["1 Undetermined>Began"]));
    assert_eq!(w.after(10).drag(0, 90.0, 60.0), strings(&[]));
    assert_eq!(
        w.after(10).drag(0, 120.0, 60.0),
        activated(&["1 Began>Active", "1 Active>End"])
    );
    w.after(10).lift(0);
    w.settled();
    // The wrong way, it fails on release.
    w.press(0, 140.0, 60.0);
    w.after(10).drag(0, 110.0, 60.0);
    w.after(10).drag(0, 80.0, 60.0);
    assert_eq!(w.after(10).lift(0), strings(&["1 Began>Failed"]));
    w.settled();
}

#[test]
fn a_child_activating_cancels_its_parent() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    w.handler("PanGestureHandler", 2, PARENT, &[]);
    assert_eq!(
        w.press(0, 60.0, 60.0),
        strings(&["1 Undetermined>Began", "2 Undetermined>Began"])
    );
    assert_eq!(
        w.after(10).drag(0, 70.0, 60.0),
        strings(&[
            "2 Began>Cancelled",
            "intercept",
            "1 cancel-responder",
            "1 Began>Active",
            "1 update"
        ])
    );
    assert_eq!(w.after(10).lift(0), strings(&["1 Active>End"]));
    w.settled();
}

#[test]
fn a_parent_activating_first_cancels_its_child() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    w.handler(
        "PanGestureHandler",
        2,
        PARENT,
        &[("activeOffsetXEnd", n(2.0))],
    );
    w.press(0, 60.0, 60.0);
    assert_eq!(
        w.after(10).drag(0, 63.0, 60.0),
        strings(&[
            "1 Began>Cancelled",
            "intercept",
            "2 cancel-responder",
            "2 Began>Active",
            "2 update"
        ])
    );
    w.after(10).lift(0);
    w.settled();
}

#[test]
fn simultaneous_handlers_both_activate() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    w.handler("PanGestureHandler", 2, PARENT, &[]);
    w.o.relate(
        1,
        Relations {
            simultaneous_with: vec![2],
            ..Relations::default()
        },
    )
    .expect("related");
    w.press(0, 60.0, 60.0);
    assert_eq!(
        w.after(10).drag(0, 70.0, 60.0),
        strings(&[
            "intercept",
            "1 cancel-responder",
            "1 Began>Active",
            "1 update",
            "2 cancel-responder",
            "2 Began>Active",
            "2 update"
        ])
    );
    // Android delivers to the handler activated last first.
    assert_eq!(
        w.after(10).lift(0),
        strings(&["2 Active>End", "1 Active>End"])
    );
    w.settled();
}

#[test]
fn a_pan_blocked_by_a_tap_waits_for_it_to_fail_and_starts_from_there() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    w.handler("TapGestureHandler", 2, VIEW, &[]);
    w.o.relate(
        2,
        Relations {
            blocks: vec![1],
            ..Relations::default()
        },
    )
    .expect("related");
    w.press(0, 60.0, 60.0);
    // Recognized, but waiting: nothing is sent, and moves are not given to it.
    assert_eq!(w.after(10).drag(0, 70.0, 60.0), strings(&[]));
    assert_eq!(w.after(10).drag(0, 75.0, 60.0), strings(&[]));
    assert_eq!(
        w.after(480).tick(),
        activated(&["1 Began>Active", "2 Began>Failed"])
    );
    assert_eq!(w.after(10).drag(0, 80.0, 60.0), strings(&["1 update"]));
    assert_eq!(data(&w.last[0]).translation, Some((0.0, 0.0)));
    w.after(10).drag(0, 90.0, 60.0);
    assert_eq!(data(&w.last[0]).translation, Some((10.0, 0.0)));
    assert_eq!(w.after(10).lift(0), strings(&["1 Active>End"]));
    w.settled();
}

#[test]
fn a_tap_succeeding_cancels_the_pan_it_blocks() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    w.handler("TapGestureHandler", 2, VIEW, &[]);
    w.o.relate(
        2,
        Relations {
            blocks: vec![1],
            ..Relations::default()
        },
    )
    .expect("related");
    w.press(0, 60.0, 60.0);
    assert_eq!(
        w.after(10).lift(0),
        strings(&[
            "1 Began>Failed",
            "intercept",
            "2 cancel-responder",
            "2 Began>Active",
            "2 Active>End"
        ])
    );
    w.settled();
}

#[test]
fn a_manual_gesture_is_moved_by_javascript() {
    let mut w = World::new();
    w.handler("ManualGestureHandler", 1, VIEW, &[]);
    assert_eq!(w.press(0, 60.0, 60.0), strings(&["1 Undetermined>Began"]));
    assert_eq!(data(&w.last[0]).position, None);
    assert_eq!(
        summary(&w.o.set_state(1, State::Active)),
        activated(&["1 Began>Active"])
    );
    assert_eq!(w.after(10).drag(0, 61.0, 60.0), strings(&["1 update"]));
    assert_eq!(
        summary(&w.o.set_state(1, State::End)),
        strings(&["1 Active>End"])
    );
    assert_eq!(w.after(10).lift(0), strings(&[]));
    w.settled();
    // One that has not begun is not activated.
    assert_eq!(summary(&w.o.set_state(1, State::Active)), strings(&[]));
}

#[test]
fn manual_activation_waits_for_javascript() {
    let mut w = World::new();
    w.handler(
        "PanGestureHandler",
        1,
        VIEW,
        &[("manualActivation", Value::Bool(true))],
    );
    w.press(0, 60.0, 60.0);
    assert_eq!(w.after(10).drag(0, 90.0, 60.0), strings(&[]));
    assert_eq!(
        summary(&w.o.set_state(1, State::Active)),
        activated(&["1 Began>Active"])
    );
}

#[test]
fn a_native_view_handler_begins_and_fails() {
    let mut w = World::new();
    w.handler("NativeViewGestureHandler", 1, VIEW, &[]);
    assert_eq!(
        w.press(0, 60.0, 60.0),
        strings(&["1 Undetermined>Began", "1 Began>Failed"])
    );
    assert_eq!(data(&w.last[0]).pointer_inside, Some(true));
    w.after(10).lift(0);
    w.settled();
}

#[test]
fn a_handler_not_begun_is_not_reported_cancelled() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[("minPointers", n(2.0))]);
    assert_eq!(w.press(0, 60.0, 60.0), strings(&[]));
    assert_eq!(w.o.recorded(), &[1]);
    assert_eq!(w.send(Action::Cancel, 0), strings(&[]));
    w.down.clear();
    w.settled();
}

#[test]
fn a_cancelled_stream_cancels_every_handler() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    w.handler("TapGestureHandler", 2, VIEW, &[]);
    w.press(0, 60.0, 60.0);
    w.after(10).drag(0, 70.0, 60.0);
    assert_eq!(w.send(Action::Cancel, 0), strings(&["1 Active>Cancelled"]));
    w.down.clear();
    w.settled();
}

#[test]
fn a_handler_whose_view_is_gone_is_cancelled() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    w.press(0, 60.0, 60.0);
    w.frames.remove(&VIEW);
    assert_eq!(
        w.after(10).drag(0, 70.0, 60.0),
        strings(&["1 Began>Cancelled"])
    );
    w.after(10).lift(0);
    w.settled();
}

#[test]
fn a_disabled_handler_takes_no_part_and_disabling_one_cancels_it() {
    let mut w = World::new();
    w.handler(
        "TapGestureHandler",
        1,
        VIEW,
        &[("enabled", Value::Bool(false))],
    );
    assert_eq!(w.press(0, 60.0, 60.0), strings(&[]));
    w.lift(0);
    w.settled();
    w.o.configure(1, &[("enabled", Value::Bool(true))], false)
        .expect("configured");
    w.press(0, 60.0, 60.0);
    w.o.configure(1, &[("enabled", Value::Bool(false))], false)
        .expect("configured");
    assert_eq!(w.o.state(1), Some(State::Cancelled));
    assert!(w.o.recorded().is_empty());
}

#[test]
fn hit_slop_reaches_past_the_view() {
    let mut w = World::new();
    w.handler("TapGestureHandler", 1, VIEW, &[("hitSlop", n(20.0))]);
    let frame = w.frames[&VIEW];
    assert!(w.o.reaches(VIEW, frame, 35.0, 60.0));
    assert!(!w.o.reaches(VIEW, frame, 25.0, 60.0));
    assert_eq!(w.press(0, 35.0, 60.0), strings(&["1 Undetermined>Began"]));
    w.lift(0);
    // Without it, the point is outside.
    let mut w = World::new();
    w.handler("TapGestureHandler", 1, VIEW, &[]);
    assert!(!w.o.reaches(VIEW, frame, 35.0, 60.0));
    assert_eq!(w.press(0, 35.0, 60.0), strings(&[]));
}

#[test]
fn a_handler_attached_elsewhere_leaves_its_old_view() {
    let mut w = World::new();
    w.handler("TapGestureHandler", 1, VIEW, &[]);
    w.o.attach(1, PARENT).expect("attached");
    assert_eq!(w.press(0, 160.0, 160.0), strings(&["1 Undetermined>Began"]));
    assert_eq!(w.o.recorded(), &[1]);
    w.lift(0);
    w.o.drop_handler(1);
    assert_eq!(w.press(0, 160.0, 160.0), strings(&[]));
}

#[test]
fn what_is_not_ported_is_refused_or_returned() {
    let mut o = Orchestrator::new();
    assert_eq!(
        o.create("PinchGestureHandler", 1, &[]),
        Err(Refused::Unsupported("PinchGestureHandler".into()))
    );
    assert_eq!(
        o.create(
            "PanGestureHandler",
            1,
            &[
                ("minDist", n(20.0)),
                ("bogus", n(1.0)),
                ("needsPointerData", Value::Bool(true)),
                ("needsPointerData", Value::Bool(false)),
                ("mouseButton", n(2.0)),
            ]
        ),
        Ok(strings(&["bogus", "needsPointerData", "mouseButton"]))
    );
    assert_eq!(
        o.create("PanGestureHandler", 1, &[]),
        Err(Refused::Exists(1))
    );
    assert_eq!(o.attach(9, VIEW), Err(Refused::Unknown(9)));
}

#[test]
fn velocity_follows_the_first_pointer_down_then_the_first_left() {
    let event = |action, index, pointers: &[(u32, f32)], time| Event {
        action,
        index,
        pointers: pointers
            .iter()
            .map(|&(id, x)| Pointer { id, x, y: 0.0 })
            .collect(),
        time,
    };
    let mut tracker = Tracker::default();
    // Pointer 0 moves at 1000 px/s, pointer 1 at -500 px/s.
    let second = |t: u64| 100.0 - 0.5 * (t as f32 - 5.0);
    tracker.add(&event(Action::Down, 0, &[(0, 0.0)], 0));
    tracker.add(&event(Action::PointerDown, 1, &[(0, 5.0), (1, 100.0)], 5));
    for t in [15u64, 25, 35, 45] {
        tracker.add(&event(Action::Move, 0, &[(0, t as f32), (1, second(t))], t));
    }
    let (vx, _) = tracker.velocity(45);
    assert!((vx - 1000.0).abs() < 1.0, "{vx}");
    tracker.add(&event(
        Action::PointerUp,
        0,
        &[(0, 46.0), (1, second(46))],
        46,
    ));
    for t in [55u64, 65, 75] {
        tracker.add(&event(Action::Move, 0, &[(1, second(t))], t));
    }
    let (vx, _) = tracker.velocity(75);
    assert!((vx + 500.0).abs() < 1.0, "{vx}");
}

#[test]
fn a_handler_waiting_is_not_given_moves() {
    let mut w = World::new();
    w.handler("LongPressGestureHandler", 1, VIEW, &[]);
    w.handler(
        "TapGestureHandler",
        2,
        VIEW,
        &[("maxDurationMs", n(1000.0))],
    );
    w.o.relate(
        1,
        Relations {
            wait_for: vec![2],
            ..Relations::default()
        },
    )
    .expect("related");
    w.press(0, 60.0, 60.0);
    // The long press is recognized, and waits for the tap.
    assert_eq!(w.after(500).tick(), strings(&[]));
    // Moved farther than a long press may: it would fail, but it waits and is not told.
    assert_eq!(w.after(100).drag(0, 80.0, 60.0), strings(&[]));
    assert_eq!(
        w.after(400).tick(),
        activated(&["1 Began>Active", "2 Began>Failed"])
    );
}

#[test]
fn a_child_let_to_activate_under_an_active_parent_is_cancelled() {
    let mut w = World::new();
    w.handler("PanGestureHandler", 1, VIEW, &[]);
    w.handler(
        "PanGestureHandler",
        2,
        PARENT,
        &[("activeOffsetXEnd", n(20.0))],
    );
    w.handler("TapGestureHandler", 3, VIEW, &[]);
    w.o.relate(
        1,
        Relations {
            wait_for: vec![3],
            ..Relations::default()
        },
    )
    .expect("related");
    w.press(0, 60.0, 60.0);
    // The child pan is recognized and waits for the tap.
    assert_eq!(w.after(10).drag(0, 70.0, 60.0), strings(&[]));
    // The parent activates and cancels the tap; the child, let go, finds its parent active.
    assert_eq!(
        w.after(10).drag(0, 85.0, 60.0),
        strings(&[
            "1 Began>Cancelled",
            "3 Began>Cancelled",
            "intercept",
            "2 cancel-responder",
            "2 Began>Active",
            "2 update"
        ])
    );
    w.after(10).lift(0);
    w.settled();
}

#[test]
fn velocity_forgets_movement_before_a_pause() {
    let event = |action, x, time| Event {
        action,
        index: 0,
        pointers: vec![Pointer { id: 0, x, y: 0.0 }],
        time,
    };
    let mut tracker = Tracker::default();
    tracker.add(&event(Action::Down, 0.0, 0));
    for t in [10u64, 20, 30] {
        tracker.add(&event(Action::Move, t as f32, t));
    }
    // Still for 45 ms, then 2000 px/s.
    tracker.add(&event(Action::Move, 30.0, 75));
    tracker.add(&event(Action::Move, 50.0, 85));
    let (vx, _) = tracker.velocity(85);
    assert!((vx - 2000.0).abs() < 1.0, "{vx}");
}
