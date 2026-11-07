//! C ABI over `oxyos_net::websocket`, the engine behind React Native's `WebSocketModule` in the Bloom
//! host. Strings cross as a pointer and a length, never NUL-terminated.

use super::fail;
use super::net::{net, string, OxyOSNet, OxyOSNetHeader};
use oxyos_net::websocket::{Connect, Event, Options, WebSockets};
use rustix::fd::{AsFd, AsRawFd};
use std::ffi::{c_char, c_int};

/// Event kinds (`OxyOSWebSocketEvent.kind`).
pub const OXYOS_WEBSOCKET_OPEN: u32 = 1;
pub const OXYOS_WEBSOCKET_TEXT: u32 = 2;
pub const OXYOS_WEBSOCKET_BINARY: u32 = 3;
pub const OXYOS_WEBSOCKET_CLOSED: u32 = 4;
pub const OXYOS_WEBSOCKET_FAILED: u32 = 5;

/// Results of `oxyos_websockets_event`.
pub const OXYOS_WEBSOCKET_EVENT: c_int = 1;
pub const OXYOS_WEBSOCKET_EMPTY: c_int = 0;

/// A protocol a socket asks for.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSWebSocketProtocol {
    pub name: *const c_char,
    pub name_length: usize,
}

/// A socket to open, as `oxyos_websockets_connect` takes it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSWebSocketRequest {
    pub id: i64,
    pub url: *const c_char,
    pub url_length: usize,
    pub protocols: *const OxyOSWebSocketProtocol,
    pub protocol_count: usize,
    pub headers: *const OxyOSNetHeader,
    pub header_count: usize,
}

/// One event, as `oxyos_websockets_event` gives it: `data` is the protocol (OPEN), the text (TEXT),
/// the bytes (BINARY), the reason (CLOSED, with `code`) or the failure (FAILED). Valid until the next
/// `oxyos_websockets_event` or `oxyos_websockets_free`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSWebSocketEvent {
    pub kind: u32,
    pub code: u32,
    pub id: i64,
    pub data: *const u8,
    pub length: usize,
}

impl Default for OxyOSWebSocketEvent {
    fn default() -> Self {
        Self {
            kind: 0,
            code: 0,
            id: 0,
            data: std::ptr::null(),
            length: 0,
        }
    }
}

/// An application's WebSockets, owned by the caller between `oxyos_websockets_new` and
/// `oxyos_websockets_free`; used by one thread at a time.
pub struct OxyOSWebSockets {
    sockets: WebSockets,
    /// The event last given, which its `OxyOSWebSocketEvent` points into.
    given: Option<Event>,
}

/// # Safety
/// `sockets` is NULL or a live pointer from `oxyos_websockets_new`, used by one thread at a time.
#[allow(unsafe_code)]
unsafe fn sockets<'a>(sockets: *mut OxyOSWebSockets) -> Option<&'a mut OxyOSWebSockets> {
    // SAFETY: per the contract, a non-null pointer is a live, exclusively used OxyOSWebSockets.
    unsafe { sockets.as_mut() }
}

/// # Safety
/// `data` is NULL with `length` 0, or points to `length` readable bytes.
#[allow(unsafe_code)]
unsafe fn bytes<'a>(data: *const u8, length: usize) -> Option<&'a [u8]> {
    if data.is_null() {
        return (length == 0).then_some(&[]);
    }
    // SAFETY: per the contract, `data` points to `length` readable bytes.
    Some(unsafe { std::slice::from_raw_parts(data, length) })
}

/// # Safety
/// `items` is NULL with `count` 0, or points to `count` readable `T`s.
#[allow(unsafe_code)]
unsafe fn items<'a, T>(items: *const T, count: usize, what: &str) -> Result<&'a [T], String> {
    match (items.is_null(), count) {
        (true, 0) => Ok(&[]),
        (true, _) => Err(format!("null {what}")),
        // SAFETY: per the contract, `items` points to `count` readable Ts.
        (false, count) => Ok(unsafe { std::slice::from_raw_parts(items, count) }),
    }
}

/// An application's WebSockets, sending `net`'s cookies, with the system's TLS, connectivity and
/// proxy. NULL on failure (see `oxyos_last_error`).
///
/// # Safety
/// `net` is a live pointer from `oxyos_net_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_websockets_new(net: *mut OxyOSNet) -> *mut OxyOSWebSockets {
    // SAFETY: forwarded caller contract.
    let Some(net) = (unsafe { self::net(net) }) else {
        fail("null network");
        return std::ptr::null_mut();
    };
    let made = Options::system().and_then(|options| {
        WebSockets::new(net.network.clone(), options).map_err(|error| error.to_string())
    });
    match made {
        Ok(sockets) => Box::into_raw(Box::new(OxyOSWebSockets {
            sockets,
            given: None,
        })),
        Err(error) => {
            fail(error);
            std::ptr::null_mut()
        }
    }
}

/// Closes every open socket with 1001 (going away), waiting a second at most for their close
/// frames to be sent, and frees them; nothing more is heard. NULL is ignored.
///
/// # Safety
/// `sockets` is NULL or a pointer from `oxyos_websockets_new` not yet freed; it is invalid after.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_websockets_free(sockets: *mut OxyOSWebSockets) {
    if !sockets.is_null() {
        // SAFETY: the pointer came from Box::into_raw in oxyos_websockets_new and is freed once.
        drop(unsafe { Box::from_raw(sockets) });
    }
}

/// Readable while events are queued (for poll/epoll).
///
/// # Safety
/// `sockets` is a live pointer from `oxyos_websockets_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_websockets_fd(sockets: *mut OxyOSWebSockets) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::sockets(sockets) } {
        Some(sockets) => sockets.sockets.as_fd().as_raw_fd(),
        None => fail("null WebSockets"),
    }
}

/// Starts opening a socket; it opens or fails through `oxyos_websockets_event`. 0 on success;
/// OXYOS_ERROR for a malformed URL or header, or an id in use, and nothing follows.
///
/// # Safety
/// `sockets` is a live pointer from `oxyos_websockets_new`; `request` points to a readable
/// `OxyOSWebSocketRequest` whose pointers are valid for their lengths for the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_websockets_connect(
    sockets: *mut OxyOSWebSockets,
    request: *const OxyOSWebSocketRequest,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(sockets) = (unsafe { self::sockets(sockets) }) else {
        return fail("null WebSockets");
    };
    // SAFETY: the caller passes a readable request or NULL.
    let Some(request) = (unsafe { request.as_ref() }) else {
        return fail("null request");
    };
    // SAFETY: the request's pointers are valid for their lengths (contract above).
    let connect = unsafe {
        (|| -> Result<Connect, String> {
            let protocols = items(request.protocols, request.protocol_count, "protocols")?
                .iter()
                .map(|protocol| string(protocol.name, protocol.name_length, "protocol"))
                .collect::<Result<_, _>>()?;
            let headers = items(request.headers, request.header_count, "headers")?
                .iter()
                .map(|header| {
                    Ok((
                        string(header.name, header.name_length, "header name")?,
                        string(header.value, header.value_length, "header value")?,
                    ))
                })
                .collect::<Result<_, String>>()?;
            Ok(Connect {
                id: request.id,
                url: string(request.url, request.url_length, "URL")?,
                protocols,
                headers,
            })
        })()
    };
    match connect.and_then(|connect| sockets.sockets.connect(connect)) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

/// Sends a message: text (`binary` 0, UTF-8) or bytes. A socket that is not open ends instead (a
/// failure, then a close with code 0, as Android's module ends it). 0, or OXYOS_ERROR for text
/// that is not UTF-8.
///
/// # Safety
/// `sockets` is a live pointer from `oxyos_websockets_new`; `data` is valid for `length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_websockets_send(
    sockets: *mut OxyOSWebSockets,
    id: i64,
    data: *const u8,
    length: usize,
    binary: u8,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(sockets) = (unsafe { self::sockets(sockets) }) else {
        return fail("null WebSockets");
    };
    // SAFETY: as above.
    let Some(data) = (unsafe { bytes(data, length) }) else {
        return fail("null message");
    };
    if binary != 0 {
        sockets.sockets.send_binary(id, data.to_vec());
    } else {
        let Ok(text) = String::from_utf8(data.to_vec()) else {
            return fail("a text message is not UTF-8");
        };
        sockets.sockets.send_text(id, text);
    }
    0
}

/// Sends a ping frame, as `oxyos_websockets_send`. 0 on success.
///
/// # Safety
/// `sockets` is a live pointer from `oxyos_websockets_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_websockets_ping(sockets: *mut OxyOSWebSockets, id: i64) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::sockets(sockets) } {
        Some(sockets) => {
            sockets.sockets.ping(id);
            0
        }
        None => fail("null WebSockets"),
    }
}

/// Starts the close handshake (an opening socket fails at once; a closed or closing one is left
/// alone). 0, or OXYOS_ERROR for a code or reason no close frame may carry (a reserved code, a
/// reason over 123 bytes), changing nothing.
///
/// # Safety
/// `sockets` is a live pointer from `oxyos_websockets_new`; `reason` is valid for its length.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_websockets_close(
    sockets: *mut OxyOSWebSockets,
    id: i64,
    code: u32,
    reason: *const c_char,
    reason_length: usize,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(sockets) = (unsafe { self::sockets(sockets) }) else {
        return fail("null WebSockets");
    };
    // SAFETY: as above.
    let reason = match unsafe { string(reason, reason_length, "reason") } {
        Ok(reason) => reason,
        Err(error) => return fail(error),
    };
    let Ok(code) = u16::try_from(code) else {
        return fail(format!("{code} is not a close code (1000 to 4999)"));
    };
    match sockets.sockets.close(id, code, &reason) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

/// Takes the next event without blocking: `OXYOS_WEBSOCKET_EVENT` with `*out` filled,
/// `OXYOS_WEBSOCKET_EMPTY`, or `OXYOS_ERROR`.
///
/// # Safety
/// `sockets` is a live pointer from `oxyos_websockets_new`; `out` points to a writable
/// `OxyOSWebSocketEvent`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_websockets_event(
    sockets: *mut OxyOSWebSockets,
    out: *mut OxyOSWebSocketEvent,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(sockets) = (unsafe { self::sockets(sockets) }) else {
        return fail("null WebSockets");
    };
    // SAFETY: the caller passes a writable event or NULL.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return fail("null event");
    };
    sockets.given = None;
    let Some(event) = sockets.sockets.event() else {
        return OXYOS_WEBSOCKET_EMPTY;
    };
    let (kind, code, data): (u32, u32, &[u8]) = match &event {
        Event::Open { protocol, .. } => (OXYOS_WEBSOCKET_OPEN, 0, protocol.as_bytes()),
        Event::Text { text, .. } => (OXYOS_WEBSOCKET_TEXT, 0, text.as_bytes()),
        Event::Binary { bytes, .. } => (OXYOS_WEBSOCKET_BINARY, 0, bytes),
        Event::Closed { code, reason, .. } => {
            (OXYOS_WEBSOCKET_CLOSED, (*code).into(), reason.as_bytes())
        }
        Event::Failed { message, .. } => (OXYOS_WEBSOCKET_FAILED, 0, message.as_bytes()),
    };
    *out = OxyOSWebSocketEvent {
        kind,
        code,
        id: event.id(),
        data: data.as_ptr(),
        length: data.len(),
    };
    // The pointer points into heap memory the event owns, which moving it here does not move.
    sockets.given = Some(event);
    OXYOS_WEBSOCKET_EVENT
}
