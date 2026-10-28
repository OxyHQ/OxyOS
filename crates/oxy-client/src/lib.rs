//! Client side of the Oxy display protocol: the handshake, surface requests, shared-memory
//! buffers reused once the compositor releases them, and event delivery for a client's own loop.
//!
//! Every client process uses this crate, whatever draws its pixels: the offscreen test clients
//! and, through `oxy-client-ffi`, the native Bloom host.

use oxy_ipc::{create_shared_memory, write_at, Connection};
use oxy_protocol::{
    AccessibleField, AccessibleRole, AccessibleState, BufferLayout, ClientConnection, Event,
    Format, Rect, Request, Size, TextChunk, MAX_ACCESSIBLE_NODES, MAX_ACCESSIBLE_TEXT_BYTES,
    MIN_VERSION, VERSION,
};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use std::io;
use std::path::Path;

#[cfg(test)]
mod tests;

/// The compositor closed the connection. A socket closed while requests were still unread
/// reports a reset instead of end-of-file; both mean the same to the client.
fn closed(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
    )
}

struct Buffer {
    id: u64,
    surface: u32,
    fd: OwnedFd,
    extent: Size,
    format: Format,
    busy: bool,
}

/// What a non-blocking poll found.
#[derive(Debug, PartialEq, Eq)]
pub enum Poll {
    Event(Event),
    /// No event is queued now.
    Empty,
    /// The compositor closed the connection.
    Closed,
}

pub struct Client {
    connection: Connection,
    buffers: Vec<Buffer>,
    next_buffer: u64,
    sequence: u64,
}

impl Client {
    /// Connects and completes the version handshake.
    pub fn connect(path: &Path) -> io::Result<Self> {
        Self::handshake(Connection::connect(path)?)
    }

    /// Completes the handshake on a connection the client was handed (a launcher's socket pair).
    pub fn handshake(connection: Connection) -> io::Result<Self> {
        let (mut state, hello) = ClientConnection::new(MIN_VERSION, VERSION);
        connection.send_request(&hello, None)?;
        state
            .receive(&connection.recv_event()?)
            .map_err(|code| io::Error::other(format!("handshake: {code:?}")))?;
        Ok(Self {
            connection,
            buffers: Vec::new(),
            next_buffer: 0,
            sequence: 0,
        })
    }

    /// Sends a request that carries no descriptor. Buffers go through [`Client::present`].
    pub fn send(&self, request: &Request) -> io::Result<()> {
        self.connection.send_request(request, None)
    }

    /// Writes `pixels` (XRGB8888: little-endian B, G, R, X rows of `extent.width * 4` bytes) into
    /// a buffer the compositor is not reading and submits it for `surface` in answer to the
    /// configure `configure`. Returns the frame's sequence number.
    pub fn present(
        &mut self,
        surface: u32,
        configure: u64,
        extent: Size,
        pixels: &[u8],
    ) -> io::Result<u64> {
        self.present_format(surface, configure, extent, Format::Xrgb8888, pixels)
    }

    /// [`Client::present`] in a chosen format. ARGB8888 is premultiplied (little-endian B, G, R,
    /// A): what a popup with rounded corners or a shadow needs.
    pub fn present_format(
        &mut self,
        surface: u32,
        configure: u64,
        extent: Size,
        format: Format,
        pixels: &[u8],
    ) -> io::Result<u64> {
        let layout = BufferLayout {
            format,
            extent,
            stride: extent.width * 4,
            offset: 0,
        };
        let bytes = layout
            .required_bytes()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid extent"))?;
        if pixels.len() as u64 != bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "pixels do not match the extent",
            ));
        }
        // Idle buffers of another size or format will not be used again.
        let stale: Vec<u64> = self
            .buffers
            .iter()
            .filter(|b| {
                b.surface == surface && !b.busy && (b.extent != extent || b.format != format)
            })
            .map(|b| b.id)
            .collect();
        for id in stale {
            self.destroy_buffer(id)?;
        }
        let index = match self.buffers.iter().position(|b| {
            b.surface == surface && !b.busy && b.extent == extent && b.format == format
        }) {
            Some(index) => index,
            None => {
                self.next_buffer += 1;
                let fd = create_shared_memory("oxy-client", bytes)?;
                self.connection.send_request(
                    &Request::AttachBuffer {
                        buffer: self.next_buffer,
                        format: layout.format,
                        extent,
                        stride: layout.stride,
                        offset: 0,
                    },
                    Some(fd.as_fd()),
                )?;
                self.buffers.push(Buffer {
                    id: self.next_buffer,
                    surface,
                    fd,
                    extent,
                    format,
                    busy: false,
                });
                self.buffers.len() - 1
            }
        };
        let buffer = &mut self.buffers[index];
        write_at(buffer.fd.as_fd(), 0, pixels)?;
        buffer.busy = true;
        let id = buffer.id;
        self.sequence += 1;
        self.send(&Request::Submit {
            surface,
            sequence: self.sequence,
            buffer: id,
            configure,
            extent,
        })?;
        Ok(self.sequence)
    }

    /// Destroys a surface and the buffers it drew with.
    pub fn destroy_surface(&mut self, surface: u32) -> io::Result<()> {
        self.send(&Request::Destroy { surface })?;
        let owned: Vec<u64> = self
            .buffers
            .iter()
            .filter(|b| b.surface == surface)
            .map(|b| b.id)
            .collect();
        for id in owned {
            self.destroy_buffer(id)?;
        }
        Ok(())
    }

    /// Publishes a surface's accessibility snapshot (ADR 0003), replacing the previous one:
    /// nodes in reading order, each parent before its children. Text longer than the protocol
    /// allows is cut at a character boundary; more nodes than it allows is an error.
    pub fn publish_accessibility(&self, surface: u32, nodes: &[Accessible<'_>]) -> io::Result<()> {
        let count = u32::try_from(nodes.len())
            .ok()
            .filter(|count| *count <= MAX_ACCESSIBLE_NODES)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "too many accessible nodes")
            })?;
        self.send(&Request::AccessibilityBegin {
            surface,
            nodes: count,
        })?;
        for (index, node) in (0..).zip(nodes) {
            self.send(&Request::AccessibleNode {
                surface,
                index,
                parent: node.parent,
                role: node.role,
                state: node.state,
                rect: node.rect,
            })?;
            for (field, text) in [
                (AccessibleField::Name, node.name),
                (AccessibleField::Value, node.value),
            ] {
                for text in TextChunk::split(truncate(text, MAX_ACCESSIBLE_TEXT_BYTES)) {
                    self.send(&Request::AccessibleText {
                        surface,
                        index,
                        field,
                        text,
                    })?;
                }
            }
        }
        self.send(&Request::AccessibilityCommit { surface })
    }

    /// The compositor retires a destroyed buffer once no frame reads it.
    fn destroy_buffer(&mut self, id: u64) -> io::Result<()> {
        self.buffers.retain(|b| b.id != id);
        self.send(&Request::DestroyBuffer { buffer: id })
    }

    /// Blocks for the next event; `None` once the compositor closes the connection.
    pub fn next_event(&mut self) -> io::Result<Option<Event>> {
        match self.connection.recv_event() {
            Ok(event) => self.observe(event).map(Some),
            Err(error) if closed(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Returns a queued event without blocking.
    pub fn poll(&mut self) -> io::Result<Poll> {
        match self.connection.try_recv_event() {
            Ok(Some(event)) => self.observe(event).map(Poll::Event),
            Ok(None) => Ok(Poll::Empty),
            Err(error) if closed(&error) => Ok(Poll::Closed),
            Err(error) => Err(error),
        }
    }

    /// Frees released buffers for reuse; a protocol error ends the connection.
    fn observe(&mut self, event: Event) -> io::Result<Event> {
        match event {
            Event::Released { buffer, .. } => {
                if let Some(entry) = self.buffers.iter_mut().find(|b| b.id == buffer) {
                    entry.busy = false;
                }
                Ok(event)
            }
            Event::ProtocolError { code } => Err(io::Error::other(format!("{code:?}"))),
            event => Ok(event),
        }
    }

    /// Buffers the client holds, busy or idle.
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }
}

impl AsFd for Client {
    /// Readable when an event is queued, for a client that polls several sources.
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.connection.as_fd()
    }
}

/// One node of a surface's accessibility snapshot (`Client::publish_accessibility`).
#[derive(Clone, Copy, Debug)]
pub struct Accessible<'a> {
    /// Index of the parent node, earlier in the list; `NO_ACCESSIBLE_PARENT` for a root.
    pub parent: u32,
    pub role: AccessibleRole,
    pub state: AccessibleState,
    /// Surface-local, logical.
    pub rect: Rect,
    pub name: &'a str,
    pub value: &'a str,
}

/// The longest prefix of `text` within `limit` bytes that ends at a character boundary.
fn truncate(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
