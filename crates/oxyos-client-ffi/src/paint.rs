//! C ABI over `oxyos-paint` (docs/adr/0031-gpu-buffers.mdx, decision 12): the Vulkan device a
//! native client's renderer (the Bloom host's Skia) draws on, and the frames it presents from the
//! renderer's image in the client's GPU buffers.

use super::{client as connection, fail, rects, OxyOSClient, OxyOSRect};
use oxyos_paint::vk::{self, Handle};
use oxyos_paint::{Paint, Source};
use oxyos_protocol::Format;
use std::ffi::{c_char, c_int, c_void, CString};

/// A painter, its device's name and its extensions' names as C arrays.
pub struct OxyOSPaint {
    paint: Paint,
    name: CString,
    instance_extensions: Vec<*const c_char>,
    device_extensions: Vec<*const c_char>,
}

/// The painter's Vulkan objects, for Skia: valid while the painter lives.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSPaintVulkan {
    pub get_instance_proc_addr: *mut c_void,
    pub instance: u64,
    pub physical_device: u64,
    pub device: u64,
    pub queue: u64,
    pub queue_family: u32,
    pub api_version: u32,
    pub instance_extensions: *const *const c_char,
    pub instance_extension_count: usize,
    pub device_extensions: *const *const c_char,
    pub device_extension_count: usize,
    pub features: *const c_void,
}

/// # Safety
/// `paint` is null or a live pointer from `oxyos_paint_new`, used by one thread at a time.
#[allow(unsafe_code)]
unsafe fn painter<'a>(paint: *mut OxyOSPaint) -> Option<&'a mut OxyOSPaint> {
    // SAFETY: per the contract.
    unsafe { paint.as_mut() }
}

/// A painter on the GPU whose render node is `device` (the compositor's, from
/// `oxyos_client_gpu_device`), its drivers alone loaded (decision 14); for 0, on Vulkan on the
/// CPU. No other GPU is opened or chosen. It touches no client, so it can be started on a thread
/// of its own (opening a device takes long). NULL with oxyos_last_error() (of the calling thread)
/// when that device has no Vulkan driver or device.
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_paint_new(device: u64) -> *mut OxyOSPaint {
    match Paint::new(device) {
        Ok(paint) => {
            let name = CString::new(paint.device_name().replace('\0', " ")).unwrap_or_default();
            let vulkan = paint.vulkan();
            // The names are 'static: the pointers outlive the painter.
            let pointers = |names: &[&'static std::ffi::CStr]| -> Vec<*const c_char> {
                names.iter().map(|name| name.as_ptr()).collect()
            };
            let instance_extensions = pointers(vulkan.instance_extensions);
            let device_extensions = pointers(vulkan.device_extensions);
            Box::into_raw(Box::new(OxyOSPaint {
                paint,
                name,
                instance_extensions,
                device_extensions,
            }))
        }
        Err(error) => {
            fail(error);
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// `paint` is null or a live pointer from `oxyos_paint_new`, not used after.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_free(paint: *mut OxyOSPaint) {
    if !paint.is_null() {
        // SAFETY: per the contract, it came from Box::into_raw.
        drop(unsafe { Box::from_raw(paint) });
    }
}

/// The device's name, valid while the painter lives.
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_device_name(paint: *mut OxyOSPaint) -> *const c_char {
    // SAFETY: per the contract.
    unsafe { painter(paint) }.map_or(std::ptr::null(), |paint| paint.name.as_ptr())
}

/// Whether it can present (it exports DMA-BUFs the compositor may take).
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_exports(paint: *mut OxyOSPaint) -> bool {
    // SAFETY: per the contract.
    unsafe { painter(paint) }.is_some_and(|paint| paint.paint.exports())
}

/// Writes the painter's Vulkan objects into `out`: 0, or OXYOS_ERROR.
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`; `out` is writable.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_vulkan(
    paint: *mut OxyOSPaint,
    out: *mut OxyOSPaintVulkan,
) -> c_int {
    // SAFETY: per the contract.
    let (Some(paint), Some(out)) = (unsafe { (painter(paint), out.as_mut()) }) else {
        return fail("no painter or output");
    };
    let vulkan = paint.paint.vulkan();
    *out = OxyOSPaintVulkan {
        get_instance_proc_addr: vulkan.get_instance_proc_addr as *mut c_void,
        instance: vulkan.instance.as_raw(),
        physical_device: vulkan.physical_device.as_raw(),
        device: vulkan.device.as_raw(),
        queue: vulkan.queue.as_raw(),
        queue_family: vulkan.queue_family,
        api_version: vulkan.api_version,
        instance_extensions: paint.instance_extensions.as_ptr(),
        instance_extension_count: paint.instance_extensions.len(),
        device_extensions: paint.device_extensions.as_ptr(),
        device_extension_count: paint.device_extensions.len(),
        features: std::ptr::from_ref(vulkan.features).cast(),
    };
    0
}

/// Presents `image` (a VkImage of `width` × `height`, B8G8R8A8_UNORM, in TRANSFER_SRC_OPTIMAL on
/// the painter's queue family, its drawing already submitted to the painter's queue) on
/// `surface` in answer to `configure` in one of the surface's GPU buffers, as `format`
/// (OXYOS_FORMAT_ARGB8888 premultiplied, or XRGB8888), of which `count` rectangles at `damage`
/// changed since the surface's previous frame (0: everything). Returns the frame's sequence
/// number, or -1: the GPU path cannot go on (no buffer the compositor and the device share, a
/// buffer it could not import, the device lost); the caller presents in shared memory.
///
/// # Safety
/// `paint` and `client` are live pointers; `damage` points to `count` rectangles; `image` is the
/// painter's device's, as above.
#[allow(unsafe_code)]
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_present(
    paint: *mut OxyOSPaint,
    client: *mut OxyOSClient,
    surface: u32,
    configure: u64,
    image: u64,
    width: u32,
    height: u32,
    format: u32,
    damage: *const OxyOSRect,
    count: usize,
) -> i64 {
    // SAFETY: per the contract.
    let (Some(paint), Some(client)) = (unsafe { (painter(paint), connection(client)) }) else {
        fail("no painter or client");
        return -1;
    };
    let format = match format {
        0x3432_5241 => Format::Argb8888,
        0x3432_5258 => Format::Xrgb8888,
        _ => {
            fail("an unknown format");
            return -1;
        }
    };
    // SAFETY: per the contract, `damage` points to `count` rectangles.
    let damage = unsafe { rects(damage, count) };
    let source = Source {
        image: vk::Image::from_raw(image),
        width,
        height,
    };
    match paint
        .paint
        .present(client, surface, configure, source, format, &damage)
    {
        Ok(sequence) => sequence as i64,
        Err(error) => {
            fail(error);
            -1
        }
    }
}

/// Where the last present's time went, in milliseconds (`oxyos_paint::PresentTimes`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyOSPaintPresentTimes {
    pub new_image: bool,
    pub freed_ms: f64,
    pub image_ms: f64,
    pub attach_ms: f64,
    pub timeline_ms: f64,
    pub release_ms: f64,
    pub wait_ms: f64,
    pub record_ms: f64,
    pub submit_ms: f64,
    pub sync_ms: f64,
    pub send_ms: f64,
    pub total_ms: f64,
    pub operations: u32,
}

/// Writes where the last present's time went into `out`.
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`; `out` is writable.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_last_present(
    paint: *mut OxyOSPaint,
    out: *mut OxyOSPaintPresentTimes,
) -> c_int {
    // SAFETY: per the contract.
    let (Some(paint), Some(out)) = (unsafe { (painter(paint), out.as_mut()) }) else {
        return fail("no painter or output");
    };
    let times = paint.paint.last_present();
    *out = OxyOSPaintPresentTimes {
        new_image: times.new_image,
        freed_ms: times.freed_ms,
        image_ms: times.image_ms,
        attach_ms: times.attach_ms,
        timeline_ms: times.timeline_ms,
        release_ms: times.release_ms,
        wait_ms: times.wait_ms,
        record_ms: times.record_ms,
        submit_ms: times.submit_ms,
        sync_ms: times.sync_ms,
        send_ms: times.send_ms,
        total_ms: times.total_ms,
        operations: times.operations,
    };
    0
}

/// Frees a surface's GPU buffers (it is destroyed).
///
/// # Safety
/// `paint` and `client` are live pointers.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_forget_surface(
    paint: *mut OxyOSPaint,
    client: *mut OxyOSClient,
    surface: u32,
) {
    // SAFETY: per the contract.
    if let (Some(paint), Some(client)) = unsafe { (painter(paint), connection(client)) } {
        paint.paint.forget_surface(client, surface);
    }
}

/// The Vulkan drivers of the GPU whose DRM node is `device` (`oxyos_client_gpu_device`'s; 0: the
/// CPU's alone), as `VK_DRIVER_FILES` takes them: their manifests' paths, `:`-separated, written
/// NUL-terminated at `out` (`capacity` bytes). Returns the length without the NUL, 0 when no
/// installed driver drives it (load none, then, rather than every one), or -1 when it does not
/// fit (nothing written) or `out` is null with a capacity. For a client whose Vulkan instances are
/// not its own to create (a browser's GPU process), set before any of them is.
///
/// # Safety
/// `out` is null or has `capacity` writable bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gpu_vulkan_driver_files(
    device: u64,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    let files = oxyos_gpu_device::Drivers::for_device(device).driver_files();
    let bytes = files.as_bytes();
    if out.is_null() || bytes.len() >= capacity || bytes.contains(&0) {
        return -1;
    }
    // SAFETY: `out` has `capacity` writable bytes (the caller's contract), more than the bytes and
    // their NUL; the source is this function's own string.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast::<u8>(), bytes.len());
        *out.add(bytes.len()) = 0;
    }
    bytes.len() as i64
}
