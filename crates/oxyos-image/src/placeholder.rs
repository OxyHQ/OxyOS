//! BlurHashes and ThumbHashes: an image's placeholder in a few dozen characters, decoded as
//! expo-image's Android implementation decodes them (`BlurhashDecoder.kt`, from Wolt's blurhash;
//! `ThumbhashDecoder.kt`, from Evan Wallace's thumbhash), its arithmetic theirs: in 32-bit floats
//! where theirs is, a power or a cosine in 64 bits and rounded to 32 where Kotlin's is, and the
//! same rounding to 8 bits. A placeholder decoded here has the pixels it has on Android.

use crate::{checked, finish, Error, Fit, Image};
use std::f64::consts::PI;

/// Kotlin's `Float.pow`: the power in 64 bits, rounded to 32.
fn pow(base: f32, exponent: f32) -> f32 {
    f64::from(base).powf(f64::from(exponent)) as f32
}

/// BlurHash's base 83 digits, each at its value.
const DIGITS: &[u8; 83] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#$%*+,-.:;=?@[]^_{|}~";

/// The number the digits of `hash` from `from` to `to` write in base 83; a character that is not
/// a digit is skipped, as Kotlin's `decode83` skips it.
fn decode83(hash: &[u16], from: usize, to: usize) -> i32 {
    hash[from..to].iter().fold(0i32, |value, &unit| {
        match DIGITS.iter().position(|&digit| u16::from(digit) == unit) {
            Some(digit) => value.wrapping_mul(83).wrapping_add(digit as i32),
            None => value,
        }
    })
}

fn srgb_to_linear(value: i32) -> f32 {
    let v = value as f32 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        pow((v + 0.055) / 1.055, 2.4)
    }
}

/// The decoder's own `linearToSrgb`: clamped (a NaN kept, and 0 after), then to 8 bits by
/// truncating its value plus a half.
fn linear_to_srgb(value: f32) -> u8 {
    let v = value.clamp(0.0, 1.0);
    let level = if v <= 0.003_130_8 {
        v * 12.92 * 255.0 + 0.5
    } else {
        (1.055 * pow(v, 1.0 / 2.4) - 0.055) * 255.0 + 0.5
    };
    level as u8
}

fn signed_pow2(value: f32) -> f32 {
    pow(value, 2.0).copysign(value)
}

/// A BlurHash decoded at `width` × `height` pixels with a punch of 1, opaque; an invalid one is
/// refused with expo-image's message.
pub fn blurhash(hash: &str, width: u32, height: u32) -> Result<Image, Error> {
    let failed = || Error::Source(format!("Cannot decode provided blurhash '{hash}'"));
    // Kotlin's string: its length and characters in UTF-16 units.
    let units: Vec<u16> = hash.encode_utf16().collect();
    if units.len() < 6 || width == 0 || height == 0 {
        return Err(failed());
    }
    checked(width, height)?;
    let components = decode83(&units, 0, 1);
    let (across, down) = ((components % 9 + 1) as usize, (components / 9 + 1) as usize);
    if units.len() != 4 + 2 * across * down {
        return Err(failed());
    }
    let max_ac = (decode83(&units, 1, 2) + 1) as f32 / 166.0;
    let punch = 1.0f32;
    let colors: Vec<[f32; 3]> = (0..across * down)
        .map(|i| {
            if i == 0 {
                let value = decode83(&units, 2, 6);
                [
                    srgb_to_linear(value >> 16),
                    srgb_to_linear((value >> 8) & 255),
                    srgb_to_linear(value & 255),
                ]
            } else {
                let value = decode83(&units, 4 + i * 2, 6 + i * 2);
                let scale = max_ac * punch;
                let (r, g, b) = (value / (19 * 19), (value / 19) % 19, value % 19);
                [r, g, b].map(|level| signed_pow2((level - 9) as f32 / 9.0) * scale)
            }
        })
        .collect();
    let cosines = |size: u32, components: usize| -> Vec<f64> {
        (0..size)
            .flat_map(|at| {
                (0..components)
                    .map(move |c| (PI * f64::from(at) * c as f64 / f64::from(size)).cos())
            })
            .collect()
    };
    let (cos_x, cos_y) = (cosines(width, across), cosines(height, down));
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let mut sum = [0f32; 3];
            for j in 0..down {
                for i in 0..across {
                    let basis = (cos_x[x * across + i] * cos_y[y * down + j]) as f32;
                    let color = colors[j * across + i];
                    for c in 0..3 {
                        sum[c] += color[c] * basis;
                    }
                }
            }
            pixels.extend(sum.map(linear_to_srgb));
            pixels.push(255);
        }
    }
    Ok(Image::still(width, height, (width, height), pixels))
}

/// A ThumbHash's coefficients for one channel, read from its nibbles.
fn channel(
    hash: &[u8],
    start: usize,
    index: &mut usize,
    across: i32,
    down: i32,
    scale: f32,
) -> Option<Vec<f32>> {
    let mut ac = Vec::new();
    for cy in 0..down {
        let mut cx = i32::from(cy == 0);
        while cx * down < across * (down - cy) {
            let byte = *hash.get(start + (*index >> 1))?;
            let nibble = (byte >> ((*index & 1) << 2)) & 15;
            ac.push((f32::from(nibble) / 7.5 - 1.0) * scale);
            *index += 1;
            cx += 1;
        }
    }
    Some(ac)
}

/// Java's `Math.round(float)`: the nearest integer, a half up; 0 for a NaN.
fn round(value: f32) -> i32 {
    (f64::from(value) + 0.5).floor() as i32
}

/// Java's `Math.min(float, float)`: a NaN if either is one.
fn min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else {
        a.min(b)
    }
}

/// A ThumbHash's straight RGBA at its own size (at most 32 pixels a side, its proportions the
/// hash's), as `ThumbhashDecoder.thumbHashToRGBA`; none if the hash is too short for what its
/// header says it holds, or gives no size.
fn thumbhash_rgba(hash: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let byte = |at: usize| hash.get(at).copied().map(u32::from);
    let header24 = byte(0)? | (byte(1)? << 8) | (byte(2)? << 16);
    let header16 = byte(3)? | (byte(4)? << 8);
    let l_dc = (header24 & 63) as f32 / 63.0;
    let p_dc = ((header24 >> 6) & 63) as f32 / 31.5 - 1.0;
    let q_dc = ((header24 >> 12) & 63) as f32 / 31.5 - 1.0;
    let l_scale = ((header24 >> 18) & 31) as f32 / 31.0;
    let has_alpha = header24 >> 23 != 0;
    let p_scale = ((header16 >> 3) & 63) as f32 / 63.0;
    let q_scale = ((header16 >> 9) & 63) as f32 / 63.0;
    let landscape = header16 >> 15 != 0;
    let own = (header16 & 7) as i32;
    let long = if has_alpha { 5 } else { 7 };
    let (lx, ly) = if landscape { (long, own) } else { (own, long) };
    let (lx, ly) = (lx.max(3), ly.max(3));
    // The alpha byte is read whether the hash has alpha or not, as Kotlin reads it.
    let a_byte = byte(5)?;
    let a_dc = if has_alpha {
        (a_byte & 15) as f32 / 15.0
    } else {
        1.0
    };
    let a_scale = ((a_byte >> 4) & 15) as f32 / 15.0;
    let start = if has_alpha { 6 } else { 5 };
    let mut index = 0;
    let l_ac = channel(hash, start, &mut index, lx, ly, l_scale)?;
    let p_ac = channel(hash, start, &mut index, 3, 3, p_scale * 1.25)?;
    let q_ac = channel(hash, start, &mut index, 3, 3, q_scale * 1.25)?;
    let a_ac = if has_alpha {
        channel(hash, start, &mut index, 5, 5, a_scale)?
    } else {
        Vec::new()
    };
    // Its size, from `thumbHashToApproximateAspectRatio`: the counts before they are at least 3.
    let ratio =
        (if landscape { long } else { own }) as f32 / (if landscape { own } else { long }) as f32;
    let width = round(if ratio > 1.0 { 32.0 } else { 32.0 * ratio });
    let height = round(if ratio > 1.0 { 32.0 / ratio } else { 32.0 });
    if width <= 0 || height <= 0 {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    let stop = if has_alpha { 5 } else { 3 };
    let (cx_stop, cy_stop) = (lx.max(stop) as usize, ly.max(stop) as usize);
    let mut fx = vec![0f32; cx_stop];
    let mut fy = vec![0f32; cy_stop];
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (mut l, mut p, mut q, mut a) = (l_dc, p_dc, q_dc, a_dc);
            for (cx, f) in fx.iter_mut().enumerate() {
                *f = (PI / w as f64 * f64::from(x as f32 + 0.5) * cx as f64).cos() as f32;
            }
            for (cy, f) in fy.iter_mut().enumerate() {
                *f = (PI / h as f64 * f64::from(y as f32 + 0.5) * cy as f64).cos() as f32;
            }
            let mut j = 0;
            for cy in 0..ly {
                let fy2 = fy[cy as usize] * 2.0;
                let mut cx = i32::from(cy == 0);
                while cx * ly < lx * (ly - cy) {
                    l += l_ac[j] * fx[cx as usize] * fy2;
                    cx += 1;
                    j += 1;
                }
            }
            // P and Q over a triangle of 3 coefficients a side, A over one of 5, in the order
            // their coefficients were read.
            let triangle = |side: usize| {
                (0..side)
                    .flat_map(move |cy| (usize::from(cy == 0)..side - cy).map(move |cx| (cx, cy)))
            };
            for (j, (cx, cy)) in triangle(3).enumerate() {
                let f = fx[cx] * (fy[cy] * 2.0);
                p += p_ac[j] * f;
                q += q_ac[j] * f;
            }
            if has_alpha {
                for (k, (cx, cy)) in triangle(5).enumerate() {
                    a += a_ac[k] * fx[cx] * (fy[cy] * 2.0);
                }
            }
            let b = l - 2.0 / 3.0 * p;
            let r = (3.0 * l - b + q) / 2.0;
            let g = r - q;
            rgba.extend([r, g, b, a].map(|value| round(255.0 * min(1.0, value)).max(0) as u8));
        }
    }
    Some((width as u32, height as u32, rgba))
}

/// A ThumbHash given in base64 (standard or URL-safe, padding optional), decoded at its own size
/// and reduced as far as `fit` asks, premultiplied; an invalid one is refused with expo-image's
/// message.
pub fn thumbhash(hash: &str, fit: Fit) -> Result<Image, Error> {
    let failed = |reason: &str| {
        Error::Source(format!(
            "Cannot decode provided thumbhash '{hash}' {reason}"
        ))
    };
    let bytes = crate::data::base64(hash.as_bytes()).ok_or_else(|| failed("(not base64)"))?;
    let (width, height, rgba) =
        thumbhash_rgba(&bytes).ok_or_else(|| failed("(shorter than its header says)"))?;
    let target = fit.size(width, height);
    let pixels = finish(rgba, (width, height), target, None)?;
    Ok(Image::still(target.0, target.1, (width, height), pixels))
}

#[cfg(test)]
#[path = "placeholder_tests.rs"]
mod tests;
