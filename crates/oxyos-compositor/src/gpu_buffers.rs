//! Clients' GPU buffers (docs/adr/0031-gpu-buffers.mdx): what the compositor offers (decision 2),
//! the DMA-BUFs it checks and imports (decisions 3, 10 and 11), the timelines it imports and the
//! frames it holds until their acquire point signals (decision 6), and their release.
//!
//! The scene knows a GPU buffer as any buffer; this module owns what only a GPU buffer has. A
//! frame waiting for its acquire point is not the scene's yet: it is given the `Submit` once the
//! point signals (`Compositor::follow_gpu_sync`), so nothing of a client's fence is ever waited
//! for, on the CPU or in the compositor's GPU work.

use super::{Compositor, Event};
use oxyos_compositor_core::{ClientId, SurfaceKey};
use oxyos_dmabuf::{DmaBuf, Mapping, RenderNode, Timeline, MODIFIER_LINEAR};
use oxyos_protocol::{Clock, ErrorCode, Format, Outcome, Request, Size};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

/// A connection's quotas (decision 11).
pub const MAX_GPU_BUFFERS: usize = 64;
pub const MAX_TIMELINES: usize = 16;
pub const MAX_GPU_BYTES: u64 = 1 << 30;
/// Planes of one DMA-BUF, at most (as DRM's framebuffers).
pub const MAX_PLANES: usize = 4;

/// What the compositor offers clients' GPU buffers: the render node its timelines are on, the
/// device it composes on (0: none, the CPU) and each format and modifier with its planes.
pub struct Offer {
    node: Arc<RenderNode>,
    pub(crate) device: u64,
    pub(crate) formats: Vec<(Format, u64, u32)>,
}

/// One plane as attached: its DMA-BUF, offset and stride.
#[derive(Debug)]
struct Plane {
    buffer: DmaBuf,
    offset: u32,
    stride: u32,
}

/// A client's GPU buffer: its planes, and how it is read (imported into the Vulkan compositor,
/// mapped for the CPU, or both with `--gpu`'s check).
pub struct Buffer {
    extent: Size,
    format: Format,
    planes: Vec<Plane>,
    mapping: Option<Mapping>,
    image: Option<u64>,
    bytes: u64,
    modifier: u64,
    /// The compositor's own number for it, unique for the session: what a display knows its
    /// framebuffer by when it is scanned out directly (decision 8).
    serial: u64,
    /// Held by every frame read from it ([`GpuFrame`]): while a transition draws a frame of it,
    /// it is not released.
    uses: Rc<()>,
}

/// A client's GPU buffer as a display scans it out: the session's number for it, its layout
/// and planes, its import, and what holds its release back while a display shows it.
pub(crate) struct ScanoutSource<'a> {
    pub serial: u64,
    pub format: Format,
    pub modifier: u64,
    pub extent: Size,
    pub image: Option<u64>,
    pub planes: Vec<(BorrowedFd<'a>, u32, u32)>,
    pub uses: Rc<()>,
}

/// A frame's pixels in a client's GPU buffer, as a composition samples them in place.
#[derive(Clone, Debug)]
pub struct GpuFrame {
    pub image: u64,
    pub opaque: bool,
    _uses: Rc<()>,
}

/// The points a surface's next frame names (`SetSync`): timeline 0 is none.
#[derive(Clone, Copy, Debug, Default)]
struct Points {
    acquire: (u32, u64),
    release: (u32, u64),
}

/// A frame held until its acquire point signals.
struct Waiting {
    request: Request,
    signalled: OwnedFd,
    buffer: u64,
}

/// A release point to signal once its frame's acquire point has: the frame was superseded before
/// the compositor read it.
struct AfterAcquire {
    signalled: OwnedFd,
    client: ClientId,
    release: (u32, u64),
}

#[derive(Default)]
pub struct GpuBuffers {
    pub(crate) offer: Option<Offer>,
    /// Planes attached for buffers not yet made, by connection and buffer.
    planes: HashMap<(ClientId, u64), Vec<Option<Plane>>>,
    buffers: HashMap<(ClientId, u64), Buffer>,
    /// Well-formed buffers the compositor could not import (`BufferFailed`).
    failed: HashSet<(ClientId, u64)>,
    timelines: HashMap<(ClientId, u32), Timeline>,
    points: HashMap<SurfaceKey, Points>,
    waiting: HashMap<SurfaceKey, Waiting>,
    /// The release point of the frame each buffer shows, signalled when it is released.
    releases: HashMap<(ClientId, u64), (u32, u64)>,
    after_acquire: Vec<AfterAcquire>,
    /// Released events held until no composition and no transition reads their buffer.
    released: Vec<(ClientId, Event)>,
    /// Imports of buffers gone that a transition still draws, by their uses.
    orphans: Vec<(Option<u64>, Rc<()>, u64)>,
    /// Buffers gone whose framebuffers on the displays go too (`Compositor::follow_direct`).
    pub(crate) gone: Vec<u64>,
    next_serial: u64,
    /// Each connection's frames of GPU buffers given to the scene, and how many of them waited
    /// for their acquire point (the development compositor reports them).
    frames: HashMap<ClientId, (u64, u64)>,
}

impl GpuBuffers {
    pub fn offered(&self) -> bool {
        self.offer.is_some()
    }

    /// `GpuDevice` and its formats, for a connection that has just been welcomed.
    pub fn announcement(&self) -> Vec<Event> {
        let Some(offer) = &self.offer else {
            return Vec::new();
        };
        let mut events = vec![Event::GpuDevice {
            device: offer.device,
            formats: offer.formats.len() as u32,
        }];
        events.extend(
            offer
                .formats
                .iter()
                .enumerate()
                .map(|(index, &(format, modifier, _))| Event::GpuFormat {
                    index: index as u32,
                    format,
                    modifier,
                }),
        );
        events
    }

    /// Whether `buffer` of `client` is a GPU buffer (made or failed).
    pub fn has(&self, client: ClientId, buffer: u64) -> bool {
        self.buffers.contains_key(&(client, buffer)) || self.failed.contains(&(client, buffer))
    }

    /// A frame of `buffer` as the compositor composes it: its size and format, the pixels the CPU
    /// reads of it (with `pixels`, through its mapping), and its import (with `image`).
    pub fn read(
        &self,
        client: ClientId,
        buffer: u64,
        pixels: bool,
    ) -> Option<(Size, Format, Vec<u8>, Option<GpuFrame>)> {
        let found = self.buffers.get(&(client, buffer))?;
        let row = found.extent.width as usize * 4;
        let bytes = match (&found.mapping, pixels || found.image.is_none()) {
            (Some(mapping), true) => {
                let mut out = vec![0; row * found.extent.height as usize];
                let plane = &found.planes[0];
                mapping
                    .read_rows(
                        plane.offset,
                        plane.stride,
                        row,
                        0..found.extent.height,
                        &mut out,
                    )
                    .ok()?;
                out
            }
            (None, true) if found.image.is_none() => return None,
            _ => Vec::new(),
        };
        let frame = found.image.map(|image| GpuFrame {
            image,
            opaque: found.format == Format::Xrgb8888,
            _uses: found.uses.clone(),
        });
        Some((found.extent, found.format, bytes, frame))
    }

    /// Everything of a connection that went away. Its imports go with its buffers once nothing
    /// draws them.
    fn disconnect(&mut self, client: ClientId, gpu: &mut Option<super::Gpu>) {
        self.planes.retain(|(owner, _), _| *owner != client);
        self.failed.retain(|(owner, _)| *owner != client);
        self.timelines.retain(|(owner, _), _| *owner != client);
        self.points.retain(|key, _| key.client != client);
        self.waiting.retain(|key, _| key.client != client);
        self.releases.retain(|(owner, _), _| *owner != client);
        self.after_acquire.retain(|after| after.client != client);
        self.released.retain(|(owner, _)| *owner != client);
        let gone: Vec<(ClientId, u64)> = self
            .buffers
            .keys()
            .filter(|(owner, _)| *owner == client)
            .copied()
            .collect();
        for key in gone {
            let buffer = self.buffers.remove(&key).expect("listed");
            self.orphans
                .push((buffer.image, buffer.uses, buffer.serial));
        }
        self.collect_orphans(gpu);
    }

    fn collect_orphans(&mut self, gpu: &mut Option<super::Gpu>) {
        let gone = &mut self.gone;
        self.orphans.retain(|(image, uses, serial)| {
            if Rc::strong_count(uses) > 1 {
                return true;
            }
            if let (Some(image), Some(gpu)) = (image, gpu.as_mut()) {
                gpu.compositor.forget_dma_buf(*image);
            }
            gone.push(*serial);
            false
        });
    }

    /// What a display needs to scan out a frame of `buffer` directly (decision 8).
    pub(crate) fn scanout_source(
        &self,
        client: ClientId,
        buffer: u64,
    ) -> Option<ScanoutSource<'_>> {
        let found = self.buffers.get(&(client, buffer))?;
        Some(ScanoutSource {
            serial: found.serial,
            format: found.format,
            modifier: found.modifier,
            extent: found.extent,
            image: found.image,
            planes: found
                .planes
                .iter()
                .map(|plane| (plane.buffer.as_fd(), plane.offset, plane.stride))
                .collect(),
            uses: found.uses.clone(),
        })
    }

    /// The descriptors that wake a turn when a point the compositor waits for signals.
    pub fn wake_fds(&self) -> Vec<BorrowedFd<'_>> {
        self.waiting
            .values()
            .map(|waiting| waiting.signalled.as_fd())
            .chain(
                self.after_acquire
                    .iter()
                    .map(|after| after.signalled.as_fd()),
            )
            .collect()
    }
}

fn readable(fd: BorrowedFd<'_>) -> bool {
    oxyos_dmabuf::sync_file_signalled(fd)
}

impl Compositor {
    /// Offers clients' GPU buffers (ADR 0031), where a render node has timeline syncobjs: in the
    /// formats and modifiers the Vulkan compositor imports, or, composing on the CPU (or with
    /// `--gpu`'s check, which composes on both), in linear buffers it maps. Says why not
    /// otherwise.
    pub fn offer_gpu_buffers(&mut self) {
        let vulkan = self.gpu.as_ref().map(|gpu| &gpu.compositor);
        let device = vulkan.and_then(|vulkan| vulkan.render_node());
        let Some(node) = RenderNode::with_timelines(device) else {
            eprintln!("[compositor] clients' GPU buffers: no render node with timeline syncobjs, shared memory only");
            return;
        };
        let formats: Vec<(Format, u64, u32)> = match vulkan {
            Some(vulkan) => vulkan
                .dma_buf_formats()
                .iter()
                .filter_map(|format| {
                    let fourcc = match format.fourcc {
                        0x3432_5241 => Format::Argb8888,
                        0x3432_5258 => Format::Xrgb8888,
                        _ => return None,
                    };
                    // The CPU reads what it composes too, unless the GPU composes alone: linear.
                    (self.gpu_only || format.modifier == MODIFIER_LINEAR).then_some((
                        fourcc,
                        format.modifier,
                        format.planes,
                    ))
                })
                .collect(),
            None => vec![
                (Format::Argb8888, MODIFIER_LINEAR, 1),
                (Format::Xrgb8888, MODIFIER_LINEAR, 1),
            ],
        };
        if formats.is_empty() {
            eprintln!(
                "[compositor] clients' GPU buffers: the device imports none, shared memory only"
            );
            return;
        }
        let composing = if vulkan.is_some() {
            device.unwrap_or(0)
        } else {
            0
        };
        eprintln!(
            "[compositor] clients' GPU buffers: {} formats and modifiers, timelines on {:#x}, composing on {}",
            formats.len(),
            node.dev(),
            vulkan.map_or("the CPU".to_string(), |vulkan| vulkan.device_name().to_string())
        );
        self.gpu_buffers.offer = Some(Offer {
            node: Arc::new(node),
            device: composing,
            formats,
        });
    }

    /// Stops offering GPU buffers, closing the render node (the session ends; with clients still
    /// connected, each is told the offer is withdrawn).
    pub fn withdraw_gpu_buffers(&mut self) {
        if self.gpu_buffers.offer.take().is_none() {
            return;
        }
        let clients: Vec<ClientId> = self.peers.keys().copied().collect();
        for client in clients {
            self.send_to(
                client,
                Event::GpuDevice {
                    device: 0,
                    formats: 0,
                },
            );
        }
    }

    /// A connection's frames of GPU buffers given to the scene so far, and how many of them
    /// waited for their acquire point.
    pub fn gpu_frames(&self, client: ClientId) -> (u64, u64) {
        self.gpu_buffers
            .frames
            .get(&client)
            .copied()
            .unwrap_or_default()
    }

    /// Whether clients are offered GPU buffers.
    pub fn offers_gpu_buffers(&self) -> bool {
        self.gpu_buffers.offered()
    }

    /// Ends a connection's GPU buffers and timelines.
    pub(crate) fn disconnect_gpu(&mut self, client: ClientId) {
        self.gpu_buffers.disconnect(client, &mut self.gpu);
    }

    /// Handles a request of ADR 0031's, or a `Submit` or `DestroyBuffer` naming a GPU buffer;
    /// `None` for anything else, which the caller handles as before.
    pub(crate) fn receive_gpu(
        &mut self,
        client: ClientId,
        request: Request,
        fd: Option<OwnedFd>,
    ) -> Option<Result<(), ErrorCode>> {
        let key = |surface| SurfaceKey {
            client,
            id: surface,
        };
        match request {
            Request::AttachDmabufPlane {
                buffer,
                plane,
                offset,
                stride,
            } => Some(self.attach_plane(client, buffer, plane, offset, stride, fd?)),
            Request::AttachDmabuf {
                buffer,
                format,
                modifier,
                extent,
                planes,
            } => {
                Some(self.attach_dmabuf(client, request, buffer, format, modifier, extent, planes))
            }
            Request::ImportTimeline { timeline } => {
                Some(self.import_timeline(client, timeline, fd?))
            }
            Request::DestroyTimeline { timeline } => Some(
                match self.gpu_buffers.timelines.remove(&(client, timeline)) {
                    Some(_) => Ok(()),
                    None => Err(ErrorCode::InvalidMessage),
                },
            ),
            Request::SetSync {
                surface,
                acquire_timeline,
                acquire_point,
                release_timeline,
                release_point,
            } => {
                let known = |timeline: u32| {
                    timeline == 0 || self.gpu_buffers.timelines.contains_key(&(client, timeline))
                };
                if !known(acquire_timeline)
                    || !known(release_timeline)
                    || (acquire_timeline != 0 && acquire_point == 0)
                    || (release_timeline != 0 && release_point == 0)
                    || (acquire_timeline != 0
                        && acquire_timeline == release_timeline
                        && release_point <= acquire_point)
                {
                    return Some(Err(ErrorCode::InvalidMessage));
                }
                self.gpu_buffers.points.insert(
                    key(surface),
                    Points {
                        acquire: (acquire_timeline, acquire_point),
                        release: (release_timeline, release_point),
                    },
                );
                Some(Ok(()))
            }
            Request::Submit {
                surface,
                sequence,
                buffer,
                ..
            } => {
                let points = self.gpu_buffers.points.remove(&key(surface));
                // Any frame supersedes one still waiting for its acquire point.
                if let Some(waiting) = self.gpu_buffers.waiting.remove(&key(surface)) {
                    if waiting.buffer == buffer {
                        return Some(Err(ErrorCode::InvalidBuffer));
                    }
                    self.supersede(key(surface), waiting);
                }
                // A buffer waiting on another surface is in use, as one shown is.
                if self
                    .gpu_buffers
                    .waiting
                    .iter()
                    .any(|(held, waiting)| held.client == client && waiting.buffer == buffer)
                {
                    return Some(Err(ErrorCode::InvalidBuffer));
                }
                if self.gpu_buffers.failed.contains(&(client, buffer)) {
                    // A buffer the compositor could not import is never shown (decision 3).
                    self.send_to(
                        client,
                        Event::Feedback {
                            surface,
                            sequence,
                            outcome: Outcome::Discarded,
                            timestamp_ns: 0,
                            clock: Clock::Synthetic,
                        },
                    );
                    self.send_to(client, Event::Released { surface, buffer });
                    if let Some(points) = points {
                        self.release_point_now(client, points.release);
                    }
                    return Some(Ok(()));
                }
                if !self.gpu_buffers.buffers.contains_key(&(client, buffer)) {
                    // Shared memory: the scene's; points name GPU buffers only.
                    return points
                        .filter(|points| points.acquire.0 != 0 || points.release.0 != 0)
                        .map(|_| Err(ErrorCode::InvalidBuffer));
                }
                let points = points.unwrap_or_default();
                if points.release.0 != 0 {
                    self.gpu_buffers
                        .releases
                        .insert((client, buffer), points.release);
                }
                if points.acquire.0 != 0 {
                    // Destroyed since SetSync named it.
                    let Some(timeline) =
                        self.gpu_buffers.timelines.get(&(client, points.acquire.0))
                    else {
                        return Some(Err(ErrorCode::InvalidMessage));
                    };
                    let signalled = timeline
                        .signalled()
                        .is_ok_and(|latest| latest >= points.acquire.1);
                    if !signalled {
                        let Ok(eventfd) = timeline.eventfd(points.acquire.1, false) else {
                            return Some(Err(ErrorCode::InvalidMessage));
                        };
                        self.gpu_buffers.waiting.insert(
                            key(surface),
                            Waiting {
                                request,
                                signalled: eventfd,
                                buffer,
                            },
                        );
                        return Some(Ok(()));
                    }
                }
                // Ready: the scene's, as any frame.
                self.gpu_buffers.frames.entry(client).or_default().0 += 1;
                None
            }
            Request::DestroyBuffer { buffer }
                if self.gpu_buffers.failed.remove(&(client, buffer)) =>
            {
                Some(Ok(()))
            }
            // A buffer destroyed while a frame of it waits: the frame goes, and the scene ends
            // the buffer as it ends any.
            Request::DestroyBuffer { buffer } => {
                let held: Vec<SurfaceKey> = self
                    .gpu_buffers
                    .waiting
                    .iter()
                    .filter(|(held, waiting)| held.client == client && waiting.buffer == buffer)
                    .map(|(held, _)| *held)
                    .collect();
                for held in held {
                    let waiting = self.gpu_buffers.waiting.remove(&held).expect("listed");
                    self.supersede(held, waiting);
                }
                None
            }
            // A surface destroyed while a frame of it waits: the frame goes with it.
            Request::Destroy { surface } => {
                self.gpu_buffers.points.remove(&key(surface));
                if let Some(waiting) = self.gpu_buffers.waiting.remove(&key(surface)) {
                    self.supersede(key(surface), waiting);
                }
                None
            }
            _ => None,
        }
    }

    fn attach_plane(
        &mut self,
        client: ClientId,
        buffer: u64,
        plane: u8,
        offset: u32,
        stride: u32,
        fd: OwnedFd,
    ) -> Result<(), ErrorCode> {
        if self.gpu_buffers.offer.is_none() {
            return Err(ErrorCode::InvalidBuffer);
        }
        let dma_buf = DmaBuf::from_fd(fd).map_err(|_| ErrorCode::InvalidBuffer)?;
        if buffer == 0
            || usize::from(plane) >= MAX_PLANES
            || self.gpu_buffers.has(client, buffer)
            || self.scene_has_buffer(client, buffer)
        {
            return Err(ErrorCode::InvalidBuffer);
        }
        let pending = self
            .gpu_buffers
            .planes
            .iter()
            .filter(|((owner, _), _)| *owner == client)
            .count();
        let planes = self
            .gpu_buffers
            .planes
            .entry((client, buffer))
            .or_insert_with(|| (0..MAX_PLANES).map(|_| None).collect());
        if planes[usize::from(plane)].is_some() || pending > MAX_GPU_BUFFERS {
            return Err(ErrorCode::InvalidBuffer);
        }
        planes[usize::from(plane)] = Some(Plane {
            buffer: dma_buf,
            offset,
            stride,
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn attach_dmabuf(
        &mut self,
        client: ClientId,
        request: Request,
        buffer: u64,
        format: Format,
        modifier: u64,
        extent: Size,
        count: u8,
    ) -> Result<(), ErrorCode> {
        let Some(offer) = &self.gpu_buffers.offer else {
            return Err(ErrorCode::InvalidBuffer);
        };
        let planes = self
            .gpu_buffers
            .planes
            .remove(&(client, buffer))
            .ok_or(ErrorCode::InvalidBuffer)?;
        // In the table sent (decision 2), with the modifier's planes, each attached once.
        let (_, _, wanted) = *offer
            .formats
            .iter()
            .find(|&&(f, m, _)| f == format && m == modifier)
            .ok_or(ErrorCode::InvalidBuffer)?;
        let count = usize::from(count);
        if count != wanted as usize
            || planes[..count].iter().any(Option::is_none)
            || planes[count..].iter().any(Option::is_some)
        {
            return Err(ErrorCode::InvalidBuffer);
        }
        let planes: Vec<Plane> = planes.into_iter().flatten().collect();
        let row = u64::from(extent.width) * u64::from(format.bytes_per_pixel());
        if modifier == MODIFIER_LINEAR
            && !planes.iter().all(|plane| {
                oxyos_dmabuf::linear_plane_fits(
                    plane.buffer.size(),
                    plane.offset,
                    plane.stride,
                    row,
                    extent.height,
                )
            })
        {
            return Err(ErrorCode::InvalidBuffer);
        }
        // Quotas: buffers and their memory (each descriptor counted once per buffer).
        let mine = || {
            self.gpu_buffers
                .buffers
                .iter()
                .filter(|((owner, _), _)| *owner == client)
        };
        let bytes = planes
            .iter()
            .map(|plane| plane.buffer.size())
            .max()
            .unwrap_or(0);
        if mine().count() >= MAX_GPU_BUFFERS
            || mine().map(|(_, buffer)| buffer.bytes).sum::<u64>() + bytes > MAX_GPU_BYTES
        {
            return Err(ErrorCode::QuotaExceeded);
        }
        // Imported, mapped, or both (`--gpu`'s check composes on the CPU too).
        let image = match &mut self.gpu {
            Some(gpu) => {
                let described: Vec<oxyos_gpu::DmaBufPlane<'_>> = planes
                    .iter()
                    .map(|plane| oxyos_gpu::DmaBufPlane {
                        fd: plane.buffer.as_fd(),
                        offset: plane.offset,
                        stride: plane.stride,
                    })
                    .collect();
                match gpu.compositor.import_dma_buf(
                    format as u32,
                    modifier,
                    extent.width,
                    extent.height,
                    &described,
                ) {
                    Ok(image) => Some(image),
                    Err(error) => {
                        eprintln!("[compositor] a GPU buffer could not be imported: {error}");
                        return self.failed_buffer(client, buffer);
                    }
                }
            }
            None => None,
        };
        let mapping = if modifier == MODIFIER_LINEAR && !self.gpu_only {
            match planes[0].buffer.try_clone().and_then(Mapping::new) {
                Ok(mapping) => Some(mapping),
                Err(error) => {
                    eprintln!("[compositor] a GPU buffer could not be mapped: {error}");
                    if let (Some(image), Some(gpu)) = (image, &mut self.gpu) {
                        gpu.compositor.forget_dma_buf(image);
                    }
                    return self.failed_buffer(client, buffer);
                }
            }
        } else {
            None
        };
        if let Err(code) = self.scene.handle(client, request) {
            if let (Some(image), Some(gpu)) = (image, &mut self.gpu) {
                gpu.compositor.forget_dma_buf(image);
            }
            return Err(code);
        }
        self.gpu_buffers.buffers.insert(
            (client, buffer),
            Buffer {
                extent,
                format,
                planes,
                mapping,
                image,
                bytes,
                modifier,
                serial: {
                    self.gpu_buffers.next_serial += 1;
                    self.gpu_buffers.next_serial
                },
                uses: Rc::new(()),
            },
        );
        Ok(())
    }

    /// A well-formed buffer that could not be read: the client hears `BufferFailed`.
    fn failed_buffer(&mut self, client: ClientId, buffer: u64) -> Result<(), ErrorCode> {
        self.gpu_buffers.failed.insert((client, buffer));
        self.send_to(client, Event::BufferFailed { buffer });
        Ok(())
    }

    fn import_timeline(
        &mut self,
        client: ClientId,
        timeline: u32,
        fd: OwnedFd,
    ) -> Result<(), ErrorCode> {
        let Some(offer) = &self.gpu_buffers.offer else {
            return Err(ErrorCode::InvalidMessage);
        };
        let mine = self
            .gpu_buffers
            .timelines
            .keys()
            .filter(|(owner, _)| *owner == client)
            .count();
        if timeline == 0 || self.gpu_buffers.timelines.contains_key(&(client, timeline)) {
            return Err(ErrorCode::InvalidMessage);
        }
        if mine >= MAX_TIMELINES {
            return Err(ErrorCode::QuotaExceeded);
        }
        // Only a syncobj imports.
        let imported =
            Timeline::import(&offer.node, fd.as_fd()).map_err(|_| ErrorCode::InvalidMessage)?;
        self.gpu_buffers
            .timelines
            .insert((client, timeline), imported);
        Ok(())
    }

    fn scene_has_buffer(&self, client: ClientId, buffer: u64) -> bool {
        self.peers
            .get(&client)
            .is_some_and(|peer| peer.buffers.contains_key(&buffer))
    }

    /// A frame waiting for its acquire point, superseded: discarded, its buffer released, its
    /// release point signalled once its acquire point has (the compositor never read it).
    fn supersede(&mut self, key: SurfaceKey, waiting: Waiting) {
        let Request::Submit {
            surface, sequence, ..
        } = waiting.request
        else {
            return;
        };
        self.send_to(
            key.client,
            Event::Feedback {
                surface,
                sequence,
                outcome: Outcome::Discarded,
                timestamp_ns: 0,
                clock: Clock::Synthetic,
            },
        );
        self.send_to(
            key.client,
            Event::Released {
                surface,
                buffer: waiting.buffer,
            },
        );
        if let Some(release) = self
            .gpu_buffers
            .releases
            .remove(&(key.client, waiting.buffer))
        {
            self.gpu_buffers.after_acquire.push(AfterAcquire {
                signalled: waiting.signalled,
                client: key.client,
                release,
            });
        }
    }

    /// Signals a release point from the CPU now.
    fn release_point_now(&mut self, client: ClientId, (timeline, point): (u32, u64)) {
        if timeline == 0 {
            return;
        }
        if let Some(timeline) = self.gpu_buffers.timelines.get(&(client, timeline)) {
            if let Err(error) = timeline.signal(point) {
                eprintln!("[compositor] a release point: {error}");
            }
        }
    }

    /// An event to a client's outbox.
    pub(crate) fn send_to(&mut self, client: ClientId, event: Event) {
        if let Some(peer) = self.peers.get_mut(&client) {
            peer.outbox.push_back((event, None));
        }
    }

    /// Whether `Released` for this GPU buffer may go now, signalling its release point (decision
    /// 6): once no transition draws it and, without a sync file of the frame in flight, once
    /// that frame no longer reads it. False: held, and tried again each turn.
    fn release_gpu_buffer(&mut self, client: ClientId, buffer: u64) -> bool {
        let Some(found) = self.gpu_buffers.buffers.get(&(client, buffer)) else {
            return true;
        };
        if Rc::strong_count(&found.uses) > 1 {
            return false;
        }
        let reading = match (found.image, &self.gpu) {
            (Some(image), Some(gpu)) => gpu.compositor.reads_dma_buf(image),
            _ => false,
        };
        let release = self.gpu_buffers.releases.get(&(client, buffer)).copied();
        if reading {
            let fence = self
                .gpu
                .as_ref()
                .and_then(|gpu| gpu.compositor.completion_sync_file());
            match (release, fence) {
                // The point signals when the composition finishes, with no CPU between.
                (Some((timeline, point)), Some(fence)) => {
                    if let Some(timeline) = self.gpu_buffers.timelines.get(&(client, timeline)) {
                        if let Err(error) = timeline.import_sync_file(point, fence.as_fd()) {
                            eprintln!("[compositor] a release point from the GPU: {error}");
                            return false;
                        }
                    }
                }
                // Without a point, or without the composition's sync file: once it is done.
                _ => return false,
            }
        } else if let Some(release) = release {
            self.release_point_now(client, release);
        }
        self.gpu_buffers.releases.remove(&(client, buffer));
        true
    }

    /// Routes a scene event about a GPU buffer: a `Released` waits until it may go. True when the
    /// event was taken here.
    pub(crate) fn hold_gpu_event(&mut self, client: ClientId, event: &Event) -> bool {
        let Event::Released { buffer, .. } = *event else {
            return false;
        };
        if !self.gpu_buffers.buffers.contains_key(&(client, buffer)) {
            return false;
        }
        if self.release_gpu_buffer(client, buffer) {
            return false;
        }
        self.gpu_buffers.released.push((client, *event));
        true
    }

    /// The scene retired a buffer (destroyed and released): a GPU buffer's import goes once no
    /// transition draws it.
    pub(crate) fn retire_gpu_buffer(&mut self, client: ClientId, buffer: u64) {
        if let Some(found) = self.gpu_buffers.buffers.remove(&(client, buffer)) {
            self.gpu_buffers
                .orphans
                .push((found.image, found.uses, found.serial));
            self.gpu_buffers.collect_orphans(&mut self.gpu);
        }
    }

    /// Each turn: frames whose acquire point signalled go to the scene, release points of frames
    /// superseded before they were read are signalled after their acquire points, and releases
    /// held for the GPU or a transition go once they may.
    pub fn follow_gpu_sync(&mut self) {
        if self.gpu_buffers.offer.is_none() {
            return;
        }
        let ready: Vec<SurfaceKey> = self
            .gpu_buffers
            .waiting
            .iter()
            .filter(|(_, waiting)| readable(waiting.signalled.as_fd()))
            .map(|(key, _)| *key)
            .collect();
        for key in ready {
            let waiting = self.gpu_buffers.waiting.remove(&key).expect("listed");
            let frames = self.gpu_buffers.frames.entry(key.client).or_default();
            frames.0 += 1;
            frames.1 += 1;
            self.submit_to_scene(key.client, waiting.request);
        }
        let signalled: Vec<AfterAcquire> = {
            let (done, waiting): (Vec<_>, Vec<_>) =
                std::mem::take(&mut self.gpu_buffers.after_acquire)
                    .into_iter()
                    .partition(|after| readable(after.signalled.as_fd()));
            self.gpu_buffers.after_acquire = waiting;
            done
        };
        for after in signalled {
            self.release_point_now(after.client, after.release);
        }
        if let Some(gpu) = &mut self.gpu {
            if let Err(error) = gpu.compositor.poll_completion() {
                eprintln!("[gpu] {error}");
            }
        }
        let held = std::mem::take(&mut self.gpu_buffers.released);
        for (client, event) in held {
            let Event::Released { buffer, .. } = event else {
                continue;
            };
            if self.release_gpu_buffer(client, buffer) {
                self.send_to(client, event);
            } else {
                self.gpu_buffers.released.push((client, event));
            }
        }
        self.gpu_buffers.collect_orphans(&mut self.gpu);
    }

    /// Whether anything waits on the GPU or a point: the turn must come back for it.
    pub fn gpu_sync_pending(&self) -> bool {
        !self.gpu_buffers.released.is_empty()
            || !self.gpu_buffers.orphans.is_empty()
            || !self.gpu_buffers.waiting.is_empty()
            || !self.gpu_buffers.after_acquire.is_empty()
    }
}
