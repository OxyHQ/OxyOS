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
    let directory = std::env::temp_dir().join(format!("oxy-image-{}", std::process::id()));
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
