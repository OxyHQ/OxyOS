//! Whether an application's icon picture is already its own shape: a rounded square, transparent
//! around it and opaque within, as Dropsea's navy tile is. The shell draws such a picture as it
//! is and puts any other (a square, a round logo, a drawing with holes) in the icon grid's rounded
//! body on white, as macOS 26 draws an icon that has its shape as it is and puts one that has not in
//! a plate (docs/adr/0011-transitions.mdx, the launcher; docs/adr/0034-the-dock.mdx).
//!
//! The picture decides, not a list of applications: its alpha is measured once, as its file is
//! read (`oxyos-client-ffi`'s applications, which keep what they measured by the file).

use crate::{decode_file, Error, Fit, Image};
use std::path::Path;

/// A pixel at least this opaque is the body's (the edge's anti-aliasing included at half).
const SOLID: u8 = 128;
/// The body is opaque: every pixel within it at least this.
const OPAQUE: u8 = 250;
/// The corners are transparent: the body's box's corner pixels at most this.
const CLEAR: u8 = 8;
/// A rounded square covers at least this share of its box; a circle covers π/4 (0.785) and is
/// not one. Dropsea's squircle covers 0.88, the icon grid's rounded square (corners 208 of 896)
/// 0.95.
const SQUARE_COVER: f64 = 5.0 / 6.0;

/// The share of the picture's width its own body takes, when it is its own shape (1 for a picture
/// that is its body edge to edge, 0.875 for one drawn on Apple's 1024 grid with its margins):
/// a body whose box is square and centred in a square picture; that box's corners transparent;
/// every pixel within the circle the box holds opaque (no hole, nothing see-through); and the
/// body a rounded square, not round. None for any other picture.
pub fn own_shape(image: &Image) -> Option<f64> {
    let (width, height) = (image.width as usize, image.height as usize);
    if width == 0 || width != height || image.pixels.len() < width * height * 4 {
        return None;
    }
    let alpha = |x: usize, y: usize| image.pixels[(y * width + x) * 4 + 3];
    // The body's box: what is at least half opaque.
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
    for y in 0..height {
        for x in 0..width {
            if alpha(x, y) >= SOLID {
                left = left.min(x);
                right = right.max(x);
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
    }
    if left > right {
        return None;
    }
    let side = right - left + 1;
    // Square and centred, to a pixel or a hundredth of the picture.
    let slack = (width / 100).max(1);
    let centred = |near: usize, far: usize| near.abs_diff(width - 1 - far) <= slack;
    if side.abs_diff(bottom - top + 1) > slack || !centred(left, right) || !centred(top, bottom) {
        return None;
    }
    if [(left, top), (right, top), (left, bottom), (right, bottom)]
        .into_iter()
        .any(|(x, y)| alpha(x, y) > CLEAR)
    {
        return None;
    }
    // Within the circle the box holds, a pixel short of its edge, all opaque; and the body covers
    // its box as a rounded square does.
    let (centre_x, centre_y) = ((left + right) as f64 / 2.0, (top + bottom) as f64 / 2.0);
    let radius = side as f64 / 2.0 - 1.5;
    let mut covered = 0usize;
    for y in top..=bottom {
        for x in left..=right {
            let a = alpha(x, y);
            if a >= SOLID {
                covered += 1;
            }
            let (dx, dy) = (x as f64 - centre_x, y as f64 - centre_y);
            if dx * dx + dy * dy <= radius * radius && a < OPAQUE {
                return None;
            }
        }
    }
    let box_pixels = side * (bottom - top + 1);
    (covered as f64 >= SQUARE_COVER * box_pixels as f64).then(|| side as f64 / width as f64)
}

/// [`own_shape`] of the picture in a file, decoded whole.
pub fn own_shape_of_file(path: &Path) -> Result<Option<f64>, Error> {
    Ok(own_shape(&decode_file(path, Fit::Whole)?))
}

#[cfg(test)]
#[path = "shape_tests.rs"]
mod tests;
