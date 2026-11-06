//! The poof a drag's image vanishes in where it is dropped on nothing (docs/adr/0035-drag-and-drop.mdx,
//! 6 November 2026): a Mac's `NSAnimationEffectPoof`, the small cloud an item dragged out of the
//! Dock puffs away in. A cluster of soft round puffs bursts out from the item's centre, each
//! growing as it drifts outward, a shade of grey beneath each white one so the cloud reads over any
//! background; whole for its first third, then fading out. The compositor draws it over the item's
//! last place (`transitions::Shift::Poof`), at the scale of the output it is on, crisp at a
//! fractional one.

use oxyos_protocol::Size;
use oxyos_vector::{Fill, Geometry, Shape};

/// How long the poof lasts, a Mac's (its five frames at about 60 ms each).
pub const POOF_DURATION: std::time::Duration = std::time::Duration::from_millis(300);
/// The poof's side for an item of `item` logical pixels: larger than the item, as a Mac's cloud
/// billows past the icon that vanishes in it.
pub fn side(item: u32) -> u32 {
    (item.saturating_mul(9) / 5).clamp(48, 180)
}

/// The cloud's white and the grey shading beneath each puff.
const WHITE: u32 = 0xf7f8fa;
const SHADE: u32 = 0xaeb3bc;
/// The puffs, around the centre: (angle in turns, how far out at the end as a share of the
/// half-side, radius at the end as a share of the half-side). Uneven, as a cloud is.
const PUFFS: [(f32, f32, f32); 8] = [
    (0.00, 0.50, 0.34),
    (0.13, 0.56, 0.28),
    (0.27, 0.48, 0.32),
    (0.40, 0.58, 0.27),
    (0.52, 0.50, 0.33),
    (0.66, 0.57, 0.29),
    (0.79, 0.49, 0.31),
    (0.91, 0.55, 0.27),
];

/// The cloud `progress` of the way through (0 to 1), `side` logical pixels square, drawn at
/// `scale_120`: its size in pixels and its premultiplied B, G, R, A bytes (`Format::Argb8888`).
/// None once it has gone (at 1), or for a side of nothing.
pub fn frame(progress: f64, side: u32, scale_120: u32) -> Option<(Size, Vec<u8>)> {
    let progress = progress.clamp(0.0, 1.0) as f32;
    let opacity = if progress < 1.0 / 3.0 {
        1.0
    } else {
        // Out over the rest, easing in: most of it gone well before the end.
        let t = (progress - 1.0 / 3.0) / (2.0 / 3.0);
        (1.0 - t) * (1.0 - t)
    };
    let alpha = (opacity * 255.0).round() as u32;
    if alpha == 0 || side == 0 {
        return None;
    }
    let pixels = crate::scaled(side, scale_120).max(1);
    // The cloud's own units: a square of 2 about its centre, whatever its pixels.
    let k = pixels as f32 / 2.0;
    let transform = [k, 0.0, 0.0, k, k, k];
    // A burst: out fast, then slowing, as a puff of air.
    let out = 1.0 - (1.0 - progress).powi(3);
    let mut shapes = Vec::with_capacity(PUFFS.len() * 2 + 2);
    let puff = |cx: f32, cy: f32, r: f32, color: u32| Shape {
        geometry: Geometry::Ellipse {
            cx,
            cy,
            rx: r,
            ry: r,
        },
        transform,
        fill: Some(Fill {
            color: 0xff00_0000 | color,
            even_odd: false,
        }),
        stroke: None,
    };
    let place = |(turn, far, radius): (f32, f32, f32)| {
        let angle = turn * std::f32::consts::TAU;
        let distance = 0.2 + (far - 0.2) * out;
        let r = radius * (0.62 + 0.38 * out);
        (angle.cos() * distance, angle.sin() * distance, r)
    };
    // The cloud's heart fills it as it opens, never leaving a hole: as large as the puffs are far.
    let heart = 0.26 + 0.2 * out;
    // The shading first, a little below each puff and the heart, so it shows only along the
    // cloud's underside; then the white over all of it.
    for &p in &PUFFS {
        let (x, y, r) = place(p);
        shapes.push(puff(x, y + r * 0.18, r, SHADE));
    }
    shapes.push(puff(0.0, heart * 0.18, heart, SHADE));
    shapes.push(puff(0.0, 0.0, heart * 0.95, WHITE));
    for &p in &PUFFS {
        let (x, y, r) = place(p);
        shapes.push(puff(x, y, r * 0.92, WHITE));
    }
    let picture = oxyos_vector::render(&shapes, pixels, pixels)?;
    // The cloud drawn whole, then faded as one (its puffs never showing through each other):
    // premultiplied, every channel scaled alike.
    let fade = |channel: u8| ((u32::from(channel) * alpha + 127) / 255) as u8;
    let bytes = picture
        .pixels
        .chunks_exact(4)
        .flat_map(|rgba| [fade(rgba[2]), fade(rgba[1]), fade(rgba[0]), fade(rgba[3])])
        .collect();
    Some((Size::new(pixels, pixels), bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(size: Size, bytes: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size.width + x) * 4) as usize;
        [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
    }

    /// The pixels covered at all.
    fn covered(size: Size, bytes: &[u8]) -> usize {
        (0..size.height)
            .flat_map(|y| (0..size.width).map(move |x| (x, y)))
            .filter(|&(x, y)| at(size, bytes, x, y)[3] > 0)
            .count()
    }

    #[test]
    fn it_puffs_out_whole_then_fades_away() {
        let (size, start) = frame(0.0, 64, 120).unwrap();
        assert_eq!(size, Size::new(64, 64));
        // At its start a small white cloud at the centre, opaque, the corners clear.
        assert_eq!(at(size, &start, 32, 32), [0xfa, 0xf8, 0xf7, 0xff]);
        assert_eq!(at(size, &start, 0, 0)[3], 0);
        // A third of the way it has burst out further, still whole.
        let (_, burst) = frame(0.33, 64, 120).unwrap();
        assert!(covered(size, &burst) > covered(size, &start));
        let opaque = |bytes: &[u8]| bytes.chunks_exact(4).map(|p| p[3]).max().unwrap();
        assert_eq!(opaque(&burst), 0xff);
        // Then it fades, and is gone at its end.
        let (_, fading) = frame(0.75, 64, 120).unwrap();
        assert!(opaque(&fading) < 0x40, "{}", opaque(&fading));
        assert!(frame(1.0, 64, 120).is_none());
    }

    #[test]
    fn crisp_at_a_fractional_scale_and_sized_for_its_item() {
        // At 1.25 it is drawn at its 80 pixels, never stretched from 64.
        let (size, bytes) = frame(0.0, 64, 150).unwrap();
        assert_eq!(size, Size::new(80, 80));
        assert_eq!(at(size, &bytes, 40, 40)[3], 0xff);
        // A dock icon of 42 gets a cloud of 75; never smaller than 48 nor larger than 180.
        assert_eq!(side(42), 75);
        assert_eq!(side(10), 48);
        assert_eq!(side(400), 180);
    }
}
