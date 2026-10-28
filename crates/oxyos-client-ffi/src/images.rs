//! C ABI over `oxyos-image`, the decoding behind React Native's `Image` in the Bloom host. It lives
//! in this library so that the host links one Rust library, not one per crate.
//!
//! Paths cross as a pointer and a length: a Linux path is bytes, not necessarily UTF-8.

use super::fail;
use oxyos_image::{Fit, Loader};
use rustix::fd::{AsFd, AsRawFd};
use std::ffi::{c_char, c_int, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

/// How `oxyos_images_load` fits an image to its box.
pub const OXYOS_IMAGE_FIT_WITHIN: u32 = 0;
pub const OXYOS_IMAGE_FIT_COVERING: u32 = 1;

/// Results of `oxyos_images_answer`.
pub const OXYOS_IMAGE_ANSWER: c_int = 1;
pub const OXYOS_IMAGE_EMPTY: c_int = 0;

/// A load's answer, as `oxyos_images_answer` gives it. `pixels` is NULL when it failed, and
/// `error` NULL when it did not. Its pointers stay valid until the next `oxyos_images_answer` or
/// `oxyos_images_free`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSImageAnswer {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    /// The image's own size, before it was reduced.
    pub natural_width: u32,
    pub natural_height: u32,
    /// `width` × `height` premultiplied RGBA pixels, rows top to bottom, sRGB.
    pub pixels: *const u8,
    pub length: usize,
    pub error: *const c_char,
    pub error_length: usize,
}

impl Default for OxyOSImageAnswer {
    fn default() -> Self {
        Self {
            id: 0,
            width: 0,
            height: 0,
            natural_width: 0,
            natural_height: 0,
            pixels: std::ptr::null(),
            length: 0,
            error: std::ptr::null(),
            error_length: 0,
        }
    }
}

/// The loader, owned by the caller between `oxyos_images_new` and `oxyos_images_free`; used by one
/// thread at a time.
pub struct OxyOSImages {
    loader: Loader,
    /// The answer last given, which its `OxyOSImageAnswer`'s pointers point into.
    given: Option<(Vec<u8>, String)>,
}

/// A caller's path.
///
/// # Safety
/// `path` points to `length` readable bytes.
#[allow(unsafe_code)]
unsafe fn path(path: *const c_char, length: usize) -> Option<PathBuf> {
    if path.is_null() || length == 0 {
        return None;
    }
    // SAFETY: per the contract, `path` points to `length` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(path.cast::<u8>(), length) };
    Some(PathBuf::from(OsStr::from_bytes(bytes)))
}

/// Reborrows a caller's loader pointer.
///
/// # Safety
/// `images` is NULL or a live pointer from `oxyos_images_new`, used by one thread at a time.
#[allow(unsafe_code)]
unsafe fn images<'a>(images: *mut OxyOSImages) -> Option<&'a mut OxyOSImages> {
    // SAFETY: per the contract, a non-null pointer is a live, exclusively used OxyOSImages.
    unsafe { images.as_mut() }
}

/// The size in pixels of the image at `path`, from its headers alone. 0 on success.
///
/// # Safety
/// `path` points to `path_length` readable bytes; `width` and `height` are writable.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_image_size(
    path: *const c_char,
    path_length: usize,
    width: *mut u32,
    height: *mut u32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(path) = (unsafe { self::path(path, path_length) }) else {
        return fail("null path");
    };
    // SAFETY: the caller passes writable sizes or NULL.
    let (Some(width), Some(height)) = (unsafe { width.as_mut() }, unsafe { height.as_mut() })
    else {
        return fail("null size");
    };
    match std::fs::read(&path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| oxyos_image::size(&bytes).map_err(|error| error.to_string()))
    {
        Ok((w, h)) => {
            (*width, *height) = (w, h);
            0
        }
        Err(error) => fail(error),
    }
}

/// A new loader. Returns NULL on failure (see `oxyos_last_error`).
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_images_new() -> *mut OxyOSImages {
    match Loader::new() {
        Ok(loader) => Box::into_raw(Box::new(OxyOSImages {
            loader,
            given: None,
        })),
        Err(error) => {
            fail(error);
            std::ptr::null_mut()
        }
    }
}

/// Frees the loader; loads still running end unheard. NULL is ignored.
///
/// # Safety
/// `images` is NULL or a pointer from `oxyos_images_new` not yet freed; it is invalid after.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_images_free(images: *mut OxyOSImages) {
    if !images.is_null() {
        // SAFETY: the pointer came from Box::into_raw in oxyos_images_new and is freed once.
        drop(unsafe { Box::from_raw(images) });
    }
}

/// The loader's descriptor, readable while answers are queued (for poll/epoll).
///
/// # Safety
/// `images` is a live pointer from `oxyos_images_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_images_fd(images: *mut OxyOSImages) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::images(images) } {
        Some(images) => images.loader.as_fd().as_raw_fd(),
        None => fail("null loader"),
    }
}

/// Starts decoding the image at `path`, reduced as far as `fit` allows for a box of `width` ×
/// `height` pixels (0 for either keeps its size); never enlarged. Its answer follows through
/// `oxyos_images_answer`. 0 on success.
///
/// # Safety
/// `images` is a live pointer from `oxyos_images_new`; `path` points to `path_length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_images_load(
    images: *mut OxyOSImages,
    id: u64,
    path: *const c_char,
    path_length: usize,
    width: u32,
    height: u32,
    fit: u32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(images) = (unsafe { self::images(images) }) else {
        return fail("null loader");
    };
    // SAFETY: forwarded caller contract.
    let Some(path) = (unsafe { self::path(path, path_length) }) else {
        return fail("null path");
    };
    let fit = match fit {
        _ if width == 0 || height == 0 => Fit::Whole,
        OXYOS_IMAGE_FIT_WITHIN => Fit::Within(width, height),
        OXYOS_IMAGE_FIT_COVERING => Fit::Covering(width, height),
        other => return fail(format!("unknown fit {other}")),
    };
    images.loader.load(id, path, fit);
    0
}

/// Takes the next answer without blocking: `OXYOS_IMAGE_ANSWER` with `*out` filled,
/// `OXYOS_IMAGE_EMPTY`, or `OXYOS_ERROR`.
///
/// # Safety
/// `images` is a live pointer from `oxyos_images_new`; `out` points to a writable `OxyOSImageAnswer`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_images_answer(
    images: *mut OxyOSImages,
    out: *mut OxyOSImageAnswer,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(images) = (unsafe { self::images(images) }) else {
        return fail("null loader");
    };
    // SAFETY: the caller passes a writable OxyOSImageAnswer or NULL.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return fail("null answer");
    };
    images.given = None;
    let Some(loaded) = images.loader.answer() else {
        return OXYOS_IMAGE_EMPTY;
    };
    let mut answer = OxyOSImageAnswer {
        id: loaded.id,
        ..OxyOSImageAnswer::default()
    };
    let given = match loaded.result {
        Ok(image) => {
            (answer.width, answer.height) = (image.width, image.height);
            (answer.natural_width, answer.natural_height) = image.natural;
            (image.pixels, String::new())
        }
        Err(error) => (Vec::new(), error.to_string()),
    };
    // The pointers point into heap memory the vector and string own, which moving them into
    // `given` does not move.
    if given.1.is_empty() {
        (answer.pixels, answer.length) = (given.0.as_ptr(), given.0.len());
    } else {
        (answer.error, answer.error_length) = (given.1.as_ptr().cast(), given.1.len());
    }
    images.given = Some(given);
    *out = answer;
    OXYOS_IMAGE_ANSWER
}
