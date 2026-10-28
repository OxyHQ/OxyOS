//! Two first-party test clients, each its own process with its own buffers. They are not Bloom:
//! they draw flat rectangles to exercise the display protocol and compositor policy.

use oxy_client::Client;
use oxy_protocol::{Adjust, Anchor, Event, Gravity, PointerKind, Positioner, Rect, Request, Size};
use std::io;
use std::path::Path;

pub use oxy_compositor::DESKTOP;
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

/// Paints every pixel of a frame and presents it.
fn present(
    client: &mut Client,
    surface: u32,
    configure: u64,
    extent: Size,
    paint: impl Fn(u32, u32) -> u32,
) -> io::Result<()> {
    let mut pixels = Vec::with_capacity(extent.width as usize * extent.height as usize * 4);
    for y in 0..extent.height {
        for x in 0..extent.width {
            // XRGB8888 is little-endian B, G, R, X.
            pixels.extend(paint(x, y).to_le_bytes());
        }
    }
    client.present(surface, configure, extent, &pixels)?;
    Ok(())
}

fn physical(rect: Rect) -> Size {
    Size::new(rect.width, rect.height)
}

/// A window the other client's menu will cover.
pub fn other(path: &Path) -> io::Result<()> {
    let mut client = Client::connect(path)?;
    client.send(&Request::CreateToplevel {
        surface: 1,
        size: OTHER_SIZE,
    })?;
    while let Some(event) = client.next_event()? {
        if let Event::Configure {
            surface: 1,
            serial,
            rect,
            ..
        } = event
        {
            present(&mut client, 1, serial, physical(rect), |_, _| OTHER_WINDOW)?;
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
    client.send(&Request::CreateToplevel {
        surface: 1,
        size: APP_SIZE,
    })?;
    let mut menu: Option<(u64, Size)> = None;
    let mut highlight = None;
    while let Some(event) = client.next_event()? {
        match event {
            Event::Configure {
                surface: 1,
                serial,
                rect,
                ..
            } => present(&mut client, 1, serial, physical(rect), paint_window)?,
            Event::Configure {
                surface: 2,
                serial,
                rect,
                ..
            } => {
                menu = Some((serial, physical(rect)));
                present(
                    &mut client,
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
                client.send(&create)?;
                client.send(&Request::GrabPopup { surface: 2, serial })?;
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
                    present(&mut client, 2, serial, size, paint_menu(size, highlight))?;
                }
            }
            Event::PopupDone { surface: 2, .. } => {
                client.destroy_surface(2)?;
                menu = None;
                highlight = None;
            }
            _ => {}
        }
    }
    Ok(())
}
