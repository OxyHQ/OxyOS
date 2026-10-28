//! Output on a connector with atomic modesetting: two dumb buffers, a modeset for the first
//! frame and page flips after it, with the kernel's flip timestamps. Needs DRM master (the
//! card's current master, a running desktop session, must have released it).

use crate::Card;
use drm::buffer::{Buffer as _, DrmFourcc};
use drm::control::dumbbuffer::DumbBuffer;
use drm::control::{
    atomic, connector, crtc, framebuffer, plane, property, AtomicCommitFlags, Device as _, Event,
    Mode,
};
use drm::Device as _;
use std::io;
use std::time::Duration;

/// A connector driven by this process: its mode, CRTC, primary plane and two buffers.
pub struct Display {
    card: Card,
    connector: connector::Handle,
    crtc: crtc::Handle,
    plane: plane::Handle,
    mode: Mode,
    buffers: Vec<(DumbBuffer, framebuffer::Handle)>,
    back: usize,
    configured: bool,
    mode_blob: Option<property::Value<'static>>,
}

fn not_found(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, what.to_string())
}

impl Display {
    /// Takes DRM master and prepares the first connected connector at its preferred mode.
    pub fn open(card: Card) -> io::Result<Self> {
        card.acquire_master_lock()?;
        let resources = card.resource_handles()?;
        let (connector, mode) = resources
            .connectors()
            .iter()
            .filter_map(|&handle| {
                let info = card.get_connector(handle, false).ok()?;
                if info.state() != connector::State::Connected {
                    return None;
                }
                let mode = info
                    .modes()
                    .iter()
                    .find(|m| {
                        m.mode_type()
                            .contains(drm::control::ModeTypeFlags::PREFERRED)
                    })
                    .or(info.modes().first())
                    .copied()?;
                Some((handle, mode))
            })
            .next()
            .ok_or_else(|| not_found("no connected output"))?;
        let info = card.get_connector(connector, false)?;
        let crtc = info
            .encoders()
            .iter()
            .filter_map(|&encoder| card.get_encoder(encoder).ok())
            .flat_map(|encoder| resources.filter_crtcs(encoder.possible_crtcs()))
            .next()
            .ok_or_else(|| not_found("no CRTC for the output"))?;
        let primary = card
            .plane_handles()?
            .into_iter()
            .find(|&handle| {
                let Ok(info) = card.get_plane(handle) else {
                    return false;
                };
                resources
                    .filter_crtcs(info.possible_crtcs())
                    .contains(&crtc)
                    && property_value(&card, handle, "type").ok() == Some(1)
            })
            .ok_or_else(|| not_found("no primary plane for the CRTC"))?;
        let (width, height) = mode.size();
        let mut display = Self {
            card,
            connector,
            crtc,
            plane: primary,
            mode,
            buffers: Vec::new(),
            back: 0,
            configured: false,
            mode_blob: None,
        };
        for _ in 0..2 {
            let buffer = display.card.create_dumb_buffer(
                (u32::from(width), u32::from(height)),
                DrmFourcc::Xrgb8888,
                32,
            )?;
            let framebuffer = display.card.add_framebuffer(&buffer, 24, 32)?;
            display.buffers.push((buffer, framebuffer));
        }
        Ok(display)
    }

    /// The card this display drives (to import buffers into it, for one).
    pub fn card(&self) -> &Card {
        &self.card
    }

    pub fn size(&self) -> (u32, u32) {
        let (width, height) = self.mode.size();
        (u32::from(width), u32::from(height))
    }

    pub fn refresh_hz(&self) -> u32 {
        self.mode.vrefresh()
    }

    pub fn mode_name(&self) -> String {
        format!(
            "{}x{}@{}",
            self.mode.size().0,
            self.mode.size().1,
            self.mode.vrefresh()
        )
    }

    /// Lets go of the output for another session (a terminal switch): drops DRM master. Wait for
    /// the last flip first. The next frame presented after [`resume`](Self::resume) modesets
    /// again, since the other session may have changed the output.
    pub fn suspend(&mut self) -> io::Result<()> {
        self.card.release_master_lock()?;
        self.configured = false;
        Ok(())
    }

    /// Takes DRM master again after [`suspend`](Self::suspend).
    pub fn resume(&mut self) -> io::Result<()> {
        self.card.acquire_master_lock()
    }

    /// Writes `pixels` (0x00RRGGBB, row by row at the display's size) into the back buffer.
    pub fn draw(&mut self, pixels: &[u32]) -> io::Result<()> {
        let (width, height) = self.size();
        let (buffer, _) = &mut self.buffers[self.back];
        let pitch = buffer.pitch() as usize;
        let mut mapping = self.card.map_dumb_buffer(buffer)?;
        for (y, row) in pixels
            .chunks_exact(width as usize)
            .take(height as usize)
            .enumerate()
        {
            let out = &mut mapping[y * pitch..y * pitch + width as usize * 4];
            for (pixel, bytes) in row.iter().zip(out.chunks_exact_mut(4)) {
                bytes.copy_from_slice(&pixel.to_le_bytes());
            }
        }
        Ok(())
    }

    /// Shows the back buffer: a modeset the first time, a page flip after it, each asking for a
    /// flip event (`wait_flip`).
    pub fn present(&mut self) -> io::Result<()> {
        let framebuffer = self.buffers[self.back].1;
        self.present_framebuffer(framebuffer)?;
        self.back = 1 - self.back;
        Ok(())
    }

    /// Shows any framebuffer of the display's size (an imported DMA-BUF, for one): a modeset
    /// the first time, a page flip after it.
    pub fn present_framebuffer(&mut self, framebuffer: framebuffer::Handle) -> io::Result<()> {
        let card = &self.card;
        let mut request = atomic::AtomicModeReq::new();
        let mut flags = AtomicCommitFlags::PAGE_FLIP_EVENT;
        if !self.configured {
            let blob = card.create_property_blob(&self.mode)?;
            let (width, height) = self.size();
            request.add_property(
                self.connector,
                property_handle(card, self.connector, "CRTC_ID")?,
                property::Value::CRTC(Some(self.crtc)),
            );
            request.add_property(
                self.crtc,
                property_handle(card, self.crtc, "MODE_ID")?,
                blob,
            );
            request.add_property(
                self.crtc,
                property_handle(card, self.crtc, "ACTIVE")?,
                property::Value::Boolean(true),
            );
            for (name, value) in [
                ("CRTC_ID", property::Value::CRTC(Some(self.crtc))),
                ("SRC_X", property::Value::UnsignedRange(0)),
                ("SRC_Y", property::Value::UnsignedRange(0)),
                (
                    "SRC_W",
                    property::Value::UnsignedRange(u64::from(width) << 16),
                ),
                (
                    "SRC_H",
                    property::Value::UnsignedRange(u64::from(height) << 16),
                ),
                ("CRTC_X", property::Value::SignedRange(0)),
                ("CRTC_Y", property::Value::SignedRange(0)),
                ("CRTC_W", property::Value::UnsignedRange(u64::from(width))),
                ("CRTC_H", property::Value::UnsignedRange(u64::from(height))),
            ] {
                request.add_property(self.plane, property_handle(card, self.plane, name)?, value);
            }
            // Planes another session left on the CRTC (a cursor, an overlay) are turned off.
            for plane in card.plane_handles()? {
                if plane == self.plane {
                    continue;
                }
                let on_crtc = card
                    .get_plane(plane)
                    .is_ok_and(|info| info.crtc() == Some(self.crtc));
                if on_crtc {
                    request.add_property(
                        plane,
                        property_handle(card, plane, "CRTC_ID")?,
                        property::Value::CRTC(None),
                    );
                    request.add_property(
                        plane,
                        property_handle(card, plane, "FB_ID")?,
                        property::Value::Framebuffer(None),
                    );
                }
            }
            self.mode_blob = Some(blob);
            flags |= AtomicCommitFlags::ALLOW_MODESET;
        } else {
            flags |= AtomicCommitFlags::NONBLOCK;
        }
        request.add_property(
            self.plane,
            property_handle(card, self.plane, "FB_ID")?,
            property::Value::Framebuffer(Some(framebuffer)),
        );
        card.atomic_commit(flags, request)?;
        self.configured = true;
        Ok(())
    }

    /// Waits for the presented frame's flip; returns the kernel's timestamp of it
    /// (CLOCK_MONOTONIC).
    pub fn wait_flip(&self) -> io::Result<Duration> {
        loop {
            for event in self.card.receive_events()? {
                if let Event::PageFlip(flip) = event {
                    if flip.crtc == self.crtc {
                        return Ok(flip.duration);
                    }
                }
            }
        }
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        for (buffer, framebuffer) in self.buffers.drain(..) {
            let _ = self.card.destroy_framebuffer(framebuffer);
            let _ = self.card.destroy_dumb_buffer(buffer);
        }
        if let Some(property::Value::Blob(blob)) = self.mode_blob.take() {
            let _ = self.card.destroy_property_blob(blob);
        }
        let _ = self.card.release_master_lock();
    }
}

fn property_handle(
    card: &Card,
    handle: impl drm::control::ResourceHandle,
    name: &str,
) -> io::Result<property::Handle> {
    card.get_properties(handle)?
        .as_hashmap(card)?
        .get(name)
        .map(|info| info.handle())
        .ok_or_else(|| not_found(&format!("property {name}")))
}

fn property_value(
    card: &Card,
    handle: impl drm::control::ResourceHandle,
    name: &str,
) -> io::Result<u64> {
    let properties = card.get_properties(handle)?;
    for (&property, &value) in properties.iter() {
        if card.get_property(property)?.name().to_bytes() == name.as_bytes() {
            return Ok(value);
        }
    }
    Err(not_found(&format!("property {name}")))
}
