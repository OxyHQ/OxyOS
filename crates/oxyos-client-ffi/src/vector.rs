//! C ABI over `oxyos-vector`, the rasterizer behind react-native-svg in the Bloom host. The host
//! flattens an SVG element tree into shapes; this layer converts them and returns the picture.
//! Codes for caps, joins and fill rules are react-native-svg's own.

use super::fail;
use oxyos_vector::{render, Cap, Fill, Geometry, Join, Shape, Stroke};
use std::ffi::{c_char, c_int};

/// Shape kinds (`OxyOSVectorShape.kind`).
pub const OXYOS_SHAPE_PATH: u8 = 0;
pub const OXYOS_SHAPE_RECT: u8 = 1;
pub const OXYOS_SHAPE_ELLIPSE: u8 = 2;
pub const OXYOS_SHAPE_LINE: u8 = 3;
/// Line caps and joins, as react-native-svg numbers them.
pub const OXYOS_CAP_BUTT: u8 = 0;
pub const OXYOS_CAP_ROUND: u8 = 1;
pub const OXYOS_CAP_SQUARE: u8 = 2;
pub const OXYOS_JOIN_MITER: u8 = 0;
pub const OXYOS_JOIN_ROUND: u8 = 1;
pub const OXYOS_JOIN_BEVEL: u8 = 2;

/// One shape: its geometry (path data for `OXYOS_SHAPE_PATH`; otherwise `geometry` holds the
/// rectangle's x, y, width, height, rx, ry, the ellipse's cx, cy, rx, ry or the line's x1, y1, x2,
/// y2), SVG's matrix to the picture's pixels, and its fill and stroke (0xAARRGGBB, not
/// premultiplied; a zero `fill` or `stroke` for none).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSVectorShape {
    pub kind: u8,
    pub data: *const c_char,
    pub data_length: usize,
    pub geometry: [f32; 6],
    pub transform: [f32; 6],
    pub fill: u8,
    pub even_odd: u8,
    pub fill_color: u32,
    pub stroke: u8,
    pub cap: u8,
    pub join: u8,
    pub stroke_color: u32,
    pub stroke_width: f32,
    pub miter_limit: f32,
    pub dash: *const f32,
    pub dash_count: usize,
    pub dash_offset: f32,
}

/// Draws `count` shapes in order onto a transparent `width` × `height` picture, written to
/// `pixels` (`length` bytes, at least `width` × `height` × 4) as premultiplied R, G, B, A bytes,
/// rows top to bottom, as decoded images are.
/// Returns how many shapes were left out (path data SVG would reject, say), or OXYOS_ERROR.
///
/// # Safety
/// `shapes` points to `count` valid shapes (any pointer when `count` is 0), each with `data`
/// pointing to `data_length` bytes and `dash` to `dash_count` floats; `pixels` points to `length`
/// writable bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_vector_render(
    shapes: *const OxyOSVectorShape,
    count: usize,
    width: u32,
    height: u32,
    pixels: *mut u8,
    length: usize,
) -> c_int {
    if pixels.is_null() || (count > 0 && shapes.is_null()) {
        return fail("null shapes or pixels");
    }
    if (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4)
        > length
    {
        return fail("the picture does not fit the pixels given");
    }
    let given: &[OxyOSVectorShape] = if count == 0 {
        &[]
    } else {
        // SAFETY: per the contract, `shapes` points to `count` valid shapes.
        unsafe { std::slice::from_raw_parts(shapes, count) }
    };
    let mut converted = Vec::with_capacity(given.len());
    for shape in given {
        let geometry = match shape.kind {
            OXYOS_SHAPE_PATH => {
                let bytes = if shape.data_length == 0 {
                    &[][..]
                } else if shape.data.is_null() {
                    return fail("null path data");
                } else {
                    // SAFETY: per the contract, `data` points to `data_length` bytes.
                    unsafe {
                        std::slice::from_raw_parts(shape.data.cast::<u8>(), shape.data_length)
                    }
                };
                let Ok(data) = std::str::from_utf8(bytes) else {
                    return fail("path data is not UTF-8");
                };
                Geometry::Path(data)
            }
            OXYOS_SHAPE_RECT => {
                let [x, y, width, height, rx, ry] = shape.geometry;
                Geometry::Rect {
                    x,
                    y,
                    width,
                    height,
                    rx,
                    ry,
                }
            }
            OXYOS_SHAPE_ELLIPSE => {
                let [cx, cy, rx, ry, ..] = shape.geometry;
                Geometry::Ellipse { cx, cy, rx, ry }
            }
            OXYOS_SHAPE_LINE => {
                let [x1, y1, x2, y2, ..] = shape.geometry;
                Geometry::Line { x1, y1, x2, y2 }
            }
            _ => return fail("unknown shape kind"),
        };
        let dash: &[f32] = if shape.dash_count == 0 {
            &[]
        } else if shape.dash.is_null() {
            return fail("null dash");
        } else {
            // SAFETY: per the contract, `dash` points to `dash_count` floats.
            unsafe { std::slice::from_raw_parts(shape.dash, shape.dash_count) }
        };
        let (Some(cap), Some(join)) = (
            [Cap::Butt, Cap::Round, Cap::Square].get(usize::from(shape.cap)),
            [Join::Miter, Join::Round, Join::Bevel].get(usize::from(shape.join)),
        ) else {
            return fail("unknown cap or join");
        };
        converted.push(Shape {
            geometry,
            transform: shape.transform,
            fill: (shape.fill != 0).then_some(Fill {
                color: shape.fill_color,
                even_odd: shape.even_odd != 0,
            }),
            stroke: (shape.stroke != 0).then_some(Stroke {
                color: shape.stroke_color,
                width: shape.stroke_width,
                cap: *cap,
                join: *join,
                miter_limit: shape.miter_limit,
                dash,
                dash_offset: shape.dash_offset,
            }),
        });
    }
    let Some(picture) = render(&converted, width, height) else {
        return fail("a picture must be 1 to 16384 pixels a side");
    };
    // SAFETY: per the contract, `pixels` points to `length` writable bytes, which hold the
    // picture (checked above).
    let out = unsafe { std::slice::from_raw_parts_mut(pixels, picture.pixels.len()) };
    out.copy_from_slice(&picture.pixels);
    c_int::try_from(picture.skipped).unwrap_or(c_int::MAX)
}
