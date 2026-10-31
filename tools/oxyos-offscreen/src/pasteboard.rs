//! Another application's clipboard, which the development compositor plays with `--launch`
//! (ADR 0007): a client of the scene only, owning what a `clipboard` step copies for the session
//! (Scene::copy_for_session) and writing it into each paste the application under test makes. After
//! every step the director reads what the clipboard holds, each type through its owner's pipe as
//! any receiver does, and reports it (`clipboard`), so a scenario checks what an application copied.

use oxyos_compositor::{Compositor, Result};
use oxyos_compositor_core::{ClientId, Grants};
use oxyos_protocol::{Event, TextChunk};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

pub const PASTEBOARD: ClientId = ClientId::MAX - 2;
/// A paste the director reads is complete within this time, or reported as unanswered.
const READ_DEADLINE: Duration = Duration::from_secs(5);
/// The most of one type the report reads.
const READ_LIMIT: u64 = 16 << 20;

#[derive(Default)]
pub struct Pasteboard {
    /// What it copied last, each type with its content.
    contents: Vec<(TextChunk, Vec<u8>)>,
    /// The selection last reported, and its report.
    reported: Option<(u32, String)>,
}

/// A pipe read to its end on a thread of its own, so the director keeps serving the owner.
pub struct Reading(std::thread::JoinHandle<Option<Vec<u8>>>, Instant);

impl Reading {
    pub fn done(&self) -> bool {
        self.0.is_finished() || Instant::now() > self.1
    }

    pub fn take(self) -> Option<Vec<u8>> {
        if !self.0.is_finished() {
            return None;
        }
        self.0.join().ok().flatten()
    }
}

impl Pasteboard {
    pub fn connect(compositor: &mut Compositor) -> Result<Self> {
        compositor
            .scene
            .connect(PASTEBOARD, Grants::default())
            .map_err(|code| format!("the pasteboard's connection: {code:?}"))?;
        compositor.local_clients.insert(PASTEBOARD);
        Ok(Self::default())
    }

    /// Copies `contents` as the clipboard, as another application the person copied from.
    pub fn copy(
        &mut self,
        compositor: &mut Compositor,
        contents: Vec<(TextChunk, Vec<u8>)>,
    ) -> Result<()> {
        let types = contents.iter().map(|(mime, _)| *mime).collect();
        compositor
            .scene
            .copy_for_session(PASTEBOARD, types)
            .map_err(|code| format!("the pasteboard's copy: {code:?}"))?;
        self.contents = contents;
        Ok(())
    }

    /// Answers the pastes of what it copied: each pipe gets its type's content, written from a
    /// thread so a reader that is slow stalls nothing.
    pub fn serve(&mut self, compositor: &mut Compositor) {
        let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut compositor.local_events)
            .into_iter()
            .partition(|(client, _)| *client == PASTEBOARD);
        compositor.local_events = others;
        for (_, event) in mine {
            let Event::ClipboardSend { mime, .. } = event else {
                continue;
            };
            let Some(index) = compositor
                .local_pipes
                .iter()
                .position(|(client, _)| *client == PASTEBOARD)
            else {
                continue;
            };
            let (_, pipe) = compositor.local_pipes.remove(index).expect("found");
            let content = self
                .contents
                .iter()
                .find(|(named, _)| *named == mime)
                .map(|(_, bytes)| bytes.clone())
                .unwrap_or_default();
            std::thread::spawn(move || {
                let _ = std::fs::File::from(pipe).write_all(&content);
            });
        }
    }

    /// Starts reading `mime` of what the clipboard holds; None if it holds no such type.
    pub fn read(compositor: &mut Compositor, mime: TextChunk) -> Result<Option<Reading>> {
        let Some(pipe) = compositor.read_clipboard(mime)? else {
            return Ok(None);
        };
        let thread = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            std::fs::File::from(pipe)
                .take(READ_LIMIT)
                .read_to_end(&mut bytes)
                .ok()
                .map(|_| bytes)
        });
        Ok(Some(Reading(thread, Instant::now() + READ_DEADLINE)))
    }

    /// The report kept for `selection`, if it was reported already.
    pub fn reported(&self, selection: u32) -> Option<&str> {
        self.reported
            .as_ref()
            .filter(|(reported, _)| *reported == selection)
            .map(|(_, json)| json.as_str())
    }

    pub fn remember(&mut self, selection: u32, json: String) {
        self.reported = Some((selection, json));
    }
}

/// One type's content as the report gives it: the text, or for other bytes their length and, for
/// a PNG image, its size.
pub fn content_json(bytes: Option<&[u8]>) -> String {
    let Some(bytes) = bytes else {
        return "null".to_string();
    };
    if let Ok(text) = std::str::from_utf8(bytes) {
        return crate::launch::json_string(text);
    }
    let png = bytes
        .strip_prefix(b"\x89PNG\r\n\x1a\n")
        .filter(|rest| rest.len() >= 16 && &rest[4..8] == b"IHDR")
        .map(|rest| {
            let word = |at: usize| {
                u32::from_be_bytes([rest[at], rest[at + 1], rest[at + 2], rest[at + 3]])
            };
            format!(",\"png\":[{},{}]", word(8), word(12))
        })
        .unwrap_or_default();
    format!("{{\"bytes\":{}{png}}}", bytes.len())
}
