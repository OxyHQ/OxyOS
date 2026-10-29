//! The handlers: what every handler has (`GestureHandler`), and the pan, tap, long-press, fling,
//! manual and native-view handlers' own recognition (`onHandle`), timers and settings.

use crate::velocity::Tracker;
use crate::{
    Action, Data, Frame, Orchestrator, Pointer, PointerType, Position, State, Tag, Touch, ViewTag,
};
use std::collections::BTreeSet;

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
}

#[derive(Debug)]
pub(crate) enum Kind {
    Pan(Pan),
    Tap(Tap),
    LongPress(LongPress),
    Fling(Fling),
    Manual,
    Native,
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
        // A tap and a long press fail once outside their view unless told otherwise.
        self.should_cancel_when_outside = matches!(self.kind, Kind::Tap(_) | Kind::LongPress(_));
        match &mut self.kind {
            Kind::Pan(pan) => pan.config = PanConfig::default(),
            Kind::Tap(tap) => tap.config = TapConfig::default(),
            Kind::LongPress(press) => press.config = LongPressConfig::default(),
            Kind::Fling(fling) => fling.config = FlingConfig::default(),
            Kind::Manual | Kind::Native => {}
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
            Kind::Manual | Kind::Native => {}
        }
    }

    pub(crate) fn reset_progress(&mut self) {
        if let Kind::Pan(pan) = &mut self.kind {
            pan.start = pan.last;
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
            Kind::Tap(_) | Kind::Fling(_) => {}
        }
        data
    }
}

/// Settings Android honours that OxyOS cannot: touch events (`onTouchesDown` and the rest) are not
/// sent, and the host feeds the primary button alone (0, the default, or 1).
fn refuses(key: &str, value: &Value) -> bool {
    match key {
        "needsPointerData" => value.bool() == Some(true),
        "mouseButton" => value.whole().is_some_and(|b| b != 0 && b != 1),
        _ => false,
    }
}

impl Kind {
    /// A setting of the handler's own.
    fn set(&mut self, key: &str, value: &Value) -> bool {
        match self {
            Kind::Pan(pan) => pan.config.set(key, value),
            Kind::Tap(tap) => tap.config.set(key, value),
            Kind::LongPress(press) => press.config.set(key, value),
            Kind::Fling(fling) => fling.config.set(key, value),
            Kind::Manual => false,
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
        Kind::Manual | Kind::Native => {}
    }
}

/// A pan's `activate` resets its progress first.
pub(crate) fn before_activate(o: &mut Orchestrator, tag: Tag) {
    let handler = o.handler_mut(tag);
    if matches!(handler.kind, Kind::Pan(_)) && handler.state != State::Active {
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
