//! Images for applications on OxyOS: the decoding behind React Native's `Image` in the Bloom host
//! (docs/react-native-android-adaptation.mdx), through the C ABI (`oxyos-client-ffi`).
//!
//! PNG, JPEG, WebP and GIF are decoded in Rust (image-rs's png, image-webp and gif, the project
//! whose png crate Chromium's Skia decodes PNG with; zune-jpeg), turned from the profile they carry
//! (an embedded ICC profile, Display P3 say) into sRGB, the space the compositor composes in, and
//! reduced to the size they are drawn at by averaging the source pixels each output pixel covers.
//! Pixels come out as premultiplied RGBA, 8 bits a channel. An image without a profile is taken as
//! sRGB, as Android takes it. PNG's gamma and chromaticity chunks are not applied. A truncated JPEG
//! shows what arrived, as Android's decoder (Skia) shows it; a damaged PNG is an error. An animated
//! GIF or WebP shows its first frame, still (as React Native's Android `Image` does without
//! Fresco's animated decoders), unless it is asked for animated (expo-image's `Image`): then every
//! frame, composed as it is shown ([`animation`]). AVIF is recognised and refused with that reason:
//! no AV1 decoder is part of OxyOS.
//!
//! Remote images (`http`, `https`) are fetched through the application's network, with a cache in
//! memory and its HTTP cache on disk ([`remote`]). `data:` URIs ([`data`]), BlurHashes and
//! ThumbHashes ([`placeholder`]) are decoded as expo-image's Android implementation decodes them.
//!
//! A [`Loader`] decodes on threads of its own and queues what it decoded, which the caller takes
//! from its own thread ([`Loader::answer`]); its descriptor is readable while any are queued.

pub mod animation;
pub mod data;
pub mod placeholder;
pub mod remote;

use moxcms::{ColorProfile, Layout, TransformOptions};
use remote::Want;
use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Cursor, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The largest image decoded, in pixels (a 16384 × 16384 image, 1 GiB decoded).
pub const MAX_PIXELS: u64 = 16384 * 16384;

/// A decoded image: `width` × `height` premultiplied RGBA pixels, rows top to bottom, reduced
/// from the image's own `natural` size. An animated one decoded as such has more than one frame,
/// each `width` × `height` and composed as it is shown, one after the other in `pixels`.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub natural: (u32, u32),
    pub pixels: Vec<u8>,
    /// Each frame's duration in milliseconds, one a frame: a still image's is a single 0.
    pub delays: Vec<u32>,
    /// How many times its frames play, 0 for forever; a still image's is 1.
    pub loops: u32,
}

impl Image {
    /// A still image of `pixels`.
    pub fn still(width: u32, height: u32, natural: (u32, u32), pixels: Vec<u8>) -> Image {
        Image {
            width,
            height,
            natural,
            pixels,
            delays: vec![0],
            loops: 1,
        }
    }

    /// How many frames it has: 1 for a still image.
    pub fn frames(&self) -> usize {
        self.delays.len()
    }
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Image({}x{}", self.width, self.height)?;
        if self.frames() > 1 {
            write!(f, ", {} frames", self.frames())?;
        }
        write!(f, ")")
    }
}

/// Where an image's bytes came from, as expo-image's `onLoad` says (its `cacheType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Origin {
    /// Their source itself: the network, a file, a `data:` URI, a hash.
    #[default]
    Source,
    /// The HTTP cache on disk, fresh or revalidated (a `304`).
    Disk,
    /// The memory cache of encoded images.
    Memory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not a PNG, JPEG, WebP or GIF image.
    Unsupported,
    /// An AVIF image, which OxyOS cannot decode.
    Avif,
    /// The file is damaged, or its profile cannot be applied.
    Invalid(String),
    /// Larger than [`MAX_PIXELS`].
    TooLarge { width: u32, height: u32 },
    /// The file could not be read.
    Io(String),
    /// A remote image could not be fetched: refused, failed, or answered with another status.
    Fetch(String),
    /// A BlurHash or ThumbHash that cannot be decoded, or a `data:` URI that cannot be read,
    /// with expo-image's message.
    Source(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported => write!(f, "not a PNG, JPEG, WebP or GIF image"),
            Error::Avif => write!(f, "AVIF images are not supported on OxyOS"),
            Error::Invalid(reason) => write!(f, "invalid image: {reason}"),
            Error::TooLarge { width, height } => write!(f, "image too large: {width}x{height}"),
            Error::Io(reason) => write!(f, "cannot read the image: {reason}"),
            Error::Fetch(reason) | Error::Source(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for Error {}

enum Format {
    Png,
    Jpeg,
    Webp,
    Gif,
}

fn format(bytes: &[u8]) -> Result<Format, Error> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok(Format::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok(Format::Jpeg)
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Ok(Format::Webp)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Ok(Format::Gif)
    } else if bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && matches!(&bytes[8..12], b"avif" | b"avis")
    {
        Err(Error::Avif)
    } else {
        Err(Error::Unsupported)
    }
}

/// A GIF's decoder, its colours expanded to RGBA.
fn gif_decoder(bytes: &[u8]) -> Result<gif::Decoder<Cursor<&[u8]>>, Error> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    options.read_info(Cursor::new(bytes)).map_err(invalid)
}

fn invalid(error: impl fmt::Display) -> Error {
    Error::Invalid(error.to_string())
}

fn checked(width: u32, height: u32) -> Result<(), Error> {
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(Error::TooLarge { width, height });
    }
    Ok(())
}

/// The image's size in pixels, from its headers alone.
pub fn size(bytes: &[u8]) -> Result<(u32, u32), Error> {
    match format(bytes)? {
        Format::Png => {
            let reader = png::Decoder::new(Cursor::new(bytes))
                .read_info()
                .map_err(invalid)?;
            let info = reader.info();
            Ok((info.width, info.height))
        }
        Format::Jpeg => {
            let mut decoder =
                zune_jpeg::JpegDecoder::new(zune_jpeg::zune_core::bytestream::ZCursor::new(bytes));
            decoder.decode_headers().map_err(invalid)?;
            let info = decoder.info().ok_or_else(|| invalid("no JPEG frame"))?;
            Ok((u32::from(info.width), u32::from(info.height)))
        }
        Format::Webp => Ok(image_webp::WebPDecoder::new(Cursor::new(bytes))
            .map_err(invalid)?
            .dimensions()),
        Format::Gif => {
            let decoder = gif_decoder(bytes)?;
            Ok((u32::from(decoder.width()), u32::from(decoder.height())))
        }
    }
}

/// Straight (not premultiplied) RGBA in the image's own colour space: its width, height, pixels
/// and ICC profile, if it has one.
type Decoded = (u32, u32, Vec<u8>, Option<Vec<u8>>);

fn decode_rgba(bytes: &[u8]) -> Result<Decoded, Error> {
    match format(bytes)? {
        Format::Png => {
            let mut decoder = png::Decoder::new(Cursor::new(bytes));
            decoder
                .set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
            let mut reader = decoder.read_info().map_err(invalid)?;
            let (width, height) = (reader.info().width, reader.info().height);
            checked(width, height)?;
            let profile = reader
                .info()
                .icc_profile
                .as_ref()
                .map(|profile| profile.to_vec());
            let mut buffer = vec![
                0;
                reader
                    .output_buffer_size()
                    .ok_or_else(|| invalid("PNG too large"))?
            ];
            let frame = reader.next_frame(&mut buffer).map_err(invalid)?;
            let samples = &buffer[..frame.buffer_size()];
            let rgba: Vec<u8> = match frame.color_type {
                png::ColorType::Rgba => samples.to_vec(),
                png::ColorType::Rgb => samples
                    .chunks_exact(3)
                    .flat_map(|p| [p[0], p[1], p[2], 255])
                    .collect(),
                png::ColorType::GrayscaleAlpha => samples
                    .chunks_exact(2)
                    .flat_map(|p| [p[0], p[0], p[0], p[1]])
                    .collect(),
                png::ColorType::Grayscale => samples.iter().flat_map(|&g| [g, g, g, 255]).collect(),
                png::ColorType::Indexed => return Err(invalid("PNG palette not expanded")),
            };
            Ok((width, height, rgba, profile))
        }
        Format::Jpeg => {
            let options = zune_jpeg::zune_core::options::DecoderOptions::default()
                .jpeg_set_out_colorspace(zune_jpeg::zune_core::colorspace::ColorSpace::RGBA);
            let mut decoder = zune_jpeg::JpegDecoder::new_with_options(
                zune_jpeg::zune_core::bytestream::ZCursor::new(bytes),
                options,
            );
            decoder.decode_headers().map_err(invalid)?;
            let info = decoder.info().ok_or_else(|| invalid("no JPEG frame"))?;
            let (width, height) = (u32::from(info.width), u32::from(info.height));
            checked(width, height)?;
            let profile = decoder.icc_profile();
            let rgba = decoder.decode().map_err(invalid)?;
            if rgba.len() != width as usize * height as usize * 4 {
                return Err(invalid("JPEG decoded to an unexpected size"));
            }
            Ok((width, height, rgba, profile))
        }
        Format::Webp => {
            let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(invalid)?;
            let (width, height) = decoder.dimensions();
            checked(width, height)?;
            let profile = decoder.icc_profile().map_err(invalid)?;
            let mut samples = vec![
                0;
                decoder
                    .output_buffer_size()
                    .ok_or_else(|| invalid("WebP too large"))?
            ];
            // The first frame of an animated one.
            decoder.read_image(&mut samples).map_err(invalid)?;
            let rgba = if decoder.has_alpha() {
                samples
            } else {
                samples
                    .chunks_exact(3)
                    .flat_map(|p| [p[0], p[1], p[2], 255])
                    .collect()
            };
            Ok((width, height, rgba, profile))
        }
        Format::Gif => {
            // The first frame, placed on the image's canvas (transparent where it does not
            // reach), as a GIF shows before it animates.
            let mut decoder = gif_decoder(bytes)?;
            let (width, height) = (u32::from(decoder.width()), u32::from(decoder.height()));
            checked(width, height)?;
            let mut rgba = vec![0u8; width as usize * height as usize * 4];
            let frame = decoder
                .read_next_frame()
                .map_err(invalid)?
                .ok_or_else(|| invalid("no GIF frame"))?;
            let (left, top) = (usize::from(frame.left), usize::from(frame.top));
            let frame_width = usize::from(frame.width);
            for (y, row) in frame.buffer.chunks_exact(frame_width * 4).enumerate() {
                let y = top + y;
                if y >= height as usize || left >= width as usize {
                    continue;
                }
                let columns = frame_width.min(width as usize - left);
                let at = (y * width as usize + left) * 4;
                rgba[at..at + columns * 4].copy_from_slice(&row[..columns * 4]);
            }
            Ok((width, height, rgba, None))
        }
    }
}

/// Converts straight RGBA from `profile` to sRGB in place; alpha is kept.
fn to_srgb(rgba: &mut [u8], profile: &[u8]) -> Result<(), Error> {
    let source = ColorProfile::new_from_slice(profile)
        .map_err(|error| invalid(format!("ICC profile: {error:?}")))?;
    let transform = source
        .create_transform_8bit(
            Layout::Rgba,
            &ColorProfile::new_srgb(),
            Layout::Rgba,
            TransformOptions::default(),
        )
        .map_err(|error| invalid(format!("ICC profile: {error:?}")))?;
    // A slice at a time through a copy of its own, never a copy of the whole image.
    let mut source = Vec::with_capacity(TRANSFORM_CHUNK);
    for chunk in rgba.chunks_mut(TRANSFORM_CHUNK) {
        source.clear();
        source.extend_from_slice(chunk);
        transform
            .transform(&source, chunk)
            .map_err(|error| invalid(format!("ICC transform: {error:?}")))?;
    }
    Ok(())
}

/// The bytes of RGBA (whole pixels) converted at a time by `to_srgb`.
const TRANSFORM_CHUNK: usize = 64 * 1024;

fn premultiply(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
    }
}

/// Reduces premultiplied RGBA from `from` to `to` pixels (no larger on either axis), each output
/// pixel the average of the source area it covers.
fn reduce(rgba: &[u8], from: (u32, u32), to: (u32, u32)) -> Vec<u8> {
    // The share of each source index each output index takes, along one axis.
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
    // Across first, row by row, then down. Each output row takes source rows that follow the
    // previous output row's, the first of them at most its last: a source row is reduced across
    // once, as it is first needed, and kept only while the next output row may need it too. A
    // wallpaper of 2880 × 5120 reduced to 1280 wide would otherwise hold every source row reduced
    // across at once, in floats: 100 MiB.
    let across = |s: usize, into: &mut Vec<f32>| {
        into.clear();
        into.resize(target_width * 4, 0.0);
        let row = &rgba[s * source_width * 4..(s + 1) * source_width * 4];
        for (x, taps) in columns.iter().enumerate() {
            let out = &mut into[x * 4..][..4];
            for &(s, weight) in taps {
                for c in 0..4 {
                    out[c] += f32::from(row[s * 4 + c]) * weight;
                }
            }
        }
    };
    let mut out = vec![0u8; target_width * to.1 as usize * 4];
    // The source rows reduced across that the current output row takes, by source row.
    let mut kept: Vec<(usize, Vec<f32>)> = Vec::new();
    for (y, taps) in rows.iter().enumerate() {
        kept.retain(|(s, _)| taps.iter().any(|&(t, _)| t == *s));
        for &(s, _) in taps {
            if !kept.iter().any(|(k, _)| *k == s) {
                let mut row = Vec::new();
                across(s, &mut row);
                kept.push((s, row));
            }
        }
        for x in 0..target_width {
            let mut sum = [0f32; 4];
            for &(s, weight) in taps {
                let row = &kept
                    .iter()
                    .find(|(k, _)| *k == s)
                    .expect("reduced across")
                    .1;
                let pixel = &row[x * 4..][..4];
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

/// The size an image is wanted at, in pixels: it is reduced as far as that allows, keeping its
/// proportions, and never enlarged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Its own size.
    Whole,
    /// Within the box, both sides at most the box's (`resizeMode: contain`).
    Within(u32, u32),
    /// Covering the box, both sides at least the box's (`resizeMode: cover`, `stretch`).
    Covering(u32, u32),
}

impl Fit {
    fn size(self, width: u32, height: u32) -> (u32, u32) {
        let (box_width, box_height, cover) = match self {
            Fit::Whole => return (width, height),
            Fit::Within(w, h) => (w, h, false),
            Fit::Covering(w, h) => (w, h, true),
        };
        if box_width == 0 || box_height == 0 {
            return (width, height);
        }
        let (across, down) = (
            box_width as f64 / width as f64,
            box_height as f64 / height as f64,
        );
        let scale = if cover {
            across.max(down)
        } else {
            across.min(down)
        };
        if scale >= 1.0 {
            return (width, height);
        }
        (
            ((width as f64 * scale).round() as u32).clamp(1, width),
            ((height as f64 * scale).round() as u32).clamp(1, height),
        )
    }
}

/// Decodes an image into sRGB, premultiplied, at the size `fit` asks for.
pub fn decode(bytes: &[u8], fit: Fit) -> Result<Image, Error> {
    // A JPEG wanted at half its size or less is decoded smaller (`decode_jpeg_reduced`); any
    // other image, or a JPEG the scaled decoder cannot read whole, is decoded whole.
    let reduced = match format(bytes)? {
        Format::Jpeg => {
            let natural = size(bytes)?;
            checked(natural.0, natural.1)?;
            let target = fit.size(natural.0, natural.1);
            (target.0 * 2 <= natural.0 && target.1 * 2 <= natural.1)
                .then(|| decode_jpeg_reduced(bytes, target))
                .flatten()
                .map(|decoded| (natural, decoded))
        }
        Format::Png | Format::Webp | Format::Gif => None,
    };
    let (natural, (width, height, rgba, profile)) = match reduced {
        Some(reduced) => reduced,
        None => {
            let decoded = decode_rgba(bytes)?;
            ((decoded.0, decoded.1), decoded)
        }
    };
    let target = fit.size(natural.0, natural.1);
    let pixels = finish(rgba, (width, height), target, profile.as_deref())?;
    Ok(Image::still(target.0, target.1, natural, pixels))
}

/// Straight RGBA of `size` in `profile`'s colour space (none: sRGB) made sRGB, premultiplied and
/// reduced to `target`.
fn finish(
    mut rgba: Vec<u8>,
    size: (u32, u32),
    target: (u32, u32),
    profile: Option<&[u8]>,
) -> Result<Vec<u8>, Error> {
    // An opaque image is reduced first and converted after, on far fewer pixels; one with
    // transparency is converted while its colours are still straight, then premultiplied.
    let opaque = rgba.chunks_exact(4).all(|pixel| pixel[3] == 255);
    if !opaque {
        if let Some(profile) = profile {
            to_srgb(&mut rgba, profile)?;
        }
        premultiply(&mut rgba);
    }
    let mut pixels = if target == size {
        rgba
    } else {
        reduce(&rgba, size, target)
    };
    if opaque {
        if let Some(profile) = profile {
            to_srgb(&mut pixels, profile)?;
        }
    }
    Ok(pixels)
}

/// Decodes an image as [`decode`], but an animated GIF or WebP when `animated` is set, which
/// comes out frame by frame ([`animation`]).
pub fn decode_as(bytes: &[u8], fit: Fit, animated: bool) -> Result<Image, Error> {
    if animated {
        animation::decode(bytes, fit)
    } else {
        decode(bytes, fit)
    }
}

/// A JPEG decoded at a half, a quarter or an eighth of its size from its DCT coefficients (as
/// libjpeg's scale-on-decode, jpeg-decoder's `scale`): the smallest of those still at least
/// `target` on both axes, as straight RGBA, with its ICC profile. A 5120 × 2880 wallpaper drawn
/// at 1280 × 720 is decoded at 1280 × 720 (3.5 MiB), never whole (56 MiB). None when the scaled
/// decoder cannot read it whole (a JPEG cut short, which the whole decode shows what arrived of)
/// or gives a colour format other than grey or RGB.
fn decode_jpeg_reduced(bytes: &[u8], target: (u32, u32)) -> Option<Decoded> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    let (width, height) = decoder
        .scale(u16::try_from(target.0).ok()?, u16::try_from(target.1).ok()?)
        .ok()?;
    let samples = decoder.decode().ok()?;
    let format = decoder.info()?.pixel_format;
    let (width, height) = (u32::from(width), u32::from(height));
    let pixels = width as usize * height as usize;
    let rgba: Vec<u8> = match format {
        jpeg_decoder::PixelFormat::RGB24 if samples.len() == pixels * 3 => samples
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        jpeg_decoder::PixelFormat::L8 if samples.len() == pixels => {
            samples.iter().flat_map(|&g| [g, g, g, 255]).collect()
        }
        _ => return None,
    };
    (width >= target.0 && height >= target.1).then(|| (width, height, rgba, decoder.icc_profile()))
}

/// Reads and decodes a file, as [`decode`].
pub fn decode_file(path: &std::path::Path, fit: Fit) -> Result<Image, Error> {
    decode(&read(path)?, fit)
}

fn read(path: &std::path::Path) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| Error::Io(error.to_string()))?;
    Ok(bytes)
}

/// What a load answered.
#[derive(Debug)]
pub enum Outcome {
    /// The image, decoded at the size it was wanted at, and where its bytes came from.
    Image(Image, Origin),
    /// Why there is none.
    Failed(Error),
    /// What arrived of a remote image so far, and its length (Content-Length), as it arrives.
    Progress { loaded: u64, total: u64 },
    /// A remote image or a `data:` URI this crate does not decode (an SVG document, by its
    /// `Content-Type` or media type, or as the caller sniffs it): its bytes, its type, where they
    /// came from, and the error decoding them gave.
    Undecoded {
        bytes: Arc<[u8]>,
        content_type: Option<String>,
        from: Origin,
        error: Error,
    },
    /// A remote image's size in pixels, from its headers (`Want::Size`).
    Size(u32, u32),
    /// A remote image fetched into the caches (`Want::Prefetch`).
    Fetched,
}

/// The pixels (an animated image's frames if `animated` is set), size or confirmation a load
/// wants of `bytes`, of the type `content_type` says, that came from `from`.
fn outcome(
    bytes: &Arc<[u8]>,
    content_type: Option<&str>,
    want: Want,
    animated: bool,
    from: Origin,
) -> Outcome {
    let undecoded = |error| Outcome::Undecoded {
        bytes: bytes.clone(),
        content_type: content_type.map(str::to_string),
        from,
        error,
    };
    // An SVG document is the host's to decode (Skia's SVG module), by its type or as it sniffs it.
    let svg = content_type.is_some_and(|kind| kind.to_ascii_lowercase().contains("image/svg"));
    match want {
        Want::Prefetch => Outcome::Fetched,
        _ if svg => undecoded(Error::Unsupported),
        Want::Size => match size(bytes) {
            Ok((width, height)) => Outcome::Size(width, height),
            Err(Error::Unsupported) => undecoded(Error::Unsupported),
            Err(error) => Outcome::Failed(error),
        },
        Want::Picture(fit) => match decode_as(bytes, fit, animated) {
            Ok(image) => Outcome::Image(image, from),
            Err(Error::Unsupported) => undecoded(Error::Unsupported),
            Err(error) => Outcome::Failed(error),
        },
    }
}

/// What [`Loader::open`] decodes, besides a remote image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Open {
    /// The image file at a path.
    File(PathBuf),
    /// A `data:` URI ([`data`]).
    Data(String),
    /// A BlurHash, decoded at `width` × `height` pixels ([`placeholder::blurhash`]).
    Blurhash {
        hash: String,
        width: u32,
        height: u32,
    },
    /// A ThumbHash in base64, decoded at its own size ([`placeholder::thumbhash`]).
    Thumbhash(String),
}

/// What a load answered, by its id: answers of progress, then one that ends it.
#[derive(Debug)]
pub struct Loaded {
    pub id: u64,
    pub outcome: Outcome,
}

struct Shared {
    answers: Mutex<VecDeque<Loaded>>,
    wake: Mutex<UnixStream>,
}

impl Shared {
    fn answer(&self, id: u64, outcome: Outcome) {
        self.answers
            .lock()
            .expect("lock")
            .push_back(Loaded { id, outcome });
        let _ = self.wake.lock().expect("lock").write_all(&[1]);
    }
}

/// Decodes files, and fetches and decodes remote images ([`remote`]), on threads of its own.
pub struct Loader {
    shared: Arc<Shared>,
    readable: UnixStream,
    remote: Option<Arc<remote::Remote>>,
}

impl Loader {
    pub fn new() -> io::Result<Self> {
        let (readable, wake) = UnixStream::pair()?;
        readable.set_nonblocking(true)?;
        wake.set_nonblocking(true)?;
        Ok(Self {
            shared: Arc::new(Shared {
                answers: Mutex::new(VecDeque::new()),
                wake: Mutex::new(wake),
            }),
            readable,
            remote: None,
        })
    }

    /// Starts decoding `path` at the size `fit` asks for; its answer follows.
    pub fn load(&self, id: u64, path: PathBuf, fit: Fit) {
        self.open(id, Open::File(path), fit, false);
    }

    /// Starts decoding what `open` names at the size `fit` asks for (a BlurHash at its own),
    /// an animated GIF or WebP frame by frame if `animated` is set; its answer follows, its
    /// bytes from their source ([`Origin::Source`]).
    pub fn open(&self, id: u64, open: Open, fit: Fit, animated: bool) {
        let shared = self.shared.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("oxyos-image-{id}"))
            .spawn(move || shared.answer(id, opened(open, fit, animated)));
        if let Err(error) = spawned {
            self.shared.answer(
                id,
                Outcome::Failed(Error::Io(format!("the load cannot start: {error}"))),
            );
        }
    }

    /// Fetches remote images through `network` from now on, `cache` its disk cache (none: memory
    /// only).
    pub fn connect(
        &mut self,
        network: Arc<oxyos_net::Network>,
        cache: Option<Arc<oxyos_net::cache::Cache>>,
    ) {
        self.remote = Some(Arc::new(remote::Remote::new(network, cache)));
    }

    /// Starts fetching a remote image; its answers follow. Without [`Loader::connect`], it fails.
    pub fn fetch(&self, id: u64, fetch: remote::Fetch) {
        match &self.remote {
            Some(remote) => remote::Remote::fetch(remote, &self.shared, id, fetch),
            None => self.shared.answer(
                id,
                Outcome::Failed(Error::Fetch("remote images need the network".into())),
            ),
        }
    }

    /// Answers nothing more for `id`: a remote image's request stops if no other load waits for
    /// it.
    pub fn cancel(&self, id: u64) {
        if let Some(remote) = &self.remote {
            remote.cancel(id);
        }
        self.shared
            .answers
            .lock()
            .expect("lock")
            .retain(|answer| answer.id != id);
    }

    /// Whether a remote image is held in memory, and on disk, by its URL or its cache key.
    pub fn cached(&self, key: &str) -> (bool, bool) {
        self.remote
            .as_ref()
            .map_or((false, false), |remote| remote.cached(key))
    }

    /// Empties the memory cache of remote images, and the disk cache, as asked (expo-image's
    /// `clearMemoryCache`, `clearDiskCache`). Loads on their way are not affected.
    pub fn clear(&self, memory: bool, disk: bool) {
        if let Some(remote) = &self.remote {
            remote.clear(memory, disk);
        }
    }

    /// The file in the disk cache holding a remote image's bytes, by its URL or its cache key,
    /// exactly as they were fetched (expo-image's `getCachePathAsync`); none if it is not held.
    pub fn cache_path(&self, key: &str) -> Option<PathBuf> {
        self.remote
            .as_ref()
            .and_then(|remote| remote.cache_path(key))
    }

    /// The next answer, if any; never blocks.
    pub fn answer(&self) -> Option<Loaded> {
        let mut answers = self.shared.answers.lock().expect("lock");
        let answer = answers.pop_front();
        if answers.is_empty() {
            let mut drained = [0u8; 64];
            while matches!((&self.readable).read(&mut drained), Ok(n) if n > 0) {}
        }
        answer
    }
}

impl AsFd for Loader {
    /// Readable while answers are queued.
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.readable.as_fd()
    }
}

/// Decodes what [`Loader::open`] was given, on its thread.
fn opened(open: Open, fit: Fit, animated: bool) -> Outcome {
    let image = match open {
        Open::File(path) => read(&path).and_then(|bytes| decode_as(&bytes, fit, animated)),
        Open::Data(uri) => {
            return match data::parse(&uri) {
                Ok((media_type, bytes)) => outcome(
                    &bytes.into(),
                    media_type.as_deref(),
                    Want::Picture(fit),
                    animated,
                    Origin::Source,
                ),
                Err(error) => Outcome::Failed(error),
            }
        }
        Open::Blurhash {
            hash,
            width,
            height,
        } => placeholder::blurhash(&hash, width, height),
        Open::Thumbhash(hash) => placeholder::thumbhash(&hash, fit),
    };
    match image {
        Ok(image) => Outcome::Image(image, Origin::Source),
        Err(error) => Outcome::Failed(error),
    }
}

#[cfg(test)]
mod tests;
