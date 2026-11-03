use super::*;

// The expected pixels are what expo-image's Kotlin decoders give: `BlurhashDecoder.kt` and
// `ThumbhashDecoder.kt` transcribed to Java with Kotlin's arithmetic (`Float.pow` a `Math.pow`
// rounded to a float) and run on the JVM. Wolt's `blurhash` and Evan Wallace's `thumbhash` npm
// packages, in 64-bit doubles, give the same BlurHash pixels and ThumbHash pixels within a level.
// The hashes were made by those packages' encoders from gradients, opaque and with transparency.

const GRADIENT_4X3: &str = "LEHewW35|aO?z3NIo2aMhTf$fOf$";
const GRADIENT_9X9: &str = "|FHewX34|cKho0o|SMxtN^z3NIo0aMfPi|a{i{a{hBfifRfifRfifRfifR#@N@o0a{fPjra{jra{eYf6fRf6fRf6fSf6fS%MO9o0bDfPj;a{j;a}d@f8fNf8fMf8fMf7fL%fOEo0bIfPj]a{j^a_d]f7fPf6fPf6fPf6fO";
/// The BlurHash of BlurHash's own README.
const README: &str = "LEHV6nWB2yk8pyo0adR*.7kCMdnj";

fn rgb(image: &Image) -> Vec<u8> {
    image
        .pixels
        .chunks_exact(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect()
}

fn sum(bytes: &[u8]) -> u64 {
    bytes.iter().map(|&byte| u64::from(byte)).sum()
}

#[test]
fn a_blurhash_decodes_to_the_pixels_expo_images_decoder_gives() {
    let image = blurhash(README, 7, 5).unwrap();
    assert_eq!((image.width, image.height, image.natural), (7, 5, (7, 5)));
    assert_eq!((image.frames(), image.loops), (1, 1));
    assert!(image.pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
    #[rustfmt::skip]
    let expected = [
        135, 164, 177, 146, 167, 177, 166, 175, 176, 180, 179, 173, 179, 178, 171, 164, 173, 173,
        146, 168, 178, 130, 160, 174, 140, 161, 172, 160, 163, 166, 173, 165, 159, 171, 165, 157,
        158, 164, 164, 141, 162, 174, 121, 152, 166, 131, 148, 160, 148, 139, 143, 160, 134, 125,
        159, 137, 125, 147, 144, 143, 133, 150, 163, 122, 145, 157, 130, 140, 149, 146, 130, 127,
        158, 124, 106, 159, 128, 106, 148, 137, 129, 133, 143, 152, 131, 144, 150, 138, 143, 144,
        154, 142, 130, 169, 143, 118, 172, 146, 117, 160, 146, 131, 140, 143, 145,
    ];
    assert_eq!(rgb(&image), expected);
    // One component: its colour alone, everywhere.
    assert_eq!(
        rgb(&blurhash("00HoE?", 2, 2).unwrap()),
        [153, 152, 144].repeat(4)
    );
    // Nine by nine components, at 32 × 32.
    let large = rgb(&blurhash(GRADIENT_9X9, 32, 32).unwrap());
    assert_eq!(sum(&large), 458_363);
    assert_eq!(large[..3], [158, 148, 135]);
    assert_eq!(large[(32 * 16 + 16) * 3..][..3], [127, 130, 153]);
    assert_eq!(large[1023 * 3..], [152, 138, 90]);
    assert_eq!(
        blurhash(GRADIENT_4X3, 16, 9).unwrap().pixels.len(),
        16 * 9 * 4
    );
}

#[test]
fn an_invalid_blurhash_is_refused_with_expo_images_message() {
    for hash in [
        "",
        "LEHV6",
        "LEHV6nWB2yk8pyo0adR*.7kCMdn",
        "LEHV6nWB2yk8pyo0adR*.7kCMdnjj",
    ] {
        assert_eq!(
            blurhash(hash, 4, 4),
            Err(Error::Source(format!(
                "Cannot decode provided blurhash '{hash}'"
            )))
        );
    }
    // No pixels to decode into.
    assert!(blurhash(README, 0, 4).is_err());
    // A character that is not a digit is skipped, as Kotlin's decode83 skips it: still decoded.
    assert!(blurhash("LEHV6nWB2yk8pyo0adR*.7kCMdn\u{e9}", 2, 2).is_ok());
}

#[test]
fn a_thumbhash_decodes_at_its_own_size_to_the_pixels_expo_images_decoder_gives() {
    // Its hash, size, sum of its straight RGBA, and its first, middle and last pixels.
    for (hash, size, total, first, middle, last) in [
        (
            "4AcKNZpwd4dweIeHd3h3eICACPh3",
            (32, 23),
            473_175,
            [6, 11, 122, 255],
            [132, 122, 136, 255],
            [240, 244, 113, 255],
        ),
        (
            "3wcKNBpwB3d3eHd3eHeAgQj4dw==",
            (18, 32),
            363_762,
            [0, 1, 104, 255],
            [135, 126, 135, 255],
            [216, 221, 97, 255],
        ),
        (
            "YkiGFIgl42awiXh4SA6KtF8IQI5IioeICA==",
            (32, 26),
            420_319,
            [168, 102, 127, 0],
            [126, 113, 124, 212],
            [223, 200, 144, 246],
        ),
        (
            "o1iGGwgl1LCJd4jUsFn6hEComJRpeIc=",
            (19, 32),
            310_329,
            [182, 113, 130, 12],
            [141, 115, 135, 132],
            [230, 216, 116, 228],
        ),
    ] {
        let bytes = crate::data::base64(hash.as_bytes()).unwrap();
        let (width, height, rgba) = thumbhash_rgba(&bytes).unwrap();
        assert_eq!((width, height), size, "{hash}");
        let pixel = |at: u32| &rgba[at as usize * 4..][..4];
        assert_eq!(sum(&rgba), total, "{hash}");
        assert_eq!(pixel(0), first, "{hash}");
        assert_eq!(pixel(height / 2 * width + width / 2), middle, "{hash}");
        assert_eq!(pixel(width * height - 1), last, "{hash}");
        // As an image: premultiplied, at its own size, and reduced as asked; its padding and
        // URL-safe alphabet optional.
        let image = thumbhash(hash.trim_end_matches('='), Fit::Whole).unwrap();
        assert_eq!(
            (image.width, image.height, image.natural),
            (size.0, size.1, size)
        );
        let mut premultiplied = rgba.clone();
        crate::premultiply(&mut premultiplied);
        assert_eq!(image.pixels, premultiplied, "{hash}");
        let small =
            thumbhash(&hash.replace('+', "-").replace('/', "_"), Fit::Within(8, 8)).unwrap();
        assert!(small.width <= 8 && small.height <= 8 && small.natural == size);
    }
}

#[test]
fn an_invalid_thumbhash_is_refused_with_expo_images_message() {
    assert_eq!(
        thumbhash("not*base64", Fit::Whole),
        Err(Error::Source(
            "Cannot decode provided thumbhash 'not*base64' (not base64)".into()
        ))
    );
    // Cut short: its header says it holds more coefficients than it has.
    for hash in ["", "4AcKNZ", "4AcKNZpwd4dweIeHd3h3"] {
        assert_eq!(
            thumbhash(hash, Fit::Whole),
            Err(Error::Source(format!(
                "Cannot decode provided thumbhash '{hash}' (shorter than its header says)"
            )))
        );
    }
}
