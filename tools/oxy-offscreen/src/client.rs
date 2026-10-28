//! Two first-party test clients, each its own process with its own buffers. They are not Bloom:
//! they draw flat rectangles to exercise the display protocol and compositor policy.

use oxy_ipc::{create_shared_memory, write_at, Connection};
use oxy_protocol::{
    Adjust, Anchor, BufferLayout, ClientConnection, Event, Format, Gravity, PointerKind,
    Positioner, Rect, Request, Size, MIN_VERSION, VERSION,
};
use rustix::fd::{AsFd, OwnedFd};
use std::collections::HashMap;
use std::io;
use std::path::Path;

pub const DESKTOP: u32 = 0x10_14_18;
pub const OTHER_WINDOW: u32 = 0x3a_4a_6b;
pub const APP_WINDOW: u32 = 0xe8_e8_ee;
pub const APP_TITLE: u32 = 0x30_34_3c;
pub const MENU: u32 = 0xff_ff_ff;
pub const MENU_HIGHLIGHT: u32 = 0xdb_e6_ff;
pub const MENU_BORDER: u32 = 0x9a_a0_aa;
pub const APP_SIZE: Size = Size::new(480, 300);
pub const OTHER_SIZE: Size = Size::new(1000, 640);
pub const MENU_SIZE: Size = Size::new(640, 460);
pub const MENU_ROW: u32 = 46;

struct Buffer {
    id: u64,
    fd: OwnedFd,
    extent: Size,
    busy: bool,
}

/// A client's shared-memory buffers, reused once the compositor releases them.
struct Pool {
    next: u64,
    buffers: HashMap<u32, Vec<Buffer>>,
}

impl Pool {
    fn draw(
        &mut self,
        connection: &Connection,
        surface: u32,
        extent: Size,
        paint: impl Fn(u32, u32) -> u32,
    ) -> io::Result<u64> {
        let buffers = self.buffers.entry(surface).or_default();
        let index = match buffers.iter().position(|b| !b.busy && b.extent == extent) {
            Some(index) => index,
            None => {
                self.next += 1;
                let layout = BufferLayout {
                    format: Format::Xrgb8888,
                    extent,
                    stride: extent.width * 4,
                    offset: 0,
                };
                let bytes = layout.required_bytes().expect("valid layout");
                let fd = create_shared_memory("oxy-test-client", bytes)?;
                connection.send_request(
                    &Request::AttachBuffer {
                        buffer: self.next,
                        format: layout.format,
                        extent,
                        stride: layout.stride,
                        offset: 0,
                    },
                    Some(fd.as_fd()),
                )?;
                buffers.push(Buffer {
                    id: self.next,
                    fd,
                    extent,
                    busy: false,
                });
                buffers.len() - 1
            }
        };
        let buffer = &mut buffers[index];
        let mut pixels = Vec::with_capacity(extent.width as usize * extent.height as usize * 4);
        for y in 0..extent.height {
            for x in 0..extent.width {
                // XRGB8888 is little-endian B, G, R, X.
                pixels.extend(paint(x, y).to_le_bytes());
            }
        }
        write_at(buffer.fd.as_fd(), 0, &pixels)?;
        buffer.busy = true;
        Ok(buffer.id)
    }

    fn release(&mut self, id: u64) {
        for buffers in self.buffers.values_mut() {
            if let Some(buffer) = buffers.iter_mut().find(|b| b.id == id) {
                buffer.busy = false;
            }
        }
    }
}

struct Client {
    connection: Connection,
    pool: Pool,
    sequence: u64,
}

impl Client {
    fn connect(path: &Path) -> io::Result<Self> {
        let connection = Connection::connect(path)?;
        let (mut state, hello) = ClientConnection::new(MIN_VERSION, VERSION);
        connection.send_request(&hello, None)?;
        state
            .receive(&connection.recv_event()?)
            .map_err(|code| io::Error::other(format!("handshake: {code:?}")))?;
        Ok(Self {
            connection,
            pool: Pool {
                next: 0,
                buffers: HashMap::new(),
            },
            sequence: 0,
        })
    }

    fn present(
        &mut self,
        surface: u32,
        configure: u64,
        extent: Size,
        paint: impl Fn(u32, u32) -> u32,
    ) -> io::Result<()> {
        let buffer = self.pool.draw(&self.connection, surface, extent, paint)?;
        self.sequence += 1;
        self.connection.send_request(
            &Request::Submit {
                surface,
                sequence: self.sequence,
                buffer,
                configure,
                extent,
            },
            None,
        )
    }

    /// Next event; `None` once the compositor closes the connection.
    fn next(&mut self) -> io::Result<Option<Event>> {
        match self.connection.recv_event() {
            Ok(Event::Released { surface, buffer }) => {
                self.pool.release(buffer);
                Ok(Some(Event::Released { surface, buffer }))
            }
            Ok(Event::ProtocolError { code }) => Err(io::Error::other(format!("{code:?}"))),
            Ok(event) => Ok(Some(event)),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn physical(rect: Rect) -> Size {
    Size::new(rect.width, rect.height)
}

/// A window the other client's menu will cover.
pub fn other(path: &Path) -> io::Result<()> {
    let mut client = Client::connect(path)?;
    client.connection.send_request(
        &Request::CreateToplevel {
            surface: 1,
            size: OTHER_SIZE,
        },
        None,
    )?;
    while let Some(event) = client.next()? {
        if let Event::Configure {
            surface: 1,
            serial,
            rect,
            ..
        } = event
        {
            client.present(1, serial, physical(rect), |_, _| OTHER_WINDOW)?;
        }
    }
    Ok(())
}

fn paint_window(x: u32, y: u32) -> u32 {
    let _ = x;
    if y < 32 {
        APP_TITLE
    } else {
        APP_WINDOW
    }
}

fn paint_menu(size: Size, highlight: Option<u32>) -> impl Fn(u32, u32) -> u32 {
    move |x, y| {
        if x == 0 || y == 0 || x + 1 == size.width || y + 1 == size.height {
            MENU_BORDER
        } else if highlight == Some(y / MENU_ROW) {
            MENU_HIGHLIGHT
        } else if y % MENU_ROW == 0 {
            MENU_BORDER
        } else {
            MENU
        }
    }
}

/// A window that opens a context menu, centered on the press, larger than the window.
pub fn app(path: &Path) -> io::Result<()> {
    let mut client = Client::connect(path)?;
    client.connection.send_request(
        &Request::CreateToplevel {
            surface: 1,
            size: APP_SIZE,
        },
        None,
    )?;
    let mut menu: Option<(u64, Size)> = None;
    let mut highlight = None;
    while let Some(event) = client.next()? {
        match event {
            Event::Configure {
                surface: 1,
                serial,
                rect,
                ..
            } => client.present(1, serial, physical(rect), paint_window)?,
            Event::Configure {
                surface: 2,
                serial,
                rect,
                ..
            } => {
                menu = Some((serial, physical(rect)));
                client.present(
                    2,
                    serial,
                    physical(rect),
                    paint_menu(physical(rect), highlight),
                )?;
            }
            Event::Pointer {
                surface: 1,
                serial,
                kind: PointerKind::Press,
                x,
                y,
                ..
            } if menu.is_none() => {
                let positioner = Positioner {
                    anchor_rect: Rect::new(x / 256, y / 256, 1, 1),
                    size: MENU_SIZE,
                    anchor: Anchor::Center,
                    gravity: Gravity::Center,
                    adjust: Adjust::SLIDE_X
                        .union(Adjust::SLIDE_Y)
                        .union(Adjust::FLIP_X)
                        .union(Adjust::FLIP_Y),
                    offset_x: 0,
                    offset_y: 0,
                };
                let create = Request::CreatePopup {
                    surface: 2,
                    parent: 1,
                    positioner,
                };
                client.connection.send_request(&create, None)?;
                client
                    .connection
                    .send_request(&Request::GrabPopup { surface: 2, serial }, None)?;
                menu = Some((0, MENU_SIZE));
            }
            Event::Pointer {
                surface: 2,
                kind: PointerKind::Press,
                y,
                ..
            } => {
                if let Some((serial, size)) = menu {
                    highlight = Some((y / 256) as u32 / MENU_ROW);
                    client.present(2, serial, size, paint_menu(size, highlight))?;
                }
            }
            Event::PopupDone { surface: 2, .. } => {
                client
                    .connection
                    .send_request(&Request::Destroy { surface: 2 }, None)?;
                menu = None;
                highlight = None;
            }
            _ => {}
        }
    }
    Ok(())
}
