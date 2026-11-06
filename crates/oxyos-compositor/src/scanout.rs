//! Direct scanout of a full-screen window (docs/adr/0031-gpu-buffers.mdx, decision 8): when a
//! surface is all an output shows, its client's GPU buffer goes on the display's primary plane
//! instead of a composition, and the surface is told which formats and modifiers that plane
//! takes so it can allocate for it.
//!
//! The decision is made for each frame, from the frame's layers: the buffer must be the topmost
//! layer, covering the screen exactly at its own size, opaque, uncut and with nothing over it.
//! Anything else (a popup, a notification, the lock, a transition, the splash) composes, and a
//! buffer the card refuses composes too.
//!
//! A window that is not all the output shows goes on an overlay plane instead (decision 15), as
//! a Mac's display controller takes a window's surface: each frame assigns the plane. The
//! candidate is the topmost layer that is a plain opaque rectangle (a client's buffer at its own
//! size, opaque, square, at full opacity, uncut, within the screen: a maximized window), told the
//! overlay plane's formats; it goes on the plane while no layer above it touches it, the card
//! having taken it at that place in a TEST_ONLY commit first (once for each buffer and place).
//! Under it the primary plane's frame is not kept: a frame composes only what changed outside
//! the window, and a frame of the window alone composes nothing, its buffer committed to the
//! overlay plane alone. What the plane stops covering is composed again in the frame that stops
//! it. A window with rounded corners never goes on a plane (its corners show what is under it),
//! and its shadow, under it, is composed with the rest.

use super::{Compositor, Event, RenderItem, SurfaceKey, DESKTOP};
use oxyos_dmabuf::MODIFIER_LINEAR;
use oxyos_kms::DrmFourcc;
use oxyos_protocol::Format;
use std::collections::HashMap;
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
    /// The surface whose buffer could go on the overlay plane in this frame (told its formats).
    pub overlay_candidate: Option<SurfaceKey>,
    /// Where the overlay plane shows a window's buffer, as last handed to the display (screen
    /// pixels): the primary plane's frame there is not kept.
    pub overlay: Option<oxyos_gpu::Rect>,
    /// Whether the card took each buffer at each place on the overlay plane (TEST_ONLY), by its
    /// serial and place on the display.
    pub tested: HashMap<(u64, oxyos_gpu::Rect), bool>,
}

/// What goes on the overlay plane in a frame: where, its surface's buffer held while shown.
pub(crate) struct Assigned {
    pub placement: oxyos_kms::Placement,
    /// Its place on the screen (screen pixels).
    pub rect: oxyos_gpu::Rect,
    pub held: Held,
    pub modifier: u64,
}

/// Whether a layer is a plain opaque rectangle a plane can show as it is: a client's buffer at
/// its own size, opaque, square, at full opacity, uncut, through no mesh, within a `width` ×
/// `height` screen.
pub fn plain(layer: &oxyos_gpu::Layer<'_>, width: u32, height: u32) -> bool {
    let rect = drawn(layer);
    layer.x >= 0
        && layer.y >= 0
        && i64::from(layer.x) + i64::from(layer.width) <= i64::from(width)
        && i64::from(layer.y) + i64::from(layer.height) <= i64::from(height)
        && layer.width > 0
        && layer.height > 0
        && layer.extent == (layer.width, layer.height)
        && layer.opacity == 255
        && layer.radius == 0
        && layer.warp.is_none()
        && layer
            .clip
            .is_none_or(|clip| clip.intersect(&rect) == Some(rect))
        && matches!(layer.paint, oxyos_gpu::Paint::Imported { opaque: true, .. })
}

/// A layer's rectangle on the screen.
fn drawn(layer: &oxyos_gpu::Layer<'_>) -> oxyos_gpu::Rect {
    oxyos_gpu::Rect {
        x: layer.x,
        y: layer.y,
        width: layer.width,
        height: layer.height,
    }
}

/// The layer the overlay plane could show, the topmost plain one (`plain`), and whether nothing
/// above it touches it (it can go on the plane in this frame).
pub fn overlay_layer(
    layers: &[oxyos_gpu::Layer<'_>],
    width: u32,
    height: u32,
) -> Option<(usize, bool)> {
    let index = layers
        .iter()
        .rposition(|layer| plain(layer, width, height))?;
    let rect = drawn(&layers[index]);
    let clear = layers[index + 1..].iter().all(|above| {
        let mut touched = drawn(above);
        if let Some(clip) = above.clip {
            match touched.intersect(&clip) {
                Some(within) => touched = within,
                None => return true,
            }
        }
        touched.intersect(&rect).is_none()
    });
    Some((index, clear))
}

/// Whether the card takes a buffer at a place (`key`: its serial and place) on the overlay
/// plane: asked once (`test`, a TEST_ONLY commit), the answer kept for every frame after.
pub(crate) fn taken(
    tested: &mut HashMap<(u64, oxyos_gpu::Rect), bool>,
    key: (u64, oxyos_gpu::Rect),
    test: impl FnOnce() -> std::io::Result<()>,
) -> bool {
    *tested.entry(key).or_insert_with(|| test().is_ok())
}

/// What a frame composes with the overlay plane covering `now` (none: off) after a frame that had
/// it at `before`: what changed and what the plane stops covering, less what it covers, on a
/// `screen`; `None` (everything) stays everything without a plane.
pub fn under_overlay(
    damage: Option<&[oxyos_gpu::Rect]>,
    before: Option<oxyos_gpu::Rect>,
    now: Option<oxyos_gpu::Rect>,
    screen: oxyos_gpu::Rect,
) -> Option<Vec<oxyos_gpu::Rect>> {
    let mut rects = match (damage, now) {
        (None, None) => return None,
        (None, Some(_)) => vec![screen],
        (Some(damage), _) => damage.to_vec(),
    };
    if let Some(before) = before {
        match now {
            Some(now) => rects.extend(before.without(&now)),
            None => rects.push(before),
        }
    }
    Some(match now {
        Some(now) => rects.iter().flat_map(|rect| rect.without(&now)).collect(),
        None => rects,
    })
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

    /// The surface a screen would scan out and its formats, if any: a full-screen window, the
    /// primary plane's; else the overlay plane's candidate, the overlay plane's.
    fn wanted_scanout(&self, index: usize) -> Option<(SurfaceKey, Vec<(Format, u64)>)> {
        let screen = &self.screens[index];
        let display = screen.display.as_ref()?;
        let offer = self.gpu_buffers.offer.as_ref()?;
        let same = offer.device != 0 && display.card().render_node() == Some(offer.device);
        let (key, plane) = match self.scene.scanout_candidate(screen.id) {
            Some(key) => (key, display.primary_formats()),
            None => (screen.direct.overlay_candidate?, display.overlay_formats()),
        };
        let formats = scanout_formats(&offer.formats, plane, same);
        (!formats.is_empty()).then_some((key, formats))
    }

    /// The overlay plane's candidate of a screen's frame (`overlay_layer`), its surface told the
    /// plane's formats; and the layer, if it can go on the plane in this frame. Only where the
    /// frame is composed on the GPU into the display's buffers, and nothing changes the whole
    /// screen (a transition, the splash).
    pub(crate) fn overlay_candidate(
        &mut self,
        index: usize,
        layers: &[oxyos_gpu::Layer<'_>],
    ) -> Option<usize> {
        let screen = &self.screens[index];
        let found = (self.gpu.is_some()
            && !screen.scanout.is_empty()
            && screen
                .display
                .as_ref()
                .is_some_and(|display| !display.overlay_formats().is_empty()))
        .then(|| overlay_layer(layers, screen.width, screen.height))
        .flatten()
        .and_then(|(layer, clear)| {
            // A surface's current frame, drawn where the scene has it.
            let source = layers[layer].source?;
            let key = SurfaceKey {
                client: (source.key >> 32) as u32,
                id: source.key as u32,
            };
            let current = self.scene.frame_list().into_iter().any(|item| {
                matches!(item, RenderItem::Surface { key: shown, frame, .. }
                    if shown == key && self.versions.get(&(key, frame.sequence)) == Some(&source.version))
            });
            current.then_some((key, layer, clear))
        });
        self.screens[index].direct.overlay_candidate = found.map(|(key, _, _)| key);
        found.and_then(|(_, layer, clear)| {
            (clear && !self.away && self.splash.is_empty() && self.transitions.is_empty())
                .then_some(layer)
        })
    }

    /// The overlay plane's assignment for a screen's frame: its candidate's buffer (`layer` of
    /// `layers`) where the card takes it, imported as a framebuffer once and tested at its place
    /// once (TEST_ONLY); none where it cannot go there.
    pub(crate) fn assign_overlay(
        &mut self,
        index: usize,
        layers: &[oxyos_gpu::Layer<'_>],
        layer: usize,
    ) -> Option<Assigned> {
        let key = self.screens[index].direct.overlay_candidate?;
        let top = &layers[layer];
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
        let source = self.gpu_buffers.scanout_source(key.client, frame.buffer)?;
        // The very buffer the frame samples, from its import.
        match (top.paint, source.image) {
            (oxyos_gpu::Paint::Imported { image, .. }, Some(held)) if image == held => {}
            _ => return None,
        }
        let screen = &self.screens[index];
        let display = screen.display.as_ref()?;
        let offer = self.gpu_buffers.offer.as_ref()?;
        let same = offer.device != 0 && display.card().render_node() == Some(offer.device);
        if source.format != Format::Xrgb8888
            || (source.extent.width, source.extent.height) != (top.width, top.height)
            || !scanout_formats(&offer.formats, display.overlay_formats(), same)
                .contains(&(source.format, source.modifier))
        {
            return None;
        }
        let (width, height) = display.size();
        let dx = (width.saturating_sub(screen.width) / 2) as i32;
        let dy = (height.saturating_sub(screen.height) / 2) as i32;
        let rect = drawn(top);
        let (serial, modifier, uses) = (source.serial, source.modifier, source.uses.clone());
        let planes: Vec<oxyos_kms::DmabufPlane<'_>> = source
            .planes
            .iter()
            .map(|&(fd, offset, pitch)| oxyos_kms::DmabufPlane { fd, offset, pitch })
            .collect();
        let display = self.screens[index].display.as_mut()?;
        let framebuffer = display.client_framebuffer(serial, |card| {
            card.import_dmabuf_planes(
                &planes,
                rect.width,
                rect.height,
                DrmFourcc::Xrgb8888,
                modifier,
            )
        })?;
        drop(planes);
        let placement = oxyos_kms::Placement {
            framebuffer,
            x: rect.x + dx,
            y: rect.y + dy,
            width: rect.width,
            height: rect.height,
        };
        let at = oxyos_gpu::Rect {
            x: placement.x,
            y: placement.y,
            ..rect
        };
        let screen = &mut self.screens[index];
        let display = screen.display.as_ref()?;
        let taken = taken(&mut screen.direct.tested, (serial, at), || {
            display.test_overlay(placement).inspect_err(|error| {
                eprintln!(
                    "[kms] the card does not take a window's buffer (modifier {modifier:#x}) on its overlay plane at {at:?}: {error}; it is composed"
                );
            })
        });
        taken.then_some(Assigned {
            placement,
            rect,
            held: (serial, uses),
            modifier,
        })
    }

    /// The card refused a commit with `assigned` on the overlay plane though its test took it:
    /// that buffer at that place is composed from now on.
    pub(crate) fn refuse_overlay(&mut self, index: usize, assigned: &Assigned) {
        let at = oxyos_gpu::Rect {
            x: assigned.placement.x,
            y: assigned.placement.y,
            ..assigned.rect
        };
        self.screens[index]
            .direct
            .tested
            .insert((assigned.held.0, at), false);
    }

    /// The director's copy of the frame a screen shows when part of it is not composed (a
    /// client's buffer on a plane): composed whole, as the reference has it.
    pub(crate) fn director_copy(&mut self, index: usize, layers: &[oxyos_gpu::Layer<'_>]) {
        let (Some(gpu), true) = (&mut self.gpu, self.gpu_only && self.reads_back) else {
            return;
        };
        let screen = &mut self.screens[index];
        screen.shown_kept = false;
        match gpu
            .compositor
            .compose(screen.width, screen.height, DESKTOP, layers)
        {
            Ok(frame) => screen.framebuffer = frame,
            Err(error) => {
                eprintln!("[gpu] {error}");
                gpu.mismatches += 1;
            }
        }
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
        // Nothing shows over a full-screen window.
        display.set_overlay(None);
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
        screen.direct.overlay = None;
        if let Some(gpu) = &mut self.gpu {
            let targets: Vec<usize> = screen.scanout.iter().map(|&(target, _)| target).collect();
            gpu.compositor.forget_scanout_contents(&targets);
        }
        // The director's copy of the frame shown.
        self.director_copy(index, layers);
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
                direct.overlay = None;
                direct.overlay_candidate = None;
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
                screen
                    .direct
                    .tested
                    .retain(|(tested, _), _| *tested != serial);
            }
        }
    }
}

#[cfg(test)]
mod tests;
