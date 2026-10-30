//! The boot logo's images (docs/adr/0017-boot-logo.mdx). The Plymouth theme `oxyos`
//! (`config/includes.chroot_after_packages/usr/share/plymouth/themes/oxyos`) shows the session's
//! splash (`oxyos-compositor`'s `splash.rs`: the Oxy mark, white, centred on black, an eighth of
//! the output's height) from the kernel's first frame until the session's own, so the session
//! taking the display over changes no pixel. Its images are drawn here, by the session's code:
//!
//! - `splash-WxH.png`, for each output size in [`SIZES`]: the session's splash frame at that size,
//!   cropped to the rectangle about its centre that holds every pixel of the mark (the rest is
//!   black, the theme's background). The theme centres it, so its pixels are the session's.
//! - `mark-H.png`, `field-H.png`, `bullet-H.png`, for each height in [`LADDER`]: the mark as an
//!   output H pixels high shows it, and the password field and its bullet at the same scale. The
//!   theme scales them, down by at most half, for an output size it has no frame for, and to ask
//!   for a disk's passphrase.
//!
//! The committed images are checked against these by `cargo test` (`the_theme_is_the_sessions_splash`).
//!
//!     oxyos-boot-logo THEME_DIR             writes every image of the theme into THEME_DIR
//!     oxyos-boot-logo --frame WxH OUT.png   writes the session's whole splash frame at WxH

use oxyos_protocol::Size;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "usage: oxyos-boot-logo THEME_DIR | oxyos-boot-logo --frame WxH OUT.png";

/// The output sizes the theme shows the session's exact pixels on: laptop and desktop panels
/// (16:9, 16:10, 3:2), among them the ThinkPad P14s's 1920×1200, 2240×1400, 2880×1800 and
/// 3840×2400.
pub const SIZES: [(u32, u32); 17] = [
    (1280, 720),
    (1280, 800),
    (1366, 768),
    (1440, 900),
    (1536, 864),
    (1600, 900),
    (1680, 1050),
    (1920, 1080),
    (1920, 1200),
    (2240, 1400),
    (2256, 1504),
    (2560, 1440),
    (2560, 1600),
    (2880, 1800),
    (3000, 2000),
    (3840, 2160),
    (3840, 2400),
];

/// The output heights the mark, the password field and its bullet are drawn for, to be scaled
/// to any other: the theme takes the smallest at least as high as its output (the highest above
/// it), so it scales down by at most half.
pub const LADDER: [u32; 3] = [1200, 2400, 4800];

/// The password field, a pill under the mark: twice the mark's side wide, a quarter of it high,
/// its centre the mark's side below the mark's. Its fill is white at a fifth over black.
const FIELD_FILL: u32 = 0xff33_3333;

/// An opaque image, its pixels R, G, B row by row.
#[derive(Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// The session's splash frame for an output of `width` × `height`, as the session shows it.
pub fn frame(width: u32, height: u32) -> Result<Image> {
    let splash = oxyos_compositor::splash::splash(Size::new(width, height))
        .ok_or_else(|| format!("no splash at {width}x{height}"))?;
    // The frame's pixels are B, G, R, X in memory (XRGB8888).
    let rgb = splash
        .pixels
        .chunks_exact(4)
        .flat_map(|bgrx| [bgrx[2], bgrx[1], bgrx[0]])
        .collect();
    Ok(Image { width, height, rgb })
}

/// The smallest rectangle about the image's centre that holds every pixel not black: its left
/// and top margins are its right and bottom ones, so centring it on an output of the image's
/// size puts each pixel where it was.
pub fn centred_crop(image: &Image) -> Image {
    let (width, height) = (image.width as usize, image.height as usize);
    let lit = |x: usize, y: usize| image.rgb[(y * width + x) * 3..][..3] != [0, 0, 0];
    let (mut left, mut top) = (width / 2, height / 2);
    for y in 0..height {
        for x in 0..width {
            if lit(x, y) {
                left = left.min(x).min(width - 1 - x);
                top = top.min(y).min(height - 1 - y);
            }
        }
    }
    let (crop_width, crop_height) = (width - 2 * left, height - 2 * top);
    let rgb = (top..top + crop_height)
        .flat_map(|y| {
            image.rgb[(y * width + left) * 3..][..crop_width * 3]
                .iter()
                .copied()
        })
        .collect();
    Image {
        width: crop_width as u32,
        height: crop_height as u32,
        rgb,
    }
}

/// Opaque shapes on black, as R, G, B.
fn draw(shapes: &[oxyos_vector::Shape<'_>], width: u32, height: u32) -> Result<Image> {
    let picture = oxyos_vector::render(shapes, width, height).ok_or("nothing drawn")?;
    // Premultiplied R, G, B, A over black is R, G, B.
    let rgb = picture
        .pixels
        .chunks_exact(4)
        .flat_map(|rgba| [rgba[0], rgba[1], rgba[2]])
        .collect();
    Ok(Image { width, height, rgb })
}

fn fill(color: u32) -> Option<oxyos_vector::Fill> {
    Some(oxyos_vector::Fill {
        color,
        even_odd: false,
    })
}

/// The password field for an output `height` pixels high, and its bullet.
pub fn field(height: u32) -> Result<(Image, Image)> {
    let side = (height / 8).max(8);
    let (width, tall) = (side * 2, side / 4);
    let field = draw(
        &[oxyos_vector::Shape {
            geometry: oxyos_vector::Geometry::Rect {
                x: 0.0,
                y: 0.0,
                width: width as f32,
                height: tall as f32,
                rx: tall as f32 / 2.0,
                ry: tall as f32 / 2.0,
            },
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            fill: fill(FIELD_FILL),
            stroke: None,
        }],
        width,
        tall,
    )?;
    let diameter = (tall * 2 / 5).max(2);
    let radius = diameter as f32 / 2.0;
    let bullet = draw(
        &[oxyos_vector::Shape {
            geometry: oxyos_vector::Geometry::Ellipse {
                cx: radius,
                cy: radius,
                rx: radius,
                ry: radius,
            },
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            fill: fill(0xffff_ffff),
            stroke: None,
        }],
        diameter,
        diameter,
    )?;
    Ok((field, bullet))
}

/// Every image of the theme, by its file's name.
pub fn images() -> Result<Vec<(String, Image)>> {
    let mut images = Vec::new();
    for (width, height) in SIZES {
        images.push((
            format!("splash-{width}x{height}.png"),
            centred_crop(&frame(width, height)?),
        ));
    }
    for height in LADDER {
        // The mark as an output this high shows it; its width does not change the mark.
        images.push((
            format!("mark-{height}.png"),
            centred_crop(&frame(height, height)?),
        ));
        let (field, bullet) = field(height)?;
        images.push((format!("field-{height}.png"), field));
        images.push((format!("bullet-{height}.png"), bullet));
    }
    Ok(images)
}

pub fn encode(image: &Image) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, image.width, image.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.rgb)?;
    writer.finish()?;
    Ok(bytes)
}

/// An 8-bit RGB PNG, as the theme's are.
pub fn decode(bytes: &[u8]) -> Result<Image> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info()?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
    let frame = reader.next_frame(&mut buffer)?;
    if frame.color_type != png::ColorType::Rgb || frame.bit_depth != png::BitDepth::Eight {
        return Err(format!(
            "{:?} {:?}, not 8-bit RGB",
            frame.color_type, frame.bit_depth
        )
        .into());
    }
    buffer.truncate(frame.buffer_size());
    Ok(Image {
        width: frame.width,
        height: frame.height,
        rgb: buffer,
    })
}

fn size(text: &str) -> Option<(u32, u32)> {
    let (width, height) = text.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

fn write(path: &Path, image: &Image) -> Result<()> {
    std::fs::write(path, encode(image)?)?;
    println!("{} ({}x{})", path.display(), image.width, image.height);
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag, at, out] if flag == "--frame" => {
            let (width, height) = size(at).ok_or(USAGE)?;
            write(Path::new(out), &frame(width, height)?)
        }
        [dir] if !dir.starts_with('-') => {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir)?;
            for (name, image) in images()? {
                write(&dir.join(name), &image)?;
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../config/includes.chroot_after_packages/usr/share/plymouth/themes/oxyos")
    }

    #[test]
    fn the_crop_holds_the_whole_mark_and_centring_it_puts_every_pixel_back() {
        let whole = frame(1920, 1200).unwrap();
        let crop = centred_crop(&whole);
        // An eighth of the height tall, 150 pixels here; the outline is narrower than it is tall.
        assert!((130..=140).contains(&crop.width), "{}", crop.width);
        assert!((150..=152).contains(&crop.height), "{}", crop.height);
        // Centred as the theme centres it, every pixel of the frame is the crop's or black.
        let (left, top) = ((1920 - crop.width) / 2, (1200 - crop.height) / 2);
        assert_eq!(left * 2 + crop.width, 1920);
        assert_eq!(top * 2 + crop.height, 1200);
        for y in 0..1200 {
            for x in 0..1920 {
                let shown = if (left..left + crop.width).contains(&x)
                    && (top..top + crop.height).contains(&y)
                {
                    let i = (((y - top) * crop.width + x - left) * 3) as usize;
                    &crop.rgb[i..i + 3]
                } else {
                    &[0u8, 0, 0][..]
                };
                let i = ((y * 1920 + x) * 3) as usize;
                assert_eq!(shown, &whole.rgb[i..i + 3], "({x}, {y})");
            }
        }
    }

    #[test]
    fn the_mark_is_white_where_it_is_whole() {
        let crop = centred_crop(&frame(1920, 1200).unwrap());
        let centre = ((crop.height / 2 * crop.width + crop.width / 2) * 3) as usize;
        assert_eq!(&crop.rgb[centre..centre + 3], &[255, 255, 255]);
    }

    #[test]
    fn the_field_is_a_grey_pill_and_the_bullet_white() {
        let (field, bullet) = field(1200).unwrap();
        assert_eq!((field.width, field.height), (300, 37));
        let centre = ((field.height / 2 * field.width + field.width / 2) * 3) as usize;
        assert_eq!(&field.rgb[centre..centre + 3], &[0x33, 0x33, 0x33]);
        assert_eq!(&field.rgb[0..3], &[0, 0, 0], "a rounded corner");
        assert_eq!((bullet.width, bullet.height), (14, 14));
        let centre = ((7 * 14 + 7) * 3) as usize;
        assert_eq!(&bullet.rgb[centre..centre + 3], &[255, 255, 255]);
    }

    #[test]
    fn images_survive_encoding() {
        let (field, _) = field(1200).unwrap();
        assert_eq!(decode(&encode(&field).unwrap()).unwrap(), field);
    }

    /// The theme in the image is what this draws: regenerate it with
    /// `cargo run -p oxyos-boot-logo -- config/includes.chroot_after_packages/usr/share/plymouth/themes/oxyos`
    /// whenever the splash changes.
    #[test]
    fn the_theme_is_the_sessions_splash() {
        let images = images().unwrap();
        let mut expected: Vec<String> = images.iter().map(|(name, _)| name.clone()).collect();
        expected.sort();
        let mut committed: Vec<String> = std::fs::read_dir(theme())
            .unwrap()
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| name.ends_with(".png"))
            .collect();
        committed.sort();
        assert_eq!(committed, expected, "the theme's images");
        for (name, image) in &images {
            let bytes = std::fs::read(theme().join(name)).unwrap();
            assert!(
                decode(&bytes).unwrap() == *image,
                "{name} is not the session's"
            );
        }
    }

    #[test]
    fn the_script_loads_the_images_drawn() {
        let script = std::fs::read_to_string(theme().join("oxyos.script")).unwrap();
        assert!(script.contains(r#"Image("splash-" + width + "x" + height + ".png")"#));
        for name in ["mark", "field", "bullet"] {
            assert!(
                script.contains(&format!(r#"Image("{name}-" + level + ".png")"#)),
                "{name}"
            );
        }
        let levels: Vec<String> = LADDER.iter().map(u32::to_string).collect();
        assert!(
            script.contains(&format!("global.ladder = [{}];", levels.join(", "))),
            "the script's ladder is {LADDER:?}"
        );
    }
}
