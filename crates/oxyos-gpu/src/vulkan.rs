//! Composition on a Vulkan device: each layer is a compute dispatch over its rectangle, in
//! order, reading the frame's pixels from one storage buffer and writing the output to another
//! (docs/adr/0004). Both live in device memory where the device has it: pixels arrive through
//! a staging buffer, the background is filled on the device, and the output is copied back into
//! host-cached memory. GPU timestamps measure the composition itself.

#![allow(unsafe_code)]

use super::{shader, shader_images, Layer, Paint};
use ash::vk;
use std::fmt;

mod import;
#[cfg(test)]
pub(crate) use import::best_first;
pub use import::{DmaBufFormat, DmaBufPlane};
use import::{Images, IMAGE_SLOTS};

/// What kind of device composes: a GPU, or a CPU implementation of Vulkan (Mesa's lavapipe),
/// which proves the Vulkan path but not GPU performance or hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Discrete,
    Integrated,
    Virtual,
    Cpu,
    Other,
}

impl From<vk::PhysicalDeviceType> for DeviceKind {
    fn from(kind: vk::PhysicalDeviceType) -> Self {
        match kind {
            vk::PhysicalDeviceType::DISCRETE_GPU => Self::Discrete,
            vk::PhysicalDeviceType::INTEGRATED_GPU => Self::Integrated,
            vk::PhysicalDeviceType::VIRTUAL_GPU => Self::Virtual,
            vk::PhysicalDeviceType::CPU => Self::Cpu,
            _ => Self::Other,
        }
    }
}

/// Where the device holds a layer's pixels at the version they changed from (`Layer::changed`):
/// their offset, and what changed. Rows are copied once per rectangle, never more than the whole
/// layer, so the upload buffer (one copy of every layer) holds them.
fn updatable<'a>(
    resident: &std::collections::HashMap<u64, (u64, u64, u64)>,
    layer: &Layer<'a>,
) -> Option<(u64, &'a [crate::Rect])> {
    let source = layer.source?;
    let (from, changed) = layer.changed?;
    let &(version, offset, length) = resident.get(&source.key)?;
    let words = u64::from(layer.extent.0) * u64::from(layer.extent.1);
    let extent = crate::Rect {
        x: 0,
        y: 0,
        width: layer.extent.0,
        height: layer.extent.1,
    };
    let copied: u64 = changed
        .iter()
        .filter_map(|rect| rect.intersect(&extent))
        .map(|rect| u64::from(rect.width) * u64::from(rect.height))
        .sum();
    (version == from && length == words && copied <= length).then_some((offset, changed))
}

/// Damage clipped to the output as rectangles that do not overlap (`crate::disjoint`), each
/// pixel composed once. Past 32, one rectangle holding them all: each is a dispatch.
fn merged(regions: Vec<crate::Rect>, output: &crate::Rect) -> Vec<crate::Rect> {
    let disjoint = crate::disjoint(&regions, output);
    if disjoint.len() <= 32 {
        return disjoint;
    }
    vec![disjoint[1..]
        .iter()
        .fold(disjoint[0], |all, region| all.bounding(region))]
}

/// A layer as `shaders/compose.wgsl`'s `Layer`: an entry of the frame's layer table.
#[derive(Clone, Copy, Default)]
struct Push {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    src_width: u32,
    src_height: u32,
    src_offset: u32,
    out_width: u32,
    out_height: u32,
    mode: u32,
    color: u32,
    base_x: u32,
    base_y: u32,
    count_x: u32,
    count_y: u32,
    opacity: u32,
    radius: u32,
    shadow_x: i32,
    shadow_y: i32,
    shadow_width: u32,
    shadow_height: u32,
    shadow_radius: u32,
    clip_x: i32,
    clip_y: i32,
    clip_width: u32,
    clip_height: u32,
    clip_radius: u32,
    scale: u32,
    filter: u32,
    /// A client's GPU buffer's slot in the shader's images, plus one; 0: pixels in the source
    /// buffer (docs/adr/0031-gpu-buffers.mdx).
    image: u32,
}

/// A layer table entry's size: 30 words.
const PUSH_BYTES: usize = 30 * 4;

/// The push constants of `shaders/compose.wgsl`'s `Frame`: the region a dispatch composes, the
/// output's size, the background and the layers' count. 8 words, within the 128 bytes every
/// device has.
const FRAME_BYTES: usize = 8 * 4;

fn frame_bytes(
    region: &crate::Rect,
    width: u32,
    height: u32,
    background: u32,
    layers: u32,
) -> [u8; FRAME_BYTES] {
    let words = [
        region.x as u32,
        region.y as u32,
        region.width,
        region.height,
        width,
        height,
        background,
        layers,
    ];
    let mut bytes = [0; FRAME_BYTES];
    for (chunk, word) in bytes.chunks_exact_mut(4).zip(words) {
        chunk.copy_from_slice(&word.to_ne_bytes());
    }
    bytes
}

impl Push {
    /// The whole layer in one dispatch.
    fn whole(self) -> Self {
        Self {
            base_x: 0,
            base_y: 0,
            count_x: self.width,
            count_y: self.height,
            ..self
        }
    }

    /// The part of what it dispatches within `region` (output pixels), if any.
    fn within(self, region: &crate::Rect) -> Option<Self> {
        let part = region.intersect(&crate::Rect {
            x: self.x + self.base_x as i32,
            y: self.y + self.base_y as i32,
            width: self.count_x,
            height: self.count_y,
        })?;
        Some(Self {
            base_x: (part.x - self.x) as u32,
            base_y: (part.y - self.y) as u32,
            count_x: part.width,
            count_y: part.height,
            ..self
        })
    }

    fn bytes(&self) -> [u8; PUSH_BYTES] {
        let words = [
            self.x as u32,
            self.y as u32,
            self.width,
            self.height,
            self.src_width,
            self.src_height,
            self.src_offset,
            self.out_width,
            self.out_height,
            self.mode,
            self.color,
            self.base_x,
            self.base_y,
            self.count_x,
            self.count_y,
            self.opacity,
            self.radius,
            self.shadow_x as u32,
            self.shadow_y as u32,
            self.shadow_width,
            self.shadow_height,
            self.shadow_radius,
            self.clip_x as u32,
            self.clip_y as u32,
            self.clip_width,
            self.clip_height,
            self.clip_radius,
            self.scale,
            self.filter,
            self.image,
        ];
        let mut bytes = [0; PUSH_BYTES];
        for (chunk, word) in bytes.chunks_exact_mut(4).zip(words) {
            chunk.copy_from_slice(&word.to_ne_bytes());
        }
        bytes
    }
}

/// A buffer and its memory; `mapped` is null unless the memory is host-visible and mapped.
struct Buffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: *mut u8,
    size: u64,
    coherent: bool,
}

/// An output buffer exported as DMA-BUF for a display, and whether its ownership was last
/// released to the external queue (the display).
struct Scanout {
    buffer: Buffer,
    width: u32,
    height: u32,
    released: bool,
    /// What changed since this target was last written (frames composed into the others of its
    /// display), or `None` when its content is unknown (never written, or a whole frame since).
    stale: Option<Vec<crate::Rect>>,
    /// The display's targets made together (`add_scanout_targets`), which take turns showing
    /// its frames: a frame's damage concerns only them (docs/adr/0018-several-displays.mdx).
    group: u64,
}

/// A scanout target for a display: its DMA-BUF, size and pitch, XRGB8888 with the linear
/// modifier (rows of `width` pixels, 0x00RRGGBB).
#[cfg(target_os = "linux")]
pub struct ScanoutTarget {
    pub fd: std::os::fd::OwnedFd,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}

/// What a buffer is for, which decides its usage and memory.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// The layers' pixels, read by the shader: device memory, filled by a copy.
    Source,
    /// The output: device memory, filled on the device, written by the shader, copied back.
    Output,
    /// Pixels written by the host for the copy into `Source`.
    Upload,
    /// The output copied back for the host to read: host-cached where possible.
    Readback,
    /// An output exported as DMA-BUF for a display.
    Scanout,
    /// The frame's layer table, written by the host, read by the shader.
    Layers,
}

#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<vk::Result> for Error {
    fn from(result: vk::Result) -> Self {
        Self(format!("Vulkan: {result}"))
    }
}

/// The chosen device and its queue.
/// What a composition writes: the compositor's own output buffer (read back, at once or, deferred,
/// once done) or a scanout target (`keep`: also copied for `read_kept`, and waited for).
#[derive(Clone, Copy)]
enum Destination {
    Output { deferred: bool },
    Scanout { index: usize, keep: bool },
}

struct Opened {
    device: ash::Device,
    queue: vk::Queue,
    family: u32,
    memory: vk::PhysicalDeviceMemoryProperties,
    name: String,
    kind: DeviceKind,
    /// Nanoseconds per timestamp tick, and the queue's valid timestamp bits (0: none).
    timestamp_period: f32,
    timestamp_bits: u32,
    dma_buf: bool,
    /// Whether a semaphore its submissions signal can be exported as a sync file.
    sync_file: bool,
    /// Whether it imports clients' DMA-BUFs and samples them in place (docs/adr/0031): the
    /// formats and modifiers it can, the foreign queue family, its render node.
    images: Option<import::Capabilities>,
}

pub struct VulkanCompositor {
    // Kept alive: the loaded Vulkan library.
    _entry: ash::Entry,
    instance: ash::Instance,
    device: ash::Device,
    queue: vk::Queue,
    memory: vk::PhysicalDeviceMemoryProperties,
    set_layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    /// The composition, from layers in the source buffer alone.
    pipeline: vk::Pipeline,
    /// The same, also sampling clients' GPU buffers in place (ADR 0031), where the device
    /// imports them: used only for frames that sample one, since indexing an array of images
    /// per layer costs every pixel of the frame (null: none).
    pipeline_images: vk::Pipeline,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    commands: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    queries: vk::QueryPool,
    buffers: [Option<Buffer>; 5],
    device_name: String,
    kind: DeviceKind,
    timestamp_period: f32,
    timestamp_bits: u32,
    last_gpu_ms: Option<f64>,
    /// The last frame's host time: copying the layers into the upload buffer, and from
    /// submitting the commands to their fence, in milliseconds.
    last_host_ms: (f64, f64),
    /// The device's time before composing (uploads, the fill) and after it (copies, release),
    /// in milliseconds.
    last_gpu_phases: Option<(f64, f64)>,
    /// Whether the device exports memory as DMA-BUF (Linux), for scanout targets.
    dma_buf: bool,
    /// Output buffers exported for displays (`scanout_targets`, `add_scanout_targets`), by
    /// index; an index stays its target's until it is removed.
    scanout: Vec<Option<Scanout>>,
    /// The group the next targets made together take.
    next_group: u64,
    /// The output buffer the descriptor set's binding 1 names now.
    bound_output: vk::Buffer,
    /// The compute queue's family.
    family: u32,
    /// The size of the frame `compose_to_scanout_kept` copied for `read_kept`, if any.
    kept: Option<(u32, u32)>,
    /// The size of the frame the output buffer holds, if it holds the last one composed.
    output_holds: Option<(u32, u32)>,
    /// Pixels kept in the source buffer by `Source` key: their version, offset and length in
    /// words. Space is taken in order from `source_next` and reclaimed all at once, when the
    /// buffer is full or replaced.
    resident: std::collections::HashMap<u64, (u64, u64, u64)>,
    source_next: u64,
    /// The bytes the last frame uploaded.
    last_uploaded: u64,
    /// Whether the last submission was not waited for yet (`complete`): a frame whose
    /// completion was handed to its display as a sync file (`compose_to_scanout`), or one
    /// composed `deferred`. Every composition, readback and buffer freed first completes it.
    submitted: bool,
    /// With a device that exports them: the semaphore a deferred submission signals, exported as
    /// a sync file, and the loader that exports it. None: every submission is waited for.
    completion: Option<(vk::Semaphore, ash::khr::external_semaphore_fd::Device)>,
    /// The last deferred submission's completion as a sync file, until it is taken.
    completed_when: Option<std::os::fd::OwnedFd>,
    /// The device's time for each frame completed and not yet taken (`take_completed_ms`).
    completed_ms: Vec<f64>,
    /// Clients' GPU buffers imported and sampled in place, where the device can (ADR 0031).
    images: Option<Images>,
    /// A copy of the last deferred submission's completion as a sync file, while it is pending:
    /// what a client's buffer it read is released after (`completion_sync_file`).
    pending_sync_file: Option<std::os::fd::OwnedFd>,
    /// Whether the last submission wrote timestamps (a composition did, a release did not).
    timed: bool,
    /// What the last submission did with clients' GPU buffers: how many it acquired from the
    /// queues that wrote them, how many it released back, and whether it sampled any.
    last_handovers: Handovers,
}

/// What a submission did with clients' GPU buffers (`VulkanCompositor::last_handovers`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Handovers {
    /// Buffers acquired from the queue that wrote them: each a frame of a client's, newly shown.
    pub acquired: usize,
    /// Buffers released back to it: each one the compositor stopped showing.
    pub released: usize,
    /// Whether the composition sampled clients' buffers (the shader that can).
    pub sampled: bool,
}

impl VulkanCompositor {
    /// Opens the GPU that drives the displays (the firmware's boot display first, from sysfs:
    /// `oxyos_gpu_device::display_device`), or Vulkan on the CPU where none does, as
    /// [`Self::on_device`]. A session gives its own card's node to [`Self::on_device`] instead.
    pub fn new() -> Result<Self, Error> {
        Self::on_device(oxyos_gpu_device::display_device(std::path::Path::new(
            oxyos_gpu_device::SYSTEM_ROOT,
        )))
    }

    /// Opens the Vulkan device whose DRM node (render or primary) is `device`, with a compute
    /// queue; for `0`, Vulkan on the CPU. Only that GPU's drivers are loaded: no other GPU is
    /// opened, woken or enumerated, and no other is ever chosen.
    pub fn on_device(device: u64) -> Result<Self, Error> {
        let drivers = oxyos_gpu_device::Drivers::for_device(device);
        let loaded = drivers.load().map_err(|error| Error(error.to_string()))?;
        // SAFETY: loading the system's Vulkan loader runs its initialization; nothing else in
        // this process races with it.
        let entry = unsafe { ash::Entry::load() }
            .map_err(|error| Error(format!("no Vulkan loader: {error}")))?;
        let app = vk::ApplicationInfo::default()
            .application_name(c"oxyos-gpu")
            .api_version(vk::API_VERSION_1_2);
        let names = [oxyos_gpu_device::Loaded::EXTENSION.as_ptr()];
        let mut only = loaded.list();
        // SAFETY: the create info and what it points to outlive the call.
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default()
                    .application_info(&app)
                    .enabled_extension_names(&names)
                    .push_next(&mut only),
                None,
            )
        }
        .map_err(|error| Error(format!("Vulkan with {}: {error}", drivers.describe())))?;
        let opened = match Self::open(&instance, device) {
            Ok(opened) => opened,
            Err(error) => {
                // SAFETY: nothing was created from the instance.
                unsafe { instance.destroy_instance(None) };
                return Err(error);
            }
        };
        // From here every object belongs to `this`: an error drops it, destroying what exists
        // (destroying a null handle does nothing in Vulkan).
        let mut this = Self {
            _entry: entry,
            instance,
            device: opened.device,
            queue: opened.queue,
            memory: opened.memory,
            set_layout: vk::DescriptorSetLayout::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            pipeline_images: vk::Pipeline::null(),
            pool: vk::DescriptorPool::null(),
            set: vk::DescriptorSet::null(),
            commands: vk::CommandPool::null(),
            command: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            queries: vk::QueryPool::null(),
            buffers: [None, None, None, None, None],
            device_name: opened.name,
            kind: opened.kind,
            timestamp_period: opened.timestamp_period,
            timestamp_bits: opened.timestamp_bits,
            last_gpu_ms: None,
            last_host_ms: (0.0, 0.0),
            last_gpu_phases: None,
            dma_buf: opened.dma_buf,
            scanout: Vec::new(),
            next_group: 0,
            bound_output: vk::Buffer::null(),
            family: opened.family,
            kept: None,
            output_holds: None,
            resident: std::collections::HashMap::new(),
            source_next: 0,
            last_uploaded: 0,
            submitted: false,
            completion: None,
            completed_when: None,
            completed_ms: Vec::new(),
            images: None,
            pending_sync_file: None,
            timed: false,
            last_handovers: Handovers::default(),
        };
        this.build(opened.family, opened.images.is_some())?;
        if let Some(capabilities) = opened.images {
            match this.build_images(capabilities) {
                Ok(images) => this.images = Some(images),
                Err(error) => eprintln!("[gpu] clients' GPU buffers are not imported: {error}"),
            }
        }
        if opened.sync_file {
            this.completion = this.completion_semaphore().ok();
        }
        // A device can name DMA-BUF export and still be unable to allocate memory it exports
        // (Mesa's lavapipe without a render node): it exports DMA-BUF only if a buffer can be
        // made and handed out as one, so a session composes on the CPU there instead of failing.
        if this.dma_buf && this.export_probe().is_err() {
            this.dma_buf = false;
        }
        Ok(this)
    }

    /// The semaphore deferred submissions signal, exportable as a sync file, and its loader.
    fn completion_semaphore(
        &self,
    ) -> Result<(vk::Semaphore, ash::khr::external_semaphore_fd::Device), Error> {
        let mut export = vk::ExportSemaphoreCreateInfo::default()
            .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        // SAFETY: the device is live; the create info outlives the call.
        let semaphore = unsafe {
            self.device.create_semaphore(
                &vk::SemaphoreCreateInfo::default().push_next(&mut export),
                None,
            )
        }?;
        let loader = ash::khr::external_semaphore_fd::Device::new(&self.instance, &self.device);
        Ok((semaphore, loader))
    }

    /// Makes one small exportable buffer, exports it as DMA-BUF and frees both.
    fn export_probe(&mut self) -> Result<(), Error> {
        use std::os::fd::FromRawFd;
        let buffer = self.buffer(Role::Scanout, 4096)?;
        let loader = ash::khr::external_memory_fd::Device::new(&self.instance, &self.device);
        let info = vk::MemoryGetFdInfoKHR::default()
            .memory(buffer.memory)
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        // SAFETY: the memory was allocated exportable as DMA-BUF and is live.
        let exported = unsafe { loader.get_memory_fd(&info) }
            // SAFETY: vkGetMemoryFdKHR returns a new descriptor that the caller owns; it is
            // closed at once.
            .map(|fd| drop(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) }));
        self.free(buffer);
        exported.map_err(Error::from)
    }

    /// The device whose DRM node is `device` (for `0`, a CPU implementation) with a compute
    /// queue, and that queue.
    fn open(instance: &ash::Instance, device: u64) -> Result<Opened, Error> {
        // SAFETY: the instance is live.
        let physicals = unsafe { instance.enumerate_physical_devices() }?;
        let (physical, family, timestamp_bits, properties) = physicals
            .into_iter()
            .filter_map(|physical| {
                // SAFETY: `physical` was enumerated from this live instance.
                let available = unsafe { instance.enumerate_device_extension_properties(physical) }
                    .unwrap_or_default();
                let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
                let mut properties = vk::PhysicalDeviceProperties2::default();
                if available.iter().any(|extension| {
                    extension.extension_name_as_c_str() == Ok(ash::ext::physical_device_drm::NAME)
                }) {
                    properties = properties.push_next(&mut drm);
                }
                // SAFETY: `physical` is live; the chain outlives the call.
                unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
                let properties = properties.properties;
                let wanted = if device == 0 {
                    properties.device_type == vk::PhysicalDeviceType::CPU
                } else {
                    oxyos_gpu_device::same_device(&drm, device)
                };
                if !wanted {
                    return None;
                }
                // SAFETY: as above.
                let families =
                    unsafe { instance.get_physical_device_queue_family_properties(physical) };
                let (family, family_properties) = families
                    .iter()
                    .enumerate()
                    .find(|(_, family)| family.queue_flags.contains(vk::QueueFlags::COMPUTE))?;
                Some((
                    physical,
                    family as u32,
                    family_properties.timestamp_valid_bits,
                    properties,
                ))
            })
            .next()
            .ok_or_else(|| {
                let (major, minor) = oxyos_gpu_device::major_minor(device);
                Error(if device == 0 {
                    "no CPU Vulkan device with a compute queue".into()
                } else {
                    format!("no Vulkan device {major}:{minor} with a compute queue")
                })
            })?;
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        // DMA-BUF export, for output buffers a display can scan out (Linux).
        let wanted = [
            ash::khr::external_memory_fd::NAME,
            ash::ext::external_memory_dma_buf::NAME,
        ];
        // SAFETY: `physical` was enumerated from this live instance.
        let available = unsafe { instance.enumerate_device_extension_properties(physical) }?;
        let dma_buf = cfg!(target_os = "linux")
            && wanted.iter().all(|name| {
                available
                    .iter()
                    .any(|extension| extension.extension_name_as_c_str() == Ok(*name))
            });
        let mut extensions: Vec<*const std::ffi::c_char> = if dma_buf {
            wanted.iter().map(|name| name.as_ptr()).collect()
        } else {
            Vec::new()
        };
        // A frame's completion as a sync file, for a display to wait on rather than the
        // compositor's thread (`compose_to_scanout`): a semaphore exported as SYNC_FD.
        let semaphore_fd = ash::khr::external_semaphore_fd::NAME;
        let sync_file = cfg!(target_os = "linux")
            && available
                .iter()
                .any(|extension| extension.extension_name_as_c_str() == Ok(semaphore_fd))
            && {
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
        if sync_file {
            extensions.push(semaphore_fd.as_ptr());
        }
        // Clients' GPU buffers (ADR 0031): DMA-BUFs imported with their modifiers and sampled in
        // place, from an array of images indexed per layer.
        let images = if dma_buf {
            import::Capabilities::of(instance, physical, &available, family)
        } else {
            None
        };
        if let Some(capabilities) = &images {
            for name in capabilities.extensions() {
                if !extensions.contains(&name.as_ptr()) {
                    extensions.push(name.as_ptr());
                }
            }
        }
        let mut indexing = vk::PhysicalDeviceVulkan12Features::default()
            .shader_sampled_image_array_non_uniform_indexing(true);
        let mut info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_extension_names(&extensions);
        if images.is_some() {
            info = info.push_next(&mut indexing);
        }
        // SAFETY: `physical` belongs to the instance; the create info, what it chains and the
        // extension names (static strings) outlive the call.
        let device = unsafe { instance.create_device(physical, &info, None) }?;
        // SAFETY: the device was created with one queue in `family`; `physical` is live.
        let (queue, memory) = unsafe {
            (
                device.get_device_queue(family, 0),
                instance.get_physical_device_memory_properties(physical),
            )
        };
        Ok(Opened {
            device,
            queue,
            family,
            memory,
            // The driver's name for the device, NUL-terminated in a fixed array of `c_char` (signed on
            // x86-64, unsigned on AArch64): ash reads it as a C string either way.
            name: properties
                .device_name_as_c_str()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            kind: properties.device_type.into(),
            timestamp_period: properties.limits.timestamp_period,
            timestamp_bits,
            dma_buf,
            sync_file,
            images,
        })
    }

    /// Creates the pipeline, descriptors, commands, fence and timestamp queries.
    fn build(&mut self, family: u32, images: bool) -> Result<(), Error> {
        let device = &self.device;
        let mut bindings: Vec<vk::DescriptorSetLayoutBinding> = [0, 1, 2]
            .map(|binding| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(binding)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::COMPUTE)
            })
            .to_vec();
        // Clients' GPU buffers, sampled in place (ADR 0031).
        if images {
            bindings.push(
                vk::DescriptorSetLayoutBinding::default()
                    .binding(3)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(IMAGE_SLOTS as u32)
                    .stage_flags(vk::ShaderStageFlags::COMPUTE),
            );
        }
        // SAFETY: the device is live; the create infos outlive each call. Each object is stored
        // in `self` as soon as it exists, so `Drop` destroys it.
        unsafe {
            self.set_layout = device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )?;
            let ranges = [vk::PushConstantRange::default()
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
                .size(FRAME_BYTES as u32)];
            let layouts = [self.set_layout];
            self.pipeline_layout = device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&layouts)
                    .push_constant_ranges(&ranges),
                None,
            )?;
            let layout = self.pipeline_layout;
            let pipeline = |code: &[u32]| -> Result<vk::Pipeline, Error> {
                let module = device.create_shader_module(
                    &vk::ShaderModuleCreateInfo::default().code(code),
                    None,
                )?;
                let stage = vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::COMPUTE)
                    .module(module)
                    .name(c"main");
                let pipelines = device.create_compute_pipelines(
                    vk::PipelineCache::null(),
                    &[vk::ComputePipelineCreateInfo::default()
                        .stage(stage)
                        .layout(layout)],
                    None,
                );
                device.destroy_shader_module(module, None);
                Ok(pipelines.map_err(|(_, result)| Error::from(result))?[0])
            };
            self.pipeline = pipeline(&shader())?;
            if images {
                self.pipeline_images = pipeline(&shader_images())?;
            }
            let mut sizes = vec![vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(3)];
            if images {
                sizes.push(
                    vk::DescriptorPoolSize::default()
                        .ty(vk::DescriptorType::SAMPLED_IMAGE)
                        .descriptor_count(IMAGE_SLOTS as u32),
                );
            }
            self.pool = device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(1)
                    .pool_sizes(&sizes),
                None,
            )?;
            self.set = device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.pool)
                    .set_layouts(&layouts),
            )?[0];
            self.commands = device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?;
            self.command = device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.commands)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0];
            self.fence = device.create_fence(&vk::FenceCreateInfo::default(), None)?;
            if self.timestamp_bits > 0 {
                self.queries = device.create_query_pool(
                    &vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count(4),
                    None,
                )?;
            }
        }
        Ok(())
    }

    /// The device that composes, by its driver's name.
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn device_kind(&self) -> DeviceKind {
        self.kind
    }

    /// Whether the device exports memory as DMA-BUF, so that `scanout_targets` can hand a
    /// display buffers it composes into (Linux only).
    pub fn exports_dma_buf(&self) -> bool {
        self.dma_buf
    }

    /// Whether a frame composed for a display is handed over with its completion as a sync file
    /// (`compose_to_scanout`) rather than waited for.
    pub fn exports_sync_file(&self) -> bool {
        self.completion.is_some()
    }

    /// How long the device spent composing the last frame, in milliseconds, between the end of
    /// its uploads and the end of its last layer (GPU timestamps); `None` where the queue has
    /// no timestamps.
    pub fn last_gpu_ms(&self) -> Option<f64> {
        self.last_gpu_ms
    }

    /// The bytes of pixels the last frame uploaded: those of layers whose `Source` was not on
    /// the device already.
    pub fn last_uploaded_bytes(&self) -> u64 {
        self.last_uploaded
    }

    /// The device's time around the last frame's composition, in milliseconds: before it (the
    /// uploads and the fill) and after it (copies and the release to the display).
    pub fn last_handovers(&self) -> Handovers {
        self.last_handovers
    }

    /// The device's time before the last composition (uploads, acquiring clients' buffers) and
    /// after it (copies, releases), in milliseconds.
    pub fn last_gpu_phases(&self) -> Option<(f64, f64)> {
        self.last_gpu_phases
    }

    /// The last frame's host time, in milliseconds: copying the layers' pixels into the upload
    /// buffer, and the thread's wait for the device: from submission to the fence for a frame
    /// waited for at once, the wait left for a deferred one when it was completed (nothing,
    /// when the device was done by then).
    pub fn last_host_ms(&self) -> (f64, f64) {
        self.last_host_ms
    }

    /// Composes `layers` onto an output of `width` × `height` filled with `background`, as
    /// [`crate::compose_cpu`] does, and returns the output's pixels.
    pub fn compose(
        &mut self,
        width: u32,
        height: u32,
        background: u32,
        layers: &[Layer<'_>],
    ) -> Result<Vec<u32>, Error> {
        Ok(self
            .run(
                width,
                height,
                background,
                layers,
                Destination::Output { deferred: false },
                None,
            )?
            .unwrap_or_default())
    }

    /// Composes as `compose`, returning once the commands are submitted, with their completion
    /// as a sync file when the device gives one; `read_deferred` waits for it and reads the frame.
    pub fn compose_deferred(
        &mut self,
        width: u32,
        height: u32,
        background: u32,
        layers: &[Layer<'_>],
    ) -> Result<Option<std::os::fd::OwnedFd>, Error> {
        self.run(
            width,
            height,
            background,
            layers,
            Destination::Output { deferred: true },
            None,
        )?;
        Ok(self.completed_when.take())
    }

    /// The frame the last `compose_deferred` composed, once the device finished it.
    pub fn read_deferred(&mut self) -> Result<Vec<u32>, Error> {
        let (width, height) = self
            .output_holds
            .ok_or_else(|| Error("no frame was composed".into()))?;
        self.complete()?;
        Ok(self.read_back(width as usize * height as usize)?)
    }

    /// Waits for the last submission, if it was not waited for yet, and reads its timestamps.
    fn complete(&mut self) -> Result<(), Error> {
        if !self.submitted {
            return Ok(());
        }
        let waiting = std::time::Instant::now();
        // SAFETY: the fence was given to the last submission, which is pending or done.
        unsafe {
            self.device.wait_for_fences(&[self.fence], true, u64::MAX)?;
            self.device.reset_fences(&[self.fence])?;
        }
        self.submitted = false;
        self.pending_sync_file = None;
        self.finish_reading();
        self.last_host_ms.1 = waiting.elapsed().as_secs_f64() * 1e3;
        if self.timed {
            self.read_timestamps()?;
        }
        Ok(())
    }

    /// The same, composing only `damage`, what changed since the previous frame composed here
    /// at this size: the rest of the output is that frame's. A whole frame is composed when
    /// there was none.
    pub fn compose_damaged(
        &mut self,
        width: u32,
        height: u32,
        background: u32,
        layers: &[Layer<'_>],
        damage: &[crate::Rect],
    ) -> Result<Vec<u32>, Error> {
        Ok(self
            .run(
                width,
                height,
                background,
                layers,
                Destination::Output { deferred: false },
                Some(damage),
            )?
            .unwrap_or_default())
    }

    /// Composes into scanout target `index` (`scanout_targets`) for a display to show; nothing
    /// is read back. With a device that exports sync files, returns once the commands are
    /// submitted, with the frame's completion as a sync file for the display to wait on (an
    /// atomic commit's IN_FENCE_FD): the caller's thread never waits for the device. Otherwise
    /// returns once the device has finished writing it, and `None`. With `damage`, what
    /// changed since the previous frame (on any target), only that and what changed since this
    /// target was last written are composed; without it, the whole frame.
    pub fn compose_to_scanout(
        &mut self,
        index: usize,
        background: u32,
        layers: &[Layer<'_>],
        damage: Option<&[crate::Rect]>,
    ) -> Result<Option<std::os::fd::OwnedFd>, Error> {
        let target = self.target(index)?;
        let (width, height) = (target.width, target.height);
        self.run(
            width,
            height,
            background,
            layers,
            Destination::Scanout { index, keep: false },
            damage,
        )?;
        Ok(self.completed_when.take())
    }

    /// The same, also copying the frame (in the same submission, before the display has it) for
    /// [`read_kept`](Self::read_kept): a caller that checks what was shown reads it after the
    /// display took it, so the check never delays the frame.
    pub fn compose_to_scanout_kept(
        &mut self,
        index: usize,
        background: u32,
        layers: &[Layer<'_>],
        damage: Option<&[crate::Rect]>,
    ) -> Result<(), Error> {
        let target = self.target(index)?;
        let (width, height) = (target.width, target.height);
        self.run(
            width,
            height,
            background,
            layers,
            Destination::Scanout { index, keep: true },
            damage,
        )?;
        self.kept = Some((width, height));
        Ok(())
    }

    fn target(&self, index: usize) -> Result<&Scanout, Error> {
        self.scanout
            .get(index)
            .and_then(Option::as_ref)
            .ok_or_else(|| Error(format!("no scanout target {index}")))
    }

    /// The frame the last [`compose_to_scanout_kept`](Self::compose_to_scanout_kept) copied,
    /// row by row at the scanout target's size; it is read once.
    pub fn read_kept(&mut self) -> Result<Vec<u32>, Error> {
        self.complete()?;
        let (width, height) = self
            .kept
            .take()
            .ok_or_else(|| Error("no frame was kept".into()))?;
        Ok(self.read_back(width as usize * height as usize)?)
    }

    /// The readback buffer's first `words` pixels, after a completed submission copied them.
    fn read_back(&self, words: usize) -> Result<Vec<u32>, vk::Result> {
        let readback = self.buffers[3].as_ref().expect("reserved");
        if !readback.coherent {
            // SAFETY: the memory is mapped and live; the submission that wrote it completed.
            unsafe {
                self.device
                    .invalidate_mapped_memory_ranges(&[vk::MappedMemoryRange::default()
                        .memory(readback.memory)
                        .size(vk::WHOLE_SIZE)])?
            };
        }
        let mut out = vec![0u32; words];
        // SAFETY: the copy's writes are visible (fence waited for, memory invalidated), and
        // `readback` is mapped for at least `words` pixels (reserved for the output's size).
        unsafe {
            std::ptr::copy_nonoverlapping(
                readback.mapped,
                out.as_mut_ptr().cast::<u8>(),
                words * 4,
            );
        }
        Ok(out)
    }

    fn run(
        &mut self,
        width: u32,
        height: u32,
        background: u32,
        layers: &[Layer<'_>],
        into: Destination,
        damage: Option<&[crate::Rect]>,
    ) -> Result<Option<Vec<u32>>, Error> {
        // A scanout target (and whether its frame is also kept), and whether the frame is
        // returned once submitted rather than once done: a display's only with a sync file to
        // wait on, since the display shows the buffer as soon as it has it.
        let (scanout, deferred) = match into {
            Destination::Output { deferred } => (None, deferred),
            Destination::Scanout { index, keep } => {
                (Some((index, keep)), !keep && self.completion.is_some())
            }
        };
        // The last submission's buffers, command buffer and descriptors are reused: it is done
        // first (a frame shown since: it was, long ago).
        self.complete()?;
        self.completed_when = None;
        let output = crate::Rect {
            x: 0,
            y: 0,
            width,
            height,
        };
        // What to compose: the damage and what changed since the target held a frame, or the
        // whole output (`None`) when its content is unknown or the damage is.
        let known = match scanout {
            Some((index, _)) => self
                .scanout
                .get(index)
                .and_then(Option::as_ref)
                .and_then(|target| target.stale.clone()),
            None => (self.output_holds == Some((width, height))).then(Vec::new),
        };
        let regions = match (known, damage) {
            (Some(mut regions), Some(damage)) => {
                regions.extend_from_slice(damage);
                Some(merged(regions, &output))
            }
            _ => None,
        };
        // A scanout target, and whether its frame is also copied for `read_kept`.
        let (scanout, keep) = match scanout {
            Some((index, keep)) => (Some(index), keep),
            None => (None, false),
        };
        // A new submission overwrites the readback buffer.
        self.kept = None;
        // A client's GPU buffer this device holds is sampled in place (ADR 0031); one it does
        // not is drawn from the pixels the CPU read of it, if any.
        let held = |image: u64| {
            self.images
                .as_ref()
                .is_some_and(|images| images.image(image).is_some())
        };
        let owned: Vec<Layer<'_>> = layers
            .iter()
            .filter(|layer| layer.valid())
            .filter_map(|layer| match layer.paint {
                Paint::Imported { image, .. } if held(image) => Some(*layer),
                Paint::Imported { .. } => layer.on_cpu(),
                _ => Some(*layer),
            })
            .collect();
        let layers: Vec<&Layer<'_>> = owned.iter().collect();
        // The images the frame samples, a slot each.
        let mut slots: Vec<u64> = Vec::new();
        for layer in &layers {
            if let Paint::Imported { image, .. } = layer.paint {
                if !slots.contains(&image) && slots.len() < IMAGE_SLOTS {
                    slots.push(image);
                }
            }
        }
        let words = |layer: &Layer<'_>| u64::from(layer.extent.0) * u64::from(layer.extent.1);
        let uploads = |layer: &Layer<'_>| {
            layer.pixels().is_some() && !matches!(layer.paint, Paint::Imported { .. })
        };
        let frame_words: u64 = layers
            .iter()
            .filter(|layer| uploads(layer))
            .map(|layer| words(layer))
            .sum();
        let out_words = width as usize * height as usize;
        if out_words == 0 {
            return Ok(None);
        }
        let out_bytes = out_words as u64 * 4;
        // The source buffer holds two frames' pixels, so kept ones leave room for new ones.
        let source = self.buffers[0].as_ref().map(|buffer| buffer.buffer);
        self.reserve(
            (frame_words * 8).max(4),
            (frame_words * 4).max(4),
            out_bytes,
            (layers.len() * PUSH_BYTES).max(PUSH_BYTES) as u64,
        )?;
        if self.buffers[0].as_ref().map(|buffer| buffer.buffer) != source {
            // A new source buffer holds no pixels.
            self.resident.clear();
            self.source_next = 0;
        }
        let capacity = self.buffers[0].as_ref().expect("reserved").size / 4;
        let kept = |resident: &std::collections::HashMap<u64, (u64, u64, u64)>,
                    layer: &Layer<'_>| {
            let source = layer.source?;
            let &(version, offset, length) = resident.get(&source.key)?;
            (version == source.version && length == words(layer)).then_some(offset)
        };
        let new_words: u64 = layers
            .iter()
            .filter(|layer| {
                uploads(layer)
                    && kept(&self.resident, layer).is_none()
                    && updatable(&self.resident, layer).is_none()
            })
            .map(|layer| words(layer))
            .sum();
        if self.source_next + new_words > capacity {
            // Full: every layer is uploaded again, from the start.
            self.resident.clear();
            self.source_next = 0;
        }
        let target = match scanout {
            Some(index) => self.target(index)?.buffer.buffer,
            None => self.buffers[1].as_ref().expect("reserved").buffer,
        };
        // A scanout buffer the display had comes back to this queue before it is written.
        let acquire = match scanout {
            Some(index) => self.target(index)?.released,
            None => false,
        };
        let family = self.family;
        self.bind_output(target);
        let mut images = self.images.take();
        if let Some(images) = &mut images {
            self.bind_slots(images, &slots);
        }
        self.images = images;
        // Clients' GPU buffers change hands as the protocol's buffers do (ADR 0031, decision
        // 6): one is acquired from the queue that wrote it when a frame first samples it (a frame
        // of the client's, newly latched: a buffer is written again only once released), held
        // while it is shown, and released once the compositor lets it go (`release_dma_buf`),
        // never per composition. On a driver keeping compressed or tiled images (Intel's), each
        // hand-over can cost a pass over the whole image.
        let (acquiring, releasing) = self
            .images
            .as_ref()
            .map_or((Vec::new(), Vec::new()), |images| images.handovers(&slots));
        let foreign = self
            .images
            .as_ref()
            .map_or(vk::QUEUE_FAMILY_EXTERNAL, |images| images.foreign_family());
        let pipeline = if slots.is_empty() || self.pipeline_images == vk::Pipeline::null() {
            self.pipeline
        } else {
            self.pipeline_images
        };
        let [Some(source), _, Some(upload), Some(readback), Some(table_buffer)] = &self.buffers
        else {
            unreachable!("reserved");
        };

        let copying = std::time::Instant::now();
        // Each layer's pixels: where the source buffer keeps them, or copied into the upload
        // buffer and from there to their place in the source buffer.
        let mut layer_pushes = Vec::with_capacity(layers.len());
        let mut copies = Vec::new();
        let mut uploaded = 0u64;
        for layer in &layers {
            let mut push = Push {
                x: layer.x,
                y: layer.y,
                width: layer.width,
                height: layer.height,
                src_width: layer.extent.0,
                src_height: layer.extent.1,
                out_width: width,
                out_height: height,
                opacity: u32::from(layer.opacity),
                radius: layer.corner_radius(),
                // 1: filtered (`Layer::filtered`); else nearest neighbour.
                filter: u32::from(layer.filtered()),
                ..Push::default()
            };
            match layer.paint {
                Paint::Fill(color) => {
                    push.mode = 2;
                    push.color = color;
                }
                // Sampled in place from its slot (held: `owned` kept it only then).
                Paint::Imported { image, opaque, .. } => {
                    let Some(slot) = slots.iter().position(|&id| id == image) else {
                        // Past the frame's slots: not drawn.
                        layer_pushes.push(None);
                        continue;
                    };
                    push.mode = if opaque { 0 } else { 1 };
                    push.image = slot as u32 + 1;
                }
                Paint::Opaque(pixels) | Paint::Premultiplied(pixels) | Paint::Shadow(pixels, _) => {
                    push.mode = match layer.paint {
                        Paint::Opaque(_) => 0,
                        Paint::Premultiplied(_) => 1,
                        _ => 3,
                    };
                    if let Paint::Shadow(_, shadow) = layer.paint {
                        let (shape, clip) = (shadow.shape, shadow.clip);
                        push.shadow_x = shape.x;
                        push.shadow_y = shape.y;
                        push.shadow_width = shape.width;
                        push.shadow_height = shape.height;
                        push.shadow_radius =
                            crate::shape::radius_on(shadow.radius, shape.width, shape.height);
                        push.clip_x = clip.x;
                        push.clip_y = clip.y;
                        push.clip_width = clip.width;
                        push.clip_height = clip.height;
                        push.clip_radius =
                            crate::shape::radius_on(shadow.clip_radius, clip.width, clip.height);
                        push.scale = shadow.scale;
                    }
                    let length = words(layer);
                    let updated = updatable(&self.resident, layer);
                    let offset = match (kept(&self.resident, layer), updated) {
                        (Some(offset), _) => offset,
                        (None, Some((offset, changed))) => {
                            let extent = crate::Rect {
                                x: 0,
                                y: 0,
                                width: layer.extent.0,
                                height: layer.extent.1,
                            };
                            let row = u64::from(layer.extent.0);
                            for rect in changed.iter().filter_map(|rect| rect.intersect(&extent)) {
                                for y in rect.y as u64..rect.y as u64 + u64::from(rect.height) {
                                    let at = y * row + rect.x as u64;
                                    let count = u64::from(rect.width);
                                    // SAFETY: `upload` is mapped for at least `frame_words` words,
                                    // and a frame copies no more than its layers' pixels; this row
                                    // lies within `pixels` (`Layer::valid`, clipped to the extent).
                                    unsafe {
                                        std::ptr::copy_nonoverlapping(
                                            pixels.as_ptr().add(at as usize * 4),
                                            upload.mapped.add(uploaded as usize * 4),
                                            count as usize * 4,
                                        );
                                    }
                                    copies.push(
                                        vk::BufferCopy::default()
                                            .src_offset(uploaded * 4)
                                            .dst_offset((offset + at) * 4)
                                            .size(count * 4),
                                    );
                                    uploaded += count;
                                }
                            }
                            let source = layer.source.expect("updatable layers have a source");
                            self.resident
                                .insert(source.key, (source.version, offset, length));
                            offset
                        }
                        (None, None) => {
                            let offset = self.source_next;
                            self.source_next += length;
                            // SAFETY: `upload` is mapped for at least `frame_words` words and
                            // the frame uploads no more; `pixels` holds at least `length` words
                            // (`Layer::valid`).
                            unsafe {
                                std::ptr::copy_nonoverlapping(
                                    pixels.as_ptr(),
                                    upload.mapped.add(uploaded as usize * 4),
                                    length as usize * 4,
                                );
                            }
                            copies.push(
                                vk::BufferCopy::default()
                                    .src_offset(uploaded * 4)
                                    .dst_offset(offset * 4)
                                    .size(length * 4),
                            );
                            uploaded += length;
                            if let Some(source) = layer.source {
                                self.resident
                                    .insert(source.key, (source.version, offset, length));
                            }
                            offset
                        }
                    };
                    push.src_offset = offset as u32;
                }
            }
            // The whole layer, or the part of it within its clip (none: nothing is dispatched).
            let push = push.whole();
            layer_pushes.push(match layer.clip {
                Some(clip) => push.within(&clip),
                None => Some(push),
            });
        }
        self.last_uploaded = uploaded * 4;
        self.last_host_ms.0 = copying.elapsed().as_secs_f64() * 1e3;
        // The layers drawn, bottom to top, into the table the shader reads; and what it composes:
        // the whole output, or each damaged region (each pixel of it once, every layer over it).
        let table: Vec<Push> = layer_pushes.iter().flatten().copied().collect();
        let regions = regions.unwrap_or_else(|| vec![output]);
        let words: Vec<u8> = table.iter().flat_map(|push| push.bytes()).collect();
        // SAFETY: the table buffer is mapped, coherent and holds at least a layer per layer
        // (`reserve`); the last submission, which read it, is done (`complete`).
        unsafe {
            std::ptr::copy_nonoverlapping(words.as_ptr(), table_buffer.mapped, words.len());
        }
        let timing = self.queries != vk::QueryPool::null();

        // SAFETY: every handle is live and owned by this compositor; the command buffer is not in
        // use (the previous submission's fence was waited for); every recorded range lies within
        // its buffer.
        unsafe {
            let device = &self.device;
            let command = self.command;
            let barrier = |src: vk::AccessFlags, dst: vk::AccessFlags, from, to| {
                device.cmd_pipeline_barrier(
                    command,
                    from,
                    to,
                    vk::DependencyFlags::empty(),
                    &[vk::MemoryBarrier::default()
                        .src_access_mask(src)
                        .dst_access_mask(dst)],
                    &[],
                    &[],
                );
            };
            device.reset_command_buffer(command, vk::CommandBufferResetFlags::empty())?;
            device.begin_command_buffer(
                command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            if timing {
                device.cmd_reset_query_pool(command, self.queries, 0, 4);
                // The start of the commands (query 2): uploads and the fill follow.
                device.cmd_write_timestamp(
                    command,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    self.queries,
                    2,
                );
            }
            let ownership =
                |from: u32, to: u32, src: vk::AccessFlags, dst: vk::AccessFlags, before, after| {
                    device.cmd_pipeline_barrier(
                        command,
                        before,
                        after,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[vk::BufferMemoryBarrier::default()
                            .src_access_mask(src)
                            .dst_access_mask(dst)
                            .src_queue_family_index(from)
                            .dst_queue_family_index(to)
                            .buffer(target)
                            .size(vk::WHOLE_SIZE)],
                        &[],
                    );
                };
            if acquire {
                ownership(
                    vk::QUEUE_FAMILY_EXTERNAL,
                    family,
                    vk::AccessFlags::empty(),
                    vk::AccessFlags::TRANSFER_WRITE,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                );
            }
            if !copies.is_empty() {
                device.cmd_copy_buffer(command, upload.buffer, source.buffer, &copies);
            }
            // Clients' GPU buffers newly shown, acquired before the shader samples them.
            let images: Vec<vk::Image> = acquiring.iter().map(|(_, image)| *image).collect();
            import::handover(device, command, &images, true, foreign, family);
            barrier(
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::COMPUTE_SHADER,
            );
            if timing {
                device.cmd_write_timestamp(
                    command,
                    vk::PipelineStageFlags::TRANSFER,
                    self.queries,
                    0,
                );
            }
            device.cmd_bind_pipeline(command, vk::PipelineBindPoint::COMPUTE, pipeline);
            device.cmd_bind_descriptor_sets(
                command,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                &[self.set],
                &[],
            );
            for region in &regions {
                device.cmd_push_constants(
                    command,
                    self.pipeline_layout,
                    vk::ShaderStageFlags::COMPUTE,
                    0,
                    &frame_bytes(region, width, height, background, table.len() as u32),
                );
                device.cmd_dispatch(
                    command,
                    region.width.div_ceil(8),
                    region.height.div_ceil(8),
                    1,
                );
                // Regions do not overlap and nothing reads the output: they run together.
            }
            // Those the compositor let go, released once nothing reads them.
            let images: Vec<vk::Image> = releasing.iter().map(|(_, image)| *image).collect();
            import::handover(device, command, &images, false, foreign, family);
            if timing {
                device.cmd_write_timestamp(
                    command,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    self.queries,
                    1,
                );
            }
            if scanout.is_some() {
                if keep {
                    barrier(
                        vk::AccessFlags::SHADER_WRITE,
                        vk::AccessFlags::TRANSFER_READ,
                        vk::PipelineStageFlags::COMPUTE_SHADER,
                        vk::PipelineStageFlags::TRANSFER,
                    );
                    device.cmd_copy_buffer(
                        command,
                        target,
                        readback.buffer,
                        &[vk::BufferCopy::default().size(out_bytes)],
                    );
                    barrier(
                        vk::AccessFlags::TRANSFER_WRITE,
                        vk::AccessFlags::HOST_READ,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::HOST,
                    );
                }
                // The display reads it next: release it to the external queue, after the
                // shader's writes (and the copy's reads).
                ownership(
                    family,
                    vk::QUEUE_FAMILY_EXTERNAL,
                    vk::AccessFlags::SHADER_WRITE,
                    vk::AccessFlags::empty(),
                    vk::PipelineStageFlags::COMPUTE_SHADER | vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                );
            } else {
                barrier(
                    vk::AccessFlags::SHADER_WRITE,
                    vk::AccessFlags::TRANSFER_READ,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::PipelineStageFlags::TRANSFER,
                );
                device.cmd_copy_buffer(
                    command,
                    target,
                    readback.buffer,
                    &[vk::BufferCopy::default().size(out_bytes)],
                );
                barrier(
                    vk::AccessFlags::TRANSFER_WRITE,
                    vk::AccessFlags::HOST_READ,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::HOST,
                );
            }
            if timing {
                // The end of the commands (query 3), after any copy and ownership release.
                device.cmd_write_timestamp(
                    command,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                    self.queries,
                    3,
                );
            }
            device.end_command_buffer(command)?;
            let commands = [command];
            // Deferred, with a device that exports its completion: the submission also signals
            // the completion semaphore, exported as a sync file for whoever waits on the frame.
            let signal: Vec<vk::Semaphore> = self
                .completion
                .as_ref()
                .filter(|_| deferred)
                .map(|(semaphore, _)| *semaphore)
                .into_iter()
                .collect();
            device.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default()
                    .command_buffers(&commands)
                    .signal_semaphores(&signal)],
                self.fence,
            )?;
            self.submitted = true;
        }
        self.timed = timing;
        self.last_handovers = Handovers {
            acquired: acquiring.len(),
            released: releasing.len(),
            sampled: !slots.is_empty(),
        };
        if let Some(images) = self.images.as_mut() {
            images.handed_over(&acquiring, &releasing);
            // What it reads, and what it releases: neither goes back to its client before it is
            // done.
            images.reading = slots;
            images.reading.extend(releasing.iter().map(|(id, _)| *id));
        }
        if let (Some(&semaphore), Some((_, loader))) = (
            self.completion
                .as_ref()
                .filter(|_| deferred)
                .map(|(semaphore, _)| semaphore),
            self.completion.as_ref(),
        ) {
            use std::os::fd::FromRawFd;
            let info = vk::SemaphoreGetFdInfoKHR::default()
                .semaphore(semaphore)
                .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
            // SAFETY: the semaphore has a signal operation pending (the submission above); the
            // export unsignals it for the next one.
            match unsafe { loader.get_semaphore_fd(&info) } {
                Ok(fd) if fd >= 0 => {
                    // SAFETY: vkGetSemaphoreFdKHR returns a new descriptor the caller owns.
                    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
                    self.pending_sync_file = fd.try_clone().ok();
                    self.completed_when = Some(fd);
                }
                // -1: the submission is done already; nothing to wait on.
                Ok(_) => {}
                Err(error) => {
                    // The semaphore stays signaled: never used again. Every frame is waited for.
                    eprintln!(
                        "[gpu] a frame's completion as a sync file: {error}; frames are waited for"
                    );
                    self.complete()?;
                    let (semaphore, _) = self.completion.take().expect("exporting from it");
                    // SAFETY: its signal operation completed with the submission waited for.
                    unsafe { self.device.destroy_semaphore(semaphore, None) };
                }
            }
        }
        if !deferred {
            self.complete()?;
        }
        if let Some(index) = scanout {
            let group = {
                let target = self.scanout[index].as_mut().expect("composed into");
                target.released = true;
                target.group
            };
            // This target holds the frame now; the others of its display lack what changed in it.
            let others = self
                .scanout
                .iter_mut()
                .enumerate()
                .filter_map(|(other, target)| {
                    target
                        .as_mut()
                        .filter(|target| target.group == group)
                        .map(|target| (other, target))
                });
            for (other, target) in others {
                target.stale = if other == index {
                    Some(Vec::new())
                } else {
                    match (target.stale.take(), damage) {
                        (Some(mut stale), Some(damage)) => {
                            stale.extend_from_slice(damage);
                            Some(merged(stale, &output))
                        }
                        _ => None,
                    }
                };
            }
            return Ok(None);
        }
        self.output_holds = Some((width, height));
        if deferred {
            // Read once it is done (`read_deferred`).
            return Ok(None);
        }
        Ok(Some(self.read_back(out_words)?))
    }

    /// The last submission's timestamps (once it completed): the device's time composing it and
    /// around that (`last_gpu_ms`, `last_gpu_phases`).
    fn read_timestamps(&mut self) -> Result<(), Error> {
        self.last_gpu_ms = if self.queries != vk::QueryPool::null() {
            let mut ticks = [0u64; 4];
            // SAFETY: the four queries were written by the completed submission.
            unsafe {
                self.device.get_query_pool_results(
                    self.queries,
                    0,
                    &mut ticks,
                    vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
                )
            }?;
            let mask = if self.timestamp_bits >= 64 {
                u64::MAX
            } else {
                (1u64 << self.timestamp_bits) - 1
            };
            let ms = |from: usize, to: usize| {
                let elapsed = (ticks[to] & mask).wrapping_sub(ticks[from] & mask) & mask;
                elapsed as f64 * f64::from(self.timestamp_period) / 1e6
            };
            self.last_gpu_phases = Some((ms(2, 0), ms(1, 3)));
            Some(ms(0, 1))
        } else {
            None
        };
        if let Some(ms) = self.last_gpu_ms {
            // Kept until taken, a few seconds' frames at most.
            if self.completed_ms.len() >= 1024 {
                self.completed_ms.remove(0);
            }
            self.completed_ms.push(ms);
        }
        Ok(())
    }

    /// The device's time composing each frame completed since the last call (GPU timestamps;
    /// a frame handed to its display completes when the next is composed), in ms.
    pub fn take_completed_ms(&mut self) -> Vec<f64> {
        std::mem::take(&mut self.completed_ms)
    }

    /// Points the descriptor set's output binding at `buffer`.
    fn bind_output(&mut self, buffer: vk::Buffer) {
        if self.bound_output == buffer {
            return;
        }
        let info = [vk::DescriptorBufferInfo::default()
            .buffer(buffer)
            .offset(0)
            .range(vk::WHOLE_SIZE)];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(self.set)
            .dst_binding(1)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&info);
        // SAFETY: the set is not in use (no submission is pending) and the buffer is live.
        unsafe { self.device.update_descriptor_sets(&[write], &[]) };
        self.bound_output = buffer;
    }

    /// Creates `count` output buffers of `width` × `height` that a display can scan out, and
    /// returns their DMA-BUFs (replacing every earlier one): targets 0 to `count - 1`. Needs a
    /// device that exports DMA-BUF.
    #[cfg(target_os = "linux")]
    pub fn scanout_targets(
        &mut self,
        count: usize,
        width: u32,
        height: u32,
    ) -> Result<Vec<ScanoutTarget>, Error> {
        let indices: Vec<usize> = (0..self.scanout.len()).collect();
        self.remove_scanout_targets(&indices);
        self.scanout.clear();
        Ok(self
            .add_scanout_targets(count, width, height)?
            .into_iter()
            .map(|(_, target)| target)
            .collect())
    }

    /// Creates `count` more output buffers of `width` × `height` for one more display, which take
    /// turns showing its frames, and returns each one's index (for `compose_to_scanout`) and
    /// DMA-BUF. The other displays' targets are kept, at their indices.
    #[cfg(target_os = "linux")]
    pub fn add_scanout_targets(
        &mut self,
        count: usize,
        width: u32,
        height: u32,
    ) -> Result<Vec<(usize, ScanoutTarget)>, Error> {
        use std::os::fd::FromRawFd;
        if !self.dma_buf {
            return Err(Error("the device cannot export DMA-BUF".into()));
        }
        let group = self.next_group;
        self.next_group += 1;
        let loader = ash::khr::external_memory_fd::Device::new(&self.instance, &self.device);
        let mut targets: Vec<(usize, ScanoutTarget)> = Vec::with_capacity(count);
        for _ in 0..count {
            let made = self
                .buffer(Role::Scanout, u64::from(width) * u64::from(height) * 4)
                .and_then(|buffer| {
                    let info = vk::MemoryGetFdInfoKHR::default()
                        .memory(buffer.memory)
                        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
                    // SAFETY: the memory was allocated exportable as DMA-BUF and is live.
                    match unsafe { loader.get_memory_fd(&info) } {
                        // SAFETY: vkGetMemoryFdKHR returns a new descriptor that the caller owns.
                        Ok(fd) => Ok((buffer, unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) })),
                        Err(error) => {
                            self.free(buffer);
                            Err(error.into())
                        }
                    }
                });
            let (buffer, fd) = match made {
                Ok(made) => made,
                Err(error) => {
                    // None of the display's targets are kept without the others.
                    let made: Vec<usize> = targets.iter().map(|(index, _)| *index).collect();
                    self.remove_scanout_targets(&made);
                    return Err(error);
                }
            };
            let target = Scanout {
                buffer,
                width,
                height,
                released: false,
                stale: None,
                group,
            };
            let index = match self.scanout.iter().position(Option::is_none) {
                Some(free) => {
                    self.scanout[free] = Some(target);
                    free
                }
                None => {
                    self.scanout.push(Some(target));
                    self.scanout.len() - 1
                }
            };
            targets.push((
                index,
                ScanoutTarget {
                    fd,
                    width,
                    height,
                    pitch: width * 4,
                },
            ));
        }
        Ok(targets)
    }

    /// Frees the scanout targets at `indices` (a display that went away); their indices may be
    /// given to targets made later.
    /// What scanout targets `indices` hold is no longer known (the display showed something else
    /// meanwhile, a client's buffer scanned out directly): each is composed whole next.
    pub fn forget_scanout_contents(&mut self, indices: &[usize]) {
        for &index in indices {
            if let Some(target) = self.scanout.get_mut(index).and_then(Option::as_mut) {
                target.stale = None;
            }
        }
    }

    pub fn remove_scanout_targets(&mut self, indices: &[usize]) {
        // A frame on its way into one of them is done first.
        if let Err(error) = self.complete() {
            eprintln!("[gpu] waiting for the last frame: {error}");
        }
        for &index in indices {
            if let Some(target) = self.scanout.get_mut(index).and_then(Option::take) {
                if self.bound_output == target.buffer.buffer {
                    self.bound_output = vk::Buffer::null();
                }
                self.free(target.buffer);
            }
        }
    }

    /// Makes the buffers at least this large, binding new device buffers to the set.
    fn reserve(
        &mut self,
        source_bytes: u64,
        upload_bytes: u64,
        out_bytes: u64,
        layers_bytes: u64,
    ) -> Result<(), Error> {
        let mut rebind = false;
        for (slot, role, bytes) in [
            (0, Role::Source, source_bytes),
            (1, Role::Output, out_bytes),
            (2, Role::Upload, upload_bytes),
            (3, Role::Readback, out_bytes),
            (4, Role::Layers, layers_bytes),
        ] {
            if self.buffers[slot]
                .as_ref()
                .is_some_and(|buffer| buffer.size >= bytes)
            {
                continue;
            }
            let buffer = self.buffer(role, bytes.next_power_of_two())?;
            if let Some(old) = self.buffers[slot].replace(buffer) {
                self.free(old);
            }
            rebind |= slot < 2 || slot == 4;
            if slot == 1 {
                // A new output buffer holds no frame.
                self.output_holds = None;
            }
        }
        if rebind {
            let [Some(source), Some(output), _, _, Some(table)] = &self.buffers else {
                unreachable!("reserved");
            };
            let infos = [source, output, table].map(|buffer| {
                [vk::DescriptorBufferInfo::default()
                    .buffer(buffer.buffer)
                    .offset(0)
                    .range(vk::WHOLE_SIZE)]
            });
            let writes = [0, 1, 2].map(|binding| {
                vk::WriteDescriptorSet::default()
                    .dst_set(self.set)
                    .dst_binding(binding)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&infos[binding as usize])
            });
            // SAFETY: the set is not in use (no submission is pending) and the buffers are live.
            unsafe { self.device.update_descriptor_sets(&writes, &[]) };
            self.bound_output = output.buffer;
        }
        Ok(())
    }

    /// A memory type allowed by `bits` with `required` flags, preferring one with `preferred`.
    fn memory_type(
        &self,
        bits: u32,
        required: vk::MemoryPropertyFlags,
        preferred: vk::MemoryPropertyFlags,
    ) -> Option<u32> {
        let types = &self.memory.memory_types[..self.memory.memory_type_count as usize];
        let find = |flags: vk::MemoryPropertyFlags| {
            (0..types.len() as u32).find(|&index| {
                bits & (1 << index) != 0 && types[index as usize].property_flags.contains(flags)
            })
        };
        find(required | preferred).or_else(|| find(required))
    }

    fn buffer(&self, role: Role, size: u64) -> Result<Buffer, Error> {
        use vk::BufferUsageFlags as Usage;
        use vk::MemoryPropertyFlags as Memory;
        let (usage, required, preferred) = match role {
            Role::Source => (
                Usage::STORAGE_BUFFER | Usage::TRANSFER_DST,
                Memory::empty(),
                Memory::DEVICE_LOCAL,
            ),
            Role::Output => (
                Usage::STORAGE_BUFFER | Usage::TRANSFER_SRC | Usage::TRANSFER_DST,
                Memory::empty(),
                Memory::DEVICE_LOCAL,
            ),
            Role::Upload => (
                Usage::TRANSFER_SRC,
                Memory::HOST_VISIBLE | Memory::HOST_COHERENT,
                Memory::empty(),
            ),
            Role::Readback => (
                Usage::TRANSFER_DST,
                Memory::HOST_VISIBLE,
                Memory::HOST_CACHED,
            ),
            Role::Scanout => (
                Usage::STORAGE_BUFFER | Usage::TRANSFER_DST,
                Memory::empty(),
                Memory::DEVICE_LOCAL,
            ),
            Role::Layers => (
                Usage::STORAGE_BUFFER,
                Memory::HOST_VISIBLE | Memory::HOST_COHERENT,
                Memory::DEVICE_LOCAL,
            ),
        };
        let export = role == Role::Scanout;
        let mut external = vk::ExternalMemoryBufferCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let mut exported = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let host = matches!(role, Role::Upload | Role::Readback | Role::Layers);
        // SAFETY: the device is live; on failure every object created here is destroyed.
        unsafe {
            let mut create = vk::BufferCreateInfo::default()
                .size(size)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            if export {
                create = create.push_next(&mut external);
            }
            let buffer = self.device.create_buffer(&create, None)?;
            let requirements = self.device.get_buffer_memory_requirements(buffer);
            let Some(kind) = self.memory_type(requirements.memory_type_bits, required, preferred)
            else {
                self.device.destroy_buffer(buffer, None);
                return Err(Error("no suitable memory type".into()));
            };
            let flags = self.memory.memory_types[kind as usize].property_flags;
            let mut allocate = vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(kind);
            if export {
                allocate = allocate.push_next(&mut exported);
            }
            let memory = match self.device.allocate_memory(&allocate, None) {
                Ok(memory) => memory,
                Err(error) => {
                    self.device.destroy_buffer(buffer, None);
                    return Err(error.into());
                }
            };
            let bound = self.device.bind_buffer_memory(buffer, memory, 0);
            let mapped = bound.and_then(|()| {
                if host {
                    self.device
                        .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
                        .map(|pointer| pointer.cast::<u8>())
                } else {
                    Ok(std::ptr::null_mut())
                }
            });
            match mapped {
                Ok(mapped) => Ok(Buffer {
                    buffer,
                    memory,
                    mapped,
                    size,
                    coherent: flags.contains(Memory::HOST_COHERENT),
                }),
                Err(error) => {
                    self.device.destroy_buffer(buffer, None);
                    self.device.free_memory(memory, None);
                    Err(error.into())
                }
            }
        }
    }

    fn free(&self, buffer: Buffer) {
        // SAFETY: no submission using the buffer is pending (each compose waits for its fence).
        unsafe {
            if !buffer.mapped.is_null() {
                self.device.unmap_memory(buffer.memory);
            }
            self.device.destroy_buffer(buffer.buffer, None);
            self.device.free_memory(buffer.memory, None);
        }
    }
}

impl Drop for VulkanCompositor {
    fn drop(&mut self) {
        // The last submission is done before what it uses goes.
        if let Err(error) = self.complete() {
            eprintln!("[gpu] waiting for the last frame: {error}");
        }
        for slot in 0..self.buffers.len() {
            if let Some(buffer) = self.buffers[slot].take() {
                self.free(buffer);
            }
        }
        for scanout in std::mem::take(&mut self.scanout).into_iter().flatten() {
            self.free(scanout.buffer);
        }
        self.destroy_images();
        // SAFETY: no submission is pending (completed above); every handle was
        // created from this device or instance and is destroyed once, children first; null
        // handles are ignored.
        unsafe {
            self.device.destroy_query_pool(self.queries, None);
            self.device.destroy_fence(self.fence, None);
            if let Some((semaphore, _)) = self.completion.take() {
                self.device.destroy_semaphore(semaphore, None);
            }
            self.device.destroy_command_pool(self.commands, None);
            self.device.destroy_descriptor_pool(self.pool, None);
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_pipeline(self.pipeline_images, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device
                .destroy_descriptor_set_layout(self.set_layout, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}
