//! Experimental Oxy display protocol.
//!
//! One schema (the `messages!` invocations below) generates the typed messages, their exact wire
//! lengths, the encoder and the decoder. Requests travel client → compositor and events
//! compositor → client; each direction has its own opcode space and decoder, so a peer cannot send
//! a message of the other direction. [`ServerConnection`] and [`ClientConnection`] enforce the
//! connection state machine. Nothing here grants authority: a successful Hello only selects a
//! version, and the compositor authorizes roles from session grants.

use std::io::{self, Read, Write};

mod schema;
mod wire;

pub use schema::{Event, Request};
pub use wire::{
    Adjust, Anchor, BufferLayout, Clock, DismissReason, ErrorCode, Format, Gravity, Outcome,
    PanelEdge, PointerKind, Positioner, Rect, ShellKind, Size, TextChunk, TouchKind,
    MAX_BUFFER_BYTES, TEXT_CHUNK_BYTES,
};

/// Lowest and highest protocol versions this implementation speaks.
pub const MIN_VERSION: u16 = 2;
pub const VERSION: u16 = 2;
/// Upper bound on a packet payload (opcode plus fields). No message reaches it.
pub const MAX_PACKET_BYTES: usize = 64;
/// Largest accepted surface or buffer dimension, in either logical or physical pixels.
pub const MAX_DIMENSION: u32 = 16_384;
/// Largest absolute logical coordinate accepted in geometry.
pub const MAX_COORDINATE: i32 = 1 << 20;

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
