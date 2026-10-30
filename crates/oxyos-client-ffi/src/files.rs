//! C ABI over `oxyos-files` (docs/adr/0028-file-access.mdx): the file service the Bloom host
//! gives an application's window as `OxyOSFiles`, beneath `expo-file-system` and the Bloom
//! adapter's places and Trash. Requests and answers are JSON with bytes beside them
//! (`oxyos_files::service`), so the ABI is the same few calls however many operations the
//! service has: one answered at once on the caller's thread, one queued and answered in order,
//! and the replies (answers and watched changes) taken from the service's queue.

use super::fail;
use oxyos_files::service::Reply;
use oxyos_files::{Scope, Service};
use rustix::fd::{AsFd, AsRawFd};
use std::ffi::{c_char, c_int};
use std::path::Path;

/// The service, behind the C ABI.
pub struct OxyOSFiles {
    service: Service,
}

/// An answer or an event (id 0), its JSON NUL-terminated, and its bytes.
pub struct OxyOSFilesReply {
    id: u64,
    json: Vec<u8>,
    bytes: Vec<u8>,
}

impl From<Reply> for OxyOSFilesReply {
    fn from(reply: Reply) -> Self {
        let mut json = reply.json.into_bytes();
        json.push(0);
        OxyOSFilesReply {
            id: reply.id,
            json,
            bytes: reply.bytes,
        }
    }
}

/// Reborrows a caller's pointer.
///
/// # Safety
/// `files` is NULL or a live pointer from `oxyos_files_new`.
#[allow(unsafe_code)]
unsafe fn files<'a>(files: *mut OxyOSFiles) -> Option<&'a OxyOSFiles> {
    // SAFETY: per the contract, a non-null pointer is a live OxyOSFiles.
    unsafe { files.as_ref() }
}

/// `length` bytes at `text` as UTF-8, or none for NULL.
///
/// # Safety
/// `text` is NULL or points to `length` readable bytes.
#[allow(unsafe_code)]
unsafe fn utf8<'a>(text: *const c_char, length: usize) -> Result<Option<&'a str>, &'static str> {
    if text.is_null() {
        return Ok(None);
    }
    // SAFETY: `text` points to `length` readable bytes (contract above).
    let bytes = unsafe { std::slice::from_raw_parts(text.cast::<u8>(), length) };
    std::str::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "not UTF-8")
}

/// `length` bytes at `bytes`, none for NULL.
///
/// # Safety
/// `bytes` is NULL or points to `length` readable bytes.
#[allow(unsafe_code)]
unsafe fn slice<'a>(bytes: *const u8, length: usize) -> &'a [u8] {
    if bytes.is_null() || length == 0 {
        return &[];
    }
    // SAFETY: `bytes` points to `length` readable bytes (contract above).
    unsafe { std::slice::from_raw_parts(bytes, length) }
}

/// The service for the user running the process (`Scope::for_user`), for the application
/// `app_id` (`app_id_length` bytes; NULL for none: no folders of its own) run from the bundle in
/// the folder `bundle` (NULL for none), or NULL (see `oxyos_last_error`).
///
/// # Safety
/// `app_id` and `bundle` are NULL or point to their lengths' readable bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_new(
    app_id: *const c_char,
    app_id_length: usize,
    bundle: *const c_char,
    bundle_length: usize,
) -> *mut OxyOSFiles {
    // SAFETY: forwarded caller contract.
    let given = unsafe { (utf8(app_id, app_id_length), utf8(bundle, bundle_length)) };
    let (app_id, bundle) = match given {
        (Ok(app_id), Ok(bundle)) => (app_id, bundle),
        _ => {
            fail("the application's id or bundle is not UTF-8");
            return std::ptr::null_mut();
        }
    };
    let scope = Scope::for_user().and_then(|scope| match app_id {
        Some(id) => scope.for_application(id, bundle.map(Path::new)),
        None => Ok(scope),
    });
    match scope.and_then(Service::new) {
        Ok(service) => Box::into_raw(Box::new(OxyOSFiles { service })),
        Err(error) => {
            fail(error);
            std::ptr::null_mut()
        }
    }
}

/// Frees the service: requests not run yet are dropped, unanswered. NULL is ignored.
///
/// # Safety
/// `files` is NULL or a pointer from `oxyos_files_new` not yet freed; it is invalid after.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_free(files: *mut OxyOSFiles) {
    if !files.is_null() {
        // SAFETY: the pointer came from Box::into_raw in oxyos_files_new and is freed once.
        drop(unsafe { Box::from_raw(files) });
    }
}

/// The service's descriptor, readable while replies are queued (for poll/epoll).
///
/// # Safety
/// `files` is a live pointer from `oxyos_files_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_fd(files: *mut OxyOSFiles) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::files(files) } {
        Some(files) => files.service.as_fd().as_raw_fd(),
        None => fail("null files"),
    }
}

/// Answers a request (`json`, `length` bytes of UTF-8; `bytes`, `bytes_length` bytes beside it,
/// NULL for none) at once, on the calling thread: the reply, the caller's to free with
/// `oxyos_files_reply_free`; NULL for a request that is not UTF-8.
///
/// # Safety
/// `files` is a live pointer from `oxyos_files_new`; `json` and `bytes` point to their lengths'
/// readable bytes (`bytes` may be NULL).
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_call(
    files: *mut OxyOSFiles,
    json: *const c_char,
    length: usize,
    bytes: *const u8,
    bytes_length: usize,
) -> *mut OxyOSFilesReply {
    // SAFETY: forwarded caller contract.
    let Some(files) = (unsafe { self::files(files) }) else {
        fail("null files");
        return std::ptr::null_mut();
    };
    // SAFETY: forwarded caller contract.
    let (request, bytes) = unsafe { (utf8(json, length), slice(bytes, bytes_length)) };
    match request {
        Ok(Some(request)) => Box::into_raw(Box::new(files.service.call(request, bytes).into())),
        _ => {
            fail("the request is not UTF-8");
            std::ptr::null_mut()
        }
    }
}

/// Queues a request as `id` (not 0), with its bytes; its answer comes as a reply with that id
/// (`oxyos_files_take`). 0, or OXYOS_ERROR for a request that is not UTF-8 (nothing is queued).
///
/// # Safety
/// As `oxyos_files_call`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_request(
    files: *mut OxyOSFiles,
    id: u64,
    json: *const c_char,
    length: usize,
    bytes: *const u8,
    bytes_length: usize,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(files) = (unsafe { self::files(files) }) else {
        return fail("null files");
    };
    if id == 0 {
        return fail("a request's id is not 0");
    }
    // SAFETY: forwarded caller contract.
    let (request, bytes) = unsafe { (utf8(json, length), slice(bytes, bytes_length)) };
    match request {
        Ok(Some(request)) => {
            files.service.request(id, request, bytes.to_vec());
            0
        }
        _ => fail("the request is not UTF-8"),
    }
}

/// Takes the next reply (an answer, or an event: id 0) without blocking; NULL when there is
/// none. The caller frees it with `oxyos_files_reply_free`.
///
/// # Safety
/// `files` is a live pointer from `oxyos_files_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_take(files: *mut OxyOSFiles) -> *mut OxyOSFilesReply {
    // SAFETY: forwarded caller contract.
    let Some(files) = (unsafe { self::files(files) }) else {
        fail("null files");
        return std::ptr::null_mut();
    };
    match files.service.take() {
        Some(reply) => Box::into_raw(Box::new(reply.into())),
        None => std::ptr::null_mut(),
    }
}

/// Opens a file the scope allows for reading (`uri`, a `file://` URI of `length` bytes): a
/// request's body, as `Networking` sends a multipart form's file. Its descriptor, the caller's
/// to close, its size in `*size` (if not NULL); OXYOS_ERROR with the reason in `oxyos_last_error`.
///
/// # Safety
/// `files` is a live pointer from `oxyos_files_new`; `uri` points to `length` readable bytes;
/// `size` is NULL or writable.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_open_read(
    files: *mut OxyOSFiles,
    uri: *const c_char,
    length: usize,
    size: *mut u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(files) = (unsafe { self::files(files) }) else {
        return fail("null files");
    };
    // SAFETY: forwarded caller contract.
    let Ok(Some(uri)) = (unsafe { utf8(uri, length) }) else {
        return fail("the URI is not UTF-8");
    };
    let opened = files
        .service
        .open_read(uri)
        .and_then(|file| Ok((file.metadata()?.len(), file)));
    match opened {
        Ok((length, file)) => {
            if !size.is_null() {
                // SAFETY: a writable size (contract above).
                unsafe { *size = length };
            }
            std::os::fd::IntoRawFd::into_raw_fd(file)
        }
        Err(error) => fail(format!("{uri}: {error}")),
    }
}

/// Reborrows a reply.
///
/// # Safety
/// `reply` is NULL or a live pointer from `oxyos_files_call` or `oxyos_files_take`.
#[allow(unsafe_code)]
unsafe fn reply<'a>(reply: *const OxyOSFilesReply) -> Option<&'a OxyOSFilesReply> {
    // SAFETY: per the contract.
    unsafe { reply.as_ref() }
}

/// The request's id a reply answers; 0 for an event.
///
/// # Safety
/// `reply` is a live reply.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_reply_id(reply: *const OxyOSFilesReply) -> u64 {
    // SAFETY: forwarded caller contract.
    unsafe { self::reply(reply) }.map_or(0, |reply| reply.id)
}

/// A reply's JSON, NUL-terminated, its length (without the NUL) in `*length`; valid until the
/// reply is freed.
///
/// # Safety
/// `reply` is a live reply; `length` is NULL or writable.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_reply_json(
    reply: *const OxyOSFilesReply,
    length: *mut usize,
) -> *const c_char {
    // SAFETY: forwarded caller contract.
    let Some(reply) = (unsafe { self::reply(reply) }) else {
        return std::ptr::null();
    };
    if !length.is_null() {
        // SAFETY: a writable length (contract above).
        unsafe { *length = reply.json.len() - 1 };
    }
    reply.json.as_ptr().cast()
}

/// A reply's bytes (what was read), their length in `*length` (0: none); valid until the reply
/// is freed.
///
/// # Safety
/// `reply` is a live reply; `length` is NULL or writable.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_reply_bytes(
    reply: *const OxyOSFilesReply,
    length: *mut usize,
) -> *const u8 {
    // SAFETY: forwarded caller contract.
    let Some(reply) = (unsafe { self::reply(reply) }) else {
        return std::ptr::null();
    };
    if !length.is_null() {
        // SAFETY: a writable length (contract above).
        unsafe { *length = reply.bytes.len() };
    }
    reply.bytes.as_ptr()
}

/// Frees a reply. NULL is ignored.
///
/// # Safety
/// `reply` is NULL or a reply not yet freed; it is invalid after.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_reply_free(reply: *mut OxyOSFilesReply) {
    if !reply.is_null() {
        // SAFETY: the pointer came from Box::into_raw and is freed once.
        drop(unsafe { Box::from_raw(reply) });
    }
}
