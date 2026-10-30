//! A window's corners and its shadow (docs/adr/0016-windows.mdx): the integer arithmetic the
//! reference composes them with, which `shaders/compose.wgsl` repeats operation for operation so
//! the two agree byte for byte, and the shadow's profile, computed once on the CPU.
//!
//! Lengths are in the output's pixels, worked in halves so a pixel's centre is a whole number: the
//! centre of pixel `x` is `2x + 1`.

use crate::Rect;

/// The largest corner radius drawn, in pixels; a larger one is drawn at this.
pub const MAX_RADIUS: u32 = 512;
/// The most entries a shadow's profile has: it reaches at most 512 pixels from its shape.
pub const MAX_PROFILE: usize = 8192;
/// The largest a shadow is drawn relative to its profile, in 256ths (four times).
pub const MAX_SHADOW_SCALE: u32 = 1024;
/// Entries of a shadow's profile per pixel of distance.
const STEPS_PER_PIXEL: u32 = 8;
/// Half-pixel distances beyond this from a shape are taken as this: far beyond any profile.
const FAR: i32 = 1 << 13;

/// A radius drawn on a `width` × `height` rectangle: at most half its shorter side.
pub(crate) fn radius_on(radius: u32, width: u32, height: u32) -> u32 {
    radius.min(width / 2).min(height / 2).min(MAX_RADIUS)
}

/// The integer square root (the largest `r` with `r * r <= n`), digit by digit.
pub(crate) fn isqrt(n: u32) -> u32 {
    let mut rest = n;
    let mut root = 0u32;
    let mut bit = 1u32 << 30;
    for _ in 0..16 {
        if rest >= root + bit {
            rest -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    root
}

/// How much of pixel (`x`, `y`) of a `width` × `height` rectangle its corners of `radius` (as
/// `radius_on` gives it) leave, 0 to 255: all of it but in a corner's square, where it is the part
/// of the pixel inside the corner's circle, from the distance of its centre to the circle's to
/// within 1/64 pixel.
pub(crate) fn corner_coverage(x: u32, y: u32, width: u32, height: u32, radius: u32) -> u32 {
    // The pixel's centre from the corner circle's centre, in half pixels, when it is in a corner's
    // square: at most 2·radius each way.
    let toward = |at: u32, length: u32| -> u32 {
        if at < radius {
            2 * radius - (2 * at + 1)
        } else if at >= length - radius {
            2 * at + 1 - 2 * (length - radius)
        } else {
            0
        }
    };
    let (dx, dy) = (toward(x, width), toward(y, height));
    if dx == 0 || dy == 0 {
        return 255;
    }
    // 64 times the distance in pixels: dx² + dy² is at most 8·512², so 1024 times it fits.
    let distance = isqrt((dx * dx + dy * dy) << 10) as i32;
    // Half a pixel inside the circle's edge and beyond: all; half a pixel outside: nothing.
    let inside = (32 - (distance - 64 * radius as i32)).clamp(0, 64) as u32;
    inside * 255 / 64
}

/// The entry of a profile of `entries` for pixel (`x`, `y`) (relative to the layer): its centre's
/// signed distance from `shape`'s rounded rectangle, of `radius` (as `radius_on` gives it), in 1/8
/// pixel, at the profile's `scale` (256ths). The entry at `entries / 2` is the shape's edge.
pub(crate) fn shadow_entry(
    x: u32,
    y: u32,
    shape: &Rect,
    radius: u32,
    scale: u32,
    entries: u32,
) -> u32 {
    // From the centre, beyond the straight part of each side, in half pixels.
    let beyond = |at: u32, start: i32, length: u32| -> i32 {
        let centre = 2 * start + length as i32;
        ((2 * at as i32 + 1 - centre).abs() - (length - 2 * radius) as i32).min(FAR)
    };
    let qx = beyond(x, shape.x, shape.width);
    let qy = beyond(y, shape.y, shape.height);
    // Past a corner the distance is to its circle; elsewhere to the nearer side.
    let distance = if qx > 0 && qy > 0 {
        isqrt(((qx * qx + qy * qy) as u32) << 4) as i32
    } else {
        4 * qx.max(qy)
    } - 8 * radius as i32;
    let steps = distance.clamp(-(1 << 18), 1 << 18) * 256 + (entries / 2 * scale) as i32;
    if steps <= 0 {
        0
    } else {
        (steps as u32 / scale).min(entries - 1)
    }
}

/// A shadow's profile: premultiplied black pixels, one for each 1/8 pixel of signed distance from
/// the shape casting it, from inside to outside, its edge in the middle; drawn at a size, a
/// shadow takes each pixel's from its distance (`Paint::Shadow`). It is the shape blurred by a
/// Gaussian, as CSS's `box-shadow` blurs (a blur of `b` has σ = b/2), at `alpha`: along a side
/// exactly that; past a corner, where it is taken from the distance to the corner's circle, a
/// little rounder than a true blur. It ends where it rounds to nothing, three σ out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowProfile {
    pixels: Vec<u8>,
}

impl ShadowProfile {
    pub fn new(blur: u32, alpha: u8) -> Self {
        let sigma = f64::from(blur) / 2.0;
        let reach = ((3.0 * sigma).ceil() as u32).clamp(1, MAX_PROFILE as u32 / 16);
        let entries = reach * 2 * STEPS_PER_PIXEL;
        let pixels = (0..entries)
            .flat_map(|entry| {
                // The distance at the entry's middle.
                let distance =
                    (f64::from(entry) - f64::from(entries / 2) + 0.5) / f64::from(STEPS_PER_PIXEL);
                let shown = if sigma == 0.0 {
                    f64::from(u8::from(distance < 0.0))
                } else {
                    normal_cdf(-distance / sigma)
                };
                let alpha = (f64::from(alpha) * shown).round() as u32;
                (alpha << 24).to_le_bytes()
            })
            .collect();
        Self { pixels }
    }

    /// The pixels, a layer's for `Paint::Shadow`, at an extent of `entries()` × 1.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn entries(&self) -> u32 {
        (self.pixels.len() / 4) as u32
    }

    /// How far beyond its shape the shadow reaches, in pixels, at the profile's size.
    pub fn reach(&self) -> u32 {
        self.entries() / 2 / STEPS_PER_PIXEL
    }
}

/// The standard normal distribution's share below `z`, from Abramowitz and Stegun's 7.1.26 for
/// erf (error under 1.5e-7, far below a 1/255 step).
fn normal_cdf(z: f64) -> f64 {
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-x * x).exp();
    if z >= 0.0 {
        0.5 * (1.0 + erf)
    } else {
        0.5 * (1.0 - erf)
    }
}
