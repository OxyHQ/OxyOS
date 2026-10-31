//! File choosers (docs/adr/0029-chromium-on-oxyos.mdx, decision 8): the open and save panels.
//!
//! Any application asks the session to let the person choose files for one of its windows
//! (`ChooseFiles`), citing a press or a key it received lately; it needs no grant, since what it
//! reaches is what the person chooses. The scene checks the window and the input and hands the
//! request whole to the program running the compositor, which reads the session's vocabulary,
//! starts the file manager in its picker mode (the chooser) and tells the scene which client that
//! is (`attach_chooser`). The chooser's first window becomes a sheet of the asking window: a dialog
//! of another client, placed at the window's top, moving with it, taking its keyboard focus and
//! keeping its presses from it, while every other window stays usable. Only that chooser answers
//! (`AnswerFileChooser`), a choice only citing an input it received; the program checks the answer
//! and the scene gives it to the asker alone (`FilesChosen`), once, and takes the sheet away. A
//! request ends answered, withdrawn by its asker, or cancelled when the window, the asker or the
//! chooser goes; the program ends the chooser's process for each that ends. The scene knows no
//! vocabulary and no file.

use super::{ClientId, DismissReason, Role, Scene, SurfaceKey};
use oxyos_protocol::{
    ErrorCode, Event, PreeditText, Request, Size, TextChunk, MAX_CHOOSER_TEXT_BYTES,
};
use std::collections::BTreeMap;

/// Requests open at once, across the session: each is a process of the file manager's.
pub const MAX_CHOOSERS: usize = 8;
/// A sheet's top, below the asking window's top (its toolbar's height), in logical pixels.
pub const SHEET_TOP: u32 = 52;
/// The room a sheet leaves beside and below it within its window, in logical pixels.
pub const SHEET_MARGIN: u32 = 24;
/// The smallest a sheet is made to fit its window: a window smaller than this has it overhang.
pub const SHEET_MIN: Size = Size::new(420, 300);

/// A request to choose files, for the program running the compositor: the number the scene gave
/// it (what `attach_chooser`, `answer_files` and the answers name), whose it is and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRequest {
    pub id: u32,
    pub client: ClientId,
    pub text: String,
}

/// The chooser's answer to request `id`, for the program to check: `with_input` whether it cited
/// an input the chooser received lately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChooserAnswer {
    pub id: u32,
    pub with_input: bool,
    pub text: String,
}

#[derive(Debug)]
struct Chooser {
    asker: ClientId,
    serial: u32,
    window: SurfaceKey,
    /// The client serving it, once the program said which (`attach_chooser`).
    chooser: Option<ClientId>,
    sheet: Option<SurfaceKey>,
}

#[derive(Debug, Default)]
pub(super) struct Choosers {
    /// Requests being received, by asker and serial: the window, the input and the text so far.
    receiving: BTreeMap<(ClientId, u32), (u32, u64, String, u16)>,
    /// Answers being received, by chooser: the input and the text so far.
    answering: BTreeMap<ClientId, (u64, String, u16)>,
    open: BTreeMap<u32, Chooser>,
    next: u32,
    asked: Vec<FileRequest>,
    answers: Vec<ChooserAnswer>,
    ended: Vec<u32>,
}

/// `text` in chunks of at most a chunk's bytes, cut on character boundaries.
fn chunks(text: &str) -> Vec<PreeditText> {
    TextChunk::split(text)
        .into_iter()
        .map(|chunk| PreeditText::new(chunk.as_str()).expect("a chunk fits"))
        .collect()
}

/// Appends a chunk to text received in chunks, as the protocol numbers them (`index` 0 begins it);
/// whether it is whole. A chunk out of order or text past the limit is a protocol error.
fn append(text: &mut String, next: &mut u16, index: u16, chunk: &str) -> Result<(), ErrorCode> {
    if index == 0 {
        text.clear();
    } else if index != *next {
        return Err(ErrorCode::InvalidOrder);
    }
    if text.len() + chunk.len() > MAX_CHOOSER_TEXT_BYTES {
        return Err(ErrorCode::QuotaExceeded);
    }
    text.push_str(chunk);
    *next = index.wrapping_add(1);
    Ok(())
}

impl Scene {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn choose_files(
        &mut self,
        client: ClientId,
        serial: u32,
        surface: u32,
        input: u64,
        index: u16,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        let choosers = &mut self.choosers;
        let entry = choosers
            .receiving
            .entry((client, serial))
            .or_insert_with(|| (surface, input, String::new(), 0));
        if index == 0 {
            *entry = (surface, input, String::new(), 0);
        }
        let (window, cited, received, next) = entry;
        if (*window, *cited) != (surface, input) {
            choosers.receiving.remove(&(client, serial));
            return Err(ErrorCode::InvalidMessage);
        }
        if let Err(code) = append(received, next, index, text.as_str()) {
            choosers.receiving.remove(&(client, serial));
            return Err(code);
        }
        if !last {
            return Ok(());
        }
        let (_, _, text, _) = choosers
            .receiving
            .remove(&(client, serial))
            .expect("received");
        let window = SurfaceKey {
            client,
            id: surface,
        };
        let node = self.nodes.get(&window).ok_or(ErrorCode::UnknownSurface)?;
        if node.role != Role::Toplevel {
            return Err(ErrorCode::InvalidRole);
        }
        // Refused at once, with no panel: what the person did not ask for now, a window that is
        // gone or already has one, the lock, or too many open.
        let refusal = if node.dismissed {
            Some("invalidWindow")
        } else if self.locked {
            Some("locked")
        } else if input == 0 || !self.recent_input(client, input) {
            Some("needsInput")
        } else if self
            .choosers
            .open
            .values()
            .any(|open| open.window == window)
            || self.choosers.open.len() >= MAX_CHOOSERS
        {
            Some("busy")
        } else if self
            .choosers
            .open
            .values()
            .any(|open| open.asker == client && open.serial == serial)
        {
            return Err(ErrorCode::InvalidOrder);
        } else {
            None
        };
        if let Some(reason) = refusal {
            let answer = format!(r#"{{"refused":{{"reason":"{reason}"}}}}"#);
            self.emit_files_chosen(client, serial, &answer);
            return Ok(());
        }
        let choosers = &mut self.choosers;
        choosers.next = choosers.next.wrapping_add(1).max(1);
        let id = choosers.next;
        choosers.open.insert(
            id,
            Chooser {
                asker: client,
                serial,
                window,
                chooser: None,
                sheet: None,
            },
        );
        choosers.asked.push(FileRequest { id, client, text });
        Ok(())
    }

    pub(super) fn cancel_choose_files(&mut self, client: ClientId, serial: u32) {
        let id = self
            .choosers
            .open
            .iter()
            .find(|(_, open)| open.asker == client && open.serial == serial)
            .map(|(id, _)| *id);
        self.choosers.receiving.remove(&(client, serial));
        if let Some(id) = id {
            self.answer_files(id, r#"{"cancelled":{}}"#);
        }
    }

    pub(super) fn answer_file_chooser(
        &mut self,
        client: ClientId,
        input: u64,
        index: u16,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        let Some(id) = self.chooser_of(client) else {
            return Err(ErrorCode::Unauthorized);
        };
        let choosers = &mut self.choosers;
        let entry = choosers
            .answering
            .entry(client)
            .or_insert_with(|| (input, String::new(), 0));
        if index == 0 {
            *entry = (input, String::new(), 0);
        }
        let (cited, received, next) = entry;
        if *cited != input {
            choosers.answering.remove(&client);
            return Err(ErrorCode::InvalidMessage);
        }
        if let Err(code) = append(received, next, index, text.as_str()) {
            choosers.answering.remove(&client);
            return Err(code);
        }
        if !last {
            return Ok(());
        }
        let (_, text, _) = choosers.answering.remove(&client).expect("received");
        let with_input = input != 0 && self.recent_input(client, input);
        self.choosers.answers.push(ChooserAnswer {
            id,
            with_input,
            text,
        });
        Ok(())
    }

    /// The requests asked for since the last call, in order.
    pub fn take_file_requests(&mut self) -> Vec<FileRequest> {
        std::mem::take(&mut self.choosers.asked)
    }

    /// The choosers' answers since the last call, in order.
    pub fn take_chooser_answers(&mut self) -> Vec<ChooserAnswer> {
        std::mem::take(&mut self.choosers.answers)
    }

    /// The requests that ended since the last call (answered, withdrawn, or their window, asker or
    /// chooser gone): the program ends their choosers' processes.
    pub fn take_ended_choosers(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.choosers.ended)
    }

    /// Whether request `id` is still open.
    pub fn chooser_open(&self, id: u32) -> bool {
        self.choosers.open.contains_key(&id)
    }

    /// The asker of request `id` and the window it asked for, while it is open.
    pub fn chooser_asker(&self, id: u32) -> Option<(ClientId, SurfaceKey)> {
        self.choosers
            .open
            .get(&id)
            .map(|open| (open.asker, open.window))
    }

    /// `client` is the chooser the program started for request `id`: its first window becomes the
    /// asking window's sheet, and it alone answers. False if the request is no longer open or has
    /// a chooser, or the client serves another.
    pub fn attach_chooser(&mut self, client: ClientId, id: u32) -> bool {
        if !self.clients.contains_key(&client) || self.chooser_of(client).is_some() {
            return false;
        }
        match self.choosers.open.get_mut(&id) {
            Some(open) if open.chooser.is_none() && open.asker != client => {
                open.chooser = Some(client);
                true
            }
            _ => false,
        }
    }

    /// The request `client` serves as its chooser, if any.
    pub fn chooser_of(&self, client: ClientId) -> Option<u32> {
        self.choosers
            .open
            .iter()
            .find(|(_, open)| open.chooser == Some(client))
            .map(|(id, _)| *id)
    }

    /// The sheet of request `id`, once its chooser made its window.
    pub fn chooser_sheet(&self, id: u32) -> Option<SurfaceKey> {
        self.choosers.open.get(&id).and_then(|open| open.sheet)
    }

    /// Gives the asker of request `id` its answer (`text`, the session's), once, and ends it: its
    /// sheet goes at once, and the program ends its chooser.
    pub fn answer_files(&mut self, id: u32, text: &str) {
        let Some(open) = self.choosers.open.remove(&id) else {
            return;
        };
        if let Some(chooser) = open.chooser {
            self.choosers.answering.remove(&chooser);
        }
        self.choosers.ended.push(id);
        if self.clients.contains_key(&open.asker) && text.len() <= MAX_CHOOSER_TEXT_BYTES {
            self.emit_files_chosen(open.asker, open.serial, text);
        }
        if let Some(sheet) = open.sheet {
            self.dismiss(sheet, DismissReason::Superseded);
        }
    }

    fn emit_files_chosen(&mut self, client: ClientId, serial: u32, text: &str) {
        let parts = chunks(text);
        let count = parts.len();
        for (index, text) in parts.into_iter().enumerate() {
            self.emit(
                client,
                Event::FilesChosen {
                    serial,
                    index: index as u16,
                    last: index + 1 == count,
                    text,
                },
            );
        }
    }

    /// The window a chooser's toplevel is a sheet of, if `client` is a chooser that has not made
    /// its sheet yet and the window is still there: where and how large the sheet is, relative to
    /// the window. Records the sheet.
    pub(super) fn sheet_for(
        &mut self,
        key: SurfaceKey,
        asked: Size,
    ) -> Option<(SurfaceKey, (i32, i32), Size)> {
        let id = self.chooser_of(key.client)?;
        let open = &self.choosers.open[&id];
        if open.sheet.is_some() {
            return None;
        }
        let window = open.window;
        let parent = self.live(window)?.upcoming().1;
        let fit = |asked: u32, room: u32, least: u32| asked.min(room.max(least.min(asked)));
        let size = Size::new(
            fit(
                asked.width,
                parent.width.saturating_sub(2 * SHEET_MARGIN),
                SHEET_MIN.width,
            ),
            fit(
                asked.height,
                parent.height.saturating_sub(SHEET_TOP + SHEET_MARGIN),
                SHEET_MIN.height,
            ),
        );
        let x = (i64::from(parent.width) - i64::from(size.width)) / 2;
        let y = i64::from(SHEET_TOP.min(parent.height.saturating_sub(size.height)));
        self.choosers.open.get_mut(&id).expect("open").sheet = Some(key);
        Some((window, (x as i32, y as i32), size))
    }

    /// A surface went: a request whose window it was is cancelled, and a sheet that was it leaves
    /// its request without one (the chooser answers no more through it).
    pub(super) fn forget_chooser_surface(&mut self, key: SurfaceKey) {
        let windowed: Vec<u32> = self
            .choosers
            .open
            .iter()
            .filter(|(_, open)| open.window == key || open.sheet == Some(key))
            .map(|(id, _)| *id)
            .collect();
        for id in windowed {
            self.answer_files(id, r#"{"cancelled":{}}"#);
        }
    }

    /// A client went: its requests end (asked: answered nothing, it is gone; served: cancelled).
    pub(super) fn forget_choosers(&mut self, client: ClientId) {
        let choosers = &mut self.choosers;
        choosers.receiving.retain(|(asker, _), _| *asker != client);
        choosers.answering.remove(&client);
        choosers.asked.retain(|request| request.client != client);
        choosers.answers.retain(|answer| {
            choosers
                .open
                .get(&answer.id)
                .is_some_and(|open| open.asker != client && open.chooser != Some(client))
        });
        let theirs: Vec<u32> = choosers
            .open
            .iter()
            .filter(|(_, open)| open.asker == client || open.chooser == Some(client))
            .map(|(id, _)| *id)
            .collect();
        for id in theirs {
            self.answer_files(id, r#"{"cancelled":{}}"#);
        }
    }

    /// Whether `key` is a file chooser's sheet: a dialog of another client's window.
    pub(super) fn is_sheet(&self, key: SurfaceKey) -> bool {
        self.nodes.get(&key).is_some_and(|node| {
            node.role == Role::Dialog
                && node
                    .parent
                    .is_some_and(|parent| parent.client != key.client)
        })
    }

    /// A toplevel's requests about its own window, from a chooser whose window is a sheet: to its
    /// client it is a window, so none is an error. A sheet has no title, application id or menus
    /// of its own (its window's stand), takes no limits, is not zoomed, made full screen or
    /// minimized; a drag of it (BeginMove with a press of its own) moves the window it is attached
    /// to, whose own presses the sheet keeps from it. Any other request is not one of these.
    pub(super) fn sheet_request(
        &mut self,
        client: ClientId,
        request: &Request,
    ) -> Option<Result<(), ErrorCode>> {
        let surface = match *request {
            Request::SetTitle { surface, .. }
            | Request::SetAppId { surface, .. }
            | Request::SetSizeLimits { surface, .. }
            | Request::SetMaximized { surface, .. }
            | Request::SetFullscreen { surface, .. }
            | Request::Minimize { surface }
            | Request::MenusBegin { surface, .. }
            | Request::MenuItem { surface, .. }
            | Request::MenuText { surface, .. }
            | Request::MenusCommit { surface }
            | Request::BeginMove { surface, .. } => surface,
            _ => return None,
        };
        let sheet = SurfaceKey {
            client,
            id: surface,
        };
        if !self.is_sheet(sheet) {
            return None;
        }
        let Request::BeginMove { serial, .. } = *request else {
            return Some(Ok(()));
        };
        let window = self.root_of(sheet);
        let Some(node) = self.live(window) else {
            return Some(Ok(()));
        };
        let valid = self.last_press.is_some_and(|press| {
            press.serial == serial
                && press.client == client
                && press.pointer
                && press.surface == sheet
        }) && self.buttons_down > 0
            && node.mapped
            && !node.fullscreen
            && !self.locked;
        if valid {
            self.moving = Some(super::Move {
                surface: window,
                pointer: self.pointer,
                origin: node.origin,
            });
            self.set_pointer_focus(None);
        }
        Some(Ok(()))
    }
}
