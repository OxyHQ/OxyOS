//! Gesture recognition for react-native-gesture-handler on OxyOS: its Android orchestrator and
//! handlers (`com.swmansion.gesturehandler.core`, version 3.3.0), ported to Rust. The Bloom host
//! feeds it the pointer stream with the views under each new pointer and their frames, and
//! delivers what comes out to JavaScript as the Android module does: state changes, updates while
//! a handler is active, and when React Native's own touch handling must let go of the gesture.
//!
//! Units are the host's logical pixels, which are React Native's density-independent pixels, and
//! milliseconds. Android's delayed runnables (a tap's maximum duration, a long press's minimum one)
//! are deadlines: the host asks for the next one and calls [`Orchestrator::tick`] when it passes,
//! so recognition never reads a clock.
//!
//! Ported: the orchestrator (recording, activation, waiting for failure, simultaneity, blocking,
//! cancellation, cleanup), state changes from JavaScript, and the pan, tap, long-press, fling and
//! manual handlers. Where OxyOS differs from Android, and the port says so:
//! - the native-view handler (`NativeViewGestureHandler`) needs a native view's own touch
//!   handling, which the host does not have: it begins and fails, never taking over;
//! - pinch, rotation and hover handlers are not ported: creating one fails;
//! - velocity is a least-squares fit like Android's default `VelocityTracker`, in `velocity.rs`;
//! - the root view's own handler is not a handler here: what its cancellation does, taking the
//!   gesture from the views under it, comes out as [`Output::Intercept`].

use std::collections::BTreeMap;

mod handler;
#[cfg(test)]
mod tests;
mod velocity;

use handler::Handler;
pub use handler::Value;

/// A gesture handler's tag, as JavaScript numbers it.
pub type Tag = i32;
/// A React Native view's tag.
pub type ViewTag = i32;

/// react-native-gesture-handler's states, by its numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Undetermined = 0,
    Failed = 1,
    Began = 2,
    Cancelled = 3,
    Active = 4,
    End = 5,
}

impl State {
    fn finished(self) -> bool {
        matches!(self, State::Cancelled | State::Failed | State::End)
    }

    /// A state by its number, as JavaScript sends it.
    pub fn from_number(number: i32) -> Option<State> {
        Some(match number {
            0 => State::Undetermined,
            1 => State::Failed,
            2 => State::Began,
            3 => State::Cancelled,
            4 => State::Active,
            5 => State::End,
            _ => return None,
        })
    }
}

/// What kind of pointer a gesture is made with, by react-native-gesture-handler's numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum PointerType {
    #[default]
    Touch = 0,
    Stylus = 1,
    Mouse = 2,
}

/// Android's `MotionEvent` actions, for the pointer stream. A mouse's primary button pressed and
/// released are `Down` and `Up`, as Android's button press and release are to the handlers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Down,
    PointerDown,
    Move,
    PointerUp,
    Up,
    Cancel,
}

/// A pointer at a point of the root (the surface, in logical pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pointer {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// One event of the pointer stream: every pointer down, `index` the one the action is about.
#[derive(Clone, Debug, PartialEq)]
pub struct Touch {
    pub action: Action,
    pub index: usize,
    pub pointers: Vec<Pointer>,
    pub time: u64,
    pub pointer_type: PointerType,
}

/// A view's frame in the root, as the host lays it out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Where a gesture is: in its view, and in the root.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub absolute_x: f32,
    pub absolute_y: f32,
}

/// A pointer as a touch event tells of it (`PointerData`): in the handler's view, and in the root.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TouchPoint {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub absolute_x: f32,
    pub absolute_y: f32,
}

/// What a touch event is, by react-native-gesture-handler's numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TouchType {
    Down = 1,
    Move = 2,
    Up = 3,
    Cancelled = 4,
}

/// A handler's event data, as react-native-gesture-handler's event builders make it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub number_of_pointers: u32,
    pub pointer_type: PointerType,
    /// Every handler's but the manual one's.
    pub position: Option<Position>,
    /// A pan's.
    pub translation: Option<(f32, f32)>,
    /// A pan's, in logical pixels per second.
    pub velocity: Option<(f32, f32)>,
    /// A long press's, in milliseconds.
    pub duration: Option<u64>,
    /// A native view's.
    pub pointer_inside: Option<bool>,
}

/// What recognition produces, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    /// `onGestureHandlerStateChange`.
    StateChange {
        tag: Tag,
        view: ViewTag,
        state: State,
        old_state: State,
        data: Data,
    },
    /// `onGestureHandlerEvent`, while a handler is active.
    Update { tag: Tag, view: ViewTag, data: Data },
    /// `onGestureHandlerEvent` as a touch event (`onTouchesDown` and the rest), for a handler that
    /// needs them: the pointers that changed, and all it is told of. `state` is the handler's as
    /// JavaScript knows it (BEGAN while it waits to activate).
    Touch {
        tag: Tag,
        view: ViewTag,
        state: State,
        event_type: TouchType,
        number_of_touches: u32,
        pointer_type: PointerType,
        changed: Vec<TouchPoint>,
        all: Vec<TouchPoint>,
    },
    /// A handler activated while a pointer is down: until every pointer is up, the views' own
    /// touch handling (a text input's drag selection) is given nothing more, as Android's root view
    /// takes the gesture from the views under it once its own handler is cancelled. React Native's
    /// JavaScript touch is [`Output::CancelResponder`]'s.
    Intercept,
    /// A handler activated and cancels React Native's touch responder (`cancelsJSResponder`):
    /// JavaScript is sent `touchcancel` and nothing more of the gesture, as Android's
    /// `JSTouchDispatcher` does when a child starts a native gesture.
    CancelResponder { tag: Tag },
}

/// The relations JavaScript configures (`configureRelations`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Relations {
    pub wait_for: Vec<Tag>,
    pub simultaneous_with: Vec<Tag>,
    pub blocks: Vec<Tag>,
}

/// Why JavaScript's request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A handler OxyOS has not ported, by its name.
    Unsupported(String),
    /// A handler of this tag exists already.
    Exists(Tag),
    /// No handler has this tag.
    Unknown(Tag),
}

/// The orchestrator: handlers by tag, the views they are attached to, and the handlers taking
/// part in the current gesture, in the order they were recorded.
#[derive(Debug, Default)]
pub struct Orchestrator {
    handlers: BTreeMap<Tag, Handler>,
    /// Attachments in the order they were made, as Android's handlers for a view are.
    attached: Vec<(Tag, ViewTag)>,
    relations: BTreeMap<Tag, Relations>,
    recorded: Vec<Tag>,
    awaiting: Vec<Tag>,
    activation_index: u64,
    intercepting: bool,
    /// Whether a pointer is down.
    pressed: bool,
    /// The time of the event being handled, for handlers' timers and a long press's duration.
    now: u64,
    out: Vec<Output>,
}

impl Orchestrator {
    pub fn new() -> Self {
        Self::default()
    }

    /// `createGestureHandler`: a handler by its name, set to `config`. The keys whose values
    /// OxyOS cannot honour are returned, for the host to report; keys Android ignores are ignored.
    pub fn create(
        &mut self,
        name: &str,
        tag: Tag,
        config: &[(&str, Value)],
    ) -> Result<Vec<String>, Refused> {
        if self.handlers.contains_key(&tag) {
            return Err(Refused::Exists(tag));
        }
        let handler = Handler::new(name).ok_or_else(|| Refused::Unsupported(name.into()))?;
        self.handlers.insert(tag, handler);
        self.configure(tag, config, true)
    }

    /// `setGestureHandlerConfig` (`reset`) or `updateGestureHandlerConfig`: the keys whose values
    /// OxyOS cannot honour are returned.
    pub fn configure(
        &mut self,
        tag: Tag,
        config: &[(&str, Value)],
        reset: bool,
    ) -> Result<Vec<String>, Refused> {
        let handler = self.handlers.get_mut(&tag).ok_or(Refused::Unknown(tag))?;
        let was_enabled = handler.enabled;
        if reset {
            handler.reset_config();
        }
        let unknown = handler.update_config(config);
        // Disabling a handler that takes part cancels it.
        let handler = &self.handlers[&tag];
        if was_enabled && !handler.enabled && handler.view.is_some() {
            self.cancel(tag);
            self.cleanup();
        }
        Ok(unknown)
    }

    /// `attachGestureHandler`: a handler attached elsewhere before is detached, and cancelled if
    /// it takes part.
    pub fn attach(&mut self, tag: Tag, view: ViewTag) -> Result<(), Refused> {
        if !self.handlers.contains_key(&tag) {
            return Err(Refused::Unknown(tag));
        }
        self.detach(tag);
        self.attached.push((tag, view));
        Ok(())
    }

    fn detach(&mut self, tag: Tag) {
        self.attached.retain(|(t, _)| *t != tag);
        if self.handlers[&tag].view.is_some() {
            self.cancel(tag);
            self.cleanup();
        }
    }

    /// `configureRelations`.
    pub fn relate(&mut self, tag: Tag, relations: Relations) -> Result<(), Refused> {
        if !self.handlers.contains_key(&tag) {
            return Err(Refused::Unknown(tag));
        }
        self.relations.insert(tag, relations);
        Ok(())
    }

    /// `dropGestureHandler`.
    pub fn drop_handler(&mut self, tag: Tag) {
        if !self.handlers.contains_key(&tag) {
            return;
        }
        self.relations.remove(&tag);
        self.detach(tag);
        self.handlers.remove(&tag);
        self.recorded.retain(|t| *t != tag);
        self.awaiting.retain(|t| *t != tag);
    }

    /// A handler's state, if it exists.
    pub fn state(&self, tag: Tag) -> Option<State> {
        self.handlers.get(&tag).map(|h| h.state)
    }

    /// Whether a handler attached to `view` reaches the point `(x, y)` of the root: its view's
    /// bounds, its hit slop added, hold it. The host's hit testing adds such views, as Android's
    /// traversal reaches a handler's view outside its own bounds through its hit slop.
    pub fn reaches(&self, view: ViewTag, frame: Frame, x: f32, y: f32) -> bool {
        self.attached.iter().any(|(tag, v)| {
            let handler = &self.handlers[tag];
            *v == view && handler.enabled && handler.within_bounds(frame, x - frame.x, y - frame.y)
        })
    }

    /// The handlers taking part in the current gesture.
    pub fn recorded(&self) -> &[Tag] {
        &self.recorded
    }

    /// Whether the gesture is not React Native's (after [`Output::Intercept`], until every
    /// pointer is up).
    pub fn intercepting(&self) -> bool {
        self.intercepting
    }

    /// The earliest timer of a handler taking part, if any.
    pub fn next_deadline(&self) -> Option<u64> {
        self.recorded
            .iter()
            .filter_map(|tag| self.handlers[tag].deadline)
            .min()
    }

    /// Time passed: the timers due fire (a tap failing, a long press activating).
    pub fn tick(&mut self, now: u64) -> Vec<Output> {
        loop {
            let due = self
                .recorded
                .iter()
                .filter_map(|tag| Some((self.handlers[tag].deadline?, *tag)))
                .filter(|(deadline, _)| *deadline <= now)
                .min();
            let Some((deadline, tag)) = due else { break };
            // The timer fires when it was due, as Android's runnable would have.
            self.now = deadline;
            self.handler_mut(tag).deadline = None;
            self.on_timer(tag);
            self.cleanup();
        }
        self.now = now;
        std::mem::take(&mut self.out)
    }

    /// `setGestureHandlerState`, JavaScript's manual state changes (a manual gesture's
    /// `activate()`, `end()` and the rest).
    pub fn set_state(&mut self, tag: Tag, state: State) -> Vec<Output> {
        if let Some(handler) = self.handlers.get(&tag) {
            // A gesture that has received no touches is not activated.
            if state != State::Active || handler.state == State::Began {
                match state {
                    State::Active => self.activate(tag, true),
                    State::Began => self.begin(tag),
                    State::End => self.end(tag),
                    State::Failed => self.fail(tag),
                    State::Cancelled => self.cancel(tag),
                    State::Undetermined => {}
                }
            }
        }
        self.cleanup();
        std::mem::take(&mut self.out)
    }

    /// One event of the pointer stream. For a pointer going down, `hits` are the views under it,
    /// deepest first, then that view's ancestors to the root; `frame` gives a view's frame in the
    /// root, none for a view no longer there.
    pub fn touch(
        &mut self,
        touch: &Touch,
        hits: &[ViewTag],
        frame: &dyn Fn(ViewTag) -> Option<Frame>,
    ) -> Vec<Output> {
        // Timers due by now fire first, as they would have before this event on Android.
        self.out = self.tick(touch.time);
        match touch.action {
            Action::Down | Action::PointerDown => {
                self.pressed = true;
                self.extract(touch, hits, frame);
            }
            Action::Cancel => self.cancel_all(),
            _ => {}
        }
        let mut order = self.recorded.clone();
        order.sort_by(|a, b| self.compare(*a, *b));
        for tag in order {
            self.deliver(tag, touch, frame);
        }
        self.cleanup();
        if matches!(touch.action, Action::Up | Action::Cancel) {
            self.pressed = false;
            self.intercepting = false;
        }
        std::mem::take(&mut self.out)
    }

    /// Records the handlers of the views under a new pointer, deepest first, whose bounds hold it.
    fn extract(
        &mut self,
        touch: &Touch,
        hits: &[ViewTag],
        frame: &dyn Fn(ViewTag) -> Option<Frame>,
    ) {
        let Some(pointer) = touch.pointers.get(touch.index).copied() else {
            return;
        };
        for (depth, view) in hits.iter().enumerate() {
            let Some(bounds) = frame(*view) else { continue };
            let tags: Vec<Tag> = self
                .attached
                .iter()
                .filter(|(_, v)| v == view)
                .map(|(t, _)| *t)
                .collect();
            for tag in tags {
                let handler = &self.handlers[&tag];
                let (x, y) = (pointer.x - bounds.x, pointer.y - bounds.y);
                if !handler.enabled || !handler.within_bounds(bounds, x, y) {
                    continue;
                }
                self.record(tag, *view, &hits[depth + 1..]);
                self.handler_mut(tag).tracked.insert(pointer.id);
            }
        }
    }

    fn record(&mut self, tag: Tag, view: ViewTag, ancestors: &[ViewTag]) {
        if self.recorded.contains(&tag) {
            return;
        }
        let handler = self.handler_mut(tag);
        handler.is_active = false;
        handler.is_awaiting = false;
        handler.activation_index = u64::MAX;
        handler.prepare(view, ancestors);
        self.recorded.push(tag);
    }

    /// Active handlers first, then awaiting ones, each the latest activated first; then the rest
    /// as recorded (`handlersComparator`).
    fn compare(&self, a: Tag, b: Tag) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        let (a, b) = (&self.handlers[&a], &self.handlers[&b]);
        if a.is_active && b.is_active || a.is_awaiting && b.is_awaiting {
            b.activation_index.cmp(&a.activation_index)
        } else if a.is_active {
            Ordering::Less
        } else if b.is_active {
            Ordering::Greater
        } else if a.is_awaiting {
            Ordering::Less
        } else if b.is_awaiting {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }

    fn deliver(&mut self, tag: Tag, touch: &Touch, frame: &dyn Fn(ViewTag) -> Option<Frame>) {
        let Some(handler) = self.handlers.get(&tag) else {
            return;
        };
        if !self.recorded.contains(&tag) {
            return;
        }
        // A handler whose view is gone is cancelled.
        let Some(bounds) = handler.view.and_then(frame) else {
            self.cancel(tag);
            return;
        };
        if !handler.wants(touch) {
            return;
        }
        if handler.needs_pointer_data {
            // Android sets the pointer type after the first touch event of a gesture, which so
            // carries the previous gesture's; here it carries its own.
            if touch.action == Action::Down {
                self.handler_mut(tag).set_pointer_type(touch.pointer_type);
            }
            self.update_pointer_data(tag, touch, bounds);
        }
        let handler = &self.handlers[&tag];
        if !handler.is_awaiting || touch.action != Action::Move {
            handler::handle(self, tag, touch, bounds);
            let handler = &self.handlers[&tag];
            if handler.state == State::Active && handler.is_active {
                // A handler done waiting starts from here, without a jump in what it reports.
                if handler.should_reset_progress {
                    let handler = self.handler_mut(tag);
                    handler.should_reset_progress = false;
                    handler.reset_progress();
                }
                self.dispatch_update(tag);
            }
            if matches!(touch.action, Action::Up | Action::PointerUp) {
                let id = touch.pointers[touch.index].id;
                self.handler_mut(tag).tracked.remove(&id);
            }
        }
    }

    // ---- state changes: GestureHandler's begin, activate, end, fail and cancel ----

    pub(crate) fn begin(&mut self, tag: Tag) {
        if self.handlers[&tag].state == State::Undetermined {
            self.move_to(tag, State::Began);
        }
    }

    pub(crate) fn activate(&mut self, tag: Tag, force: bool) {
        handler::before_activate(self, tag);
        let handler = &self.handlers[&tag];
        if (!handler.manual_activation || force)
            && matches!(handler.state, State::Undetermined | State::Began)
        {
            self.move_to(tag, State::Active);
        }
        handler::after_activate(self, tag);
    }

    pub(crate) fn end(&mut self, tag: Tag) {
        if matches!(self.handlers[&tag].state, State::Began | State::Active) {
            self.move_to(tag, State::End);
        }
    }

    pub(crate) fn fail(&mut self, tag: Tag) {
        if matches!(
            self.handlers[&tag].state,
            State::Active | State::Undetermined | State::Began
        ) {
            self.move_to(tag, State::Failed);
        }
    }

    pub(crate) fn cancel(&mut self, tag: Tag) {
        let handler = &self.handlers[&tag];
        if matches!(
            handler.state,
            State::Active | State::Undetermined | State::Began
        ) || handler.is_awaiting
        {
            // `onCancel`: every ported handler drops its timer.
            self.handler_mut(tag).deadline = None;
            self.move_to(tag, State::Cancelled);
        }
    }

    fn move_to(&mut self, tag: Tag, state: State) {
        let handler = self.handler_mut(tag);
        if handler.state == state {
            return;
        }
        // Pointers told of are cancelled when it finishes (`cancelPointers`).
        if state.finished() && !handler.touches.is_empty() {
            let all: Vec<TouchPoint> = std::mem::take(&mut handler.touches).into_values().collect();
            self.dispatch_touch(tag, TouchType::Cancelled, all.clone(), all);
        }
        let handler = self.handler_mut(tag);
        let old = handler.state;
        handler.state = state;
        self.on_state_change(tag, state, old);
        handler::on_state_change(self, tag);
    }

    // ---- the orchestrator's side of a state change ----

    fn on_state_change(&mut self, tag: Tag, new: State, prev: State) {
        if self.handlers[&tag].is_awaiting && matches!(new, State::Cancelled | State::Failed) {
            self.handler_mut(tag).is_awaiting = false;
        }
        if new.finished() {
            for other in self.awaiting.clone() {
                if !self.waits_for(other, tag) || !self.awaiting.contains(&other) {
                    continue;
                }
                if new == State::End {
                    // What it waited for succeeded: it is cancelled.
                    self.cancel(other);
                    if self.handlers[&other].state == State::End {
                        self.dispatch_state(other, State::Cancelled, State::Began);
                    }
                    self.handler_mut(other).is_awaiting = false;
                } else {
                    // What it waited for failed: it may activate.
                    self.try_activate(other);
                }
            }
            self.cleanup_awaiting();
        }
        if new == State::Active {
            self.try_activate(tag);
        } else if matches!(prev, State::Active | State::End) {
            if self.handlers[&tag].is_active {
                self.dispatch_state(tag, new, prev);
            } else if matches!(new, State::Cancelled | State::Failed) {
                // It activated while awaiting and finished before it was let: JavaScript saw it
                // begin only.
                self.dispatch_state(tag, new, State::Began);
            }
        } else if prev != State::Undetermined || new != State::Cancelled {
            // One cancelled before it began is not reported: JavaScript never saw it begin.
            self.dispatch_state(tag, new, prev);
        }
    }

    /// Whether `handler` waits for `other` to fail.
    fn waits_for(&self, handler: Tag, other: Tag) -> bool {
        let lists = |tag: Tag, f: fn(&Relations) -> &Vec<Tag>, of: Tag| {
            self.relations.get(&tag).is_some_and(|r| f(r).contains(&of))
        };
        handler != other
            && (lists(handler, |r| &r.wait_for, other) || lists(other, |r| &r.blocks, handler))
    }

    fn simultaneous(&self, a: Tag, b: Tag) -> bool {
        let with = |x: Tag, y: Tag| {
            self.relations
                .get(&x)
                .is_some_and(|r| r.simultaneous_with.contains(&y))
        };
        a == b || with(a, b) || with(b, a)
    }

    fn common_pointers(&self, a: Tag, b: Tag) -> bool {
        !self.handlers[&a]
            .tracked
            .is_disjoint(&self.handlers[&b].tracked)
    }

    fn descendant_of(&self, handler: Tag, other: Tag) -> bool {
        let of = self.handlers[&other].view;
        of.is_some_and(|view| self.handlers[&handler].ancestors.contains(&view))
    }

    fn try_activate(&mut self, tag: Tag) {
        let by_finished = self
            .recorded
            .iter()
            .any(|o| self.waits_for(tag, *o) && self.handlers[o].state == State::End);
        let by_active = self.recorded.iter().any(|o| {
            self.common_pointers(tag, *o)
                && self.handlers[o].is_active
                && !self.simultaneous(tag, *o)
                && self.descendant_of(tag, *o)
        });
        if by_finished || by_active {
            self.cancel(tag);
            return;
        }
        let must_wait = self
            .recorded
            .iter()
            .any(|o| !self.handlers[o].state.finished() && self.waits_for(tag, *o));
        if must_wait {
            if !self.awaiting.contains(&tag) {
                self.awaiting.push(tag);
                let index = self.next_activation_index();
                let handler = self.handler_mut(tag);
                handler.is_awaiting = true;
                handler.activation_index = index;
            }
        } else {
            self.make_active(tag);
            self.handler_mut(tag).is_awaiting = false;
        }
    }

    fn next_activation_index(&mut self) -> u64 {
        let index = self.activation_index;
        self.activation_index += 1;
        index
    }

    fn cleanup_awaiting(&mut self) {
        let handlers = &self.handlers;
        self.awaiting.retain(|t| handlers[t].is_awaiting);
    }

    /// Whether `handler` is cancelled when `other` activates.
    fn cancelled_by(&self, handler: Tag, other: Tag) -> bool {
        if !self.common_pointers(handler, other) || self.simultaneous(handler, other) {
            return false;
        }
        let h = &self.handlers[&handler];
        // One awaiting or active is kept: only a native view's handler or the root's would
        // cancel it, and neither activates here.
        !(handler != other && (h.is_awaiting || h.state == State::Active))
    }

    fn make_active(&mut self, tag: Tag) {
        let current = self.handlers[&tag].state;
        let index = self.next_activation_index();
        {
            let handler = self.handler_mut(tag);
            handler.is_awaiting = false;
            handler.is_active = true;
            handler.should_reset_progress = true;
            handler.activation_index = index;
        }
        for other in self.recorded.clone().into_iter().rev() {
            if self.cancelled_by(other, tag) {
                self.cancel(other);
            }
        }
        // Those awaiting this one's failure wait no more.
        for other in self.awaiting.clone().into_iter().rev() {
            if self.cancelled_by(other, tag) {
                self.handler_mut(other).is_awaiting = false;
            }
        }
        self.cleanup_awaiting();
        // One that failed or was cancelled while awaiting has been reported already.
        if matches!(current, State::Failed | State::Cancelled) {
            return;
        }
        // The root view's handler, which tracks every pointer, is cancelled by any activation:
        // the gesture is no longer React Native's. One activating once every pointer is up (a
        // single tap whose double tap failed) takes nothing from it.
        if self.pressed && !self.intercepting {
            self.intercepting = true;
            self.out.push(Output::Intercept);
        }
        if self.handlers[&tag].cancels_js_responder {
            self.out.push(Output::CancelResponder { tag });
        }
        // It may have activated, or even ended, while awaiting: JavaScript is told now.
        self.dispatch_state(tag, State::Active, State::Began);
        if current != State::Active {
            self.dispatch_state(tag, State::End, State::Active);
            if current != State::End {
                self.dispatch_state(tag, State::Undetermined, State::End);
            }
        }
    }

    fn cancel_all(&mut self) {
        for tag in self.awaiting.clone().into_iter().rev() {
            self.cancel(tag);
        }
        for tag in self.recorded.clone().into_iter().rev() {
            self.cancel(tag);
        }
    }

    /// Finished handlers leave the gesture and are reset (`cleanupFinishedHandlers`).
    fn cleanup(&mut self) {
        let handlers = &mut self.handlers;
        self.recorded.retain(|tag| {
            let handler = handlers.get_mut(tag).expect("recorded");
            if handler.state.finished() && !handler.is_awaiting {
                handler.reset();
                false
            } else {
                true
            }
        });
    }

    fn dispatch_state(&mut self, tag: Tag, state: State, old_state: State) {
        let handler = &self.handlers[&tag];
        let output = Output::StateChange {
            tag,
            view: handler.view.unwrap_or(-1),
            state,
            old_state,
            data: handler.data(self.now),
        };
        self.out.push(output);
    }

    fn dispatch_update(&mut self, tag: Tag) {
        let handler = &self.handlers[&tag];
        let output = Output::Update {
            tag,
            view: handler.view.unwrap_or(-1),
            data: handler.data(self.now),
        };
        self.out.push(output);
    }

    // ---- touch events (`updatePointerData`) ----

    /// Tells a handler that needs pointer data of the pointers going down, moving and going up.
    fn update_pointer_data(&mut self, tag: Tag, touch: &Touch, bounds: Frame) {
        let point = |p: &Pointer| TouchPoint {
            id: p.id,
            x: p.x - bounds.x,
            y: p.y - bounds.y,
            absolute_x: p.x,
            absolute_y: p.y,
        };
        let actor = touch.pointers.get(touch.index).map(point);
        match (touch.action, actor) {
            (Action::Down | Action::PointerDown, Some(actor)) => {
                let handler = self.handler_mut(tag);
                handler.touches.insert(actor.id, actor);
                let all = handler.touches.values().copied().collect();
                self.dispatch_touch(tag, TouchType::Down, vec![actor], all);
                self.touches_moved(tag, touch, &point);
            }
            (Action::Up | Action::PointerUp, Some(actor)) => {
                self.touches_moved(tag, touch, &point);
                let handler = self.handler_mut(tag);
                if handler.touches.contains_key(&actor.id) {
                    // All as they were, the one going up included.
                    let all = handler.touches.values().copied().collect();
                    handler.touches.remove(&actor.id);
                    self.dispatch_touch(tag, TouchType::Up, vec![actor], all);
                }
            }
            (Action::Move, _) => self.touches_moved(tag, touch, &point),
            _ => {}
        }
    }

    fn touches_moved(&mut self, tag: Tag, touch: &Touch, point: &dyn Fn(&Pointer) -> TouchPoint) {
        let handler = self.handler_mut(tag);
        let mut changed = Vec::new();
        for pointer in &touch.pointers {
            let now = point(pointer);
            if let Some(told) = handler.touches.get_mut(&pointer.id) {
                if (told.x, told.y) != (now.x, now.y) {
                    *told = now;
                    changed.push(now);
                }
            }
        }
        if !changed.is_empty() {
            let all = handler.touches.values().copied().collect();
            self.dispatch_touch(tag, TouchType::Move, changed, all);
        }
    }

    fn dispatch_touch(
        &mut self,
        tag: Tag,
        event_type: TouchType,
        changed: Vec<TouchPoint>,
        all: Vec<TouchPoint>,
    ) {
        let handler = &self.handlers[&tag];
        // One activated but waiting is, to JavaScript, still beginning.
        let state = if handler.is_awaiting && handler.state == State::Active {
            State::Began
        } else {
            handler.state
        };
        let output = Output::Touch {
            tag,
            view: handler.view.unwrap_or(-1),
            state,
            event_type,
            number_of_touches: handler.touches.len() as u32,
            pointer_type: handler.pointer_type(),
            changed,
            all,
        };
        self.out.push(output);
    }

    fn on_timer(&mut self, tag: Tag) {
        handler::on_timer(self, tag);
    }

    pub(crate) fn handler(&self, tag: Tag) -> &Handler {
        &self.handlers[&tag]
    }

    pub(crate) fn handler_mut(&mut self, tag: Tag) -> &mut Handler {
        self.handlers.get_mut(&tag).expect("present")
    }
}
