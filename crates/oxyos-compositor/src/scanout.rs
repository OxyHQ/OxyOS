//! Direct scanout of a full-screen window (docs/adr/0031-gpu-buffers.mdx, decision 8): when a
//! surface is all an output shows, its client's GPU buffer goes on the display's primary plane
//! instead of a composition, and the surface is told which formats and modifiers that plane
//! takes so it can allocate for it.
//!
//! The decision is made for each frame, from the frame's layers: the buffer must be the topmost
//! layer, covering the screen exactly at its own size, opaque, uncut and with nothing over it.
//! Anything else (a popup, a notification, the lock, a transition, the splash) composes, and a
//! buffer the card refuses composes too.

use super::{Compositor, Event, RenderItem, SurfaceKey, DESKTOP};
use oxyos_dmabuf::MODIFIER_LINEAR;
use oxyos_kms::DrmFourcc;
use oxyos_protocol::Format;
use std::rc::Rc;

/// A client's buffer a display shows or is about to: its serial and what holds its release back.
pub(crate) type Held = (u64, Rc<()>);

/// What a screen scans out, or what it tells its candidate surface.
#[derive(Default)]
pub(crate) struct Direct {
    /// The buffer the display shows, and the one the flip on its way shows: held until a later
    /// flip replaces it.
    pub shown: Option<Held>,
    pub next: Option<Held>,
    /// The surface told its scanout formats, and the formats told.
    pub told: Option<(SurfaceKey, Vec<(Format, u64)>)>,
    /// Whether the frame last handed to the display was a client's buffer: then the director's
    /// copy of the frame is already the screen's (`framebuffer`).
    pub shown_direct: bool,
}

/// Whether a layer is a client's buffer covering a `width` × `height` screen at its own size, at
/// full opacity, square and uncut.
pub fn covers_screen(layer: &oxyos_gpu::Layer<'_>, width: u32, height: u32) -> bool {
    let whole = oxyos_gpu::Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    layer.x == 0
        && layer.y == 0
        && (layer.width, layer.height) == (width, height)
        && layer.extent == (width, height)
        && layer.opacity == 255
        && layer.radius == 0
        && layer
            .clip
            .is_none_or(|clip| clip.intersect(&whole) == Some(whole))
        && matches!(
            layer.paint,
            oxyos_gpu::Paint::Imported { .. }
                | oxyos_gpu::Paint::Opaque(_)
                | oxyos_gpu::Paint::Premultiplied(_)
        )
}

/// The scanout formats of a surface: the compositor's offer that the primary plane takes,
/// opaque (XRGB8888: nothing is under the plane to blend with), on the composing device or
/// linear (which any device reads).
pub fn scanout_formats(
    offer: &[(Format, u64, u32)],
    primary: &[(DrmFourcc, u64)],
    same_device: bool,
) -> Vec<(Format, u64)> {
    offer
        .iter()
        .filter(|&&(format, modifier, _)| {
            format == Format::Xrgb8888
                && (same_device || modifier == MODIFIER_LINEAR)
                && primary.contains(&(DrmFourcc::Xrgb8888, modifier))
        })
        .map(|&(format, modifier, _)| (format, modifier))
        .collect()
}

impl Compositor {
    /// Whether screen `index`'s display shows a client's buffer directly now.
    pub fn shows_client_buffer(&self, index: usize) -> bool {
        self.screens[index].direct.shown.is_some()
    }

    /// The surface a screen would scan out and its formats, if any.
    fn wanted_scanout(&self, index: usize) -> Option<(SurfaceKey, Vec<(Format, u64)>)> {
        let screen = &self.screens[index];
        let display = screen.display.as_ref()?;
        let offer = self.gpu_buffers.offer.as_ref()?;
        let key = self.scene.scanout_candidate(screen.id)?;
        let same = offer.device != 0 && display.card().render_node() == Some(offer.device);
        let formats = scanout_formats(&offer.formats, display.primary_formats(), same);
        (!formats.is_empty()).then_some((key, formats))
    }

    /// Tells a screen's candidate surface its scanout formats when it becomes one or they change,
    /// and a surface that stops being one that it is not (decision 8).
    pub(crate) fn tell_scanout(&mut self, index: usize) {
        let wanted = self.wanted_scanout(index);
        let told = &self.screens[index].direct.told;
        if *told == wanted {
            return;
        }
        if let Some((old, _)) = told.clone() {
            if wanted.as_ref().is_none_or(|(key, _)| *key != old) {
                self.send_to(
                    old.client,
                    Event::GpuScanout {
                        surface: old.id,
                        formats: 0,
                    },
                );
            }
        }
        if let Some((key, formats)) = &wanted {
            self.send_to(
                key.client,
                Event::GpuScanout {
                    surface: key.id,
                    formats: formats.len() as u32,
                },
            );
            for (index, &(format, modifier)) in formats.iter().enumerate() {
                self.send_to(
                    key.client,
                    Event::GpuScanoutFormat {
                        surface: key.id,
                        index: index as u32,
                        format,
                        modifier,
                    },
                );
            }
        }
        self.screens[index].direct.told = wanted;
    }

    /// Shows the screen's frame by putting its full-screen window's buffer on the primary plane,
    /// if it can: `Some` with whether it was handed to the display, `None` to compose it.
    pub(crate) fn present_direct(
        &mut self,
        index: usize,
        layers: &[oxyos_gpu::Layer<'_>],
    ) -> Option<bool> {
        if self.away || !self.splash.is_empty() || !self.transitions.is_empty() {
            return None;
        }
        let (key, _) = self.screens[index].direct.told.clone()?;
        let screen = &self.screens[index];
        let display = screen.display.as_ref()?;
        let (width, height) = display.size();
        if (width, height) != (screen.width, screen.height) {
            return None;
        }
        // The candidate's frame, on top of everything the screen draws.
        let frame = self
            .scene
            .frame_list()
            .into_iter()
            .find_map(|item| match item {
                RenderItem::Surface {
                    key: shown, frame, ..
                } if shown == key => Some(frame),
                _ => None,
            })?;
        let top = layers.last()?;
        if !covers_screen(top, width, height) {
            return None;
        }
        let source = self.gpu_buffers.scanout_source(key.client, frame.buffer)?;
        let offer = self.gpu_buffers.offer.as_ref()?;
        let same = offer.device != 0 && display.card().render_node() == Some(offer.device);
        if source.format != Format::Xrgb8888
            || source.extent.width != width
            || source.extent.height != height
            || !scanout_formats(&offer.formats, display.primary_formats(), same)
                .contains(&(source.format, source.modifier))
        {
            return None;
        }
        // With a Vulkan device, the top layer must be this very buffer, sampled from its import.
        match (top.paint, source.image) {
            (oxyos_gpu::Paint::Imported { image, .. }, Some(held)) if image == held => {}
            (oxyos_gpu::Paint::Imported { .. }, _) => return None,
            _ if self.gpu.is_some() && self.gpu_only => return None,
            _ => {}
        }
        let (serial, modifier, uses) = (source.serial, source.modifier, source.uses.clone());
        let planes: Vec<oxyos_kms::DmabufPlane<'_>> = source
            .planes
            .iter()
            .map(|&(fd, offset, pitch)| oxyos_kms::DmabufPlane { fd, offset, pitch })
            .collect();
        let display = self.screens[index].display.as_mut()?;
        let framebuffer = display.client_framebuffer(serial, |card| {
            card.import_dmabuf_planes(&planes, width, height, DrmFourcc::Xrgb8888, modifier)
        })?;
        drop(planes);
        if let Err(error) = display.present_framebuffer_after(framebuffer, None) {
            // The card took the framebuffer but not the commit: composed from now on.
            eprintln!("[kms] a client's buffer could not be shown directly: {error}");
            display.refuse_client_framebuffer(serial);
            return None;
        }
        let submitted = super::monotonic();
        let screen = &mut self.screens[index];
        if !screen.direct.shown_direct {
            eprintln!(
                "[compositor] output {} shows a full-screen window's buffer directly (modifier {modifier:#x})",
                screen.id
            );
        }
        screen.direct.next = Some((serial, uses));
        screen.direct.shown_direct = true;
        self.direct_frames += 1;
        // The scanout targets hold frames older than the screen now shows: composed whole again.
        if let Some(gpu) = &mut self.gpu {
            let targets: Vec<usize> = screen.scanout.iter().map(|&(target, _)| target).collect();
            gpu.compositor.forget_scanout_contents(&targets);
            // The director's copy of the frame shown.
            if self.gpu_only && self.reads_back {
                screen.shown_kept = false;
                match gpu.compositor.compose(width, height, DESKTOP, layers) {
                    Ok(frame) => screen.framebuffer = frame,
                    Err(error) => {
                        eprintln!("[gpu] {error}");
                        gpu.mismatches += 1;
                    }
                }
            }
        }
        if self.records {
            self.compositions
                .push((submitted - self.composing_since).as_secs_f64() * 1e3);
        }
        Some(self.handed_over(index, Ok(()), submitted))
    }

    /// A screen composes its frame: what it scanned out goes once that frame flips.
    pub(crate) fn composing_again(&mut self, index: usize) {
        let screen = &mut self.screens[index];
        screen.direct.next = None;
        if screen.direct.shown_direct {
            screen.direct.shown_direct = false;
            eprintln!("[compositor] output {} composes again", screen.id);
        }
    }

    /// Each turn: a screen with no display holds nothing and tells no surface it could be scanned
    /// out; the framebuffers of buffers gone are removed from every display.
    pub(crate) fn follow_direct(&mut self) {
        for index in 0..self.screens.len() {
            if self.screens[index].display.is_none() {
                let direct = &mut self.screens[index].direct;
                direct.shown = None;
                direct.next = None;
                direct.shown_direct = false;
                if direct.told.is_some() {
                    self.tell_scanout(index);
                }
            }
        }
        let gone = std::mem::take(&mut self.gpu_buffers.gone);
        for serial in gone {
            for screen in &mut self.screens {
                if let Some(display) = &mut screen.display {
                    display.forget_client_framebuffer(serial);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
