//! The input method (ADR 0008): one client the session grants `input_method`. While the focused
//! surface has a field with text input enabled, and the field is not sensitive, the input method
//! is told the field's state (never its surface, its client or where it is on screen), receives
//! the keys first and gives back those it does not use, and sends changes the field applies
//! whole. It may have one candidates surface, which the compositor places next to the caret, or
//! hand its candidates to the system (ADR 0041): the session's shell of the caret's output draws
//! them in a candidates surface of its own, placed the same way, and its presses on them come
//! back to the input method.

use super::text_input::{TextTransaction, MAX_PREEDIT_BYTES, MAX_PREEDIT_SEGMENTS};
use super::{ClientId, Role, Scene, SurfaceKey};
use crate::placement::{anchor_point, place};
use oxyos_protocol::{
    Adjust, Anchor, ErrorCode, Event, Gravity, Positioner, PreeditStyle, PreeditText, Rect,
    Request, Size, TextChunk, TextHints, TextPurpose, MAX_CANDIDATES_AUX_BYTES,
    MAX_CANDIDATE_BYTES, MAX_INPUT_CANDIDATES, MAX_SURROUNDING_BYTES, NO_CANDIDATE,
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

/// What the keymap made of a key as it happened (ADR 0041): its XKB keysym and the modifiers in
/// effect, in X11's bits, told to the input method before the key (`InputMethodKeySymbol`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeySymbol {
    pub keysym: u32,
    pub modifiers: u32,
}

/// Candidates an input method handed the system to draw (ADR 0041).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct CandidateList {
    pub(super) candidates: Vec<String>,
    pub(super) aux: String,
    pub(super) highlighted: u8,
    pub(super) flags: u8,
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

/// A list of candidates being received from the input method.
#[derive(Debug, Default)]
struct ListParts {
    candidates: Vec<String>,
    aux: String,
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
    /// The candidates it handed the system, being received, and as last closed.
    list_parts: ListParts,
    list: Option<CandidateList>,
}

impl InputMethod {
    pub(super) fn new(client: ClientId) -> Self {
        Self {
            client,
            active: None,
            keys: VecDeque::new(),
            parts: Parts::default(),
            candidates: None,
            list_parts: ListParts::default(),
            list: None,
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
        self.tell_candidates();
        self.refresh_autofill();
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
        im.list_parts = ListParts::default();
        im.list = None;
        let client = im.client;
        self.release_keys(true);
        self.emit(client, Event::InputMethodDeactivate {});
        self.tell_candidates();
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
        self.tell_candidates();
    }

    /// The input method's candidates surface (one), placed by the compositor; or the session's
    /// shell's (one per shell client), where it draws the candidates an input method hands the
    /// system (ADR 0041).
    pub(super) fn create_candidates(
        &mut self,
        client: ClientId,
        key: SurfaceKey,
        size: Size,
    ) -> Result<(), ErrorCode> {
        let shell = self.clients.get(&client).is_some_and(|grants| grants.shell);
        let im = self.input_method.as_ref().filter(|im| im.client == client);
        let existing = if let Some(im) = im {
            im.candidates
        } else if shell {
            self.shell_candidates.get(&client).copied()
        } else {
            return Err(ErrorCode::Unauthorized);
        };
        let existing = existing.map(|id| SurfaceKey { client, id });
        if existing.is_some_and(|existing| self.nodes.contains_key(&existing)) {
            return Err(ErrorCode::InvalidRole);
        }
        self.check_new(key)?;
        let output = self.outputs.first().map_or(0, |output| output.id);
        self.insert(key, Role::Candidates, None, (0, 0), size, output);
        match self.input_method.as_mut().filter(|im| im.client == client) {
            Some(im) => im.candidates = Some(key.id),
            None => {
                self.shell_candidates.insert(client, key.id);
            }
        }
        self.place_candidates();
        Ok(())
    }

    /// A candidates surface of the client's at a new size: configured anew, its content kept
    /// until a frame of that size, then placed by it.
    pub(super) fn resize_candidates(
        &mut self,
        key: SurfaceKey,
        size: Size,
    ) -> Result<(), ErrorCode> {
        let node = self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        if !matches!(node.role, Role::Candidates) {
            return Err(ErrorCode::InvalidRole);
        }
        self.reconfigure(key, None, size);
        self.place_candidates();
        Ok(())
    }

    /// The shell that draws the candidates handed to the system now: the one showing the
    /// output the served field's caret is on (none while no list is shown).
    fn candidates_shell(&self) -> Option<ClientId> {
        let im = self.input_method.as_ref()?;
        im.list.as_ref()?;
        self.caret_shell(im.active?.surface)
    }

    /// The shell of the output the caret of `surface`'s field is on, which draws what the system
    /// shows at that caret (an input method's candidates, AutoFill's list).
    pub(super) fn caret_shell(&self, surface: SurfaceKey) -> Option<ClientId> {
        let caret = self.text_input(surface)?.caret?;
        let field = self.global_rect(surface)?;
        let output = self
            .output_at(
                i64::from(field.x) + i64::from(caret.x),
                i64::from(field.y) + i64::from(caret.y),
            )
            .map(|output| output.id)
            .or_else(|| self.live(surface).map(|node| node.output))?;
        self.shortcut_shell(output)
    }

    /// Tells the shells the candidates handed to the system: the one that draws them now gets
    /// the list; one that drew them before and does not now, an empty one.
    pub(super) fn tell_candidates(&mut self) {
        let shell = self.candidates_shell();
        let list = self
            .input_method
            .as_ref()
            .and_then(|im| im.list.clone())
            .filter(|_| shell.is_some());
        let told = std::mem::replace(&mut self.candidates_told, shell.zip(list.clone()));
        if let Some((before, _)) = &told {
            if Some(*before) != shell && self.clients.contains_key(before) {
                self.emit(
                    *before,
                    Event::InputCandidates {
                        count: 0,
                        highlighted: NO_CANDIDATE,
                        flags: 0,
                    },
                );
            }
        }
        if told == self.candidates_told {
            return self.place_candidates();
        }
        if let Some(shell) = shell {
            let list = list.unwrap_or_default();
            for (index, candidate) in list.candidates.iter().enumerate() {
                for text in TextChunk::split(candidate) {
                    self.emit(
                        shell,
                        Event::InputCandidate {
                            index: index as u8,
                            text,
                        },
                    );
                }
            }
            for text in TextChunk::split(&list.aux) {
                self.emit(shell, Event::InputCandidatesAux { text });
            }
            self.emit(
                shell,
                Event::InputCandidates {
                    count: list.candidates.len() as u8,
                    highlighted: list.highlighted,
                    flags: list.flags,
                },
            );
        }
        self.place_candidates();
    }

    /// A shell client is gone: its candidates surface with it, and another shell draws them.
    pub(super) fn forget_candidates_shell(&mut self, client: ClientId) {
        self.shell_candidates.remove(&client);
        if self
            .candidates_told
            .as_ref()
            .is_some_and(|(shell, _)| *shell == client)
        {
            self.candidates_told = None;
            self.tell_candidates();
        }
    }

    /// What the keymap made of the next key the scene is given (ADR 0041): the program sets it
    /// before each `keyboard` call, as it sets the input's time.
    pub fn set_key_symbol(&mut self, symbol: KeySymbol) {
        self.key_symbol = symbol;
    }

    /// The input-source shortcuts pressed since the last call (ADR 0041), in order.
    pub fn take_input_source_shortcuts(&mut self) -> Vec<super::InputSourceShortcut> {
        std::mem::take(&mut self.input_source_shortcuts)
    }

    /// The candidates surfaces shown, and where: the input method's own, or the shell's drawing
    /// the candidates handed to the system (ADR 0041).
    pub fn shown_candidates(&self) -> Vec<(SurfaceKey, Rect)> {
        let im = self.input_method.as_ref().and_then(|im| {
            im.candidates.map(|id| SurfaceKey {
                client: im.client,
                id,
            })
        });
        self.shell_candidates
            .iter()
            .map(|(client, id)| SurfaceKey {
                client: *client,
                id: *id,
            })
            .chain(im)
            .filter(|key| self.visible(*key))
            .filter_map(|key| Some((key, self.global_rect(key)?)))
            .collect()
    }

    /// Whether an input method is connected (the session's frontend connects only while an input
    /// method is the current input source).
    pub fn has_input_method(&self) -> bool {
        self.input_method.is_some()
    }

    /// Whether the input method serves a field now.
    pub fn input_method_serving(&self) -> bool {
        self.input_method
            .as_ref()
            .is_some_and(|im| im.active.is_some())
    }

    /// The shell's press on a candidate it draws, or on a chevron: passed to the input method.
    pub(super) fn candidates_request(
        &mut self,
        client: ClientId,
        request: Request,
    ) -> Result<(), ErrorCode> {
        let is_shell = self.clients.get(&client).is_some_and(|grants| grants.shell);
        if !is_shell {
            return Err(ErrorCode::Unauthorized);
        }
        // A list hidden since the press, or drawn by another shell now: nothing to choose.
        let Some(count) = self
            .candidates_told
            .as_ref()
            .filter(|(shell, _)| *shell == client)
            .map(|(_, list)| list.candidates.len())
        else {
            return Ok(());
        };
        let im = self
            .input_method
            .as_ref()
            .expect("a list is the input method's")
            .client;
        let event = match request {
            Request::ChooseInputCandidate { index } => {
                if usize::from(index) >= count {
                    return Err(ErrorCode::InvalidMessage);
                }
                Event::InputMethodCandidateChosen { index }
            }
            Request::TurnInputCandidatesPage { forward } => {
                Event::InputMethodCandidatesPage { forward }
            }
            _ => unreachable!("only the candidates' requests are passed here"),
        };
        self.emit(im, event);
        Ok(())
    }

    /// Places the candidates below the caret of the field the input method serves (above it
    /// when there is no room), on the caret's output; hides them while it serves none or the
    /// field has not said where its caret is. A shell's candidates surface is placed the same
    /// way while it draws a list, at the caret of the field the list is for: AutoFill's
    /// (docs/adr/0050-keychain-autofill-and-passkeys.mdx), else the input method's; and hidden
    /// otherwise.
    pub(super) fn place_candidates(&mut self) {
        let served = self
            .input_method
            .as_ref()
            .and_then(|im| im.active)
            .map(|served| served.surface);
        let autofill = self
            .autofill
            .told
            .as_ref()
            .map(|(shell, _)| *shell)
            .zip(self.autofill_surface());
        let mut surfaces: Vec<(SurfaceKey, Option<SurfaceKey>)> = self
            .shell_candidates
            .iter()
            .map(|(client, id)| {
                let anchor = match autofill {
                    Some((shell, surface)) if shell == *client => Some(surface),
                    _ => served.filter(|_| {
                        self.candidates_told
                            .as_ref()
                            .is_some_and(|(shell, _)| shell == client)
                    }),
                };
                (
                    SurfaceKey {
                        client: *client,
                        id: *id,
                    },
                    anchor,
                )
            })
            .collect();
        if let Some(im) = self.input_method.as_ref() {
            if let Some(id) = im.candidates {
                surfaces.push((
                    SurfaceKey {
                        client: im.client,
                        id,
                    },
                    served,
                ));
            }
        }
        for (key, anchor) in surfaces {
            self.place_candidates_surface(key, anchor);
        }
    }

    /// Places a candidates surface below the caret of `anchor`'s field, or hides it (none).
    fn place_candidates_surface(&mut self, key: SurfaceKey, anchor: Option<SurfaceKey>) {
        let Some(node) = self.nodes.get(&key) else {
            return;
        };
        let size = node.pending.map_or(node.size, |(_, _, size)| size);
        let placed = anchor.and_then(|surface| {
            let caret = self.text_input(surface)?.caret?;
            let field = self.global_rect(surface)?;
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
        self.key_with_super(key, pressed);
        self.hold_modifier(key, pressed);
        if self.reader_key(key, pressed)
            || self.hardware_key(key, pressed)
            || self.system_key(key, pressed)
            || self.autofill_key(key, pressed)
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
        let symbol = self.key_symbol;
        self.emit(
            client,
            Event::InputMethodKeySymbol {
                serial,
                keysym: symbol.keysym,
                modifiers: symbol.modifiers,
            },
        );
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
            Request::InputMethodCandidate { index, text } => {
                let list = &mut im.list_parts;
                let index = usize::from(index);
                if index == list.candidates.len() {
                    if index >= MAX_INPUT_CANDIDATES {
                        return Err(ErrorCode::QuotaExceeded);
                    }
                    list.candidates.push(String::new());
                } else if index + 1 != list.candidates.len() {
                    return Err(ErrorCode::InvalidOrder);
                }
                let candidate = list.candidates.last_mut().expect("present");
                if candidate.len() + text.as_str().len() > MAX_CANDIDATE_BYTES {
                    return Err(ErrorCode::QuotaExceeded);
                }
                candidate.push_str(text.as_str());
            }
            Request::InputMethodCandidatesAux { text } => {
                let aux = &mut im.list_parts.aux;
                if aux.len() + text.as_str().len() > MAX_CANDIDATES_AUX_BYTES {
                    return Err(ErrorCode::QuotaExceeded);
                }
                aux.push_str(text.as_str());
            }
            Request::InputMethodCandidates {
                count,
                highlighted,
                flags,
            } => {
                let parts = std::mem::take(&mut im.list_parts);
                if usize::from(count) != parts.candidates.len()
                    || (highlighted != NO_CANDIDATE && highlighted >= count)
                {
                    return Err(ErrorCode::InvalidMessage);
                }
                // A list for no field served is not shown: focus moved on.
                im.list = (count > 0 && im.active.is_some()).then_some(CandidateList {
                    candidates: parts.candidates,
                    aux: parts.aux,
                    highlighted,
                    flags,
                });
                self.tell_candidates();
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
