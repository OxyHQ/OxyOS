//! Clients' GPU buffers (docs/adr/0031-gpu-buffers.mdx): DMA-BUFs imported as images with their
//! DRM format modifiers (`VK_EXT_external_memory_dma_buf`, `VK_EXT_image_drm_format_modifier`)
//! and sampled in place by the composition shader, from an array of images each layer indexes
//! (`shaders/compose.wgsl`'s `texel`). A frame acquires each image it reads from the foreign
//! queue family that wrote it and releases it back after.

#![allow(unsafe_code)]

use super::{Error, VulkanCompositor};
use ash::vk;
use std::collections::HashMap;
use std::ffi::CStr;
use std::os::fd::{AsRawFd, BorrowedFd, IntoRawFd, OwnedFd};

/// How many clients' GPU buffers one frame composes, at most; build.rs's `IMAGE_SLOTS`.
pub(crate) const IMAGE_SLOTS: usize = 64;

/// The DRM formats composed (`oxyos_protocol::Format`'s), each Vulkan's `B8G8R8A8_UNORM`: B, G,
/// R, A in memory, premultiplied (ARGB8888) or with the alpha ignored (XRGB8888).
const ARGB8888: u32 = 0x3432_5241;
const XRGB8888: u32 = 0x3432_5258;
const FORMAT: vk::Format = vk::Format::B8G8R8A8_UNORM;

/// A DRM format and modifier the device imports DMA-BUFs in and samples, and how many memory
/// planes a buffer of it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaBufFormat {
    pub fourcc: u32,
    pub modifier: u64,
    pub planes: u32,
}

/// One plane of a DMA-BUF: its descriptor, where it starts and its row pitch.
#[derive(Clone, Copy, Debug)]
pub struct DmaBufPlane<'a> {
    pub fd: BorrowedFd<'a>,
    pub offset: u32,
    pub stride: u32,
}

/// What the device offers for clients' GPU buffers, found before it is created.
pub(crate) struct Capabilities {
    formats: Vec<DmaBufFormat>,
    foreign: bool,
    node: Option<u64>,
    extensions: Vec<&'static CStr>,
}

/// `makedev` of glibc, as `st_rdev` holds a device number.
fn makedev(major: u64, minor: u64) -> u64 {
    ((major & 0xffff_f000) << 32)
        | ((major & 0xfff) << 8)
        | ((minor & 0xffff_ff00) << 12)
        | (minor & 0xff)
}

impl Capabilities {
    /// What `physical` offers, if it can import and sample DMA-BUFs with modifiers: Vulkan 1.2,
    /// the modifier extension, non-uniform indexing of sampled images and room for every slot.
    pub(crate) fn of(
        instance: &ash::Instance,
        physical: vk::PhysicalDevice,
        available: &[vk::ExtensionProperties],
        _family: u32,
    ) -> Option<Self> {
        let has = |name: &CStr| {
            available
                .iter()
                .any(|extension| extension.extension_name_as_c_str() == Ok(name))
        };
        // SAFETY: `physical` was enumerated from this live instance.
        let properties = unsafe { instance.get_physical_device_properties(physical) };
        if properties.api_version < vk::API_VERSION_1_2
            || !has(ash::ext::image_drm_format_modifier::NAME)
            || (properties.limits.max_per_stage_descriptor_sampled_images as usize) < IMAGE_SLOTS
        {
            return None;
        }
        let mut indexing = vk::PhysicalDeviceVulkan12Features::default();
        let mut features = vk::PhysicalDeviceFeatures2::default().push_next(&mut indexing);
        // SAFETY: as above; Vulkan 1.2 devices answer its features.
        unsafe { instance.get_physical_device_features2(physical, &mut features) };
        if indexing.shader_sampled_image_array_non_uniform_indexing == vk::FALSE {
            return None;
        }
        let foreign = has(ash::ext::queue_family_foreign::NAME);
        let drm = has(ash::ext::physical_device_drm::NAME);
        let node = drm
            .then(|| {
                let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
                let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut drm);
                // SAFETY: as above; the extension is available on the device.
                unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
                (drm.has_render == vk::TRUE)
                    .then(|| makedev(drm.render_major as u64, drm.render_minor as u64))
            })
            .flatten();
        let formats = Self::formats(instance, physical);
        if formats.is_empty() {
            return None;
        }
        let mut extensions = vec![ash::ext::image_drm_format_modifier::NAME];
        if foreign {
            extensions.push(ash::ext::queue_family_foreign::NAME);
        }
        if drm {
            extensions.push(ash::ext::physical_device_drm::NAME);
        }
        Some(Self {
            formats,
            foreign,
            node,
            extensions,
        })
    }

    pub(crate) fn extensions(&self) -> &[&'static CStr] {
        &self.extensions
    }

    /// Each modifier of `B8G8R8A8_UNORM` the device samples and imports from a DMA-BUF, for
    /// both DRM formats it is.
    fn formats(instance: &ash::Instance, physical: vk::PhysicalDevice) -> Vec<DmaBufFormat> {
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
        let mut properties = vk::FormatProperties2::default().push_next(&mut list);
        // SAFETY: `physical` is live; the query fills only the count first.
        unsafe {
            instance.get_physical_device_format_properties2(physical, FORMAT, &mut properties)
        };
        let mut modifiers = vec![
            vk::DrmFormatModifierPropertiesEXT::default();
            list.drm_format_modifier_count as usize
        ];
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default()
            .drm_format_modifier_properties(&mut modifiers);
        let mut properties = vk::FormatProperties2::default().push_next(&mut list);
        // SAFETY: as above, with room for every modifier counted.
        unsafe {
            instance.get_physical_device_format_properties2(physical, FORMAT, &mut properties)
        };
        let importable = |modifier: u64| {
            let mut drm = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
                .drm_format_modifier(modifier)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let mut external = vk::PhysicalDeviceExternalImageFormatInfo::default()
                .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let info = vk::PhysicalDeviceImageFormatInfo2::default()
                .format(FORMAT)
                .ty(vk::ImageType::TYPE_2D)
                .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                .usage(vk::ImageUsageFlags::SAMPLED)
                .push_next(&mut drm)
                .push_next(&mut external);
            let mut answer = vk::ExternalImageFormatProperties::default();
            let mut out = vk::ImageFormatProperties2::default().push_next(&mut answer);
            // SAFETY: `physical` is live; every struct chained outlives the call.
            let result = unsafe {
                instance.get_physical_device_image_format_properties2(physical, &info, &mut out)
            };
            let _ = out;
            result.is_ok()
                && answer
                    .external_memory_properties
                    .external_memory_features
                    .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
        };
        let mut formats = Vec::new();
        for properties in &modifiers {
            if !properties
                .drm_format_modifier_tiling_features
                .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
                || !importable(properties.drm_format_modifier)
            {
                continue;
            }
            for fourcc in [ARGB8888, XRGB8888] {
                formats.push(DmaBufFormat {
                    fourcc,
                    modifier: properties.drm_format_modifier,
                    planes: properties.drm_format_modifier_plane_count,
                });
            }
        }
        formats
    }
}

/// An image a DMA-BUF was imported as, with its memory and view.
pub(crate) struct Import {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

/// The device's clients' GPU buffers: what it can import, what it holds, and what the frame in
/// flight reads.
pub(crate) struct Images {
    capabilities: Capabilities,
    loader: ash::khr::external_memory_fd::Device,
    /// What a slot no layer uses names: a 1×1 image never read.
    dummy: Import,
    imports: HashMap<u64, Import>,
    next: u64,
    /// The imports the submission in flight reads, and those forgotten meanwhile, destroyed once
    /// it completes.
    pub(crate) reading: Vec<u64>,
    forgotten: Vec<u64>,
    /// What each slot of the descriptor set's binding 3 names now.
    bound: Vec<vk::ImageView>,
}

impl Images {
    /// The queue family a client's writes come from: foreign (another device or process), or
    /// external where the device lacks that.
    pub(crate) fn foreign_family(&self) -> u32 {
        if self.capabilities.foreign {
            vk::QUEUE_FAMILY_FOREIGN_EXT
        } else {
            vk::QUEUE_FAMILY_EXTERNAL
        }
    }

    pub(crate) fn image(&self, id: u64) -> Option<vk::Image> {
        self.imports.get(&id).map(|import| import.image)
    }
}

impl VulkanCompositor {
    /// The dummy image, the descriptors every slot starts with, and the import's loader.
    pub(crate) fn build_images(&mut self, capabilities: Capabilities) -> Result<Images, Error> {
        let dummy = self.dummy_image()?;
        let loader = ash::khr::external_memory_fd::Device::new(&self.instance, &self.device);
        let mut images = Images {
            capabilities,
            loader,
            bound: vec![vk::ImageView::null(); IMAGE_SLOTS],
            dummy,
            imports: HashMap::new(),
            next: 0,
            reading: Vec::new(),
            forgotten: Vec::new(),
        };
        self.bind_slots(&mut images, &[]);
        Ok(images)
    }

    /// A 1×1 image in the layout the shader samples, named by every slot no layer uses.
    fn dummy_image(&mut self) -> Result<Import, Error> {
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(FORMAT)
            .extent(vk::Extent3D {
                width: 1,
                height: 1,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: the device is live; the create info outlives the call. Each object is destroyed
        // on an error below, or by `destroy_import` once it is the compositor's.
        unsafe {
            let image = self.device.create_image(&info, None)?;
            let requirements = self.device.get_image_memory_requirements(image);
            let Some(kind) = self.memory_type(
                requirements.memory_type_bits,
                vk::MemoryPropertyFlags::empty(),
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            ) else {
                self.device.destroy_image(image, None);
                return Err(Error("no memory for an image".into()));
            };
            let memory = match self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(kind),
                None,
            ) {
                Ok(memory) => memory,
                Err(error) => {
                    self.device.destroy_image(image, None);
                    return Err(error.into());
                }
            };
            let import = Import {
                image,
                memory,
                view: vk::ImageView::null(),
            };
            if let Err(error) = self.device.bind_image_memory(image, memory, 0) {
                self.destroy_import(import);
                return Err(error.into());
            }
            let view = match self.view(image) {
                Ok(view) => view,
                Err(error) => {
                    self.destroy_import(import);
                    return Err(error);
                }
            };
            let import = Import { view, ..import };
            // Into the layout every slot is sampled in, once.
            let command = self.command;
            let done = (|| -> Result<(), vk::Result> {
                self.device
                    .reset_command_buffer(command, vk::CommandBufferResetFlags::empty())?;
                self.device.begin_command_buffer(
                    command,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )?;
                self.device.cmd_pipeline_barrier(
                    command,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::UNDEFINED)
                        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(image)
                        .subresource_range(color_range())
                        .dst_access_mask(vk::AccessFlags::SHADER_READ)],
                );
                self.device.end_command_buffer(command)?;
                let commands = [command];
                self.device.queue_submit(
                    self.queue,
                    &[vk::SubmitInfo::default().command_buffers(&commands)],
                    self.fence,
                )?;
                self.device.wait_for_fences(&[self.fence], true, u64::MAX)?;
                self.device.reset_fences(&[self.fence])
            })();
            if let Err(error) = done {
                self.destroy_import(import);
                return Err(error.into());
            }
            Ok(import)
        }
    }

    /// A view of `image` as the shader samples it.
    fn view(&self, image: vk::Image) -> Result<vk::ImageView, Error> {
        // SAFETY: the image is live; the create info outlives the call.
        Ok(unsafe {
            self.device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(FORMAT)
                    .subresource_range(color_range()),
                None,
            )
        }?)
    }

    /// Destroys an import's objects (null handles are ignored).
    pub(crate) fn destroy_import(&self, import: Import) {
        // SAFETY: no submission in flight reads it (the caller completed it or it was never
        // submitted); each handle is destroyed once.
        unsafe {
            self.device.destroy_image_view(import.view, None);
            self.device.destroy_image(import.image, None);
            self.device.free_memory(import.memory, None);
        }
    }

    /// Points the descriptor set's binding 3 at `ids`' images, slot by slot, and every other
    /// slot at the dummy image; only slots that change are written.
    pub(crate) fn bind_slots(&self, images: &mut Images, ids: &[u64]) {
        let wanted: Vec<vk::ImageView> = (0..IMAGE_SLOTS)
            .map(|slot| {
                ids.get(slot)
                    .and_then(|id| images.imports.get(id))
                    .map_or(images.dummy.view, |import| import.view)
            })
            .collect();
        let changed: Vec<usize> = (0..IMAGE_SLOTS)
            .filter(|&slot| images.bound[slot] != wanted[slot])
            .collect();
        let infos: Vec<[vk::DescriptorImageInfo; 1]> = changed
            .iter()
            .map(|&slot| {
                [vk::DescriptorImageInfo::default()
                    .image_view(wanted[slot])
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)]
            })
            .collect();
        let writes: Vec<vk::WriteDescriptorSet<'_>> = changed
            .iter()
            .zip(&infos)
            .map(|(&slot, info)| {
                vk::WriteDescriptorSet::default()
                    .dst_set(self.set)
                    .dst_binding(3)
                    .dst_array_element(slot as u32)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(info)
            })
            .collect();
        if !writes.is_empty() {
            // SAFETY: the set is not in use (no submission is pending when a frame binds) and
            // every view is live.
            unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        }
        images.bound = wanted;
    }

    /// The DRM formats and modifiers clients' GPU buffers can be imported in; none when the
    /// device cannot import them (ADR 0031, decision 2).
    pub fn dma_buf_formats(&self) -> &[DmaBufFormat] {
        self.images
            .as_ref()
            .map_or(&[], |images| &images.capabilities.formats)
    }

    /// The device's DRM render node (its `dev_t`), where Vulkan says (`VK_EXT_physical_device_drm`).
    pub fn render_node(&self) -> Option<u64> {
        self.images
            .as_ref()
            .and_then(|images| images.capabilities.node)
    }

    /// Imports a client's DMA-BUF of `width` × `height` in `fourcc` with `modifier`, its planes
    /// in one buffer object, and returns its id for [`crate::Paint::Imported`].
    pub fn import_dma_buf(
        &mut self,
        fourcc: u32,
        modifier: u64,
        width: u32,
        height: u32,
        planes: &[DmaBufPlane<'_>],
    ) -> Result<u64, Error> {
        let Some(images) = self.images.as_ref() else {
            return Err(Error("this device imports no DMA-BUF".into()));
        };
        let format = images
            .capabilities
            .formats
            .iter()
            .find(|format| format.fourcc == fourcc && format.modifier == modifier)
            .copied()
            .ok_or_else(|| {
                Error(format!(
                    "{fourcc:#x} with modifier {modifier:#x} is not imported"
                ))
            })?;
        if planes.len() != format.planes as usize || planes.is_empty() {
            return Err(Error(format!(
                "{} planes for a modifier of {}",
                planes.len(),
                format.planes
            )));
        }
        if width == 0 || height == 0 {
            return Err(Error("an empty buffer".into()));
        }
        // One memory object: every plane in the first's buffer object.
        let inode = |fd: BorrowedFd<'_>| {
            std::fs::File::from(fd.try_clone_to_owned()?)
                .metadata()
                .map(|metadata| std::os::unix::fs::MetadataExt::ino(&metadata))
        };
        let first = inode(planes[0].fd).map_err(|error| Error(format!("a plane: {error}")))?;
        for plane in &planes[1..] {
            if inode(plane.fd).ok() != Some(first) {
                return Err(Error("planes in different DMA-BUFs".into()));
            }
        }
        // A DMA-BUF tells its size by seeking to its end (a duplicate shares the offset, so it
        // is put back).
        let size = {
            use std::io::Seek;
            let mut file = std::fs::File::from(
                planes[0]
                    .fd
                    .try_clone_to_owned()
                    .map_err(|error| Error(format!("a plane: {error}")))?,
            );
            let size = file.seek(std::io::SeekFrom::End(0)).unwrap_or(0);
            let _ = file.seek(std::io::SeekFrom::Start(0));
            size
        };
        let layouts: Vec<vk::SubresourceLayout> = planes
            .iter()
            .map(|plane| vk::SubresourceLayout {
                offset: u64::from(plane.offset),
                size: 0,
                row_pitch: u64::from(plane.stride),
                array_pitch: 0,
                depth_pitch: 0,
            })
            .collect();
        let mut explicit = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
            .drm_format_modifier(modifier)
            .plane_layouts(&layouts);
        let mut external = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(FORMAT)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut explicit)
            .push_next(&mut external);
        let fd: OwnedFd = planes[0]
            .fd
            .try_clone_to_owned()
            .map_err(|error| Error(format!("a plane: {error}")))?;
        // SAFETY: the device is live; the create infos and what they chain outlive each call. The
        // image is destroyed on any error below; the memory import takes the descriptor only when
        // it succeeds.
        unsafe {
            let image = self.device.create_image(&info, None)?;
            let fail = |this: &Self, error: Error| {
                this.device.destroy_image(image, None);
                Err(error)
            };
            let requirements = self.device.get_image_memory_requirements(image);
            if requirements.size > size {
                return fail(
                    self,
                    Error(format!(
                        "the DMA-BUF holds {size} bytes, the image needs {}",
                        requirements.size
                    )),
                );
            }
            let images = self.images.as_ref().expect("checked");
            let mut properties = vk::MemoryFdPropertiesKHR::default();
            if let Err(error) = images.loader.get_memory_fd_properties(
                vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
                fd.as_raw_fd(),
                &mut properties,
            ) {
                return fail(self, error.into());
            }
            let Some(kind) = self.memory_type(
                properties.memory_type_bits & requirements.memory_type_bits,
                vk::MemoryPropertyFlags::empty(),
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            ) else {
                return fail(self, Error("no memory type imports this DMA-BUF".into()));
            };
            let mut import = vk::ImportMemoryFdInfoKHR::default()
                .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
                .fd(fd.as_raw_fd());
            let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
            let memory = match self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(kind)
                    .push_next(&mut import)
                    .push_next(&mut dedicated),
                None,
            ) {
                Ok(memory) => {
                    // The import owns the descriptor now.
                    let _ = fd.into_raw_fd();
                    memory
                }
                Err(error) => return fail(self, error.into()),
            };
            let import = Import {
                image,
                memory,
                view: vk::ImageView::null(),
            };
            if let Err(error) = self.device.bind_image_memory(image, memory, 0) {
                self.destroy_import(import);
                return Err(error.into());
            }
            let view = match self.view(image) {
                Ok(view) => view,
                Err(error) => {
                    self.destroy_import(import);
                    return Err(error);
                }
            };
            let images = self.images.as_mut().expect("checked");
            images.next += 1;
            let id = images.next;
            images.imports.insert(id, Import { view, ..import });
            Ok(id)
        }
    }

    /// The import is no longer used: destroyed now, or once the frame in flight reading it is
    /// done.
    pub fn forget_dma_buf(&mut self, id: u64) {
        let Some(images) = self.images.as_mut() else {
            return;
        };
        if images.reading.contains(&id) {
            images.forgotten.push(id);
            return;
        }
        if let Some(import) = images.imports.remove(&id) {
            // A slot naming it names the dummy again before any frame binds.
            if let Some(slot) = images.bound.iter().position(|view| *view == import.view) {
                images.bound[slot] = vk::ImageView::null();
            }
            self.destroy_import(import);
        }
    }

    /// Whether the frame in flight reads import `id`.
    pub fn reads_dma_buf(&self, id: u64) -> bool {
        self.images
            .as_ref()
            .is_some_and(|images| images.reading.contains(&id))
    }

    /// A copy of the frame in flight's completion as a sync file, where the device exports it
    /// and the frame is not known to be done: what a client's buffer it reads is released after
    /// (ADR 0031, decision 6).
    pub fn completion_sync_file(&self) -> Option<OwnedFd> {
        self.pending_sync_file
            .as_ref()
            .and_then(|fd| fd.try_clone().ok())
    }

    /// The frame in flight's completion as a sync file, to wake a turn when it is done.
    pub fn completion_fd(&self) -> Option<BorrowedFd<'_>> {
        use std::os::fd::AsFd;
        self.pending_sync_file.as_ref().map(|fd| fd.as_fd())
    }

    /// Whether the last submission is done, without waiting: completed (its timestamps read, what
    /// it read released) when it is.
    pub fn poll_completion(&mut self) -> Result<bool, Error> {
        if !self.submitted {
            return Ok(true);
        }
        // SAFETY: the fence belongs to the last submission.
        if unsafe { self.device.get_fence_status(self.fence) }? {
            self.complete()?;
            return Ok(true);
        }
        Ok(false)
    }

    /// The submission in flight completed: what it read is no longer read, and imports forgotten
    /// meanwhile go.
    pub(crate) fn finish_reading(&mut self) {
        let Some(images) = self.images.as_mut() else {
            return;
        };
        images.reading.clear();
        let forgotten = std::mem::take(&mut images.forgotten);
        for id in forgotten {
            self.forget_dma_buf(id);
        }
    }

    /// Destroys every import and the dummy image (the device is idle).
    pub(crate) fn destroy_images(&mut self) {
        if let Some(images) = self.images.take() {
            for (_, import) in images.imports {
                self.destroy_import(import);
            }
            self.destroy_import(images.dummy);
        }
    }
}

pub(crate) fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1)
}
