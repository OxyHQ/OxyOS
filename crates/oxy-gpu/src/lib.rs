//! Composition of a frame's layers (docs/adr/0004-gpu-composition.mdx): the reference on the
//! CPU, [`compose_cpu`], and the same composition on a Vulkan device, [`VulkanCompositor`],
//! which must agree with it byte for byte.
//!
//! The workspace forbids `unsafe`; this crate denies it and allows it only on the Vulkan items,
//! each with the contract it relies on.

use std::ffi::CStr;

/// What a layer puts on the output.
#[derive(Clone, Copy, Debug)]
pub enum Paint<'a> {
    /// XRGB8888 pixels (B, G, R, X in memory), copied.
    Opaque(&'a [u8]),
    /// Premultiplied ARGB8888 pixels (B, G, R, A in memory), blended source-over.
    Premultiplied(&'a [u8]),
    /// One colour, 0x00RRGGBB.
    Fill(u32),
}

/// One layer of a frame, bottom to top: its rectangle on the output in logical pixels (which
/// may extend past the output) and its pixels at `extent`, scaled to the rectangle by nearest
/// neighbour.
#[derive(Clone, Copy, Debug)]
pub struct Layer<'a> {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub extent: (u32, u32),
    pub paint: Paint<'a>,
}

impl Layer<'_> {
    fn pixels(&self) -> Option<&[u8]> {
        match self.paint {
            Paint::Opaque(pixels) | Paint::Premultiplied(pixels) => Some(pixels),
            Paint::Fill(_) => None,
        }
    }

    /// Whether the pixels cover the extent (4 bytes each).
    fn valid(&self) -> bool {
        let needed = u64::from(self.extent.0) * u64::from(self.extent.1) * 4;
        self.width > 0
            && self.height > 0
            && self.pixels().is_none_or(|pixels| {
                self.extent.0 > 0 && self.extent.1 > 0 && pixels.len() as u64 >= needed
            })
    }
}

fn pixel(pixels: &[u8], index: usize) -> u32 {
    u32::from_le_bytes([
        pixels[index * 4],
        pixels[index * 4 + 1],
        pixels[index * 4 + 2],
        pixels[index * 4 + 3],
    ])
}

/// Composes `layers` onto an output of `width` × `height` filled with `background`, as the
/// compositor shows them: 0x00RRGGBB per pixel, row by row. The reference composition.
pub fn compose_cpu(width: u32, height: u32, background: u32, layers: &[Layer<'_>]) -> Vec<u32> {
    let mut out = vec![background; width as usize * height as usize];
    for layer in layers.iter().filter(|layer| layer.valid()) {
        let x0 = layer.x.max(0);
        let y0 = layer.y.max(0);
        let x1 = (i64::from(layer.x) + i64::from(layer.width)).min(i64::from(width));
        let y1 = (i64::from(layer.y) + i64::from(layer.height)).min(i64::from(height));
        for y in i64::from(y0)..y1 {
            let ly = (y - i64::from(layer.y)) as u64;
            let sy = ly * u64::from(layer.extent.1) / u64::from(layer.height);
            for x in i64::from(x0)..x1 {
                let lx = (x - i64::from(layer.x)) as u64;
                let index = y as usize * width as usize + x as usize;
                let source_index = || {
                    let sx = lx * u64::from(layer.extent.0) / u64::from(layer.width);
                    (sy * u64::from(layer.extent.0) + sx) as usize
                };
                out[index] = match layer.paint {
                    Paint::Fill(color) => color,
                    Paint::Opaque(pixels) => pixel(pixels, source_index()) & 0x00ff_ffff,
                    Paint::Premultiplied(pixels) => {
                        let source = pixel(pixels, source_index());
                        let keep = 255 - (source >> 24);
                        let under = out[index];
                        let channel = |shift: u32| {
                            let s = (source >> shift) & 0xff;
                            let d = (under >> shift) & 0xff;
                            (s + d * keep / 255).min(255) << shift
                        };
                        channel(16) | channel(8) | channel(0)
                    }
                };
            }
        }
    }
    out
}

pub mod frame_file;
mod vulkan;
pub use vulkan::{DeviceKind, VulkanCompositor};

/// The SPIR-V of the composition shader (`shaders/compose.wgsl`).
fn shader() -> Vec<u32> {
    include_bytes!(concat!(env!("OUT_DIR"), "/compose.spv"))
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
        .collect()
}

fn name(raw: &[std::ffi::c_char]) -> String {
    CStr::from_bytes_until_nul(&raw.iter().map(|&c| c as u8).collect::<Vec<u8>>())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
