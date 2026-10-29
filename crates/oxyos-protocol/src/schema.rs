//! The authoritative message schema. `docs/display-protocol.mdx` documents the same table; the
//! `schema_table_matches_documentation` test keeps them in step.

use crate::wire::{
    AccessibleAction, AccessibleField, AccessibleRole, AccessibleState, Clock, DismissReason,
    ErrorCode, Format, GesturePhase, Outcome, PanelEdge, PointerKind, PointerShape, Positioner,
    PreeditStyle, PreeditText, Rect, ShellKind, Size, TextChunk, TextHints, TextPurpose, TouchKind,
    Wire,
};
use crate::{invalid, MAX_PACKET_BYTES};
use std::io;

macro_rules! messages {
    ($(#[$meta:meta])* $name:ident { $($opcode:literal => $variant:ident { $($field:ident: $type:ty),* $(,)? }),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name {
            $($variant { $($field: $type),* }),*
        }

        impl $name {
            /// (opcode, name, payload bytes including the opcode) for every message.
            pub const SCHEMA: &'static [(u8, &'static str, usize)] = &[
                $(($opcode, stringify!($variant), 1 $(+ <$type as Wire>::SIZE)*)),*
            ];

            pub fn opcode(&self) -> u8 {
                match self {
                    $(Self::$variant { .. } => $opcode),*
                }
            }

            pub(crate) fn encode(&self) -> io::Result<Vec<u8>> {
                let mut out = Vec::with_capacity(MAX_PACKET_BYTES);
                match self {
                    $(Self::$variant { $($field),* } => {
                        out.push($opcode);
                        $(Wire::put($field, &mut out);)*
                    })*
                }
                // Encoding goes through the same validation as decoding, so a peer never emits a
                // message it would itself reject.
                Self::decode(&out)?;
                Ok(out)
            }

            pub(crate) fn decode(body: &[u8]) -> io::Result<Self> {
                let opcode = *body.first().ok_or_else(|| invalid("empty packet"))?;
                match opcode {
                    $($opcode => {
                        let expected = 1 $(+ <$type as Wire>::SIZE)*;
                        if body.len() != expected {
                            return Err(invalid("invalid message length"));
                        }
                        #[allow(unused_mut)]
                        let mut offset = 1;
                        $(
                            let $field = <$type as Wire>::get(&body[offset..offset + <$type as Wire>::SIZE])?;
                            offset += <$type as Wire>::SIZE;
                        )*
                        debug_assert_eq!(offset, expected);
                        Ok(Self::$variant { $($field),* })
                    })*
                    _ => Err(invalid("unknown opcode")),
                }
            }
        }
    };
}

messages! {
    /// Client → compositor. Surface identifiers are chosen by the client and are connection-local.
    Request {
        1 => Hello { min_version: u16, max_version: u16 },
        2 => CreateToplevel { surface: u32, size: Size },
        3 => CreatePopup { surface: u32, parent: u32, positioner: Positioner },
        4 => CreateDialog { surface: u32, parent: u32, size: Size },
        5 => CreateShellSurface { surface: u32, kind: ShellKind, edge: PanelEdge, output: u32, size: Size, exclusive: u32 },
        6 => CreateLockSurface { surface: u32, output: u32 },
        7 => GrabPopup { surface: u32, serial: u64 },
        8 => Reposition { surface: u32, positioner: Positioner },
        9 => SetInputRegion { surface: u32, region: Rect },
        10 => Submit { surface: u32, sequence: u64, buffer: u64, configure: u64, extent: Size },
        11 => Destroy { surface: u32 },
        12 => BeginMove { surface: u32, serial: u64 },
        13 => SetFullscreen { surface: u32, enabled: bool },
        14 => Goodbye {},
        15 => AttachBuffer { buffer: u64, format: Format, extent: Size, stride: u32, offset: u32 },
        16 => DestroyBuffer { buffer: u64 },
        // Accessibility (ADR 0003): one snapshot per surface, replaced at commit.
        17 => AccessibilityBegin { surface: u32, nodes: u32 },
        18 => AccessibleNode { surface: u32, index: u32, parent: u32, role: AccessibleRole, state: AccessibleState, rect: Rect },
        19 => AccessibleText { surface: u32, index: u32, field: AccessibleField, text: TextChunk },
        20 => AccessibilityCommit { surface: u32 },
        // Assistive technology only (the `accessibility` grant): a node of another client.
        21 => AccessibilityAction { client: u32, surface: u32, index: u32, action: AccessibleAction },
        // What changed in the next Submit's buffer since the surface's previous one, in buffer
        // pixels (ADR 0005); none damages the whole surface.
        22 => Damage { surface: u32, rect: Rect },
        // The clipboard (ADR 0007). An offer of content the client holds, begun with a user
        // input's serial, its media types, then committed as the clipboard while the client has
        // keyboard focus; or the owner empties it.
        23 => ClipboardOffer { offer: u32, serial: u64 },
        24 => ClipboardType { offer: u32, mime: TextChunk },
        25 => ClipboardCommit { offer: u32 },
        26 => ClipboardClear { serial: u64 },
        // A paste: one type of the clipboard's content, written by its owner into the pipe this
        // request carries, while the client has keyboard focus, with a user input's serial.
        27 => ClipboardReceive { selection: u32, mime: TextChunk, serial: u64 },
        // Text input (ADR 0008). A focused field describes itself, double-buffered: its purpose
        // and hints, the text around the caret (chunks, appended, at most 4,000 bytes), the
        // selection within it in bytes, the caret's rectangle; TextInputCommit applies them, its
        // serial counting the client's commits upwards.
        28 => TextInputEnable { surface: u32 },
        29 => TextInputDisable { surface: u32 },
        30 => TextInputPurpose { surface: u32, purpose: TextPurpose, hints: TextHints },
        31 => TextInputSurrounding { surface: u32, text: TextChunk },
        32 => TextInputSelection { surface: u32, anchor: u32, cursor: u32 },
        33 => TextInputCaret { surface: u32, rect: Rect },
        34 => TextInputCommit { surface: u32, serial: u32 },
        // The input method (a client granted `input_method`, ADR 0008): its answer for each key
        // it was given (`used`: it took the key; otherwise the key goes on to the application),
        // and a change for the field, in parts applied at InputMethodCommit, whose serial is the
        // field's commit (InputMethodDone) it was made for.
        35 => InputMethodKeyHandled { serial: u64, used: bool },
        36 => InputMethodDelete { before: u32, after: u32 },
        37 => InputMethodCommitText { text: TextChunk },
        38 => InputMethodPreeditText { text: TextChunk },
        39 => InputMethodPreeditCursor { begin: u32, end: u32 },
        40 => InputMethodPreeditSegment { begin: u32, end: u32, style: PreeditStyle },
        41 => InputMethodCommit { serial: u32 },
        // The input method's candidates: one surface of this size the compositor places next to
        // the caret of the field it serves, shown only while it serves one. Its configure carries
        // the size alone: the input method never learns where the caret is.
        42 => CreateInputCandidates { surface: u32, size: Size },
        // A window's title and application id (ADR 0009), each in chunks of up to 32 bytes:
        // `index` 0 begins it, `last` applies it. The application id is the application's own
        // claim until launch records give it; the shell shows it, nothing is authorized by it.
        43 => SetTitle { surface: u32, index: u8, last: bool, text: PreeditText },
        44 => SetAppId { surface: u32, index: u8, last: bool, text: PreeditText },
        // The session's shell (the `shell` grant): acting on windows by the numbers the
        // compositor gave them, and on workspaces (numbered from 0).
        45 => FocusWindow { window: u32 },
        46 => CloseWindow { window: u32 },
        47 => MinimizeWindow { window: u32 },
        48 => MoveWindowToWorkspace { window: u32, workspace: u32 },
        49 => ActivateWorkspace { workspace: u32 },
        50 => AddWorkspace {},
        51 => RemoveWorkspace { workspace: u32 },
        // The pointer's shape while it is over the surface (the arrow until one is asked for).
        52 => SetPointerShape { surface: u32, shape: PointerShape },
    }
}

messages! {
    /// Compositor → client. Popup configures carry a position relative to the parent; clients
    /// never learn global coordinates or other clients' geometry.
    Event {
        1 => Welcome { version: u16 },
        2 => Configure { surface: u32, serial: u64, rect: Rect, scale_120: u32, fullscreen: bool },
        3 => PopupDone { surface: u32, reason: DismissReason },
        4 => Pointer { surface: u32, serial: u64, kind: PointerKind, x: i32, y: i32, button: u32 },
        5 => Key { surface: u32, serial: u64, key: u32, pressed: bool },
        6 => Focus { surface: u32, focused: bool },
        7 => Touch { surface: u32, serial: u64, kind: TouchKind, id: u32, x: i32, y: i32 },
        8 => Feedback { surface: u32, sequence: u64, outcome: Outcome, timestamp_ns: u64, clock: Clock },
        9 => Released { surface: u32, buffer: u64 },
        10 => ProtocolError { code: ErrorCode },
        11 => Scroll { surface: u32, serial: u64, dx: i32, dy: i32 },
        12 => Text { surface: u32, serial: u64, text: TextChunk },
        13 => Preedit { surface: u32, serial: u64, text: PreeditText, cursor: u8 },
        // To the application: an assistive technology's action on one of its nodes.
        // `serial` is an activation, like a key press: a popup opened by it may grab with it.
        14 => AccessibilityAction { surface: u32, serial: u64, index: u32, action: AccessibleAction },
        // To an assistive technology only: committed snapshots, global rectangles.
        15 => AccessibilityTree { client: u32, surface: u32, parent: u32, nodes: u32 },
        16 => AccessibleNode { client: u32, surface: u32, index: u32, parent: u32, role: AccessibleRole, state: AccessibleState, rect: Rect },
        17 => AccessibleText { client: u32, surface: u32, index: u32, field: AccessibleField, text: TextChunk },
        18 => AccessibilityCommitted { client: u32, surface: u32 },
        19 => AccessibilityRemoved { client: u32, surface: u32 },
        // To the client with keyboard focus: what the clipboard holds now (`selection` names it,
        // not its owner), its types one by one, or that it is empty.
        20 => ClipboardTypes { selection: u32, count: u8 },
        21 => ClipboardType { selection: u32, mime: TextChunk },
        22 => ClipboardEmpty {},
        // To the clipboard's owner: write that type of the offer into the pipe this event
        // carries, then close it.
        23 => ClipboardSend { offer: u32, mime: TextChunk },
        // To a focused field with text input enabled (ADR 0008), one change applied whole at
        // TextInputDone, in this order: delete bytes around the selection, insert the committed
        // text (chunks) at it, then replace the composition with the new one (chunks; none
        // ends it), its cursor range and its styled segments, in bytes. Done's `serial` is the
        // input's, as a key press's; `commit` the field's commit the change was made for.
        24 => TextInputDelete { surface: u32, before: u32, after: u32 },
        25 => TextInputCommitText { surface: u32, text: TextChunk },
        26 => TextInputPreeditText { surface: u32, text: TextChunk },
        27 => TextInputPreeditCursor { surface: u32, begin: u32, end: u32 },
        28 => TextInputPreeditSegment { surface: u32, begin: u32, end: u32, style: PreeditStyle },
        29 => TextInputDone { surface: u32, serial: u64, commit: u32 },
        // To the input method: the focused field it may serve, never its surface or client.
        // Activate begins one; each state follows as its purpose, surrounding text (chunks) and
        // selection, closed by InputMethodDone with the field's commit serial; Deactivate ends
        // it. While one is active it receives the keys first (`text` what the layout types, empty
        // if nothing) and answers each with InputMethodKeyHandled.
        30 => InputMethodActivate {},
        31 => InputMethodDeactivate {},
        32 => InputMethodPurpose { purpose: TextPurpose, hints: TextHints },
        33 => InputMethodSurrounding { text: TextChunk },
        34 => InputMethodSelection { anchor: u32, cursor: u32 },
        35 => InputMethodDone { serial: u32 },
        36 => InputMethodKey { serial: u64, key: u32, pressed: bool, text: PreeditText },
        // To the session's shell (ADR 0009): each window, by a number the compositor gives it
        // (never a surface id), as it appears and changes, its title and application id in chunks
        // as SetTitle's, its closing; and the workspaces and the active one.
        37 => WindowState { window: u32, workspace: u32, minimized: bool, focused: bool },
        38 => WindowTitle { window: u32, index: u8, last: bool, text: PreeditText },
        39 => WindowAppId { window: u32, index: u8, last: bool, text: PreeditText },
        40 => WindowClosed { window: u32 },
        41 => Workspaces { count: u32, active: u32 },
        // To an application: the shell asks it to close the window; it may refuse.
        42 => CloseRequested { surface: u32 },
        // A touchpad pinch, to the surface under the pointer when it began, for as long as it
        // lasts: at (x, y), the pointer (surface-local, as a Pointer event's), which a pinch does
        // not move; `scale_65536` the fingers' spread over their spread when it began (65536 = as
        // they started); `rotation_256` how far they turned since, in 1/256 degrees, clockwise.
        43 => Pinch { surface: u32, serial: u64, phase: GesturePhase, x: i32, y: i32, scale_65536: u32, rotation_256: i32 },
    }
}

impl Event {
    /// Whether this event carries exactly one file descriptor (a pipe to write a clipboard's
    /// content into); every other event carries none.
    pub fn carries_fd(&self) -> bool {
        matches!(self, Event::ClipboardSend { .. })
    }
}

impl Request {
    /// Whether this request must carry exactly one file descriptor; every other request must
    /// carry none.
    pub fn carries_fd(&self) -> bool {
        matches!(
            self,
            Request::AttachBuffer { .. } | Request::ClipboardReceive { .. }
        )
    }

    /// The validated buffer layout of an AttachBuffer.
    pub fn buffer_layout(&self) -> Option<crate::BufferLayout> {
        match *self {
            Request::AttachBuffer {
                format,
                extent,
                stride,
                offset,
                ..
            } => Some(crate::BufferLayout {
                format,
                extent,
                stride,
                offset,
            }),
            _ => None,
        }
    }
}

const _: () = {
    // Fails to compile if a message outgrows the bounded packet.
    let mut index = 0;
    while index < Request::SCHEMA.len() {
        assert!(Request::SCHEMA[index].2 <= MAX_PACKET_BYTES);
        index += 1;
    }
    let mut index = 0;
    while index < Event::SCHEMA.len() {
        assert!(Event::SCHEMA[index].2 <= MAX_PACKET_BYTES);
        index += 1;
    }
};
