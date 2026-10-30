//! C ABI over `oxyos-files` (docs/adr/0027-files.mdx): the file service the Bloom host gives an
//! application as `OxyOSFiles`. Requests and answers are JSON (`oxyos_files::service`), so the
//! ABI is four calls however many operations the service has.

use super::fail;
use oxyos_files::{Scope, Service};
use rustix::fd::{AsFd, AsRawFd};
use std::ffi::{c_char, c_int};

/// The service, behind the C ABI.
pub struct OxyOSFiles {
    service: Service,
}

/// Reborrows a caller's pointer.
///
/// # Safety
/// `files` is NULL or a live pointer from `oxyos_files_new`, used by one thread at a time.
#[allow(unsafe_code)]
unsafe fn files<'a>(files: *mut OxyOSFiles) -> Option<&'a mut OxyOSFiles> {
    // SAFETY: per the contract, a non-null pointer is a live, exclusively used OxyOSFiles.
    unsafe { files.as_mut() }
}

/// The service for the user running the process (`Scope::for_user`), or NULL (see
/// `oxyos_last_error`).
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_files_new() -> *mut OxyOSFiles {
    match Scope::for_user().and_then(Service::new) {
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

/// The service's descriptor, readable while answers are queued (for poll/epoll).
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

/// Queues a request (`json`, `length` bytes of UTF-8, not NUL-terminated); its answer follows
/// through `oxyos_files_answer`. 0, or OXYOS_ERROR for text that is not UTF-8 (nothing is queued).
///
/// # Safety
/// `files` is a live pointer from `oxyos_files_new`; `json` points to `length` readable bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_request(
    files: *mut OxyOSFiles,
    json: *const c_char,
    length: usize,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(files) = (unsafe { self::files(files) }) else {
        return fail("null files");
    };
    if json.is_null() {
        return fail("null request");
    }
    // SAFETY: `json` points to `length` readable bytes (contract above).
    let bytes = unsafe { std::slice::from_raw_parts(json.cast::<u8>(), length) };
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            files.service.request(text);
            0
        }
        Err(_) => fail("the request is not UTF-8"),
    }
}

/// Takes the next answer or change without blocking, as JSON written into `out` (`capacity`
/// bytes, NUL-terminated). Returns its length; 0 when there is none. A length of `capacity` or
/// more did not fit: nothing was taken, and the next call with that much more room takes it.
///
/// # Safety
/// `files` is a live pointer from `oxyos_files_new`; `out` points to `capacity` writable bytes,
/// or is NULL with `capacity` 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_files_answer(
    files: *mut OxyOSFiles,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let Some(files) = (unsafe { self::files(files) }) else {
        return i64::from(fail("null files"));
    };
    if out.is_null() && capacity != 0 {
        return i64::from(fail("null buffer"));
    }
    let Some(answer) = files.service.take() else {
        return 0;
    };
    let length = answer.len();
    if length >= capacity {
        files.service.untake(answer);
    } else {
        // SAFETY: `out` has `capacity` bytes, more than the answer and its NUL.
        unsafe {
            std::ptr::copy_nonoverlapping(answer.as_ptr(), out.cast::<u8>(), length);
            *out.add(length) = 0;
        }
    }
    i64::try_from(length).unwrap_or(i64::MAX)
}
