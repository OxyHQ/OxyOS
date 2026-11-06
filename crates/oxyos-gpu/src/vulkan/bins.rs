//! Which layers each part of the output takes (`shaders/compose.wgsl`'s `bins`), worked out on the
//! CPU from the layer table before a frame is composed: the output in bins of [`BIN`] pixels, each
//! with a bit for every layer whose part drawn touches it, bottom to top, less those under the
//! topmost layer covering the whole bin with an opaque copy or a fill at full opacity (never seen
//! there). A pixel takes only its bin's layers, so a frame's cost is what each part of it shows,
//! not every layer at every pixel; and the shader decides nothing per tile, which on a device that
//! waits on memory (Intel's, at its efficient clock) costs as much as composing.
//!
//! A workgroup composes a tile of [`TILE`] pixels on the output's grid of tiles, which a bin holds
//! whole: the regions composed are put on that grid first ([`on_tiles`]), every pixel of a tile
//! composed (a pixel composed again is the same pixel), and a row of a tile is a whole cache line
//! of the output.

use super::Push;
use crate::Rect;

/// The pixels a workgroup composes: `TILE_WIDTH` × `TILE_HEIGHT` in the shader.
pub(super) const TILE: (u32, u32) = (16, 8);

/// The pixels of a bin: `BIN_WIDTH` × `BIN_HEIGHT` in the shader, a multiple of a tile's.
pub(super) const BIN: (u32, u32) = (32, 32);

/// Each bin's layers, a bit each, row by row of bins across the output.
pub(super) struct Bins {
    /// Bins in a row of the output.
    pub across: u32,
    /// Words of each bin's bits: one for every 32 layers.
    pub words: u32,
    pub bits: Vec<u32>,
}

impl Bins {
    /// The words the bins of a `width` × `height` output with `layers` layers take, at most.
    pub fn words_for(width: u32, height: u32, layers: usize) -> usize {
        width.div_ceil(BIN.0) as usize
            * height.div_ceil(BIN.1) as usize
            * (layers as u32).div_ceil(32) as usize
    }

    /// The bins of a `width` × `height` output for `table` (bottom to top), worked out within
    /// `regions` alone (the others are never read).
    pub fn new(table: &[Push], width: u32, height: u32, regions: &[Rect]) -> Self {
        let across = width.div_ceil(BIN.0);
        let down = height.div_ceil(BIN.1);
        let words = (table.len() as u32).div_ceil(32);
        let count = (across * down) as usize;
        let mut bits = vec![0u32; count * words as usize];
        if words == 0 {
            return Self {
                across,
                words,
                bits,
            };
        }
        // The bins `rect` (output pixels) touches, if any: columns and rows of bins.
        let span = |rect: &Rect| -> Option<(u32, u32, u32, u32)> {
            let left = i64::from(rect.x).max(0);
            let top = i64::from(rect.y).max(0);
            let right = (i64::from(rect.x) + i64::from(rect.width)).min(i64::from(width));
            let bottom = (i64::from(rect.y) + i64::from(rect.height)).min(i64::from(height));
            (left < right && top < bottom).then(|| {
                (
                    (left / i64::from(BIN.0)) as u32,
                    ((right - 1) / i64::from(BIN.0)) as u32,
                    (top / i64::from(BIN.1)) as u32,
                    ((bottom - 1) / i64::from(BIN.1)) as u32,
                )
            })
        };
        let mut wanted = vec![false; count];
        for (left, right, top, bottom) in regions.iter().filter_map(span) {
            for row in top..=bottom {
                for column in left..=right {
                    wanted[(row * across + column) as usize] = true;
                }
            }
        }
        // One past the topmost layer covering each bin whole.
        let mut covered = vec![0u32; count];
        for (index, push) in table.iter().enumerate() {
            let drawn = Rect {
                x: push.x + push.base_x as i32,
                y: push.y + push.base_y as i32,
                width: push.count_x,
                height: push.count_y,
            };
            let Some((left, right, top, bottom)) = span(&drawn) else {
                continue;
            };
            let (word, bit) = (index / 32, 1u32 << (index % 32));
            for row in top..=bottom {
                for column in left..=right {
                    let at = (row * across + column) as usize;
                    if !wanted[at] {
                        continue;
                    }
                    bits[at * words as usize + word] |= bit;
                    let bin = Rect {
                        x: (column * BIN.0) as i32,
                        y: (row * BIN.1) as i32,
                        width: BIN.0.min(width - column * BIN.0),
                        height: BIN.1.min(height - row * BIN.1),
                    };
                    if covers(push, &drawn, &bin) {
                        covered[at] = index as u32 + 1;
                    }
                }
            }
        }
        for (at, &covered) in covered.iter().enumerate() {
            let Some(lowest) = covered.checked_sub(1) else {
                continue;
            };
            let bin = &mut bits[at * words as usize..(at + 1) * words as usize];
            for (word, value) in bin.iter_mut().enumerate() {
                let low = word as u32 * 32;
                if lowest >= low + 32 {
                    *value = 0;
                } else if lowest > low {
                    *value &= !((1u32 << (lowest - low)) - 1);
                }
            }
        }
        Self {
            across,
            words,
            bits,
        }
    }
}

/// Whether `push`, its part drawn `drawn`, covers every pixel of `bin` whole, opaque, at full
/// opacity, so that each takes its colour whatever is under it (the shader's `blend` of an opaque
/// copy or a fill at 255 is its pixel): within the part drawn, and on none of its rounded corners
/// (`corner_coverage` is 255 wherever the column or the row is between them).
fn covers(push: &Push, drawn: &Rect, bin: &Rect) -> bool {
    let opaque = matches!(push.mode, 0 | 2) && push.opacity == 255 && push.warp_rows == 0;
    if !opaque || drawn.intersect(bin) != Some(*bin) {
        return false;
    }
    let radius = i64::from(push.radius);
    let (left, top) = (
        i64::from(bin.x) - i64::from(push.x),
        i64::from(bin.y) - i64::from(push.y),
    );
    let (right, bottom) = (left + i64::from(bin.width), top + i64::from(bin.height));
    let columns = left >= radius && right <= i64::from(push.width) - radius;
    let rows = top >= radius && bottom <= i64::from(push.height) - radius;
    columns || rows
}

/// `regions` (within `output`) grown to the tiles' grid and made disjoint again: every tile a
/// workgroup composes is within one region, and within one bin.
pub(super) fn on_tiles(regions: &[Rect], output: &Rect) -> Vec<Rect> {
    let grown: Vec<Rect> = regions
        .iter()
        .filter_map(|rect| rect.intersect(output))
        .map(|rect| {
            let left = rect.x.div_euclid(TILE.0 as i32) * TILE.0 as i32;
            let top = rect.y.div_euclid(TILE.1 as i32) * TILE.1 as i32;
            // Within the output, so not negative.
            let right = (rect.right() as u64).div_ceil(u64::from(TILE.0)) * u64::from(TILE.0);
            let bottom = (rect.bottom() as u64).div_ceil(u64::from(TILE.1)) * u64::from(TILE.1);
            Rect {
                x: left,
                y: top,
                width: (right - left as u64) as u32,
                height: (bottom - top as u64) as u32,
            }
        })
        .collect();
    crate::disjoint(&grown, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(x: i32, y: i32, width: u32, height: u32, opacity: u32, radius: u32) -> Push {
        Push {
            x,
            y,
            width,
            height,
            mode: 2,
            opacity,
            radius,
            ..Push::default()
        }
        .whole()
    }

    fn whole(width: u32, height: u32) -> Vec<Rect> {
        vec![Rect {
            x: 0,
            y: 0,
            width,
            height,
        }]
    }

    /// The bits of bin (`column`, `row`).
    fn bin(bins: &Bins, column: u32, row: u32) -> Vec<u32> {
        let at = ((row * bins.across + column) * bins.words) as usize;
        bins.bits[at..at + bins.words as usize].to_vec()
    }

    #[test]
    fn a_bin_takes_the_layers_touching_it_from_the_topmost_covering_it() {
        // A background, a window with rounded corners over it, a translucent bar over both.
        let table = [
            fill(0, 0, 128, 96, 255, 0),
            fill(16, 16, 96, 64, 255, 12),
            fill(0, 0, 128, 20, 128, 0),
        ];
        let bins = Bins::new(&table, 128, 96, &whole(128, 96));
        assert_eq!((bins.across, bins.words), (4, 1));
        // The window's corner is in bin (0, 0): the background stays under it; the bar touches it.
        assert_eq!(bin(&bins, 0, 0), [0b111]);
        // Bin (1, 1) is within the window off its corners: the background is never seen there.
        assert_eq!(bin(&bins, 1, 1), [0b010]);
        // Bin (0, 2), rows 64 to 96: the window's bottom left corner, and the background.
        assert_eq!(bin(&bins, 0, 2), [0b011]);
        // Bin (3, 2) touches the window's corner (x 96 to 112, y 64 to 80) too.
        assert_eq!(bin(&bins, 3, 2), [0b011]);
        // A layer at less than full opacity covers nothing.
        let faded = [fill(0, 0, 128, 96, 255, 0), fill(0, 0, 128, 96, 254, 0)];
        assert_eq!(bin(&Bins::new(&faded, 128, 96, &whole(128, 96)), 1, 1), [0b11]);
    }

    #[test]
    fn bins_hold_any_number_of_layers_and_only_where_composed() {
        // 40 layers of a pixel each along a row, and one covering everything at 35.
        let mut table: Vec<Push> = (0..40).map(|i| fill(i * 3, 0, 1, 1, 255, 0)).collect();
        table[35] = fill(0, 0, 64, 64, 255, 0);
        let region = [Rect {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        }];
        let bins = Bins::new(&table, 64, 64, &region);
        assert_eq!(bins.words, 2);
        // Bin (0, 0): layers 35 and those over it touching it (36 to 39 are at x 108 and on).
        assert_eq!(bin(&bins, 0, 0), [0, 1 << 3]);
        // Outside the region composed, nothing is worked out.
        assert_eq!(bin(&bins, 1, 0), [0, 0]);
    }

    #[test]
    fn regions_grow_to_the_tiles_grid_within_the_output() {
        let output = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 60,
        };
        let regions = on_tiles(
            &[
                Rect {
                    x: 17,
                    y: 9,
                    width: 2,
                    height: 2,
                },
                Rect {
                    x: 20,
                    y: 12,
                    width: 30,
                    height: 2,
                },
                Rect {
                    x: 95,
                    y: 55,
                    width: 20,
                    height: 20,
                },
            ],
            &output,
        );
        assert_eq!(
            regions,
            [
                Rect {
                    x: 16,
                    y: 8,
                    width: 48,
                    height: 8,
                },
                Rect {
                    x: 80,
                    y: 48,
                    width: 20,
                    height: 12,
                },
            ]
        );
    }
}
