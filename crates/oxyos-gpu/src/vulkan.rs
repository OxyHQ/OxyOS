//! Composition on a Vulkan device: each layer is a compute dispatch over its rectangle, in
//! order, reading the frame's pixels from one storage buffer and writing the output to another
//! (docs/adr/0004). Both live in device memory where the device has it: pixels arrive through
//! a staging buffer, the background is filled on the device, and the output is copied back into
//! host-cached memory. GPU timestamps measure the composition itself.

#![allow(unsafe_code)]

use super::{name, shader, Layer, Paint};
use ash::vk;
use std::fmt;

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

/// Damage clipped to the output, without empty rectangles. Past eight, one rectangle holding
/// them all: each region costs a dispatch per layer.
fn merged(regions: Vec<crate::Rect>, output: &crate::Rect) -> Vec<crate::Rect> {
    let clipped: Vec<crate::Rect> = regions
        .iter()
        .filter_map(|region| region.intersect(output))
        .collect();
    if clipped.len() <= 8 {
        return clipped;
    }
    vec![clipped[1..]
        .iter()
        .fold(clipped[0], |all, region| all.bounding(region))]
}

/// Push constants of `shaders/compose.wgsl`'s `Layer`.
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

    /// The part of the layer within `region` (output pixels), if any.
    fn within(self, region: &crate::Rect) -> Option<Self> {
        let part = region.intersect(&crate::Rect {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        })?;
        Some(Self {
            base_x: (part.x - self.x) as u32,
            base_y: (part.y - self.y) as u32,
            count_x: part.width,
            count_y: part.height,
            ..self
        })
    }

    fn bytes(&self) -> [u8; 64] {
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
        ];
        let mut bytes = [0; 64];
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
    /// What changed since this target was last written (frames composed into the others),
    /// or `None` when its content is unknown (never written, or a whole frame since).
    stale: Option<Vec<crate::Rect>>,
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
    pipeline: vk::Pipeline,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    commands: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    queries: vk::QueryPool,
    buffers: [Option<Buffer>; 4],
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
    /// Output buffers exported for display (`scanout_targets`).
    scanout: Vec<Scanout>,
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
}

impl VulkanCompositor {
    /// Opens the best Vulkan device (a GPU before a CPU implementation) with a compute queue.
    pub fn new() -> Result<Self, Error> {
        // SAFETY: loading the system's Vulkan loader runs its initialization; nothing else in
        // this process races with it.
        let entry = unsafe { ash::Entry::load() }
            .map_err(|error| Error(format!("no Vulkan loader: {error}")))?;
        let app = vk::ApplicationInfo::default()
            .application_name(c"oxyos-gpu")
            .api_version(vk::API_VERSION_1_1);
        // SAFETY: the create info and what it points to outlive the call.
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default().application_info(&app),
                None,
            )
        }?;
        let opened = match Self::open(&instance) {
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
            pool: vk::DescriptorPool::null(),
            set: vk::DescriptorSet::null(),
            commands: vk::CommandPool::null(),
            command: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            queries: vk::QueryPool::null(),
            buffers: [None, None, None, None],
            device_name: opened.name,
            kind: opened.kind,
            timestamp_period: opened.timestamp_period,
            timestamp_bits: opened.timestamp_bits,
            last_gpu_ms: None,
            last_host_ms: (0.0, 0.0),
            last_gpu_phases: None,
            dma_buf: opened.dma_buf,
            scanout: Vec::new(),
            bound_output: vk::Buffer::null(),
            family: opened.family,
            kept: None,
            output_holds: None,
            resident: std::collections::HashMap::new(),
            source_next: 0,
            last_uploaded: 0,
        };
        this.build(opened.family)?;
        Ok(this)
    }

    /// The best device with a compute queue, and that queue.
    fn open(instance: &ash::Instance) -> Result<Opened, Error> {
        // SAFETY: the instance is live.
        let physicals = unsafe { instance.enumerate_physical_devices() }?;
        let rank = |kind: DeviceKind| match kind {
            DeviceKind::Discrete => 0,
            DeviceKind::Integrated => 1,
            DeviceKind::Virtual => 2,
            DeviceKind::Other => 3,
            DeviceKind::Cpu => 4,
        };
        let (physical, family, timestamp_bits, properties) = physicals
            .into_iter()
            .filter_map(|physical| {
                // SAFETY: `physical` was enumerated from this live instance.
                let families =
                    unsafe { instance.get_physical_device_queue_family_properties(physical) };
                let (family, properties) = families
                    .iter()
                    .enumerate()
                    .find(|(_, family)| family.queue_flags.contains(vk::QueueFlags::COMPUTE))?;
                // SAFETY: as above.
                let device = unsafe { instance.get_physical_device_properties(physical) };
                Some((
                    physical,
                    family as u32,
                    properties.timestamp_valid_bits,
                    device,
                ))
            })
            .min_by_key(|(_, _, _, properties)| rank(properties.device_type.into()))
            .ok_or_else(|| Error("no Vulkan device with a compute queue".into()))?;
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
        let extensions: Vec<*const std::ffi::c_char> = if dma_buf {
            wanted.iter().map(|name| name.as_ptr()).collect()
        } else {
            Vec::new()
        };
        // SAFETY: `physical` belongs to the instance; the create info and the extension names
        // (static strings) outlive the call.
        let device = unsafe {
            instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queues)
                    .enabled_extension_names(&extensions),
                None,
            )
        }?;
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
            name: name(&properties.device_name),
            kind: properties.device_type.into(),
            timestamp_period: properties.limits.timestamp_period,
            timestamp_bits,
            dma_buf,
        })
    }

    /// Creates the pipeline, descriptors, commands, fence and timestamp queries.
    fn build(&mut self, family: u32) -> Result<(), Error> {
        let device = &self.device;
        let bindings = [0, 1].map(|binding| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        });
        // SAFETY: the device is live; the create infos outlive each call. Each object is stored
        // in `self` as soon as it exists, so `Drop` destroys it.
        unsafe {
            self.set_layout = device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )?;
            let ranges = [vk::PushConstantRange::default()
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
                .size(64)];
            let layouts = [self.set_layout];
            self.pipeline_layout = device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&layouts)
                    .push_constant_ranges(&ranges),
                None,
            )?;
            let code = shader();
            let module = device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)?;
            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(module)
                .name(c"main");
            let pipelines = device.create_compute_pipelines(
                vk::PipelineCache::null(),
                &[vk::ComputePipelineCreateInfo::default()
                    .stage(stage)
                    .layout(self.pipeline_layout)],
                None,
            );
            device.destroy_shader_module(module, None);
            self.pipeline = pipelines.map_err(|(_, result)| Error::from(result))?[0];
            let sizes = [vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(2)];
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
    pub fn last_gpu_phases(&self) -> Option<(f64, f64)> {
        self.last_gpu_phases
    }

    /// The last frame's host time, in milliseconds: copying the layers' pixels into the upload
    /// buffer, and waiting from submission to the fence (the device's whole work for it).
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
            .run(width, height, background, layers, None, None)?
            .unwrap_or_default())
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
            .run(width, height, background, layers, None, Some(damage))?
            .unwrap_or_default())
    }

    /// Composes into scanout target `index` (`scanout_targets`) for a display to show; nothing
    /// is read back. Returns once the device has finished writing it. With `damage`, what
    /// changed since the previous frame (on any target), only that and what changed since this
    /// target was last written are composed; without it, the whole frame.
    pub fn compose_to_scanout(
        &mut self,
        index: usize,
        background: u32,
        layers: &[Layer<'_>],
        damage: Option<&[crate::Rect]>,
    ) -> Result<(), Error> {
        let target = self
            .scanout
            .get(index)
            .ok_or_else(|| Error(format!("no scanout target {index}")))?;
        let (width, height) = (target.width, target.height);
        self.run(
            width,
            height,
            background,
            layers,
            Some((index, false)),
            damage,
        )?;
        Ok(())
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
        let target = self
            .scanout
            .get(index)
            .ok_or_else(|| Error(format!("no scanout target {index}")))?;
        let (width, height) = (target.width, target.height);
        self.run(
            width,
            height,
            background,
            layers,
            Some((index, true)),
            damage,
        )?;
        self.kept = Some((width, height));
        Ok(())
    }

    /// The frame the last [`compose_to_scanout_kept`](Self::compose_to_scanout_kept) copied,
    /// row by row at the scanout target's size; it is read once.
    pub fn read_kept(&mut self) -> Result<Vec<u32>, Error> {
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
        scanout: Option<(usize, bool)>,
        damage: Option<&[crate::Rect]>,
    ) -> Result<Option<Vec<u32>>, Error> {
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
        let layers: Vec<&Layer<'_>> = layers.iter().filter(|layer| layer.valid()).collect();
        let words = |layer: &Layer<'_>| u64::from(layer.extent.0) * u64::from(layer.extent.1);
        let frame_words: u64 = layers
            .iter()
            .filter(|layer| layer.pixels().is_some())
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
                layer.pixels().is_some()
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
            Some(index) => self.scanout[index].buffer.buffer,
            None => self.buffers[1].as_ref().expect("reserved").buffer,
        };
        // A scanout buffer the display had comes back to this queue before it is written.
        let acquire = scanout.is_some_and(|index| self.scanout[index].released);
        let family = self.family;
        self.bind_output(target);
        let [Some(source), _, Some(upload), Some(readback)] = &self.buffers else {
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
                ..Push::default()
            };
            match layer.paint {
                Paint::Fill(color) => {
                    push.mode = 2;
                    push.color = color;
                }
                Paint::Opaque(pixels) | Paint::Premultiplied(pixels) => {
                    push.mode = u32::from(matches!(layer.paint, Paint::Premultiplied(_)));
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
            layer_pushes.push(push);
        }
        self.last_uploaded = uploaded * 4;
        self.last_host_ms.0 = copying.elapsed().as_secs_f64() * 1e3;
        // Every layer whole, or, region by region, the background and the parts of the layers
        // within it.
        let pushes: Vec<Push> = match &regions {
            None => layer_pushes.iter().map(|push| push.whole()).collect(),
            Some(regions) => regions
                .iter()
                .flat_map(|region| {
                    let background = Push {
                        x: region.x,
                        y: region.y,
                        width: region.width,
                        height: region.height,
                        out_width: width,
                        out_height: height,
                        mode: 2,
                        color: background,
                        opacity: 255,
                        ..Push::default()
                    }
                    .whole();
                    std::iter::once(background)
                        .chain(layer_pushes.iter().filter_map(|push| push.within(region)))
                })
                .collect(),
        };
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
            if regions.is_none() {
                device.cmd_fill_buffer(command, target, 0, out_bytes, background);
            }
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
            device.cmd_bind_pipeline(command, vk::PipelineBindPoint::COMPUTE, self.pipeline);
            device.cmd_bind_descriptor_sets(
                command,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                &[self.set],
                &[],
            );
            for push in &pushes {
                device.cmd_push_constants(
                    command,
                    self.pipeline_layout,
                    vk::ShaderStageFlags::COMPUTE,
                    0,
                    &push.bytes(),
                );
                device.cmd_dispatch(
                    command,
                    push.count_x.div_ceil(8),
                    push.count_y.div_ceil(8),
                    1,
                );
                // Layers compose in order: each sees the one below written.
                barrier(
                    vk::AccessFlags::SHADER_WRITE,
                    vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                );
            }
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
            let submitting = std::time::Instant::now();
            device.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default().command_buffers(&commands)],
                self.fence,
            )?;
            device.wait_for_fences(&[self.fence], true, u64::MAX)?;
            device.reset_fences(&[self.fence])?;
            self.last_host_ms.1 = submitting.elapsed().as_secs_f64() * 1e3;
        }
        self.last_gpu_ms = if timing {
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
        if let Some(index) = scanout {
            self.scanout[index].released = true;
            // This target holds the frame now; the others lack what changed in it.
            for (other, target) in self.scanout.iter_mut().enumerate() {
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
        Ok(Some(self.read_back(out_words)?))
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
    /// returns their DMA-BUFs (replacing earlier ones). Needs a device that exports DMA-BUF.
    #[cfg(target_os = "linux")]
    pub fn scanout_targets(
        &mut self,
        count: usize,
        width: u32,
        height: u32,
    ) -> Result<Vec<ScanoutTarget>, Error> {
        use std::os::fd::FromRawFd;
        if !self.dma_buf {
            return Err(Error("the device cannot export DMA-BUF".into()));
        }
        for old in std::mem::take(&mut self.scanout) {
            self.free(old.buffer);
        }
        self.bound_output = vk::Buffer::null();
        let loader = ash::khr::external_memory_fd::Device::new(&self.instance, &self.device);
        let mut targets = Vec::with_capacity(count);
        for _ in 0..count {
            let buffer = self.buffer(Role::Scanout, u64::from(width) * u64::from(height) * 4)?;
            let info = vk::MemoryGetFdInfoKHR::default()
                .memory(buffer.memory)
                .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            // SAFETY: the memory was allocated exportable as DMA-BUF and is live.
            let fd = match unsafe { loader.get_memory_fd(&info) } {
                // SAFETY: vkGetMemoryFdKHR returns a new descriptor that the caller owns.
                Ok(fd) => unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) },
                Err(error) => {
                    self.free(buffer);
                    return Err(error.into());
                }
            };
            self.scanout.push(Scanout {
                buffer,
                width,
                height,
                released: false,
                stale: None,
            });
            targets.push(ScanoutTarget {
                fd,
                width,
                height,
                pitch: width * 4,
            });
        }
        Ok(targets)
    }

    /// Makes the buffers at least this large, binding new device buffers to the set.
    fn reserve(
        &mut self,
        source_bytes: u64,
        upload_bytes: u64,
        out_bytes: u64,
    ) -> Result<(), Error> {
        let mut rebind = false;
        for (slot, role, bytes) in [
            (0, Role::Source, source_bytes),
            (1, Role::Output, out_bytes),
            (2, Role::Upload, upload_bytes),
            (3, Role::Readback, out_bytes),
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
            rebind |= slot < 2;
            if slot == 1 {
                // A new output buffer holds no frame.
                self.output_holds = None;
            }
        }
        if rebind {
            let [Some(source), Some(output), ..] = &self.buffers else {
                unreachable!("reserved");
            };
            let infos = [source, output].map(|buffer| {
                [vk::DescriptorBufferInfo::default()
                    .buffer(buffer.buffer)
                    .offset(0)
                    .range(vk::WHOLE_SIZE)]
            });
            let writes = [0, 1].map(|binding| {
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
        };
        let export = role == Role::Scanout;
        let mut external = vk::ExternalMemoryBufferCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let mut exported = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let host = matches!(role, Role::Upload | Role::Readback);
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
        for slot in 0..self.buffers.len() {
            if let Some(buffer) = self.buffers[slot].take() {
                self.free(buffer);
            }
        }
        for scanout in std::mem::take(&mut self.scanout) {
            self.free(scanout.buffer);
        }
        // SAFETY: no submission is pending (each compose waits for its fence); every handle was
        // created from this device or instance and is destroyed once, children first; null
        // handles are ignored.
        unsafe {
            self.device.destroy_query_pool(self.queries, None);
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.commands, None);
            self.device.destroy_descriptor_pool(self.pool, None);
            self.device.destroy_pipeline(self.pipeline, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device
                .destroy_descriptor_set_layout(self.set_layout, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}
