//! A composed frame's layers in a file, so frames composed by one machine's compositor can be
//! composed again elsewhere (on another device, or another OS's Vulkan driver) and compared.
//!
//! Format, little-endian: `OXYFRAME3`, then width, height, background and the layer count (u32
//! each); per layer x, y (i32), width, height, extent width, extent height (u32), kind (u8:
//! 0 opaque, 1 premultiplied, 2 fill, 3 shadow), opacity (u8), radius (u32), a shadow's shape
//! and what it is beneath (x, y as i32, width, height and radius as u32, each) and its scale
//! (u32), colour (u32), byte length (u64) and the pixels. `OXYFRAME2` files, from before corners
//! and shadows, are read with every radius 0; `OXYFRAME1` files, from before opacities, also with
//! every layer at 255.

use crate::{Layer, Paint, Rect, Shadow};
use std::io::{self, Read, Write};

const MAGIC: &[u8; 9] = b"OXYFRAME3";
const MAGIC_2: &[u8; 9] = b"OXYFRAME2";
const MAGIC_1: &[u8; 9] = b"OXYFRAME1";

/// A frame read back: owns its pixels; `layers()` borrows them.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub background: u32,
    layers: Vec<(Layer<'static>, Kind, Vec<u8>)>,
}

/// What a layer read paints with its pixels.
#[derive(Clone, Copy)]
enum Kind {
    Opaque,
    Premultiplied,
    Fill,
    Shadow(Shadow),
}

impl Frame {
    pub fn layers(&self) -> Vec<Layer<'_>> {
        self.layers
            .iter()
            .map(|(layer, kind, pixels)| Layer {
                paint: match kind {
                    Kind::Opaque => Paint::Opaque(pixels),
                    Kind::Premultiplied => Paint::Premultiplied(pixels),
                    Kind::Fill => layer.paint,
                    Kind::Shadow(shadow) => Paint::Shadow(pixels, *shadow),
                },
                ..*layer
            })
            .collect()
    }
}

pub fn write(
    out: &mut impl Write,
    width: u32,
    height: u32,
    background: u32,
    layers: &[Layer<'_>],
) -> io::Result<()> {
    out.write_all(MAGIC)?;
    for value in [width, height, background, layers.len() as u32] {
        out.write_all(&value.to_le_bytes())?;
    }
    for layer in layers {
        let (kind, color, pixels, shadow): (u8, u32, &[u8], _) = match layer.paint {
            Paint::Opaque(pixels) => (0, 0, pixels, None),
            Paint::Premultiplied(pixels) => (1, 0, pixels, None),
            Paint::Fill(color) => (2, color, &[], None),
            Paint::Shadow(pixels, shadow) => (3, 0, pixels, Some(shadow)),
        };
        out.write_all(&layer.x.to_le_bytes())?;
        out.write_all(&layer.y.to_le_bytes())?;
        for value in [layer.width, layer.height, layer.extent.0, layer.extent.1] {
            out.write_all(&value.to_le_bytes())?;
        }
        out.write_all(&[kind, layer.opacity])?;
        out.write_all(&layer.radius.to_le_bytes())?;
        let shadow = shadow.unwrap_or_default();
        for (rect, radius) in [
            (shadow.shape, shadow.radius),
            (shadow.clip, shadow.clip_radius),
        ] {
            out.write_all(&rect.x.to_le_bytes())?;
            out.write_all(&rect.y.to_le_bytes())?;
            for value in [rect.width, rect.height, radius] {
                out.write_all(&value.to_le_bytes())?;
            }
        }
        out.write_all(&shadow.scale.to_le_bytes())?;
        out.write_all(&color.to_le_bytes())?;
        out.write_all(&(pixels.len() as u64).to_le_bytes())?;
        out.write_all(pixels)?;
    }
    Ok(())
}

fn u32s(input: &mut impl Read, count: usize) -> io::Result<Vec<u32>> {
    let mut bytes = vec![0; count * 4];
    input.read_exact(&mut bytes)?;
    Ok(bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

pub fn read(input: &mut impl Read) -> io::Result<Frame> {
    let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let mut magic = [0; 9];
    input.read_exact(&mut magic)?;
    // The version: 1 without opacities, 2 without corners and shadows.
    let version = match &magic {
        magic if magic == MAGIC => 3,
        magic if magic == MAGIC_2 => 2,
        magic if magic == MAGIC_1 => 1,
        _ => return Err(invalid("not a frame file")),
    };
    let head = u32s(input, 4)?;
    let (width, height, background, count) = (head[0], head[1], head[2], head[3]);
    let mut layers = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let fields = u32s(input, 6)?;
        let mut kind = [0; 1];
        input.read_exact(&mut kind)?;
        let mut opacity = [255; 1];
        if version >= 2 {
            input.read_exact(&mut opacity)?;
        }
        // Radius, then the shadow's shape and clip (x, y, width, height, radius) and scale.
        let shape = if version >= 3 {
            u32s(input, 12)?
        } else {
            vec![0; 12]
        };
        let rect = |at: usize| Rect {
            x: shape[at] as i32,
            y: shape[at + 1] as i32,
            width: shape[at + 2],
            height: shape[at + 3],
        };
        let shadow = Shadow {
            shape: rect(1),
            radius: shape[5],
            clip: rect(6),
            clip_radius: shape[10],
            scale: shape[11],
        };
        let color = u32s(input, 1)?[0];
        let mut length = [0; 8];
        input.read_exact(&mut length)?;
        let length = u64::from_le_bytes(length);
        if length > 1 << 32 {
            return Err(invalid("layer too large"));
        }
        let mut pixels = vec![0; length as usize];
        input.read_exact(&mut pixels)?;
        let layer = Layer {
            x: fields[0] as i32,
            y: fields[1] as i32,
            width: fields[2],
            height: fields[3],
            extent: (fields[4], fields[5]),
            paint: Paint::Fill(color),
            source: None,
            changed: None,
            opacity: opacity[0],
            radius: shape[0],
        };
        let kind = match kind[0] {
            0 => Kind::Opaque,
            1 => Kind::Premultiplied,
            2 => Kind::Fill,
            3 if version >= 3 => Kind::Shadow(shadow),
            _ => return Err(invalid("unknown layer kind")),
        };
        layers.push((layer, kind, pixels));
    }
    Ok(Frame {
        width,
        height,
        background,
        layers,
    })
}
