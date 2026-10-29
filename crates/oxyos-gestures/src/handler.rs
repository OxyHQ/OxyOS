//! The handlers: what every handler has (`GestureHandler`), and the pan, tap, long-press, fling,
//! manual, native-view, pinch and rotation handlers' own recognition (`onHandle`), timers and
//! settings, with the pinch's and the rotation's detectors.

use crate::velocity::Tracker;
use crate::{
    Action, Data, Frame, Orchestrator, Pointer, PointerType, Position, State, Tag, Touch,
    TouchPoint, ViewTag,
};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

/// Android's touch slop, in density-independent pixels (`ViewConfiguration`).
pub(crate) const TOUCH_SLOP: f32 = 8.0;
/// How far a long press may move, in density-independent pixels.
const LONG_PRESS_MAX_DIST: f32 = 10.0;

/// A setting's value, as JavaScript sends it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    /// A list of numbers, a hit slop's (`[left, top, right, bottom, width, height]`, none for an
    /// edge not set).
    List(Vec<Option<f64>>),
}

impl Value {
    fn bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Number(n) => Some(*n != 0.0),
            _ => None,
        }
    }

    fn number(&self) -> Option<f32> {
        match self {
            Value::Number(n) => Some(*n as f32),
            _ => None,
        }
    }

    fn whole(&self) -> Option<i64> {
        match self {
            Value::Number(n) => Some(*n as i64),
            _ => None,
        }
    }

    fn millis(&self) -> Option<u64> {
        self.whole().map(|n| n.max(0) as u64)
    }

    fn count(&self) -> Option<usize> {
        self.whole().map(|n| n.max(0) as usize)
    }
}

/// One handler's view of an event: the pointers it tracks, the action as it sees it
/// (`adaptEvent`), in root coordinates.
#[derive(Debug)]
pub(crate) struct Event {
    pub(crate) action: Action,
    pub(crate) index: usize,
    pub(crate) pointers: Vec<Pointer>,
    pub(crate) time: u64,
}

impl Event {
    /// `GestureUtils.getLastPointerX/Y`: the pointers' average, or the last pointer; never the
    /// one going up.
    fn last_pointer(&self, average: bool) -> (f32, f32) {
        let exclude = (self.action == Action::PointerUp).then_some(self.index);
        if average {
            let kept = self
                .pointers
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != exclude);
            let (mut x, mut y, mut n) = (0.0, 0.0, 0.0);
            for (_, p) in kept {
                x += p.x;
                y += p.y;
                n += 1.0;
            }
            (x / n, y / n)
        } else {
            let mut last = self.pointers.len() - 1;
            if Some(last) == exclude && last > 0 {
                last -= 1;
            }
            (self.pointers[last].x, self.pointers[last].y)
        }
    }

    /// A long press's `getAverageCoords`: every pointer, or every one but the action's.
    fn average(&self, exclude_action: bool) -> (f32, f32) {
        let (mut x, mut y, mut n) = (0.0, 0.0, 0.0);
        for (i, p) in self.pointers.iter().enumerate() {
            if exclude_action && i == self.index {
                continue;
            }
            x += p.x;
            y += p.y;
            n += 1.0;
        }
        (x / n, y / n)
    }
}

#[derive(Debug)]
pub(crate) struct Handler {
    pub(crate) kind: Kind,
    pub(crate) state: State,
    pub(crate) enabled: bool,
    pub(crate) manual_activation: bool,
    pub(crate) cancels_js_responder: bool,
    should_cancel_when_outside: bool,
    hit_slop: Option<[Option<f32>; 6]>,
    /// The view it was recorded on, and that view's ancestors.
    pub(crate) view: Option<ViewTag>,
    pub(crate) ancestors: Vec<ViewTag>,
    pub(crate) tracked: BTreeSet<u32>,
    pub(crate) is_active: bool,
    pub(crate) is_awaiting: bool,
    pub(crate) activation_index: u64,
    pub(crate) should_reset_progress: bool,
    number_of_pointers: u32,
    pointer_type: PointerType,
    pointer_inside: bool,
    position: Position,
    /// When its timer fires, if one is set (Android's delayed runnable).
    pub(crate) deadline: Option<u64>,
    /// Whether it sends touch events (`needsPointerData`), and the pointers it has told of, by id.
    pub(crate) needs_pointer_data: bool,
    pub(crate) touches: BTreeMap<u32, TouchPoint>,
}

#[derive(Debug)]
pub(crate) enum Kind {
    Pan(Pan),
    Tap(Tap),
    LongPress(LongPress),
    Fling(Fling),
    Manual,
    Native,
    Pinch(Pinch),
    Rotation(Rotation),
}

impl Handler {
    pub(crate) fn new(name: &str) -> Option<Self> {
        let kind = match name {
            "PanGestureHandler" => Kind::Pan(Pan::default()),
            "TapGestureHandler" => Kind::Tap(Tap::default()),
            "LongPressGestureHandler" => Kind::LongPress(LongPress::default()),
            "FlingGestureHandler" => Kind::Fling(Fling::default()),
            "ManualGestureHandler" => Kind::Manual,
            "NativeViewGestureHandler" => Kind::Native,
            "PinchGestureHandler" => Kind::Pinch(Pinch::default()),
            "RotationGestureHandler" => Kind::Rotation(Rotation::default()),
            _ => return None,
        };
        let mut handler = Self {
            kind,
            state: State::Undetermined,
            enabled: true,
            manual_activation: false,
            cancels_js_responder: true,
            should_cancel_when_outside: false,
            hit_slop: None,
            view: None,
            ancestors: Vec::new(),
            tracked: BTreeSet::new(),
            is_active: false,
            is_awaiting: false,
            activation_index: u64::MAX,
            should_reset_progress: false,
            number_of_pointers: 0,
            pointer_type: PointerType::Touch,
            pointer_inside: false,
            position: Position::default(),
            deadline: None,
            needs_pointer_data: false,
            touches: BTreeMap::new(),
        };
        handler.reset_config();
        Some(handler)
    }

    /// `resetConfig`: every setting back to its default.
    pub(crate) fn reset_config(&mut self) {
        self.manual_activation = false;
        self.enabled = true;
        self.hit_slop = None;
        self.cancels_js_responder = true;
        self.needs_pointer_data = false;
        // A tap and a long press fail once outside their view unless told otherwise.
        self.should_cancel_when_outside = matches!(self.kind, Kind::Tap(_) | Kind::LongPress(_));
        match &mut self.kind {
            Kind::Pan(pan) => pan.config = PanConfig::default(),
            Kind::Tap(tap) => tap.config = TapConfig::default(),
            Kind::LongPress(press) => press.config = LongPressConfig::default(),
            Kind::Fling(fling) => fling.config = FlingConfig::default(),
            Kind::Manual | Kind::Native | Kind::Pinch(_) | Kind::Rotation(_) => {}
        }
    }

    /// `Factory.updateConfig`: the settings given change, the rest stay. Keys Android's handlers
    /// do not read (iOS's and the web's) are ignored, as there; those whose values OxyOS cannot
    /// honour are returned.
    pub(crate) fn update_config(&mut self, config: &[(&str, Value)]) -> Vec<String> {
        let mut refused = Vec::new();
        for (key, value) in config {
            if refuses(key, value) {
                refused.push((*key).to_string());
            } else if !self.set(key, value) {
                self.kind.set(key, value);
            }
        }
        if let Kind::Pan(pan) = &mut self.kind {
            let min_dist = config.iter().any(|(key, _)| *key == "minDist");
            if !min_dist && pan.config.custom_activation && !pan.config.explicit_min_dist {
                pan.config.min_dist = None;
            }
        }
        refused
    }

    /// A setting every handler has.
    fn set(&mut self, key: &str, value: &Value) -> bool {
        match key {
            "shouldCancelWhenOutside" => value.bool().map(|b| self.should_cancel_when_outside = b),
            "enabled" => value.bool().map(|b| self.enabled = b),
            "manualActivation" => value.bool().map(|b| self.manual_activation = b),
            "cancelsJSResponder" => value.bool().map(|b| self.cancels_js_responder = b),
            "needsPointerData" => value.bool().map(|b| self.needs_pointer_data = b),
            "hitSlop" => {
                let slop = match value {
                    Value::Null => None,
                    Value::Number(n) => {
                        let n = Some(*n as f32);
                        Some([n, n, n, n, None, None])
                    }
                    Value::List(edges) => {
                        let mut slop = [None; 6];
                        for (edge, value) in slop.iter_mut().zip(edges) {
                            *edge = value.map(|v| v as f32);
                        }
                        Some(slop)
                    }
                    _ => return false,
                };
                self.hit_slop = slop;
                Some(())
            }
            _ => None,
        }
        .is_some()
    }

    /// `prepare`: the handler takes part in a gesture on `view`.
    pub(crate) fn prepare(&mut self, view: ViewTag, ancestors: &[ViewTag]) {
        self.tracked.clear();
        self.state = State::Undetermined;
        self.view = Some(view);
        self.ancestors = ancestors.to_vec();
    }

    /// `reset`: after a gesture. The state stays what it ended in until the next one.
    pub(crate) fn reset(&mut self) {
        self.view = None;
        self.ancestors.clear();
        self.tracked.clear();
        self.touches.clear();
        self.deadline = None;
        self.is_active = false;
        self.is_awaiting = false;
        self.activation_index = u64::MAX;
        match &mut self.kind {
            Kind::Pan(pan) => pan.tracker = None,
            Kind::Tap(tap) => {
                tap.taps = 0;
                tap.max_pointers = 0;
            }
            Kind::LongPress(press) => press.pointers = 0,
            Kind::Fling(fling) => fling.tracker = None,
            // `onReset`: no detector, and no focal point or anchor until the next gesture.
            Kind::Pinch(pinch) => {
                pinch.detector = None;
                pinch.focal = (f32::NAN, f32::NAN);
                pinch.reset_progress();
            }
            Kind::Rotation(rotation) => {
                rotation.detector = None;
                rotation.anchor = (f32::NAN, f32::NAN);
                rotation.reset_progress();
            }
            Kind::Manual | Kind::Native => {}
        }
    }

    pub(crate) fn reset_progress(&mut self) {
        match &mut self.kind {
            Kind::Pan(pan) => pan.start = pan.last,
            Kind::Pinch(pinch) => pinch.reset_progress(),
            Kind::Rotation(rotation) => rotation.reset_progress(),
            _ => {}
        }
    }

    /// `isWithinBounds`: a point of the view inside it, its hit slop added.
    pub(crate) fn within_bounds(&self, bounds: Frame, x: f32, y: f32) -> bool {
        let (mut left, mut top) = (0.0, 0.0);
        let (mut right, mut bottom) = (bounds.width, bounds.height);
        if let Some([pad_left, pad_top, pad_right, pad_bottom, width, height]) = self.hit_slop {
            if let Some(pad) = pad_left {
                left -= pad;
            }
            if let Some(pad) = pad_top {
                top -= pad;
            }
            if let Some(pad) = pad_right {
                right += pad;
            }
            if let Some(pad) = pad_bottom {
                bottom += pad;
            }
            if let Some(width) = width {
                if pad_left.is_none() {
                    left = right - width;
                } else if pad_right.is_none() {
                    right = left + width;
                }
            }
            if let Some(height) = height {
                if pad_top.is_none() {
                    top = bottom - height;
                } else if pad_bottom.is_none() {
                    bottom = top + height;
                }
            }
        }
        (left..=right).contains(&x) && (top..=bottom).contains(&y)
    }

    /// `wantsEvent`.
    pub(crate) fn wants(&self, touch: &Touch) -> bool {
        if !self.enabled || self.state.finished() {
            return false;
        }
        match touch.action {
            Action::Move => touch.pointers.iter().any(|p| self.tracked.contains(&p.id)),
            _ => touch
                .pointers
                .get(touch.index)
                .is_some_and(|p| self.tracked.contains(&p.id)),
        }
    }

    /// `adaptEvent`: the pointers it tracks, and the action as it sees it: a pointer it does not
    /// track going down or up is a move to it.
    fn adapt(&self, touch: &Touch) -> Event {
        let pointers: Vec<Pointer> = touch
            .pointers
            .iter()
            .copied()
            .filter(|p| self.tracked.contains(&p.id))
            .collect();
        let actor = touch.pointers.get(touch.index).map(|p| p.id);
        let tracked_actor = actor.filter(|id| self.tracked.contains(id));
        let single = self.tracked.len() == 1;
        let action = match (touch.action, tracked_actor) {
            (Action::Down | Action::PointerDown, Some(_)) if single => Action::Down,
            (Action::Down | Action::PointerDown, Some(_)) => Action::PointerDown,
            (Action::Up | Action::PointerUp, Some(_)) if single => Action::Up,
            (Action::Up | Action::PointerUp, Some(_)) => Action::PointerUp,
            (Action::Cancel, _) => Action::Cancel,
            _ => Action::Move,
        };
        let index = tracked_actor
            .and_then(|id| pointers.iter().position(|p| p.id == id))
            .unwrap_or(0);
        Event {
            action,
            index,
            pointers,
            time: touch.time,
        }
    }

    pub(crate) fn pointer_type(&self) -> PointerType {
        self.pointer_type
    }

    pub(crate) fn set_pointer_type(&mut self, pointer_type: PointerType) {
        self.pointer_type = pointer_type;
    }

    /// The event builders' data, now.
    pub(crate) fn data(&self, now: u64) -> Data {
        let mut data = Data {
            number_of_pointers: self.number_of_pointers,
            pointer_type: self.pointer_type,
            position: Some(self.position),
            ..Data::default()
        };
        match &self.kind {
            Kind::Pan(pan) => {
                data.translation = Some(pan.translation());
                data.velocity = Some(pan.velocity);
            }
            Kind::LongPress(press) => data.duration = Some(now.saturating_sub(press.start_time)),
            Kind::Native => data.pointer_inside = Some(self.pointer_inside),
            Kind::Manual => data.position = None,
            // Their event builders have no position, only their own.
            Kind::Pinch(pinch) => {
                data.position = None;
                data.scale = Some(pinch.scale);
                data.focal = Some(pinch.focal);
                data.rate = Some(pinch.velocity);
            }
            Kind::Rotation(rotation) => {
                data.position = None;
                data.rotation = Some(rotation.rotation);
                data.anchor = Some(rotation.anchor);
                data.rate = Some(rotation.velocity);
            }
            Kind::Tap(_) | Kind::Fling(_) => {}
        }
        data
    }
}

/// Settings Android honours that OxyOS cannot: the host feeds the primary button alone (0, the
/// default, or 1).
fn refuses(key: &str, value: &Value) -> bool {
    key == "mouseButton" && value.whole().is_some_and(|b| b != 0 && b != 1)
}

impl Kind {
    /// A setting of the handler's own.
    fn set(&mut self, key: &str, value: &Value) -> bool {
        match self {
            Kind::Pan(pan) => pan.config.set(key, value),
            Kind::Tap(tap) => tap.config.set(key, value),
            Kind::LongPress(press) => press.config.set(key, value),
            Kind::Fling(fling) => fling.config.set(key, value),
            // Android's have no settings of their own.
            Kind::Manual | Kind::Pinch(_) | Kind::Rotation(_) => false,
            // They matter only to a native view that takes over, which this one never does.
            Kind::Native => matches!(key, "shouldActivateOnStart" | "disallowInterruption"),
        }
    }
}

/// `GestureHandler.handle`, then the handler's `onHandle`.
pub(crate) fn handle(o: &mut Orchestrator, tag: Tag, touch: &Touch, bounds: Frame) {
    let handler = o.handler(tag);
    if !handler.enabled || handler.state.finished() || handler.tracked.is_empty() {
        return;
    }
    let event = handler.adapt(touch);
    if event.pointers.is_empty() {
        return;
    }
    let handler = o.handler_mut(tag);
    handler.number_of_pointers = match event.action {
        Action::PointerUp => event.pointers.len() as u32 - 1,
        _ => event.pointers.len() as u32,
    };
    let first = event.pointers[0];
    handler.pointer_inside = handler.within_bounds(bounds, first.x - bounds.x, first.y - bounds.y);
    if handler.should_cancel_when_outside
        && !handler.pointer_inside
        && matches!(handler.state, State::Active | State::Began)
    {
        o.fail(tag);
        return;
    }
    let (x, y) = event.last_pointer(true);
    handler.position = Position {
        x: x - bounds.x,
        y: y - bounds.y,
        absolute_x: x,
        absolute_y: y,
    };
    if touch.action == Action::Down {
        handler.pointer_type = touch.pointer_type;
    }
    match handler.kind {
        Kind::Pan(_) => pan_handle(o, tag, &event),
        Kind::Tap(_) => tap_handle(o, tag, &event),
        Kind::LongPress(_) => long_press_handle(o, tag, &event),
        Kind::Fling(_) => fling_handle(o, tag, &event),
        Kind::Pinch(_) => pinch_handle(o, tag, &event, bounds),
        Kind::Rotation(_) => rotation_handle(o, tag, &event, bounds),
        Kind::Manual => {
            if o.handler(tag).state == State::Undetermined {
                o.begin(tag);
            }
        }
        // No native view takes over: it begins, and fails.
        Kind::Native => {
            if o.handler(tag).state == State::Undetermined {
                o.begin(tag);
                o.fail(tag);
            }
        }
    }
}

/// A handler's timer fired.
pub(crate) fn on_timer(o: &mut Orchestrator, tag: Tag) {
    match o.handler(tag).kind {
        Kind::Pan(_) | Kind::LongPress(_) => o.activate(tag, false),
        Kind::Tap(_) | Kind::Fling(_) => o.fail(tag),
        Kind::Manual | Kind::Native | Kind::Pinch(_) | Kind::Rotation(_) => {}
    }
}

/// A pan's, a pinch's and a rotation's `activate` reset their progress first.
pub(crate) fn before_activate(o: &mut Orchestrator, tag: Tag) {
    let handler = o.handler_mut(tag);
    let resets = matches!(
        handler.kind,
        Kind::Pan(_) | Kind::Pinch(_) | Kind::Rotation(_)
    );
    if resets && handler.state != State::Active {
        handler.reset_progress();
    }
}

/// A tap and a fling end as soon as they activate.
pub(crate) fn after_activate(o: &mut Orchestrator, tag: Tag) {
    if matches!(o.handler(tag).kind, Kind::Tap(_) | Kind::Fling(_)) {
        o.end(tag);
    }
}

/// `onStateChange`: a long press drops its timer on any change.
pub(crate) fn on_state_change(o: &mut Orchestrator, tag: Tag) {
    let handler = o.handler_mut(tag);
    if matches!(handler.kind, Kind::LongPress(_)) {
        handler.deadline = None;
    }
}

// ---- pan (PanGestureHandler) ----

#[derive(Debug)]
struct PanConfig {
    /// None where Android's is the value it ignores.
    min_dist: Option<f32>,
    custom_activation: bool,
    explicit_min_dist: bool,
    active_offset_x_start: Option<f32>,
    active_offset_x_end: Option<f32>,
    fail_offset_x_start: Option<f32>,
    fail_offset_x_end: Option<f32>,
    active_offset_y_start: Option<f32>,
    active_offset_y_end: Option<f32>,
    fail_offset_y_start: Option<f32>,
    fail_offset_y_end: Option<f32>,
    min_velocity_x: Option<f32>,
    min_velocity_y: Option<f32>,
    min_velocity: Option<f32>,
    min_pointers: usize,
    max_pointers: usize,
    average_touches: bool,
    activate_after_long_press: u64,
}

impl Default for PanConfig {
    fn default() -> Self {
        Self {
            min_dist: Some(TOUCH_SLOP),
            custom_activation: false,
            explicit_min_dist: false,
            active_offset_x_start: None,
            active_offset_x_end: None,
            fail_offset_x_start: None,
            fail_offset_x_end: None,
            active_offset_y_start: None,
            active_offset_y_end: None,
            fail_offset_y_start: None,
            fail_offset_y_end: None,
            min_velocity_x: None,
            min_velocity_y: None,
            min_velocity: None,
            min_pointers: 1,
            max_pointers: 10,
            average_touches: false,
            activate_after_long_press: 0,
        }
    }
}

impl PanConfig {
    fn set(&mut self, key: &str, value: &Value) -> bool {
        let criterion = match key {
            "activeOffsetXStart" => &mut self.active_offset_x_start,
            "activeOffsetXEnd" => &mut self.active_offset_x_end,
            "failOffsetXStart" => &mut self.fail_offset_x_start,
            "failOffsetXEnd" => &mut self.fail_offset_x_end,
            "activeOffsetYStart" => &mut self.active_offset_y_start,
            "activeOffsetYEnd" => &mut self.active_offset_y_end,
            "failOffsetYStart" => &mut self.fail_offset_y_start,
            "failOffsetYEnd" => &mut self.fail_offset_y_end,
            "minVelocity" => &mut self.min_velocity,
            "minVelocityX" => &mut self.min_velocity_x,
            "minVelocityY" => &mut self.min_velocity_y,
            _ => {
                return match key {
                    "minDist" => value.number().map(|d| {
                        self.min_dist = Some(d);
                        self.explicit_min_dist = true;
                    }),
                    "minPointers" => value.count().map(|n| self.min_pointers = n),
                    "maxPointers" => value.count().map(|n| self.max_pointers = n),
                    "avgTouches" => value.bool().map(|b| self.average_touches = b),
                    "activateAfterLongPress" => {
                        value.millis().map(|ms| self.activate_after_long_press = ms)
                    }
                    _ => None,
                }
                .is_some();
            }
        };
        let Some(number) = value.number() else {
            return false;
        };
        *criterion = Some(number);
        self.custom_activation = true;
        true
    }
}

#[derive(Debug, Default)]
pub(crate) struct Pan {
    config: PanConfig,
    start: (f32, f32),
    last: (f32, f32),
    offset: (f32, f32),
    velocity: (f32, f32),
    tracker: Option<Tracker>,
}

impl Pan {
    fn translation(&self) -> (f32, f32) {
        (
            self.last.0 - self.start.0 + self.offset.0,
            self.last.1 - self.start.1 + self.offset.1,
        )
    }

    fn should_activate(&self) -> bool {
        let c = &self.config;
        let (dx, dy) = self.translation();
        let below = |limit: Option<f32>, v: f32| limit.is_some_and(|l| v < l);
        let above = |limit: Option<f32>, v: f32| limit.is_some_and(|l| v > l);
        if below(c.active_offset_x_start, dx) || above(c.active_offset_x_end, dx) {
            return true;
        }
        if below(c.active_offset_y_start, dy) || above(c.active_offset_y_end, dy) {
            return true;
        }
        if c.min_dist.is_some_and(|d| dx * dx + dy * dy >= d * d) {
            return true;
        }
        let (vx, vy) = self.velocity;
        if c.min_velocity_x.is_some_and(|m| vx.abs() >= m.abs()) {
            return true;
        }
        if c.min_velocity_y.is_some_and(|m| vy.abs() >= m.abs()) {
            return true;
        }
        c.min_velocity.is_some_and(|m| vx * vx + vy * vy >= m * m)
    }

    /// Whether it fails; a long press it waited for that moved too far drops its timer.
    fn should_fail(&self) -> (bool, bool) {
        let c = &self.config;
        let (dx, dy) = self.translation();
        if c.activate_after_long_press > 0 && dx * dx + dy * dy > TOUCH_SLOP * TOUCH_SLOP {
            return (true, true);
        }
        let below = |limit: Option<f32>, v: f32| limit.is_some_and(|l| v < l);
        let above = |limit: Option<f32>, v: f32| limit.is_some_and(|l| v > l);
        let fail = below(c.fail_offset_x_start, dx)
            || above(c.fail_offset_x_end, dx)
            || below(c.fail_offset_y_start, dy)
            || above(c.fail_offset_y_end, dy);
        (fail, false)
    }
}

fn pan(o: &mut Orchestrator, tag: Tag) -> &mut Pan {
    match &mut o.handler_mut(tag).kind {
        Kind::Pan(pan) => pan,
        _ => unreachable!("a pan"),
    }
}

fn pan_handle(o: &mut Orchestrator, tag: Tag, event: &Event) {
    let state = o.handler(tag).state;
    let now = event.time;
    let p = pan(o, tag);
    let point = event.last_pointer(p.config.average_touches);
    if matches!(event.action, Action::PointerUp | Action::PointerDown) {
        p.offset.0 += p.last.0 - p.start.0;
        p.offset.1 += p.last.1 - p.start.1;
        p.last = point;
        p.start = point;
    } else {
        p.last = point;
    }
    if state == State::Undetermined && event.pointers.len() >= p.config.min_pointers {
        // `initialize`.
        p.start = p.last;
        p.offset = (0.0, 0.0);
        p.velocity = (0.0, 0.0);
        let mut tracker = Tracker::default();
        tracker.add(event);
        p.tracker = Some(tracker);
        let long_press = p.config.activate_after_long_press;
        o.begin(tag);
        if long_press > 0 {
            o.handler_mut(tag).deadline = Some(now + long_press);
        }
    } else if let Some(tracker) = &mut p.tracker {
        tracker.add(event);
        p.velocity = tracker.velocity(now);
    }
    let p = pan(o, tag);
    let (min_pointers, max_pointers) = (p.config.min_pointers, p.config.max_pointers);
    let count = event.pointers.len();
    match event.action {
        Action::Up => {
            if state == State::Active {
                o.end(tag);
            } else {
                o.fail(tag);
            }
        }
        Action::PointerDown if count > max_pointers => {
            if state == State::Active {
                o.cancel(tag);
            } else {
                o.fail(tag);
            }
        }
        Action::PointerUp if state == State::Active && count < min_pointers => o.fail(tag),
        _ if state == State::Began => {
            let (fail, drop_timer) = p.should_fail();
            if drop_timer {
                o.handler_mut(tag).deadline = None;
            }
            if fail {
                o.fail(tag);
            } else if pan(o, tag).should_activate() {
                o.activate(tag, false);
            }
        }
        _ => {}
    }
}

// ---- tap (TapGestureHandler) ----

#[derive(Debug)]
struct TapConfig {
    max_delta_x: Option<f32>,
    max_delta_y: Option<f32>,
    max_dist: Option<f32>,
    max_duration: u64,
    max_delay: u64,
    number_of_taps: u32,
    min_pointers: usize,
}

impl Default for TapConfig {
    fn default() -> Self {
        Self {
            max_delta_x: None,
            max_delta_y: None,
            max_dist: None,
            max_duration: 500,
            max_delay: 200,
            number_of_taps: 1,
            min_pointers: 1,
        }
    }
}

impl TapConfig {
    fn set(&mut self, key: &str, value: &Value) -> bool {
        match key {
            "numberOfTaps" => value.whole().map(|n| self.number_of_taps = n.max(0) as u32),
            "maxDurationMs" => value.millis().map(|ms| self.max_duration = ms),
            "maxDelayMs" => value.millis().map(|ms| self.max_delay = ms),
            "maxDeltaX" => value.number().map(|d| self.max_delta_x = Some(d)),
            "maxDeltaY" => value.number().map(|d| self.max_delta_y = Some(d)),
            "maxDist" => value.number().map(|d| self.max_dist = Some(d)),
            "minPointers" => value.count().map(|n| self.min_pointers = n),
            _ => None,
        }
        .is_some()
    }
}

#[derive(Debug)]
pub(crate) struct Tap {
    config: TapConfig,
    start: (f32, f32),
    last: (f32, f32),
    offset: (f32, f32),
    taps: u32,
    /// The most pointers down at once (`currentMaxNumberOfPointers`, 1 before the first gesture
    /// and 0 after each, as Android's).
    max_pointers: usize,
}

impl Default for Tap {
    fn default() -> Self {
        Self {
            config: TapConfig::default(),
            start: (0.0, 0.0),
            last: (0.0, 0.0),
            offset: (0.0, 0.0),
            taps: 0,
            max_pointers: 1,
        }
    }
}

impl Tap {
    fn should_fail(&self) -> bool {
        let dx = self.last.0 - self.start.0 + self.offset.0;
        let dy = self.last.1 - self.start.1 + self.offset.1;
        let c = &self.config;
        c.max_delta_x.is_some_and(|m| dx.abs() > m)
            || c.max_delta_y.is_some_and(|m| dy.abs() > m)
            || c.max_dist.is_some_and(|m| dx * dx + dy * dy > m * m)
    }
}

fn tap(o: &mut Orchestrator, tag: Tag) -> &mut Tap {
    match &mut o.handler_mut(tag).kind {
        Kind::Tap(tap) => tap,
        _ => unreachable!("a tap"),
    }
}

fn tap_handle(o: &mut Orchestrator, tag: Tag, event: &Event) {
    let state = o.handler(tag).state;
    let now = event.time;
    let t = tap(o, tag);
    let point = event.last_pointer(true);
    if state == State::Undetermined {
        t.offset = (0.0, 0.0);
        t.start = point;
    }
    if matches!(event.action, Action::PointerUp | Action::PointerDown) {
        t.offset.0 += t.last.0 - t.start.0;
        t.offset.1 += t.last.1 - t.start.1;
        t.last = point;
        t.start = point;
    } else {
        t.last = point;
    }
    t.max_pointers = t.max_pointers.max(event.pointers.len());
    let max_duration = t.config.max_duration;
    if t.should_fail() {
        o.fail(tag);
    } else if state == State::Undetermined {
        if event.action == Action::Down {
            o.begin(tag);
        }
        o.handler_mut(tag).deadline = Some(now + max_duration);
    } else if state == State::Began {
        match event.action {
            Action::Up => {
                // `endTap`.
                o.handler_mut(tag).deadline = None;
                let t = tap(o, tag);
                t.taps += 1;
                if t.taps == t.config.number_of_taps && t.max_pointers >= t.config.min_pointers {
                    o.activate(tag, false);
                } else {
                    let max_delay = t.config.max_delay;
                    o.handler_mut(tag).deadline = Some(now + max_delay);
                }
            }
            Action::Down => o.handler_mut(tag).deadline = Some(now + max_duration),
            _ => {}
        }
    }
}

// ---- long press (LongPressGestureHandler) ----

#[derive(Debug)]
struct LongPressConfig {
    min_duration: u64,
    max_dist: f32,
    pointers: i64,
}

impl Default for LongPressConfig {
    fn default() -> Self {
        Self {
            min_duration: 500,
            max_dist: LONG_PRESS_MAX_DIST,
            pointers: 1,
        }
    }
}

impl LongPressConfig {
    fn set(&mut self, key: &str, value: &Value) -> bool {
        match key {
            "minDurationMs" => value.millis().map(|ms| self.min_duration = ms),
            "maxDist" => value.number().map(|d| self.max_dist = d),
            "numberOfPointers" => value.whole().map(|n| self.pointers = n),
            _ => None,
        }
        .is_some()
    }
}

#[derive(Debug, Default)]
pub(crate) struct LongPress {
    config: LongPressConfig,
    start: (f32, f32),
    start_time: u64,
    pointers: i64,
}

fn long_press(o: &mut Orchestrator, tag: Tag) -> &mut LongPress {
    match &mut o.handler_mut(tag).kind {
        Kind::LongPress(press) => press,
        _ => unreachable!("a long press"),
    }
}

fn long_press_handle(o: &mut Orchestrator, tag: Tag, event: &Event) {
    let now = event.time;
    let state = |o: &Orchestrator| o.handler(tag).state;
    if state(o) == State::Undetermined {
        long_press(o, tag).start_time = now;
        o.begin(tag);
        let press = long_press(o, tag);
        press.start = event.average(false);
        press.pointers += 1;
    }
    if event.action == Action::PointerDown {
        let press = long_press(o, tag);
        press.pointers += 1;
        press.start = event.average(false);
        if press.pointers > press.config.pointers {
            o.fail(tag);
            long_press(o, tag).pointers = 0;
        }
    }
    let press = long_press(o, tag);
    let (pointers, required) = (press.pointers, press.config.pointers);
    let min_duration = press.config.min_duration;
    if state(o) == State::Began
        && pointers == required
        && matches!(event.action, Action::Down | Action::PointerDown)
    {
        o.handler_mut(tag).deadline = None;
        if min_duration > 0 {
            o.handler_mut(tag).deadline = Some(now + min_duration);
        } else {
            o.activate(tag, false);
        }
    }
    match event.action {
        Action::Up => {
            long_press(o, tag).pointers -= 1;
            o.handler_mut(tag).deadline = None;
            if state(o) == State::Active {
                o.end(tag);
            } else {
                o.fail(tag);
            }
        }
        Action::PointerUp => {
            let press = long_press(o, tag);
            press.pointers -= 1;
            if press.pointers < required && state(o) != State::Active {
                o.fail(tag);
                long_press(o, tag).pointers = 0;
            } else {
                long_press(o, tag).start = event.average(true);
            }
        }
        _ => {
            let press = long_press(o, tag);
            let (x, y) = event.average(false);
            let (dx, dy) = (x - press.start.0, y - press.start.1);
            let max = press.config.max_dist;
            if dx * dx + dy * dy > max * max {
                if state(o) == State::Active {
                    o.cancel(tag);
                } else {
                    o.fail(tag);
                }
            }
        }
    }
}

// ---- fling (FlingGestureHandler) ----

const DIRECTION_RIGHT: u32 = 1;
const DIRECTION_LEFT: u32 = 2;
const DIRECTION_UP: u32 = 4;
const DIRECTION_DOWN: u32 = 8;
const FLING_MAX_DURATION: u64 = 800;
/// Android's is in device pixels per second; here it is logical pixels per second.
const FLING_MIN_VELOCITY: f64 = 2000.0;
const FLING_ALIGNMENT_CONE: f64 = 30.0;

#[derive(Debug)]
struct FlingConfig {
    pointers: usize,
    direction: u32,
}

impl Default for FlingConfig {
    fn default() -> Self {
        Self {
            pointers: 1,
            direction: DIRECTION_RIGHT,
        }
    }
}

impl FlingConfig {
    fn set(&mut self, key: &str, value: &Value) -> bool {
        match key {
            "numberOfPointers" => value.count().map(|n| self.pointers = n),
            "direction" => value.whole().map(|d| self.direction = d.max(0) as u32),
            _ => None,
        }
        .is_some()
    }
}

#[derive(Debug, Default)]
pub(crate) struct Fling {
    config: FlingConfig,
    max_pointers: usize,
    tracker: Option<Tracker>,
}

/// A unit vector, none below Android's smallest recognizable magnitude (`Vector`).
fn unit(x: f64, y: f64) -> (f64, f64) {
    let magnitude = x.hypot(y);
    if magnitude > 0.1 {
        (x / magnitude, y / magnitude)
    } else {
        (0.0, 0.0)
    }
}

fn direction_vector(direction: u32) -> (f64, f64) {
    let (x, y) = match direction {
        DIRECTION_LEFT => (-1.0, 0.0),
        DIRECTION_RIGHT => (1.0, 0.0),
        DIRECTION_UP => (0.0, -1.0),
        DIRECTION_DOWN => (0.0, 1.0),
        d if d == DIRECTION_RIGHT | DIRECTION_UP => (1.0, -1.0),
        d if d == DIRECTION_RIGHT | DIRECTION_DOWN => (1.0, 1.0),
        d if d == DIRECTION_LEFT | DIRECTION_UP => (-1.0, -1.0),
        d if d == DIRECTION_LEFT | DIRECTION_DOWN => (-1.0, 1.0),
        _ => (0.0, 0.0),
    };
    unit(x, y)
}

fn fling(o: &mut Orchestrator, tag: Tag) -> &mut Fling {
    match &mut o.handler_mut(tag).kind {
        Kind::Fling(fling) => fling,
        _ => unreachable!("a fling"),
    }
}

/// `tryEndFling`: it activates if fast enough in one of its directions.
fn try_end_fling(o: &mut Orchestrator, tag: Tag, event: &Event) -> bool {
    let f = fling(o, tag);
    let Some(tracker) = &mut f.tracker else {
        return false;
    };
    tracker.add(event);
    let (vx, vy) = tracker.velocity(event.time);
    let (vx, vy) = (f64::from(vx), f64::from(vy));
    let velocity = unit(vx, vy);
    let aligned = |direction: u32, cone: f64| {
        let deviation = (cone / 2.0).to_radians().cos();
        let (dx, dy) = direction_vector(direction);
        f.config.direction & direction == direction && velocity.0 * dx + velocity.1 * dy > deviation
    };
    let axial = [
        DIRECTION_LEFT,
        DIRECTION_RIGHT,
        DIRECTION_UP,
        DIRECTION_DOWN,
    ]
    .into_iter()
    .any(|d| aligned(d, FLING_ALIGNMENT_CONE));
    let diagonal = [
        DIRECTION_RIGHT | DIRECTION_UP,
        DIRECTION_RIGHT | DIRECTION_DOWN,
        DIRECTION_LEFT | DIRECTION_UP,
        DIRECTION_LEFT | DIRECTION_DOWN,
    ]
    .into_iter()
    .any(|d| aligned(d, 90.0 - FLING_ALIGNMENT_CONE));
    let fast = vx.hypot(vy) > FLING_MIN_VELOCITY;
    if f.max_pointers == f.config.pointers && (axial || diagonal) && fast {
        o.handler_mut(tag).deadline = None;
        o.activate(tag, false);
        true
    } else {
        false
    }
}

fn fling_handle(o: &mut Orchestrator, tag: Tag, event: &Event) {
    let state = o.handler(tag).state;
    if state == State::Undetermined {
        // `startFling`.
        fling(o, tag).tracker = Some(Tracker::default());
        o.begin(tag);
        fling(o, tag).max_pointers = 1;
        o.handler_mut(tag).deadline = Some(event.time + FLING_MAX_DURATION);
    }
    if state == State::Began {
        try_end_fling(o, tag, event);
        let f = fling(o, tag);
        f.max_pointers = f.max_pointers.max(event.pointers.len());
        if event.action == Action::Up && !try_end_fling(o, tag, event) {
            o.fail(tag);
        }
    }
}

// ---- pinch (PinchGestureHandler, ScaleGestureDetector) ----

/// How far a pinch's span must change from where its scale began to activate it (the touch slop,
/// as Android's `spanSlop`).
const PINCH_SPAN_SLOP: f32 = TOUCH_SLOP;
/// How far the detector's span must change before a scale begins: twice the touch slop, as
/// Android's `ScaleGestureDetector` takes.
const SCALE_SPAN_SLOP: f32 = TOUCH_SLOP * 2.0;
/// The detector's smallest span: react-native-gesture-handler's copy sets it to zero, so fingers
/// close together scale too.
const SCALE_MIN_SPAN: f32 = 0.0;

#[derive(Debug)]
pub(crate) struct Pinch {
    scale: f64,
    /// Scale per second.
    velocity: f64,
    /// In its view; NaN, as Android's, until a gesture's first pointer.
    focal: (f32, f32),
    detector: Option<ScaleDetector>,
    starting_span: f32,
}

impl Default for Pinch {
    fn default() -> Self {
        Self {
            scale: 0.0,
            velocity: 0.0,
            focal: (f32::NAN, f32::NAN),
            detector: None,
            starting_span: 0.0,
        }
    }
}

impl Pinch {
    fn reset_progress(&mut self) {
        self.velocity = 0.0;
        self.scale = 1.0;
    }
}

/// react-native-gesture-handler's `ScaleGestureDetector`, Android's with its minimum span zero, in
/// root coordinates. Its anchored scale (a double tap then a drag, or a stylus button held) is not
/// ported: a pinch makes a new detector with each gesture, which so never has an earlier tap, and
/// the host feeds no stylus buttons. Its spans along each axis are left out, as a pinch reads only
/// the span.
#[derive(Debug, Default)]
struct ScaleDetector {
    focus: (f32, f32),
    curr_span: f32,
    prev_span: f32,
    initial_span: f32,
    curr_time: u64,
    prev_time: u64,
    in_progress: bool,
}

/// What the detector tells its listener: a scale begins (`onScaleBegin`), or goes on (`onScale`).
/// A pinch ignores `onScaleEnd`, which so is not told.
enum ScaleCall {
    Begin,
    Scale,
}

impl ScaleDetector {
    /// `getScaleFactor`: the span since the last scale.
    fn scale_factor(&self) -> f32 {
        if self.prev_span > 0.0 {
            self.curr_span / self.prev_span
        } else {
            1.0
        }
    }

    /// `getTimeDeltaSeconds`: the time since the last scale.
    fn time_delta_seconds(&self) -> f64 {
        (self.curr_time as i64 - self.prev_time as i64) as f64 / 1000.0
    }

    /// `onTouchEvent`. The listener's `onScaleBegin` and `onScale` always return true for a
    /// pinch, as it continues and takes every scale.
    fn on_touch(&mut self, event: &Event, listener: &mut dyn FnMut(ScaleCall, &ScaleDetector)) {
        self.curr_time = event.time;
        let action = event.action;
        let stream_complete = matches!(action, Action::Up | Action::Cancel);
        if action == Action::Down || stream_complete {
            // A scale in progress ends: a new stream begins, or this one is over.
            if self.in_progress {
                self.in_progress = false;
                self.initial_span = 0.0;
            }
            if stream_complete {
                return;
            }
        }
        let config_changed = matches!(
            action,
            Action::Down | Action::PointerUp | Action::PointerDown
        );
        // The pointer going up counts no more.
        let skip = (action == Action::PointerUp).then_some(event.index);
        let kept = || {
            event
                .pointers
                .iter()
                .enumerate()
                .filter(move |(i, _)| Some(*i) != skip)
                .map(|(_, p)| p)
        };
        let div = kept().count() as f32;
        let (mut sum_x, mut sum_y) = (0.0, 0.0);
        for p in kept() {
            sum_x += p.x;
            sum_y += p.y;
        }
        let focus = (sum_x / div, sum_y / div);
        // The span is the diameter of the circle whose radius is the pointers' average distance
        // from the focal point, along each axis.
        let (mut dev_x, mut dev_y) = (0.0f32, 0.0f32);
        for p in kept() {
            dev_x += (p.x - focus.0).abs();
            dev_y += (p.y - focus.1).abs();
        }
        let (span_x, span_y) = (dev_x / div * 2.0, dev_y / div * 2.0);
        let span = f64::from(span_x).hypot(f64::from(span_y)) as f32;
        let was_in_progress = self.in_progress;
        self.focus = focus;
        // A pointer down or up restarts the scale from the pointers there are now.
        if self.in_progress && (span < SCALE_MIN_SPAN || config_changed) {
            self.in_progress = false;
            self.initial_span = span;
        }
        if config_changed {
            self.initial_span = span;
            self.prev_span = span;
            self.curr_span = span;
        }
        if !self.in_progress
            && span >= SCALE_MIN_SPAN
            && (was_in_progress || (span - self.initial_span).abs() > SCALE_SPAN_SLOP)
        {
            self.prev_span = span;
            self.curr_span = span;
            self.prev_time = self.curr_time;
            listener(ScaleCall::Begin, self);
            self.in_progress = true;
        }
        if action == Action::Move {
            self.curr_span = span;
            if self.in_progress {
                listener(ScaleCall::Scale, self);
            }
            self.prev_span = self.curr_span;
            self.prev_time = self.curr_time;
        }
    }
}

fn pinch(o: &mut Orchestrator, tag: Tag) -> &mut Pinch {
    match &mut o.handler_mut(tag).kind {
        Kind::Pinch(pinch) => pinch,
        _ => unreachable!("a pinch"),
    }
}

fn pinch_handle(o: &mut Orchestrator, tag: Tag, event: &Event, bounds: Frame) {
    if o.handler(tag).state == State::Undetermined {
        match event.action {
            Action::Down => {
                // `initialize`: the focal point is the first pointer's until the detector has
                // one, as a NaN one keeps the event from JavaScript.
                let p = pinch(o, tag);
                p.reset_progress();
                p.detector = Some(ScaleDetector::default());
                let first = event.pointers[0];
                p.focal = (first.x - bounds.x, first.y - bounds.y);
            }
            Action::PointerDown => o.begin(tag),
            _ => {}
        }
    }
    let began = o.handler(tag).state == State::Began;
    let Pinch {
        scale,
        velocity,
        starting_span,
        detector,
        ..
    } = pinch(o, tag);
    let Some(detector) = detector else {
        return;
    };
    // It activates once the span has moved the slop from where the scale began; `activate`
    // waits for the detector, which reads nothing of the handler's state after `onScale`.
    let mut activate = false;
    detector.on_touch(event, &mut |call, d| match call {
        ScaleCall::Begin => *starting_span = d.curr_span,
        ScaleCall::Scale => {
            let previous = *scale;
            *scale *= f64::from(d.scale_factor());
            let delta = d.time_delta_seconds();
            if delta > 0.0 {
                *velocity = (*scale - previous) / delta;
            }
            if (*starting_span - d.curr_span).abs() >= PINCH_SPAN_SLOP && began {
                activate = true;
            }
        }
    });
    let focus = detector.focus;
    if activate {
        o.activate(tag, false);
    }
    // In its view, as Android transforms the detector's.
    pinch(o, tag).focal = (focus.0 - bounds.x, focus.1 - bounds.y);
    if event.action == Action::Up {
        match o.handler(tag).state {
            State::Undetermined => o.cancel(tag),
            State::Active => o.end(tag),
            _ => o.fail(tag),
        }
    }
}

// ---- rotation (RotationGestureHandler, RotationGestureDetector) ----

/// How far a rotation must turn to activate: five degrees, Android's
/// `ROTATION_RECOGNITION_THRESHOLD`.
const ROTATION_RECOGNITION_THRESHOLD: f64 = PI / 36.0;

#[derive(Debug)]
pub(crate) struct Rotation {
    rotation: f64,
    /// Radians per millisecond, as Android's.
    velocity: f64,
    /// In its view; NaN, as Android's, until a gesture's first pointer.
    anchor: (f32, f32),
    detector: Option<RotationDetector>,
}

impl Default for Rotation {
    fn default() -> Self {
        Self {
            rotation: 0.0,
            velocity: 0.0,
            anchor: (f32::NAN, f32::NAN),
            detector: None,
        }
    }
}

impl Rotation {
    fn reset_progress(&mut self) {
        self.velocity = 0.0;
        self.rotation = 0.0;
    }
}

/// react-native-gesture-handler's `RotationGestureDetector`, in root coordinates: the angle of
/// the line from its first pointer to its second. It pauses when one of them goes up, until
/// another goes down; the last going up finishes it.
#[derive(Debug, Default)]
struct RotationDetector {
    current_time: u64,
    previous_time: u64,
    previous_angle: f64,
    /// Radians since the last rotation event.
    rotation: f64,
    anchor: (f32, f32),
    in_progress: bool,
    paused: bool,
    pointers: [Option<u32>; 2],
}

/// What the detector tells its listener: `onRotationBegin`, `onRotation` and `onRotationEnd`.
enum RotationCall {
    Begin,
    Rotation,
    End,
}

impl RotationDetector {
    /// `timeDelta`: milliseconds since the last rotation event.
    fn time_delta(&self) -> i64 {
        self.current_time as i64 - self.previous_time as i64
    }

    fn update_current(&mut self, event: &Event) {
        self.previous_time = self.current_time;
        self.current_time = event.time;
        let find = |id: Option<u32>| id.and_then(|id| event.pointers.iter().find(|p| p.id == id));
        let (Some(first), Some(second)) = (find(self.pointers[0]), find(self.pointers[1])) else {
            return;
        };
        let (vector_x, vector_y) = (second.x - first.x, second.y - first.y);
        self.anchor = ((first.x + second.x) * 0.5, (first.y + second.y) * 0.5);
        // Negated so that a clockwise turn, y growing downwards, is a positive difference.
        let angle = -f64::from(vector_y).atan2(f64::from(vector_x));
        self.try_unpause(angle);
        self.rotation = if self.previous_angle.is_nan() {
            0.0
        } else {
            self.previous_angle - angle
        };
        self.previous_angle = angle;
        // Android's folding, half turns at a time.
        if self.rotation > PI {
            self.rotation -= PI;
        } else if self.rotation < -PI {
            self.rotation += PI;
        }
        if self.rotation > PI / 2.0 {
            self.rotation -= PI;
        } else if self.rotation < -PI / 2.0 {
            self.rotation += PI;
        }
    }

    /// A new second pointer goes on from its own angle, without a jump.
    fn try_unpause(&mut self, angle: f64) {
        if self.paused {
            self.previous_angle = angle;
            self.paused = false;
        }
    }

    /// `onTouchEvent`.
    fn on_touch(&mut self, event: &Event) -> Option<RotationCall> {
        let actor = event.pointers.get(event.index).map(|p| p.id);
        match event.action {
            Action::Down => {
                self.in_progress = false;
                self.pointers = [actor, None];
                None
            }
            Action::PointerDown => {
                if !self.in_progress || self.paused {
                    self.pointers[1] = actor;
                    self.update_current(event);
                }
                if self.in_progress {
                    return None;
                }
                self.in_progress = true;
                self.previous_time = event.time;
                // The first move after the second pointer down turns nothing, as on Android.
                self.previous_angle = f64::NAN;
                Some(RotationCall::Begin)
            }
            Action::Move if self.in_progress => {
                self.update_current(event);
                (!self.paused).then_some(RotationCall::Rotation)
            }
            Action::PointerUp if self.in_progress => {
                if actor == self.pointers[0] {
                    self.pointers = [self.pointers[1], None];
                    self.paused = true;
                } else if actor == self.pointers[1] {
                    self.pointers[1] = None;
                    self.paused = true;
                }
                None
            }
            // `finish`.
            Action::Up if self.in_progress => {
                self.paused = false;
                self.in_progress = false;
                Some(RotationCall::End)
            }
            _ => None,
        }
    }
}

fn rotation(o: &mut Orchestrator, tag: Tag) -> &mut Rotation {
    match &mut o.handler_mut(tag).kind {
        Kind::Rotation(rotation) => rotation,
        _ => unreachable!("a rotation"),
    }
}

fn rotation_handle(o: &mut Orchestrator, tag: Tag, event: &Event, bounds: Frame) {
    if o.handler(tag).state == State::Undetermined {
        match event.action {
            Action::Down => {
                // `initialize`: the anchor is the first pointer's, as a NaN one keeps the event
                // from JavaScript.
                let r = rotation(o, tag);
                r.reset_progress();
                r.detector = Some(RotationDetector::default());
                let first = event.pointers[0];
                r.anchor = (first.x - bounds.x, first.y - bounds.y);
            }
            Action::PointerDown => o.begin(tag),
            _ => {}
        }
    }
    let state = o.handler(tag).state;
    let r = rotation(o, tag);
    let Some(detector) = &mut r.detector else {
        return;
    };
    let call = detector.on_touch(event);
    let (step, delta, anchor) = (detector.rotation, detector.time_delta(), detector.anchor);
    match call {
        Some(RotationCall::Rotation) => {
            let previous = r.rotation;
            r.rotation += step;
            if delta > 0 {
                r.velocity = (r.rotation - previous) / delta as f64;
            }
            if r.rotation.abs() >= ROTATION_RECOGNITION_THRESHOLD && state == State::Began {
                o.activate(tag, false);
            }
        }
        Some(RotationCall::End) => {
            if state == State::Active {
                o.end(tag);
            } else {
                o.fail(tag);
            }
        }
        Some(RotationCall::Begin) | None => {}
    }
    // In its view, as Android transforms the detector's: before a second pointer, the detector's
    // is the root's origin.
    rotation(o, tag).anchor = (anchor.0 - bounds.x, anchor.1 - bounds.y);
    // The last pointer up with two or more down has been the detector's; with one, it begins
    // nothing.
    if event.action == Action::Up {
        match o.handler(tag).state {
            State::Undetermined => o.cancel(tag),
            State::Began => o.fail(tag),
            _ => {}
        }
    }
}
