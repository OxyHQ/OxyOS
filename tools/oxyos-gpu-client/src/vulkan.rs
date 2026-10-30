//! The client's GPU: a Vulkan device that shares memory with the compositor's, images it draws
//! into and exports as DMA-BUF with a modifier from the compositor's table, and each frame's
//! completion as a sync file (docs/adr/0031-gpu-buffers.mdx).

#![allow(unsafe_code)]

use ash::vk;
use std::ffi::CStr;
use std::os::fd::{FromRawFd, OwnedFd};

/// The linear modifier, which any device reads.
pub const LINEAR: u64 = 0;

/// A rectangle of an image, in its pixels, and its colour (premultiplied, 0 to 1: B, G, R, A in
/// memory is Vulkan's B8G8R8A8_UNORM, the protocol's Argb8888).
#[derive(Clone, Copy, Debug)]
pub struct Fill {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: [f32; 4],
}

/// An image the client draws into, exported as one DMA-BUF.
pub struct Image {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    framebuffer: vk::Framebuffer,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    /// Its memory as a DMA-BUF, for every plane.
    pub fd: OwnedFd,
    pub modifier: u64,
    /// Each plane's offset and stride.
    pub planes: Vec<(u32, u32)>,
    pub width: u32,
    pub height: u32,
    /// Drawn before: the GPU's work on it is waited for before it is drawn again, and it comes
    /// back from the compositor's queue.
    drawn: bool,
}

pub struct Gpu {
    _entry: ash::Entry,
    instance: ash::Instance,
    physical: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    family: u32,
    memory: vk::PhysicalDeviceMemoryProperties,
    render_pass: vk::RenderPass,
    pool: vk::CommandPool,
    memory_fd: ash::khr::external_memory_fd::Device,
    modifiers: ash::ext::image_drm_format_modifier::Device,
    /// Each submission's completion, exported as a sync file; None: waited for on the CPU.
    completion: Option<(vk::Semaphore, ash::khr::external_semaphore_fd::Device)>,
    /// The queue family the compositor reads from: foreign where the device names one.
    foreign: u32,
    pub name: String,
    /// The device is the compositor's own (its render node is the table's), so tiled modifiers
    /// can be shared; otherwise only linear ones.
    pub same_device: bool,
    /// The modifiers this device exports B8G8R8A8_UNORM color attachments in, with their planes.
    exportable: Vec<(u64, u32)>,
}

fn name(raw: &[std::ffi::c_char]) -> String {
    // SAFETY: Vulkan's names are NUL-terminated within their arrays.
    unsafe { CStr::from_ptr(raw.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

impl Gpu {
    /// The device whose render node is `device` (a dev_t), else the first that exports DMA-BUF.
    pub fn open(device: u64) -> Result<Self, String> {
        // SAFETY: loading the system's Vulkan loader; nothing else in this process races with it.
        let entry = unsafe { ash::Entry::load() }.map_err(|error| format!("no Vulkan: {error}"))?;
        let app = vk::ApplicationInfo::default()
            .application_name(c"oxyos-gpu-client")
            .api_version(vk::API_VERSION_1_1);
        // SAFETY: the create info outlives the call.
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default().application_info(&app),
                None,
            )
        }
        .map_err(|error| format!("Vulkan instance: {error}"))?;
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
            if !wanted.iter().all(|name| has(name)) {
                continue;
            }
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
            let foreign = has(ash::ext::queue_family_foreign::NAME);
            let semaphore = has(ash::khr::external_semaphore_fd::NAME);
            candidates.push((
                physical,
                render == Some(device) && device != 0,
                label,
                foreign,
                semaphore,
            ));
        }
        // The compositor's device first, then any.
        candidates.sort_by_key(|candidate| !candidate.1);
        let (physical, same_device, label, foreign, semaphore) = candidates
            .into_iter()
            .next()
            .ok_or("no Vulkan device exports DMA-BUF with modifiers")?;
        // SAFETY: `physical` is live.
        let families = unsafe { instance.get_physical_device_queue_family_properties(physical) };
        let family = families
            .iter()
            .position(|family| family.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .ok_or("no graphics queue")? as u32;
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
        let mut extensions: Vec<*const std::ffi::c_char> =
            wanted.iter().map(|name| name.as_ptr()).collect();
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
        }
        .map_err(|error| format!("Vulkan device: {error}"))?;
        // SAFETY: the device was created with a queue in `family`; `physical` is live.
        let (queue, memory) = unsafe {
            (
                device.get_device_queue(family, 0),
                instance.get_physical_device_memory_properties(physical),
            )
        };
        let attachment = [vk::AttachmentDescription::default()
            .format(vk::Format::B8G8R8A8_UNORM)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .final_layout(vk::ImageLayout::GENERAL)];
        let reference = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let subpass = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&reference)];
        // SAFETY: the device is live; the infos outlive the calls.
        let (render_pass, pool) = unsafe {
            (
                device.create_render_pass(
                    &vk::RenderPassCreateInfo::default()
                        .attachments(&attachment)
                        .subpasses(&subpass),
                    None,
                ),
                device.create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(family)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                ),
            )
        };
        let render_pass = render_pass.map_err(|error| error.to_string())?;
        let pool = pool.map_err(|error| error.to_string())?;
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
            render_pass,
            pool,
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
        gpu.exportable = gpu.exportable_modifiers();
        Ok(gpu)
    }

    /// Whether each frame's completion is a sync file (else the CPU waits for it).
    pub fn exports_sync_files(&self) -> bool {
        self.completion.is_some()
    }

    const USAGE: vk::ImageUsageFlags = vk::ImageUsageFlags::COLOR_ATTACHMENT;

    /// The modifiers B8G8R8A8_UNORM color attachments are exported as DMA-BUF in, with each one's
    /// plane count; linear alone on another device than the compositor's.
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
                    .contains(vk::FormatFeatureFlags::COLOR_ATTACHMENT)
                    && (self.same_device || modifier.drm_format_modifier == LINEAR)
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
                    .usage(Self::USAGE)
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

    /// Of `offered` (the compositor's modifiers for Argb8888), those this device can export.
    pub fn shareable(&self, offered: &[u64]) -> Vec<u64> {
        self.exportable
            .iter()
            .map(|(modifier, _)| *modifier)
            .filter(|modifier| offered.contains(modifier))
            .collect()
    }

    /// A new image of `width` × `height` in one of `modifiers` (the driver chooses), exported.
    pub fn image(&self, width: u32, height: u32, modifiers: &[u64]) -> Result<Image, String> {
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
            .usage(Self::USAGE)
            .push_next(&mut list)
            .push_next(&mut external);
        let device = &self.device;
        // SAFETY: the device is live and every info outlives its call; each object made is
        // destroyed by `destroy` (or below when a later step fails).
        unsafe {
            let image = device
                .create_image(&info, None)
                .map_err(|error| format!("image: {error}"))?;
            let requirements = device.get_image_memory_requirements(image);
            let Some(kind) = (0..self.memory.memory_type_count)
                .find(|index| requirements.memory_type_bits & (1 << index) != 0)
            else {
                device.destroy_image(image, None);
                return Err("no memory type for the image".into());
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
                    return Err(format!("exportable memory: {error}"));
                }
            };
            let fail = |error: String| {
                device.destroy_image(image, None);
                device.free_memory(memory, None);
                error
            };
            device
                .bind_image_memory(image, memory, 0)
                .map_err(|error| fail(error.to_string()))?;
            let fd = self
                .memory_fd
                .get_memory_fd(
                    &vk::MemoryGetFdInfoKHR::default()
                        .memory(memory)
                        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT),
                )
                .map_err(|error| fail(format!("DMA-BUF export: {error}")))?;
            // SAFETY: vkGetMemoryFdKHR returns a new descriptor the caller owns.
            let fd = OwnedFd::from_raw_fd(fd);
            let mut chosen = vk::ImageDrmFormatModifierPropertiesEXT::default();
            self.modifiers
                .get_image_drm_format_modifier_properties(image, &mut chosen)
                .map_err(|error| fail(error.to_string()))?;
            let modifier = chosen.drm_format_modifier;
            let plane_count = self
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
            let planes = aspects[..plane_count.min(4) as usize]
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
            let view = device
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(vk::Format::B8G8R8A8_UNORM)
                        .subresource_range(color_range()),
                    None,
                )
                .map_err(|error| fail(error.to_string()))?;
            let views = [view];
            let framebuffer = device
                .create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(self.render_pass)
                        .attachments(&views)
                        .width(width)
                        .height(height)
                        .layers(1),
                    None,
                )
                .map_err(|error| fail(error.to_string()))?;
            let command = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(self.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .map_err(|error| fail(error.to_string()))?[0];
            let fence = device
                .create_fence(&vk::FenceCreateInfo::default(), None)
                .map_err(|error| fail(error.to_string()))?;
            Ok(Image {
                image,
                memory,
                view,
                framebuffer,
                command,
                fence,
                fd,
                modifier,
                planes,
                width,
                height,
                drawn: false,
            })
        }
    }

    /// Draws `fills` into `image` (the first covering it whole), on the GPU, and hands it to the
    /// compositor's queue. Returns the drawing's completion as a sync file; None when the device
    /// gives none (the drawing is then waited for here, and done).
    pub fn draw(&self, image: &mut Image, fills: &[Fill]) -> Result<Option<OwnedFd>, String> {
        let device = &self.device;
        let background = fills.first().map_or([0.0; 4], |fill| fill.rgba);
        // SAFETY: every handle is live and the image's; its previous drawing is waited for before
        // its command buffer is recorded again; every info outlives its call.
        unsafe {
            if image.drawn {
                device
                    .wait_for_fences(&[image.fence], true, u64::MAX)
                    .map_err(|error| error.to_string())?;
                device
                    .reset_fences(&[image.fence])
                    .map_err(|error| error.to_string())?;
            }
            let command = image.command;
            device
                .reset_command_buffer(command, vk::CommandBufferResetFlags::empty())
                .map_err(|error| error.to_string())?;
            device
                .begin_command_buffer(
                    command,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(|error| error.to_string())?;
            // Back from the compositor's queue (its content is drawn again whole), or new.
            let (from, old) = if image.drawn {
                (self.foreign, vk::ImageLayout::GENERAL)
            } else {
                (vk::QUEUE_FAMILY_IGNORED, vk::ImageLayout::UNDEFINED)
            };
            let to = if image.drawn {
                self.family
            } else {
                vk::QUEUE_FAMILY_IGNORED
            };
            device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::empty())
                    .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .old_layout(old)
                    .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .src_queue_family_index(from)
                    .dst_queue_family_index(to)
                    .image(image.image)
                    .subresource_range(color_range())],
            );
            let clear = [vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: background,
                },
            }];
            let area = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: image.width,
                    height: image.height,
                },
            };
            device.cmd_begin_render_pass(
                command,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(self.render_pass)
                    .framebuffer(image.framebuffer)
                    .render_area(area)
                    .clear_values(&clear),
                vk::SubpassContents::INLINE,
            );
            for fill in fills.iter().skip(1) {
                let x = fill.x.min(image.width);
                let y = fill.y.min(image.height);
                let width = fill.width.min(image.width - x);
                let height = fill.height.min(image.height - y);
                if width == 0 || height == 0 {
                    continue;
                }
                device.cmd_clear_attachments(
                    command,
                    &[vk::ClearAttachment {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        color_attachment: 0,
                        clear_value: vk::ClearValue {
                            color: vk::ClearColorValue { float32: fill.rgba },
                        },
                    }],
                    &[vk::ClearRect {
                        rect: vk::Rect2D {
                            offset: vk::Offset2D {
                                x: x as i32,
                                y: y as i32,
                            },
                            extent: vk::Extent2D { width, height },
                        },
                        base_array_layer: 0,
                        layer_count: 1,
                    }],
                );
            }
            device.cmd_end_render_pass(command);
            // To the compositor's queue.
            device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .dst_access_mask(vk::AccessFlags::empty())
                    .old_layout(vk::ImageLayout::GENERAL)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(self.family)
                    .dst_queue_family_index(self.foreign)
                    .image(image.image)
                    .subresource_range(color_range())],
            );
            device
                .end_command_buffer(command)
                .map_err(|error| error.to_string())?;
            let commands = [command];
            let signal: Vec<vk::Semaphore> = self
                .completion
                .iter()
                .map(|(semaphore, _)| *semaphore)
                .collect();
            device
                .queue_submit(
                    self.queue,
                    &[vk::SubmitInfo::default()
                        .command_buffers(&commands)
                        .signal_semaphores(&signal)],
                    image.fence,
                )
                .map_err(|error| format!("submit: {error}"))?;
            image.drawn = true;
            match &self.completion {
                Some((semaphore, loader)) => {
                    let fd = loader
                        .get_semaphore_fd(
                            &vk::SemaphoreGetFdInfoKHR::default()
                                .semaphore(*semaphore)
                                .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
                        )
                        .map_err(|error| format!("completion as a sync file: {error}"))?;
                    if fd < 0 {
                        // Done already: nothing to wait on.
                        return Ok(None);
                    }
                    // SAFETY: vkGetSemaphoreFdKHR returns a new descriptor the caller owns.
                    Ok(Some(OwnedFd::from_raw_fd(fd)))
                }
                None => {
                    device
                        .wait_for_fences(&[image.fence], true, u64::MAX)
                        .map_err(|error| error.to_string())?;
                    Ok(None)
                }
            }
        }
    }

    /// Frees an image the compositor no longer reads.
    pub fn destroy(&self, image: Image) {
        // SAFETY: the image's objects are this device's; its drawing is waited for first.
        unsafe {
            if image.drawn {
                let _ = self.device.wait_for_fences(&[image.fence], true, u64::MAX);
            }
            self.device.destroy_fence(image.fence, None);
            self.device
                .free_command_buffers(self.pool, &[image.command]);
            self.device.destroy_framebuffer(image.framebuffer, None);
            self.device.destroy_image_view(image.view, None);
            self.device.destroy_image(image.image, None);
            self.device.free_memory(image.memory, None);
        }
    }
}

fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
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
            self.device.destroy_command_pool(self.pool, None);
            self.device.destroy_render_pass(self.render_pass, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}
