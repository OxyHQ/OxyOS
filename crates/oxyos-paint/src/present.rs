//! The device and the surfaces' GPU buffers (docs/adr/0031-gpu-buffers.mdx): images exported as
//! DMA-BUF with a modifier from the compositor's table, attached once, each with a timeline of its
//! own (a timeline's points signal in order and buffers are released out of order); a frame's
//! completion goes into its acquire point, and a buffer is drawn into again once its release point
//! has signalled.

#![allow(unsafe_code)]

use super::{name, Error, Paint, Result};
use ash::vk;
use oxyos_client::{Client, SyncPoints};
use oxyos_protocol::{Format, Rect, Size};
use std::collections::VecDeque;
use std::ffi::CStr;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};

/// Buffers kept for one size: one shown, one queued, one drawn into.
const BUFFERS: usize = 3;
/// Frames remembered for what a reused buffer lacks.
const HISTORY: usize = 8;

const USAGE: vk::ImageUsageFlags = vk::ImageUsageFlags::TRANSFER_DST;

pub(crate) struct Gpu {
    _entry: ash::Entry,
    instance: ash::Instance,
    physical: vk::PhysicalDevice,
    pub(crate) device: ash::Device,
    pub(crate) queue: vk::Queue,
    pub(crate) family: u32,
    pub(crate) memory: vk::PhysicalDeviceMemoryProperties,
    memory_fd: ash::khr::external_memory_fd::Device,
    modifiers: ash::ext::image_drm_format_modifier::Device,
    /// Each frame's completion, exported as a sync file; None: waited for on the CPU.
    completion: Option<(vk::Semaphore, ash::khr::external_semaphore_fd::Device)>,
    foreign: u32,
    pub(crate) name: String,
    pub(crate) same_device: bool,
    /// The modifiers B8G8R8A8_UNORM images are exported in as transfer destinations, with planes.
    exportable: Vec<(u64, u32)>,
}

impl Gpu {
    /// The device whose render node is `device`, else the first that exports DMA-BUF with
    /// modifiers (then with linear ones alone).
    pub(crate) fn open(device: u64) -> Result<Self> {
        let entry =
            // SAFETY: loading the system's Vulkan loader; nothing else in this process races with it.
            unsafe { ash::Entry::load() }.map_err(|error| Error(format!("no Vulkan: {error}")))?;
        let app = vk::ApplicationInfo::default()
            .application_name(c"oxyos-paint")
            .api_version(vk::API_VERSION_1_1);
        // SAFETY: the create info outlives the call.
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default().application_info(&app),
                None,
            )
        }?;
        let wanted = [
            ash::khr::external_memory_fd::NAME,
            ash::ext::external_memory_dma_buf::NAME,
            ash::ext::image_drm_format_modifier::NAME,
        ];
        // SAFETY: the instance is live.
        let physicals = unsafe { instance.enumerate_physical_devices() }.unwrap_or_default();
        let mut candidates = Vec::new();
        for physical in physicals {
            // SAFETY: `physical` was enumerated from this live instance.
            let available = unsafe { instance.enumerate_device_extension_properties(physical) }
                .unwrap_or_default();
            let has = |name: &CStr| {
                available
                    .iter()
                    .any(|extension| extension.extension_name_as_c_str() == Ok(name))
            };
            // Painting needs none of them; presenting in DMA-BUFs needs them all.
            let exports = wanted.iter().all(|name| has(name));
            let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
            let label;
            {
                let mut properties = vk::PhysicalDeviceProperties2::default();
                if has(ash::ext::physical_device_drm::NAME) {
                    properties = properties.push_next(&mut drm);
                }
                // SAFETY: `physical` is live; the chain outlives the call.
                unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
                label = name(&properties.properties.device_name);
            }
            let render = (drm.has_render == vk::TRUE)
                .then(|| rustix::fs::makedev(drm.render_major as u32, drm.render_minor as u32));
            candidates.push((
                physical,
                render == Some(device) && device != 0,
                label,
                has(ash::ext::queue_family_foreign::NAME),
                has(ash::khr::external_semaphore_fd::NAME),
                exports,
            ));
        }
        // The compositor's device first, then one that exports, then any.
        candidates.sort_by_key(|candidate| (!candidate.1, !candidate.5));
        let Some((physical, same_device, label, foreign, semaphore, exports)) =
            candidates.into_iter().next()
        else {
            // SAFETY: nothing was made from the instance.
            unsafe { instance.destroy_instance(None) };
            return Err(Error("no Vulkan device".into()));
        };
        // SAFETY: `physical` is live.
        let families = unsafe { instance.get_physical_device_queue_family_properties(physical) };
        let family = families
            .iter()
            .position(|family| family.queue_flags.contains(vk::QueueFlags::COMPUTE))
            .ok_or_else(|| Error("no compute queue".into()))? as u32;
        let sync_file = semaphore && {
            let mut properties = vk::ExternalSemaphoreProperties::default();
            // SAFETY: `physical` is live; Vulkan 1.1 has the query in its core.
            unsafe {
                instance.get_physical_device_external_semaphore_properties(
                    physical,
                    &vk::PhysicalDeviceExternalSemaphoreInfo::default()
                        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
                    &mut properties,
                )
            };
            properties
                .external_semaphore_features
                .contains(vk::ExternalSemaphoreFeatureFlags::EXPORTABLE)
        };
        let mut extensions: Vec<*const std::ffi::c_char> = if exports {
            wanted.iter().map(|name| name.as_ptr()).collect()
        } else {
            Vec::new()
        };
        if foreign {
            extensions.push(ash::ext::queue_family_foreign::NAME.as_ptr());
        }
        if sync_file {
            extensions.push(ash::khr::external_semaphore_fd::NAME.as_ptr());
        }
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        // SAFETY: `physical` belongs to the instance; the infos and names outlive the call.
        let device = unsafe {
            instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queues)
                    .enabled_extension_names(&extensions),
                None,
            )
        }?;
        // SAFETY: the device was created with a queue in `family`; `physical` is live.
        let (queue, memory) = unsafe {
            (
                device.get_device_queue(family, 0),
                instance.get_physical_device_memory_properties(physical),
            )
        };
        let completion = if sync_file {
            let mut export = vk::ExportSemaphoreCreateInfo::default()
                .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
            // SAFETY: the device is live; the info outlives the call.
            unsafe {
                device.create_semaphore(
                    &vk::SemaphoreCreateInfo::default().push_next(&mut export),
                    None,
                )
            }
            .ok()
            .map(|semaphore| {
                (
                    semaphore,
                    ash::khr::external_semaphore_fd::Device::new(&instance, &device),
                )
            })
        } else {
            None
        };
        let mut gpu = Self {
            memory_fd: ash::khr::external_memory_fd::Device::new(&instance, &device),
            modifiers: ash::ext::image_drm_format_modifier::Device::new(&instance, &device),
            _entry: entry,
            instance,
            physical,
            device,
            queue,
            family,
            memory,
            completion,
            foreign: if foreign {
                vk::QUEUE_FAMILY_FOREIGN_EXT
            } else {
                vk::QUEUE_FAMILY_EXTERNAL
            },
            name: label,
            same_device,
            exportable: Vec::new(),
        };
        if exports {
            gpu.exportable = gpu.exportable_modifiers();
        }
        Ok(gpu)
    }

    pub(crate) fn exportable_is_empty(&self) -> bool {
        self.exportable.is_empty()
    }

    fn exportable_modifiers(&self) -> Vec<(u64, u32)> {
        let format = vk::Format::B8G8R8A8_UNORM;
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
        let mut properties = vk::FormatProperties2::default().push_next(&mut list);
        // SAFETY: `physical` is live; the chain outlives the call.
        unsafe {
            self.instance.get_physical_device_format_properties2(
                self.physical,
                format,
                &mut properties,
            )
        };
        let mut modifiers = vec![
            vk::DrmFormatModifierPropertiesEXT::default();
            list.drm_format_modifier_count as usize
        ];
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default()
            .drm_format_modifier_properties(&mut modifiers);
        let mut properties = vk::FormatProperties2::default().push_next(&mut list);
        // SAFETY: as above, with room for every modifier.
        unsafe {
            self.instance.get_physical_device_format_properties2(
                self.physical,
                format,
                &mut properties,
            )
        };
        modifiers
            .iter()
            .filter(|modifier| {
                modifier
                    .drm_format_modifier_tiling_features
                    .contains(vk::FormatFeatureFlags::TRANSFER_DST)
                    && (self.same_device || modifier.drm_format_modifier == super::LINEAR)
            })
            .filter(|modifier| {
                let mut drm = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
                    .drm_format_modifier(modifier.drm_format_modifier)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE);
                let mut external = vk::PhysicalDeviceExternalImageFormatInfo::default()
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
                let info = vk::PhysicalDeviceImageFormatInfo2::default()
                    .format(format)
                    .ty(vk::ImageType::TYPE_2D)
                    .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                    .usage(USAGE)
                    .push_next(&mut drm)
                    .push_next(&mut external);
                let mut exported = vk::ExternalImageFormatProperties::default();
                let mut out = vk::ImageFormatProperties2::default().push_next(&mut exported);
                // SAFETY: `physical` is live; the chains outlive the call.
                let supported = unsafe {
                    self.instance.get_physical_device_image_format_properties2(
                        self.physical,
                        &info,
                        &mut out,
                    )
                }
                .is_ok();
                let _ = out;
                supported
                    && exported
                        .external_memory_properties
                        .external_memory_features
                        .contains(vk::ExternalMemoryFeatureFlags::EXPORTABLE)
            })
            .map(|modifier| {
                (
                    modifier.drm_format_modifier,
                    modifier.drm_format_modifier_plane_count,
                )
            })
            .collect()
    }

    /// An image of `width` × `height` in one of `modifiers`, exported.
    fn image(&self, width: u32, height: u32, modifiers: &[u64]) -> Result<Image> {
        let mut list =
            vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(modifiers);
        let mut external = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::B8G8R8A8_UNORM)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(USAGE)
            .push_next(&mut list)
            .push_next(&mut external);
        let device = &self.device;
        // SAFETY: the device is live and every info outlives its call; what is made is destroyed
        // by `free_image`, or below when a later step fails.
        unsafe {
            let image = device.create_image(&info, None)?;
            let requirements = device.get_image_memory_requirements(image);
            let Some(kind) = (0..self.memory.memory_type_count)
                .find(|index| requirements.memory_type_bits & (1 << index) != 0)
            else {
                device.destroy_image(image, None);
                return Err(Error("no memory type for the image".into()));
            };
            let mut export = vk::ExportMemoryAllocateInfo::default()
                .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
            let memory = match device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(kind)
                    .push_next(&mut export)
                    .push_next(&mut dedicated),
                None,
            ) {
                Ok(memory) => memory,
                Err(error) => {
                    device.destroy_image(image, None);
                    return Err(error.into());
                }
            };
            let fail = |error: Error| {
                device.destroy_image(image, None);
                device.free_memory(memory, None);
                error
            };
            device
                .bind_image_memory(image, memory, 0)
                .map_err(|error| fail(error.into()))?;
            let fd = self
                .memory_fd
                .get_memory_fd(
                    &vk::MemoryGetFdInfoKHR::default()
                        .memory(memory)
                        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT),
                )
                .map_err(|error| fail(error.into()))?;
            // SAFETY: vkGetMemoryFdKHR returns a new descriptor the caller owns.
            let fd = OwnedFd::from_raw_fd(fd);
            let mut chosen = vk::ImageDrmFormatModifierPropertiesEXT::default();
            self.modifiers
                .get_image_drm_format_modifier_properties(image, &mut chosen)
                .map_err(|error| fail(error.into()))?;
            let modifier = chosen.drm_format_modifier;
            let count = self
                .exportable
                .iter()
                .find(|(exportable, _)| *exportable == modifier)
                .map_or(1, |(_, planes)| *planes);
            let aspects = [
                vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
                vk::ImageAspectFlags::MEMORY_PLANE_1_EXT,
                vk::ImageAspectFlags::MEMORY_PLANE_2_EXT,
                vk::ImageAspectFlags::MEMORY_PLANE_3_EXT,
            ];
            let planes = aspects[..count.min(4) as usize]
                .iter()
                .map(|&aspect| {
                    let layout = device.get_image_subresource_layout(
                        image,
                        vk::ImageSubresource {
                            aspect_mask: aspect,
                            mip_level: 0,
                            array_layer: 0,
                        },
                    );
                    (layout.offset as u32, layout.row_pitch as u32)
                })
                .collect();
            Ok(Image {
                image,
                memory,
                fd,
                modifier,
                planes,
            })
        }
    }

    fn free_image(&self, image: &Image) {
        // SAFETY: no submission in flight uses it (the caller waited); each handle once.
        unsafe {
            self.device.destroy_image(image.image, None);
            self.device.free_memory(image.memory, None);
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: every object is this device's and no work is pending once it is idle.
        unsafe {
            let _ = self.device.device_wait_idle();
            if let Some((semaphore, _)) = self.completion.take() {
                self.device.destroy_semaphore(semaphore, None);
            }
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

struct Image {
    image: vk::Image,
    memory: vk::DeviceMemory,
    fd: OwnedFd,
    modifier: u64,
    planes: Vec<(u32, u32)>,
}

/// One of a surface's GPU buffers.
struct Buffer {
    image: Image,
    width: u32,
    height: u32,
    format: Format,
    /// The connection's buffer and the buffer's own timeline.
    id: u64,
    timeline: u32,
    /// Its last frame's release point (0: never submitted).
    released_at: u64,
    /// The sequence of the frame it holds (0: none).
    written: u64,
}

/// A frame presented: its sequence, size and damage (none: everything).
type Presented = (u64, (u32, u32), Option<Vec<Rect>>);

/// A surface's GPU buffers, and the frames presented lately (what a reused buffer lacks).
#[derive(Default)]
pub(crate) struct Swapchain {
    buffers: Vec<Buffer>,
    history: VecDeque<Presented>,
}

impl Swapchain {
    fn idle(buffer: &Buffer, client: &Client) -> bool {
        buffer.released_at == 0
            || client
                .timeline_signalled(buffer.timeline)
                .is_ok_and(|signalled| signalled >= buffer.released_at)
    }

    fn drop_buffer(gpu: &Gpu, client: &mut Client, buffer: Buffer) {
        let _ = client.detach_buffer(buffer.id);
        let _ = client.destroy_timeline(buffer.timeline);
        gpu.free_image(&buffer.image);
    }

    /// Detaches and frees every buffer (the surface is gone; the device is idle).
    pub(crate) fn destroy(self, gpu: &Gpu, client: &mut Client) {
        for buffer in self.buffers {
            Self::drop_buffer(gpu, client, buffer);
        }
    }

    /// Frees the images alone (the connection is gone with the painter).
    pub(crate) fn free(self, gpu: &Gpu) {
        for buffer in self.buffers {
            gpu.free_image(&buffer.image);
        }
    }
}

/// `Paint::present`.
pub(crate) fn present(
    paint: &mut Paint,
    client: &mut Client,
    surface: u32,
    configure: u64,
    canvas: u32,
    format: Format,
    damage: &[Rect],
) -> Result<u64> {
    let (offset, width, height) = paint
        .canvas_offset(canvas)
        .ok_or_else(|| Error("no such canvas".into()))?;
    let table = client
        .gpu()
        .ok_or_else(|| Error("the compositor offers no GPU buffers".into()))?
        .clone();
    // The surface's scanout formats first (it is all its output shows), else what composes.
    let scanout: Vec<u64> = client
        .scanout_formats(surface)
        .iter()
        .filter(|(told, _)| *told == format)
        .map(|(_, modifier)| *modifier)
        .collect();
    let composed: Vec<u64> = table
        .formats
        .iter()
        .filter(|(told, _)| *told == format)
        .map(|(_, modifier)| *modifier)
        .collect();
    let mut modifiers = best_shared(&scanout, &paint.gpu.exportable);
    if modifiers.is_empty() {
        modifiers = best_shared(&composed, &paint.gpu.exportable);
    }
    if modifiers.is_empty() {
        return Err(Error(
            "no modifier both the compositor and this device take".into(),
        ));
    }
    let mut chain = paint.surfaces.remove(&surface).unwrap_or_default();
    let result = present_in(
        paint,
        client,
        &mut chain,
        surface,
        configure,
        (offset, width, height),
        format,
        damage,
        &modifiers,
    );
    paint.surfaces.insert(surface, chain);
    result
}

/// The modifiers offered for a buffer the driver picks from (it knows which of its own tilings
/// is best): of those the compositor takes (`offered`) and this device exports (`exportable`,
/// with their plane counts), the best kind both share, as the compositor ranks them (ADR 0031,
/// decision 2): tiled ones of a single plane, else linear, else those with auxiliary planes.
/// Offered all at once, a driver may take linear or a compressed one when a tiled one would do.
pub(crate) fn best_shared(offered: &[u64], exportable: &[(u64, u32)]) -> Vec<u64> {
    let shared: Vec<(u64, u32)> = exportable
        .iter()
        .copied()
        .filter(|(modifier, _)| offered.contains(modifier))
        .collect();
    let kind = |(modifier, planes): (u64, u32)| match (modifier, planes) {
        (_, planes) if planes > 1 => 2,
        (super::LINEAR, _) => 1,
        _ => 0,
    };
    let Some(best) = shared.iter().map(|&entry| kind(entry)).min() else {
        return Vec::new();
    };
    shared
        .into_iter()
        .filter(|&entry| kind(entry) == best)
        .map(|(modifier, _)| modifier)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn present_in(
    paint: &mut Paint,
    client: &mut Client,
    chain: &mut Swapchain,
    surface: u32,
    configure: u64,
    (offset, width, height): (u64, u32, u32),
    format: Format,
    damage: &[Rect],
    modifiers: &[u64],
) -> Result<u64> {
    let started = std::time::Instant::now();
    let mut times = super::PresentTimes::default();
    let mut since = started;
    let mut lap = || {
        let now = std::time::Instant::now();
        let ms = (now - since).as_secs_f64() * 1e3;
        since = now;
        ms
    };
    if chain
        .buffers
        .iter()
        .any(|buffer| client.buffer_failed(buffer.id))
    {
        return Err(Error("the compositor could not import a GPU buffer".into()));
    }
    // Buffers of another size or format, once released, go.
    let (keep, stale): (Vec<Buffer>, Vec<Buffer>) = std::mem::take(&mut chain.buffers)
        .into_iter()
        .partition(|buffer| {
            (buffer.width, buffer.height, buffer.format) == (width, height, format)
                || !Swapchain::idle(buffer, client)
        });
    chain.buffers = keep;
    if !stale.is_empty() {
        paint.begin()?;
        for buffer in stale {
            Swapchain::drop_buffer(&paint.gpu, client, buffer);
        }
    }
    times.freed_ms = lap();
    let fits =
        |buffer: &Buffer| (buffer.width, buffer.height, buffer.format) == (width, height, format);
    let index = match chain
        .buffers
        .iter()
        .position(|buffer| fits(buffer) && Swapchain::idle(buffer, client))
    {
        Some(index) => index,
        None if chain.buffers.iter().filter(|buffer| fits(buffer)).count() < BUFFERS => {
            times.new_image = true;
            let image = paint.gpu.image(width, height, modifiers)?;
            times.image_ms = lap();
            let planes: Vec<_> = image
                .planes
                .iter()
                .map(|&(offset, stride)| (image.fd.as_fd(), offset, stride))
                .collect();
            let id =
                client.attach_dmabuf(format, image.modifier, Size::new(width, height), &planes)?;
            times.attach_ms = lap();
            let timeline = client.create_timeline()?;
            times.timeline_ms = lap();
            chain.buffers.push(Buffer {
                image,
                width,
                height,
                format,
                id,
                timeline,
                released_at: 0,
                written: 0,
            });
            chain.buffers.len() - 1
        }
        None => {
            // Every buffer of this size is the compositor's: the one released first is waited for.
            let index = chain
                .buffers
                .iter()
                .enumerate()
                .filter(|(_, buffer)| fits(buffer))
                .min_by_key(|(_, buffer)| buffer.released_at)
                .map(|(index, _)| index)
                .expect("buffers of this size");
            let buffer = &chain.buffers[index];
            let ready = client.timeline_eventfd(buffer.timeline, buffer.released_at, false)?;
            let mut fds = [rustix::event::PollFd::new(
                &ready,
                rustix::event::PollFlags::IN,
            )];
            let second = rustix::event::Timespec {
                tv_sec: 1,
                tv_nsec: 0,
            };
            rustix::event::poll(&mut fds, Some(&second)).map_err(std::io::Error::from)?;
            if !Swapchain::idle(&chain.buffers[index], client) {
                return Err(Error("no GPU buffer was released within a second".into()));
            }
            index
        }
    };
    times.release_ms += lap();
    // What this buffer lacks: everything when new or when the history does not reach back, else
    // the damage of every frame since it was written, and this one's.
    let whole = vec![Rect::new(0, 0, width, height)];
    let written = chain.buffers[index].written;
    let missed: Option<Vec<Rect>> = if written == 0 || damage.is_empty() {
        None
    } else {
        let reaches = chain
            .history
            .iter()
            .any(|(sequence, ..)| *sequence <= written);
        reaches
            .then(|| {
                chain
                    .history
                    .iter()
                    .filter(|(sequence, ..)| *sequence > written)
                    .try_fold(damage.to_vec(), |mut all, (_, size, rects)| {
                        if *size != (width, height) {
                            return None;
                        }
                        all.extend(rects.as_ref()?);
                        Some(all)
                    })
            })
            .flatten()
    };
    let regions: Vec<Rect> = missed
        .unwrap_or(whole)
        .into_iter()
        .filter_map(|rect| {
            let x0 = rect.x.max(0) as u32;
            let y0 = rect.y.max(0) as u32;
            let x1 = (i64::from(rect.x) + i64::from(rect.width)).clamp(0, i64::from(width)) as u32;
            let y1 =
                (i64::from(rect.y) + i64::from(rect.height)).clamp(0, i64::from(height)) as u32;
            (x0 < x1 && y0 < y1).then(|| Rect::new(x0 as i32, y0 as i32, x1 - x0, y1 - y0))
        })
        .collect();
    lap();
    paint.begin()?;
    times.wait_ms = lap();
    paint.begin_commands()?;
    times.record_ms = lap();
    times.operations = paint.dispatches.len() as u32;
    let buffer = &chain.buffers[index];
    let gpu = &paint.gpu;
    let device = &gpu.device;
    let command = paint.command;
    let first = buffer.released_at == 0;
    // SAFETY: the command buffer is recording; every handle is live; each region lies within the
    // image and the canvas.
    unsafe {
        device.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::COMPUTE_SHADER | vk::PipelineStageFlags::HOST,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::HOST_WRITE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ)],
            &[],
            &[],
        );
        // Back from the compositor's queue with what it held, or new.
        let (old, from, to) = if first {
            (
                vk::ImageLayout::UNDEFINED,
                vk::QUEUE_FAMILY_IGNORED,
                vk::QUEUE_FAMILY_IGNORED,
            )
        } else {
            (vk::ImageLayout::GENERAL, gpu.foreign, gpu.family)
        };
        device.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[vk::ImageMemoryBarrier::default()
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .old_layout(old)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(from)
                .dst_queue_family_index(to)
                .image(buffer.image.image)
                .subresource_range(range())],
        );
        let copies: Vec<vk::BufferImageCopy> = regions
            .iter()
            .map(|rect| {
                vk::BufferImageCopy::default()
                    .buffer_offset(
                        (offset
                            + u64::from(rect.y as u32) * u64::from(width)
                            + u64::from(rect.x as u32))
                            * 4,
                    )
                    .buffer_row_length(width)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_offset(vk::Offset3D {
                        x: rect.x,
                        y: rect.y,
                        z: 0,
                    })
                    .image_extent(vk::Extent3D {
                        width: rect.width,
                        height: rect.height,
                        depth: 1,
                    })
            })
            .collect();
        if !copies.is_empty() {
            device.cmd_copy_buffer_to_image(
                command,
                paint.heap.buffer,
                buffer.image.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &copies,
            );
        }
        // To the compositor's queue.
        device.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(gpu.family)
                .dst_queue_family_index(gpu.foreign)
                .image(buffer.image.image)
                .subresource_range(range())],
        );
    }
    let signal: Vec<vk::Semaphore> = gpu
        .completion
        .iter()
        .map(|(semaphore, _)| *semaphore)
        .collect();
    paint.submit(&signal)?;
    times.submit_ms = lap();
    let buffer = &chain.buffers[index];
    let (acquire, release) = (buffer.released_at + 1, buffer.released_at + 2);
    // The frame's completion is its acquire point: as a sync file, or signalled once done.
    let fence = match &paint.gpu.completion {
        Some((semaphore, loader)) => {
            // SAFETY: the semaphore has a signal operation pending (the submission above).
            let fd = unsafe {
                loader.get_semaphore_fd(
                    &vk::SemaphoreGetFdInfoKHR::default()
                        .semaphore(*semaphore)
                        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
                )
            }?;
            // SAFETY: vkGetSemaphoreFdKHR returns a new descriptor the caller owns (-1: done).
            (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
        }
        None => None,
    };
    match fence {
        Some(fence) => client.timeline_import_sync_file(buffer.timeline, acquire, fence.as_fd())?,
        None => {
            paint.wait()?;
            client.timeline_signal(buffer.timeline, acquire)?;
        }
    }
    times.sync_ms = lap();
    let told = (!damage.is_empty()).then_some(damage);
    let sequence = client.submit_buffer_synced(
        surface,
        buffer.id,
        configure,
        told,
        Some(SyncPoints {
            acquire: (buffer.timeline, acquire),
            release: (buffer.timeline, release),
        }),
    )?;
    times.send_ms = lap();
    times.total_ms = (std::time::Instant::now() - started).as_secs_f64() * 1e3;
    paint.last_present = times;
    let buffer = &mut chain.buffers[index];
    buffer.released_at = release;
    buffer.written = sequence;
    chain
        .history
        .push_back((sequence, (width, height), told.map(<[Rect]>::to_vec)));
    if chain.history.len() > HISTORY {
        chain.history.pop_front();
    }
    let _ = buffer.image.fd.as_raw_fd();
    Ok(sequence)
}

fn range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}
