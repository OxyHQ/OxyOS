//! Images for applications on OxyOS: the decoding behind React Native's `Image` in the Bloom host
//! (docs/react-native-android-adaptation.mdx), through the C ABI (`oxyos-client-ffi`).
//!
//! PNG and JPEG are decoded in Rust, turned from the profile they carry (an embedded ICC profile,
//! Display P3 say) into sRGB, the space the compositor composes in, and reduced to the size they
//! are drawn at by averaging the source pixels each output pixel covers. Pixels come out as
//! premultiplied RGBA, 8 bits a channel. An image without a profile is taken as sRGB, as Android
//! takes it. PNG's gamma and chromaticity chunks are not applied. A truncated JPEG shows what
//! arrived, as Android's decoder (Skia) shows it; a damaged PNG is an error.
//!
//! A [`Loader`] decodes on threads of its own and queues what it decoded, which the caller takes
//! from its own thread ([`Loader::answer`]); its descriptor is readable while any are queued.

use moxcms::{ColorProfile, Layout, TransformOptions};
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
/// from the image's own `natural` size.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub natural: (u32, u32),
    pub pixels: Vec<u8>,
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Image({}x{})", self.width, self.height)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Neither PNG nor JPEG.
    Unsupported,
    /// The file is damaged, or its profile cannot be applied.
    Invalid(String),
    /// Larger than [`MAX_PIXELS`].
    TooLarge { width: u32, height: u32 },
    /// The file could not be read.
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported => write!(f, "not a PNG or JPEG image"),
            Error::Invalid(reason) => write!(f, "invalid image: {reason}"),
            Error::TooLarge { width, height } => write!(f, "image too large: {width}x{height}"),
            Error::Io(reason) => write!(f, "cannot read the image: {reason}"),
        }
    }
}

impl std::error::Error for Error {}

enum Format {
    Png,
    Jpeg,
}

fn format(bytes: &[u8]) -> Result<Format, Error> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok(Format::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok(Format::Jpeg)
    } else {
        Err(Error::Unsupported)
    }
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
    let source = rgba.to_vec();
    transform
        .transform(&source, rgba)
        .map_err(|error| invalid(format!("ICC transform: {error:?}")))
}

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
    // Across first, row by row, then down.
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
    let (width, height, mut rgba, profile) = decode_rgba(bytes)?;
    let target = fit.size(width, height);
    // An opaque image is reduced first and converted after, on far fewer pixels; one with
    // transparency is converted while its colours are still straight, then premultiplied.
    let opaque = rgba.chunks_exact(4).all(|pixel| pixel[3] == 255);
    if !opaque {
        if let Some(profile) = &profile {
            to_srgb(&mut rgba, profile)?;
        }
        premultiply(&mut rgba);
    }
    let mut pixels = if target == (width, height) {
        rgba
    } else {
        reduce(&rgba, (width, height), target)
    };
    if opaque {
        if let Some(profile) = &profile {
            to_srgb(&mut pixels, profile)?;
        }
    }
    Ok(Image {
        width: target.0,
        height: target.1,
        natural: (width, height),
        pixels,
    })
}

/// Reads and decodes a file, as [`decode`].
pub fn decode_file(path: &std::path::Path, fit: Fit) -> Result<Image, Error> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| Error::Io(error.to_string()))?;
    decode(&bytes, fit)
}

/// What a load answered: the image, or why there is none.
#[derive(Debug)]
pub struct Loaded {
    pub id: u64,
    pub result: Result<Image, Error>,
}

struct Shared {
    answers: Mutex<VecDeque<Loaded>>,
    wake: Mutex<UnixStream>,
}

/// Decodes files on threads of its own.
pub struct Loader {
    shared: Arc<Shared>,
    readable: UnixStream,
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
        })
    }

    /// Starts decoding `path` at the size `fit` asks for; its answer follows.
    pub fn load(&self, id: u64, path: PathBuf, fit: Fit) {
        let shared = self.shared.clone();
        let answer = move |result| {
            shared
                .answers
                .lock()
                .expect("lock")
                .push_back(Loaded { id, result });
            let _ = shared.wake.lock().expect("lock").write_all(&[1]);
        };
        let spawned = std::thread::Builder::new()
            .name(format!("oxyos-image-{id}"))
            .spawn({
                let answer = answer.clone();
                move || answer(decode_file(&path, fit))
            });
        if let Err(error) = spawned {
            answer(Err(Error::Io(format!("the load cannot start: {error}"))));
        }
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

#[cfg(test)]
mod tests;
