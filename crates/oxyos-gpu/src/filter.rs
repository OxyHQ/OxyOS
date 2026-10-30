//! Filtered sampling of a layer drawn at another size than its pixels'
//! (docs/adr/0011-transitions.mdx, step 5): the integer arithmetic the reference samples with,
//! which `shaders/compose.wgsl` repeats operation for operation so the two agree byte for byte.
//!
//! Along each axis an output pixel averages the source over a box centred where its own centre
//! falls in the source, as wide as the pixel's footprint there and never narrower than one source
//! pixel: drawn smaller, the box is the footprint (area averaging, every source pixel counted by
//! the share of it each output pixel covers); drawn larger, it is one source pixel wide, which is
//! bilinear interpolation between the two nearest pixel centres. Beyond the edges the edge pixels
//! continue (clamp to edge). The two axes' weights multiply; premultiplied channels and alpha are
//! averaged alike, so a pixel stays premultiplied.
//!
//! Positions are in 1/(2·length) of a source pixel, so every edge and centre is a whole number:
//! output pixel `at`'s centre is at `(2·at + 1)·extent`, its box's half width `max(extent,
//! length)`, and source pixel `i` spans `2·i·length` to `2·(i + 1)·length`. A pixel's weight is its
//! overlap with the box, as `ONE`ths of the box: the running total of overlaps is rounded at each
//! pixel's end and each weight is the difference, so an output pixel's weights sum to exactly `ONE`
//! (a flat colour stays exactly that colour, an opaque pixel opaque) and at 1:1 the one pixel under
//! it has all of it.
//!
//! An axis of at least `SNAP` pixels within one pixel of its source's size is sampled by nearest
//! neighbour, the one pixel under each output pixel's corner: the difference is rounding (an
//! output at a fractional scale places a window's edges on whole pixels, so its rectangle may be a
//! pixel narrower than its buffer) or the last frames of a transition, and filtering would soften
//! the whole layer, half of its pixels blended with their neighbours, to hide one row or column.
//! A smaller one a pixel off is really drawn at another size (two pixels into one), and filtered.

/// Bits of an axis's weights.
const WEIGHT_BITS: u32 = 12;
/// An output pixel's weights along an axis sum to this. Two axes' make 2²⁴, and 2²⁴ times a
/// channel's 255 fits in 32 bits, as the shader's arithmetic must: it has no 64-bit integers.
pub(crate) const ONE: u32 = 1 << WEIGHT_BITS;
/// The largest extent and length filtered: every position then fits in 30 bits.
pub const MAX_FILTERED: u32 = 1 << 14;
/// The most source pixels an output pixel's box spans along an axis, so the work of one output
/// pixel stays bounded (at most 65 × 65 source pixels): a tenth of a window at four times the
/// output's scale is 40.
pub const MAX_RATIO: u32 = 64;

/// Whether an axis drawing `extent` source pixels into `length` output pixels is filtered;
/// otherwise it is sampled by nearest neighbour, as without filtering.
pub(crate) fn filters(extent: u32, length: u32) -> bool {
    (1..=MAX_FILTERED).contains(&extent)
        && (1..=MAX_FILTERED).contains(&length)
        && extent <= MAX_RATIO * length
}

/// The shortest axis whose one pixel of difference from its source is rounding, not scaling.
pub const SNAP: u32 = 64;

/// Whether an axis is drawn at another size than its source's beyond rounding: more than a pixel
/// apart, or a pixel apart and shorter than `SNAP`.
pub(crate) fn scaled(extent: u32, length: u32) -> bool {
    extent.abs_diff(length) > 1 || (extent != length && extent.max(length) < SNAP)
}

/// The source pixels one output pixel averages along an axis: `first..=last`, its box's start
/// (`low`, which may be before the first pixel) and half width, in 1/(2·length) source pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Taps {
    pub first: u32,
    pub last: u32,
    low: i32,
    half: u32,
    length: u32,
}

/// The taps of output pixel `at` of an axis drawing `extent` source pixels into `length`
/// (`filters` holds for them).
pub(crate) fn taps(at: u32, length: u32, extent: u32) -> Taps {
    if !scaled(extent, length) {
        // Nearest neighbour: one pixel, all of the weight.
        let source = at * extent / length;
        return Taps {
            first: source,
            last: source,
            low: 0,
            half: 1,
            length,
        };
    }
    let half = extent.max(length);
    let centre = ((2 * at + 1) * extent) as i32;
    let low = centre - half as i32;
    let high = centre + half as i32;
    // The pixels the box overlaps; before the first and after the last, those continue.
    let first = if low > 0 {
        (low as u32 / (2 * length)).min(extent - 1)
    } else {
        0
    };
    let last = ((high - 1) as u32 / (2 * length)).min(extent - 1);
    Taps {
        first,
        last,
        low,
        half,
        length,
    }
}

impl Taps {
    /// The weights of pixels `first..=at` together, in `ONE`ths: the box's part up to the end of
    /// pixel `at` (all of it at the last), rounded to the nearest.
    pub(crate) fn through(&self, at: u32) -> u32 {
        if at == self.last {
            return ONE;
        }
        let end = (2 * (at + 1) * self.length) as i32 - self.low;
        (end as u32 * ONE + self.half) / (2 * self.half)
    }
}

/// The weights of output pixels `from..to` of an axis, each its first source pixel and the
/// weights from there (`Taps::through`'s differences), worked out once for a layer.
pub(crate) struct Axis {
    first: Vec<u32>,
    bounds: Vec<usize>,
    weights: Vec<u32>,
}

impl Axis {
    pub(crate) fn new(from: u32, to: u32, length: u32, extent: u32) -> Self {
        let mut axis = Axis {
            first: Vec::with_capacity(to.saturating_sub(from) as usize),
            bounds: vec![0],
            weights: Vec::new(),
        };
        for at in from..to {
            let taps = taps(at, length, extent);
            let mut before = 0;
            for source in taps.first..=taps.last {
                let through = taps.through(source);
                axis.weights.push(through - before);
                before = through;
            }
            axis.first.push(taps.first);
            axis.bounds.push(axis.weights.len());
        }
        axis
    }

    /// The `index`th output pixel's (from `from`) first source pixel and weights.
    pub(crate) fn get(&self, index: usize) -> (u32, &[u32]) {
        (
            self.first[index],
            &self.weights[self.bounds[index]..self.bounds[index + 1]],
        )
    }
}

/// The filtered sample of `pixels` (`width` a row) for the output pixel with these columns and
/// rows (`Axis::get`): its premultiplied colour (0x00RRGGBB) and alpha. An opaque layer's pixels
/// have alpha 255 whatever their fourth byte.
pub(crate) fn sample(
    pixels: &[u8],
    width: u32,
    (x0, columns): (u32, &[u32]),
    (y0, rows): (u32, &[u32]),
    opaque: bool,
) -> (u32, u32) {
    // Weights times channels: a row's at most `ONE`·255, all rows' at most `ONE`²·255 < 2³².
    let mut sum = [0u32; 4];
    for (y, &wy) in (y0..).zip(rows) {
        let start = (y * width + x0) as usize * 4;
        let row_pixels = &pixels[start..start + columns.len() * 4];
        let mut row = [0u32; 4];
        for (pixel, &wx) in row_pixels.chunks_exact(4).zip(columns) {
            // B, G, R, A in memory.
            let alpha = if opaque { 255 } else { u32::from(pixel[3]) };
            row[0] += wx * u32::from(pixel[2]);
            row[1] += wx * u32::from(pixel[1]);
            row[2] += wx * u32::from(pixel[0]);
            row[3] += wx * alpha;
        }
        for (sum, row) in sum.iter_mut().zip(row) {
            *sum += wy * row;
        }
    }
    let round = |sum: u32| (sum + (1 << (2 * WEIGHT_BITS - 1))) >> (2 * WEIGHT_BITS);
    (
        (round(sum[0]) << 16) | (round(sum[1]) << 8) | round(sum[2]),
        round(sum[3]),
    )
}

/// The output pixels, `start..end` of `length`, whose samples read any of source pixels
/// `from..to` of `extent` along an axis: where a layer's damage (in its pixels) is on the output
/// (docs/adr/0005-damage.mdx). Exact for a filtered layer; at 1:1 it is `from..to`; for a layer
/// sampled by nearest neighbour throughout (`Sampling::Nearest`, or beyond what is filtered) it
/// holds every output pixel that reads them, and may hold one more each side.
pub fn span(from: u32, to: u32, extent: u32, length: u32) -> (u32, u32) {
    if from >= to || extent == 0 || length == 0 {
        return (0, 0);
    }
    // Output pixel `at` reads them when its box, from (2·at + 1)·extent − half to
    // (2·at + 1)·extent + half, overlaps 2·from·length to 2·to·length; the edge pixels' boxes
    // reach as far as they need beyond the edges.
    let nearest = !scaled(extent, length);
    let (from, to) = (i64::from(from), i64::from(to.min(extent)));
    let (extent, length) = (i64::from(extent), i64::from(length));
    if nearest {
        // Output pixel `at` reads source pixel at·extent/length, rounded down.
        let first = |source: i64| ((source * length + extent - 1) / extent).min(length) as u32;
        return (first(from), first(to));
    }
    let half = extent.max(length);
    let before = 2 * from * length - half - extent;
    let start = if before < 0 {
        0
    } else {
        before / (2 * extent) + 1
    };
    let end = (2 * to * length + half - extent + 2 * extent - 1) / (2 * extent);
    let end = end.min(length);
    (start.min(end) as u32, end as u32)
}
