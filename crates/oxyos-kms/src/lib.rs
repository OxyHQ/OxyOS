//! DRM/KMS output (docs/adr/0004-gpu-composition.mdx, step 4). This first part reads what a
//! card offers, which needs no DRM master: its connectors and their modes, and its planes with
//! the format and modifier pairs each accepts (`IN_FORMATS`), the compatibility the buffer ABI
//! must be checked against before it is frozen.
//!
//! A card drives several outputs, each a [`Display`] on a CRTC of its own sharing the card
//! (docs/adr/0018-several-displays.mdx); [`Hotplug`] hears when its connectors change.

use drm::control::{connector, Device as ControlDevice};
use drm::{ClientCapability, Device};
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::Path;
use std::time::Duration;

pub use drm::control::framebuffer::Handle as Framebuffer;
pub use drm_fourcc::DrmFourcc;

mod display;
pub use display::{Display, PointerImage};
mod edid;
pub use edid::{identity_safe, Edid};
mod hotplug;
pub use hotplug::Hotplug;

/// A DMA-BUF imported through PRIME, as `add_planar_framebuffer` takes it: up to four planes,
/// each its buffer handle, pitch and offset, in a format with a modifier.
struct Imported {
    handles: [Option<drm::buffer::Handle>; 4],
    size: (u32, u32),
    format: DrmFourcc,
    /// None: the framebuffer names no modifier (a linear buffer on a card without modifiers).
    modifier: Option<u64>,
    pitches: [u32; 4],
    offsets: [u32; 4],
}

impl drm::buffer::PlanarBuffer for Imported {
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn format(&self) -> DrmFourcc {
        self.format
    }
    fn modifier(&self) -> Option<drm_fourcc::DrmModifier> {
        self.modifier.map(drm_fourcc::DrmModifier::from)
    }
    fn pitches(&self) -> [u32; 4] {
        self.pitches
    }
    fn handles(&self) -> [Option<drm::buffer::Handle>; 4] {
        self.handles
    }
    fn offsets(&self) -> [u32; 4] {
        self.offsets
    }
}

/// One plane of a DMA-BUF to scan out: its descriptor, where it starts and its pitch.
#[derive(Clone, Copy, Debug)]
pub struct DmabufPlane<'a> {
    pub fd: BorrowedFd<'a>,
    pub offset: u32,
    pub pitch: u32,
}

/// An open DRM card. Its displays share it, and its events: the flips read for one CRTC wait here
/// for the display of that CRTC (`Display::flip_done`).
pub struct Card {
    file: File,
    flips: RefCell<Vec<(drm::control::crtc::Handle, Duration)>>,
    /// Its render node's device number, once looked up (`render_node`).
    render: std::cell::OnceCell<Option<u64>>,
}

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

impl Device for Card {}
impl ControlDevice for Card {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mode {
    pub width: u16,
    pub height: u16,
    pub refresh_hz: u32,
    /// Its refresh exactly, in millihertz, from its timings (59.94 Hz is 59940).
    pub refresh_mhz: u32,
    pub preferred: bool,
}

impl Mode {
    fn of(mode: &drm::control::Mode) -> Self {
        let (width, height) = mode.size();
        Self {
            width,
            height,
            refresh_hz: mode.vrefresh(),
            refresh_mhz: refresh_mhz(mode),
            preferred: mode
                .mode_type()
                .contains(drm::control::ModeTypeFlags::PREFERRED),
        }
    }

    /// Whether `mode` is this one: its pixels and refresh.
    pub fn is(&self, mode: &drm::control::Mode) -> bool {
        let of = Self::of(mode);
        (of.width, of.height, of.refresh_mhz) == (self.width, self.height, self.refresh_mhz)
    }
}

/// A mode's refresh in millihertz, from its pixel clock and totals as the kernel computes its
/// `vrefresh` (`drm_mode_vrefresh`), to the nearest.
fn refresh_mhz(mode: &drm::control::Mode) -> u32 {
    let (_, _, htotal) = mode.hsync();
    let (_, _, vtotal) = mode.vsync();
    let mut frame = u64::from(htotal) * u64::from(vtotal);
    let flags = mode.flags();
    if flags.contains(drm::control::ModeFlags::DBLSCAN) {
        frame *= 2;
    }
    if mode.vscan() > 1 {
        frame *= u64::from(mode.vscan());
    }
    if frame == 0 {
        return mode.vrefresh() * 1000;
    }
    let mut mhz = (u64::from(mode.clock()) * 1_000_000 + frame / 2) / frame;
    if flags.contains(drm::control::ModeFlags::INTERLACE) {
        mhz *= 2;
    }
    u32::try_from(mhz).unwrap_or(u32::MAX)
}

/// Whether the session drives a display at `mode`: progressive scan only, as every compositor
/// does (an interlaced or double-scanned mode is a television's).
pub(crate) fn drivable(mode: &drm::control::Mode) -> bool {
    !mode
        .flags()
        .intersects(drm::control::ModeFlags::INTERLACE | drm::control::ModeFlags::DBLSCAN)
}

#[derive(Clone, Debug)]
pub struct Output {
    /// The connector's object id, which `Display::on_connector` takes.
    pub id: u32,
    pub name: String,
    pub connected: bool,
    /// A panel built into the machine (eDP, LVDS, DSI), not a monitor plugged into it.
    pub internal: bool,
    /// The physical size the display reports (its EDID), in millimetres; 0 when unknown.
    pub size_mm: (u32, u32),
    /// What the display says of itself, if it says it (a connected one's EDID).
    pub edid: Option<Edid>,
    /// The modes it can be driven at, each once (progressive only), as the card lists them.
    pub modes: Vec<Mode>,
    /// Whether the display and the card can refresh it at a variable rate (its connector's
    /// `vrr_capable`: Adaptive-Sync, FreeSync), within its EDID's `refresh_range_hz`.
    pub vrr_capable: bool,
}

impl Output {
    /// The mode it is driven at: its preferred (the panel's native), else its first.
    pub fn preferred_mode(&self) -> Option<&Mode> {
        self.modes
            .iter()
            .find(|mode| mode.preferred)
            .or(self.modes.first())
    }

    /// Who the display is (docs/adr/0018-several-displays.mdx): its EDID's manufacturer, product
    /// and serial, so that its settings follow it to any port; without a serial, its maker and
    /// product on this connector; without an EDID, the connector.
    pub fn identity(&self) -> String {
        match &self.edid {
            Some(edid) => edid.identity().unwrap_or_else(|| {
                format!(
                    "{}-{:04X}-{}",
                    edid.manufacturer,
                    edid.product,
                    identity_safe(&self.name)
                )
            }),
            None => identity_safe(&self.name),
        }
    }
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
        Self::from_fd(file.into())
    }

    /// The same with a card someone else opened (the session's seat manager, which makes it
    /// DRM master while the session is active).
    pub fn from_fd(fd: std::os::fd::OwnedFd) -> io::Result<Self> {
        let card = Self {
            file: File::from(fd),
            flips: RefCell::new(Vec::new()),
            render: std::cell::OnceCell::new(),
        };
        card.set_client_capability(ClientCapability::UniversalPlanes, true)?;
        card.set_client_capability(ClientCapability::Atomic, true)?;
        Ok(card)
    }

    /// Imports a DMA-BUF holding XRGB8888 pixels with the linear modifier (rows `pitch` bytes
    /// apart) as a framebuffer. Needs no DRM master: this is the check that a buffer from
    /// another device or API can be scanned out.
    pub fn import_dmabuf(
        &self,
        fd: BorrowedFd<'_>,
        width: u32,
        height: u32,
        pitch: u32,
    ) -> io::Result<drm::control::framebuffer::Handle> {
        self.import_dmabuf_planes(
            &[DmabufPlane {
                fd,
                offset: 0,
                pitch,
            }],
            width,
            height,
            DrmFourcc::Xrgb8888,
            0,
        )
    }

    /// Imports a DMA-BUF of `width` × `height` in `format` with `modifier`, its planes as given
    /// (up to four), as a framebuffer: a client's buffer scanned out directly
    /// (docs/adr/0031-gpu-buffers.mdx, decision 8). The kernel checks the layout against the
    /// buffer and the modifier against the card.
    pub fn import_dmabuf_planes(
        &self,
        planes: &[DmabufPlane<'_>],
        width: u32,
        height: u32,
        format: DrmFourcc,
        modifier: u64,
    ) -> io::Result<drm::control::framebuffer::Handle> {
        if planes.is_empty() || planes.len() > 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a framebuffer has 1 to 4 planes",
            ));
        }
        let mut imported = Imported {
            handles: [None; 4],
            size: (width, height),
            format,
            modifier: Some(modifier),
            pitches: [0; 4],
            offsets: [0; 4],
        };
        let mut handles = Vec::new();
        let mut result = Ok(());
        for (index, plane) in planes.iter().enumerate() {
            match self.prime_fd_to_buffer(plane.fd) {
                Ok(handle) => {
                    imported.handles[index] = Some(handle);
                    imported.pitches[index] = plane.pitch;
                    imported.offsets[index] = plane.offset;
                    handles.push(handle);
                }
                Err(error) => {
                    result = Err(error);
                    break;
                }
            }
        }
        // A card without modifiers (virtio-gpu's) takes a linear buffer without naming one.
        let modifiers = self
            .get_driver_capability(drm::DriverCapability::AddFB2Modifiers)
            .is_ok_and(|value| value == 1);
        let flags = if modifiers || modifier != 0 {
            drm::control::FbCmd2Flags::MODIFIERS
        } else {
            imported.modifier = None;
            drm::control::FbCmd2Flags::empty()
        };
        let framebuffer = result.and_then(|()| self.add_planar_framebuffer(&imported, flags));
        // The framebuffer keeps its own references to the buffers (planes of one buffer share a
        // handle, closed once).
        handles.sort_by_key(|&handle| u32::from(handle));
        handles.dedup();
        for handle in handles {
            let _ = self.close_buffer(handle);
        }
        framebuffer
    }

    /// Removes a framebuffer made by `import_dmabuf_planes`.
    pub fn remove_framebuffer(&self, framebuffer: drm::control::framebuffer::Handle) {
        let _ = self.destroy_framebuffer(framebuffer);
    }

    /// The device number (`st_rdev`) of this card's render node, from sysfs
    /// (`/sys/dev/char/MAJOR:MINOR/device/drm/renderD*/dev`); none when it has none.
    pub fn render_node(&self) -> Option<u64> {
        *self.render.get_or_init(|| {
            let stat = rustix::fs::fstat(&self.file).ok()?;
            let (major, minor) = (
                rustix::fs::major(stat.st_rdev),
                rustix::fs::minor(stat.st_rdev),
            );
            let drm = std::path::PathBuf::from(format!("/sys/dev/char/{major}:{minor}/device/drm"));
            std::fs::read_dir(drm)
                .ok()?
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
                .find_map(|entry| {
                    let dev = std::fs::read_to_string(entry.path().join("dev")).ok()?;
                    let (major, minor) = dev.trim().split_once(':')?;
                    Some(rustix::fs::makedev(
                        major.parse().ok()?,
                        minor.parse().ok()?,
                    ))
                })
        })
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
            let internal = matches!(
                info.interface(),
                connector::Interface::EmbeddedDisplayPort
                    | connector::Interface::LVDS
                    | connector::Interface::DSI
            );
            let mut modes: Vec<Mode> = Vec::new();
            for mode in info.modes().iter().filter(|mode| drivable(mode)) {
                let mode = Mode::of(mode);
                // The same pixels and refresh listed twice (two timings for one mode) is one
                // choice; the preferred one's flag is kept.
                match modes.iter_mut().find(|m| {
                    (m.width, m.height, m.refresh_mhz)
                        == (mode.width, mode.height, mode.refresh_mhz)
                }) {
                    Some(listed) => listed.preferred |= mode.preferred,
                    None => modes.push(mode),
                }
            }
            let connected = info.state() == connector::State::Connected;
            outputs.push(Output {
                id: u32::from(handle),
                name,
                connected,
                internal,
                size_mm: info.size().unwrap_or((0, 0)),
                edid: if connected { self.edid(handle) } else { None },
                modes,
                vrr_capable: connected && self.property(handle, b"vrr_capable") == Some(1),
            });
        }
        Ok(outputs)
    }

    /// A connector's EDID, from its `EDID` property's blob; none if it has none or it is not one.
    fn edid(&self, connector: connector::Handle) -> Option<Edid> {
        let blob = self
            .property(connector, b"EDID")
            .filter(|&blob| blob != 0)?;
        Edid::parse(&self.get_property_blob(blob).ok()?)
    }

    /// The value of a connector's property `name`, if it has it.
    fn property(&self, connector: connector::Handle, name: &[u8]) -> Option<u64> {
        let properties = self.get_properties(connector).ok()?;
        let value = properties.iter().find_map(|(&property, &value)| {
            self.get_property(property)
                .is_ok_and(|info| info.name().to_bytes() == name)
                .then_some(value)
        });
        value
    }

    /// The page flips the kernel reported since the last call, and those read earlier for no
    /// display yet, for `crtc`: their timestamps (CLOCK_MONOTONIC). Never blocks; flips of other
    /// CRTCs stay for their displays.
    fn flips_of(&self, crtc: drm::control::crtc::Handle, read: bool) -> io::Result<Vec<Duration>> {
        if read {
            for event in self.receive_events()? {
                if let drm::control::Event::PageFlip(flip) = event {
                    self.flips.borrow_mut().push((flip.crtc, flip.duration));
                }
            }
        }
        let mut flips = self.flips.borrow_mut();
        let mine = flips
            .iter()
            .filter(|(of, _)| *of == crtc)
            .map(|(_, at)| *at)
            .collect();
        flips.retain(|(of, _)| *of != crtc);
        Ok(mine)
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
