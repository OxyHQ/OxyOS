//! The session's splash (docs/adr/0011-transitions.mdx): the Oxy mark, white, centred on black
//! over the whole output, from the moment the session shows anything until its shell is on
//! screen, as a Mac shows its mark while it starts; it then fades out over the shell arriving.

use crate::transitions::Snapshot;
use oxyos_protocol::{Format, Rect, Size};

/// The Oxy mark's outline, as Bloom draws it (`src/social-button/OxyMark.tsx`,
/// `OXY_MARK_OUTER_PATH`, the drawing the Oxy SDK ships), verbatim, in its view box.
const MARK_PATH: &str = "M520.4336 284.25c-48.73438 5.57422-93.89844 40.73047-120.91016 93.60938-13.57422 26.72656-22.57813 60.8828-24.4375 92.32421l-.85547 15.29297-12.29297-6.28906c-34.8711-17.72266-66.16797-25.4414-103.75781-25.58203-33.15625-.14453-56.59375 4.85937-81.3164 17.14844-37.73048 18.86718-61.59767 48.16406-72.46095 88.60937-4 15.0039-4.42968 47.30469-.85546 63.59766 8.28906 36.8711 26.72656 70.74218 54.30859 99.46875 13.71875 14.14843 36.01562 32.30078 48.01953 39.01562 3.14453 1.85938 5.85938 3.71485 5.85938 4.14453 0 .4297-2.57422 2.28907-5.71875 4.14454-12.28907 7.0039-34.29688 25.01171-47.875 39.16015-59.88282 62.16797-74.60157 144.77344-36.30079 203.51172 21.4375 32.8711 55.3086 54.3086 99.1836 62.88281 17.57812 3.42969 53.44922 3.42969 72.74219 0 23.72656-4.28906 45.44921-11.57422 67.88671-22.86719l13.00391-6.57421v8.28906c0 19.15234 4.85938 46.44922 12.29297 68.88672 29.72656 90.03515 106.32812 140.6289 182.64453 120.76172 35.44531-9.28907 69.3164-35.15625 92.32422-70.7422 19.72266-30.58202 33.01562-72.60155 35.30078-111.7578l.85938-15.15235 12.71875 6.4336c36.01562 18.0078 66.74218 25.4375 105.47265 25.29687 36.15625-.14453 62.59766-6.86328 90.75-23.29687 14.71875-8.57422 36.30078-29.8711 44.875-44.16016 14.00781-23.29688 19.00781-41.875 19.15234-70.3164.14063-29.15235-3.85937-46.16016-17.57812-75.31641-16.57813-35.01563-49.02344-71.17188-83.17969-92.32422-5.28515-3.28516-9.71875-6.28906-9.71875-6.71484 0-.4297 4.4336-3.4297 9.71875-6.71875 24.15235-15.00391 52.45313-42.44532 68.17188-66.16797 25.4375-38.16016 36.8711-79.32032 32.30078-116.6211-7.71875-62.16797-52.02344-106.75781-119.48047-120.05078-19.00781-3.85547-54.8789-3.57031-75.7461.57422-21.72265 4.28516-39.73046 10.28906-60.30859 20.29297l-17.57812 8.4336v-8.14844c0-40.30079-16.57813-91.89454-40.16016-125.33594-27.72656-39.01563-64.88281-62.45313-107.61719-67.88672-12.71875-1.57031-14.14843-1.57031-29.4375.14453zm0 0";
const MARK_VIEW_BOX: (f32, f32, f32, f32) = (80.58, 263.58, 1009.84, 1002.84);

/// The splash for an output of `size`: the mark an eighth of its height tall, white on black.
pub fn splash(size: Size) -> Option<Snapshot> {
    let side = (size.height / 8).max(1);
    let (vx, vy, vw, vh) = MARK_VIEW_BOX;
    let scale = side as f32 / vw.max(vh);
    let (left, top) = (
        (size.width as f32 - vw * scale) / 2.0,
        (size.height as f32 - vh * scale) / 2.0,
    );
    let mark = oxyos_vector::Shape {
        geometry: oxyos_vector::Geometry::Path(MARK_PATH),
        transform: [scale, 0.0, 0.0, scale, left - vx * scale, top - vy * scale],
        fill: Some(oxyos_vector::Fill {
            color: 0xffff_ffff,
            even_odd: false,
        }),
        stroke: None,
    };
    let picture = oxyos_vector::render(&[mark], size.width, size.height)?;
    // Premultiplied R, G, B, A over black is R, G, B: the frame's pixels, B, G, R, X in memory.
    let pixels = picture
        .pixels
        .chunks_exact(4)
        .flat_map(|rgba| [rgba[2], rgba[1], rgba[0], 0xff])
        .collect();
    Some(Snapshot {
        rect: Rect::new(0, 0, size.width, size.height),
        extent: size,
        format: Format::Xrgb8888,
        pixels,
        look: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_is_white_and_centred_on_black() {
        let splash = splash(Size::new(640, 480)).unwrap();
        let at = |x: usize, y: usize| {
            let i = (y * 640 + x) * 4;
            u32::from_le_bytes([
                splash.pixels[i],
                splash.pixels[i + 1],
                splash.pixels[i + 2],
                splash.pixels[i + 3],
            ])
        };
        // The mark's centre is white; the corners and edges black.
        assert_eq!(at(320, 240), 0xffff_ffff);
        for (x, y) in [
            (0, 0),
            (639, 0),
            (0, 479),
            (639, 479),
            (320, 100),
            (320, 380),
        ] {
            assert_eq!(at(x, y), 0xff00_0000, "({x}, {y})");
        }
        // An eighth of the height tall: white within 30 pixels of the centre, none beyond 31.
        let white_rows: Vec<usize> = (0..480)
            .filter(|&y| (0..640).any(|x| at(x, y) & 0xff_ffff != 0))
            .collect();
        assert!(
            white_rows.len() >= 58 && white_rows.len() <= 61,
            "{}",
            white_rows.len()
        );
        assert!((white_rows[0] as i32 - (240 - 30)).abs() <= 2);
    }
}
