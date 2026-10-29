//! Windows cursor files (`.cur`), for the pointer's image: ICO's directory of images, each with its
//! hotspot. The session takes the 32-bit bitmaps among them (straight alpha, rows bottom up, an
//! AND mask after them) and turns the one chosen into the premultiplied pixels the cursor plane
//! shows. PNG entries and bitmaps of fewer bits are passed over.

use oxyos_kms::PointerImage;
use oxyos_protocol::PointerShape;
use std::path::{Path, PathBuf};

/// A Windows cursor scheme's roles (its `Install.inf`'s `[Strings]` keys) and the shapes they are.
const ROLES: [(&str, PointerShape); 16] = [
    ("pointer", PointerShape::Arrow),
    ("text", PointerShape::Text),
    ("link", PointerShape::Hand),
    ("unavailiable", PointerShape::NotAllowed),
    ("busy", PointerShape::Wait),
    ("help", PointerShape::Help),
    ("cross", PointerShape::Crosshair),
    ("move", PointerShape::Move),
    ("grab", PointerShape::Grab),
    ("grabbing", PointerShape::Grabbing),
    ("horz", PointerShape::ResizeHorizontal),
    ("vert", PointerShape::ResizeVertical),
    ("dgn1", PointerShape::ResizeTopLeft),
    ("dgn2", PointerShape::ResizeTopRight),
    ("zoom-in", PointerShape::ZoomIn),
    ("zoom-out", PointerShape::ZoomOut),
];

/// The cursor files of a scheme's folder, by shape, as its `Install.inf` names them. Animated
/// cursors (`.ani`) are left out: their shapes show the arrow.
pub fn scheme(folder: &Path) -> Result<Vec<(PointerShape, PathBuf)>, String> {
    let inf = std::fs::read(folder.join("Install.inf"))
        .map_err(|error| format!("its Install.inf: {error}"))?;
    Ok(scheme_files(&String::from_utf8_lossy(&inf))
        .into_iter()
        .map(|(shape, file)| (shape, folder.join(file)))
        .collect())
}

fn scheme_files(inf: &str) -> Vec<(PointerShape, String)> {
    let mut strings = false;
    let mut files = Vec::new();
    for line in inf.lines().map(str::trim) {
        if line.starts_with('[') {
            strings = line.eq_ignore_ascii_case("[Strings]");
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| strings) else {
            continue;
        };
        let (key, file) = (key.trim(), value.trim().trim_matches('"'));
        let Some(&(_, shape)) = ROLES
            .iter()
            .find(|(role, _)| role.eq_ignore_ascii_case(key))
        else {
            continue;
        };
        if file.to_ascii_lowercase().ends_with(".cur") {
            files.push((shape, file.to_string()));
        }
    }
    files
}

struct Entry {
    width: u32,
    height: u32,
    hotspot: (u32, u32),
    data: std::ops::Range<usize>,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The image of `bytes` closest to `size` pixels tall that fits `max`: the smallest at least that
/// size, else the largest smaller.
pub fn read(bytes: &[u8], size: u32, max: (u32, u32)) -> Result<PointerImage, String> {
    if u16_at(bytes, 2) != Some(2) {
        return Err("not a cursor file".into());
    }
    let count = u16_at(bytes, 4).ok_or("a truncated cursor file")?;
    let mut entries = Vec::new();
    for index in 0..usize::from(count) {
        let at = 6 + 16 * index;
        let side = |byte: u8| if byte == 0 { 256 } else { u32::from(byte) };
        let (Some(&width), Some(&height)) = (bytes.get(at), bytes.get(at + 1)) else {
            return Err("a truncated cursor file".into());
        };
        let hotspot = (
            u32::from(u16_at(bytes, at + 4).ok_or("a truncated cursor file")?),
            u32::from(u16_at(bytes, at + 6).ok_or("a truncated cursor file")?),
        );
        let length = u32_at(bytes, at + 8).ok_or("a truncated cursor file")? as usize;
        let offset = u32_at(bytes, at + 12).ok_or("a truncated cursor file")? as usize;
        let data = offset..offset.saturating_add(length);
        let Some(image) = bytes.get(data.clone()) else {
            continue;
        };
        // A 32-bit bitmap: its header's size, then its bits per pixel.
        if u32_at(image, 0) != Some(40) || u16_at(image, 14) != Some(32) {
            continue;
        }
        let (width, height) = (side(width), side(height));
        if width <= max.0 && height <= max.1 {
            entries.push(Entry {
                width,
                height,
                hotspot,
                data,
            });
        }
    }
    let chosen = entries
        .iter()
        .filter(|entry| entry.height >= size)
        .min_by_key(|entry| entry.height)
        .or_else(|| entries.iter().max_by_key(|entry| entry.height))
        .ok_or("no 32-bit image in the cursor file fits the cursor plane")?;
    bitmap(&bytes[chosen.data.clone()], chosen)
}

fn bitmap(image: &[u8], entry: &Entry) -> Result<PointerImage, String> {
    let (width, height) = (entry.width as usize, entry.height as usize);
    if u32_at(image, 16) != Some(0) {
        return Err("a compressed cursor bitmap".into());
    }
    let colours = 40..40 + width * height * 4;
    // The AND mask: a bit per pixel, rows padded to 32 bits, bottom up.
    let mask_row = width.div_ceil(32) * 4;
    let mask = colours.end..colours.end + mask_row * height;
    let colours = image.get(colours).ok_or("a truncated cursor bitmap")?;
    let mask = image.get(mask);
    let has_alpha = colours.chunks_exact(4).any(|pixel| pixel[3] != 0);
    let mut pixels = vec![0u32; width * height];
    for y in 0..height {
        // Rows are stored bottom up.
        let row = height - 1 - y;
        for x in 0..width {
            let at = (row * width + x) * 4;
            let (b, g, r) = (colours[at], colours[at + 1], colours[at + 2]);
            let alpha = if has_alpha {
                colours[at + 3]
            } else {
                // Without alpha, the mask says which pixels are transparent.
                let transparent =
                    mask.is_some_and(|mask| mask[row * mask_row + x / 8] & (0x80 >> (x % 8)) != 0);
                if transparent {
                    0
                } else {
                    255
                }
            };
            let premultiply = |channel: u8| u32::from(channel) * u32::from(alpha) / 255;
            pixels[y * width + x] = (u32::from(alpha) << 24)
                | (premultiply(r) << 16)
                | (premultiply(g) << 8)
                | premultiply(b);
        }
    }
    Ok(PointerImage {
        width: entry.width,
        height: entry.height,
        pixels,
        hotspot: (
            entry.hotspot.0.min(entry.width - 1),
            entry.hotspot.1.min(entry.height - 1),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image of a test cursor file: its side, its hotspot, and its BGRA pixels top down (none
    /// for a PNG entry).
    type Image = (u8, (u16, u16), Option<Vec<[u8; 4]>>);

    /// A cursor file of `images`.
    fn cursor(images: &[Image]) -> Vec<u8> {
        let mut out = vec![0, 0, 2, 0];
        out.extend((images.len() as u16).to_le_bytes());
        let mut data = Vec::new();
        let base = 6 + 16 * images.len();
        for (side, hotspot, pixels) in images {
            let image = match pixels {
                Some(pixels) => {
                    let side = usize::from(*side);
                    let mut image = Vec::new();
                    image.extend(40u32.to_le_bytes());
                    image.extend((side as i32).to_le_bytes());
                    image.extend((2 * side as i32).to_le_bytes());
                    image.extend(1u16.to_le_bytes());
                    image.extend(32u16.to_le_bytes());
                    image.extend([0; 24]);
                    for row in (0..side).rev() {
                        for pixel in &pixels[row * side..(row + 1) * side] {
                            image.extend(pixel);
                        }
                    }
                    image.extend(vec![0; side.div_ceil(32) * 4 * side]);
                    image
                }
                None => b"\x89PNG\r\n\x1a\n".to_vec(),
            };
            out.extend([*side, *side, 0, 0]);
            out.extend(hotspot.0.to_le_bytes());
            out.extend(hotspot.1.to_le_bytes());
            out.extend((image.len() as u32).to_le_bytes());
            out.extend(((base + data.len()) as u32).to_le_bytes());
            data.extend(image);
        }
        out.extend(data);
        out
    }

    #[test]
    fn the_closest_32_bit_image_is_read_premultiplied_top_down() {
        // 2x2: top row opaque red and transparent; bottom row half-transparent white and black.
        let small = vec![
            [0, 0, 255, 255],
            [9, 9, 9, 0],
            [255, 255, 255, 128],
            [0, 0, 0, 255],
        ];
        let large = vec![[0, 255, 0, 255]; 16];
        let file = cursor(&[
            (2, (1, 0), Some(small)),
            (4, (3, 2), Some(large)),
            (8, (0, 0), None),
        ]);
        let image = read(&file, 2, (64, 64)).expect("read");
        assert_eq!((image.width, image.height, image.hotspot), (2, 2, (1, 0)));
        assert_eq!(image.pixels, [0xffff_0000, 0, 0x8080_8080, 0xff00_0000]);
        // Larger than asked for, the next size up; the PNG entry is passed over.
        let image = read(&file, 3, (64, 64)).expect("read");
        assert_eq!((image.width, image.hotspot), (4, (3, 2)));
        assert_eq!(read(&file, 100, (64, 64)).expect("read").width, 4);
        // What does not fit the cursor plane is passed over.
        assert_eq!(read(&file, 3, (2, 2)).expect("read").width, 2);
        assert!(read(&file, 3, (1, 1)).is_err());
    }

    #[test]
    fn a_schemes_roles_name_its_shapes_files() {
        let inf = "[Version]\r\npointer = \"Nope.cur\"\r\n[Strings]\r\nSCHEME_NAME = \"x\"\r\n\
                   pointer      = \"Normal.cur\"\r\nbusy = \"Busy.ani\"\r\ntext = \"Text.cur\"\r\n\
                   dgn2 = \"Diagonal Resize 2.cur\"\r\n";
        assert_eq!(
            scheme_files(inf),
            [
                (PointerShape::Arrow, "Normal.cur".to_string()),
                (PointerShape::Text, "Text.cur".to_string()),
                (
                    PointerShape::ResizeTopRight,
                    "Diagonal Resize 2.cur".to_string()
                ),
            ]
        );
    }

    #[test]
    fn without_alpha_the_mask_says_what_is_transparent() {
        let file = cursor(&[(1, (0, 0), Some(vec![[1, 2, 3, 0]]))]);
        // A clear mask: the pixel is opaque.
        assert_eq!(read(&file, 1, (8, 8)).expect("read").pixels, [0xff03_0201]);
        assert!(read(b"nope", 1, (8, 8)).is_err());
    }
}
