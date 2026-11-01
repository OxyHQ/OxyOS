//! The input method (ADR 0008): one client the session grants `input_method`. While the focused
//! surface has a field with text input enabled, and the field is not sensitive, the input method
//! is told the field's state (never its surface, its client or where it is on screen), receives
//! the keys first and gives back those it does not use, and sends changes the field applies
//! whole. It may have one candidates surface, which the compositor places next to the caret.

use super::text_input::{TextTransaction, MAX_PREEDIT_BYTES, MAX_PREEDIT_SEGMENTS};
use super::{ClientId, Role, Scene, SurfaceKey};
use crate::placement::{anchor_point, place};
use oxyos_protocol::{
    Adjust, Anchor, ErrorCode, Event, Gravity, Positioner, PreeditStyle, PreeditText, Request,
    Size, TextChunk, TextHints, TextPurpose, MAX_SURROUNDING_BYTES,
};
use std::collections::VecDeque;

/// Keys the input method may hold unanswered. Past this it is not answering: every key it holds
/// goes on to the application, so a stuck input method never takes the keyboard with it.
pub const MAX_PENDING_KEYS: usize = 64;

#[derive(Debug)]
struct PendingKey {
    serial: u64,
    /// When it happened (`Scene::set_input_time`): the time it reaches the surface with, however
    /// long the input method held it.
    time_ns: u64,
    key: u32,
    pressed: bool,
    /// What the layout types with it.
    text: Option<String>,
    /// The input method's answer: whether it took the key.
    used: Option<bool>,
}

/// A change being received from the input method.
#[derive(Debug, Default)]
struct Parts {
    delete: (u32, u32),
    commit: String,
    preedit: String,
    cursor: (u32, u32),
    segments: Vec<(u32, u32, PreeditStyle)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Served {
    surface: SurfaceKey,
    field: u32,
    serial: u32,
}

#[derive(Debug)]
pub(super) struct InputMethod {
    pub(super) client: ClientId,
    /// The field it serves: its surface, which field of it, and the commit serial it was last
    /// told.
    active: Option<Served>,
    keys: VecDeque<PendingKey>,
    parts: Parts,
    /// Its candidates surface, if it made one.
    candidates: Option<u32>,
}

impl InputMethod {
    pub(super) fn new(client: ClientId) -> Self {
        Self {
            client,
            active: None,
            keys: VecDeque::new(),
            parts: Parts::default(),
            candidates: None,
        }
    }
}

/// Whether a field's content must never reach an input method: a password or PIN, or anything
/// the application marked sensitive.
fn sensitive(purpose: TextPurpose, hints: TextHints) -> bool {
    matches!(purpose, TextPurpose::Password | TextPurpose::Pin)
        || hints.contains(TextHints::SENSITIVE)
}

impl Scene {
    /// The field the input method may serve now: the focused surface's, if it has text input
    /// enabled and is not sensitive, and is not the input method's own.
    fn served_field(&self) -> Option<Served> {
        let im = self.input_method.as_ref()?;
        let key = self
            .focus
            .filter(|f| self.visible(*f) && f.client != im.client)?;
        let state = self.text_input(key)?;
        (!sensitive(state.purpose, state.hints)).then_some(Served {
            surface: key,
            field: state.field,
            serial: state.serial,
        })
    }

    /// Brings the input method up to date with the focused field, and its candidates with the
    /// caret.
    pub(super) fn refresh_input_method(&mut self) {
        self.tell_input_method();
        self.place_candidates();
    }

    /// Tells the input method what changed of the field it serves: another field (deactivating
    /// the one before first, its held keys going on to it), none, or the same field's new state.
    fn tell_input_method(&mut self) {
        let Some(active) = self.input_method.as_ref().map(|im| im.active) else {
            return;
        };
        let want = self.served_field();
        if active == want {
            return;
        }
        let same = |a: Served, b: Served| (a.surface, a.field) == (b.surface, b.field);
        if !matches!((active, want), (Some(a), Some(b)) if same(a, b)) {
            self.deactivate_input_method();
        }
        let Some(Served {
            surface: key,
            serial,
            ..
        }) = want
        else {
            return;
        };
        let im = self.input_method.as_mut().expect("present");
        let client = im.client;
        let starting = im.active.is_none();
        im.active = want;
        if starting {
            self.emit(client, Event::InputMethodActivate {});
        }
        let state = self.text_input(key).expect("served").clone();
        self.emit(
            client,
            Event::InputMethodPurpose {
                purpose: state.purpose,
                hints: state.hints,
            },
        );
        for text in TextChunk::split(&state.surrounding) {
            self.emit(client, Event::InputMethodSurrounding { text });
        }
        self.emit(
            client,
            Event::InputMethodSelection {
                anchor: state.anchor,
                cursor: state.cursor,
            },
        );
        self.emit(client, Event::InputMethodDone { serial });
    }

    /// The input method stops serving its field: the keys it holds go on to the field first, as
    /// they would have without it, and a change it was sending is dropped.
    pub(super) fn deactivate_input_method(&mut self) {
        let Some(im) = self.input_method.as_mut() else {
            return;
        };
        if im.active.take().is_none() {
            return;
        }
        im.parts = Parts::default();
        let client = im.client;
        self.release_keys(true);
        self.emit(client, Event::InputMethodDeactivate {});
    }

    /// The input method is gone (disconnected, or its grant revoked): the keys it held go on.
    pub(super) fn forget_input_method(&mut self, client: ClientId) {
        let Some(im) = self.input_method.as_ref().filter(|im| im.client == client) else {
            return;
        };
        // Its candidates stay hidden until the client destroys them.
        let candidates = im.candidates.map(|id| SurfaceKey { client, id });
        if let Some(node) = candidates.and_then(|key| self.nodes.get_mut(&key)) {
            node.hidden = true;
        }
        self.release_keys(true);
        self.input_method = None;
    }

    /// The input method's candidates surface (one), placed by the compositor.
    pub(super) fn create_candidates(
        &mut self,
        client: ClientId,
        key: SurfaceKey,
        size: Size,
    ) -> Result<(), ErrorCode> {
        let im = self
            .input_method
            .as_ref()
            .filter(|im| im.client == client)
            .ok_or(ErrorCode::Unauthorized)?;
        let existing = im.candidates.map(|id| SurfaceKey { client, id });
        if existing.is_some_and(|existing| self.nodes.contains_key(&existing)) {
            return Err(ErrorCode::InvalidRole);
        }
        self.check_new(key)?;
        let output = self.outputs.first().map_or(0, |output| output.id);
        self.insert(key, Role::Candidates, None, (0, 0), size, output);
        self.input_method.as_mut().expect("present").candidates = Some(key.id);
        self.place_candidates();
        Ok(())
    }

    /// Places the candidates below the caret of the field the input method serves (above it
    /// when there is no room), on the caret's output; hides them while it serves none or the
    /// field has not said where its caret is.
    pub(super) fn place_candidates(&mut self) {
        let Some(im) = self.input_method.as_ref() else {
            return;
        };
        let Some(key) = im.candidates.map(|id| SurfaceKey {
            client: im.client,
            id,
        }) else {
            return;
        };
        let Some(node) = self.nodes.get(&key) else {
            return;
        };
        let size = node.pending.map_or(node.size, |(_, _, size)| size);
        let placed = im.active.and_then(|served| {
            let caret = self.text_input(served.surface)?.caret?;
            let field = self.global_rect(served.surface)?;
            let positioner = Positioner {
                anchor_rect: caret,
                size,
                anchor: Anchor::BottomLeft,
                gravity: Gravity::BottomRight,
                adjust: Adjust::FLIP_Y.union(Adjust::SLIDE_X).union(Adjust::SLIDE_Y),
                offset_x: 0,
                offset_y: 0,
            };
            let origin = (field.x, field.y);
            let (x, y) = anchor_point(&positioner, origin);
            let output = self.output_at(x, y)?;
            Some((place(&positioner, origin, output.rect)?, output.id))
        });
        let node = self.nodes.get_mut(&key).expect("present");
        let Some((rect, output)) = placed else {
            node.hidden = true;
            return;
        };
        node.hidden = false;
        node.origin = (rect.x, rect.y);
        if let Some((serial, _, size)) = node.pending {
            node.pending = Some((serial, (rect.x, rect.y), size));
        }
        if node.output != output {
            self.move_to_output(key, output);
        }
    }

    /// A key from the keyboard, with what the layout types with it. The screen reader's keys, the
    /// hardware keys (ADR 0025) and the system shortcuts (ADR 0024) come first, the text of a key they take with it; then,
    /// while the input method serves a field, it receives the key and the key waits for its
    /// answer; otherwise the key, then its text, go to the focused surface. It never repeats.
    pub fn keyboard(&mut self, key: u32, pressed: bool, text: Option<&str>) {
        self.keyboard_repeating(key, pressed, text, false);
    }

    /// The same, for a key the keymap repeats while held when `repeats` is set (a press: never a
    /// modifier, a lock or a dead key): where its press went to something keys repeat into, it is
    /// pressed again while held, with the same text (docs/adr/0036-key-repeat.mdx).
    pub fn keyboard_repeating(
        &mut self,
        key: u32,
        pressed: bool,
        text: Option<&str>,
        repeats: bool,
    ) {
        self.end_repeat_for(key, pressed);
        self.route_keyboard(key, pressed, text);
        if pressed && repeats {
            self.start_repeat(key, text);
        }
    }

    /// A key from the keyboard, or a repeat of the held one, where it goes.
    pub(super) fn route_keyboard(&mut self, key: u32, pressed: bool, text: Option<&str>) {
        self.user_input();
        self.hold_modifier(key, pressed);
        if self.reader_key(key, pressed)
            || self.hardware_key(key, pressed)
            || self.system_key(key, pressed)
        {
            return;
        }
        let served = self
            .input_method
            .as_ref()
            .and_then(|im| im.active.map(|_| im.client));
        let Some(client) = served else {
            self.surface_key_typed(key, pressed, text);
            return;
        };
        let serial = self.next_serial();
        if pressed {
            self.last_user_serial = serial;
        }
        let time_ns = self.input_time_ns;
        let im = self.input_method.as_mut().expect("served");
        im.keys.push_back(PendingKey {
            serial,
            time_ns,
            key,
            pressed,
            text: text.map(str::to_string),
            used: None,
        });
        let overflow = im.keys.len() > MAX_PENDING_KEYS;
        // A key whose text is longer than one chunk reaches the input method without it.
        let typed = text
            .and_then(PreeditText::new)
            .unwrap_or_else(|| PreeditText::new("").expect("empty"));
        self.emit(
            client,
            Event::InputMethodKey {
                serial,
                key,
                pressed,
                text: typed,
                time_ns,
            },
        );
        if overflow {
            self.release_keys(true);
        }
    }

    /// Delivers the held keys in order: up to the first unanswered one, or (`all`) every one the
    /// input method did not take.
    fn release_keys(&mut self, all: bool) {
        loop {
            let Some(im) = self.input_method.as_mut() else {
                return;
            };
            let Some(front) = im.keys.front() else {
                return;
            };
            if front.used.is_none() && !all {
                return;
            }
            let key = im.keys.pop_front().expect("present");
            if key.used != Some(true) {
                self.deliver_key(key);
            }
        }
    }

    /// A key given back reaches the focused surface as if it had come straight.
    fn deliver_key(&mut self, pending: PendingKey) {
        let typed = pending.text.as_deref();
        let left = match self.focus.filter(|f| self.visible(*f)) {
            Some(target) => self.emit_key(
                target,
                pending.serial,
                pending.key,
                pending.pressed,
                typed,
                pending.time_ns,
            ),
            None => typed,
        };
        if let Some(text) = left.map(str::to_string) {
            self.text(&text);
        }
    }

    /// A request only the input method may send.
    pub(super) fn input_method_request(
        &mut self,
        client: ClientId,
        request: Request,
    ) -> Result<(), ErrorCode> {
        let im = self
            .input_method
            .as_mut()
            .filter(|im| im.client == client)
            .ok_or(ErrorCode::Unauthorized)?;
        let parts = &mut im.parts;
        match request {
            Request::InputMethodKeyHandled { serial, used } => {
                // A key already released (its field deactivated, or too many held) is answered
                // late: nothing to do.
                if let Some(key) = im.keys.iter_mut().find(|key| key.serial == serial) {
                    key.used.get_or_insert(used);
                }
                self.release_keys(false);
            }
            Request::InputMethodDelete { before, after } => parts.delete = (before, after),
            Request::InputMethodCommitText { text } => {
                if parts.commit.len() + text.as_str().len() > MAX_SURROUNDING_BYTES {
                    return Err(ErrorCode::QuotaExceeded);
                }
                parts.commit.push_str(text.as_str());
            }
            Request::InputMethodPreeditText { text } => {
                if parts.preedit.len() + text.as_str().len() > MAX_PREEDIT_BYTES {
                    return Err(ErrorCode::QuotaExceeded);
                }
                parts.preedit.push_str(text.as_str());
            }
            Request::InputMethodPreeditCursor { begin, end } => parts.cursor = (begin, end),
            Request::InputMethodPreeditSegment { begin, end, style } => {
                if parts.segments.len() >= MAX_PREEDIT_SEGMENTS {
                    return Err(ErrorCode::QuotaExceeded);
                }
                parts.segments.push((begin, end, style));
            }
            Request::InputMethodCommit { serial } => {
                let parts = std::mem::take(parts);
                // A change for a field no longer served is dropped: focus moved on.
                let Some(Served {
                    surface: target,
                    serial: current,
                    ..
                }) = im.active
                else {
                    return Ok(());
                };
                // It names a state it was told, never a later one.
                if serial > current {
                    return Err(ErrorCode::InvalidMessage);
                }
                let change = TextTransaction {
                    delete_before: parts.delete.0,
                    delete_after: parts.delete.1,
                    commit: &parts.commit,
                    preedit: &parts.preedit,
                    cursor: parts.cursor,
                    segments: &parts.segments,
                };
                if !change.is_valid() {
                    return Err(ErrorCode::InvalidMessage);
                }
                self.emit_transaction(target, &change, serial);
            }
            _ => unreachable!("only input-method requests are passed here"),
        }
        Ok(())
    }
}
