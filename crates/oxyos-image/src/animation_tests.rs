use super::*;
use crate::Error;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

/// A GIF of `width` × `height` with a palette of red, green, blue and a fourth colour that is
/// transparent, its frames each `(left, top, width, height, indices, disposal, delay)`.
type GifFrame = (u16, u16, u16, u16, Vec<u8>, gif::DisposalMethod, u16);

fn gif_of(width: u16, height: u16, repeat: Option<gif::Repeat>, frames: &[GifFrame]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let palette = [255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9];
        let mut encoder = gif::Encoder::new(&mut bytes, width, height, &palette).unwrap();
        if let Some(repeat) = repeat {
            encoder.set_repeat(repeat).unwrap();
        }
        for (left, top, w, h, indices, dispose, delay) in frames {
            let mut frame = gif::Frame::from_indexed_pixels(*w, *h, indices.clone(), Some(3));
            (frame.left, frame.top, frame.dispose, frame.delay) = (*left, *top, *dispose, *delay);
            encoder.write_frame(&frame).unwrap();
        }
    }
    bytes
}

fn frame(image: &Image, n: usize) -> Vec<[u8; 4]> {
    let size = image.width as usize * image.height as usize * 4;
    image.pixels[n * size..][..size]
        .chunks_exact(4)
        .map(|pixel| [pixel[0], pixel[1], pixel[2], pixel[3]])
        .collect()
}

/// Four frames on a 3 × 1 canvas, each disposed of another way.
fn disposals() -> Vec<u8> {
    use gif::DisposalMethod::{Any, Background, Keep, Previous};
    gif_of(
        3,
        1,
        Some(gif::Repeat::Finite(2)),
        &[
            // Red across, kept.
            (0, 0, 3, 1, vec![0, 0, 0], Keep, 0),
            // Green in the middle, then cleared to transparent.
            (1, 0, 1, 1, vec![1], Background, 1),
            // Blue at the right, then the canvas put back as it was before it.
            (2, 0, 1, 1, vec![2], Previous, 2),
            // Green at the right; its transparent pixels show what is below.
            (0, 0, 3, 1, vec![3, 3, 1], Any, 7),
        ],
    )
}

#[test]
fn a_gifs_frames_are_composed_and_disposed_of_as_they_are_shown() {
    let image = decode(&disposals(), Fit::Whole).unwrap();
    assert_eq!((image.width, image.height, image.frames()), (3, 1, 4));
    assert_eq!(image.pixels.len(), 4 * 3 * 4);
    assert_eq!(frame(&image, 0), [RED, RED, RED]);
    assert_eq!(frame(&image, 1), [RED, GREEN, RED]);
    assert_eq!(frame(&image, 2), [RED, CLEAR, BLUE]);
    assert_eq!(frame(&image, 3), [RED, CLEAR, GREEN]);
    // 0 and 1 hundredths are 100 ms; others as given, in milliseconds.
    assert_eq!(image.delays, [100, 100, 20, 70]);
    // A NETSCAPE count of 2: it repeats twice, so plays three times.
    assert_eq!(image.loops, 3);
    // Its first frame is the still image's.
    let still = crate::decode(&disposals(), Fit::Whole).unwrap();
    assert_eq!(still.pixels, frame(&image, 0).concat());
    assert_eq!(
        (still.frames(), still.delays.as_slice(), still.loops),
        (1, &[0][..], 1)
    );
}

#[test]
fn a_gif_plays_once_without_a_loop_count_and_forever_with_zero() {
    use gif::DisposalMethod::Keep;
    let frames = [
        (0, 0, 1, 1, vec![0], Keep, 10),
        (0, 0, 1, 1, vec![1], Keep, 10),
    ];
    for (repeat, loops) in [(None, 1), (Some(gif::Repeat::Infinite), 0)] {
        let image = decode(&gif_of(1, 1, repeat, &frames), Fit::Whole).unwrap();
        assert_eq!((image.frames(), image.loops), (2, loops), "{repeat:?}");
    }
}

#[test]
fn each_frame_is_reduced_to_the_size_drawn() {
    use gif::DisposalMethod::Keep;
    let frames = [
        (0, 0, 4, 2, vec![0; 8], Keep, 10),
        (0, 0, 2, 2, vec![1; 4], Keep, 10),
    ];
    let image = decode(&gif_of(4, 2, None, &frames), Fit::Within(2, 2)).unwrap();
    assert_eq!((image.width, image.height, image.natural), (2, 1, (4, 2)));
    assert_eq!(frame(&image, 0), [RED, RED]);
    // Green over the left half of red: the left pixel green, the right red.
    assert_eq!(frame(&image, 1), [GREEN, RED]);
}

#[test]
fn a_gif_cut_short_shows_the_frames_that_arrived() {
    let bytes = disposals();
    let whole = decode(&bytes, Fit::Whole).unwrap();
    // Cut inside its last frame.
    let cut = decode(&bytes[..bytes.len() - 6], Fit::Whole).unwrap();
    assert!(cut.frames() >= 2 && cut.frames() < 4, "{cut:?}");
    assert_eq!(cut.pixels[..], whole.pixels[..cut.pixels.len()]);
    assert!(matches!(
        decode(b"GIF89a....", Fit::Whole),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn an_animation_too_large_to_hold_decoded_is_its_first_frame_still() {
    // 16 frames of 1024 × 1024 are 64 MiB: they fit; 17 do not.
    assert!(fits(16, (1024, 1024)) && !fits(17, (1024, 1024)));
    assert!(!fits(u64::MAX, (1, 1)));
    use gif::DisposalMethod::Keep;
    let frames: Vec<GifFrame> = (0..17)
        .map(|n| (n, 0, 1, 1, vec![(n % 3) as u8], Keep, 10))
        .collect();
    let bytes = gif_of(1024, 1024, None, &frames);
    let still = decode(&bytes, Fit::Whole).unwrap();
    assert_eq!((still.frames(), still.delays.as_slice()), (1, &[0][..]));
    assert_eq!(still.pixels.len(), 1024 * 1024 * 4);
    assert_eq!(still.pixels[..4], RED);
    // Drawn at a quarter of its size, they fit.
    let animated = decode(&bytes, Fit::Within(512, 512)).unwrap();
    assert_eq!((animated.width, animated.frames()), (512, 17));
}

#[test]
fn a_still_image_asked_for_animated_is_still() {
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, 1, 1);
    encoder.set_color(png::ColorType::Rgba);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&RED)
        .unwrap();
    let image = crate::decode_as(&png, Fit::Whole, true).unwrap();
    assert_eq!(
        (image.frames(), image.delays.as_slice(), image.loops),
        (1, &[0][..], 1)
    );
    let single = gif_of(
        1,
        1,
        Some(gif::Repeat::Infinite),
        &[(0, 0, 1, 1, vec![2], gif::DisposalMethod::Keep, 50)],
    );
    let image = crate::decode_as(&single, Fit::Whole, true).unwrap();
    assert_eq!((image.frames(), image.pixels.as_slice()), (1, &BLUE[..]));
}

/// A lossless WebP bitstream (`VP8L`) of `rgba`, from image-webp's encoder.
fn vp8l(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut file = Vec::new();
    image_webp::WebPEncoder::new(&mut file)
        .encode(rgba, width, height, image_webp::ColorType::Rgba8)
        .unwrap();
    let (tag, data) = chunks(&file[12..]).next().unwrap();
    assert_eq!(tag, b"VP8L");
    data.to_vec()
}

/// An `ANMF` chunk's data: a frame at (`left`, `top`) lasting `duration` ms, blended over what is
/// below unless `blend` is false, disposed of after if `dispose` is set.
fn anmf(
    left: u32,
    top: u32,
    width: u32,
    rgba: &[u8],
    duration: u32,
    blend: bool,
    dispose: bool,
) -> Vec<u8> {
    let height = (rgba.len() / 4) as u32 / width;
    let mut data = Vec::new();
    for value in [left / 2, top / 2, width - 1, height - 1, duration] {
        data.extend_from_slice(&value.to_le_bytes()[..3]);
    }
    data.push((u8::from(!blend) << 1) | u8::from(dispose));
    let bitstream = vp8l(width, height, rgba);
    data.extend_from_slice(b"VP8L");
    data.extend_from_slice(&(bitstream.len() as u32).to_le_bytes());
    data.extend_from_slice(&bitstream);
    if bitstream.len() & 1 == 1 {
        data.push(0);
    }
    data
}

/// An animated WebP of `width` × `height` looping `loops` times (0: forever), its background
/// colour hint white, which is not used.
fn animated_webp(width: u32, height: u32, loops: u16, frames: &[Vec<u8>]) -> Vec<u8> {
    let mut header = vec![0x12, 0, 0, 0];
    header.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    header.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
    let mut anim = vec![255, 255, 255, 255];
    anim.extend_from_slice(&loops.to_le_bytes());
    let mut all: Vec<(&[u8; 4], &[u8])> = vec![(b"VP8X", &header), (b"ANIM", &anim)];
    all.extend(frames.iter().map(|frame| (b"ANMF", frame.as_slice())));
    riff(&all)
}

#[test]
fn a_webps_frames_are_blended_and_disposed_of_as_libwebp_composes_them() {
    let half_green = [0, 255, 0, 128];
    let bytes = animated_webp(
        4,
        1,
        3,
        &[
            // Red across, not blended.
            anmf(0, 0, 4, &RED.repeat(4), 0, false, false),
            // Half-transparent green and a transparent pixel over the right half, blended, then
            // disposed of: that half cleared to transparent.
            anmf(2, 0, 2, &[half_green, CLEAR].concat(), 50, true, true),
            // Blue and a transparent pixel there, not blended: the transparent pixel replaces.
            anmf(2, 0, 2, &[BLUE, [0, 0, 255, 0]].concat(), 30, false, false),
        ],
    );
    let image = decode(&bytes, Fit::Whole).unwrap();
    assert_eq!((image.width, image.height, image.frames()), (4, 1, 3));
    assert_eq!(frame(&image, 0), [RED, RED, RED, RED]);
    // Half green over red: as much of each, opaque.
    assert_eq!(frame(&image, 1), [RED, RED, [127, 128, 0, 255], RED]);
    assert_eq!(frame(&image, 2), [RED, RED, BLUE, CLEAR]);
    // A duration of 0 is 100 ms; the loop count is how many times it plays.
    assert_eq!(
        (image.delays.as_slice(), image.loops),
        (&[100, 50, 30][..], 3)
    );
    // Its first frame is the still image's.
    let still = crate::decode(&bytes, Fit::Whole).unwrap();
    assert_eq!(
        (still.frames(), still.pixels),
        (1, frame(&image, 0).concat())
    );
    let forever = animated_webp(
        1,
        1,
        0,
        &[
            anmf(0, 0, 1, &RED, 10, true, false),
            anmf(0, 0, 1, &BLUE, 10, true, false),
        ],
    );
    assert_eq!(decode(&forever, Fit::Whole).unwrap().loops, 0);
}

#[test]
fn blending_over_rounds_to_the_nearest_level_and_keeps_the_ends_exact() {
    assert_eq!(
        over(&[10, 20, 30, 255], &[200, 200, 200, 255]),
        [10, 20, 30, 255]
    );
    assert_eq!(
        over(&[10, 20, 30, 0], &[200, 200, 200, 99]),
        [200, 200, 200, 99]
    );
    assert_eq!(
        over(&[10, 20, 30, 77], &[200, 200, 200, 0]),
        [10, 20, 30, 77]
    );
    // Half over half: three quarters opaque, the source's colour weighing twice the other's.
    assert_eq!(
        over(&[255, 0, 0, 128], &[0, 0, 255, 128]),
        [170, 0, 85, 192]
    );
}
