//! C ABI over `oxy-client` for native runtimes written in C or C++ (the React Native host that
//! runs Bloom), and over `oxy-image` for their images ([`images`]). The protocol, transport,
//! buffer handling and decoding stay in the safe Rust crates; this layer only converts between C
//! values and theirs. `include/oxy_client.h` declares it.
//!
//! The workspace forbids `unsafe`; this crate denies it and allows it only on the items below,
//! each with the caller contract it relies on.

use oxy_client::{Accessible, Client, Poll};
use oxy_protocol::{
    AccessibleRole, AccessibleState, Adjust, Anchor, Event, Format, Gravity, Positioner, Rect,
    Request, Size, TEXT_CHUNK_BYTES,
};
use rustix::fd::{AsFd, AsRawFd};
use std::cell::RefCell;
use std::ffi::{c_char, c_int, CStr, CString};
use std::path::Path;

pub mod images;
#[cfg(test)]
mod tests;

/// Event kinds; the values are the protocol's event opcodes.
pub const OXY_EVENT_CONFIGURE: u32 = 2;
pub const OXY_EVENT_POPUP_DONE: u32 = 3;
pub const OXY_EVENT_POINTER: u32 = 4;
pub const OXY_EVENT_KEY: u32 = 5;
pub const OXY_EVENT_FOCUS: u32 = 6;
pub const OXY_EVENT_TOUCH: u32 = 7;
pub const OXY_EVENT_FEEDBACK: u32 = 8;
pub const OXY_EVENT_RELEASED: u32 = 9;
pub const OXY_EVENT_SCROLL: u32 = 11;
pub const OXY_EVENT_TEXT: u32 = 12;
pub const OXY_EVENT_PREEDIT: u32 = 13;
pub const OXY_EVENT_ACCESSIBILITY_ACTION: u32 = 14;

/// Pixel formats (DRM fourcc codes), as the protocol's `Format`.
pub const OXY_FORMAT_XRGB8888: u32 = 0x3432_5258;
pub const OXY_FORMAT_ARGB8888: u32 = 0x3432_5241;

/// Results of `oxy_client_poll`.
pub const OXY_POLL_EVENT: c_int = 1;
pub const OXY_POLL_EMPTY: c_int = 0;
pub const OXY_POLL_CLOSED: c_int = -1;
pub const OXY_ERROR: c_int = -2;

/// One compositor event. Which fields are meaningful depends on `kind`; see `oxy_client.h`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OxyEvent {
    pub kind: u32,
    pub surface: u32,
    pub serial: u64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_120: u32,
    pub detail: u32,
    pub code: u32,
    pub flag: u8,
    /// A text event's UTF-8, NUL-terminated.
    pub text: [u8; TEXT_CHUNK_BYTES + 1],
}

impl Default for OxyEvent {
    fn default() -> Self {
        Self {
            kind: 0,
            surface: 0,
            serial: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            scale_120: 0,
            detail: 0,
            code: 0,
            flag: 0,
            text: [0; TEXT_CHUNK_BYTES + 1],
        }
    }
}

/// A popup's placement, as the protocol's `Positioner`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyPositioner {
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub anchor_width: u32,
    pub anchor_height: u32,
    pub width: u32,
    pub height: u32,
    pub anchor: u8,
    pub gravity: u8,
    pub adjust: u8,
    pub offset_x: i32,
    pub offset_y: i32,
}

/// One node of a surface's accessibility snapshot, as `oxy_client_publish_accessibility` takes it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyAccessibleNode {
    pub parent: u32,
    pub role: u8,
    pub state: u16,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub name: *const c_char,
    pub value: *const c_char,
}

/// An opaque connection, owned by the caller between `oxy_client_connect` and
/// `oxy_client_close`.
pub struct OxyClient {
    client: Client,
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn fail(message: impl ToString) -> c_int {
    let message = message.to_string().replace('\0', " ");
    LAST_ERROR.with(|last| *last.borrow_mut() = CString::new(message).unwrap_or_default());
    OXY_ERROR
}

fn status(result: std::io::Result<()>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

/// Converts a compositor event; `None` for events a client never sees (the handshake).
pub fn convert(event: &Event) -> Option<OxyEvent> {
    let mut out = OxyEvent::default();
    match *event {
        Event::Configure {
            surface,
            serial,
            rect,
            scale_120,
            fullscreen,
        } => {
            out = OxyEvent {
                kind: OXY_EVENT_CONFIGURE,
                surface,
                serial,
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                scale_120,
                flag: fullscreen.into(),
                ..out
            }
        }
        Event::PopupDone { surface, reason } => {
            out = OxyEvent {
                kind: OXY_EVENT_POPUP_DONE,
                surface,
                detail: reason as u32,
                ..out
            }
        }
        Event::Pointer {
            surface,
            serial,
            kind,
            x,
            y,
            button,
        } => {
            out = OxyEvent {
                kind: OXY_EVENT_POINTER,
                surface,
                serial,
                x,
                y,
                detail: kind as u32,
                code: button,
                ..out
            }
        }
        Event::Key {
            surface,
            serial,
            key,
            pressed,
        } => {
            out = OxyEvent {
                kind: OXY_EVENT_KEY,
                surface,
                serial,
                code: key,
                flag: pressed.into(),
                ..out
            }
        }
        Event::Focus { surface, focused } => {
            out = OxyEvent {
                kind: OXY_EVENT_FOCUS,
                surface,
                flag: focused.into(),
                ..out
            }
        }
        Event::Touch {
            surface,
            serial,
            kind,
            id,
            x,
            y,
        } => {
            out = OxyEvent {
                kind: OXY_EVENT_TOUCH,
                surface,
                serial,
                x,
                y,
                detail: kind as u32,
                code: id,
                ..out
            }
        }
        Event::Feedback {
            surface,
            sequence,
            outcome,
            ..
        } => {
            out = OxyEvent {
                kind: OXY_EVENT_FEEDBACK,
                surface,
                serial: sequence,
                detail: outcome as u32,
                ..out
            }
        }
        Event::Released { surface, buffer } => {
            out = OxyEvent {
                kind: OXY_EVENT_RELEASED,
                surface,
                serial: buffer,
                ..out
            }
        }
        Event::Scroll {
            surface,
            serial,
            dx,
            dy,
        } => {
            out = OxyEvent {
                kind: OXY_EVENT_SCROLL,
                surface,
                serial,
                x: dx,
                y: dy,
                ..out
            }
        }
        Event::Text {
            surface,
            serial,
            text,
        } => {
            let bytes = text.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyEvent {
                kind: OXY_EVENT_TEXT,
                surface,
                serial,
                ..out
            }
        }
        Event::Preedit {
            surface,
            serial,
            text,
            cursor,
        } => {
            let bytes = text.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyEvent {
                kind: OXY_EVENT_PREEDIT,
                surface,
                serial,
                detail: u32::from(cursor),
                ..out
            }
        }
        Event::AccessibilityAction {
            surface,
            serial,
            index,
            action,
        } => {
            out = OxyEvent {
                kind: OXY_EVENT_ACCESSIBILITY_ACTION,
                surface,
                serial,
                detail: action as u32,
                code: index,
                ..out
            }
        }
        // Sent only to assistive technologies, which do not use this ABI.
        Event::AccessibilityTree { .. }
        | Event::AccessibleNode { .. }
        | Event::AccessibleText { .. }
        | Event::AccessibilityCommitted { .. }
        | Event::AccessibilityRemoved { .. } => return None,
        Event::Welcome { .. } | Event::ProtocolError { .. } => return None,
    }
    Some(out)
}

/// Converts a C positioner; `None` if an enumeration value is not the protocol's.
pub fn positioner(p: &OxyPositioner) -> Option<Positioner> {
    Some(Positioner {
        anchor_rect: Rect::new(p.anchor_x, p.anchor_y, p.anchor_width, p.anchor_height),
        size: Size::new(p.width, p.height),
        anchor: Anchor::from_raw(p.anchor)?,
        gravity: Gravity::from_raw(p.gravity)?,
        adjust: Adjust::from_bits(p.adjust)?,
        offset_x: p.offset_x,
        offset_y: p.offset_y,
    })
}

/// The message of this thread's last failed call, valid until its next failure.
// `no_mangle` exports a symbol the linker may collide with; the `oxy_` prefix owns these names.
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxy_last_error() -> *const c_char {
    LAST_ERROR.with(|last| last.borrow().as_ptr())
}

/// Connects to the compositor socket at `path` and completes the handshake. Returns NULL on
/// failure (see `oxy_last_error`).
///
/// # Safety
/// `path` is a NUL-terminated string valid for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxy_client_connect(path: *const c_char) -> *mut OxyClient {
    if path.is_null() {
        fail("null path");
        return std::ptr::null_mut();
    }
    // SAFETY: the caller passes a valid NUL-terminated string (contract above).
    let path = unsafe { CStr::from_ptr(path) };
    let Ok(path) = path.to_str() else {
        fail("socket path is not UTF-8");
        return std::ptr::null_mut();
    };
    match Client::connect(Path::new(path)) {
        Ok(client) => Box::into_raw(Box::new(OxyClient { client })),
        Err(error) => {
            fail(error);
            std::ptr::null_mut()
        }
    }
}

/// Closes the connection and frees the client. NULL is ignored.
///
/// # Safety
/// `client` is NULL or a pointer from `oxy_client_connect` not yet closed; it is invalid after.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxy_client_close(client: *mut OxyClient) {
    if !client.is_null() {
        // SAFETY: the pointer came from Box::into_raw in oxy_client_connect and is closed once.
        drop(unsafe { Box::from_raw(client) });
    }
}

/// Reborrows a caller's client pointer.
///
/// # Safety
/// `client` is a live pointer from `oxy_client_connect`, used by one thread at a time.
#[allow(unsafe_code)]
unsafe fn client<'a>(client: *mut OxyClient) -> Option<&'a mut Client> {
    // SAFETY: per the contract, a non-null pointer is a live, exclusively used OxyClient.
    unsafe { client.as_mut() }.map(|c| &mut c.client)
}

/// The connection's descriptor, readable when an event is queued (for poll/epoll).
///
/// # Safety
/// `client` is a live pointer from `oxy_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxy_client_fd(client: *mut OxyClient) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => client.as_fd().as_raw_fd(),
        None => fail("null client"),
    }
}

/// Sends one request; 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxy_client_connect`.
#[allow(unsafe_code)]
unsafe fn send(client: *mut OxyClient, request: Request) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.send(&request)),
        None => fail("null client"),
    }
}

/// Requests a toplevel window of the given logical size; the compositor answers with a
/// configure. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxy_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxy_client_create_toplevel(
    client: *mut OxyClient,
    surface: u32,
    width: u32,
    height: u32,
) -> c_int {
    let size = Size::new(width, height);
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::CreateToplevel { surface, size }) }
}

/// Requests a popup placed relative to `parent` by `positioner`. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxy_client_connect`; `positioner` points to a valid
/// `OxyPositioner` for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxy_client_create_popup(
    client: *mut OxyClient,
    surface: u32,
    parent: u32,
    positioner: *const OxyPositioner,
) -> c_int {
    // SAFETY: the caller passes a valid positioner or NULL.
    let Some(positioner) = unsafe { positioner.as_ref() }.and_then(self::positioner) else {
        return fail("invalid positioner");
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        send(
            client,
            Request::CreatePopup {
                surface,
                parent,
                positioner,
            },
        )
    }
}

/// Places an existing popup again (its anchor moved or its content resized); the compositor
/// answers with a configure. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxy_client_connect`; `positioner` points to a valid
/// `OxyPositioner` for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxy_client_reposition(
    client: *mut OxyClient,
    surface: u32,
    positioner: *const OxyPositioner,
) -> c_int {
    // SAFETY: the caller passes a valid positioner or NULL.
    let Some(positioner) = unsafe { positioner.as_ref() }.and_then(self::positioner) else {
        return fail("invalid positioner");
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        send(
            client,
            Request::Reposition {
                surface,
                positioner,
            },
        )
    }
}

/// Publishes a surface's accessibility snapshot (ADR 0003). 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxy_client_connect`; `nodes` points to `count` nodes (or is
/// NULL with `count` 0), whose `name` and `value` are NULL or NUL-terminated strings.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxy_client_publish_accessibility(
    client: *mut OxyClient,
    surface: u32,
    nodes: *const OxyAccessibleNode,
    count: usize,
) -> c_int {
    let nodes: &[OxyAccessibleNode] = if count == 0 {
        &[]
    } else if nodes.is_null() {
        return fail("null nodes");
    } else {
        // SAFETY: the caller passes `count` valid nodes.
        unsafe { std::slice::from_raw_parts(nodes, count) }
    };
    let text = |pointer: *const c_char| -> Option<&str> {
        if pointer.is_null() {
            Some("")
        } else {
            // SAFETY: the caller passes NULL or NUL-terminated strings that outlive the call.
            unsafe { CStr::from_ptr(pointer) }.to_str().ok()
        }
    };
    let mut accessible = Vec::with_capacity(nodes.len());
    for node in nodes {
        let (Some(role), Some(state), Some(name), Some(value)) = (
            AccessibleRole::from_raw(node.role),
            AccessibleState::new(node.state),
            text(node.name),
            text(node.value),
        ) else {
            return fail("invalid accessible node");
        };
        accessible.push(Accessible {
            parent: node.parent,
            role,
            state,
            rect: Rect::new(node.x, node.y, node.width, node.height),
            name,
            value,
        });
    }
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.publish_accessibility(surface, &accessible)),
        None => fail("null client"),
    }
}

/// Asks for the popup's input grab, citing the serial of the input event that opened it.
///
/// # Safety
/// `client` is a live pointer from `oxy_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxy_client_grab_popup(
    client: *mut OxyClient,
    surface: u32,
    serial: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::GrabPopup { surface, serial }) }
}

/// Destroys a surface and the buffers it drew with. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxy_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxy_client_destroy_surface(client: *mut OxyClient, surface: u32) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.destroy_surface(surface)),
        None => fail("null client"),
    }
}

/// Presents `pixels` (rows of `width * 4` bytes, `height` rows) on `surface` in answer to the
/// configure with serial `configure`. `format` is a DRM fourcc: `OXY_FORMAT_XRGB8888` (opaque,
/// little-endian B, G, R, X) or `OXY_FORMAT_ARGB8888` (premultiplied B, G, R, A). Returns the
/// frame's sequence number, or -1 on failure.
///
/// # Safety
/// `client` is a live pointer from `oxy_client_connect`; `pixels` points to `length` readable
/// bytes for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxy_client_present(
    client: *mut OxyClient,
    surface: u32,
    configure: u64,
    width: u32,
    height: u32,
    format: u32,
    pixels: *const u8,
    length: usize,
) -> i64 {
    if pixels.is_null() {
        fail("null pixels");
        return -1;
    }
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    // SAFETY: the caller guarantees `length` readable bytes at `pixels`.
    let pixels = unsafe { std::slice::from_raw_parts(pixels, length) };
    let Some(format) = Format::from_raw(format) else {
        fail("unknown pixel format");
        return -1;
    };
    match client.present_format(surface, configure, Size::new(width, height), format, pixels) {
        Ok(sequence) => i64::try_from(sequence).unwrap_or(-1),
        Err(error) => {
            fail(error);
            -1
        }
    }
}

/// A rectangle of a buffer, in its pixels: what changed in a frame (`oxy_client_present_damaged`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// [`oxy_client_present`] for a frame of which only `count` rectangles at `damage` changed
/// since the surface's previous frame (none: everything). The compositor hears the damage, and
/// a reused buffer receives only what changed since it was last written
/// (docs/adr/0005-damage.mdx).
///
/// # Safety
/// As [`oxy_client_present`]; `damage` points to `count` readable `OxyRect`s (any pointer when
/// `count` is 0).
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxy_client_present_damaged(
    client: *mut OxyClient,
    surface: u32,
    configure: u64,
    width: u32,
    height: u32,
    format: u32,
    pixels: *const u8,
    length: usize,
    damage: *const OxyRect,
    count: usize,
) -> i64 {
    if pixels.is_null() || (count > 0 && damage.is_null()) {
        fail("null pixels or damage");
        return -1;
    }
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    // SAFETY: the caller guarantees `length` readable bytes at `pixels`.
    let pixels = unsafe { std::slice::from_raw_parts(pixels, length) };
    let damage: Vec<Rect> = if count == 0 {
        Vec::new()
    } else {
        // SAFETY: the caller guarantees `count` readable rectangles at `damage`.
        unsafe { std::slice::from_raw_parts(damage, count) }
            .iter()
            .map(|rect| Rect::new(rect.x, rect.y, rect.width, rect.height))
            .collect()
    };
    let Some(format) = Format::from_raw(format) else {
        fail("unknown pixel format");
        return -1;
    };
    let damage = (!damage.is_empty()).then_some(damage.as_slice());
    match client.present_damaged(
        surface,
        configure,
        Size::new(width, height),
        format,
        pixels,
        damage,
    ) {
        Ok(sequence) => i64::try_from(sequence).unwrap_or(-1),
        Err(error) => {
            fail(error);
            -1
        }
    }
}

/// Takes one queued event without blocking: `OXY_POLL_EVENT` with `*out` filled,
/// `OXY_POLL_EMPTY`, `OXY_POLL_CLOSED` once the compositor closed the connection, or `OXY_ERROR`
/// (a protocol error ends the connection).
///
/// # Safety
/// `client` is a live pointer from `oxy_client_connect`; `out` points to a writable `OxyEvent`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxy_client_poll(client: *mut OxyClient, out: *mut OxyEvent) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    // SAFETY: the caller passes a writable OxyEvent or NULL.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return fail("null event");
    };
    loop {
        match client.poll() {
            Ok(Poll::Event(event)) => {
                if let Some(event) = convert(&event) {
                    *out = event;
                    return OXY_POLL_EVENT;
                }
            }
            Ok(Poll::Empty) => return OXY_POLL_EMPTY,
            Ok(Poll::Closed) => return OXY_POLL_CLOSED,
            Err(error) => return fail(error),
        }
    }
}
