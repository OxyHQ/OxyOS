//! Client side of the OxyOS display protocol: the handshake, surface requests, shared-memory
//! buffers reused once the compositor releases them, and event delivery for a client's own loop.
//!
//! Every client process uses this crate, whatever draws its pixels: the offscreen test clients
//! and, through `oxyos-client-ffi`, the native Bloom host.

use oxyos_ipc::{create_shared_memory, write_at, Connection};
use oxyos_protocol::{
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
    /// The sequence of the frame last written into it; 0 for none.
    written: u64,
}

/// Frames' damage kept per client, enough for buffers reused a few frames later.
const HISTORY: usize = 16;

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
    /// The latest frames: surface, sequence, and what changed in them (`None`: everything).
    history: std::collections::VecDeque<(u32, u64, Option<Vec<Rect>>)>,
    /// The descriptor the event last returned carried (a ClipboardSend's pipe).
    descriptor: Option<OwnedFd>,
    /// What this client offered to the clipboard, by offer: each type's content. Only the
    /// latest offers are kept; the clipboard holds at most one of them.
    offers: std::collections::VecDeque<(u32, Offer)>,
    next_offer: u32,
}

/// What one offer to the clipboard holds: each media type with its content.
type Offer = Vec<(TextChunk, std::sync::Arc<[u8]>)>;

/// Offers kept for the clipboard: the latest few, of which it holds one.
const OFFERS_KEPT: usize = 4;

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
            .receive(&connection.recv_event()?.0)
            .map_err(|code| io::Error::other(format!("handshake: {code:?}")))?;
        Ok(Self {
            connection,
            buffers: Vec::new(),
            next_buffer: 0,
            sequence: 0,
            history: std::collections::VecDeque::new(),
            descriptor: None,
            offers: std::collections::VecDeque::new(),
            next_offer: 0,
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
        self.present_damaged(surface, configure, extent, format, pixels, None)
    }

    /// [`Client::present_format`] for a frame of which only `damage` changed since the
    /// surface's previous frame (in buffer pixels; `None` or none at all: everything). The
    /// compositor hears the damage (docs/adr/0005-damage.mdx). A reused buffer holds a frame
    /// from some presents ago, so only what changed since that frame is written into it: the
    /// damage of every frame it missed, or everything when that is not known.
    pub fn present_damaged(
        &mut self,
        surface: u32,
        configure: u64,
        extent: Size,
        format: Format,
        pixels: &[u8],
        damage: Option<&[Rect]>,
    ) -> io::Result<u64> {
        let buffer = Rect::new(0, 0, extent.width, extent.height);
        let damage: Option<Vec<Rect>> = damage
            .map(|rects| {
                rects
                    .iter()
                    .filter_map(|rect| clipped(rect, &buffer))
                    .collect()
            })
            .filter(|rects: &Vec<Rect>| !rects.is_empty());
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
                let fd = create_shared_memory("oxyos-client", bytes)?;
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
                    written: 0,
                });
                self.buffers.len() - 1
            }
        };
        // What this buffer lacks: the frames presented since it was written, and this one.
        let written = self.buffers[index].written;
        let missed: Option<Vec<Rect>> = if written == 0 {
            None
        } else {
            let since: Vec<&(u32, u64, Option<Vec<Rect>>)> = self
                .history
                .iter()
                .filter(|(s, sequence, _)| *s == surface && *sequence > written)
                .collect();
            // The history must reach back to the frame after the one written.
            let complete = self
                .history
                .iter()
                .any(|(s, sequence, _)| *s == surface && *sequence <= written);
            complete
                .then(|| {
                    since
                        .iter()
                        .try_fold(damage.clone()?, |mut all, (_, _, frame)| {
                            all.extend(frame.as_ref()?);
                            Some(all)
                        })
                })
                .flatten()
        };
        let buffer = &mut self.buffers[index];
        match missed {
            Some(rects) => {
                let stride = u64::from(extent.width) * 4;
                for rect in rects {
                    for y in rect.y as u32..rect.y as u32 + rect.height {
                        let start = u64::from(y) * stride + u64::from(rect.x as u32) * 4;
                        let end = start + u64::from(rect.width) * 4;
                        write_at(
                            buffer.fd.as_fd(),
                            start,
                            &pixels[start as usize..end as usize],
                        )?;
                    }
                }
            }
            None => write_at(buffer.fd.as_fd(), 0, pixels)?,
        }
        buffer.busy = true;
        let id = buffer.id;
        self.sequence += 1;
        buffer.written = self.sequence;
        for rect in damage.iter().flatten() {
            self.send(&Request::Damage {
                surface,
                rect: *rect,
            })?;
        }
        self.history.push_back((surface, self.sequence, damage));
        if self.history.len() > HISTORY {
            self.history.pop_front();
        }
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
            Ok((event, fd)) => {
                self.descriptor = fd;
                self.observe(event).map(Some)
            }
            Err(error) if closed(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Returns a queued event without blocking.
    pub fn poll(&mut self) -> io::Result<Poll> {
        match self.connection.try_recv_event() {
            Ok(Some((event, fd))) => {
                self.descriptor = fd;
                self.observe(event).map(Poll::Event)
            }
            Ok(None) => Ok(Poll::Empty),
            Err(error) if closed(&error) => Ok(Poll::Closed),
            Err(error) => Err(error),
        }
    }

    /// Copies: offers `contents` (each media type with its bytes, the most faithful first) as
    /// the clipboard, with the serial of the user input that asked for it (ADR 0007). The
    /// client keeps the content and answers each paste of it itself, writing it into the
    /// paste's pipe from a thread of its own; the compositor refuses the offer if the serial is
    /// not a recent input of this client or the client lost keyboard focus.
    pub fn copy(&mut self, serial: u64, contents: Vec<(TextChunk, Vec<u8>)>) -> io::Result<()> {
        self.next_offer = self.next_offer.wrapping_add(1);
        let offer = self.next_offer;
        self.send(&Request::ClipboardOffer { offer, serial })?;
        for (mime, _) in &contents {
            self.send(&Request::ClipboardType { offer, mime: *mime })?;
        }
        self.send(&Request::ClipboardCommit { offer })?;
        let contents = contents
            .into_iter()
            .map(|(mime, bytes)| (mime, std::sync::Arc::from(bytes)))
            .collect();
        self.offers.push_back((offer, contents));
        if self.offers.len() > OFFERS_KEPT {
            self.offers.pop_front();
        }
        Ok(())
    }

    /// Empties the clipboard, if this client's offer is what it holds (a copied password
    /// cleared after a while, say).
    pub fn clear_clipboard(&mut self, serial: u64) -> io::Result<()> {
        self.send(&Request::ClipboardClear { serial })
    }

    /// Pastes `mime` from the clipboard content the compositor named `selection`, with the
    /// serial of the user input that asked for it (ADR 0007). Returns the read end of the pipe
    /// its owner writes the content into: it reaches its end when the owner is done, or at once
    /// if the paste was refused.
    pub fn receive_clipboard(
        &self,
        selection: u32,
        mime: TextChunk,
        serial: u64,
    ) -> io::Result<OwnedFd> {
        let (read, write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?;
        let request = Request::ClipboardReceive {
            selection,
            mime,
            serial,
        };
        self.connection
            .send_request(&request, Some(write.as_fd()))?;
        Ok(read)
    }

    /// The descriptor the event last returned carried, if any: a ClipboardSend's pipe, to write
    /// the offer's content into and close. It is dropped (closed) with the next event.
    pub fn take_descriptor(&mut self) -> Option<OwnedFd> {
        self.descriptor.take()
    }

    /// Frees released buffers for reuse; a protocol error ends the connection.
    fn observe(&mut self, event: Event) -> io::Result<Event> {
        match event {
            // A paste of what this client copied: written into its pipe from a thread, so a
            // slow reader stalls neither the client nor the compositor.
            Event::ClipboardSend { offer, mime } => {
                let content = self
                    .offers
                    .iter()
                    .find(|(id, _)| *id == offer)
                    .and_then(|(_, contents)| contents.iter().find(|(m, _)| *m == mime))
                    .map(|(_, bytes)| bytes.clone());
                // Only for content this client holds: a program offering by its own requests
                // keeps the pipe (take_descriptor).
                if let Some((bytes, pipe)) =
                    content.and_then(|bytes| Some((bytes, self.descriptor.take()?)))
                {
                    let _ = std::thread::Builder::new()
                        .name("oxyos-clipboard".into())
                        .spawn(move || {
                            use std::io::Write;
                            let _ = std::fs::File::from(pipe).write_all(&bytes);
                        });
                }
                Ok(event)
            }
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

/// The part of `rect` within `bounds`, if any.
fn clipped(rect: &Rect, bounds: &Rect) -> Option<Rect> {
    let left = i64::from(rect.x).max(i64::from(bounds.x));
    let top = i64::from(rect.y).max(i64::from(bounds.y));
    let right = (i64::from(rect.x) + i64::from(rect.width))
        .min(i64::from(bounds.x) + i64::from(bounds.width));
    let bottom = (i64::from(rect.y) + i64::from(rect.height))
        .min(i64::from(bounds.y) + i64::from(bounds.height));
    (right > left && bottom > top).then(|| {
        Rect::new(
            left as i32,
            top as i32,
            (right - left) as u32,
            (bottom - top) as u32,
        )
    })
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
