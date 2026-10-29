//! Text input (ADR 0008): a focused field describes what it is editing, and changes reach it as
//! transactions applied whole. A surface that never enabled text input receives the older `Text`
//! and `Preedit` events instead, as much of a change as they can say.

use super::{Scene, SurfaceKey};
use oxyos_protocol::{
    ErrorCode, Event, PreeditStyle, PreeditText, Rect, TextChunk, TextHints, TextPurpose,
    MAX_SURROUNDING_BYTES,
};

/// Styled segments one composition may have.
pub const MAX_PREEDIT_SEGMENTS: usize = 16;
/// Bytes of one composition.
pub const MAX_PREEDIT_BYTES: usize = MAX_SURROUNDING_BYTES;

/// What a field last committed about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextInputState {
    pub purpose: TextPurpose,
    pub hints: TextHints,
    /// Up to `MAX_SURROUNDING_BYTES` of the field's text around the caret.
    pub surrounding: String,
    /// The selection within `surrounding`, as byte offsets on character boundaries; equal when
    /// nothing is selected.
    pub anchor: u32,
    pub cursor: u32,
    /// The caret's rectangle, surface-local and logical, once the field has said where it is.
    pub caret: Option<Rect>,
    /// The client's number for this commit; changes say which one they were made for.
    pub serial: u32,
}

impl Default for TextInputState {
    fn default() -> Self {
        Self {
            purpose: TextPurpose::Normal,
            hints: TextHints::default(),
            surrounding: String::new(),
            anchor: 0,
            cursor: 0,
            caret: None,
            serial: 0,
        }
    }
}

/// What a field has described since its last commit.
#[derive(Debug, Default)]
struct Pending {
    enabled: Option<bool>,
    purpose: Option<(TextPurpose, TextHints)>,
    /// Chunks received since the last commit, appended; `None` keeps the committed text.
    surrounding: Option<String>,
    selection: Option<(u32, u32)>,
    caret: Option<Rect>,
}

#[derive(Debug, Default)]
pub(super) struct SurfaceTextInput {
    pending: Pending,
    /// The committed state while text input is enabled.
    committed: Option<TextInputState>,
    /// The latest commit's serial, enabled or not: serials only go up.
    serial: u32,
}

/// One change to a field: delete around its selection, insert `commit` at it, then show
/// `preedit` (empty for none) as the composition, with its cursor range and styled segments in
/// bytes of `preedit`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextTransaction<'a> {
    pub delete_before: u32,
    pub delete_after: u32,
    pub commit: &'a str,
    pub preedit: &'a str,
    pub cursor: (u32, u32),
    pub segments: &'a [(u32, u32, PreeditStyle)],
}

impl TextTransaction<'_> {
    /// Whether the ranges are whole characters of the composition and every limit holds.
    fn is_valid(&self) -> bool {
        let range = |begin: u32, end: u32| {
            let (begin, end) = (begin as usize, end as usize);
            begin <= end
                && end <= self.preedit.len()
                && self.preedit.is_char_boundary(begin)
                && self.preedit.is_char_boundary(end)
        };
        let deletion = MAX_SURROUNDING_BYTES as u32;
        self.preedit.len() <= MAX_PREEDIT_BYTES
            && self.delete_before <= deletion
            && self.delete_after <= deletion
            && range(self.cursor.0, self.cursor.1)
            && self.segments.len() <= MAX_PREEDIT_SEGMENTS
            && self
                .segments
                .iter()
                .all(|&(begin, end, _)| begin < end && range(begin, end))
    }
}

impl Scene {
    fn text_input_entry(&mut self, key: SurfaceKey) -> Result<&mut SurfaceTextInput, ErrorCode> {
        self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        Ok(self.text_inputs.entry(key).or_default())
    }

    pub(super) fn text_input_enable(
        &mut self,
        key: SurfaceKey,
        enabled: bool,
    ) -> Result<(), ErrorCode> {
        let input = self.text_input_entry(key)?;
        // Enabling describes the field anew: what was pending belonged to the one before.
        input.pending = Pending {
            enabled: Some(enabled),
            ..Pending::default()
        };
        Ok(())
    }

    pub(super) fn text_input_purpose(
        &mut self,
        key: SurfaceKey,
        purpose: TextPurpose,
        hints: TextHints,
    ) -> Result<(), ErrorCode> {
        self.text_input_entry(key)?.pending.purpose = Some((purpose, hints));
        Ok(())
    }

    pub(super) fn text_input_surrounding(
        &mut self,
        key: SurfaceKey,
        text: TextChunk,
    ) -> Result<(), ErrorCode> {
        let surrounding = self
            .text_input_entry(key)?
            .pending
            .surrounding
            .get_or_insert_with(String::new);
        if surrounding.len() + text.as_str().len() > MAX_SURROUNDING_BYTES {
            return Err(ErrorCode::QuotaExceeded);
        }
        surrounding.push_str(text.as_str());
        Ok(())
    }

    pub(super) fn text_input_selection(
        &mut self,
        key: SurfaceKey,
        anchor: u32,
        cursor: u32,
    ) -> Result<(), ErrorCode> {
        self.text_input_entry(key)?.pending.selection = Some((anchor, cursor));
        Ok(())
    }

    pub(super) fn text_input_caret(
        &mut self,
        key: SurfaceKey,
        rect: Rect,
    ) -> Result<(), ErrorCode> {
        self.text_input_entry(key)?.pending.caret = Some(rect);
        Ok(())
    }

    /// Applies what the field described. A serial that does not go up is refused, and so is a
    /// selection outside the text or inside a character.
    pub(super) fn text_input_commit(
        &mut self,
        key: SurfaceKey,
        serial: u32,
    ) -> Result<(), ErrorCode> {
        let input = self.text_input_entry(key)?;
        if serial <= input.serial {
            return Err(ErrorCode::InvalidOrder);
        }
        let pending = std::mem::take(&mut input.pending);
        let enabled = pending.enabled.unwrap_or(input.committed.is_some());
        if !enabled {
            input.serial = serial;
            input.committed = None;
            return Ok(());
        }
        // A field enabled anew starts from nothing; one already enabled keeps what it did not
        // describe again.
        let mut state = match (pending.enabled, input.committed.take()) {
            (None, Some(state)) => state,
            _ => TextInputState::default(),
        };
        if let Some((purpose, hints)) = pending.purpose {
            state.purpose = purpose;
            state.hints = hints;
        }
        if let Some(surrounding) = pending.surrounding {
            state.surrounding = surrounding;
        }
        if let Some((anchor, cursor)) = pending.selection {
            state.anchor = anchor;
            state.cursor = cursor;
        }
        if pending.caret.is_some() {
            state.caret = pending.caret;
        }
        let within = |offset: u32| state.surrounding.is_char_boundary(offset as usize);
        if !within(state.anchor) || !within(state.cursor) {
            return Err(ErrorCode::InvalidMessage);
        }
        state.serial = serial;
        input.serial = serial;
        input.committed = Some(state);
        Ok(())
    }

    pub(super) fn forget_text_input(&mut self, key: SurfaceKey) {
        self.text_inputs.remove(&key);
    }

    /// The state a surface last committed while it has text input enabled.
    pub fn text_input(&self, key: SurfaceKey) -> Option<&TextInputState> {
        self.text_inputs.get(&key)?.committed.as_ref()
    }

    /// Delivers one change to the focused surface: as a transaction if it has text input
    /// enabled, otherwise as what `Text` and `Preedit` can say (no deletion, a composition of
    /// one chunk with its cursor's start, else none). Committed text counts as user input, like
    /// a key press. Returns false, delivering nothing, for an invalid change.
    pub fn text_transaction(&mut self, change: &TextTransaction) -> bool {
        if !change.is_valid() {
            return false;
        }
        let Some(target) = self.focus.filter(|f| self.visible(*f)) else {
            return true;
        };
        let surface = target.id;
        let Some(commit) = self.text_input(target).map(|state| state.serial) else {
            self.legacy_text(target, change);
            return true;
        };
        let serial = self.next_serial();
        if !change.commit.is_empty() {
            self.last_user_serial = serial;
            self.record_press(serial, target, false);
        }
        let client = target.client;
        if change.delete_before > 0 || change.delete_after > 0 {
            self.emit(
                client,
                Event::TextInputDelete {
                    surface,
                    before: change.delete_before,
                    after: change.delete_after,
                },
            );
        }
        for text in TextChunk::split(change.commit) {
            self.emit(client, Event::TextInputCommitText { surface, text });
        }
        if !change.preedit.is_empty() {
            for text in TextChunk::split(change.preedit) {
                self.emit(client, Event::TextInputPreeditText { surface, text });
            }
            let (begin, end) = change.cursor;
            self.emit(
                client,
                Event::TextInputPreeditCursor {
                    surface,
                    begin,
                    end,
                },
            );
            for &(begin, end, style) in change.segments {
                self.emit(
                    client,
                    Event::TextInputPreeditSegment {
                        surface,
                        begin,
                        end,
                        style,
                    },
                );
            }
        }
        self.emit(
            client,
            Event::TextInputDone {
                surface,
                serial,
                commit,
            },
        );
        true
    }

    /// A change for a surface without text input: its commit as `Text` chunks, each a user input
    /// as before, then its composition as one `Preedit` when it fits, or an empty one.
    fn legacy_text(&mut self, target: SurfaceKey, change: &TextTransaction) {
        for chunk in TextChunk::split(change.commit) {
            let serial = self.next_serial();
            self.last_user_serial = serial;
            self.record_press(serial, target, false);
            self.emit(
                target.client,
                Event::Text {
                    surface: target.id,
                    serial,
                    text: chunk,
                },
            );
        }
        // Committed text ends a composition by itself; the field shows none unless told.
        if !change.commit.is_empty() && change.preedit.is_empty() {
            return;
        }
        let cursor = u8::try_from(change.cursor.0).ok();
        let (text, cursor) = match (PreeditText::new(change.preedit), cursor) {
            (Some(text), Some(cursor)) => (text, cursor),
            _ => (PreeditText::new("").expect("empty preedit"), 0),
        };
        let serial = self.next_serial();
        self.emit(
            target.client,
            Event::Preedit {
                surface: target.id,
                serial,
                text,
                cursor,
            },
        );
    }
}
