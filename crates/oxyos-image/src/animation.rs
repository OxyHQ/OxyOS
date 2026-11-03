//! Animated GIF and WebP, frame by frame, as expo-image plays them on Android: every frame
//! composed on the image's canvas as it is shown, then made sRGB, premultiplied and reduced to the
//! size it is drawn at as a still image is ([`crate::decode`]), the frames one after the other.
//!
//! A GIF's frame is placed where it says, its transparent pixels showing the canvas below, and
//! disposed of as it says before the next: kept, its area cleared to transparent (what "restore to
//! background" is in browsers; Glide fills it with the background colour only when the next frame
//! has no transparency, which no browser does), or the canvas put back as it was before it
//! ("restore to previous"). Its delay is in hundredths of a second; one of 0 or 1 is 100 ms, as
//! Glide's GIF decoder (expo-image's on Android) and browsers show it. It plays once without a
//! NETSCAPE loop extension, forever with a count of 0, and else once more than its count (the
//! times it repeats), as Glide and Chromium play it.
//!
//! A WebP's frames are composed as libwebp's `WebPAnimDecoder` composes them: on a transparent
//! canvas (its background colour is a hint, which browsers ignore), each frame blended over what
//! is below (non-premultiplied "over") unless it says not to, its area cleared to transparent
//! after it when it says to dispose of it. Each frame's bitstream is decoded by image-webp, alone:
//! its own composition (`read_frame`) clears the wrong pixels when a frame without alpha follows
//! one disposed of. A duration of 0 is 100 ms, as APNG4Android's WebP decoder (expo-image's on
//! Android) plays it; any other as given. Its loop count is how many times it plays, 0 forever.
//!
//! An animation is held decoded whole for the host to play. One whose frames would take more than
//! [`MAX_FRAMES_BYTES`] at the size drawn (a long animation drawn large) is decoded still, its
//! first frame alone, rather than hold hundreds of MiB per image. A frame that cannot be decoded
//! ends the animation at the frames before it, as browsers show what they could; the first one
//! failing fails it.

use crate::{checked, finish, format, gif_decoder, invalid, Error, Fit, Format, Image};
use std::io::Cursor;

/// The most the frames of one animated image take decoded, at the size they are drawn, in bytes.
pub const MAX_FRAMES_BYTES: u64 = 64 << 20;

/// Decodes an image as [`crate::decode`] does, but an animated GIF or WebP frame by frame.
pub fn decode(bytes: &[u8], fit: Fit) -> Result<Image, Error> {
    match format(bytes)? {
        Format::Gif => gif(bytes, fit),
        Format::Webp => webp(bytes, fit),
        Format::Png | Format::Jpeg => crate::decode(bytes, fit),
    }
}

/// Whether `count` frames of `target` fit in [`MAX_FRAMES_BYTES`].
fn fits(count: u64, target: (u32, u32)) -> bool {
    count.saturating_mul(u64::from(target.0) * u64::from(target.1) * 4) <= MAX_FRAMES_BYTES
}

/// The frames decoded, made into an image: a still one if there is only one.
fn image(
    target: (u32, u32),
    natural: (u32, u32),
    pixels: Vec<u8>,
    delays: Vec<u32>,
    loops: u32,
) -> Image {
    if delays.len() <= 1 {
        return Image::still(target.0, target.1, natural, pixels);
    }
    Image {
        width: target.0,
        height: target.1,
        natural,
        pixels,
        delays,
        loops,
    }
}

/// The part of a canvas `canvas_width` pixels wide a rectangle covers, clipped to the canvas: each
/// row's range of bytes, with the row of the rectangle it is.
fn rows(
    canvas: (u32, u32),
    left: u32,
    top: u32,
    width: u32,
    height: u32,
) -> impl Iterator<Item = (usize, std::ops::Range<usize>)> {
    let columns = width.min(canvas.0.saturating_sub(left)) as usize;
    let lines = height.min(canvas.1.saturating_sub(top));
    (0..lines).map(move |y| {
        let at = ((top + y) as usize * canvas.0 as usize + left as usize) * 4;
        (y as usize, at..at + columns * 4)
    })
}

fn gif(bytes: &[u8], fit: Fit) -> Result<Image, Error> {
    // Counted first, their pixels not decoded, to know whether they fit.
    let count = {
        let mut options = gif::DecodeOptions::new();
        options.skip_frame_decoding(true);
        let mut decoder = options.read_info(Cursor::new(bytes)).map_err(invalid)?;
        let mut count = 0u64;
        while let Ok(Some(_)) = decoder.read_next_frame() {
            count += 1;
        }
        count
    };
    let mut decoder = gif_decoder(bytes)?;
    let size = (u32::from(decoder.width()), u32::from(decoder.height()));
    checked(size.0, size.1)?;
    let target = fit.size(size.0, size.1);
    if count <= 1 || !fits(count, target) {
        return crate::decode(bytes, fit);
    }
    let mut canvas = vec![0u8; size.0 as usize * size.1 as usize * 4];
    let (mut pixels, mut delays) = (Vec::new(), Vec::new());
    // How the frame before is disposed of: its area, and the canvas before it if it is put back.
    let mut dispose: Option<(gif::DisposalMethod, [u32; 4])> = None;
    let mut before: Option<Vec<u8>> = None;
    loop {
        let frame = match decoder.read_next_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) if delays.is_empty() => return Err(invalid(error)),
            Err(_) => break,
        };
        match dispose.take() {
            Some((gif::DisposalMethod::Background, [left, top, width, height])) => {
                for (_, range) in rows(size, left, top, width, height) {
                    canvas[range].fill(0);
                }
            }
            Some((gif::DisposalMethod::Previous, _)) => {
                if let Some(before) = before.take() {
                    canvas = before;
                }
            }
            _ => {}
        }
        if frame.dispose == gif::DisposalMethod::Previous {
            before = Some(canvas.clone());
        }
        let area = [
            u32::from(frame.left),
            u32::from(frame.top),
            u32::from(frame.width),
            u32::from(frame.height),
        ];
        let frame_row = usize::from(frame.width) * 4;
        for (y, range) in rows(size, area[0], area[1], area[2], area[3]) {
            let row = &frame.buffer[y * frame_row..][..range.len()];
            for (out, pixel) in canvas[range].chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                // A transparent pixel shows the canvas below.
                if pixel[3] != 0 {
                    out.copy_from_slice(pixel);
                }
            }
        }
        dispose = Some((frame.dispose, area));
        delays.push(if frame.delay < 2 {
            100
        } else {
            u32::from(frame.delay) * 10
        });
        pixels.extend(finish(canvas.clone(), size, target, None)?);
    }
    let loops = match decoder.repeat() {
        gif::Repeat::Infinite => 0,
        // No NETSCAPE extension (a count of 0 is read as forever): it plays once.
        gif::Repeat::Finite(repeats) => u32::from(repeats) + 1,
    };
    Ok(image(target, size, pixels, delays, loops))
}

/// A frame of an animated WebP as its `ANMF` chunk gives it.
struct WebpFrame<'a> {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    duration: u32,
    blend: bool,
    dispose: bool,
    /// Its bitstream's chunks: an `ALPH` and a `VP8 `, or a `VP8L`.
    alpha: Option<&'a [u8]>,
    image: (&'a [u8; 4], &'a [u8]),
}

/// The chunks of a RIFF payload, each its tag and its data; none past one cut short.
fn chunks(mut bytes: &[u8]) -> impl Iterator<Item = (&[u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        let tag: &[u8; 4] = bytes.get(..4)?.try_into().ok()?;
        let length = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize;
        let data = bytes.get(8..8 + length)?;
        bytes = bytes.get(8 + length + (length & 1)..).unwrap_or(&[]);
        Some((tag, data))
    })
}

fn u24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16)
}

/// The frames of an animated WebP, in order; a frame cut short or without a bitstream ends them.
fn webp_frames(bytes: &[u8]) -> Vec<WebpFrame<'_>> {
    let mut frames = Vec::new();
    for (tag, data) in chunks(bytes.get(12..).unwrap_or(&[])) {
        if tag != b"ANMF" {
            continue;
        }
        let Some(header) = data.get(..16) else {
            break;
        };
        let (mut alpha, mut image) = (None, None);
        for (tag, data) in chunks(&data[16..]) {
            match tag {
                b"ALPH" => alpha = Some(data),
                b"VP8 " | b"VP8L" => {
                    image = Some((tag, data));
                    break;
                }
                _ => {}
            }
        }
        let Some(image) = image else {
            break;
        };
        frames.push(WebpFrame {
            left: u24(&header[0..]) * 2,
            top: u24(&header[3..]) * 2,
            width: u24(&header[6..]) + 1,
            height: u24(&header[9..]) + 1,
            duration: u24(&header[12..]),
            blend: header[15] & 2 == 0,
            dispose: header[15] & 1 != 0,
            alpha: alpha.filter(|_| image.0 == b"VP8 "),
            image,
        });
    }
    frames
}

/// A WebP file of chunks.
fn riff(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let length: usize = chunks
        .iter()
        .map(|(_, data)| 8 + data.len() + (data.len() & 1))
        .sum();
    let mut bytes = Vec::with_capacity(12 + length);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((4 + length) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WEBP");
    for (tag, data) in chunks {
        bytes.extend_from_slice(*tag);
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(data);
        if data.len() & 1 == 1 {
            bytes.push(0);
        }
    }
    bytes
}

/// A frame's own pixels, straight RGBA: its bitstream decoded as a WebP image of its own.
fn webp_frame_pixels(frame: &WebpFrame<'_>) -> Result<Vec<u8>, Error> {
    let file = match frame.alpha {
        Some(alpha) => {
            // An extended file whose flags say it has alpha, its canvas the frame's size.
            let mut header = [0u8; 10];
            header[0] = 0x10;
            header[4..7].copy_from_slice(&(frame.width - 1).to_le_bytes()[..3]);
            header[7..10].copy_from_slice(&(frame.height - 1).to_le_bytes()[..3]);
            riff(&[(b"VP8X", &header), (b"ALPH", alpha), frame.image])
        }
        None => riff(&[frame.image]),
    };
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(&file)).map_err(invalid)?;
    if decoder.dimensions() != (frame.width, frame.height) {
        return Err(invalid("a WebP frame's size is not its bitstream's"));
    }
    let mut samples = vec![
        0;
        decoder
            .output_buffer_size()
            .ok_or_else(|| invalid("WebP frame too large"))?
    ];
    decoder.read_image(&mut samples).map_err(invalid)?;
    Ok(if decoder.has_alpha() {
        samples
    } else {
        samples
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect()
    })
}

/// `source` over `below`, neither premultiplied, rounded to the nearest level.
fn over(source: &[u8], below: &[u8]) -> [u8; 4] {
    let (source_alpha, below_alpha) = (u32::from(source[3]), u32::from(below[3]));
    if source_alpha == 255 || below_alpha == 0 {
        return [source[0], source[1], source[2], source[3]];
    }
    if source_alpha == 0 {
        return [below[0], below[1], below[2], below[3]];
    }
    // Weights in 255ths of 255ths: the source's alpha, and what of the below's shows through.
    let (front, back) = (source_alpha * 255, below_alpha * (255 - source_alpha));
    let total = front + back;
    let mut blended = [0u8; 4];
    for c in 0..3 {
        let sum = u32::from(source[c]) * front + u32::from(below[c]) * back;
        blended[c] = ((sum + total / 2) / total) as u8;
    }
    blended[3] = ((total + 127) / 255) as u8;
    blended
}

fn webp(bytes: &[u8], fit: Fit) -> Result<Image, Error> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(invalid)?;
    if !decoder.is_animated() {
        return crate::decode(bytes, fit);
    }
    let size = decoder.dimensions();
    checked(size.0, size.1)?;
    let target = fit.size(size.0, size.1);
    let frames = webp_frames(bytes);
    if frames.len() <= 1 || !fits(frames.len() as u64, target) {
        return crate::decode(bytes, fit);
    }
    let profile = decoder.icc_profile().map_err(invalid)?;
    let loops = match decoder.loop_count() {
        image_webp::LoopCount::Forever => 0,
        image_webp::LoopCount::Times(times) => u32::from(times.get()),
    };
    let mut canvas = vec![0u8; size.0 as usize * size.1 as usize * 4];
    let (mut pixels, mut delays) = (Vec::new(), Vec::new());
    for frame in &frames {
        let own = if frame.left + frame.width > size.0 || frame.top + frame.height > size.1 {
            Err(invalid("a WebP frame outside its canvas"))
        } else {
            webp_frame_pixels(frame)
        };
        let own = match own {
            Ok(own) => own,
            Err(error) if delays.is_empty() => return Err(error),
            Err(_) => break,
        };
        let area = rows(size, frame.left, frame.top, frame.width, frame.height);
        for (y, range) in area {
            let row = &own[y * frame.width as usize * 4..][..range.len()];
            for (out, pixel) in canvas[range].chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                let shown = if frame.blend {
                    over(pixel, out)
                } else {
                    [pixel[0], pixel[1], pixel[2], pixel[3]]
                };
                out.copy_from_slice(&shown);
            }
        }
        delays.push(if frame.duration == 0 {
            100
        } else {
            frame.duration
        });
        pixels.extend(finish(canvas.clone(), size, target, profile.as_deref())?);
        if frame.dispose {
            for (_, range) in rows(size, frame.left, frame.top, frame.width, frame.height) {
                canvas[range].fill(0);
            }
        }
    }
    Ok(image(target, size, pixels, delays, loops))
}

#[cfg(test)]
#[path = "animation_tests.rs"]
mod tests;
