//! What changed in the focused text field between two of its trees, as TalkBack's
//! TextEventInterpreter and VoiceOver's typing echo read it: text typed or deleted at the caret,
//! the text the caret passed over, and the text selected or unselected. Offsets are UTF-8 bytes
//! of the value (ADR 0003), taken down to a character's start where they fall inside one.

use oxyos_speech::TextChange;

/// A text field's value and its selection (start, end), as one tree had them.
pub type Field<'a> = (&'a str, (u32, u32));

/// The change from `before` to `after`, if it is an edit at the caret or a move of the caret or
/// the selection. A value the application replaced elsewhere than at the caret is none.
pub fn change(before: Field, after: Field) -> Option<TextChange> {
    let (old, (s0, e0)) = selection(before);
    let (new, (s1, e1)) = selection(after);
    if old != new {
        return edited(old, (s0, e0), new, (s1, e1)).map(spoken);
    }
    if (s0, e0) == (s1, e1) {
        return None;
    }
    let text = new;
    let change = if s0 == e0 && s1 == e1 {
        let traversed = &text[s0.min(s1)..s0.max(s1)];
        if traversed.chars().nth(1).is_some() && traversed.contains('\n') {
            // A move across lines (Up, Down, a page): the line now under the caret.
            TextChange::Line(line_at(text, s1).to_string())
        } else {
            TextChange::Traversed(traversed.to_string())
        }
    } else if s1 == 0 && e1 == text.len() {
        // Select All: the whole text, selected.
        TextChange::Selected(text.to_string())
    } else if s1 == e1 {
        // The selection collapsed (an arrow, a click): what it held is unselected.
        TextChange::Unselected(text[s0..e0].to_string())
    } else if s1 <= s0 && e0 <= e1 {
        let (left, right) = (&text[s1..s0], &text[e0..e1]);
        if left.is_empty() || right.is_empty() {
            TextChange::Selected(format!("{left}{right}"))
        } else {
            // Grown on both sides (a word selected around the caret): the whole selection.
            TextChange::Selected(text[s1..e1].to_string())
        }
    } else if s0 <= s1 && e1 <= e0 {
        let (left, right) = (&text[s0..s1], &text[e1..e0]);
        TextChange::Unselected(
            [left, right]
                .iter()
                .filter(|piece| !piece.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" "),
        )
    } else {
        // Elsewhere: what is selected now.
        TextChange::Selected(text[s1..e1].to_string())
    };
    Some(spoken(change))
}

/// The change with each run of text as said: a character as it is, more without the spaces
/// around it (a word's, a selection's edge), only spaces as the first of them. A line is said
/// whole.
fn spoken(change: TextChange) -> TextChange {
    use TextChange as C;
    let trim = |text: String| word(&text).to_string();
    match change {
        C::Inserted(text) => C::Inserted(trim(text)),
        C::Deleted(text) => C::Deleted(trim(text)),
        C::Traversed(text) => C::Traversed(trim(text)),
        C::Selected(text) => C::Selected(trim(text)),
        C::Unselected(text) => C::Unselected(trim(text)),
        line @ C::Line(_) => line,
    }
}

/// A field's value with its selection in order and on character boundaries.
fn selection((value, (start, end)): Field) -> (&str, (usize, usize)) {
    let at = |offset: u32| {
        let mut offset = (offset as usize).min(value.len());
        while !value.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    };
    let (start, end) = (at(start), at(end));
    (value, (start.min(end), start.max(end)))
}

/// An edit of the value at the caret: what was typed or pasted over the selection (or at the
/// caret), the caret after it; or what Backspace or Delete took from beside the caret.
fn edited(
    old: &str,
    (s0, e0): (usize, usize),
    new: &str,
    (s1, e1): (usize, usize),
) -> Option<TextChange> {
    if s1 != e1 {
        return None;
    }
    // Typed, pasted or deleted over the selection: the text around it kept, the caret after
    // what replaced it.
    let (head, tail) = (&old[..s0], &old[e0..]);
    if new.starts_with(head) && new.len() >= head.len() + tail.len() && new.ends_with(tail) {
        let end = new.len() - tail.len();
        if s1 == end {
            let typed = &new[s0..end];
            return Some(if typed.is_empty() {
                TextChange::Deleted(old[s0..e0].to_string())
            } else {
                TextChange::Inserted(typed.to_string())
            });
        }
    }
    if s0 != e0 || new.len() >= old.len() {
        return None;
    }
    let caret = s0;
    let gone = old.len() - new.len();
    // Backspace (or a word of it): the text before the caret, the caret where it began.
    if s1 + gone == caret && old.get(..s1) == new.get(..s1) && old.get(caret..) == new.get(s1..) {
        return Some(TextChange::Deleted(old[s1..caret].to_string()));
    }
    // Delete: the text after the caret, which stays.
    if s1 == caret && old.get(..caret) == new.get(..caret) {
        let after = old.get(caret + gone..)?;
        if new.get(caret..) == Some(after) {
            return Some(TextChange::Deleted(old[caret..caret + gone].to_string()));
        }
    }
    None
}

/// The line `at` is on, without its line breaks.
fn line_at(text: &str, at: usize) -> &str {
    let start = text[..at].rfind('\n').map_or(0, |index| index + 1);
    let end = text[at..].find('\n').map_or(text.len(), |index| at + index);
    &text[start..end]
}

/// A run of text as said: a character as it is, more without the spaces around it; only
/// spaces, the first of them.
fn word(text: &str) -> &str {
    if text.chars().nth(1).is_none() {
        return text;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        text.char_indices()
            .nth(1)
            .map_or(text, |(index, _)| &text[..index])
    } else {
        trimmed
    }
}
