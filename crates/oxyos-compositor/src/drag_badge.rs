//! The badge a drag's pointer shows for its action (docs/adr/0035-drag-and-drop.mdx), as a Mac's
//! drag cursors show it: a copy's white plus on a green disc, a link's curved arrow on a white
//! one; a move shows none, and neither does a drop that would be refused. The scene places it
//! below and right of the pointer's tip (`RenderItem::DragBadge`); this draws it, at the scale of
//! the output it is on, crisp at a fractional one.

use oxyos_protocol::{DragActions, Size};
use oxyos_vector::{Cap, Fill, Geometry, Join, Shape, Stroke};

/// A Mac's green (systemGreen) and the white and near black its badges are drawn in.
const GREEN: u32 = 0xff28_cd41;
const WHITE: u32 = 0xffff_ffff;
const INK: u32 = 0xff1c_1c1e;
/// The badge's outline, so it reads over any colour: a thin ring of shadow round its disc.
const RING: u32 = 0x5900_0000;

/// The badge for `action` at `side` logical pixels square, drawn at `scale_120`: its size in
/// pixels and its premultiplied B, G, R, A bytes (`Format::Argb8888`). None for an action that
/// shows no badge.
pub fn badge(action: DragActions, side: u32, scale_120: u32) -> Option<(Size, Vec<u8>)> {
    let pixels = crate::scaled(side, scale_120).max(1);
    // The badge's own units: a square of 16, whatever its pixels.
    let k = pixels as f32 / 16.0;
    let transform = [k, 0.0, 0.0, k, 0.0, 0.0];
    let disc = |color: u32| Shape {
        geometry: Geometry::Ellipse {
            cx: 8.0,
            cy: 8.0,
            rx: 7.0,
            ry: 7.0,
        },
        transform,
        fill: Some(Fill {
            color,
            even_odd: false,
        }),
        stroke: Some(Stroke {
            color: RING,
            width: 1.0,
            cap: Cap::Butt,
            join: Join::Round,
            miter_limit: 4.0,
            dash: &[],
            dash_offset: 0.0,
        }),
    };
    let line = |color: u32, width: f32, path: &'static str| Shape {
        geometry: Geometry::Path(path),
        transform,
        fill: None,
        stroke: Some(Stroke {
            color,
            width,
            cap: Cap::Round,
            join: Join::Round,
            miter_limit: 4.0,
            dash: &[],
            dash_offset: 0.0,
        }),
    };
    let shapes = if action == DragActions::COPY {
        vec![disc(GREEN), line(WHITE, 2.0, "M8 4.25V11.75M4.25 8H11.75")]
    } else if action == DragActions::LINK {
        vec![
            disc(WHITE),
            // An arrow curving up and to the right, an alias's.
            line(INK, 1.5, "M5 11.5V9.5C5 7.6 6.3 6.5 8.2 6.5H11"),
            line(INK, 1.5, "M9 4.5L11.2 6.5L9 8.5"),
        ]
    } else {
        return None;
    };
    let picture = oxyos_vector::render(&shapes, pixels, pixels)?;
    let bytes = picture
        .pixels
        .chunks_exact(4)
        .flat_map(|rgba| [rgba[2], rgba[1], rgba[0], rgba[3]])
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

    #[test]
    fn a_copy_is_a_white_plus_on_green() {
        let (size, bytes) = badge(DragActions::COPY, 16, 120).unwrap();
        assert_eq!(size, Size::new(16, 16));
        // The plus's middle white; a point of the disc beside its arms green; the corners clear.
        assert_eq!(at(size, &bytes, 8, 8), [0xff, 0xff, 0xff, 0xff]);
        assert_eq!(at(size, &bytes, 5, 5), [0x41, 0xcd, 0x28, 0xff]);
        assert_eq!(at(size, &bytes, 0, 0)[3], 0);
    }

    #[test]
    fn crisp_at_a_fractional_scale() {
        // At 1.25 it is drawn at its 20 pixels, never stretched from 16.
        let (size, bytes) = badge(DragActions::COPY, 16, 150).unwrap();
        assert_eq!(size, Size::new(20, 20));
        assert_eq!(at(size, &bytes, 10, 10), [0xff, 0xff, 0xff, 0xff]);
        assert_eq!(at(size, &bytes, 6, 6), [0x41, 0xcd, 0x28, 0xff]);
    }

    #[test]
    fn a_link_is_an_arrow_on_white_and_a_move_has_none() {
        let (size, bytes) = badge(DragActions::LINK, 16, 120).unwrap();
        assert_eq!(at(size, &bytes, 3, 8), [0xff, 0xff, 0xff, 0xff]);
        let inked = (0..16)
            .flat_map(|y| (0..16).map(move |x| (x, y)))
            .filter(|&(x, y)| at(size, &bytes, x, y)[0] < 0x40 && at(size, &bytes, x, y)[3] == 0xff)
            .count();
        assert!(inked > 6, "{inked}");
        assert!(badge(DragActions::MOVE, 16, 120).is_none());
        assert!(badge(DragActions::NONE, 16, 120).is_none());
    }
}
