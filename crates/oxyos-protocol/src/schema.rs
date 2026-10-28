//! The authoritative message schema. `docs/display-protocol.mdx` documents the same table; the
//! `schema_table_matches_documentation` test keeps them in step.

use crate::wire::{
    AccessibleAction, AccessibleField, AccessibleRole, AccessibleState, Clock, DismissReason,
    ErrorCode, Format, Outcome, PanelEdge, PointerKind, Positioner, PreeditText, Rect, ShellKind,
    Size, TextChunk, TouchKind, Wire,
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
