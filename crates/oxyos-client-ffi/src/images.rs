//! C ABI over `oxyos-image`, the decoding behind React Native's `Image` in the Bloom host. It lives
//! in this library so that the host links one Rust library, not one per crate.
//!
//! Paths cross as a pointer and a length: a Linux path is bytes, not necessarily UTF-8.
//!
//! Remote images (`oxyos_images_fetch`) are fetched with the application's network, the one
//! `oxyos_net_new` made for its `fetch`, given with `oxyos_images_connect` (crates/oxyos-image's
//! `remote`). Files, `data:` URIs, BlurHashes and ThumbHashes are opened with `oxyos_images_open`
//! (expo-image's sources besides remote ones).

use super::fail;
use super::net::{string, OxyOSNet, OxyOSNetHeader};
use oxyos_image::remote::{Fetch, Want};
use oxyos_image::{Fit, Loader, Open, Origin, Outcome};
use oxyos_net::cache::{Cache, Policy};
use rustix::fd::{AsFd, AsRawFd};
use std::ffi::{c_char, c_int, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;

/// How `oxyos_images_load` fits an image to its box.
pub const OXYOS_IMAGE_FIT_WITHIN: u32 = 0;
pub const OXYOS_IMAGE_FIT_COVERING: u32 = 1;

/// What an answer is (`OxyOSImageAnswer.kind`).
pub const OXYOS_IMAGE_PICTURE: u32 = 0;
pub const OXYOS_IMAGE_FAILED: u32 = 1;
pub const OXYOS_IMAGE_PROGRESS: u32 = 2;
pub const OXYOS_IMAGE_UNDECODED: u32 = 3;
pub const OXYOS_IMAGE_SIZE: u32 = 4;
pub const OXYOS_IMAGE_FETCHED: u32 = 5;

/// A remote image's `cache` policy (`OxyOSImageFetch.cache`), as React Native's source's.
pub const OXYOS_IMAGE_CACHE_DEFAULT: u32 = 0;
pub const OXYOS_IMAGE_CACHE_RELOAD: u32 = 1;
pub const OXYOS_IMAGE_CACHE_FORCE: u32 = 2;
pub const OXYOS_IMAGE_CACHE_ONLY_IF_CACHED: u32 = 3;

/// What a remote image's load wants (`OxyOSImageFetch.want`).
pub const OXYOS_IMAGE_WANT_PICTURE: u32 = 0;
pub const OXYOS_IMAGE_WANT_SIZE: u32 = 1;
pub const OXYOS_IMAGE_WANT_PREFETCH: u32 = 2;

/// Where a picture's bytes came from (`OxyOSImageAnswer.from`), as expo-image's `cacheType`.
pub const OXYOS_IMAGE_FROM_SOURCE: u32 = 0;
pub const OXYOS_IMAGE_FROM_DISK: u32 = 1;
pub const OXYOS_IMAGE_FROM_MEMORY: u32 = 2;

/// What `oxyos_images_open` decodes (`OxyOSImageOpen.kind`).
pub const OXYOS_IMAGE_OPEN_FILE: u32 = 0;
pub const OXYOS_IMAGE_OPEN_DATA: u32 = 1;
pub const OXYOS_IMAGE_OPEN_BLURHASH: u32 = 2;
pub const OXYOS_IMAGE_OPEN_THUMBHASH: u32 = 3;

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
    /// The image's own size, before it was reduced (`OXYOS_IMAGE_SIZE`'s size).
    pub natural_width: u32,
    pub natural_height: u32,
    /// `width` × `height` premultiplied RGBA pixels, rows top to bottom, sRGB.
    pub pixels: *const u8,
    pub length: usize,
    pub error: *const c_char,
    pub error_length: usize,
    /// `OXYOS_IMAGE_PICTURE` (pixels), `OXYOS_IMAGE_FAILED` (error), `OXYOS_IMAGE_PROGRESS`
    /// (loaded of total bytes), `OXYOS_IMAGE_UNDECODED` (a remote image's or a `data:` URI's
    /// bytes it does not decode, `data`, with their `content_type` and the `error` decoding gave),
    /// `OXYOS_IMAGE_SIZE` or `OXYOS_IMAGE_FETCHED`. Every kind but progress ends its load.
    pub kind: u32,
    pub loaded: u64,
    pub total: u64,
    pub data: *const u8,
    pub data_length: usize,
    pub content_type: *const c_char,
    pub content_type_length: usize,
    /// Where a picture's or undecoded bytes came from: `OXYOS_IMAGE_FROM_*`.
    pub from: u32,
    /// A picture's frames, each `width` × `height`, one after the other in `pixels`: 1 for a
    /// still image (0 for an answer that is not a picture).
    pub frames: u32,
    /// Each frame's duration in milliseconds, `frames` of them (a still image's a single 0).
    pub delays: *const u32,
    /// How many times the frames play, 0 for forever (a still image's 1).
    pub loops: u32,
}

/// A remote image's load, as `oxyos_images_fetch` takes it: its URL, the headers sent with it,
/// its `cache` policy, whether the memory and disk caches are used, and what it wants (its
/// picture fitted to `width` × `height` by `fit`, as `oxyos_images_load`; its size; it fetched).
/// `key` (NULL or empty: the URL) is what both caches hold it by; with `animated`, an animated GIF
/// or WebP is decoded frame by frame.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSImageFetch {
    pub url: *const c_char,
    pub url_length: usize,
    pub headers: *const OxyOSNetHeader,
    pub header_count: usize,
    pub cache: u32,
    pub memory: u8,
    pub disk: u8,
    pub want: u32,
    pub width: u32,
    pub height: u32,
    pub fit: u32,
    pub key: *const c_char,
    pub key_length: usize,
    pub animated: u8,
}

/// What `oxyos_images_open` decodes: an `OXYOS_IMAGE_OPEN_*` kind and its text (a path, a `data:`
/// URI, a BlurHash, a ThumbHash in base64), the box it is fitted to (a BlurHash's own size), and
/// whether an animated image is decoded frame by frame.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSImageOpen {
    pub kind: u32,
    pub text: *const c_char,
    pub text_length: usize,
    pub width: u32,
    pub height: u32,
    pub fit: u32,
    pub animated: u8,
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
            kind: OXYOS_IMAGE_PICTURE,
            loaded: 0,
            total: 0,
            data: std::ptr::null(),
            data_length: 0,
            content_type: std::ptr::null(),
            content_type_length: 0,
            from: OXYOS_IMAGE_FROM_SOURCE,
            frames: 0,
            delays: std::ptr::null(),
            loops: 0,
        }
    }
}

/// What the answer last given owns, which its `OxyOSImageAnswer`'s pointers point into.
#[derive(Default)]
struct Given {
    pixels: Vec<u8>,
    delays: Vec<u32>,
    error: String,
    data: Option<Arc<[u8]>>,
    content_type: String,
}

fn from(origin: Origin) -> u32 {
    match origin {
        Origin::Source => OXYOS_IMAGE_FROM_SOURCE,
        Origin::Disk => OXYOS_IMAGE_FROM_DISK,
        Origin::Memory => OXYOS_IMAGE_FROM_MEMORY,
    }
}

/// The loader, owned by the caller between `oxyos_images_new` and `oxyos_images_free`; used by one
/// thread at a time.
pub struct OxyOSImages {
    loader: Loader,
    /// The answer last given, which its `OxyOSImageAnswer`'s pointers point into.
    given: Option<Given>,
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
    let fit = match self::fit(width, height, fit) {
        Ok(fit) => fit,
        Err(error) => return fail(error),
    };
    images.loader.load(id, path, fit);
    0
}

fn fit(width: u32, height: u32, fit: u32) -> Result<Fit, String> {
    Ok(match fit {
        _ if width == 0 || height == 0 => Fit::Whole,
        OXYOS_IMAGE_FIT_WITHIN => Fit::Within(width, height),
        OXYOS_IMAGE_FIT_COVERING => Fit::Covering(width, height),
        other => return Err(format!("unknown fit {other}")),
    })
}

/// Fetches remote images from now on with `net`'s network (shared: its cookies, TLS and proxy),
/// through a disk cache of at most `cache_limit` bytes in the directory `cache` (NULL: memory
/// only). 0 on success; the disk cache failing to open fails it, connecting nothing.
///
/// # Safety
/// `images` and `net` are live pointers from `oxyos_images_new` and `oxyos_net_new`; `cache` is
/// NULL or points to `cache_length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_images_connect(
    images: *mut OxyOSImages,
    net: *mut OxyOSNet,
    cache: *const c_char,
    cache_length: usize,
    cache_limit: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(images) = (unsafe { self::images(images) }) else {
        return fail("null loader");
    };
    // SAFETY: forwarded caller contract.
    let Some(net) = (unsafe { super::net::net(net) }) else {
        return fail("null network");
    };
    // SAFETY: forwarded caller contract.
    let cache = match unsafe { self::path(cache, cache_length) } {
        Some(directory) => match Cache::open(&directory, cache_limit) {
            Ok(cache) => Some(Arc::new(cache)),
            Err(error) => {
                return fail(format!(
                    "the image cache at {} cannot open: {error}",
                    directory.display()
                ))
            }
        },
        None => None,
    };
    images.loader.connect(net.network.clone(), cache);
    0
}

/// Starts a remote image's load; its answers, with `id`, follow (progress, then the one that
/// ends it). 0 on success.
///
/// # Safety
/// `images` is a live pointer from `oxyos_images_new`; `fetch` points to an `OxyOSImageFetch`
/// whose pointers are valid for their lengths.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_images_fetch(
    images: *mut OxyOSImages,
    id: u64,
    fetch: *const OxyOSImageFetch,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(images) = (unsafe { self::images(images) }) else {
        return fail("null loader");
    };
    // SAFETY: the caller passes a readable OxyOSImageFetch or NULL.
    let Some(fetch) = (unsafe { fetch.as_ref() }) else {
        return fail("null fetch");
    };
    // SAFETY: forwarded caller contract.
    let url = match unsafe { string(fetch.url, fetch.url_length, "url") } {
        Ok(url) => url,
        Err(error) => return fail(error),
    };
    let mut headers = Vec::new();
    if fetch.header_count > 0 {
        if fetch.headers.is_null() {
            return fail("null headers");
        }
        // SAFETY: per the contract, `headers` points to `header_count` headers.
        let given = unsafe { std::slice::from_raw_parts(fetch.headers, fetch.header_count) };
        for header in given {
            // SAFETY: forwarded caller contract.
            let name = unsafe { string(header.name, header.name_length, "header name") };
            // SAFETY: forwarded caller contract.
            let value = unsafe { string(header.value, header.value_length, "header value") };
            match (name, value) {
                (Ok(name), Ok(value)) => headers.push((name, value)),
                (Err(error), _) | (_, Err(error)) => return fail(error),
            }
        }
    }
    let policy = match fetch.cache {
        OXYOS_IMAGE_CACHE_DEFAULT => Policy::Default,
        OXYOS_IMAGE_CACHE_RELOAD => Policy::Reload,
        OXYOS_IMAGE_CACHE_FORCE => Policy::ForceCache,
        OXYOS_IMAGE_CACHE_ONLY_IF_CACHED => Policy::OnlyIfCached,
        other => return fail(format!("unknown cache policy {other}")),
    };
    let want = match fetch.want {
        OXYOS_IMAGE_WANT_PICTURE => match self::fit(fetch.width, fetch.height, fetch.fit) {
            Ok(fit) => Want::Picture(fit),
            Err(error) => return fail(error),
        },
        OXYOS_IMAGE_WANT_SIZE => Want::Size,
        OXYOS_IMAGE_WANT_PREFETCH => Want::Prefetch,
        other => return fail(format!("unknown want {other}")),
    };
    let key = if fetch.key.is_null() || fetch.key_length == 0 {
        None
    } else {
        // SAFETY: forwarded caller contract.
        match unsafe { string(fetch.key, fetch.key_length, "cache key") } {
            Ok(key) => Some(key),
            Err(error) => return fail(error),
        }
    };
    images.loader.fetch(
        id,
        Fetch {
            url,
            headers,
            policy,
            memory: fetch.memory != 0,
            disk: fetch.disk != 0,
            want,
            key,
            animated: fetch.animated != 0,
        },
    );
    0
}

/// Starts decoding what `open` names (a file, a `data:` URI, a BlurHash, a ThumbHash), off the
/// caller's thread; its answer, with `id`, follows (from `OXYOS_IMAGE_FROM_SOURCE`). 0 on
/// success; fails at once for an unknown kind or fit, an empty text (or one not UTF-8 but a
/// path), or a BlurHash without a size. A hash or URI that cannot be decoded is answered
/// `OXYOS_IMAGE_FAILED`, with expo-image's message.
///
/// # Safety
/// `images` is a live pointer from `oxyos_images_new`; `open` points to an `OxyOSImageOpen` whose
/// `text` is valid for `text_length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_images_open(
    images: *mut OxyOSImages,
    id: u64,
    open: *const OxyOSImageOpen,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(images) = (unsafe { self::images(images) }) else {
        return fail("null loader");
    };
    // SAFETY: the caller passes a readable OxyOSImageOpen or NULL.
    let Some(open) = (unsafe { open.as_ref() }) else {
        return fail("null open");
    };
    if open.text.is_null() || open.text_length == 0 {
        return fail("empty text");
    }
    let fit = match self::fit(open.width, open.height, open.fit) {
        Ok(fit) => fit,
        Err(error) => return fail(error),
    };
    // SAFETY: forwarded caller contract.
    let text = || unsafe { string(open.text, open.text_length, "text") };
    let opened = match open.kind {
        // SAFETY: forwarded caller contract.
        OXYOS_IMAGE_OPEN_FILE => match unsafe { path(open.text, open.text_length) } {
            Some(path) => Ok(Open::File(path)),
            None => Err("null path".to_string()),
        },
        OXYOS_IMAGE_OPEN_DATA => text().map(Open::Data),
        OXYOS_IMAGE_OPEN_BLURHASH if open.width == 0 || open.height == 0 => {
            Err("a BlurHash is decoded at a width and height, not 0".to_string())
        }
        OXYOS_IMAGE_OPEN_BLURHASH => text().map(|hash| Open::Blurhash {
            hash,
            width: open.width,
            height: open.height,
        }),
        OXYOS_IMAGE_OPEN_THUMBHASH => text().map(Open::Thumbhash),
        other => Err(format!("unknown kind {other}")),
    };
    match opened {
        Ok(opened) => {
            images.loader.open(id, opened, fit, open.animated != 0);
            0
        }
        Err(error) => fail(error),
    }
}

/// Answers nothing more for the load `id`; a remote image's request stops once no load waits for
/// it. 0 on success.
///
/// # Safety
/// `images` is a live pointer from `oxyos_images_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_images_cancel(images: *mut OxyOSImages, id: u64) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(images) = (unsafe { self::images(images) }) else {
        return fail("null loader");
    };
    images.loader.cancel(id);
    0
}

/// Where a remote image is held: `OXYOS_IMAGE_IN_MEMORY` and `OXYOS_IMAGE_ON_DISK`, or 0 for
/// neither (`Image.queryCache`).
///
/// # Safety
/// `images` is a live pointer from `oxyos_images_new`; `url` points to `url_length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_images_cached(
    images: *mut OxyOSImages,
    url: *const c_char,
    url_length: usize,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(images) = (unsafe { self::images(images) }) else {
        return fail("null loader");
    };
    // SAFETY: forwarded caller contract.
    let url = match unsafe { string(url, url_length, "url") } {
        Ok(url) => url,
        Err(error) => return fail(error),
    };
    let (memory, disk) = images.loader.cached(&url);
    (if memory { OXYOS_IMAGE_IN_MEMORY } else { 0 }) | (if disk { OXYOS_IMAGE_ON_DISK } else { 0 })
}

/// Where `oxyos_images_cached` finds a remote image, and the caches `oxyos_images_clear` empties.
pub const OXYOS_IMAGE_IN_MEMORY: c_int = 1;
pub const OXYOS_IMAGE_ON_DISK: c_int = 2;

/// Empties the caches named (`OXYOS_IMAGE_IN_MEMORY`, `OXYOS_IMAGE_ON_DISK`): expo-image's
/// `clearMemoryCache` and `clearDiskCache`. Loads on their way are not affected. 0 on success.
///
/// # Safety
/// `images` is a live pointer from `oxyos_images_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_images_clear(images: *mut OxyOSImages, caches: u32) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(images) = (unsafe { self::images(images) }) else {
        return fail("null loader");
    };
    let known = (OXYOS_IMAGE_IN_MEMORY | OXYOS_IMAGE_ON_DISK) as u32;
    if caches & !known != 0 {
        return fail(format!("unknown caches {caches:#x}"));
    }
    images.loader.clear(
        caches & OXYOS_IMAGE_IN_MEMORY as u32 != 0,
        caches & OXYOS_IMAGE_ON_DISK as u32 != 0,
    );
    0
}

/// The file in the disk cache holding a remote image's bytes exactly as they were fetched, by its
/// URL or cache key (expo-image's `getCachePathAsync`): its path written into `out` (at most
/// `capacity` bytes, not terminated), its length in `*length`. 0 when it is held; 1 when it is not,
/// or there is no disk cache; `OXYOS_ERROR` when `out` is too small (`*length` the length it
/// needs). The file stays while the entry does.
///
/// # Safety
/// `images` is a live pointer from `oxyos_images_new`; `key` points to `key_length` bytes; `out`
/// is writable for `capacity` bytes (or NULL with 0); `length` is writable.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_images_cache_path(
    images: *mut OxyOSImages,
    key: *const c_char,
    key_length: usize,
    out: *mut c_char,
    capacity: usize,
    length: *mut usize,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(images) = (unsafe { self::images(images) }) else {
        return fail("null loader");
    };
    // SAFETY: forwarded caller contract.
    let key = match unsafe { string(key, key_length, "key") } {
        Ok(key) => key,
        Err(error) => return fail(error),
    };
    // SAFETY: the caller passes a writable length or NULL.
    let Some(length) = (unsafe { length.as_mut() }) else {
        return fail("null length");
    };
    let Some(path) = images.loader.cache_path(&key) else {
        *length = 0;
        return 1;
    };
    let bytes = path.as_os_str().as_bytes();
    *length = bytes.len();
    if bytes.len() > capacity || out.is_null() {
        return fail(format!(
            "the path takes {} bytes, more than {capacity}",
            bytes.len()
        ));
    }
    // SAFETY: `out` is writable for `capacity` bytes, at least the path's.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast::<u8>(), bytes.len()) };
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
    let mut given = Given::default();
    match loaded.outcome {
        Outcome::Image(image, origin) => {
            (answer.width, answer.height) = (image.width, image.height);
            (answer.natural_width, answer.natural_height) = image.natural;
            answer.from = from(origin);
            answer.frames = image.frames() as u32;
            answer.loops = image.loops;
            given.pixels = image.pixels;
            given.delays = image.delays;
        }
        Outcome::Failed(error) => {
            answer.kind = OXYOS_IMAGE_FAILED;
            given.error = error.to_string();
        }
        Outcome::Progress { loaded, total } => {
            answer.kind = OXYOS_IMAGE_PROGRESS;
            (answer.loaded, answer.total) = (loaded, total);
        }
        Outcome::Undecoded {
            bytes,
            content_type,
            from: origin,
            error,
        } => {
            answer.kind = OXYOS_IMAGE_UNDECODED;
            answer.from = from(origin);
            given.error = error.to_string();
            given.data = Some(bytes);
            given.content_type = content_type.unwrap_or_default();
        }
        Outcome::Size(width, height) => {
            answer.kind = OXYOS_IMAGE_SIZE;
            (answer.natural_width, answer.natural_height) = (width, height);
        }
        Outcome::Fetched => answer.kind = OXYOS_IMAGE_FETCHED,
    }
    // The pointers point into heap memory the vectors, strings and slice own, which moving them
    // into `given` does not move.
    if answer.kind == OXYOS_IMAGE_PICTURE {
        (answer.pixels, answer.length) = (given.pixels.as_ptr(), given.pixels.len());
        answer.delays = given.delays.as_ptr();
    }
    if !given.error.is_empty() {
        (answer.error, answer.error_length) = (given.error.as_ptr().cast(), given.error.len());
    }
    if let Some(data) = &given.data {
        (answer.data, answer.data_length) = (data.as_ptr(), data.len());
    }
    if !given.content_type.is_empty() {
        (answer.content_type, answer.content_type_length) =
            (given.content_type.as_ptr().cast(), given.content_type.len());
    }
    images.given = Some(given);
    *out = answer;
    OXYOS_IMAGE_ANSWER
}
