//! C ABI over `oxyos-client` for native runtimes written in C or C++ (the React Native host that
//! runs Bloom), and over `oxyos-net` for their HTTP ([`net`]), `oxyos-image` for their images
//! ([`images`]), `oxyos-vector` for their vector graphics ([`vector`]) and `oxyos-gestures` for
//! react-native-gesture-handler's recognizers ([`gestures`]). The protocol,
//! transport, buffer handling, network, decoding and rasterizing stay in the safe Rust crates; this layer only converts between C values and theirs. `include/oxyos_client.h`
//! declares it.
//!
//! The workspace forbids `unsafe`; this crate denies it and allows it only on the items below,
//! each with the caller contract it relies on.

use oxyos_client::{Accessible, Client, Field, Poll};
use oxyos_protocol::{
    AccessibleRole, AccessibleState, Adjust, Anchor, Event, Format, Gravity, PanelEdge, Positioner,
    Rect, Request, ShellKind, Size, TextChunk, TextHints, TextPurpose, TEXT_CHUNK_BYTES,
};
use rustix::fd::{AsFd, AsRawFd};
use std::cell::RefCell;
use std::ffi::{c_char, c_int, CStr, CString};
use std::path::Path;

pub mod gestures;
pub mod images;
pub mod net;
#[cfg(test)]
mod tests;
pub mod vector;

/// The type text is copied and pasted as.
pub const CLIPBOARD_TEXT: &str = "text/plain;charset=utf-8";

/// Event kinds; the values are the protocol's event opcodes.
pub const OXYOS_EVENT_CONFIGURE: u32 = 2;
pub const OXYOS_EVENT_POPUP_DONE: u32 = 3;
pub const OXYOS_EVENT_POINTER: u32 = 4;
pub const OXYOS_EVENT_KEY: u32 = 5;
pub const OXYOS_EVENT_FOCUS: u32 = 6;
pub const OXYOS_EVENT_TOUCH: u32 = 7;
pub const OXYOS_EVENT_FEEDBACK: u32 = 8;
pub const OXYOS_EVENT_RELEASED: u32 = 9;
pub const OXYOS_EVENT_SCROLL: u32 = 11;
pub const OXYOS_EVENT_TEXT: u32 = 12;
pub const OXYOS_EVENT_PREEDIT: u32 = 13;
pub const OXYOS_EVENT_ACCESSIBILITY_ACTION: u32 = 14;
pub const OXYOS_EVENT_CLIPBOARD_TYPES: u32 = 20;
pub const OXYOS_EVENT_CLIPBOARD_TYPE: u32 = 21;
pub const OXYOS_EVENT_CLIPBOARD_EMPTY: u32 = 22;
/// A transaction for a field with text input enabled is complete (its Done's opcode); its
/// contents come from `oxyos_client_text_transaction`.
pub const OXYOS_EVENT_TEXT_INPUT: u32 = 29;
/// To the session's shell (ADR 0009): a window's state, a chunk of its title or application id,
/// its closing, and the workspaces.
pub const OXYOS_EVENT_WINDOW_STATE: u32 = 37;
pub const OXYOS_EVENT_WINDOW_TITLE: u32 = 38;
pub const OXYOS_EVENT_WINDOW_APP_ID: u32 = 39;
pub const OXYOS_EVENT_WINDOW_CLOSED: u32 = 40;
pub const OXYOS_EVENT_WORKSPACES: u32 = 41;
/// To an application: the shell asks it to close a window.
pub const OXYOS_EVENT_CLOSE_REQUESTED: u32 = 42;
/// A touchpad pinch over one of the client's surfaces.
pub const OXYOS_EVENT_PINCH: u32 = 43;
/// A pinch's phase (OxyOSEvent.detail of a PINCH event).
pub const OXYOS_PINCH_BEGIN: u32 = 1;
pub const OXYOS_PINCH_UPDATE: u32 = 2;
pub const OXYOS_PINCH_END: u32 = 3;
pub const OXYOS_PINCH_CANCEL: u32 = 4;
/// A window's state bits (OxyOSEvent.flag of a WINDOW_STATE event).
pub const OXYOS_WINDOW_MINIMIZED: u8 = 1;
pub const OXYOS_WINDOW_FOCUSED: u8 = 2;
/// Pointer shapes (`oxyos_client_set_pointer_shape`), the protocol's.
pub const OXYOS_POINTER_ARROW: u8 = 1;
pub const OXYOS_POINTER_TEXT: u8 = 2;
pub const OXYOS_POINTER_HAND: u8 = 3;
pub const OXYOS_POINTER_NOT_ALLOWED: u8 = 4;
pub const OXYOS_POINTER_WAIT: u8 = 5;
pub const OXYOS_POINTER_HELP: u8 = 6;
pub const OXYOS_POINTER_CROSSHAIR: u8 = 7;
pub const OXYOS_POINTER_MOVE: u8 = 8;
pub const OXYOS_POINTER_GRAB: u8 = 9;
pub const OXYOS_POINTER_GRABBING: u8 = 10;
pub const OXYOS_POINTER_RESIZE_HORIZONTAL: u8 = 11;
pub const OXYOS_POINTER_RESIZE_VERTICAL: u8 = 12;
pub const OXYOS_POINTER_RESIZE_TOP_LEFT: u8 = 13;
pub const OXYOS_POINTER_RESIZE_TOP_RIGHT: u8 = 14;
pub const OXYOS_POINTER_ZOOM_IN: u8 = 15;
pub const OXYOS_POINTER_ZOOM_OUT: u8 = 16;
/// What the shell asks of windows and workspaces (`oxyos_client_manage`).
pub const OXYOS_MANAGE_FOCUS: u8 = 1;
pub const OXYOS_MANAGE_CLOSE: u8 = 2;
pub const OXYOS_MANAGE_MINIMIZE: u8 = 3;
pub const OXYOS_MANAGE_MOVE: u8 = 4;
pub const OXYOS_MANAGE_ACTIVATE_WORKSPACE: u8 = 5;
pub const OXYOS_MANAGE_ADD_WORKSPACE: u8 = 6;
pub const OXYOS_MANAGE_REMOVE_WORKSPACE: u8 = 7;

/// Pixel formats (DRM fourcc codes), as the protocol's `Format`.
pub const OXYOS_FORMAT_XRGB8888: u32 = 0x3432_5258;
pub const OXYOS_FORMAT_ARGB8888: u32 = 0x3432_5241;

/// Results of `oxyos_client_poll`.
pub const OXYOS_POLL_EVENT: c_int = 1;
pub const OXYOS_POLL_EMPTY: c_int = 0;
pub const OXYOS_POLL_CLOSED: c_int = -1;
pub const OXYOS_ERROR: c_int = -2;

/// One compositor event. Which fields are meaningful depends on `kind`; see `oxyos_client.h`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OxyOSEvent {
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
    pub scale_65536: u32,
    pub rotation_256: i32,
    pub flag: u8,
    /// A text event's UTF-8, NUL-terminated.
    pub text: [u8; TEXT_CHUNK_BYTES + 1],
}

impl Default for OxyOSEvent {
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
            scale_65536: 0,
            rotation_256: 0,
            flag: 0,
            text: [0; TEXT_CHUNK_BYTES + 1],
        }
    }
}

/// A popup's placement, as the protocol's `Positioner`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyOSPositioner {
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

/// One node of a surface's accessibility snapshot, as `oxyos_client_publish_accessibility` takes it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSAccessibleNode {
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

/// A focused text field, as `oxyos_client_text_input` takes it (ADR 0008).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSTextField {
    pub purpose: u8,
    pub hints: u32,
    pub text: *const c_char,
    pub length: usize,
    pub anchor: u32,
    pub cursor: u32,
    pub has_caret: u8,
    pub caret_x: i32,
    pub caret_y: i32,
    pub caret_width: u32,
    pub caret_height: u32,
}

/// One styled segment of a composition, in bytes of it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OxyOSPreeditSegment {
    pub begin: u32,
    pub end: u32,
    pub style: u8,
}

/// A transaction for a field, as `oxyos_client_text_transaction` gives it. Its strings and
/// segments stay valid until the next `oxyos_client_poll`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSTextTransaction {
    pub surface: u32,
    pub delete_before: u32,
    pub delete_after: u32,
    pub cursor_begin: u32,
    pub cursor_end: u32,
    pub commit_serial: u32,
    pub serial: u64,
    pub commit: *const c_char,
    pub commit_length: usize,
    pub preedit: *const c_char,
    pub preedit_length: usize,
    pub segments: *const OxyOSPreeditSegment,
    pub segment_count: usize,
}

/// An opaque connection, owned by the caller between `oxyos_client_connect` and
/// `oxyos_client_close`.
pub struct OxyOSClient {
    client: Client,
    /// The segments of the transaction last completed, as C reads them.
    segments: Vec<OxyOSPreeditSegment>,
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn fail(message: impl ToString) -> c_int {
    let message = message.to_string().replace('\0', " ");
    LAST_ERROR.with(|last| *last.borrow_mut() = CString::new(message).unwrap_or_default());
    OXYOS_ERROR
}

fn status(result: std::io::Result<()>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

/// Converts a compositor event; `None` for events a client never sees (the handshake).
pub fn convert(event: &Event) -> Option<OxyOSEvent> {
    let mut out = OxyOSEvent::default();
    match *event {
        Event::Configure {
            surface,
            serial,
            rect,
            scale_120,
            fullscreen,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_CONFIGURE,
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
            out = OxyOSEvent {
                kind: OXYOS_EVENT_POPUP_DONE,
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
            out = OxyOSEvent {
                kind: OXYOS_EVENT_POINTER,
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
            out = OxyOSEvent {
                kind: OXYOS_EVENT_KEY,
                surface,
                serial,
                code: key,
                flag: pressed.into(),
                ..out
            }
        }
        Event::Focus { surface, focused } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_FOCUS,
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
            out = OxyOSEvent {
                kind: OXYOS_EVENT_TOUCH,
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
            out = OxyOSEvent {
                kind: OXYOS_EVENT_FEEDBACK,
                surface,
                serial: sequence,
                detail: outcome as u32,
                ..out
            }
        }
        Event::Released { surface, buffer } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_RELEASED,
                surface,
                serial: buffer,
                ..out
            }
        }
        Event::Pinch {
            surface,
            serial,
            phase,
            x,
            y,
            scale_65536,
            rotation_256,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_PINCH,
                surface,
                serial,
                x,
                y,
                detail: phase as u32,
                scale_65536,
                rotation_256,
                ..out
            }
        }
        Event::Scroll {
            surface,
            serial,
            dx,
            dy,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_SCROLL,
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
            out = OxyOSEvent {
                kind: OXYOS_EVENT_TEXT,
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
            out = OxyOSEvent {
                kind: OXYOS_EVENT_PREEDIT,
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
            out = OxyOSEvent {
                kind: OXYOS_EVENT_ACCESSIBILITY_ACTION,
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
        // What the clipboard holds, for the client with keyboard focus (ADR 0007).
        Event::ClipboardTypes { selection, count } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_CLIPBOARD_TYPES,
                serial: selection.into(),
                detail: count.into(),
                ..out
            }
        }
        Event::ClipboardType { selection, mime } => {
            let bytes = mime.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: OXYOS_EVENT_CLIPBOARD_TYPE,
                serial: selection.into(),
                ..out
            }
        }
        Event::ClipboardEmpty {} => out.kind = OXYOS_EVENT_CLIPBOARD_EMPTY,
        // Answered by the client library, which holds what this client copied.
        Event::ClipboardSend { .. } => return None,
        // A transaction's parts are collected by the client library; its Done completes it.
        Event::TextInputDelete { .. }
        | Event::TextInputCommitText { .. }
        | Event::TextInputPreeditText { .. }
        | Event::TextInputPreeditCursor { .. }
        | Event::TextInputPreeditSegment { .. } => return None,
        // Only to the input method, which is never a C client.
        Event::InputMethodActivate {}
        | Event::InputMethodDeactivate {}
        | Event::InputMethodPurpose { .. }
        | Event::InputMethodSurrounding { .. }
        | Event::InputMethodSelection { .. }
        | Event::InputMethodDone { .. }
        | Event::InputMethodKey { .. } => return None,
        Event::WindowState {
            window,
            workspace,
            minimized,
            focused,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_WINDOW_STATE,
                code: window,
                detail: workspace,
                flag: u8::from(minimized) * OXYOS_WINDOW_MINIMIZED
                    + u8::from(focused) * OXYOS_WINDOW_FOCUSED,
                ..out
            }
        }
        Event::WindowTitle {
            window,
            index,
            last,
            text,
        }
        | Event::WindowAppId {
            window,
            index,
            last,
            text,
        } => {
            let bytes = text.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: if matches!(event, Event::WindowTitle { .. }) {
                    OXYOS_EVENT_WINDOW_TITLE
                } else {
                    OXYOS_EVENT_WINDOW_APP_ID
                },
                code: window,
                detail: u32::from(index),
                flag: u8::from(last),
                ..out
            }
        }
        Event::WindowClosed { window } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_WINDOW_CLOSED,
                code: window,
                ..out
            }
        }
        Event::Workspaces { count, active } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_WORKSPACES,
                detail: count,
                code: active,
                ..out
            }
        }
        Event::CloseRequested { surface } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_CLOSE_REQUESTED,
                surface,
                ..out
            }
        }
        Event::TextInputDone {
            surface,
            serial,
            commit,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_TEXT_INPUT,
                surface,
                serial,
                detail: commit,
                ..out
            }
        }
    }
    Some(out)
}

/// Converts a C positioner; `None` if an enumeration value is not the protocol's.
pub fn positioner(p: &OxyOSPositioner) -> Option<Positioner> {
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
// `no_mangle` exports a symbol the linker may collide with; the `oxyos_` prefix owns these names.
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_last_error() -> *const c_char {
    LAST_ERROR.with(|last| last.borrow().as_ptr())
}

/// Connects to the compositor socket at `path` and completes the handshake. Returns NULL on
/// failure (see `oxyos_last_error`).
///
/// # Safety
/// `path` is a NUL-terminated string valid for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_connect(path: *const c_char) -> *mut OxyOSClient {
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
        Ok(client) => Box::into_raw(Box::new(OxyOSClient {
            client,
            segments: Vec::new(),
        })),
        Err(error) => {
            fail(error);
            std::ptr::null_mut()
        }
    }
}

/// Closes the connection and frees the client. NULL is ignored.
///
/// # Safety
/// `client` is NULL or a pointer from `oxyos_client_connect` not yet closed; it is invalid after.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_close(client: *mut OxyOSClient) {
    if !client.is_null() {
        // SAFETY: the pointer came from Box::into_raw in oxyos_client_connect and is closed once.
        drop(unsafe { Box::from_raw(client) });
    }
}

/// Reborrows a caller's client pointer.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`, used by one thread at a time.
#[allow(unsafe_code)]
unsafe fn client<'a>(client: *mut OxyOSClient) -> Option<&'a mut Client> {
    // SAFETY: per the contract, a non-null pointer is a live, exclusively used OxyOSClient.
    unsafe { client.as_mut() }.map(|c| &mut c.client)
}

/// The connection's descriptor, readable when an event is queued (for poll/epoll).
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_fd(client: *mut OxyOSClient) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => client.as_fd().as_raw_fd(),
        None => fail("null client"),
    }
}

/// Sends one request; 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
unsafe fn send(client: *mut OxyOSClient, request: Request) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.send(&request)),
        None => fail("null client"),
    }
}

/// Requests a shell surface (the session's shell, with the `shell` grant): `kind` is
/// OXYOS_SHELL_BACKGROUND, _PANEL, _OVERLAY or _NOTIFICATION, `edge` OXYOS_EDGE_*, on `output`, of
/// the given logical size (a panel's thickness along its edge; a background takes the output),
/// keeping `exclusive` logical pixels of a panel's edge out of where windows are placed. The
/// compositor answers with a configure. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_create_shell_surface(
    client: *mut OxyOSClient,
    surface: u32,
    kind: u8,
    edge: u8,
    output: u32,
    width: u32,
    height: u32,
    exclusive: u32,
) -> c_int {
    let (Some(kind), Some(edge)) = (ShellKind::from_raw(kind), PanelEdge::from_raw(edge)) else {
        return fail("unknown shell surface kind or edge");
    };
    let request = Request::CreateShellSurface {
        surface,
        kind,
        edge,
        output,
        size: Size::new(width, height),
        exclusive,
    };
    // SAFETY: forwarded caller contract.
    unsafe { send(client, request) }
}

/// Names a window (ADR 0009): its title, or with a non-zero `app_id` its application id
/// (`length` bytes of UTF-8; at most 256 bytes for a title, 128 for an id). 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `text` points to `length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_name_window(
    client: *mut OxyOSClient,
    surface: u32,
    app_id: u8,
    text: *const c_char,
    length: usize,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    let bytes = if length == 0 {
        &[][..]
    } else if text.is_null() {
        return fail("null text");
    } else {
        // SAFETY: per the contract, `text` points to `length` readable bytes.
        unsafe { std::slice::from_raw_parts(text.cast::<u8>(), length) }
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return fail("a name is UTF-8");
    };
    status(client.name_window(surface, app_id != 0, text))
}

/// What the session's shell asks of windows and workspaces (ADR 0009, the `shell` grant):
/// OXYOS_MANAGE_FOCUS, _CLOSE (the application is asked), _MINIMIZE or _MOVE (to `workspace`)
/// `window`; _ACTIVATE_WORKSPACE, _ADD_WORKSPACE or _REMOVE_WORKSPACE. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_manage(
    client: *mut OxyOSClient,
    action: u8,
    window: u32,
    workspace: u32,
) -> c_int {
    let request = match action {
        OXYOS_MANAGE_FOCUS => Request::FocusWindow { window },
        OXYOS_MANAGE_CLOSE => Request::CloseWindow { window },
        OXYOS_MANAGE_MINIMIZE => Request::MinimizeWindow { window },
        OXYOS_MANAGE_MOVE => Request::MoveWindowToWorkspace { window, workspace },
        OXYOS_MANAGE_ACTIVATE_WORKSPACE => Request::ActivateWorkspace { workspace },
        OXYOS_MANAGE_ADD_WORKSPACE => Request::AddWorkspace {},
        OXYOS_MANAGE_REMOVE_WORKSPACE => Request::RemoveWorkspace { workspace },
        _ => return fail("unknown action"),
    };
    // SAFETY: forwarded caller contract.
    unsafe { send(client, request) }
}

/// The pointer's shape while it is over `surface`: one of OXYOS_POINTER_*. 0 on success, or
/// OXYOS_ERROR for a shape that is not one.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_pointer_shape(
    client: *mut OxyOSClient,
    surface: u32,
    shape: u8,
) -> c_int {
    let Some(shape) = oxyos_protocol::PointerShape::from_raw(shape) else {
        return fail("not a pointer shape");
    };
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::SetPointerShape { surface, shape }) }
}

/// Limits where a surface takes pointer and touch input to a surface-local logical rectangle
/// (an empty one: nowhere); input elsewhere reaches what is below. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_input_region(
    client: *mut OxyOSClient,
    surface: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> c_int {
    let region = Rect::new(x, y, width, height);
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::SetInputRegion { surface, region }) }
}

/// Requests a toplevel window of the given logical size; the compositor answers with a
/// configure. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_create_toplevel(
    client: *mut OxyOSClient,
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
/// `client` is a live pointer from `oxyos_client_connect`; `positioner` points to a valid
/// `OxyOSPositioner` for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_create_popup(
    client: *mut OxyOSClient,
    surface: u32,
    parent: u32,
    positioner: *const OxyOSPositioner,
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
/// `client` is a live pointer from `oxyos_client_connect`; `positioner` points to a valid
/// `OxyOSPositioner` for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_reposition(
    client: *mut OxyOSClient,
    surface: u32,
    positioner: *const OxyOSPositioner,
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
/// `client` is a live pointer from `oxyos_client_connect`; `nodes` points to `count` nodes (or is
/// NULL with `count` 0), whose `name` and `value` are NULL or NUL-terminated strings.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_publish_accessibility(
    client: *mut OxyOSClient,
    surface: u32,
    nodes: *const OxyOSAccessibleNode,
    count: usize,
) -> c_int {
    let nodes: &[OxyOSAccessibleNode] = if count == 0 {
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
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_grab_popup(
    client: *mut OxyOSClient,
    surface: u32,
    serial: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::GrabPopup { surface, serial }) }
}

/// Destroys a surface and the buffers it drew with. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_destroy_surface(
    client: *mut OxyOSClient,
    surface: u32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.destroy_surface(surface)),
        None => fail("null client"),
    }
}

/// Presents `pixels` (rows of `width * 4` bytes, `height` rows) on `surface` in answer to the
/// configure with serial `configure`. `format` is a DRM fourcc: `OXYOS_FORMAT_XRGB8888` (opaque,
/// little-endian B, G, R, X) or `OXYOS_FORMAT_ARGB8888` (premultiplied B, G, R, A). Returns the
/// frame's sequence number, or -1 on failure.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `pixels` points to `length` readable
/// bytes for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_present(
    client: *mut OxyOSClient,
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

/// A rectangle of a buffer, in its pixels: what changed in a frame (`oxyos_client_present_damaged`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Copies `length` bytes of UTF-8 text at `text` as the clipboard (`text/plain;charset=utf-8`),
/// with the serial of the user input that asked for it (ADR 0007). The client library keeps the
/// text and writes it into each paste itself. 0 on success; the compositor may still refuse the
/// copy (a serial that is not a recent input of this client, or keyboard focus lost).
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `text` points to `length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_copy_text(
    client: *mut OxyOSClient,
    serial: u64,
    text: *const c_char,
    length: usize,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    if text.is_null() && length > 0 {
        return fail("null text");
    }
    let bytes = if length == 0 {
        Vec::new()
    } else {
        // SAFETY: per the contract, `text` points to `length` readable bytes.
        unsafe { std::slice::from_raw_parts(text.cast::<u8>(), length) }.to_vec()
    };
    if std::str::from_utf8(&bytes).is_err() {
        return fail("text is not UTF-8");
    }
    let mime = TextChunk::new(CLIPBOARD_TEXT).expect("a short type");
    status(client.copy(serial, vec![(mime, bytes)]))
}

/// Empties the clipboard if this client's copy is what it holds. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_clear_clipboard(
    client: *mut OxyOSClient,
    serial: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.clear_clipboard(serial)),
        None => fail("null client"),
    }
}

/// Pastes `mime` (`mime_length` bytes, at most 32) of the clipboard content the compositor
/// named `selection` (a CLIPBOARD_TYPES event's serial), with the serial of the user input that
/// asked for it. Returns the read end of a pipe the content arrives through, which the caller
/// reads to its end and closes; it ends at once, empty, if the paste was refused. OXYOS_ERROR on
/// failure.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `mime` points to `mime_length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_paste(
    client: *mut OxyOSClient,
    selection: u32,
    mime: *const c_char,
    mime_length: usize,
    serial: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    if mime.is_null() {
        return fail("null type");
    }
    // SAFETY: per the contract, `mime` points to `mime_length` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(mime.cast::<u8>(), mime_length) };
    let Some(mime) = std::str::from_utf8(bytes).ok().and_then(TextChunk::new) else {
        return fail("a type is UTF-8 of 1 to 32 bytes");
    };
    match client.receive_clipboard(selection, mime, serial) {
        Ok(pipe) => std::os::fd::IntoRawFd::into_raw_fd(pipe),
        Err(error) => fail(error),
    }
}

/// [`oxyos_client_present`] for a frame of which only `count` rectangles at `damage` changed
/// since the surface's previous frame (none: everything). The compositor hears the damage, and
/// a reused buffer receives only what changed since it was last written
/// (docs/adr/0005-damage.mdx).
///
/// # Safety
/// As [`oxyos_client_present`]; `damage` points to `count` readable `OxyOSRect`s (any pointer when
/// `count` is 0).
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_present_damaged(
    client: *mut OxyOSClient,
    surface: u32,
    configure: u64,
    width: u32,
    height: u32,
    format: u32,
    pixels: *const u8,
    length: usize,
    damage: *const OxyOSRect,
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

/// Takes one queued event without blocking: `OXYOS_POLL_EVENT` with `*out` filled,
/// `OXYOS_POLL_EMPTY`, `OXYOS_POLL_CLOSED` once the compositor closed the connection, or `OXYOS_ERROR`
/// (a protocol error ends the connection).
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `out` points to a writable `OxyOSEvent`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_poll(
    client: *mut OxyOSClient,
    out: *mut OxyOSEvent,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    // SAFETY: the caller passes a writable OxyOSEvent or NULL.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return fail("null event");
    };
    loop {
        match client.poll() {
            Ok(Poll::Event(event)) => {
                if let Some(event) = convert(&event) {
                    *out = event;
                    return OXYOS_POLL_EVENT;
                }
            }
            Ok(Poll::Empty) => return OXYOS_POLL_EMPTY,
            Ok(Poll::Closed) => return OXYOS_POLL_CLOSED,
            Err(error) => return fail(error),
        }
    }
}

/// Tells the compositor about the text field focused in `surface` (ADR 0008), or that none is
/// (`field` NULL): its purpose (OXYOS_PURPOSE_*), hints (OXYOS_HINT_* bits), its text (`length`
/// bytes of UTF-8, the composition left out) with the selection's byte offsets in it, and its
/// caret's rectangle in surface coordinates if `has_caret`. Only what changed is sent. A
/// non-zero `new_field` describes another field than the one told before. Returns the commit
/// serial transactions for this state carry (0 once disabled), or -1 on failure.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `field` is NULL or points to a
/// valid `OxyOSTextField` whose `text` points to `length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_text_input(
    client: *mut OxyOSClient,
    surface: u32,
    field: *const OxyOSTextField,
    new_field: u8,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    // SAFETY: the caller passes NULL or a valid field.
    let result = match unsafe { field.as_ref() } {
        None => client.text_input(surface, None, new_field != 0),
        Some(field) => {
            let bytes = if field.length == 0 {
                &[][..]
            } else if field.text.is_null() {
                fail("null text");
                return -1;
            } else {
                // SAFETY: per the contract, `text` points to `length` readable bytes.
                unsafe { std::slice::from_raw_parts(field.text.cast::<u8>(), field.length) }
            };
            let (Some(purpose), Some(hints), Ok(text)) = (
                TextPurpose::from_raw(field.purpose),
                TextHints::new(field.hints),
                std::str::from_utf8(bytes),
            ) else {
                fail("invalid purpose, hints or text");
                return -1;
            };
            let (anchor, cursor) = (field.anchor as usize, field.cursor as usize);
            if !text.is_char_boundary(anchor) || !text.is_char_boundary(cursor) {
                fail("selection outside the text or inside a character");
                return -1;
            }
            let caret = (field.has_caret != 0).then(|| {
                Rect::new(
                    field.caret_x,
                    field.caret_y,
                    field.caret_width,
                    field.caret_height,
                )
            });
            let field = Field {
                purpose,
                hints,
                text,
                anchor,
                cursor,
                caret,
            };
            client.text_input(surface, Some(&field), new_field != 0)
        }
    };
    match result {
        Ok(serial) => serial.into(),
        Err(error) => {
            fail(error);
            -1
        }
    }
}

/// Fills `*out` with the transaction the TEXT_INPUT event just returned completed: 1, or 0 if
/// the last event was not one. Its strings (not NUL-terminated) and segments stay valid until the
/// next `oxyos_client_poll`.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `out` points to a writable
/// `OxyOSTextTransaction`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_text_transaction(
    client: *mut OxyOSClient,
    out: *mut OxyOSTextTransaction,
) -> c_int {
    // SAFETY: per the contract, a non-null pointer is a live, exclusively used OxyOSClient.
    let Some(wrapper) = (unsafe { client.as_mut() }) else {
        return fail("null client");
    };
    // SAFETY: the caller passes a writable transaction or NULL.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return fail("null transaction");
    };
    let Some(transaction) = wrapper.client.transaction() else {
        return 0;
    };
    wrapper.segments = transaction
        .segments
        .iter()
        .map(|&(begin, end, style)| OxyOSPreeditSegment {
            begin,
            end,
            style: style as u8,
        })
        .collect();
    *out = OxyOSTextTransaction {
        surface: transaction.surface,
        delete_before: transaction.delete_before,
        delete_after: transaction.delete_after,
        cursor_begin: transaction.cursor.0,
        cursor_end: transaction.cursor.1,
        commit_serial: transaction.commit_serial,
        serial: transaction.serial,
        commit: transaction.commit.as_ptr().cast(),
        commit_length: transaction.commit.len(),
        preedit: transaction.preedit.as_ptr().cast(),
        preedit_length: transaction.preedit.len(),
        segments: wrapper.segments.as_ptr(),
        segment_count: wrapper.segments.len(),
    };
    1
}
