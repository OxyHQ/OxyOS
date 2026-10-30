//! GPU buffers (docs/adr/0031-gpu-buffers.mdx): the compositor's tables of what it imports and
//! scans out, DMA-BUFs attached plane by plane, DRM syncobj timelines and frames submitted with
//! their acquire and release points. Nothing here allocates or draws: the caller's GPU does.

use oxyos_dmabuf::{DmaBuf, RenderNode, Timeline};
use oxyos_protocol::{Event, Format, Rect, Request, Size};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;

use crate::Client;

/// What the compositor composes from (decision 2): its render node (a `dev_t`; 0 when its device
/// has none) and each DRM format and modifier it imports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuTable {
    pub device: u64,
    pub formats: Vec<(Format, u64)>,
}

impl GpuTable {
    pub fn has(&self, format: Format, modifier: u64) -> bool {
        self.formats.contains(&(format, modifier))
    }
}

/// A frame's synchronization (decision 6): the acquire point the compositor waits for before it
/// reads the buffer and the release point it signals once it no longer does, each a timeline of
/// this connection and a point on it. Timeline 0 is none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncPoints {
    pub acquire: (u32, u64),
    pub release: (u32, u64),
}

/// A table or a surface's scanout formats that just became complete, or were withdrawn: what a
/// caller that reads events one by one hears about the tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuChange {
    /// The GPU table is complete with this many formats, or withdrawn (0).
    Device(u32),
    /// A surface's scanout formats are complete with this many, or there are none (0).
    Scanout(u32, u32),
}

/// A table being told: how many entries it has and those that arrived.
#[derive(Debug, Default)]
struct Partial<T> {
    entries: Vec<Option<(Format, u64)>>,
    head: T,
}

impl<T> Partial<T> {
    fn new(count: u32, head: T) -> Self {
        Self {
            entries: vec![None; count.min(4096) as usize],
            head,
        }
    }

    /// Records entry `index`; all of them once every one arrived.
    fn add(&mut self, index: u32, format: Format, modifier: u64) -> Option<Vec<(Format, u64)>> {
        *self.entries.get_mut(index as usize)? = Some((format, modifier));
        self.entries.iter().copied().collect()
    }
}

/// The connection's GPU state.
#[derive(Debug, Default)]
pub(crate) struct Gpu {
    table: Option<GpuTable>,
    told: Option<Partial<u64>>,
    scanout: HashMap<u32, Vec<(Format, u64)>>,
    scanout_told: HashMap<u32, Partial<()>>,
    failed: HashSet<u64>,
    /// The render node timelines are made and bridged on, opened when first needed.
    node: Option<Arc<RenderNode>>,
    timelines: HashMap<u32, Timeline>,
    next_timeline: u32,
    change: Option<GpuChange>,
}

impl Gpu {
    /// Follows the compositor's GPU events.
    pub(crate) fn observe(&mut self, event: &Event) {
        match *event {
            Event::GpuDevice { device, formats } => {
                self.told = None;
                if formats == 0 {
                    self.table = None;
                    self.change = Some(GpuChange::Device(0));
                } else {
                    self.told = Some(Partial::new(formats, device));
                }
            }
            Event::GpuFormat {
                index,
                format,
                modifier,
            } => {
                let complete = self
                    .told
                    .as_mut()
                    .and_then(|told| told.add(index, format, modifier));
                if let Some(formats) = complete {
                    let device = self.told.take().map(|told| told.head).unwrap_or_default();
                    self.change = Some(GpuChange::Device(formats.len() as u32));
                    self.table = Some(GpuTable { device, formats });
                }
            }
            Event::GpuScanout { surface, formats } => {
                self.scanout_told.remove(&surface);
                if formats == 0 {
                    self.scanout.remove(&surface);
                    self.change = Some(GpuChange::Scanout(surface, 0));
                } else {
                    self.scanout_told.insert(surface, Partial::new(formats, ()));
                }
            }
            Event::GpuScanoutFormat {
                surface,
                index,
                format,
                modifier,
            } => {
                let complete = self
                    .scanout_told
                    .get_mut(&surface)
                    .and_then(|told| told.add(index, format, modifier));
                if let Some(formats) = complete {
                    self.scanout_told.remove(&surface);
                    self.change = Some(GpuChange::Scanout(surface, formats.len() as u32));
                    self.scanout.insert(surface, formats);
                }
            }
            Event::BufferFailed { buffer } => {
                self.failed.insert(buffer);
            }
            _ => {}
        }
    }

    fn node(&mut self) -> io::Result<Arc<RenderNode>> {
        if let Some(node) = &self.node {
            return Ok(node.clone());
        }
        let device = self.table.as_ref().map(|table| table.device);
        let node = RenderNode::with_timelines(device.filter(|&device| device != 0))
            .map(Arc::new)
            .ok_or_else(|| io::Error::other("no render node with timeline syncobjs"))?;
        self.node = Some(node.clone());
        Ok(node)
    }

    fn timeline(&self, id: u32) -> io::Result<&Timeline> {
        self.timelines
            .get(&id)
            .ok_or_else(|| invalid("no such timeline"))
    }
}

fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason.to_string())
}

/// Most planes of a DMA-BUF (the protocol's, DRM's).
pub const MAX_PLANES: usize = 4;

impl Client {
    /// The compositor's GPU table once it is complete; `None`: shared memory only.
    pub fn gpu(&self) -> Option<&GpuTable> {
        self.gpu.table.as_ref()
    }

    /// The formats and modifiers `surface`'s buffers would be scanned out in, while it is all its
    /// output shows (decision 8); empty otherwise.
    pub fn scanout_formats(&self, surface: u32) -> &[(Format, u64)] {
        self.gpu.scanout.get(&surface).map_or(&[], Vec::as_slice)
    }

    /// The GPU table or a surface's scanout formats completed or withdrawn by the event last
    /// returned, once.
    pub fn take_gpu_change(&mut self) -> Option<GpuChange> {
        self.gpu.change.take()
    }

    /// Whether the compositor could not import buffer `buffer` (`BufferFailed`).
    pub fn buffer_failed(&self, buffer: u64) -> bool {
        self.gpu.failed.contains(&buffer)
    }

    /// Makes a DMA-BUF the caller owns a buffer of this connection: `planes` (1 to 4, each a
    /// descriptor with its offset and stride) in `format` with `modifier`, of `extent` pixels.
    /// Checked as the compositor checks it (each descriptor a DMA-BUF, the format and modifier in
    /// its table, a linear plane within its descriptor) before anything is sent; then each
    /// plane is sent with its descriptor and the buffer after them. The caller keeps the
    /// descriptors. Returns the buffer's id: submitted as a shared buffer is
    /// ([`Client::submit_buffer`], [`Client::submit_buffer_synced`]) and ended by
    /// [`Client::detach_buffer`]. A `BufferFailed` naming it later means the compositor could not
    /// import it ([`Client::buffer_failed`]).
    pub fn attach_dmabuf(
        &mut self,
        format: Format,
        modifier: u64,
        extent: Size,
        planes: &[(BorrowedFd<'_>, u32, u32)],
    ) -> io::Result<u64> {
        if planes.is_empty() || planes.len() > MAX_PLANES {
            return Err(invalid("a DMA-BUF has 1 to 4 planes"));
        }
        if !extent.is_valid() {
            return Err(invalid("invalid extent"));
        }
        let table = self
            .gpu
            .table
            .as_ref()
            .ok_or_else(|| invalid("the compositor takes no GPU buffers"))?;
        if !table.has(format, modifier) {
            return Err(invalid(
                "the compositor does not import this format and modifier",
            ));
        }
        let row = u64::from(extent.width) * u64::from(format.bytes_per_pixel());
        for &(fd, offset, stride) in planes {
            let buffer = DmaBuf::from_fd(fd.try_clone_to_owned()?)
                .map_err(|error| invalid(&format!("a plane is not a DMA-BUF: {error:?}")))?;
            if offset % 4 != 0 || stride % 4 != 0 {
                return Err(invalid("a plane's offset and stride are aligned to 4"));
            }
            if modifier == oxyos_dmabuf::MODIFIER_LINEAR
                && !oxyos_dmabuf::linear_plane_fits(
                    buffer.size(),
                    offset,
                    stride,
                    row,
                    extent.height,
                )
            {
                return Err(invalid("a linear plane lies beyond its DMA-BUF"));
            }
        }
        if modifier == oxyos_dmabuf::MODIFIER_LINEAR && planes.len() != 1 {
            return Err(invalid("a linear RGB buffer has one plane"));
        }
        self.next_buffer += 1;
        let buffer = self.next_buffer;
        for (plane, &(fd, offset, stride)) in planes.iter().enumerate() {
            self.connection.send_request(
                &Request::AttachDmabufPlane {
                    buffer,
                    plane: plane as u8,
                    offset,
                    stride,
                },
                Some(fd),
            )?;
        }
        self.send(&Request::AttachDmabuf {
            buffer,
            format,
            modifier,
            extent,
            planes: planes.len() as u8,
        })?;
        self.attached.push((buffer, extent));
        Ok(buffer)
    }

    /// A new timeline of this connection (a DRM syncobj on the compositor's render node, or the
    /// first with timelines), imported by the compositor. Its points are signalled and waited for
    /// through the calls below, or by the caller's GPU through [`Client::timeline_export`].
    pub fn create_timeline(&mut self) -> io::Result<u32> {
        let node = self.gpu.node()?;
        let timeline = Timeline::create(&node)?;
        self.add_timeline(timeline)
    }

    /// The caller's own syncobj timeline (`fd`, as `DRM_IOCTL_SYNCOBJ_HANDLE_TO_FD` or a Vulkan
    /// timeline semaphore's `OPAQUE_FD` export gives it) as a timeline of this connection. The
    /// caller keeps `fd`.
    pub fn import_timeline(&mut self, fd: BorrowedFd<'_>) -> io::Result<u32> {
        let node = self.gpu.node()?;
        let timeline = Timeline::import(&node, fd)?;
        self.add_timeline(timeline)
    }

    fn add_timeline(&mut self, timeline: Timeline) -> io::Result<u32> {
        let exported = timeline.export()?;
        self.gpu.next_timeline += 1;
        let id = self.gpu.next_timeline;
        self.connection.send_request(
            &Request::ImportTimeline { timeline: id },
            Some(exported.as_fd()),
        )?;
        self.gpu.timelines.insert(id, timeline);
        Ok(id)
    }

    /// Ends a timeline; the compositor forgets it too.
    pub fn destroy_timeline(&mut self, id: u32) -> io::Result<()> {
        self.gpu
            .timelines
            .remove(&id)
            .ok_or_else(|| invalid("no such timeline"))?;
        self.send(&Request::DestroyTimeline { timeline: id })
    }

    /// `point` of timeline `id` signals when the sync file `fence` does (a GL native fence, a
    /// Vulkan binary semaphore's `SYNC_FD`). The caller keeps `fence`.
    pub fn timeline_import_sync_file(
        &self,
        id: u32,
        point: u64,
        fence: BorrowedFd<'_>,
    ) -> io::Result<()> {
        self.gpu.timeline(id)?.import_sync_file(point, fence)
    }

    /// The fence of `point` of timeline `id` as a new sync file, for a GPU that waits on sync
    /// files. The point must have a fence: a release point does once the compositor's `Released`
    /// names the buffer.
    pub fn timeline_export_sync_file(&self, id: u32, point: u64) -> io::Result<OwnedFd> {
        self.gpu.timeline(id)?.export_sync_file(point)
    }

    /// Signals `point` of timeline `id` from the CPU (a buffer written by the CPU, or work the
    /// caller waited for).
    pub fn timeline_signal(&self, id: u32, point: u64) -> io::Result<()> {
        self.gpu.timeline(id)?.signal(point)
    }

    /// A new descriptor of timeline `id`'s syncobj, for a Vulkan timeline semaphore
    /// (`VK_EXTERNAL_SEMAPHORE_HANDLE_TYPE_OPAQUE_FD_BIT`) or another process.
    pub fn timeline_export(&self, id: u32) -> io::Result<OwnedFd> {
        self.gpu.timeline(id)?.export()
    }

    /// An eventfd readable once `point` of timeline `id` signals, or, `available`, once it has a
    /// fence: a release point watched in the caller's event loop.
    pub fn timeline_eventfd(&self, id: u32, point: u64, available: bool) -> io::Result<OwnedFd> {
        self.gpu.timeline(id)?.eventfd(point, available)
    }

    /// The latest point of timeline `id` that signalled.
    pub fn timeline_signalled(&self, id: u32) -> io::Result<u64> {
        self.gpu.timeline(id)?.signalled()
    }

    /// [`Client::submit_buffer`] with the frame's synchronization: `sync`'s acquire point, which
    /// the compositor waits for before it reads the buffer, and its release point, which it
    /// signals once it no longer does (`SetSync`, sent after the damage and before the
    /// `Submit`). Refused before anything is sent for a timeline this connection does not have,
    /// or a release point not after the acquire point on the same timeline.
    pub fn submit_buffer_synced(
        &mut self,
        surface: u32,
        buffer: u64,
        configure: u64,
        damage: Option<&[Rect]>,
        sync: Option<SyncPoints>,
    ) -> io::Result<u64> {
        if let Some(sync) = sync {
            for (timeline, point) in [sync.acquire, sync.release] {
                if timeline != 0 && !self.gpu.timelines.contains_key(&timeline) {
                    return Err(invalid("no such timeline"));
                }
                if timeline != 0 && point == 0 {
                    return Err(invalid("points start at 1"));
                }
            }
            if sync.acquire.0 != 0
                && sync.acquire.0 == sync.release.0
                && sync.release.1 <= sync.acquire.1
            {
                return Err(invalid("the release point must follow the acquire point"));
            }
        }
        let presented = self.queue_damage(surface, buffer, damage)?;
        if let Some(sync) = sync {
            self.send(&Request::SetSync {
                surface,
                acquire_timeline: sync.acquire.0,
                acquire_point: sync.acquire.1,
                release_timeline: sync.release.0,
                release_point: sync.release.1,
            })?;
        }
        self.submit_queued(surface, buffer, configure, presented)
    }
}

#[cfg(test)]
mod tests;
