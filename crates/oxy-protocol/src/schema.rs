//! The authoritative message schema. `docs/display-protocol.mdx` documents the same table; the
//! `schema_table_matches_documentation` test keeps them in step.

use crate::wire::{
    Clock, DismissReason, ErrorCode, Outcome, PanelEdge, PointerKind, Positioner, Rect, ShellKind,
    Size, TouchKind, Wire,
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
