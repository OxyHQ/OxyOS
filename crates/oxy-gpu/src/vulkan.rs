//! Composition on a Vulkan device: each layer is a compute dispatch over its rectangle, in
//! order, reading the frame's pixels from one storage buffer and writing the output to another
//! (docs/adr/0004). This slice copies pixels in and reads the output back; sampled images,
//! DMA-BUF import and display output come after it.

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

/// Push constants of `shaders/compose.wgsl`'s `Layer`.
#[repr(C)]
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
}

impl Push {
    fn bytes(&self) -> [u8; 44] {
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
        ];
        let mut bytes = [0; 44];
        for (chunk, word) in bytes.chunks_exact_mut(4).zip(words) {
            chunk.copy_from_slice(&word.to_ne_bytes());
        }
        bytes
    }
}

/// A host-visible storage buffer, persistently mapped.
struct Buffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: *mut u8,
    size: u64,
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
    src: Option<Buffer>,
    dst: Option<Buffer>,
    device_name: String,
    kind: DeviceKind,
}

impl VulkanCompositor {
    /// Opens the best Vulkan device (a GPU before a CPU implementation) with a compute queue.
    pub fn new() -> Result<Self, Error> {
        // SAFETY: loading the system's Vulkan loader runs its initialization; nothing else in
        // this process races with it.
        let entry = unsafe { ash::Entry::load() }
            .map_err(|error| Error(format!("no Vulkan loader: {error}")))?;
        let app = vk::ApplicationInfo::default()
            .application_name(c"oxy-gpu")
            .api_version(vk::API_VERSION_1_1);
        // SAFETY: the create info and what it points to outlive the call.
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default().application_info(&app),
                None,
            )
        }?;
        let opened = Self::device(&instance);
        let (device, queue, memory, device_name, kind, family) = match opened {
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
            device,
            queue,
            memory,
            set_layout: vk::DescriptorSetLayout::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            pool: vk::DescriptorPool::null(),
            set: vk::DescriptorSet::null(),
            commands: vk::CommandPool::null(),
            command: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            src: None,
            dst: None,
            device_name,
            kind,
        };
        this.build(family)?;
        Ok(this)
    }

    /// The best device with a compute queue, and that queue.
    #[allow(clippy::type_complexity)]
    fn device(
        instance: &ash::Instance,
    ) -> Result<
        (
            ash::Device,
            vk::Queue,
            vk::PhysicalDeviceMemoryProperties,
            String,
            DeviceKind,
            u32,
        ),
        Error,
    > {
        // SAFETY: the instance is live.
        let physicals = unsafe { instance.enumerate_physical_devices() }?;
        let rank = |kind: DeviceKind| match kind {
            DeviceKind::Discrete => 0,
            DeviceKind::Integrated => 1,
            DeviceKind::Virtual => 2,
            DeviceKind::Other => 3,
            DeviceKind::Cpu => 4,
        };
        let (physical, family, properties) = physicals
            .into_iter()
            .filter_map(|physical| {
                // SAFETY: `physical` was enumerated from this live instance.
                let families =
                    unsafe { instance.get_physical_device_queue_family_properties(physical) };
                let family = families
                    .iter()
                    .position(|family| family.queue_flags.contains(vk::QueueFlags::COMPUTE))?;
                // SAFETY: as above.
                let properties = unsafe { instance.get_physical_device_properties(physical) };
                Some((physical, family as u32, properties))
            })
            .min_by_key(|(_, _, properties)| rank(properties.device_type.into()))
            .ok_or_else(|| Error("no Vulkan device with a compute queue".into()))?;
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        // SAFETY: `physical` belongs to the instance; the create info outlives the call.
        let device = unsafe {
            instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default().queue_create_infos(&queues),
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
        Ok((
            device,
            queue,
            memory,
            name(&properties.device_name),
            properties.device_type.into(),
            family,
        ))
    }

    /// Creates the pipeline, descriptors, commands and fence on `self.device`.
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
                .size(44)];
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

    /// Composes `layers` onto an output of `width` × `height` filled with `background`, as
    /// [`crate::compose_cpu`] does, and returns the output's pixels.
    pub fn compose(
        &mut self,
        width: u32,
        height: u32,
        background: u32,
        layers: &[Layer<'_>],
    ) -> Result<Vec<u32>, Error> {
        let layers: Vec<&Layer<'_>> = layers.iter().filter(|layer| layer.valid()).collect();
        let words: usize = layers
            .iter()
            .filter_map(|layer| layer.pixels())
            .map(|pixels| pixels.len() / 4)
            .sum();
        let out_words = width as usize * height as usize;
        if out_words == 0 {
            return Ok(Vec::new());
        }
        self.reserve(words.max(1) as u64 * 4, out_words as u64 * 4)?;
        let (src, dst) = (
            self.src.as_ref().expect("reserved"),
            self.dst.as_ref().expect("reserved"),
        );

        // Upload: the layers' pixels one after another, and the background.
        let mut pushes = Vec::with_capacity(layers.len());
        let mut offset = 0usize;
        for layer in &layers {
            let mut push = Push {
                x: layer.x,
                y: layer.y,
                width: layer.width,
                height: layer.height,
                src_width: layer.extent.0,
                src_height: layer.extent.1,
                src_offset: offset as u32,
                out_width: width,
                out_height: height,
                ..Push::default()
            };
            match layer.paint {
                Paint::Fill(color) => {
                    push.mode = 2;
                    push.color = color;
                }
                Paint::Opaque(pixels) | Paint::Premultiplied(pixels) => {
                    push.mode = u32::from(matches!(layer.paint, Paint::Premultiplied(_)));
                    let used = layer.extent.0 as usize * layer.extent.1 as usize * 4;
                    // SAFETY: `src` is mapped for at least `words * 4` bytes and this range ends
                    // within it; `pixels` holds at least `used` bytes (`Layer::valid`).
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            pixels.as_ptr(),
                            src.mapped.add(offset * 4),
                            used,
                        );
                    }
                    offset += used / 4;
                }
            }
            pushes.push(push);
        }
        let background = vec![background; out_words];
        // SAFETY: `dst` is mapped for at least `out_words * 4` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(
                background.as_ptr().cast::<u8>(),
                dst.mapped,
                out_words * 4,
            );
        }

        // SAFETY: every handle is live and owned by this compositor; the command buffer is not in
        // use (the previous submission's fence was waited for); the recorded ranges lie within
        // the buffers bound to the set.
        unsafe {
            let device = &self.device;
            device.reset_command_buffer(self.command, vk::CommandBufferResetFlags::empty())?;
            device.begin_command_buffer(
                self.command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            device.cmd_bind_pipeline(self.command, vk::PipelineBindPoint::COMPUTE, self.pipeline);
            device.cmd_bind_descriptor_sets(
                self.command,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                &[self.set],
                &[],
            );
            for push in &pushes {
                device.cmd_push_constants(
                    self.command,
                    self.pipeline_layout,
                    vk::ShaderStageFlags::COMPUTE,
                    0,
                    &push.bytes(),
                );
                device.cmd_dispatch(
                    self.command,
                    push.width.div_ceil(8),
                    push.height.div_ceil(8),
                    1,
                );
                // Layers compose in order: each sees the one below written.
                let barrier = vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE);
                device.cmd_pipeline_barrier(
                    self.command,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[barrier],
                    &[],
                    &[],
                );
            }
            // The host reads the output after the fence.
            let barrier = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                .dst_access_mask(vk::AccessFlags::HOST_READ);
            device.cmd_pipeline_barrier(
                self.command,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::HOST,
                vk::DependencyFlags::empty(),
                &[barrier],
                &[],
                &[],
            );
            device.end_command_buffer(self.command)?;
            let commands = [self.command];
            device.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default().command_buffers(&commands)],
                self.fence,
            )?;
            device.wait_for_fences(&[self.fence], true, u64::MAX)?;
            device.reset_fences(&[self.fence])?;
        }
        let mut out = vec![0u32; out_words];
        // SAFETY: the fence was waited for, so the device's writes to `dst` (host-coherent) are
        // visible; `dst` is mapped for at least `out_words * 4` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(dst.mapped, out.as_mut_ptr().cast::<u8>(), out_words * 4);
        }
        Ok(out)
    }

    /// Makes the buffers at least this large, binding the new ones to the descriptor set.
    fn reserve(&mut self, src_bytes: u64, dst_bytes: u64) -> Result<(), Error> {
        let mut changed = false;
        for (slot, bytes) in [(0, src_bytes), (1, dst_bytes)] {
            let current = if slot == 0 { &self.src } else { &self.dst };
            if current.as_ref().is_some_and(|buffer| buffer.size >= bytes) {
                continue;
            }
            let buffer = self.buffer(bytes.next_power_of_two())?;
            let old = if slot == 0 {
                self.src.replace(buffer)
            } else {
                self.dst.replace(buffer)
            };
            if let Some(old) = old {
                self.free(old);
            }
            changed = true;
        }
        if changed {
            let (src, dst) = (
                self.src.as_ref().expect("set"),
                self.dst.as_ref().expect("set"),
            );
            let infos = [src, dst].map(|buffer| {
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
        }
        Ok(())
    }

    fn buffer(&self, size: u64) -> Result<Buffer, Error> {
        // SAFETY: the device is live; on failure every object created here is destroyed.
        unsafe {
            let buffer = self.device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )?;
            let requirements = self.device.get_buffer_memory_requirements(buffer);
            let wanted =
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let Some(kind) = (0..self.memory.memory_type_count).find(|&index| {
                requirements.memory_type_bits & (1 << index) != 0
                    && self.memory.memory_types[index as usize]
                        .property_flags
                        .contains(wanted)
            }) else {
                self.device.destroy_buffer(buffer, None);
                return Err(Error("no host-visible coherent memory".into()));
            };
            let memory = match self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(kind),
                None,
            ) {
                Ok(memory) => memory,
                Err(error) => {
                    self.device.destroy_buffer(buffer, None);
                    return Err(error.into());
                }
            };
            let mapped = self
                .device
                .bind_buffer_memory(buffer, memory, 0)
                .and_then(|()| {
                    self.device
                        .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
                });
            match mapped {
                Ok(mapped) => Ok(Buffer {
                    buffer,
                    memory,
                    mapped: mapped.cast(),
                    size,
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
            self.device.unmap_memory(buffer.memory);
            self.device.destroy_buffer(buffer.buffer, None);
            self.device.free_memory(buffer.memory, None);
        }
    }
}

impl Drop for VulkanCompositor {
    fn drop(&mut self) {
        for buffer in [self.src.take(), self.dst.take()].into_iter().flatten() {
            self.free(buffer);
        }
        // SAFETY: no submission is pending (each compose waits for its fence); every handle was
        // created from this device or instance and is destroyed once, children first.
        unsafe {
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
