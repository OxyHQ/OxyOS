//! Text input (ADR 0008): a client's focused field told to the compositor, and the changes that
//! reach it, each collected into one transaction.

use crate::Client;
use oxyos_protocol::{
    Event, PreeditStyle, Rect, Request, TextChunk, TextHints, TextPurpose, MAX_SURROUNDING_BYTES,
};
use std::collections::BTreeMap;
use std::io;

/// A focused text field: what it is for, its text and selection (byte offsets on character
/// boundaries, the composition left out), and its caret's rectangle in surface coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Field<'a> {
    pub purpose: TextPurpose,
    pub hints: TextHints,
    pub text: &'a str,
    pub anchor: usize,
    pub cursor: usize,
    pub caret: Option<Rect>,
}

/// One change for a field, applied whole: delete `delete_before` bytes before the selection and
/// `delete_after` after it, insert `commit` at it, then show `preedit` as the composition (none
/// if empty) with `cursor` as the selection within it and its styled `segments`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transaction {
    pub surface: u32,
    pub delete_before: u32,
    pub delete_after: u32,
    pub commit: String,
    pub preedit: String,
    pub cursor: (u32, u32),
    pub segments: Vec<(u32, u32, PreeditStyle)>,
    /// The input's serial, as a key press's.
    pub serial: u64,
    /// The field's commit the change was made for (what `Client::text_input` returned).
    pub commit_serial: u32,
}

/// What the compositor holds of a field: the part of its text it was told and the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Told {
    purpose: TextPurpose,
    hints: TextHints,
    surrounding: String,
    anchor: u32,
    cursor: u32,
    caret: Option<Rect>,
}

#[derive(Debug, Default)]
pub(crate) struct TextInputs {
    /// Per surface with text input enabled: what the compositor holds.
    told: BTreeMap<u32, Told>,
    /// Per surface: its latest commit serial, enabled or not (serials only go up).
    serials: BTreeMap<u32, u32>,
    /// The transaction being received, and the one the last event completed.
    building: Option<Transaction>,
    pub(crate) done: Option<Transaction>,
}

/// The part of `text` around the selection the compositor is told (at most
/// `MAX_SURROUNDING_BYTES`, whole characters), with the selection's offsets within it.
fn surrounding(text: &str, anchor: usize, cursor: usize) -> (&str, u32, u32) {
    let (anchor, cursor) = (anchor.min(text.len()), cursor.min(text.len()));
    if text.len() <= MAX_SURROUNDING_BYTES {
        return (text, anchor as u32, cursor as u32);
    }
    let (low, high) = (anchor.min(cursor), anchor.max(cursor));
    // The selection with as much on each side as fits; a selection too long for it keeps the
    // text around the cursor.
    let (mut start, mut end) = if high - low <= MAX_SURROUNDING_BYTES {
        let room = MAX_SURROUNDING_BYTES - (high - low);
        let start = low.saturating_sub(room / 2);
        let end = (start + MAX_SURROUNDING_BYTES).min(text.len());
        (end - MAX_SURROUNDING_BYTES, end)
    } else {
        let start = cursor.saturating_sub(MAX_SURROUNDING_BYTES / 2);
        let end = (start + MAX_SURROUNDING_BYTES).min(text.len());
        (end - MAX_SURROUNDING_BYTES, end)
    };
    while !text.is_char_boundary(start) {
        start += 1;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let within = |offset: usize| {
        let mut offset = offset.clamp(start, end);
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        (offset - start) as u32
    };
    (&text[start..end], within(anchor), within(cursor))
}

impl Client {
    /// Tells the compositor about the field focused in `surface`, or that none is (`None`):
    /// what changed since it was last told, committed. `new_field` describes another field than
    /// the one told before (focus moved between two inputs of the surface). Returns the commit
    /// serial transactions for this state carry; 0 once disabled.
    pub fn text_input(
        &mut self,
        surface: u32,
        field: Option<&Field<'_>>,
        new_field: bool,
    ) -> io::Result<u32> {
        let inputs = &mut self.text_inputs;
        let serial = inputs.serials.get(&surface).copied().unwrap_or(0);
        let Some(field) = field else {
            if inputs.told.remove(&surface).is_none() {
                return Ok(0);
            }
            self.send(&Request::TextInputDisable { surface })?;
            self.commit_text_input(surface, serial + 1)?;
            return Ok(0);
        };
        let (text, anchor, cursor) = surrounding(field.text, field.anchor, field.cursor);
        let told = Told {
            purpose: field.purpose,
            hints: field.hints,
            surrounding: text.to_string(),
            anchor,
            cursor,
            caret: field.caret,
        };
        let before = if new_field {
            None
        } else {
            inputs.told.get(&surface).cloned()
        };
        if before.as_ref() == Some(&told) {
            return Ok(serial);
        }
        let mut requests = Vec::new();
        if before.is_none() {
            requests.push(Request::TextInputEnable { surface });
        }
        if before
            .as_ref()
            .is_none_or(|b| (b.purpose, b.hints) != (told.purpose, told.hints))
        {
            requests.push(Request::TextInputPurpose {
                surface,
                purpose: told.purpose,
                hints: told.hints,
            });
        }
        let text_changed = before
            .as_ref()
            .is_none_or(|b| b.surrounding != told.surrounding);
        if text_changed {
            for text in TextChunk::split(&told.surrounding) {
                requests.push(Request::TextInputSurrounding { surface, text });
            }
        }
        if text_changed
            || before
                .as_ref()
                .is_none_or(|b| (b.anchor, b.cursor) != (anchor, cursor))
        {
            requests.push(Request::TextInputSelection {
                surface,
                anchor,
                cursor,
            });
        }
        // A caret the field no longer knows keeps the last one told.
        if let Some(rect) = told
            .caret
            .filter(|_| before.as_ref().is_none_or(|b| b.caret != told.caret))
        {
            requests.push(Request::TextInputCaret { surface, rect });
        }
        for request in &requests {
            self.send(request)?;
        }
        self.commit_text_input(surface, serial + 1)?;
        self.text_inputs.told.insert(surface, told);
        Ok(serial + 1)
    }

    fn commit_text_input(&mut self, surface: u32, serial: u32) -> io::Result<()> {
        self.send(&Request::TextInputCommit { surface, serial })?;
        self.text_inputs.serials.insert(surface, serial);
        Ok(())
    }

    /// The transaction the event last returned completed (a TextInputDone), if any. It is
    /// dropped with the next event.
    pub fn transaction(&self) -> Option<&Transaction> {
        self.text_inputs.done.as_ref()
    }

    /// A surface destroyed: the compositor forgets its field, serials included.
    pub(crate) fn forget_text_input(&mut self, surface: u32) {
        self.text_inputs.told.remove(&surface);
        self.text_inputs.serials.remove(&surface);
    }

    /// Collects a transaction's parts; its Done completes it.
    pub(crate) fn observe_text_input(&mut self, event: &Event) {
        let inputs = &mut self.text_inputs;
        inputs.done = None;
        let surface = match *event {
            Event::TextInputDelete { surface, .. }
            | Event::TextInputCommitText { surface, .. }
            | Event::TextInputPreeditText { surface, .. }
            | Event::TextInputPreeditCursor { surface, .. }
            | Event::TextInputPreeditSegment { surface, .. }
            | Event::TextInputDone { surface, .. } => surface,
            _ => return,
        };
        // Parts for another surface begin another transaction.
        let building = match inputs.building.take() {
            Some(building) if building.surface == surface => building,
            _ => Transaction {
                surface,
                ..Transaction::default()
            },
        };
        let mut building = building;
        match *event {
            Event::TextInputDelete { before, after, .. } => {
                building.delete_before = before;
                building.delete_after = after;
            }
            Event::TextInputCommitText { text, .. } => building.commit.push_str(text.as_str()),
            Event::TextInputPreeditText { text, .. } => building.preedit.push_str(text.as_str()),
            Event::TextInputPreeditCursor { begin, end, .. } => building.cursor = (begin, end),
            Event::TextInputPreeditSegment {
                begin, end, style, ..
            } => building.segments.push((begin, end, style)),
            Event::TextInputDone { serial, commit, .. } => {
                building.serial = serial;
                building.commit_serial = commit;
                inputs.done = Some(building);
                return;
            }
            _ => unreachable!("matched above"),
        }
        inputs.building = Some(building);
    }
}
