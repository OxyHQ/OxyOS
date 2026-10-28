//! C ABI over `oxyos-net`, the HTTP engine behind React Native's networking in the Bloom host. It
//! lives in this library so that the host links one Rust library (one standard library, one
//! allocator), not one per crate.
//!
//! Strings cross as a pointer and a length, never NUL-terminated: JavaScript's strings may hold
//! NUL, and bodies are bytes.

use super::fail;
use oxyos_net::{Answer, Network, Request};
use rustix::fd::{AsFd, AsRawFd};
use std::ffi::{c_char, c_int};
use std::time::Duration;

/// Answer kinds (`OxyOSNetAnswer.kind`).
pub const OXYOS_NET_RESPONSE: u32 = 1;
pub const OXYOS_NET_CHUNK: u32 = 2;
pub const OXYOS_NET_DATA: u32 = 3;
pub const OXYOS_NET_COMPLETE: u32 = 4;

/// Results of `oxyos_net_answer`.
pub const OXYOS_NET_ANSWER: c_int = 1;
pub const OXYOS_NET_EMPTY: c_int = 0;

/// One header: name and value, each a pointer and a length.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSNetHeader {
    pub name: *const c_char,
    pub name_length: usize,
    pub value: *const c_char,
    pub value_length: usize,
}

/// A request, as `oxyos_net_send` takes it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSNetRequest {
    pub id: i64,
    pub method: *const c_char,
    pub method_length: usize,
    pub url: *const c_char,
    pub url_length: usize,
    pub headers: *const OxyOSNetHeader,
    pub header_count: usize,
    pub body: *const u8,
    pub body_length: usize,
    /// Milliseconds for the whole request; 0 for none.
    pub timeout_ms: u64,
    /// Whether `body` is sent (a request may send an empty body, or none).
    pub has_body: u8,
    pub with_credentials: u8,
    pub incremental: u8,
}

/// One answer, as `oxyos_net_answer` gives it. Which fields are meaningful depends on `kind`; see
/// `oxyos_client.h`. Its pointers stay valid until the next `oxyos_net_answer` or `oxyos_net_free`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSNetAnswer {
    pub kind: u32,
    pub status: u32,
    pub id: i64,
    pub headers: *const OxyOSNetHeader,
    pub header_count: usize,
    pub url: *const c_char,
    pub url_length: usize,
    pub bytes: *const u8,
    pub length: usize,
    pub progress: u64,
    /// A chunk's expected total, or -1 if unknown.
    pub total: i64,
    /// Data's `Content-Type`, NULL if it had none.
    pub content_type: *const c_char,
    pub content_type_length: usize,
    /// A completion's failure, NULL if it succeeded.
    pub error: *const c_char,
    pub error_length: usize,
    pub timed_out: u8,
}

impl Default for OxyOSNetAnswer {
    fn default() -> Self {
        Self {
            kind: 0,
            status: 0,
            id: 0,
            headers: std::ptr::null(),
            header_count: 0,
            url: std::ptr::null(),
            url_length: 0,
            bytes: std::ptr::null(),
            length: 0,
            progress: 0,
            total: -1,
            content_type: std::ptr::null(),
            content_type_length: 0,
            error: std::ptr::null(),
            error_length: 0,
            timed_out: 0,
        }
    }
}

/// The network, owned by the caller between `oxyos_net_new` and `oxyos_net_free`; used by one thread
/// at a time.
pub struct OxyOSNet {
    network: Network,
    /// The answer last given, which its `OxyOSNetAnswer`'s pointers point into.
    given: Option<(Answer, Vec<OxyOSNetHeader>)>,
}

fn text(text: &str) -> (*const c_char, usize) {
    (text.as_ptr().cast(), text.len())
}

fn optional(text: Option<&str>) -> (*const c_char, usize) {
    text.map_or((std::ptr::null(), 0), self::text)
}

/// The `length` bytes at `data`; NULL with a length of 0 is empty.
///
/// # Safety
/// `data` is NULL with `length` 0, or points to `length` readable bytes that outlive `'a`.
#[allow(unsafe_code)]
unsafe fn bytes<'a>(data: *const u8, length: usize) -> Option<&'a [u8]> {
    if data.is_null() {
        return (length == 0).then_some(&[]);
    }
    // SAFETY: per the contract, `data` points to `length` readable bytes.
    Some(unsafe { std::slice::from_raw_parts(data, length) })
}

/// A caller's string, as UTF-8.
///
/// # Safety
/// As [`bytes`].
#[allow(unsafe_code)]
unsafe fn string(data: *const c_char, length: usize, what: &str) -> Result<String, String> {
    // SAFETY: forwarded caller contract.
    let bytes = unsafe { bytes(data.cast(), length) }.ok_or(format!("null {what}"))?;
    String::from_utf8(bytes.to_vec()).map_err(|_| format!("{what} is not UTF-8"))
}

/// Reborrows a caller's network pointer.
///
/// # Safety
/// `net` is NULL or a live pointer from `oxyos_net_new`, used by one thread at a time.
#[allow(unsafe_code)]
unsafe fn net<'a>(net: *mut OxyOSNet) -> Option<&'a mut OxyOSNet> {
    // SAFETY: per the contract, a non-null pointer is a live, exclusively used OxyOSNet.
    unsafe { net.as_mut() }
}

/// A new network with an empty cookie jar. Returns NULL on failure (see `oxyos_last_error`).
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_net_new() -> *mut OxyOSNet {
    match Network::new() {
        Ok(network) => Box::into_raw(Box::new(OxyOSNet {
            network,
            given: None,
        })),
        Err(error) => {
            fail(error);
            std::ptr::null_mut()
        }
    }
}

/// Frees the network. Requests still running end unheard. NULL is ignored.
///
/// # Safety
/// `net` is NULL or a pointer from `oxyos_net_new` not yet freed; it is invalid after.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_net_free(net: *mut OxyOSNet) {
    if !net.is_null() {
        // SAFETY: the pointer came from Box::into_raw in oxyos_net_new and is freed once.
        drop(unsafe { Box::from_raw(net) });
    }
}

/// The network's descriptor, readable while answers are queued (for poll/epoll).
///
/// # Safety
/// `net` is a live pointer from `oxyos_net_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_net_fd(net: *mut OxyOSNet) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::net(net) } {
        Some(net) => net.network.as_fd().as_raw_fd(),
        None => fail("null network"),
    }
}

/// Starts a request; its answers follow through `oxyos_net_answer`, ending with a completion. 0 on
/// success; on failure (a malformed request) nothing is sent and no answer follows.
///
/// # Safety
/// `net` is a live pointer from `oxyos_net_new`; `request` points to a readable `OxyOSNetRequest`
/// whose pointers are valid for their lengths for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_net_send(
    net: *mut OxyOSNet,
    request: *const OxyOSNetRequest,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(net) = (unsafe { self::net(net) }) else {
        return fail("null network");
    };
    // SAFETY: the caller passes a readable OxyOSNetRequest or NULL.
    let Some(request) = (unsafe { request.as_ref() }) else {
        return fail("null request");
    };
    // SAFETY: the request's pointers are valid for their lengths (contract above).
    match unsafe { convert(request) } {
        Ok(request) => {
            net.network.send(request);
            0
        }
        Err(error) => fail(error),
    }
}

/// # Safety
/// As `oxyos_net_send`'s request.
#[allow(unsafe_code)]
unsafe fn convert(request: &OxyOSNetRequest) -> Result<Request, String> {
    let headers: &[OxyOSNetHeader] = match (request.headers.is_null(), request.header_count) {
        (true, 0) => &[],
        (true, _) => return Err("null headers".into()),
        // SAFETY: the caller's headers point to `header_count` readable OxyOSNetHeaders.
        (false, count) => unsafe { std::slice::from_raw_parts(request.headers, count) },
    };
    let headers = headers
        .iter()
        .map(|header| {
            // SAFETY: each header's strings are valid for their lengths.
            unsafe {
                Ok((
                    string(header.name, header.name_length, "header name")?,
                    string(header.value, header.value_length, "header value")?,
                ))
            }
        })
        .collect::<Result<_, String>>()?;
    let body = if request.has_body != 0 {
        // SAFETY: the body is valid for its length.
        let body = unsafe { bytes(request.body, request.body_length) };
        Some(body.ok_or("null body")?.to_vec())
    } else {
        None
    };
    Ok(Request {
        id: request.id,
        // SAFETY: the method and URL are valid for their lengths.
        method: unsafe { string(request.method, request.method_length, "method") }?,
        // SAFETY: as above.
        url: unsafe { string(request.url, request.url_length, "URL") }?,
        headers,
        body,
        timeout: (request.timeout_ms > 0).then(|| Duration::from_millis(request.timeout_ms)),
        with_credentials: request.with_credentials != 0,
        incremental: request.incremental != 0,
    })
}

/// Delivers nothing more of request `id`, not even its completion. 0 on success.
///
/// # Safety
/// `net` is a live pointer from `oxyos_net_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_net_abort(net: *mut OxyOSNet, id: i64) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::net(net) } {
        Some(net) => {
            net.network.abort(id);
            0
        }
        None => fail("null network"),
    }
}

/// Forgets every cookie. 0 on success.
///
/// # Safety
/// `net` is a live pointer from `oxyos_net_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_net_clear_cookies(net: *mut OxyOSNet) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::net(net) } {
        Some(net) => {
            net.network.clear_cookies();
            0
        }
        None => fail("null network"),
    }
}

/// Takes the next answer without blocking: `OXYOS_NET_ANSWER` with `*out` filled, `OXYOS_NET_EMPTY`,
/// or `OXYOS_ERROR`.
///
/// # Safety
/// `net` is a live pointer from `oxyos_net_new`; `out` points to a writable `OxyOSNetAnswer`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_net_answer(net: *mut OxyOSNet, out: *mut OxyOSNetAnswer) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(net) = (unsafe { self::net(net) }) else {
        return fail("null network");
    };
    // SAFETY: the caller passes a writable OxyOSNetAnswer or NULL.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return fail("null answer");
    };
    net.given = None;
    let Some(answer) = net.network.answer() else {
        return OXYOS_NET_EMPTY;
    };
    let headers = match &answer {
        Answer::Response { headers, .. } => headers
            .iter()
            .map(|(name, value)| {
                let ((name, name_length), (value, value_length)) = (text(name), text(value));
                OxyOSNetHeader {
                    name,
                    name_length,
                    value,
                    value_length,
                }
            })
            .collect(),
        _ => Vec::new(),
    };
    let mut view = OxyOSNetAnswer {
        id: answer.id(),
        ..OxyOSNetAnswer::default()
    };
    match &answer {
        Answer::Response { status, url, .. } => {
            view.kind = OXYOS_NET_RESPONSE;
            view.status = (*status).into();
            view.headers = headers.as_ptr();
            view.header_count = headers.len();
            (view.url, view.url_length) = text(url);
        }
        Answer::Chunk {
            bytes,
            progress,
            total,
            ..
        } => {
            view.kind = OXYOS_NET_CHUNK;
            (view.bytes, view.length) = (bytes.as_ptr(), bytes.len());
            view.progress = *progress;
            view.total = total
                .and_then(|total| i64::try_from(total).ok())
                .unwrap_or(-1);
        }
        Answer::Data {
            bytes,
            content_type,
            ..
        } => {
            view.kind = OXYOS_NET_DATA;
            (view.bytes, view.length) = (bytes.as_ptr(), bytes.len());
            (view.content_type, view.content_type_length) = optional(content_type.as_deref());
        }
        Answer::Complete {
            error, timed_out, ..
        } => {
            view.kind = OXYOS_NET_COMPLETE;
            (view.error, view.error_length) = optional(error.as_deref());
            view.timed_out = (*timed_out).into();
        }
    }
    // The pointers point into heap memory the answer and headers own, which moving them here
    // does not move.
    net.given = Some((answer, headers));
    *out = view;
    OXYOS_NET_ANSWER
}
