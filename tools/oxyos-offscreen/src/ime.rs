//! The input method the development compositor plays with `--input-method` (ADR 0008): a client
//! of the scene only, granted `input_method`, that records what it is told of the focused field
//! and answers the keys it receives. It gives keys back, or (`--compose-keys`) takes them: the
//! text they type becomes its composition, which Enter commits. `--im-change` makes it send a
//! change of its own, for the field's latest state or the one before.
//!
//! With `--im-candidates horizontal|vertical` it also hands the system candidates for what it
//! composes (ADR 0041), as an engine that draws nothing does: a fixed list of twelve, five a page
//! (`CANDIDATES`), which the session's shell draws at the caret. While it composes, the arrows
//! move the highlight along the list's direction and turn the page across it, Page Up and Page
//! Down turn it, a digit chooses that candidate of the page (1 to 9, then 0), Space the
//! highlighted one, Escape cancels; and the shell's presses choose a candidate or turn the page.
//! A candidate chosen is committed in place of the composition.

use oxyos_compositor::{Compositor, Result};
use oxyos_compositor_core::{ClientId, Grants};
use oxyos_protocol::{
    Event, PreeditStyle, Request, TextChunk, TextHints, TextPurpose, CANDIDATES_NEXT,
    CANDIDATES_PREVIOUS, CANDIDATES_VERTICAL,
};

pub const IME: ClientId = ClientId::MAX - 1;
const ENTER: u32 = 28;
const BACKSPACE: u32 = 14;
const ESCAPE: u32 = 1;
const SPACE: u32 = 57;
const UP: u32 = 103;
const PAGE_UP: u32 = 104;
const LEFT: u32 = 105;
const RIGHT: u32 = 106;
const DOWN: u32 = 108;
const PAGE_DOWN: u32 = 109;
/// The digit keys, 1 to 9 then 0: the candidate of the page each chooses.
const DIGITS: std::ops::RangeInclusive<u32> = 2..=11;

/// What it offers for any composition: twelve candidates, five a page, an emoji among them as
/// fcitx5's pinyin offers one.
pub const CANDIDATES: [&str; 12] = [
    "你好", "👋", "拟好", "你号", "倪浩", "你", "尼", "妮", "霓", "呢", "拟", "逆",
];
pub const PAGE: usize = 5;

/// How it lays out the candidates it hands the system (`--im-candidates`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Horizontal,
    Vertical,
}

impl Layout {
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "horizontal" => Some(Self::Horizontal),
            "vertical" => Some(Self::Vertical),
            _ => None,
        }
    }
}

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
    /// `--im-candidates`: it hands the system candidates for its composition, so laid out.
    pub candidates: Option<Layout>,
    /// The page shown and the candidate highlighted on it, while a list is shown.
    page: usize,
    highlighted: usize,
    shown: bool,
    /// The shell's presses since the step began: the candidates chosen (their index on the page)
    /// and the pages asked for (forward or not).
    pub chosen: Vec<u8>,
    pub turned: Vec<bool>,
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
                    // The scene forgets the list a field it no longer serves had.
                    self.shown = false;
                }
                Event::InputMethodCandidateChosen { index } => {
                    self.chosen.push(index);
                    self.choose(compositor, usize::from(index))?;
                }
                Event::InputMethodCandidatesPage { forward } => {
                    self.turned.push(forward);
                    self.turn(compositor, forward)?;
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
        if composing && self.candidates.is_some() {
            if let Some(taken) = self.candidate_key(compositor, key)? {
                if taken {
                    self.taken.insert(key);
                }
                return Ok(taken);
            }
        }
        let taken = if key == ENTER && composing {
            let composed = std::mem::take(&mut self.composing);
            self.change(compositor, (0, 0), &composed, "", false)?;
            self.hand_candidates(compositor)?;
            true
        } else if key == BACKSPACE && composing {
            self.composing.pop();
            let composing = self.composing.clone();
            self.change(compositor, (0, 0), "", &composing, false)?;
            (self.page, self.highlighted) = (0, 0);
            self.hand_candidates(compositor)?;
            true
        } else if !text.is_empty() && key != ENTER {
            self.composing.push_str(text);
            let composing = self.composing.clone();
            self.change(compositor, (0, 0), "", &composing, false)?;
            (self.page, self.highlighted) = (0, 0);
            self.hand_candidates(compositor)?;
            true
        } else {
            false
        };
        if taken {
            self.taken.insert(key);
        }
        Ok(taken)
    }

    /// A key while it composes with candidates shown: `Some(taken)` for one of the candidates'
    /// keys, `None` for the others (letters compose, Enter commits, Backspace takes back).
    fn candidate_key(&mut self, compositor: &mut Compositor, key: u32) -> Result<Option<bool>> {
        let vertical = self.candidates == Some(Layout::Vertical);
        let (back, ahead, page_back, page_ahead) = if vertical {
            (UP, DOWN, LEFT, RIGHT)
        } else {
            (LEFT, RIGHT, UP, DOWN)
        };
        let on_page = self.page_candidates().len();
        match key {
            _ if key == back || key == ahead => {
                let highlighted = if key == back {
                    self.highlighted.saturating_sub(1)
                } else {
                    (self.highlighted + 1).min(on_page - 1)
                };
                if highlighted != self.highlighted {
                    self.highlighted = highlighted;
                    self.hand_candidates(compositor)?;
                }
            }
            _ if [page_back, PAGE_UP].contains(&key) => self.turn(compositor, false)?,
            _ if [page_ahead, PAGE_DOWN].contains(&key) => self.turn(compositor, true)?,
            _ if DIGITS.contains(&key) => {
                let index = (key - DIGITS.start()) as usize;
                if index < on_page {
                    self.choose(compositor, index)?;
                }
            }
            SPACE => self.choose(compositor, self.highlighted)?,
            ESCAPE => {
                self.composing.clear();
                self.change(compositor, (0, 0), "", "", false)?;
                self.hand_candidates(compositor)?;
            }
            _ => return Ok(None),
        }
        Ok(Some(true))
    }

    /// The candidates of the page shown.
    fn page_candidates(&self) -> &'static [&'static str] {
        let start = self.page * PAGE;
        &CANDIDATES[start..(start + PAGE).min(CANDIDATES.len())]
    }

    /// The candidate at `index` of the page chosen: committed in place of the composition.
    fn choose(&mut self, compositor: &mut Compositor, index: usize) -> Result<()> {
        let Some(candidate) = self.page_candidates().get(index).copied() else {
            return Err(format!("no candidate {index} on page {}", self.page).into());
        };
        if self.composing.is_empty() {
            return Err("a candidate chosen with nothing composed".into());
        }
        self.composing.clear();
        self.change(compositor, (0, 0), candidate, "", false)?;
        self.hand_candidates(compositor)
    }

    /// The page after (`forward`) or before, if there is one, its first candidate highlighted.
    fn turn(&mut self, compositor: &mut Compositor, forward: bool) -> Result<()> {
        let pages = CANDIDATES.len().div_ceil(PAGE);
        let page = if forward {
            (self.page + 1).min(pages - 1)
        } else {
            self.page.saturating_sub(1)
        };
        if page != self.page {
            (self.page, self.highlighted) = (page, 0);
            self.hand_candidates(compositor)?;
        }
        Ok(())
    }

    /// Hands the system the page of candidates for what it composes, or (composing nothing) an
    /// empty list that hides them, when it hands any.
    fn hand_candidates(&mut self, compositor: &mut Compositor) -> Result<()> {
        let Some(layout) = self.candidates else {
            return Ok(());
        };
        if self.composing.is_empty() {
            if self.shown {
                self.shown = false;
                (self.page, self.highlighted) = (0, 0);
                send(
                    compositor,
                    Request::InputMethodCandidates {
                        count: 0,
                        highlighted: oxyos_protocol::NO_CANDIDATE,
                        flags: 0,
                    },
                )?;
            }
            return Ok(());
        }
        let page = self.page_candidates();
        for (index, candidate) in page.iter().enumerate() {
            for text in TextChunk::split(candidate) {
                send(
                    compositor,
                    Request::InputMethodCandidate {
                        index: index as u8,
                        text,
                    },
                )?;
            }
        }
        // The reading: what it composes.
        for text in TextChunk::split(&self.composing) {
            send(compositor, Request::InputMethodCandidatesAux { text })?;
        }
        let mut flags = 0;
        if self.page > 0 {
            flags |= CANDIDATES_PREVIOUS;
        }
        if (self.page + 1) * PAGE < CANDIDATES.len() {
            flags |= CANDIDATES_NEXT;
        }
        if layout == Layout::Vertical {
            flags |= CANDIDATES_VERTICAL;
        }
        send(
            compositor,
            Request::InputMethodCandidates {
                count: page.len() as u8,
                highlighted: self.highlighted as u8,
                flags,
            },
        )?;
        self.shown = true;
        Ok(())
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

    /// What it was told, and the keys of this step, as JSON; with `--im-candidates`, the page it
    /// shows ("candidates": null while none, else the page, the highlighted one and its texts) and
    /// the shell's presses in the step.
    pub fn json(&self) -> String {
        let told = &self.told;
        let purpose = told
            .purpose
            .map_or("null".to_string(), |p| format!("\"{p:?}\""));
        let candidates = if self.shown {
            let texts: Vec<String> = self
                .page_candidates()
                .iter()
                .map(|text| crate::launch::json_string(text))
                .collect();
            format!(
                "{{\"page\":{},\"highlighted\":{},\"list\":[{}]}}",
                self.page,
                self.highlighted,
                texts.join(",")
            )
        } else {
            "null".to_string()
        };
        format!(
            "{{\"active\":{},\"purpose\":{purpose},\"hints\":{},\"text\":{},\"anchor\":{},\"cursor\":{},\"serial\":{},\"keys\":{},\"given_back\":{},\"composing\":{},\"candidates\":{candidates},\"chosen\":{:?},\"turned\":{:?}}}",
            told.active,
            told.hints.bits(),
            crate::launch::json_string(&told.text),
            told.anchor,
            told.cursor,
            told.serial,
            self.keys,
            self.given_back,
            crate::launch::json_string(&self.composing),
            self.chosen,
            self.turned,
        )
    }
}
