use super::*;
use std::time::{Duration, Instant};

fn png(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(data)
        .unwrap();
    bytes
}

/// A wallpaper frame of the repository: a 5120 × 2880 JPEG with a Display P3 profile.
fn wallpaper() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/wallpapers/solar-gradients/frames/frame-08.jpg"
    );
    std::fs::read(path).unwrap()
}

#[test]
fn a_png_decodes_premultiplied() {
    let bytes = png(
        2,
        1,
        png::ColorType::Rgba,
        &[255, 0, 0, 128, 0, 255, 0, 255],
    );
    assert_eq!(size(&bytes), Ok((2, 1)));
    let image = decode(&bytes, Fit::Whole).unwrap();
    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(image.pixels, [128, 0, 0, 128, 0, 255, 0, 255]);
    // Grey is spread over three channels, and opaque.
    let grey = decode(&png(1, 1, png::ColorType::Grayscale, &[90]), Fit::Whole).unwrap();
    assert_eq!(grey.pixels, [90, 90, 90, 255]);
}

#[test]
fn an_image_is_reduced_to_fit_by_averaging_and_never_enlarged() {
    // Black and white columns: each output pixel covers one of each.
    let columns: Vec<u8> = (0..8)
        .flat_map(|i| {
            if i % 2 == 0 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        })
        .collect();
    let bytes = png(4, 2, png::ColorType::Rgba, &columns);
    let image = decode(&bytes, Fit::Within(2, 10)).unwrap();
    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(image.pixels, [128, 128, 128, 255, 128, 128, 128, 255]);
    // A box larger than the image keeps it as it is.
    assert_eq!(decode(&bytes, Fit::Within(100, 100)).unwrap().width, 4);
}

#[test]
fn a_jpeg_with_a_display_p3_profile_comes_out_in_srgb_at_the_size_drawn() {
    let bytes = wallpaper();
    assert_eq!(size(&bytes), Ok((5120, 2880)));
    let started = Instant::now();
    let image = decode(&bytes, Fit::Within(1280, 800)).unwrap();
    let took = started.elapsed();
    assert_eq!((image.width, image.height), (1280, 720));
    assert_eq!(image.natural, (5120, 2880));
    assert_eq!(image.pixels.len(), 1280 * 720 * 4);
    assert!(image.pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
    // The profile was applied: the same pixels read as sRGB come out otherwise.
    let (width, height, raw, profile) = decode_rgba(&bytes).unwrap();
    assert!(profile.is_some(), "the wallpaper carries its profile");
    let unconverted = reduce(&raw, (width, height), (1280, 720));
    assert_ne!(unconverted, image.pixels);
    eprintln!("5120x2880 JPEG decoded to 1280x720 in {took:?}");
}

#[test]
fn what_is_not_an_image_is_refused() {
    assert_eq!(decode(b"GIF89a....", Fit::Whole), Err(Error::Unsupported));
    let bytes = png(2, 2, png::ColorType::Rgba, &[7; 16]);
    assert!(matches!(
        decode(&bytes[..bytes.len() - 20], Fit::Whole),
        Err(Error::Invalid(_))
    ));
    // A JPEG cut short shows what arrived, as Android's decoder does; a bare header is refused.
    let cut = decode(&wallpaper()[..400_000], Fit::Within(512, 512)).unwrap();
    assert_eq!((cut.width, cut.height), (512, 288));
    assert!(decode(&wallpaper()[..64], Fit::Whole).is_err());
}

#[test]
fn the_loader_answers_each_load_on_its_own_thread() {
    let directory = std::env::temp_dir().join(format!("oxyos-image-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("dot.png");
    std::fs::write(&file, png(1, 1, png::ColorType::Rgba, &[10, 20, 30, 255])).unwrap();
    let loader = Loader::new().unwrap();
    loader.load(1, file, Fit::Whole);
    loader.load(2, directory.join("missing.png"), Fit::Whole);
    let mut answers = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while answers.len() < 2 && Instant::now() < deadline {
        match loader.answer() {
            Some(answer) => answers.push(answer),
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    answers.sort_by_key(|answer| answer.id);
    assert_eq!(
        answers[0].result.as_ref().unwrap().pixels,
        [10, 20, 30, 255]
    );
    assert!(matches!(answers[1].result, Err(Error::Io(_))));
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn covering_keeps_both_sides_at_least_the_box() {
    // 5120 × 2880 to cover 1000 × 1000: the height decides.
    assert_eq!(Fit::Covering(1000, 1000).size(5120, 2880), (1778, 1000));
    assert_eq!(Fit::Within(1000, 1000).size(5120, 2880), (1000, 563));
    // Never enlarged, either way.
    assert_eq!(Fit::Covering(9000, 9000).size(5120, 2880), (5120, 2880));
    assert_eq!(Fit::Whole.size(5120, 2880), (5120, 2880));
}

/// Reduces as `reduce` did before it streamed its rows: every source row reduced across first,
/// then down. What `reduce` gives must be exactly this.
fn reduce_every_row_first(rgba: &[u8], from: (u32, u32), to: (u32, u32)) -> Vec<u8> {
    fn weights(source: u32, target: u32) -> Vec<Vec<(usize, f32)>> {
        let scale = source as f32 / target as f32;
        (0..target)
            .map(|i| {
                let (start, end) = (i as f32 * scale, (i + 1) as f32 * scale);
                let mut taps = Vec::new();
                let mut s = start.floor() as usize;
                while (s as f32) < end && s < source as usize {
                    let covered = (end.min(s as f32 + 1.0) - start.max(s as f32)).max(0.0);
                    if covered > 0.0 {
                        taps.push((s, covered / scale));
                    }
                    s += 1;
                }
                taps
            })
            .collect()
    }
    let (columns, rows) = (weights(from.0, to.0), weights(from.1, to.1));
    let (source_width, target_width) = (from.0 as usize, to.0 as usize);
    let mut across = vec![0f32; target_width * from.1 as usize * 4];
    for y in 0..from.1 as usize {
        let row = &rgba[y * source_width * 4..(y + 1) * source_width * 4];
        for (x, taps) in columns.iter().enumerate() {
            let out = &mut across[(y * target_width + x) * 4..][..4];
            for &(s, weight) in taps {
                for c in 0..4 {
                    out[c] += f32::from(row[s * 4 + c]) * weight;
                }
            }
        }
    }
    let mut out = vec![0u8; target_width * to.1 as usize * 4];
    for (y, taps) in rows.iter().enumerate() {
        for x in 0..target_width {
            let mut sum = [0f32; 4];
            for &(s, weight) in taps {
                let pixel = &across[(s * target_width + x) * 4..][..4];
                for c in 0..4 {
                    sum[c] += pixel[c] * weight;
                }
            }
            for c in 0..4 {
                out[(y * target_width + x) * 4 + c] = sum[c].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[test]
fn reducing_row_by_row_gives_the_same_pixels() {
    let (width, height, raw, _) = decode_rgba(&wallpaper()).unwrap();
    // An even ratio, an uneven one, and one axis kept either way.
    for to in [(1280, 720), (1000, 563), (5120, 1000), (37, 2880)] {
        assert_eq!(
            reduce(&raw, (width, height), to),
            reduce_every_row_first(&raw, (width, height), to),
            "reduced to {to:?}"
        );
    }
}

#[test]
fn converting_a_slice_at_a_time_gives_the_same_pixels() {
    let (_, _, raw, profile) = decode_rgba(&wallpaper()).unwrap();
    let profile = profile.unwrap();
    // More than a few slices, the last one short, with every alpha.
    let mut pixels: Vec<u8> = raw[..TRANSFORM_CHUNK * 5 + 1024].to_vec();
    for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        pixel[3] = i as u8;
    }
    let mut whole = pixels.clone();
    ColorProfile::new_from_slice(&profile)
        .unwrap()
        .create_transform_8bit(
            Layout::Rgba,
            &ColorProfile::new_srgb(),
            Layout::Rgba,
            TransformOptions::default(),
        )
        .unwrap()
        .transform(&pixels, &mut whole)
        .unwrap();
    to_srgb(&mut pixels, &profile).unwrap();
    assert_eq!(pixels, whole);
}

#[test]
fn a_jpeg_drawn_smaller_is_decoded_smaller_and_looks_the_same() {
    let bytes = wallpaper();
    // The smallest of a half, a quarter and an eighth of 5120 × 2880 still covering the target.
    for (target, decoded) in [
        ((1280, 720), (1280, 720)),
        ((1000, 563), (1280, 720)),
        ((2000, 1200), (2560, 1440)),
        ((300, 100), (640, 360)),
    ] {
        let (width, height, _, profile) = decode_jpeg_reduced(&bytes, target).unwrap();
        assert_eq!((width, height), decoded, "for {target:?}");
        assert!(profile.is_some(), "the profile goes with it");
    }
    // Within a level of what decoding it whole and averaging gives, on average: the same picture.
    let (_, _, small, _) = decode_jpeg_reduced(&bytes, (1280, 720)).unwrap();
    let (width, height, whole, _) = decode_rgba(&bytes).unwrap();
    let reference = reduce(&whole, (width, height), (1280, 720));
    let difference: u64 = small
        .iter()
        .zip(&reference)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    let mean = difference as f64 / small.len() as f64;
    eprintln!("decoded at a quarter: {mean:.2} levels from decoded whole, on average");
    assert!(mean < 1.0, "{mean:.2} levels apart on average");
    // `decode` takes that way, and keeps the image's own size as its natural one.
    let image = decode(&bytes, Fit::Within(1280, 800)).unwrap();
    assert_eq!(
        (image.width, image.height, image.natural),
        (1280, 720, (5120, 2880))
    );
    // A JPEG cut short is the whole decode's, which shows what arrived.
    assert!(decode_jpeg_reduced(&bytes[..400_000], (512, 288)).is_none());
}
