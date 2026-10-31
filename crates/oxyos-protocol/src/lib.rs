//! Experimental OxyOS display protocol.
//!
//! One schema (the `messages!` invocations below) generates the typed messages, their exact wire
//! lengths, the encoder and the decoder. Requests travel client → compositor and events
//! compositor → client; each direction has its own opcode space and decoder, so a peer cannot send
//! a message of the other direction. [`ServerConnection`] and [`ClientConnection`] enforce the
//! connection state machine. Nothing here grants authority: a successful Hello only selects a
//! version, and the compositor authorizes roles from session grants.

use std::io::{self, Read, Write};

mod schema;
mod versions;
mod wire;

pub use schema::{Event, Request};
pub use wire::{
    AccessibleAction, AccessibleActions, AccessibleField, AccessibleRole, AccessibleState, Adjust,
    Anchor, AuthOutcome, BatteryState, BluetoothState, BufferLayout, Clock, Control, DismissReason,
    ErrorCode, Format, GesturePhase, Gravity, MenuField, MenuFlags, MenuKind, NotificationField,
    NotificationOutcome, Outcome, PanelEdge, PointerKind, PointerShape, Politeness, Positioner,
    PreeditStyle, PreeditText, ReaderCommand, Rect, ScrollSource, SecretChunk, ShellKind, Size,
    SystemShortcut, TextChunk, TextHints, TextPurpose, TouchKind, WifiState, MAX_BUFFER_BYTES,
    TEXT_CHUNK_BYTES,
};

/// Lowest and highest protocol versions this implementation speaks (ADR 0033): it serves every
/// version from the oldest still supported to the current one, encoding each connection's events
/// as its version defines them.
pub const MIN_VERSION: u16 = 2;
pub const VERSION: u16 = 5;

pub use versions::{event_since, read_event_for, request_since, write_event_for};
/// Upper bound on a packet payload (opcode plus fields). No message reaches it.
pub const MAX_PACKET_BYTES: usize = 64;
/// The longest password a lock surface may give (ADR 0010), as Linux-PAM's longest answer.
pub const MAX_SECRET_BYTES: usize = 512;
/// The longest account a login screen's lock surface names (AuthenticateAccount, ADR 0020): a Unix
/// user's name, at most 31 bytes as utmp and systemd take one.
pub const MAX_USER_NAME_BYTES: usize = 31;
/// The text around the caret a field may describe (TextInputSurrounding, ADR 0008), in bytes.
pub const MAX_SURROUNDING_BYTES: usize = 4000;
/// Most nodes in one surface's accessibility snapshot (ADR 0003).
pub const MAX_ACCESSIBLE_NODES: u32 = 4096;
/// Most bytes of one accessible node's name or value.
pub const MAX_ACCESSIBLE_TEXT_BYTES: usize = 1024;
/// `parent` of an accessible node that is a root of its surface's tree.
pub const NO_ACCESSIBLE_PARENT: u32 = u32::MAX;
/// The longest announcement (`AccessibilityAnnounce`, ADR 0003), in bytes.
pub const MAX_ANNOUNCEMENT_BYTES: usize = 1024;
/// The longest `key=value` of a preference changed (`SetPreference`) or told (`Preference`,
/// ADR 0019), in bytes.
pub const MAX_PREFERENCE_BYTES: usize = 128;
/// The most a SystemRequest, SystemText or SheetText may hold, in bytes (ADR 0030).
pub const MAX_SYSTEM_TEXT_BYTES: usize = 256 * 1024;
/// Media types a clipboard offer may name (ADR 0007).
pub const MAX_CLIPBOARD_TYPES: usize = 16;
/// The longest keyboard layout told to clients, `layout\tvariant\toptions` (ADR 0029).
pub const MAX_KEYBOARD_LAYOUT_BYTES: usize = 1024;
/// The largest pointer image a client may set, on either side, in its pixels (ADR 0029): the
/// largest cursor planes take 256.
pub const MAX_POINTER_IMAGE_SIDE: u32 = 256;
/// Most entries (menus, commands, submenus and separators) in one window's menus (ADR 0021).
pub const MAX_MENU_ITEMS: u32 = 256;
/// How deep a window's menus nest: a menu of the bar is at depth 1, its entries at 2; so at most
/// two submenus deep.
pub const MAX_MENU_DEPTH: u32 = 4;
/// Most bytes of a menu entry's label.
pub const MAX_MENU_LABEL_BYTES: usize = 128;
/// Most bytes of a command's shortcut (one chunk: `Mod+Shift+Z` and its like).
pub const MAX_MENU_SHORTCUT_BYTES: usize = 32;
/// `parent` of a menu of the bar.
pub const NO_MENU_PARENT: u32 = u32::MAX;
/// Notifications (ADR 0026): the most actions one offers, and the most bytes of each of its texts.
/// An action is numbered from 1 (0 is the notification itself, pressed).
pub const MAX_NOTIFICATION_ACTIONS: usize = 3;
pub const MAX_NOTIFICATION_IDENTIFIER_BYTES: usize = 64;
pub const MAX_NOTIFICATION_TITLE_BYTES: usize = 256;
pub const MAX_NOTIFICATION_BODY_BYTES: usize = 2048;
pub const MAX_NOTIFICATION_DATA_BYTES: usize = 4096;
pub const MAX_NOTIFICATION_ACTION_BYTES: usize = 64;
pub const MAX_NOTIFICATION_REPLY_BYTES: usize = 1024;
/// Most windows one shell surface shows live (`PlaceWindowView`, ADR 0022).
pub const MAX_WINDOW_VIEWS: usize = 64;
/// A whole level of a control (`ControlLevel`, `SetControlLevel`, ADR 0025): 65536 is all of it.
pub const FULL_LEVEL: u32 = 65_536;
/// Largest accepted surface or buffer dimension, in either logical or physical pixels.
pub const MAX_DIMENSION: u32 = 16_384;
/// Largest absolute logical coordinate accepted in geometry.
pub const MAX_COORDINATE: i32 = 1 << 20;
/// The radius of every window's corners (a toplevel's or a dialog's, not full-screen), in logical
/// pixels: the compositor cuts them from the window's pixels (docs/adr/0016-windows.mdx), and a
/// client keeps what it draws concentric with them.
pub const WINDOW_RADIUS: u32 = 16;

/// Picks the highest version both peers support.
pub fn negotiate(min_version: u16, max_version: u16) -> Result<u16, ErrorCode> {
    if min_version > max_version {
        return Err(ErrorCode::InvalidMessage);
    }
    let version = max_version.min(VERSION);
    if version < min_version || version < MIN_VERSION {
        Err(ErrorCode::UnsupportedVersion)
    } else {
        Ok(version)
    }
}

pub(crate) fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

fn write_packet(writer: &mut impl Write, body: &[u8]) -> io::Result<()> {
    debug_assert!(!body.is_empty() && body.len() <= MAX_PACKET_BYTES);
    let mut packet = [0; MAX_PACKET_BYTES + 4];
    packet[..4].copy_from_slice(&(body.len() as u32).to_le_bytes());
    packet[4..4 + body.len()].copy_from_slice(body);
    writer.write_all(&packet[..4 + body.len()])?;
    writer.flush()
}

fn read_packet<'a>(
    reader: &mut impl Read,
    storage: &'a mut [u8; MAX_PACKET_BYTES],
) -> io::Result<&'a [u8]> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_le_bytes(header) as usize;
    if !(1..=MAX_PACKET_BYTES).contains(&length) {
        return Err(invalid("packet length out of bounds"));
    }
    reader.read_exact(&mut storage[..length])?;
    Ok(&storage[..length])
}

pub fn write_request(writer: &mut impl Write, request: &Request) -> io::Result<()> {
    write_packet(writer, &request.encode()?)
}

pub fn read_request(reader: &mut impl Read) -> io::Result<Request> {
    let mut storage = [0; MAX_PACKET_BYTES];
    Request::decode(read_packet(reader, &mut storage)?)
}

pub fn write_event(writer: &mut impl Write, event: &Event) -> io::Result<()> {
    write_packet(writer, &event.encode()?)
}

pub fn read_event(reader: &mut impl Read) -> io::Result<Event> {
    let mut storage = [0; MAX_PACKET_BYTES];
    Event::decode(read_packet(reader, &mut storage)?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    AwaitHello,
    Ready(u16),
    Closed,
}

/// Compositor side of one connection: Hello first, then requests, then Goodbye.
#[derive(Debug)]
pub struct ServerConnection {
    state: State,
}

impl Default for ServerConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerConnection {
    pub fn new() -> Self {
        Self {
            state: State::AwaitHello,
        }
    }

    pub fn version(&self) -> Option<u16> {
        match self.state {
            State::Ready(version) => Some(version),
            _ => None,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.state == State::Closed
    }

    /// Validates `request` against the connection state. On success returns the Welcome to send
    /// for a Hello, otherwise `None`. Any error closes the connection.
    pub fn receive(&mut self, request: &Request) -> Result<Option<Event>, ErrorCode> {
        let result = match (self.state, request) {
            (
                State::AwaitHello,
                Request::Hello {
                    min_version,
                    max_version,
                },
            ) => negotiate(*min_version, *max_version).map(|version| {
                self.state = State::Ready(version);
                Some(Event::Welcome { version })
            }),
            (State::AwaitHello, _) => Err(ErrorCode::InvalidOrder),
            (State::Ready(_), Request::Hello { .. }) => Err(ErrorCode::InvalidOrder),
            (State::Ready(_), Request::Goodbye {}) => {
                self.state = State::Closed;
                Ok(None)
            }
            (State::Ready(_), _) => Ok(None),
            (State::Closed, _) => Err(ErrorCode::InvalidOrder),
        };
        if result.is_err() {
            self.state = State::Closed;
        }
        result
    }
}

/// Client side of one connection: the first event must be a Welcome for a requested version.
#[derive(Debug)]
pub struct ClientConnection {
    state: State,
    min_version: u16,
    max_version: u16,
}

impl ClientConnection {
    pub fn new(min_version: u16, max_version: u16) -> (Self, Request) {
        (
            Self {
                state: State::AwaitHello,
                min_version,
                max_version,
            },
            Request::Hello {
                min_version,
                max_version,
            },
        )
    }

    pub fn version(&self) -> Option<u16> {
        match self.state {
            State::Ready(version) => Some(version),
            _ => None,
        }
    }

    pub fn receive(&mut self, event: &Event) -> Result<(), ErrorCode> {
        let result = match (self.state, event) {
            (State::AwaitHello, Event::Welcome { version })
                if (self.min_version..=self.max_version).contains(version) =>
            {
                self.state = State::Ready(*version);
                Ok(())
            }
            (State::AwaitHello, Event::ProtocolError { code }) => Err(*code),
            (State::AwaitHello, _) | (State::Ready(_), Event::Welcome { .. }) => {
                Err(ErrorCode::InvalidOrder)
            }
            (State::Ready(_), Event::ProtocolError { code }) => Err(*code),
            (State::Ready(_), _) => Ok(()),
            (State::Closed, _) => Err(ErrorCode::InvalidOrder),
        };
        if result.is_err() {
            self.state = State::Closed;
        }
        result
    }
}

#[cfg(test)]
mod tests;
