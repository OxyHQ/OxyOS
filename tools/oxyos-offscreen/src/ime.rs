//! The input method the development compositor plays with `--input-method` (ADR 0008): a client
//! of the scene only, granted `input_method`, that records what it is told of the focused field
//! and answers the keys it receives. It gives keys back, or (`--compose-keys`) takes them: the
//! text they type becomes its composition, which Enter commits. `--im-change` makes it send a
//! change of its own, for the field's latest state or the one before.

use oxyos_compositor::{Compositor, Result};
use oxyos_compositor_core::{ClientId, Grants};
use oxyos_protocol::{Event, PreeditStyle, Request, TextChunk, TextHints, TextPurpose};

pub const IME: ClientId = ClientId::MAX - 1;
const ENTER: u32 = 28;
const BACKSPACE: u32 = 14;

/// What the input method was last told of the field it serves.
#[derive(Clone, Debug, Default)]
pub struct Told {
    pub active: bool,
    pub purpose: Option<TextPurpose>,
    pub hints: TextHints,
    pub text: String,
    pub anchor: u32,
    pub cursor: u32,
    pub serial: u32,
    /// The state's serial before the latest: what a change for an older state names.
    pub previous: u32,
}

#[derive(Debug, Default)]
pub struct Ime {
    /// Keys are taken and composed (`--compose-keys`), or given back.
    pub takes_keys: bool,
    pub told: Told,
    /// Keys received, and of those given back, since the step began.
    pub keys: usize,
    pub given_back: usize,
    composing: String,
    receiving: String,
    /// Keys whose press it took: it takes their release too.
    taken: std::collections::BTreeSet<u32>,
}

fn send(compositor: &mut Compositor, request: Request) -> Result<()> {
    compositor
        .scene
        .handle(IME, request)
        .map_err(|code| format!("the input method's {request:?}: {code:?}").into())
}

impl Ime {
    pub fn connect(compositor: &mut Compositor) -> Result<Self> {
        compositor
            .scene
            .connect(
                IME,
                Grants {
                    input_method: true,
                    ..Grants::default()
                },
            )
            .map_err(|code| format!("the input method: {code:?}"))?;
        compositor.local_clients.insert(IME);
        Ok(Self::default())
    }

    /// Reads what the scene sent it and answers each key.
    pub fn serve(&mut self, compositor: &mut Compositor) -> Result<()> {
        let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut compositor.local_events)
            .into_iter()
            .partition(|(client, _)| *client == IME);
        compositor.local_events = others;
        let events = mine.into_iter().map(|(_, event)| event);
        for event in events {
            match event {
                Event::InputMethodActivate {} => {
                    self.told = Told {
                        active: true,
                        ..Told::default()
                    };
                    self.composing.clear();
                }
                Event::InputMethodDeactivate {} => {
                    self.told.active = false;
                    self.composing.clear();
                }
                Event::InputMethodPurpose { purpose, hints } => {
                    self.told.purpose = Some(purpose);
                    self.told.hints = hints;
                    self.receiving.clear();
                }
                Event::InputMethodSurrounding { text } => self.receiving.push_str(text.as_str()),
                Event::InputMethodSelection { anchor, cursor } => {
                    self.told.anchor = anchor;
                    self.told.cursor = cursor;
                }
                Event::InputMethodDone { serial } => {
                    self.told.text = std::mem::take(&mut self.receiving);
                    self.told.previous = self.told.serial;
                    self.told.serial = serial;
                }
                Event::InputMethodKey {
                    serial,
                    key,
                    pressed,
                    text,
                    ..
                } => {
                    self.keys += 1;
                    let used =
                        self.takes_keys && self.key(compositor, key, pressed, text.as_str())?;
                    if !used {
                        self.given_back += 1;
                    }
                    send(compositor, Request::InputMethodKeyHandled { serial, used })?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Composes what a key types: Backspace takes back the last character, Enter commits the
    /// composition. Other keys, and every key with nothing composed, are given back, release
    /// with press. Whether the key was taken.
    fn key(
        &mut self,
        compositor: &mut Compositor,
        key: u32,
        pressed: bool,
        text: &str,
    ) -> Result<bool> {
        if !pressed {
            return Ok(self.taken.remove(&key));
        }
        let composing = !self.composing.is_empty();
        let taken = if key == ENTER && composing {
            let composed = std::mem::take(&mut self.composing);
            self.change(compositor, (0, 0), &composed, "", false)?;
            true
        } else if key == BACKSPACE && composing {
            self.composing.pop();
            let composing = self.composing.clone();
            self.change(compositor, (0, 0), "", &composing, false)?;
            true
        } else if !text.is_empty() && key != ENTER {
            self.composing.push_str(text);
            let composing = self.composing.clone();
            self.change(compositor, (0, 0), "", &composing, false)?;
            true
        } else {
            false
        };
        if taken {
            self.taken.insert(key);
        }
        Ok(taken)
    }

    /// Sends one change, for the field's latest state or (`stale`) the one before.
    pub fn change(
        &mut self,
        compositor: &mut Compositor,
        delete: (u32, u32),
        commit: &str,
        preedit: &str,
        stale: bool,
    ) -> Result<()> {
        if !self.told.active {
            return Err("the input method serves no field".into());
        }
        let (before, after) = delete;
        if before > 0 || after > 0 {
            send(compositor, Request::InputMethodDelete { before, after })?;
        }
        for text in TextChunk::split(commit) {
            send(compositor, Request::InputMethodCommitText { text })?;
        }
        for text in TextChunk::split(preedit) {
            send(compositor, Request::InputMethodPreeditText { text })?;
        }
        if !preedit.is_empty() {
            let end = preedit.len() as u32;
            send(
                compositor,
                Request::InputMethodPreeditCursor { begin: end, end },
            )?;
            send(
                compositor,
                Request::InputMethodPreeditSegment {
                    begin: 0,
                    end,
                    style: PreeditStyle::Composing,
                },
            )?;
        }
        let serial = if stale {
            self.told.previous
        } else {
            self.told.serial
        };
        send(compositor, Request::InputMethodCommit { serial })
    }

    /// What it was told, and the keys of this step, as JSON.
    pub fn json(&self) -> String {
        let told = &self.told;
        let purpose = told
            .purpose
            .map_or("null".to_string(), |p| format!("\"{p:?}\""));
        format!(
            "{{\"active\":{},\"purpose\":{purpose},\"hints\":{},\"text\":{},\"anchor\":{},\"cursor\":{},\"serial\":{},\"keys\":{},\"given_back\":{}}}",
            told.active,
            told.hints.bits(),
            crate::launch::json_string(&told.text),
            told.anchor,
            told.cursor,
            told.serial,
            self.keys,
            self.given_back,
        )
    }
}
