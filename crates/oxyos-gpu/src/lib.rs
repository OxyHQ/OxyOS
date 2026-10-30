//! Composition of a frame's layers (docs/adr/0004-gpu-composition.mdx): the reference on the
//! CPU, [`compose_cpu`], and the same composition on a Vulkan device, [`VulkanCompositor`],
//! which must agree with it byte for byte.
//!
//! The workspace forbids `unsafe`; this crate denies it and allows it only on the Vulkan items,
//! each with the contract it relies on.

use std::ffi::CStr;

/// What a layer puts on the output.
#[derive(Clone, Copy, Debug)]
pub enum Paint<'a> {
    /// XRGB8888 pixels (B, G, R, X in memory), copied.
    Opaque(&'a [u8]),
    /// Premultiplied ARGB8888 pixels (B, G, R, A in memory), blended source-over.
    Premultiplied(&'a [u8]),
    /// One colour, 0x00RRGGBB.
    Fill(u32),
    /// A shadow (docs/adr/0016-windows.mdx): its profile's premultiplied pixels
    /// ([`ShadowProfile`]), each output pixel the profile's for its distance from the shape
    /// casting it, never within what it is beneath.
    Shadow(&'a [u8], Shadow),
}

/// Where a shadow is, relative to its layer's rectangle (output pixels): the rounded rectangle
/// casting it, and the one it is beneath (a window), within which none of it is drawn and at
/// whose edge only the part outside; and the size it is drawn at, in 256ths of its profile's (a
/// window drawn smaller, in a transition, casts a smaller blur).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shadow {
    pub shape: Rect,
    pub radius: u32,
    pub clip: Rect,
    pub clip_radius: u32,
    pub scale: u32,
}

impl Shadow {
    /// Whether its arithmetic stays in range: rectangles within 2²⁰ pixels and a scale the
    /// profile may be drawn at.
    fn valid(&self) -> bool {
        let within = |rect: &Rect| {
            rect.x.unsigned_abs() <= 1 << 20
                && rect.y.unsigned_abs() <= 1 << 20
                && rect.width <= 1 << 20
                && rect.height <= 1 << 20
        };
        within(&self.shape) && within(&self.clip) && (1..=MAX_SHADOW_SCALE).contains(&self.scale)
    }
}

/// One layer of a frame, bottom to top: its rectangle on the output in logical pixels (which
/// may extend past the output) and its pixels at `extent`, scaled to the rectangle by nearest
/// neighbour or filtered ([`Filter`]), at an opacity (a surface's transition,
/// docs/adr/0011-transitions.mdx), with its corners rounded (a window's,
/// docs/adr/0016-windows.mdx), within a clip (a window's live view in the shell,
/// docs/adr/0022-window-views.mdx).
#[derive(Clone, Copy, Debug)]
pub struct Layer<'a> {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub extent: (u32, u32),
    pub paint: Paint<'a>,
    /// Where the pixels come from, when they may be the same as an earlier frame's.
    pub source: Option<Source>,
    /// What changed in the pixels (in their own coordinates) since an earlier version of their
    /// source: where the device holds that version, only these rows are uploaded.
    pub changed: Option<(u64, &'a [Rect])>,
    /// 0 (nothing) to 255 (as its pixels are): the premultiplied pixels are scaled by it before
    /// they are blended.
    pub opacity: u8,
    /// Its corners' radius, in the rectangle's pixels (at most half its shorter side, and
    /// `shape::MAX_RADIUS`): pixels outside the rounded rectangle are not drawn, and those on its
    /// edge by the part of them inside, scaling the opacity. 0: square, as without it.
    pub radius: u32,
    /// How its pixels are sampled where it is drawn at another size than theirs.
    pub filter: Filter,
    /// The part of the output it is drawn in, if not all of it: nothing outside is touched.
    pub clip: Option<Rect>,
}

/// How a layer drawn at another size than its pixels' samples them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    /// The pixel under each output pixel's corner: exact for a surface at its own size, and
    /// what a frame file from before filtering was composed with.
    #[default]
    Nearest,
    /// Filtered (`filter`, docs/adr/0011-transitions.mdx, step 5): along each axis, a box as
    /// wide as the output pixel's footprint and never narrower than one source pixel. Drawn
    /// smaller, each output pixel is the average of the source pixels it covers, each by the share
    /// of it covered; drawn larger, bilinear; at 1:1, the pixels as they are. Premultiplied
    /// pixels are averaged with their alpha. Every surface the compositor draws is filtered: a
    /// window arriving, going, minimized or zoomed (ADR 0011, ADR 0016), shown in the overview
    /// (docs/adr/0022-window-views.mdx), on an output at another scale. A layer with an axis
    /// beyond `filter::MAX_RATIO` or `filter::MAX_FILTERED` is sampled by nearest neighbour, and
    /// so is an axis of `filter::SNAP` pixels or more within one of its source's.
    Box,
}

/// The identity of a layer's pixels: the same key and version are the same pixels, so the
/// Vulkan compositor keeps them on the device and uploads them once (docs/adr/0005-damage.mdx).
/// A new version of a key replaces its pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub key: u64,
    pub version: u64,
}

impl Layer<'_> {
    fn pixels(&self) -> Option<&[u8]> {
        match self.paint {
            Paint::Opaque(pixels) | Paint::Premultiplied(pixels) | Paint::Shadow(pixels, _) => {
                Some(pixels)
            }
            Paint::Fill(_) => None,
        }
    }

    /// Whether the pixels cover the extent (4 bytes each), and a shadow's profile is one row
    /// of at most `MAX_PROFILE` entries.
    fn valid(&self) -> bool {
        let needed = u64::from(self.extent.0) * u64::from(self.extent.1) * 4;
        self.width > 0
            && self.height > 0
            && self.pixels().is_none_or(|pixels| {
                self.extent.0 > 0 && self.extent.1 > 0 && pixels.len() as u64 >= needed
            })
            && match self.paint {
                Paint::Shadow(_, shadow) => {
                    shadow.valid()
                        && self.extent.1 == 1
                        && self.extent.0 as usize <= shape::MAX_PROFILE
                }
                _ => true,
            }
    }

    /// The radius its corners are drawn with.
    fn corner_radius(&self) -> u32 {
        shape::radius_on(self.radius, self.width, self.height)
    }

    /// Whether its pixels are filtered: they are drawn at another size (beyond rounding, on an
    /// axis at least), and asked to be, within what filtering draws. Drawn 1:1 they are copied as
    /// they are, as before filtering.
    fn filtered(&self) -> bool {
        self.filter == Filter::Box
            && matches!(self.paint, Paint::Opaque(_) | Paint::Premultiplied(_))
            && (filter::scaled(self.extent.0, self.width)
                || filter::scaled(self.extent.1, self.height))
            && filter::filters(self.extent.0, self.width)
            && filter::filters(self.extent.1, self.height)
    }
}

/// A rectangle of the output, in its pixels: a frame's damage, what changed since the previous
/// frame composed into the same place (docs/adr/0005-damage.mdx).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    fn right(&self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    fn bottom(&self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    /// The part of both, if any.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > i64::from(x) && bottom > i64::from(y)).then(|| Rect {
            x,
            y,
            width: (right - i64::from(x)) as u32,
            height: (bottom - i64::from(y)) as u32,
        })
    }

    /// The smallest rectangle holding both.
    pub fn bounding(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect {
            x,
            y,
            width: (self.right().max(other.right()) - i64::from(x)) as u32,
            height: (self.bottom().max(other.bottom()) - i64::from(y)) as u32,
        }
    }
}

fn pixel(pixels: &[u8], index: usize) -> u32 {
    u32::from_le_bytes([
        pixels[index * 4],
        pixels[index * 4 + 1],
        pixels[index * 4 + 2],
        pixels[index * 4 + 3],
    ])
}

/// The pixels of `rects` within `output` as rectangles that do not overlap, so that none is
/// composed twice: in bands between the rectangles' tops and bottoms, each band's spans joined.
pub(crate) fn disjoint(rects: &[Rect], output: &Rect) -> Vec<Rect> {
    let rects: Vec<Rect> = rects
        .iter()
        .filter_map(|rect| rect.intersect(output))
        .collect();
    let mut edges: Vec<i64> = rects
        .iter()
        .flat_map(|rect| [i64::from(rect.y), rect.bottom()])
        .collect();
    edges.sort_unstable();
    edges.dedup();
    let mut out = Vec::new();
    for band in edges.windows(2) {
        let (top, bottom) = (band[0], band[1]);
        let mut spans: Vec<(i64, i64)> = rects
            .iter()
            .filter(|rect| i64::from(rect.y) <= top && rect.bottom() >= bottom)
            .map(|rect| (i64::from(rect.x), rect.right()))
            .collect();
        spans.sort_unstable();
        let mut joined: Vec<(i64, i64)> = Vec::new();
        for (left, right) in spans {
            match joined.last_mut() {
                Some(last) if left <= last.1 => last.1 = last.1.max(right),
                _ => joined.push((left, right)),
            }
        }
        out.extend(joined.into_iter().map(|(left, right)| Rect {
            x: left as i32,
            y: top as i32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        }));
    }
    out
}

/// Composes `layers` onto an output of `width` × `height` filled with `background`, as the
/// compositor shows them: 0x00RRGGBB per pixel, row by row. The reference composition.
pub fn compose_cpu(width: u32, height: u32, background: u32, layers: &[Layer<'_>]) -> Vec<u32> {
    let mut out = vec![background; width as usize * height as usize];
    for layer in layers.iter().filter(|layer| layer.valid()) {
        // Within the output and the layer's clip.
        let clip = layer.clip.unwrap_or(Rect {
            x: 0,
            y: 0,
            width,
            height,
        });
        let x0 = layer.x.max(0).max(clip.x);
        let y0 = layer.y.max(0).max(clip.y);
        let x1 = (i64::from(layer.x) + i64::from(layer.width))
            .min(i64::from(width))
            .min(clip.right());
        let y1 = (i64::from(layer.y) + i64::from(layer.height))
            .min(i64::from(height))
            .min(clip.bottom());
        let radius = layer.corner_radius();
        // A filtered layer's weights, for the columns and rows shown.
        let filtered = (layer.filtered() && i64::from(x0) < x1 && i64::from(y0) < y1).then(|| {
            let axis = |start: i32, end: i64, at: i32, length: u32, extent: u32| {
                let from = (i64::from(start) - i64::from(at)) as u32;
                let to = (end - i64::from(at)) as u32;
                filter::Axis::new(from, to, length, extent)
            };
            (
                axis(x0, x1, layer.x, layer.width, layer.extent.0),
                axis(y0, y1, layer.y, layer.height, layer.extent.1),
            )
        });
        // A layer at its pixels' own size takes each pixel as it is, with no scaling to work out;
        // an opaque one at full opacity is copied where it covers a pixel whole: away from its
        // rounded corners, each pixel is its source's (what the blending below gives there).
        let one_to_one = (layer.width, layer.height) == layer.extent;
        let copied = one_to_one && matches!(layer.paint, Paint::Opaque(_)) && layer.opacity == 255;
        for y in i64::from(y0)..y1 {
            let ly = (y - i64::from(layer.y)) as u64;
            let sy = if one_to_one {
                ly
            } else {
                ly * u64::from(layer.extent.1) / u64::from(layer.height)
            };
            let rows = filtered
                .as_ref()
                .map(|(columns, rows)| (columns, rows.get((y - i64::from(y0)) as usize)));
            // The columns of this row (the layer's) that no rounded corner touches.
            let whole =
                if u64::from(radius) <= ly && ly + u64::from(radius) < u64::from(layer.height) {
                    0..u64::from(layer.width)
                } else {
                    u64::from(radius)..u64::from(layer.width.saturating_sub(radius))
                };
            // The output's columns copied whole, at once.
            let copy = match layer.paint {
                Paint::Opaque(pixels) if copied => {
                    let from = (i64::from(x0)).max(i64::from(layer.x) + whole.start as i64);
                    let to = x1.min(i64::from(layer.x) + whole.end as i64);
                    if from < to {
                        let row = y as usize * width as usize;
                        let source = (sy * u64::from(layer.extent.0)) as usize
                            + (from - i64::from(layer.x)) as usize;
                        let pixels = &pixels[source * 4..(source + (to - from) as usize) * 4];
                        for (out, pixel) in out[row + from as usize..row + to as usize]
                            .iter_mut()
                            .zip(pixels.chunks_exact(4))
                        {
                            *out = u32::from_le_bytes([pixel[0], pixel[1], pixel[2], 0]);
                        }
                    }
                    from..to
                }
                _ => 0..0,
            };
            // A shadow's columns wholly beneath what casts it (`beneath` all of the pixel): none of
            // it is drawn there.
            let hidden = match layer.paint {
                Paint::Shadow(_, shadow) => {
                    let clip = &shadow.clip;
                    let cy = ly as i64 - i64::from(clip.y);
                    if cy < 0 || cy >= i64::from(clip.height) {
                        0..0
                    } else {
                        let r = i64::from(shape::radius_on(
                            shadow.clip_radius,
                            clip.width,
                            clip.height,
                        ));
                        let inset = if r <= cy && cy + r < i64::from(clip.height) {
                            0
                        } else {
                            r
                        };
                        let at = i64::from(layer.x) + i64::from(clip.x);
                        at + inset..at + i64::from(clip.width) - inset
                    }
                }
                _ => 0..0,
            };
            for x in i64::from(x0)..x1 {
                if copy.contains(&x) || hidden.contains(&x) {
                    continue;
                }
                let lx = (x - i64::from(layer.x)) as u64;
                let index = y as usize * width as usize + x as usize;
                // How much of the pixel the layer covers: all of it, but at its rounded corners
                // and, a shadow, within what it is beneath.
                let mut coverage =
                    shape::corner_coverage(lx as u32, ly as u32, layer.width, layer.height, radius);
                if let Paint::Shadow(_, shadow) = layer.paint {
                    coverage = coverage * (255 - beneath(lx as u32, ly as u32, &shadow)) / 255;
                }
                if coverage == 0 {
                    continue;
                }
                let source_index = || {
                    let sx = if one_to_one {
                        lx
                    } else {
                        lx * u64::from(layer.extent.0) / u64::from(layer.width)
                    };
                    (sy * u64::from(layer.extent.0) + sx) as usize
                };
                // The pixel, premultiplied, with its alpha (opaque ones and fills have 255).
                let (source, alpha) = match (layer.paint, rows) {
                    (
                        Paint::Opaque(pixels) | Paint::Premultiplied(pixels),
                        Some((columns, rows)),
                    ) => filter::sample(
                        pixels,
                        layer.extent.0,
                        columns.get((x - i64::from(x0)) as usize),
                        rows,
                        matches!(layer.paint, Paint::Opaque(_)),
                    ),
                    (Paint::Fill(color), _) => (color & 0x00ff_ffff, 255),
                    (Paint::Opaque(pixels), _) => {
                        (pixel(pixels, source_index()) & 0x00ff_ffff, 255)
                    }
                    (Paint::Premultiplied(pixels), _) => {
                        let source = pixel(pixels, source_index());
                        (source & 0x00ff_ffff, source >> 24)
                    }
                    (Paint::Shadow(profile, shadow), _) => {
                        let entry = shape::shadow_entry(
                            lx as u32,
                            ly as u32,
                            &shadow.shape,
                            shape::radius_on(
                                shadow.radius,
                                shadow.shape.width,
                                shadow.shape.height,
                            ),
                            shadow.scale,
                            layer.extent.0,
                        );
                        let source = pixel(profile, entry as usize);
                        (source & 0x00ff_ffff, source >> 24)
                    }
                };
                // Scaled by the layer's opacity at this pixel (its opacity by its coverage), then
                // source-over: at 255 an opaque pixel is copied and a premultiplied one blended,
                // as without an opacity.
                let opacity = u32::from(layer.opacity) * coverage / 255;
                let keep = 255 - alpha * opacity / 255;
                let under = out[index];
                let channel = |shift: u32| {
                    let s = ((source >> shift) & 0xff) * opacity / 255;
                    let d = (under >> shift) & 0xff;
                    (s + d * keep / 255).min(255) << shift
                };
                out[index] = channel(16) | channel(8) | channel(0);
            }
        }
    }
    out
}

/// How much of a shadow's pixel (`x`, `y`) what it is beneath covers, 0 to 255.
fn beneath(x: u32, y: u32, shadow: &Shadow) -> u32 {
    let clip = &shadow.clip;
    let (cx, cy) = (
        i64::from(x) - i64::from(clip.x),
        i64::from(y) - i64::from(clip.y),
    );
    if cx < 0 || cy < 0 || cx >= i64::from(clip.width) || cy >= i64::from(clip.height) {
        return 0;
    }
    let radius = shape::radius_on(shadow.clip_radius, clip.width, clip.height);
    shape::corner_coverage(cx as u32, cy as u32, clip.width, clip.height, radius)
}

pub mod filter;
pub mod frame_file;
pub mod shape;
pub use shape::{ShadowProfile, MAX_SHADOW_SCALE};
mod vulkan;
#[cfg(target_os = "linux")]
pub use vulkan::ScanoutTarget;
pub use vulkan::{DeviceKind, VulkanCompositor};

/// The SPIR-V of the composition shader (`shaders/compose.wgsl`).
fn shader() -> Vec<u32> {
    include_bytes!(concat!(env!("OUT_DIR"), "/compose.spv"))
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
        .collect()
}

fn name(raw: &[std::ffi::c_char]) -> String {
    CStr::from_bytes_until_nul(&raw.iter().map(|&c| c as u8).collect::<Vec<u8>>())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
