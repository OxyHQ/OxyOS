//! A layer drawn through a mesh (docs/adr/0011-transitions.mdx, step 7: the genie): its pixels
//! warped to a mesh of rows, each a horizontal line of the output from a left to a right end, every
//! row taking the source's whole width at one height of the source. Each quad between two rows is
//! filled bilinearly: an output pixel's centre is mapped back into the source by the inverse of the
//! bilinear map through the quad's four corners, as a Mac's window server fills the Dock's mesh
//! (Salaja measured a bilinear or projective fill within 2 pixels of a recording, and two triangles
//! 4 pixels off). Rows are horizontal, so the inverse is exact and simple: the share down the quad
//! is the pixel's height between the rows', and across it the share between the two sides there.
//!
//! The source is sampled over a box as wide as the output pixel's footprint in it along each axis,
//! never narrower than one source pixel (`filter`'s rule, so a window squeezed into a dock's slot
//! is the average of what it covers, and one drawn larger bilinear), its edges continuing beyond
//! (clamp to edge). The mesh's edges are anti-aliased: a pixel the mesh covers in part is drawn by
//! the part covered. A window's rounded corners (the layer's `radius`, in source pixels here) are
//! the source's, so they bend with it.
//!
//! The arithmetic is integer, in 32 bits, and `shaders/compose.wgsl` repeats it operation for
//! operation, so the CPU reference and the Vulkan composition agree byte for byte (ADR 0004).
//! Positions are in sixteenths of a pixel (`SUB`): the mesh's on the output, relative to the
//! layer's rectangle, and the source's in its own pixels. The share down a quad is in 4096ths.

use crate::shape;

/// A row of a mesh, as a fraction of the layer's rectangle and of its source: its height on the
/// output (`y`, 0 the rectangle's top, 1 its bottom), its two ends (`left`, `right`, 0 the
/// rectangle's left edge, 1 its right) and the height in the source it shows (`source`, 0 the
/// source's top, 1 its bottom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WarpRow {
    pub y: f32,
    pub left: f32,
    pub right: f32,
    pub source: f32,
}

/// A mesh a layer is drawn through: its rows, top to bottom (each `y` at or below the one before,
/// at least two; rows at one height draw nothing between them).
#[derive(Clone, Copy, Debug)]
pub struct Warp<'a> {
    pub rows: &'a [WarpRow],
}

/// The most rows a mesh has: the Dock's function keeps at most 400.
pub const MAX_ROWS: usize = 400;
/// Sixteenths of a pixel.
pub(crate) const SUB: i32 = 16;
/// Bits of the share down a quad.
const SHARE_BITS: u32 = 12;
/// The widest half box, in sixteenths of a source pixel: `filter::MAX_RATIO` source pixels to an
/// output pixel.
const MAX_HALF: i32 = SUB / 2 * crate::filter::MAX_RATIO as i32;
/// Weights along an axis sum to this (`filter::ONE`).
const ONE: u32 = crate::filter::ONE;

/// One row in the layer's pixels: its height and ends on the output, relative to the layer's
/// rectangle, and its height in the source, each in sixteenths of a pixel.
pub(crate) type Fixed = [i32; 4];

impl Warp<'_> {
    /// Whether it can be drawn: two rows or more and at most `MAX_ROWS`, every value finite and
    /// within reach, the rows in order and their ends in order.
    pub(crate) fn valid(&self) -> bool {
        (2..=MAX_ROWS).contains(&self.rows.len())
            && self.rows.iter().all(|row| {
                [row.y, row.left, row.right, row.source]
                    .iter()
                    .all(|v| v.is_finite() && (-0.001..=1.001).contains(v))
                    && row.left <= row.right
            })
            && self.rows.windows(2).all(|pair| pair[0].y <= pair[1].y)
    }

    /// The rows in the layer's pixels (`width` × `height`, its source `extent`), in sixteenths:
    /// worked out once, the same for the CPU and the device.
    pub(crate) fn fixed(&self, width: u32, height: u32, extent: (u32, u32)) -> Vec<Fixed> {
        let at = |share: f32, length: u32| {
            (f64::from(share.clamp(0.0, 1.0)) * f64::from(length) * f64::from(SUB)).round() as i32
        };
        self.rows
            .iter()
            .map(|row| {
                [
                    at(row.y, height),
                    at(row.left, width),
                    at(row.right, width),
                    at(row.source, extent.1),
                ]
            })
            .collect()
    }
}

/// Where an output pixel falls through the mesh: how much of it the mesh covers (0 to 255), where
/// its centre is in the source (sixteenths of a source pixel), and the half widths of its box
/// there along each axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fall {
    pub coverage: u32,
    pub x: i32,
    pub y: i32,
    pub half_x: i32,
    pub half_y: i32,
}

/// One output row of a layer drawn through a mesh: how much of its height the mesh covers, the
/// two sides there (sixteenths, relative to the layer), how many source sixteenths an output
/// sixteenth across is (in 4096ths), the source height its centre shows and the half widths of a
/// pixel's box along each axis. All of a pixel's mapping but where it is across: worked out once
/// for the row on the CPU, and per pixel in the shader, with the same arithmetic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    covered_y: i32,
    left: i32,
    right: i32,
    rate: u32,
    y: i32,
    half_x: i32,
    half_y: i32,
}

/// Output row `ly` of a layer drawn through `rows` (`Warp::fixed`), its source `extent`: none
/// where the mesh covers nothing of it.
pub(crate) fn span(rows: &[Fixed], ly: u32, extent: (u32, u32)) -> Option<Span> {
    let count = rows.len();
    let (top, bottom) = (rows[0][0], rows[count - 1][0]);
    let centre_y = ly as i32 * SUB + SUB / 2;
    // The share of the pixel's height within the mesh's.
    let covered_y =
        ((centre_y + SUB / 2).min(bottom) - (centre_y - SUB / 2).max(top)).clamp(0, SUB);
    if covered_y == 0 {
        return None;
    }
    // The quad holding the pixel's centre (taken within the mesh): the last row at or above it.
    let y = centre_y.clamp(top, bottom - 1);
    let (mut upper, mut lower) = (0, count - 1);
    while lower - upper > 1 {
        let middle = (upper + lower) / 2;
        if rows[middle][0] <= y {
            upper = middle;
        } else {
            lower = middle;
        }
    }
    let (a, b) = (rows[upper], rows[lower]);
    let down = (b[0] - a[0]) as u32;
    let share = (((y - a[0]) as u32) << SHARE_BITS) / down;
    let between = |from: i32, to: i32| {
        from + (((to - from) * share as i32 + (1 << (SHARE_BITS - 1))) >> SHARE_BITS)
    };
    let (left, right) = (between(a[1], b[1]), between(a[2], b[2]));
    if right <= left {
        return None;
    }
    let across = (right - left) as u32;
    // Source sixteenths per output sixteenth, in 4096ths: at most the source's width · 2¹⁶.
    let rate = (extent.0 << 16) / across;
    // A pixel's footprint is a source sixteenth's rate times 16 along x, the quad's source height
    // over its height along y; half of it, at least half a source pixel.
    Some(Span {
        covered_y,
        left,
        right,
        rate,
        y: between(a[3], b[3]),
        half_x: ((rate >> 9) as i32).clamp(SUB / 2, MAX_HALF),
        half_y: ((b[3] - a[3]) * (SUB / 2) / down as i32).clamp(SUB / 2, MAX_HALF),
    })
}

impl Span {
    /// The output pixels of the row the mesh covers any of: from `start` to before `end`.
    pub(crate) fn pixels(&self) -> (i32, i32) {
        (self.left / SUB, (self.right + SUB - 1) / SUB)
    }

    /// The source height its pixels' centres show, and their boxes' half height there.
    pub(crate) fn source_y(&self) -> i32 {
        self.y
    }

    pub(crate) fn half_y(&self) -> i32 {
        self.half_y
    }

    /// Where the row's pixel `lx` falls: none where the mesh does not cover it.
    pub(crate) fn fall(&self, lx: u32) -> Option<Fall> {
        let centre_x = lx as i32 * SUB + SUB / 2;
        let covered_x = ((centre_x + SUB / 2).min(self.right)
            - (centre_x - SUB / 2).max(self.left))
        .clamp(0, SUB);
        if covered_x == 0 {
            return None;
        }
        let x = ((((centre_x.clamp(self.left, self.right) - self.left) as u32) * self.rate)
            >> SHARE_BITS) as i32;
        Some(Fall {
            coverage: ((covered_x * self.covered_y) as u32 * 255 + 128) >> 8,
            x,
            y: self.y,
            half_x: self.half_x,
            half_y: self.half_y,
        })
    }
}

/// The source pixels a box centred at `centre` with half width `half` (sixteenths) averages along
/// an axis `extent` pixels long: the first and last, and the box's start.
pub(crate) fn taps(centre: i32, half: i32, extent: u32) -> (u32, u32, i32) {
    let low = centre - half;
    let high = centre + half;
    let first = if low > 0 {
        (low as u32 / SUB as u32).min(extent - 1)
    } else {
        0
    };
    let last = ((high - 1).max(0) as u32 / SUB as u32).min(extent - 1);
    (first, last, low)
}

/// The weights of pixels `first..=at` together, in `ONE`ths: the box's part up to pixel `at`'s
/// end, all of it at the last.
pub(crate) fn through(at: u32, last: u32, low: i32, half: i32) -> u32 {
    if at == last {
        return ONE;
    }
    let end = (at as i32 + 1) * SUB - low;
    (end as u32 * ONE + half as u32) / (2 * half as u32)
}

/// The source rows a sample centred at `centre` with half height `half` averages, and their
/// weights (`through`'s differences): the same for every pixel of an output row.
pub(crate) fn weights(centre: i32, half: i32, extent: u32) -> (u32, Vec<u32>) {
    let (first, last, low) = taps(centre, half, extent);
    let mut before = 0;
    let weights = (first..=last)
        .map(|at| {
            let to = through(at, last, low, half);
            let weight = to - before;
            before = to;
            weight
        })
        .collect();
    (first, weights)
}

/// The source's columns blended down the rows one output row reads (`weights`): for each column,
/// every channel's Σ weight · channel over those rows (alpha 255 for an opaque source), exactly. A
/// sample of the row is then Σ weight · column across (`sample_columns`): the shader's sum,
/// Σ rows (weight · Σ columns (weight · channel)), in the other order, which integers keep equal
/// (and below 2³², `filter`'s bound), so every pixel of the row stops reading each source row.
pub(crate) fn blend_rows(
    pixels: &[u8],
    extent: (u32, u32),
    opaque: bool,
    (y0, rows): &(u32, Vec<u32>),
    columns: &mut Vec<[u32; 4]>,
) {
    columns.clear();
    columns.resize(extent.0 as usize, [0; 4]);
    for (y, &wy) in (*y0..).zip(rows) {
        let start = (y * extent.0) as usize * 4;
        let row = &pixels[start..start + extent.0 as usize * 4];
        for (column, pixel) in columns.iter_mut().zip(row.chunks_exact(4)) {
            // B, G, R, A in memory.
            let alpha = if opaque { 255 } else { u32::from(pixel[3]) };
            column[0] += wy * u32::from(pixel[2]);
            column[1] += wy * u32::from(pixel[1]);
            column[2] += wy * u32::from(pixel[0]);
            column[3] += wy * alpha;
        }
    }
}

/// The sample where `fall` says, from its row's blended columns (`blend_rows`): its premultiplied
/// colour (0x00RRGGBB) and alpha.
pub(crate) fn sample_columns(columns: &[[u32; 4]], fall: &Fall) -> (u32, u32) {
    let (x0, x1, x_low) = taps(fall.x, fall.half_x, columns.len() as u32);
    let mut sum = [0u32; 4];
    let mut before = 0;
    for x in x0..=x1 {
        let to_x = through(x, x1, x_low, fall.half_x);
        let wx = to_x - before;
        before = to_x;
        for (sum, channel) in sum.iter_mut().zip(columns[x as usize]) {
            *sum += wx * channel;
        }
    }
    let round = |sum: u32| (sum + (1 << 23)) >> 24;
    (
        (round(sum[0]) << 16) | (round(sum[1]) << 8) | round(sum[2]),
        round(sum[3]),
    )
}

/// How much of a source's rounded corners (`radius` source pixels) leave of the source pixel the
/// sample is centred in.
pub(crate) fn corner(fall: &Fall, extent: (u32, u32), radius: u32) -> u32 {
    if radius == 0 {
        return 255;
    }
    let pixel = |at: i32, length: u32| ((at.max(0) / SUB) as u32).min(length - 1);
    shape::corner_coverage(
        pixel(fall.x, extent.0),
        pixel(fall.y, extent.1),
        extent.0,
        extent.1,
        radius,
    )
}
