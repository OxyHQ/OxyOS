//! The screen reader's keys (docs/adr/0014-screen-reader.mdx): Ctrl+Alt with Right, Left or
//! Space are its commands while a client holding `screen_reader` is connected. The compositor
//! takes them from the keyboard before shortcuts, the input method and the focused surface, and
//! sends them to the reader alone; the press's release is taken with it. Without a reader they
//! are ordinary keys.

use super::{ClientId, Scene};
use oxyos_protocol::{Event, ReaderCommand};
use std::collections::BTreeSet;

const LEFT_CTRL: u32 = 29;
const RIGHT_CTRL: u32 = 97;
/// Left Alt only: on many layouts (Spanish among them) right Alt is AltGr, which types.
const LEFT_ALT: u32 = 56;
const SPACE: u32 = 57;
const LEFT: u32 = 105;
const RIGHT: u32 = 106;

#[derive(Debug, Default)]
pub(super) struct ReaderKeys {
    /// The modifiers held, by their codes.
    held: BTreeSet<u32>,
    /// Keys whose press was the reader's: their release is the reader's too.
    taken: BTreeSet<u32>,
}

impl ReaderKeys {
    fn chord(&self) -> bool {
        (self.held.contains(&LEFT_CTRL) || self.held.contains(&RIGHT_CTRL))
            && self.held.contains(&LEFT_ALT)
    }
}

impl Scene {
    /// The connected client holding `screen_reader`, if any.
    pub fn screen_reader(&self) -> Option<ClientId> {
        self.clients
            .iter()
            .find(|(_, grants)| grants.screen_reader)
            .map(|(client, _)| *client)
    }

    /// Whether a press of `key` now would be the screen reader's and, if so, whether it repeats
    /// while held (docs/adr/0036-key-repeat.mdx): moving (Right, Left) does, activating never.
    pub(super) fn reader_repeats(&self, key: u32) -> Option<bool> {
        let repeats = match key {
            RIGHT | LEFT => true,
            SPACE => false,
            _ => return None,
        };
        (self.reader_keys.chord() && self.screen_reader().is_some()).then_some(repeats)
    }

    /// Whether a key from the keyboard is the screen reader's: then it went to the reader, as a
    /// command for a press, and nothing else sees it.
    pub(super) fn reader_key(&mut self, key: u32, pressed: bool) -> bool {
        let keys = &mut self.reader_keys;
        if matches!(key, LEFT_CTRL | RIGHT_CTRL | LEFT_ALT) {
            if pressed {
                keys.held.insert(key);
            } else {
                keys.held.remove(&key);
            }
            return false;
        }
        if !pressed {
            return keys.taken.remove(&key);
        }
        let command = match key {
            RIGHT => ReaderCommand::Next,
            LEFT => ReaderCommand::Previous,
            SPACE => ReaderCommand::Activate,
            _ => return false,
        };
        if !keys.chord() {
            return false;
        }
        let Some(reader) = self.screen_reader() else {
            return false;
        };
        self.reader_keys.taken.insert(key);
        self.emit(reader, Event::ScreenReaderCommand { command });
        true
    }
}
