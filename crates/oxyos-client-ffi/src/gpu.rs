//! GPU buffers in the C ABI (docs/adr/0031-gpu-buffers.mdx): the compositor's tables, DMA-BUFs
//! attached plane by plane, DRM syncobj timelines with their sync-file bridge, and frames
//! submitted with their acquire and release points. General: what the Bloom host and Astro's
//! Ozone platform (Chromium's GPU path) both call.

use super::{client, fail, rects, OxyOSClient, OxyOSRect};
use oxyos_client::SyncPoints;
use oxyos_protocol::{Format, Size};
use std::ffi::c_int;
use std::os::fd::{BorrowedFd, IntoRawFd};

/// A DRM format and modifier the compositor imports or scans out.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OxyOSGpuFormat {
    pub format: u32,
    pub modifier: u64,
}

/// One plane of a DMA-BUF: its descriptor (the caller keeps it), offset and stride in bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OxyOSDmabufPlane {
    pub fd: i32,
    pub offset: u32,
    pub stride: u32,
}

/// A frame's synchronization: the acquire point the compositor waits for before it reads the
/// buffer and the release point it signals once it no longer does; timeline 0 is none.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OxyOSSync {
    pub acquire_timeline: u32,
    pub acquire_point: u64,
    pub release_timeline: u32,
    pub release_point: u64,
}

#[allow(unsafe_code)]
fn copy_formats(formats: &[(Format, u64)], out: *mut OxyOSGpuFormat, capacity: usize) -> i64 {
    if capacity > 0 && out.is_null() {
        fail("null formats");
        return -1;
    }
    for (index, &(format, modifier)) in formats.iter().take(capacity).enumerate() {
        // SAFETY: the caller passes room for `capacity` formats at `out`; `index` < capacity.
        unsafe {
            out.add(index).write(OxyOSGpuFormat {
                format: format as u32,
                modifier,
            });
        }
    }
    formats.len() as i64
}

/// Whether the compositor takes GPU buffers now: 1, with `*device` its render node (a `dev_t`; 0
/// when its device has none, so only linear buffers can be shared), or 0 (shared memory only).
/// OXYOS_ERROR for a null client.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `device` is NULL or writable.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_gpu_device(
    client: *mut OxyOSClient,
    device: *mut u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    match client.gpu() {
        Some(table) => {
            // SAFETY: the caller passes NULL or a writable u64.
            if let Some(device) = unsafe { device.as_mut() } {
                *device = table.device;
            }
            1
        }
        None => 0,
    }
}

/// The formats and modifiers the compositor imports: up to `capacity` written at `out`; returns
/// how many there are (call again with more room if more), or -1.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `out` has room for `capacity`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_gpu_formats(
    client: *mut OxyOSClient,
    out: *mut OxyOSGpuFormat,
    capacity: usize,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    let formats = client
        .gpu()
        .map(|table| table.formats.clone())
        .unwrap_or_default();
    copy_formats(&formats, out, capacity)
}

/// The formats and modifiers `surface`'s buffers would be scanned out in while it is all its
/// output shows (none otherwise), as `oxyos_client_gpu_formats` gives them.
///
/// # Safety
/// As `oxyos_client_gpu_formats`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_gpu_scanout_formats(
    client: *mut OxyOSClient,
    surface: u32,
    out: *mut OxyOSGpuFormat,
    capacity: usize,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    copy_formats(client.scanout_formats(surface), out, capacity)
}

/// Makes a DMA-BUF the caller owns a buffer of the connection: `count` planes (1 to 4) at
/// `planes`, in `format` (OXYOS_FORMAT_*, a DRM fourcc) with `modifier`, of `width` × `height`
/// pixels. Checked as the compositor checks it first (DMA-BUFs, in its table, linear planes
/// within their buffers). Returns its id (above 0), or -1. The caller keeps the descriptors and
/// submits it as a shared buffer (`oxyos_client_submit_buffer`, `_synced`); BUFFER_FAILED names
/// it if the compositor could not import it.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `planes` points to `count` readable
/// planes whose descriptors are open for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_attach_dmabuf(
    client: *mut OxyOSClient,
    format: u32,
    modifier: u64,
    width: u32,
    height: u32,
    planes: *const OxyOSDmabufPlane,
    count: usize,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    let Some(format) = Format::from_raw(format) else {
        fail("unknown pixel format");
        return -1;
    };
    if planes.is_null() || count == 0 || count > oxyos_client::MAX_PLANES {
        fail("a DMA-BUF has 1 to 4 planes");
        return -1;
    }
    // SAFETY: the caller passes `count` readable planes.
    let planes = unsafe { std::slice::from_raw_parts(planes, count) };
    if planes.iter().any(|plane| plane.fd < 0) {
        fail("no descriptor");
        return -1;
    }
    let planes: Vec<(BorrowedFd<'_>, u32, u32)> = planes
        .iter()
        .map(|plane| {
            (
                // SAFETY: the caller guarantees the descriptors are open for the call.
                unsafe { BorrowedFd::borrow_raw(plane.fd) },
                plane.offset,
                plane.stride,
            )
        })
        .collect();
    match client.attach_dmabuf(format, modifier, Size::new(width, height), &planes) {
        Ok(buffer) => i64::try_from(buffer).unwrap_or(-1),
        Err(error) => {
            fail(error);
            -1
        }
    }
}

fn timeline_id(result: std::io::Result<u32>) -> i64 {
    match result {
        Ok(id) => i64::from(id),
        Err(error) => {
            fail(error);
            -1
        }
    }
}

fn status(result: std::io::Result<()>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

fn descriptor(result: std::io::Result<std::os::fd::OwnedFd>) -> c_int {
    match result {
        Ok(fd) => fd.into_raw_fd(),
        Err(error) => {
            fail(error);
            -1
        }
    }
}

/// A new timeline of the connection (a DRM syncobj on the compositor's render node), imported by
/// the compositor. Returns its id (above 0), or -1 (no render node with timelines).
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_create_timeline(client: *mut OxyOSClient) -> i64 {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    timeline_id(client.create_timeline())
}

/// The caller's own syncobj timeline (`fd`, from DRM_IOCTL_SYNCOBJ_HANDLE_TO_FD or a Vulkan
/// timeline semaphore's OPAQUE_FD export) as a timeline of the connection. The caller keeps `fd`.
/// Returns its id, or -1.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `fd` is open for the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_import_timeline(client: *mut OxyOSClient, fd: c_int) -> i64 {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    if fd < 0 {
        fail("no descriptor");
        return -1;
    }
    // SAFETY: the caller guarantees `fd` is open for the call.
    timeline_id(client.import_timeline(unsafe { BorrowedFd::borrow_raw(fd) }))
}

/// Ends a timeline; the compositor forgets it too. 0, or OXYOS_ERROR.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_destroy_timeline(
    client: *mut OxyOSClient,
    timeline: u32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    status(client.destroy_timeline(timeline))
}

/// `point` of `timeline` signals when the sync file `fence` does (an EGL native fence, a Vulkan
/// binary semaphore's SYNC_FD). The caller keeps `fence`. 0, or OXYOS_ERROR.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `fence` is open for the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_timeline_import_sync_file(
    client: *mut OxyOSClient,
    timeline: u32,
    point: u64,
    fence: c_int,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    if fence < 0 {
        return fail("no descriptor");
    }
    // SAFETY: the caller guarantees `fence` is open for the call.
    let fence = unsafe { BorrowedFd::borrow_raw(fence) };
    status(client.timeline_import_sync_file(timeline, point, fence))
}

/// The fence of `point` of `timeline` as a new sync file the caller owns, or -1 when the point
/// has none yet (a release point has one once RELEASED names its buffer).
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_timeline_export_sync_file(
    client: *mut OxyOSClient,
    timeline: u32,
    point: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    descriptor(client.timeline_export_sync_file(timeline, point))
}

/// Signals `point` of `timeline` from the CPU. 0, or OXYOS_ERROR.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_timeline_signal(
    client: *mut OxyOSClient,
    timeline: u32,
    point: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    status(client.timeline_signal(timeline, point))
}

/// A new descriptor of `timeline`'s syncobj the caller owns, for a Vulkan timeline semaphore
/// (OPAQUE_FD) or another process; -1 on failure.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_timeline_export(
    client: *mut OxyOSClient,
    timeline: u32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    descriptor(client.timeline_export(timeline))
}

/// A new eventfd the caller owns (non-blocking), readable once `point` of `timeline` signals, or,
/// with `available`, once it has a fence; -1 on failure.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_timeline_eventfd(
    client: *mut OxyOSClient,
    timeline: u32,
    point: u64,
    available: bool,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    descriptor(client.timeline_eventfd(timeline, point, available))
}

/// `oxyos_client_submit_buffer` with the frame's synchronization (`sync`, NULL for none): the
/// compositor reads the buffer once the acquire point signals and signals the release point once
/// it no longer does. Refused for a timeline the connection does not have, or a release point not
/// after the acquire point on one timeline. Returns the sequence number, or -1.
///
/// # Safety
/// As `oxyos_client_submit_buffer`; `sync` is NULL or readable.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_submit_buffer_synced(
    client: *mut OxyOSClient,
    surface: u32,
    buffer: u64,
    configure: u64,
    damage: *const OxyOSRect,
    count: usize,
    sync: *const OxyOSSync,
) -> i64 {
    if count > 0 && damage.is_null() {
        fail("null damage");
        return -1;
    }
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    // SAFETY: forwarded caller contract.
    let damage = unsafe { rects(damage, count) };
    let damage = (!damage.is_empty()).then_some(damage.as_slice());
    // SAFETY: the caller passes NULL or a readable OxyOSSync.
    let sync = unsafe { sync.as_ref() }.map(|sync| SyncPoints {
        acquire: (sync.acquire_timeline, sync.acquire_point),
        release: (sync.release_timeline, sync.release_point),
    });
    match client.submit_buffer_synced(surface, buffer, configure, damage, sync) {
        Ok(sequence) => i64::try_from(sequence).unwrap_or(-1),
        Err(error) => {
            fail(error);
            -1
        }
    }
}
