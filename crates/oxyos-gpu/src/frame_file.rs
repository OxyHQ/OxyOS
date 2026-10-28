//! A composed frame's layers in a file, so frames composed by one machine's compositor can be
//! composed again elsewhere (on another device, or another OS's Vulkan driver) and compared.
//!
//! Format, little-endian: `OXYFRAME1`, then width, height, background and the layer count (u32
//! each); per layer x, y (i32), width, height, extent width, extent height (u32), kind (u8:
//! 0 opaque, 1 premultiplied, 2 fill), colour (u32), byte length (u64) and the pixels.

use crate::{Layer, Paint};
use std::io::{self, Read, Write};

const MAGIC: &[u8; 9] = b"OXYFRAME1";

/// A frame read back: owns its pixels; `layers()` borrows them.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub background: u32,
    layers: Vec<(Layer<'static>, u8, Vec<u8>)>,
}

impl Frame {
    pub fn layers(&self) -> Vec<Layer<'_>> {
        self.layers
            .iter()
            .map(|(layer, kind, pixels)| Layer {
                paint: match kind {
                    0 => Paint::Opaque(pixels),
                    1 => Paint::Premultiplied(pixels),
                    _ => layer.paint,
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
        let (kind, color, pixels): (u8, u32, &[u8]) = match layer.paint {
            Paint::Opaque(pixels) => (0, 0, pixels),
            Paint::Premultiplied(pixels) => (1, 0, pixels),
            Paint::Fill(color) => (2, color, &[]),
        };
        out.write_all(&layer.x.to_le_bytes())?;
        out.write_all(&layer.y.to_le_bytes())?;
        for value in [layer.width, layer.height, layer.extent.0, layer.extent.1] {
            out.write_all(&value.to_le_bytes())?;
        }
        out.write_all(&[kind])?;
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
    if &magic != MAGIC {
        return Err(invalid("not a frame file"));
    }
    let head = u32s(input, 4)?;
    let (width, height, background, count) = (head[0], head[1], head[2], head[3]);
    let mut layers = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let fields = u32s(input, 6)?;
        let mut kind = [0; 1];
        input.read_exact(&mut kind)?;
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
        };
        if kind[0] > 2 {
            return Err(invalid("unknown layer kind"));
        }
        layers.push((layer, kind[0], pixels));
    }
    Ok(Frame {
        width,
        height,
        background,
        layers,
    })
}
