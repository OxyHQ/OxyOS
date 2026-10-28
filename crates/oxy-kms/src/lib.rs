//! DRM/KMS output (docs/adr/0004-gpu-composition.mdx, step 4). This first part reads what a
//! card offers, which needs no DRM master: its connectors and their modes, and its planes with
//! the format and modifier pairs each accepts (`IN_FORMATS`), the compatibility the buffer ABI
//! must be checked against before it is frozen.

use drm::control::{connector, Device as ControlDevice};
use drm::{ClientCapability, Device};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::Path;

pub use drm_fourcc::DrmFourcc;

mod display;
pub use display::Display;

/// An open DRM card.
pub struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl Device for Card {}
impl ControlDevice for Card {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mode {
    pub width: u16,
    pub height: u16,
    pub refresh_hz: u32,
    pub preferred: bool,
}

#[derive(Clone, Debug)]
pub struct Output {
    pub name: String,
    pub connected: bool,
    pub modes: Vec<Mode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaneKind {
    Primary,
    Overlay,
    Cursor,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Plane {
    pub id: u32,
    pub kind: PlaneKind,
    /// Format (fourcc) and modifier pairs from `IN_FORMATS`; without the property, each format
    /// with the linear modifier only.
    pub formats: Vec<(drm_fourcc::DrmFourcc, u64)>,
}

impl Card {
    /// Opens a card and asks for universal planes and atomic modesetting (neither needs master).
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let card = Self(file);
        card.set_client_capability(ClientCapability::UniversalPlanes, true)?;
        card.set_client_capability(ClientCapability::Atomic, true)?;
        Ok(card)
    }

    /// The kernel driver's name (for example `i915`).
    pub fn driver_name(&self) -> io::Result<String> {
        Ok(self.get_driver()?.name().to_string_lossy().into_owned())
    }

    pub fn outputs(&self) -> io::Result<Vec<Output>> {
        let resources = self.resource_handles()?;
        let mut outputs = Vec::new();
        for &handle in resources.connectors() {
            let info = self.get_connector(handle, false)?;
            let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
            let modes = info
                .modes()
                .iter()
                .map(|mode| {
                    let (width, height) = mode.size();
                    Mode {
                        width,
                        height,
                        refresh_hz: mode.vrefresh(),
                        preferred: mode
                            .mode_type()
                            .contains(drm::control::ModeTypeFlags::PREFERRED),
                    }
                })
                .collect();
            outputs.push(Output {
                name,
                connected: info.state() == connector::State::Connected,
                modes,
            });
        }
        Ok(outputs)
    }

    pub fn planes(&self) -> io::Result<Vec<Plane>> {
        let mut planes = Vec::new();
        for handle in self.plane_handles()? {
            let info = self.get_plane(handle)?;
            let (mut kind, mut in_formats) = (PlaneKind::Unknown, None);
            let properties = self.get_properties(handle)?;
            for (&property, &value) in properties.iter() {
                let name = self
                    .get_property(property)?
                    .name()
                    .to_string_lossy()
                    .into_owned();
                match name.as_str() {
                    "type" => {
                        kind = match value {
                            0 => PlaneKind::Overlay,
                            1 => PlaneKind::Primary,
                            2 => PlaneKind::Cursor,
                            _ => PlaneKind::Unknown,
                        }
                    }
                    "IN_FORMATS" if value != 0 => in_formats = Some(value),
                    _ => {}
                }
            }
            let formats = match in_formats {
                Some(blob) => parse_in_formats(&self.get_property_blob(blob)?)?,
                None => info
                    .formats()
                    .iter()
                    .filter_map(|&code| drm_fourcc::DrmFourcc::try_from(code).ok())
                    .map(|format| (format, 0))
                    .collect(),
            };
            planes.push(Plane {
                id: u32::from(handle),
                kind,
                formats,
            });
        }
        Ok(planes)
    }
}

/// Parses an `IN_FORMATS` blob (`struct drm_format_modifier_blob`): a header, the formats
/// (u32 each), then entries of a 64-bit format mask, the offset of its first format and the
/// modifier.
pub fn parse_in_formats(blob: &[u8]) -> io::Result<Vec<(drm_fourcc::DrmFourcc, u64)>> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "malformed IN_FORMATS blob");
    let u32_at = |at: usize| -> io::Result<u32> {
        let bytes = blob.get(at..at + 4).ok_or_else(invalid)?;
        Ok(u32::from_ne_bytes(bytes.try_into().expect("4 bytes")))
    };
    let u64_at = |at: usize| -> io::Result<u64> {
        let bytes = blob.get(at..at + 8).ok_or_else(invalid)?;
        Ok(u64::from_ne_bytes(bytes.try_into().expect("8 bytes")))
    };
    let count_formats = u32_at(8)? as usize;
    let formats_offset = u32_at(12)? as usize;
    let count_modifiers = u32_at(16)? as usize;
    let modifiers_offset = u32_at(20)? as usize;
    let formats: Vec<u32> = (0..count_formats)
        .map(|i| u32_at(formats_offset + i * 4))
        .collect::<io::Result<_>>()?;
    let mut pairs = Vec::new();
    for i in 0..count_modifiers {
        let entry = modifiers_offset + i * 24;
        let mask = u64_at(entry)?;
        let offset = u32_at(entry + 8)? as usize;
        let modifier = u64_at(entry + 16)?;
        for bit in 0..64 {
            if mask & (1 << bit) != 0 {
                let code = *formats.get(offset + bit).ok_or_else(invalid)?;
                if let Ok(format) = drm_fourcc::DrmFourcc::try_from(code) {
                    pairs.push((format, modifier));
                }
            }
        }
    }
    Ok(pairs)
}

#[cfg(test)]
mod tests;
