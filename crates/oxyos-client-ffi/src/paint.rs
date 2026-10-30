//! C ABI over `oxyos-paint` (docs/adr/0031-gpu-buffers.mdx, decision 12): a painter on the GPU
//! for any native client, its canvases, what it draws into them and the frames it presents in the
//! client's GPU buffers. Each operation names the device pixels it may touch, worked out by the
//! caller as its own CPU painter works them out.

use super::{client as connection, fail, rects, OxyOSClient, OxyOSRect};
use oxyos_paint::{Common, Fill, Glyph, Image, Layer, Paint, Pixels, Smooth, Source};
use oxyos_protocol::Format;
use std::ffi::{c_char, c_int, CString};

/// A painter and its device's name.
pub struct OxyOSPaint {
    paint: Paint,
    name: CString,
}

/// What every operation has: the device pixels it may touch ([x0, x1) × [y0, y1)), the canvas's
/// scale, its clip (logical x0, y0, x1, y1), its straight colour (R, G, B, A, 0 to 1) and opacity.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyOSPaintCommon {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub scale: f32,
    pub clip: [f32; 4],
    pub color: [f32; 4],
    pub opacity: f32,
}

/// A rounded box (x, y, width, height; radii top-left, top-right, bottom-left, bottom-right),
/// less a hole when `hole`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyOSPaintFill {
    pub rect: [f32; 4],
    pub radii: [f32; 4],
    pub hole: bool,
    pub hole_rect: [f32; 4],
    pub hole_radii: [f32; 4],
}

/// A picture (`picture`, when `canvas` is 0) or a canvas stretched over `rect`, rounded by `radii`
/// over `bounds`, tinted in the common colour when `tinted`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyOSPaintImage {
    pub picture: u64,
    pub canvas: u32,
    pub rect: [f32; 4],
    pub bounds: [f32; 4],
    pub radii: [f32; 4],
    pub tinted: bool,
}

/// A canvas drawn through an affine map: `inverse` takes the target's logical pixels to the
/// layer's (a, b, c, d, tx, ty), whose origin is `origin`, at `layer_scale` pixels per logical.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyOSPaintLayer {
    pub canvas: u32,
    pub layer_scale: f32,
    pub origin: [f32; 2],
    pub inverse: [f32; 6],
}

/// A glyph: its mask's top-left device pixel and the mask's key.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyOSPaintGlyph {
    pub x: i32,
    pub y: i32,
    pub mask: u64,
}

/// A canvas's levels blurred by `sigma` and drawn smooth (dithered) over `rect`, rounded.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyOSPaintSmooth {
    pub source: u32,
    pub sigma: f32,
    pub rect: [f32; 4],
    pub bounds: [f32; 4],
    pub radii: [f32; 4],
}

fn common(given: &OxyOSPaintCommon) -> Common {
    Common {
        pixels: Pixels {
            x0: given.x0,
            y0: given.y0,
            x1: given.x1,
            y1: given.y1,
        },
        scale: given.scale,
        clip: given.clip,
        color: given.color,
        opacity: given.opacity,
    }
}

/// # Safety
/// `paint` is null or a live pointer from `oxyos_paint_new`, used by one thread at a time.
#[allow(unsafe_code)]
unsafe fn painter<'a>(paint: *mut OxyOSPaint) -> Option<&'a mut OxyOSPaint> {
    // SAFETY: per the contract.
    unsafe { paint.as_mut() }
}

fn status(result: Result<(), oxyos_paint::Error>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

/// A painter on the GPU the client's compositor composes on (its GPU table's render node), else
/// on any Vulkan device (its frames then in linear buffers, or none it can export). NULL with
/// oxyos_last_error() when there is no Vulkan device.
///
/// # Safety
/// `client` is null or a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_new(client: *mut OxyOSClient) -> *mut OxyOSPaint {
    // SAFETY: per the contract.
    let device = unsafe { connection(client) }
        .and_then(|client| client.gpu().map(|table| table.device))
        .unwrap_or(0);
    match Paint::new(device) {
        Ok(paint) => {
            let name = CString::new(paint.device_name().replace('\0', " ")).unwrap_or_default();
            Box::into_raw(Box::new(OxyOSPaint { paint, name }))
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

/// A canvas of `width` × `height`, every pixel `value` (0xAARRGGBB, premultiplied); 0 on failure.
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_canvas(
    paint: *mut OxyOSPaint,
    width: u32,
    height: u32,
    value: u32,
) -> u32 {
    // SAFETY: per the contract.
    let Some(paint) = (unsafe { painter(paint) }) else {
        fail("no painter");
        return 0;
    };
    paint
        .paint
        .canvas(width, height, value)
        .unwrap_or_else(|error| {
            fail(error);
            0
        })
}

/// The canvas at another size, its pixels kept where both sizes have them (top left), the rest
/// `value`. 0, or OXYOS_ERROR.
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_canvas_resize(
    paint: *mut OxyOSPaint,
    canvas: u32,
    width: u32,
    height: u32,
    value: u32,
) -> c_int {
    // SAFETY: per the contract.
    let Some(paint) = (unsafe { painter(paint) }) else {
        return fail("no painter");
    };
    status(paint.paint.resize(canvas, width, height, value))
}

/// A copy of a canvas as everything drawn so far leaves it; 0 on failure.
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_canvas_copy(paint: *mut OxyOSPaint, canvas: u32) -> u32 {
    // SAFETY: per the contract.
    let Some(paint) = (unsafe { painter(paint) }) else {
        fail("no painter");
        return 0;
    };
    paint.paint.copy(canvas).unwrap_or_else(|error| {
        fail(error);
        0
    })
}

/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_canvas_free(paint: *mut OxyOSPaint, canvas: u32) {
    // SAFETY: per the contract.
    if let Some(paint) = unsafe { painter(paint) } {
        paint.paint.free_canvas(canvas);
    }
}

/// Reads a canvas (rows of 0xAARRGGBB) into `out`, `count` pixels at most, once everything drawn
/// is done. Returns its pixel count, or OXYOS_ERROR.
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`; `out` holds `count` words.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_canvas_read(
    paint: *mut OxyOSPaint,
    canvas: u32,
    out: *mut u32,
    count: usize,
) -> i64 {
    // SAFETY: per the contract.
    let Some(paint) = (unsafe { painter(paint) }) else {
        return i64::from(fail("no painter"));
    };
    match paint.paint.read(canvas) {
        Ok(pixels) => {
            let copied = pixels.len().min(count);
            if !out.is_null() {
                // SAFETY: `out` holds `count` words, at least `copied`.
                unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), out, copied) };
            }
            pixels.len() as i64
        }
        Err(error) => i64::from(fail(error)),
    }
}

/// Sets [x0, x1) × [y0, y1) of a canvas to `value`. 0, or OXYOS_ERROR.
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_clear(
    paint: *mut OxyOSPaint,
    canvas: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    value: u32,
) -> c_int {
    // SAFETY: per the contract.
    let Some(paint) = (unsafe { painter(paint) }) else {
        return fail("no painter");
    };
    status(paint.paint.clear(canvas, Pixels { x0, y0, x1, y1 }, value))
}

/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`; `given` and `fill` point to one each.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_fill(
    paint: *mut OxyOSPaint,
    canvas: u32,
    given: *const OxyOSPaintCommon,
    fill: *const OxyOSPaintFill,
) -> c_int {
    // SAFETY: per the contract.
    let (Some(paint), Some(given), Some(fill)) =
        (unsafe { (painter(paint), given.as_ref(), fill.as_ref()) })
    else {
        return fail("no painter or operation");
    };
    status(paint.paint.fill(
        canvas,
        &common(given),
        &Fill {
            rect: fill.rect,
            radii: fill.radii,
            hole: fill.hole.then_some((fill.hole_rect, fill.hole_radii)),
        },
    ))
}

/// Makes a picture (`width` × `height` premultiplied R, G, B, A bytes) known by `key`: 1 when it
/// was uploaded, 0 when it was known already (`rgba` is not read), OXYOS_ERROR.
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`; `rgba` holds `width * height * 4` bytes or
/// is null for a picture known.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_picture(
    paint: *mut OxyOSPaint,
    key: u64,
    width: u32,
    height: u32,
    rgba: *const u8,
) -> c_int {
    // SAFETY: per the contract.
    let Some(paint) = (unsafe { painter(paint) }) else {
        return fail("no painter");
    };
    let bytes = if rgba.is_null() {
        &[][..]
    } else {
        // SAFETY: per the contract, `rgba` holds the picture's bytes.
        unsafe { std::slice::from_raw_parts(rgba, width as usize * height as usize * 4) }
    };
    match paint.paint.picture(key, width, height, bytes) {
        Ok(uploaded) => c_int::from(uploaded),
        Err(error) => fail(error),
    }
}

/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_forget_picture(paint: *mut OxyOSPaint, key: u64) {
    // SAFETY: per the contract.
    if let Some(paint) = unsafe { painter(paint) } {
        paint.paint.forget_picture(key);
    }
}

/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`; `given` and `image` point to one each.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_image(
    paint: *mut OxyOSPaint,
    canvas: u32,
    given: *const OxyOSPaintCommon,
    image: *const OxyOSPaintImage,
) -> c_int {
    // SAFETY: per the contract.
    let (Some(paint), Some(given), Some(image)) =
        (unsafe { (painter(paint), given.as_ref(), image.as_ref()) })
    else {
        return fail("no painter or operation");
    };
    let source = if image.canvas != 0 {
        Source::Canvas(image.canvas)
    } else {
        Source::Picture(image.picture)
    };
    status(paint.paint.image(
        canvas,
        &common(given),
        &Image {
            source,
            rect: image.rect,
            bounds: image.bounds,
            radii: image.radii,
            tinted: image.tinted,
        },
    ))
}

/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`; `given` and `layer` point to one each.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_layer(
    paint: *mut OxyOSPaint,
    canvas: u32,
    given: *const OxyOSPaintCommon,
    layer: *const OxyOSPaintLayer,
) -> c_int {
    // SAFETY: per the contract.
    let (Some(paint), Some(given), Some(layer)) =
        (unsafe { (painter(paint), given.as_ref(), layer.as_ref()) })
    else {
        return fail("no painter or operation");
    };
    status(paint.paint.layer(
        canvas,
        &common(given),
        &Layer {
            canvas: layer.canvas,
            layer_scale: layer.layer_scale,
            origin: layer.origin,
            inverse: layer.inverse,
        },
    ))
}

/// Makes a glyph mask (`width` × `height` coverage bytes) known by `key`: 1 uploaded, 0 known,
/// OXYOS_ERROR.
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`; `mask` holds `width * height` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_glyph(
    paint: *mut OxyOSPaint,
    key: u64,
    width: u32,
    height: u32,
    mask: *const u8,
) -> c_int {
    // SAFETY: per the contract.
    let Some(paint) = (unsafe { painter(paint) }) else {
        return fail("no painter");
    };
    let bytes = if mask.is_null() || width == 0 || height == 0 {
        &[][..]
    } else {
        // SAFETY: per the contract, `mask` holds the mask's bytes.
        unsafe { std::slice::from_raw_parts(mask, width as usize * height as usize) }
    };
    match paint.paint.glyph(key, width, height, bytes) {
        Ok(uploaded) => c_int::from(uploaded),
        Err(error) => fail(error),
    }
}

/// Whether a glyph mask is known by `key`.
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_has_glyph(paint: *mut OxyOSPaint, key: u64) -> bool {
    // SAFETY: per the contract.
    unsafe { painter(paint) }.is_some_and(|paint| paint.paint.has_glyph(key))
}

/// A run of `count` glyphs in the common colour (its alpha with the opacity in it).
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`; `given` points to one; `glyphs` to `count`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_glyphs(
    paint: *mut OxyOSPaint,
    canvas: u32,
    given: *const OxyOSPaintCommon,
    glyphs: *const OxyOSPaintGlyph,
    count: usize,
) -> c_int {
    // SAFETY: per the contract.
    let (Some(paint), Some(given)) = (unsafe { (painter(paint), given.as_ref()) }) else {
        return fail("no painter or operation");
    };
    if count == 0 || glyphs.is_null() {
        return 0;
    }
    // SAFETY: per the contract, `glyphs` points to `count` glyphs.
    let glyphs: Vec<Glyph> = unsafe { std::slice::from_raw_parts(glyphs, count) }
        .iter()
        .map(|glyph| Glyph {
            x: glyph.x,
            y: glyph.y,
            mask: glyph.mask,
        })
        .collect();
    status(paint.paint.glyphs(canvas, &common(given), &glyphs))
}

/// A canvas blurred in place by `sigma` device pixels (a Gaussian as three box passes).
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_blur(
    paint: *mut OxyOSPaint,
    canvas: u32,
    sigma: f32,
) -> c_int {
    // SAFETY: per the contract.
    let Some(paint) = (unsafe { painter(paint) }) else {
        return fail("no painter");
    };
    status(paint.paint.blur(canvas, sigma))
}

/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`; `given` and `smooth` point to one each.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_smooth(
    paint: *mut OxyOSPaint,
    canvas: u32,
    given: *const OxyOSPaintCommon,
    smooth: *const OxyOSPaintSmooth,
) -> c_int {
    // SAFETY: per the contract.
    let (Some(paint), Some(given), Some(smooth)) =
        (unsafe { (painter(paint), given.as_ref(), smooth.as_ref()) })
    else {
        return fail("no painter or operation");
    };
    status(paint.paint.smooth(
        canvas,
        &common(given),
        &Smooth {
            source: smooth.source,
            sigma: smooth.sigma,
            rect: smooth.rect,
            bounds: smooth.bounds,
            radii: smooth.radii,
        },
    ))
}

/// `content` kept by the alpha of `mask`, a canvas of its size.
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_mask(
    paint: *mut OxyOSPaint,
    content: u32,
    mask: u32,
) -> c_int {
    // SAFETY: per the contract.
    let Some(paint) = (unsafe { painter(paint) }) else {
        return fail("no painter");
    };
    status(paint.paint.mask(content, mask))
}

/// Runs what is recorded (returns once submitted).
///
/// # Safety
/// `paint` is a live pointer from `oxyos_paint_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_flush(paint: *mut OxyOSPaint) -> c_int {
    // SAFETY: per the contract.
    let Some(paint) = (unsafe { painter(paint) }) else {
        return fail("no painter");
    };
    status(paint.paint.flush())
}

/// Presents `canvas` on `surface` in answer to `configure` in one of the surface's GPU buffers, as
/// `format` (OXYOS_FORMAT_ARGB8888 premultiplied, or XRGB8888), of which `count` rectangles at
/// `damage` changed since the surface's previous frame (0: everything). Returns the frame's
/// sequence number, or -1: the GPU path cannot go on (no buffer the compositor and the device
/// share, a buffer it could not import, the device lost); the caller presents in shared memory.
///
/// # Safety
/// `paint` and `client` are live pointers; `damage` points to `count` rectangles.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_paint_present(
    paint: *mut OxyOSPaint,
    client: *mut OxyOSClient,
    surface: u32,
    configure: u64,
    canvas: u32,
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
    match paint
        .paint
        .present(client, surface, configure, canvas, format, &damage)
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
