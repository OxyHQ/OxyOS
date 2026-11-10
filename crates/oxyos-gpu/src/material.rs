//! The platform's material (docs/adr/0069-the-platforms-material.mdx): what the compositor has
//! composed beneath a surface's material region, blurred and saturated, before the surface is
//! drawn over it, as a Mac's vibrancy. The integer arithmetic the reference composes it with,
//! which the compositor's shader is to repeat operation for operation, as `filter` and `shape`
//! are, so the two agree byte for byte.
//!
//! The blur is a Gaussian of σ output pixels, approximated as the W3C's Filter Effects do (three
//! box blurs per axis), on the output downsampled by 4:
//!
//! 1. The backdrop (the layers beneath, composed afresh over the region and its reach) is averaged
//!    over 4 × 4 cells aligned to the output's own grid, so a cell's value depends only on the
//!    pixels under it, never on where the region is.
//! 2. Each axis of the cells is blurred by three box passes of the width `box_width(σ / 4)` gives:
//!    of that width, centred, three times when it is odd; two of it (the first's centre half a
//!    cell left, the second's right) and one a cell wider, centred, when it is even. Beyond the
//!    source's cells the edge cell continues (clamp to edge).
//! 3. Each output pixel of the region takes its value bilinearly from the four cells nearest its
//!    centre, in eighths of a cell along each axis.
//! 4. The colour is saturated × 1.8 by the luminance-preserving matrix with Rec. 709's weights, in
//!    4096ths, each row summing to 4096 so a grey stays that grey.
//! 5. It replaces what is beneath by the region's rounded corners' coverage (`shape`) and the
//!    surface's opacity.
//!
//! The region's pixels depend on the backdrop within `reach(σ)` of them and on nothing else, so
//! composing the backdrop over the region grown by that reach gives every one exactly; the
//! compositor damages a region whenever what is beneath it changes within that reach.

use crate::shape::{corner_coverage, radius_on};
use crate::Rect;

/// The downsampling: cells of `CELL` × `CELL` output pixels.
pub const CELL: u32 = 4;
/// The saturation matrix, in 4096ths: Rec. 709's luminance (0.2126, 0.7152, 0.0722) kept and the
/// rest scaled by 1.8, each row rounded so it sums to 4096.
const SATURATION: [[i32; 3]; 3] = [[6677, -2344, -237], [-697, 5030, -237], [-697, -2344, 7137]];
/// The widest σ drawn, in output pixels; a larger one is drawn at this.
pub const MAX_SIGMA: u32 = 256;

/// A material region as the compositor draws it: its rounded rectangle on the output, its blur's σ
/// in output pixels, and the opacity it is drawn at (the surface's, 0 to 255).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialArea {
    pub rect: Rect,
    pub radius: u32,
    pub sigma: u32,
    pub opacity: u8,
}

/// The width, in cells, of each box pass for a Gaussian of `sigma` output pixels (σ / 4 cells):
/// ⌊(σ / 4) · 3√(2π) / 4 + ½⌋, at least 1, 3√(2π)/4 (1.87997) taken as 18800/10000.
pub fn box_width(sigma: u32) -> u32 {
    let sigma = sigma.min(MAX_SIGMA);
    // σ / CELL cells times 1.88, rounded: (σ · 18800 / 4 + 5000) / 10000.
    ((sigma * 18_800 / CELL + 5_000) / 10_000).max(1)
}

/// How far, in output pixels, the backdrop reaches into a region's pixels: the three passes'
/// half widths and the bilinear neighbour, in cells, and a cell for the grid's alignment.
pub fn reach(sigma: u32) -> u32 {
    let d = box_width(sigma);
    // Each pass reaches at most d/2 cells to either side; one more for the bilinear neighbour, and
    // one each for rounding and the grid's alignment.
    (3 * (d / 2 + 1) + 2) * CELL
}

/// The source a region's blur is made from: the region grown by its reach and aligned outward to
/// the cells' grid, within the output (`width` × `height`); none for a region off the output.
pub fn source(area: &MaterialArea, width: u32, height: u32) -> Option<Rect> {
    let output = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let shown = area.rect.intersect(&output)?;
    let grow = i64::from(reach(area.sigma));
    let cell = i64::from(CELL);
    let left = ((i64::from(shown.x) - grow).max(0) / cell) * cell;
    let top = ((i64::from(shown.y) - grow).max(0) / cell) * cell;
    let right = (shown.right() + grow).min(i64::from(width));
    let bottom = (shown.bottom() + grow).min(i64::from(height));
    let right = (right.div_ceil(cell) * cell).min(i64::from(width));
    let bottom = (bottom.div_ceil(cell) * cell).min(i64::from(height));
    Some(Rect {
        x: left as i32,
        y: top as i32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
}

/// One box pass of `width` cells along a line of `values` (one channel): centred when `width` is
/// odd; when even, its extra cell on the left (`left_heavy`, the W3C's first pass, centred half a
/// cell left) or the right (the second). The line's edge cells continue beyond it.
fn box_pass(values: &[i32], width: u32, left_heavy: bool) -> Vec<i32> {
    let n = values.len() as i64;
    let w = i64::from(width);
    // The box covers [i - before, i + after]: centred when odd, the extra cell left or right when
    // even.
    let before = if w % 2 == 1 || left_heavy {
        w / 2
    } else {
        w / 2 - 1
    };
    let after = w - 1 - before;
    let at = |i: i64| values[i.clamp(0, n - 1) as usize];
    let mut sum: i64 = (-before..=after).map(|i| i64::from(at(i))).sum();
    let mut out = Vec::with_capacity(values.len());
    for i in 0..n {
        out.push(((sum + w / 2) / w) as i32);
        sum += i64::from(at(i + after + 1)) - i64::from(at(i - before));
    }
    out
}

/// The three passes along one line.
fn blur_line(values: &[i32], width: u32) -> Vec<i32> {
    if width <= 1 {
        return values.to_vec();
    }
    if width % 2 == 1 {
        let once = box_pass(values, width, true);
        let twice = box_pass(&once, width, true);
        box_pass(&twice, width, true)
    } else {
        let left = box_pass(values, width, true);
        let right = box_pass(&left, width, false);
        box_pass(&right, width + 1, true)
    }
}

/// The blurred, saturated backdrop over `area`, from `backdrop` (the whole output, `width` wide,
/// holding the layers beneath composed over `source`): 0x00RRGGBB per pixel of the area's part on
/// the output, row by row, and that part.
pub fn blurred(
    backdrop: &[u32],
    width: u32,
    height: u32,
    area: &MaterialArea,
) -> Option<(Rect, Vec<u32>)> {
    let src = source(area, width, height)?;
    let output = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let shown = area.rect.intersect(&output)?;
    // 1. Cells: each the rounded average of the pixels under it (fewer at the output's edge).
    let cols = src.width.div_ceil(CELL) as usize;
    let rows = src.height.div_ceil(CELL) as usize;
    let mut cells = [
        vec![0i32; cols * rows],
        vec![0i32; cols * rows],
        vec![0i32; cols * rows],
    ];
    for cy in 0..rows {
        for cx in 0..cols {
            let x0 = src.x as u32 + cx as u32 * CELL;
            let y0 = src.y as u32 + cy as u32 * CELL;
            let x1 = (x0 + CELL).min(src.x as u32 + src.width);
            let y1 = (y0 + CELL).min(src.y as u32 + src.height);
            let count = ((x1 - x0) * (y1 - y0)) as i32;
            let mut sums = [0i32; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    let pixel = backdrop[(y * width + x) as usize];
                    sums[0] += ((pixel >> 16) & 0xff) as i32;
                    sums[1] += ((pixel >> 8) & 0xff) as i32;
                    sums[2] += (pixel & 0xff) as i32;
                }
            }
            for (channel, sum) in cells.iter_mut().zip(sums) {
                channel[cy * cols + cx] = (sum + count / 2) / count;
            }
        }
    }
    // 2. Three box passes along rows, then along columns.
    let d = box_width(area.sigma);
    for channel in cells.iter_mut() {
        for cy in 0..rows {
            let line = blur_line(&channel[cy * cols..(cy + 1) * cols], d);
            channel[cy * cols..(cy + 1) * cols].copy_from_slice(&line);
        }
        for cx in 0..cols {
            let column: Vec<i32> = (0..rows).map(|cy| channel[cy * cols + cx]).collect();
            for (cy, value) in blur_line(&column, d).into_iter().enumerate() {
                channel[cy * cols + cx] = value;
            }
        }
    }
    // 3. Bilinear, in eighths of a cell: pixel k of a cell (0..4) has its centre (2k − 3)/8 of a
    // cell from that cell's centre.
    let sample = |channel: &[i32], px: u32, py: u32| -> i32 {
        let axis = |p: u32, origin: u32, count: usize| -> (usize, usize, i32) {
            let local = p - origin;
            let cell = (local / CELL) as i64;
            let offset = 2 * (local % CELL) as i64 - 3; // eighths, −3 ..= 3
            let (a, b, wb) = if offset < 0 {
                (cell - 1, cell, 8 + offset)
            } else {
                (cell, cell + 1, offset)
            };
            let clamp = |c: i64| c.clamp(0, count as i64 - 1) as usize;
            (clamp(a), clamp(b), wb as i32)
        };
        let (x0, x1, wx) = axis(px, src.x as u32, cols);
        let (y0, y1, wy) = axis(py, src.y as u32, rows);
        let top = channel[y0 * cols + x0] * (8 - wx) + channel[y0 * cols + x1] * wx;
        let bottom = channel[y1 * cols + x0] * (8 - wx) + channel[y1 * cols + x1] * wx;
        (top * (8 - wy) + bottom * wy + 32) / 64
    };
    // 4. Saturation.
    let mut out = Vec::with_capacity(shown.width as usize * shown.height as usize);
    for y in shown.y as u32..shown.y as u32 + shown.height {
        for x in shown.x as u32..shown.x as u32 + shown.width {
            let rgb = [
                sample(&cells[0], x, y),
                sample(&cells[1], x, y),
                sample(&cells[2], x, y),
            ];
            let mut pixel = 0u32;
            for row in SATURATION {
                let value = (row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2] + 2048) >> 12;
                pixel = (pixel << 8) | value.clamp(0, 255) as u32;
            }
            out.push(pixel);
        }
    }
    Some((shown, out))
}

/// 5. Draws `blurred` (as `blurred` gives it) onto `out` (the output, `width` wide) within
/// `within`, by the area's rounded corners and opacity, over what is there.
pub fn draw(
    out: &mut [u32],
    width: u32,
    area: &MaterialArea,
    shown: Rect,
    blurred: &[u32],
    within: Rect,
) {
    let Some(clip) = shown.intersect(&within) else {
        return;
    };
    let radius = radius_on(area.radius, area.rect.width, area.rect.height);
    for y in clip.y..clip.y + clip.height as i32 {
        for x in clip.x..clip.x + clip.width as i32 {
            let lx = (x - area.rect.x) as u32;
            let ly = (y - area.rect.y) as u32;
            let coverage = corner_coverage(lx, ly, area.rect.width, area.rect.height, radius);
            let alpha = coverage * u32::from(area.opacity) / 255;
            if alpha == 0 {
                continue;
            }
            let from =
                blurred[((y - shown.y) as u32 * shown.width + (x - shown.x) as u32) as usize];
            let index = (y as u32 * width + x as u32) as usize;
            let under = out[index];
            let mut pixel = 0u32;
            for shift in [16, 8, 0] {
                let a = (from >> shift) & 0xff;
                let b = (under >> shift) & 0xff;
                pixel |= ((a * alpha + b * (255 - alpha) + 127) / 255) << shift;
            }
            out[index] = pixel;
        }
    }
}

#[cfg(test)]
mod tests;
