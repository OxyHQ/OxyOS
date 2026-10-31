//! A painter on the GPU for native clients (docs/adr/0031-gpu-buffers.mdx, decision 12): the Bloom
//! host's CPU painter's primitives (tools/rn-host-spike/renderer.cpp) on a Vulkan device, each a
//! compute dispatch with the CPU's own arithmetic per pixel (`shaders/paint.wgsl`), into canvases
//! that live on the device, and frames handed to the compositor as DMA-BUFs with explicit
//! synchronization.
//!
//! - [`Paint::new`]: the device the compositor composes on (its render node, from the client's GPU
//!   table), or another one that exports linear DMA-BUFs.
//! - Canvases ([`Paint::canvas`]): 0xAARRGGBB premultiplied pixels, as the CPU's `Canvas`, in one
//!   heap of words with pictures, glyph masks and blurs' planes. The heap is host-visible memory
//!   (device-local where the device has both): what is uploaded is written in place and a canvas
//!   is read back by copying it.
//! - Operations ([`Fill`], [`Image`], [`Layer`], [`Glyphs`], [`Smooth`], blur, mask, clear) are
//!   recorded with the device pixels they may touch, worked out by the caller as its CPU painter
//!   works them out, and run in order at the next [`Paint::flush`] or present.
//! - [`Paint::present`]: the canvas copied (what changed since each buffer last held it) into one
//!   of the surface's exportable images, a modifier from the compositor's table, attached once,
//!   each with a timeline of its own; the frame's completion goes into its acquire point and the
//!   buffer is drawn again only once its release point has signalled.

#![allow(unsafe_code)]

use ash::vk;
use oxyos_client::Client;
use oxyos_protocol::{Format, Rect};
use std::collections::HashMap;
use std::ffi::CStr;

mod present;

/// The linear modifier.
pub const LINEAR: u64 = 0;

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<vk::Result> for Error {
    fn from(result: vk::Result) -> Self {
        Self(format!("Vulkan: {result}"))
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

type Result<T> = std::result::Result<T, Error>;

/// Where the last present's time went, in milliseconds (the host's report says it for each frame):
/// buffers of another size freed, an image made and exported, attached, its timeline made, a
/// release waited for, the frame before waited for, the frame's operations recorded, the copy
/// recorded and submitted, the acquire point given its fence, the frame sent; and whether it made
/// an image.
#[derive(Clone, Copy, Debug, Default)]
pub struct PresentTimes {
    pub new_image: bool,
    pub freed_ms: f64,
    pub image_ms: f64,
    pub attach_ms: f64,
    pub timeline_ms: f64,
    pub release_ms: f64,
    pub wait_ms: f64,
    pub record_ms: f64,
    pub submit_ms: f64,
    pub sync_ms: f64,
    pub send_ms: f64,
    pub total_ms: f64,
    /// The frame's operations (dispatches), the copy's submission aside.
    pub operations: u32,
}

/// Device pixels an operation may touch, [x0, x1) × [y0, y1), within its canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pixels {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

/// What every drawing operation has: the pixels it may touch, its canvas's scale (device pixels
/// per logical pixel), its clip (logical), its straight colour and its opacity.
#[derive(Clone, Copy, Debug, Default)]
pub struct Common {
    pub pixels: Pixels,
    pub scale: f32,
    pub clip: [f32; 4],
    pub color: [f32; 4],
    pub opacity: f32,
}

/// A rounded box, less a hole: the CPU painter's `fill`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fill {
    pub rect: [f32; 4],
    pub radii: [f32; 4],
    pub hole: Option<([f32; 4], [f32; 4])>,
}

/// Where an image's texels come from.
#[derive(Clone, Copy, Debug)]
pub enum Source {
    /// A picture uploaded with [`Paint::picture`] (R, G, B, A bytes, premultiplied).
    Picture(u64),
    /// A canvas's pixels.
    Canvas(u32),
}

/// A picture stretched over `rect`, rounded by `radii` over `bounds`, tinted or not: `image`.
#[derive(Clone, Copy, Debug)]
pub struct Image {
    pub source: Source,
    pub rect: [f32; 4],
    pub bounds: [f32; 4],
    pub radii: [f32; 4],
    /// The colour of `Common` is the tint's.
    pub tinted: bool,
}

/// A canvas drawn through an affine map: `layer`. `inverse` maps the canvas's logical pixels to
/// the layer's (a, b, c, d, tx, ty: x' = a x + c y + tx).
#[derive(Clone, Copy, Debug)]
pub struct Layer {
    pub canvas: u32,
    pub layer_scale: f32,
    pub origin: [f32; 2],
    pub inverse: [f32; 6],
}

/// A glyph of a run: where its mask's top-left pixel goes (device pixels) and the mask
/// ([`Paint::glyph`]).
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub x: i32,
    pub y: i32,
    pub mask: u64,
}

/// A canvas blurred and drawn at full precision, dithered: `smooth` of the levels of `source`
/// blurred by `sigma` (device pixels of the source).
#[derive(Clone, Copy, Debug)]
pub struct Smooth {
    pub source: u32,
    pub sigma: f32,
    pub rect: [f32; 4],
    pub bounds: [f32; 4],
    pub radii: [f32; 4],
}

/// The operations of `shaders/paint.wgsl`.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum Kind {
    Fill = 1,
    Image = 2,
    Layer = 3,
    Glyphs = 4,
    Smooth = 5,
    Levels = 6,
    Box = 7,
    FromLevels = 8,
    Mask = 9,
    Clear = 10,
    Copy = 11,
}

/// Words of an operation's record.
const RECORD: usize = 40;

struct Allocation {
    offset: u64,
    words: u64,
}

struct Canvas {
    at: Allocation,
    width: u32,
    height: u32,
}

/// A host-visible buffer and its mapping.
struct Buffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: *mut u32,
    words: u64,
    coherent: bool,
}

/// What runs on the device: one operation, or a run of operations on one canvas worked out tile by
/// tile in one dispatch (its bins in the heap, its grid of tiles from `origin`).
enum Step {
    One(usize),
    Tiles {
        bins: u32,
        origin: (u32, u32),
        grid: (u32, u32),
    },
}

/// Operations a tile can work out among others: each writes only its own pixel of its canvas.
fn tiled(kind: u32) -> bool {
    [
        Kind::Fill as u32,
        Kind::Image as u32,
        Kind::Layer as u32,
        Kind::Glyphs as u32,
        Kind::Smooth as u32,
        Kind::Mask as u32,
        Kind::Clear as u32,
    ]
    .contains(&kind)
}

/// Pixels of a tile's side.
const TILE: u32 = 16;

/// One dispatch: its record and its grid.
struct Dispatch {
    record: u32,
    groups: (u32, u32),
}

pub struct Paint {
    pub(crate) gpu: present::Gpu,
    pipeline: vk::Pipeline,
    /// The same shader's `tiles` entry: runs of operations, tile by tile.
    tiles: vk::Pipeline,
    /// What the recorded operations run as, worked out when they are submitted.
    steps: Vec<Step>,
    layout: vk::PipelineLayout,
    set_layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    commands: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    submitted: bool,
    heap: Buffer,
    /// Free spans of the heap (offset, words), in order.
    free: Vec<(u64, u64)>,
    /// Spans to free once the submission in flight is done.
    freeing: Vec<Allocation>,
    records: Buffer,
    recorded: Vec<u32>,
    dispatches: Vec<Dispatch>,
    canvases: HashMap<u32, Canvas>,
    next_canvas: u32,
    /// Pictures and glyph masks by their caller's key, and the frame each was last used in.
    pictures: HashMap<u64, (Allocation, u32, u32, u64)>,
    masks: HashMap<u64, (Allocation, u32, u32, u64)>,
    /// `i / scale` for each scale drawn at (by its bits) and how many coordinates it holds.
    quotients: HashMap<u32, (Allocation, u32)>,
    frame: u64,
    /// What is presented: each surface's buffers.
    surfaces: HashMap<u32, present::Swapchain>,
    /// Where the last present's time went.
    pub(crate) last_present: PresentTimes,
}

fn words_of(slice: &[u8]) -> Vec<u32> {
    slice
        .chunks(4)
        .map(|chunk| {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            u32::from_le_bytes(word)
        })
        .collect()
}

impl Paint {
    /// A painter on the device whose render node is `device` (the compositor's, its GPU table's),
    /// else on any device that exports DMA-BUF, with linear buffers.
    pub fn new(device: u64) -> Result<Self> {
        let gpu = present::Gpu::open(device)?;
        let mut paint = Self {
            gpu,
            pipeline: vk::Pipeline::null(),
            tiles: vk::Pipeline::null(),
            steps: Vec::new(),
            layout: vk::PipelineLayout::null(),
            set_layout: vk::DescriptorSetLayout::null(),
            pool: vk::DescriptorPool::null(),
            set: vk::DescriptorSet::null(),
            commands: vk::CommandPool::null(),
            command: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            submitted: false,
            heap: Buffer {
                buffer: vk::Buffer::null(),
                memory: vk::DeviceMemory::null(),
                mapped: std::ptr::null_mut(),
                words: 0,
                coherent: true,
            },
            free: Vec::new(),
            freeing: Vec::new(),
            records: Buffer {
                buffer: vk::Buffer::null(),
                memory: vk::DeviceMemory::null(),
                mapped: std::ptr::null_mut(),
                words: 0,
                coherent: true,
            },
            recorded: Vec::new(),
            dispatches: Vec::new(),
            canvases: HashMap::new(),
            next_canvas: 0,
            pictures: HashMap::new(),
            masks: HashMap::new(),
            quotients: HashMap::new(),
            frame: 0,
            surfaces: HashMap::new(),
            last_present: PresentTimes::default(),
        };
        paint.build()?;
        paint.heap = paint.host_buffer(16 << 20)?;
        paint.free = vec![(0, paint.heap.words)];
        paint.records = paint.host_buffer(1 << 16)?;
        paint.bind();
        // Both entry points run once, now: a driver that compiles a pipeline when it first runs
        // (lavapipe) does it before the first frame, not in one (it took 0.5 to 1.8 s there).
        let warm = paint.canvas(1, 1, 0)?;
        paint.flush()?;
        let pixel = Pixels {
            x0: 0,
            y0: 0,
            x1: 1,
            y1: 1,
        };
        paint.clear(warm, pixel, 1)?;
        paint.clear(warm, pixel, 2)?;
        paint.flush()?;
        paint.wait()?;
        paint.free_canvas(warm);
        Ok(paint)
    }

    /// The device's name, as its driver gives it.
    pub fn device_name(&self) -> &str {
        &self.gpu.name
    }

    /// Whether it exports DMA-BUFs to present in (it paints either way).
    pub fn exports(&self) -> bool {
        !self.gpu.exportable_is_empty()
    }

    /// Whether it is the compositor's own device (tiled modifiers may be shared).
    pub fn same_device(&self) -> bool {
        self.gpu.same_device
    }

    fn build(&mut self) -> Result<()> {
        let device = &self.gpu.device;
        let bindings = [0, 1].map(|binding| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(binding)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        });
        let code: Vec<u32> = include_bytes!(concat!(env!("OUT_DIR"), "/paint.spv"))
            .chunks_exact(4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .collect();
        // SAFETY: the device is live; every info outlives its call; each object is stored in
        // `self` at once, so `Drop` destroys it.
        unsafe {
            self.set_layout = device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )?;
            let layouts = [self.set_layout];
            let ranges = [vk::PushConstantRange::default()
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
                .size(24)];
            self.layout = device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&layouts)
                    .push_constant_ranges(&ranges),
                None,
            )?;
            let module = device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)?;
            let entry = |name: &'static std::ffi::CStr| {
                vk::ComputePipelineCreateInfo::default()
                    .stage(
                        vk::PipelineShaderStageCreateInfo::default()
                            .stage(vk::ShaderStageFlags::COMPUTE)
                            .module(module)
                            .name(name),
                    )
                    .layout(self.layout)
            };
            let pipelines = device.create_compute_pipelines(
                vk::PipelineCache::null(),
                &[entry(c"main"), entry(c"tiles")],
                None,
            );
            device.destroy_shader_module(module, None);
            let pipelines = pipelines.map_err(|(_, result)| Error::from(result))?;
            self.pipeline = pipelines[0];
            self.tiles = pipelines[1];
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
                    .queue_family_index(self.gpu.family)
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

    /// A storage buffer of `words` in host-visible memory, device-local where there is both,
    /// mapped.
    fn host_buffer(&self, words: u64) -> Result<Buffer> {
        let device = &self.gpu.device;
        // SAFETY: the device is live; every info outlives its call; the buffer is freed on an
        // error below or by `free_buffer`.
        unsafe {
            let buffer = device.create_buffer(
                &vk::BufferCreateInfo::default().size(words * 4).usage(
                    vk::BufferUsageFlags::STORAGE_BUFFER
                        | vk::BufferUsageFlags::TRANSFER_SRC
                        | vk::BufferUsageFlags::TRANSFER_DST,
                ),
                None,
            )?;
            let requirements = device.get_buffer_memory_requirements(buffer);
            let types = &self.gpu.memory.memory_types[..self.gpu.memory.memory_type_count as usize];
            let find = |flags: vk::MemoryPropertyFlags| {
                (0..types.len()).find(|&index| {
                    requirements.memory_type_bits & (1 << index) != 0
                        && types[index].property_flags.contains(flags)
                })
            };
            use vk::MemoryPropertyFlags as Memory;
            let Some(kind) = find(Memory::HOST_VISIBLE | Memory::DEVICE_LOCAL)
                .or_else(|| find(Memory::HOST_VISIBLE))
            else {
                device.destroy_buffer(buffer, None);
                return Err(Error("no host-visible memory".into()));
            };
            let coherent = types[kind].property_flags.contains(Memory::HOST_COHERENT);
            let memory = match device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(kind as u32),
                None,
            ) {
                Ok(memory) => memory,
                Err(error) => {
                    device.destroy_buffer(buffer, None);
                    return Err(error.into());
                }
            };
            let mapped = device.bind_buffer_memory(buffer, memory, 0).and_then(|()| {
                device.map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
            });
            match mapped {
                Ok(mapped) => Ok(Buffer {
                    buffer,
                    memory,
                    mapped: mapped.cast(),
                    words,
                    coherent,
                }),
                Err(error) => {
                    device.destroy_buffer(buffer, None);
                    device.free_memory(memory, None);
                    Err(error.into())
                }
            }
        }
    }

    fn free_buffer(&self, buffer: &Buffer) {
        // SAFETY: no submission in flight uses it (the caller waited); each handle once.
        unsafe {
            self.gpu.device.destroy_buffer(buffer.buffer, None);
            self.gpu.device.free_memory(buffer.memory, None);
        }
    }

    /// Points the descriptor set at the heap and the records.
    fn bind(&self) {
        let heap = [vk::DescriptorBufferInfo::default()
            .buffer(self.heap.buffer)
            .range(vk::WHOLE_SIZE)];
        let records = [vk::DescriptorBufferInfo::default()
            .buffer(self.records.buffer)
            .range(vk::WHOLE_SIZE)];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(self.set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&heap),
            vk::WriteDescriptorSet::default()
                .dst_set(self.set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&records),
        ];
        // SAFETY: the set is not in use (no submission is pending when it is bound).
        unsafe { self.gpu.device.update_descriptor_sets(&writes, &[]) };
    }

    /// Waits for the submission in flight, then frees what waited for it.
    fn wait(&mut self) -> Result<()> {
        if self.submitted {
            // SAFETY: the fence belongs to the last submission.
            unsafe {
                self.gpu
                    .device
                    .wait_for_fences(&[self.fence], true, u64::MAX)?;
                self.gpu.device.reset_fences(&[self.fence])?;
            }
            self.submitted = false;
        }
        for allocation in std::mem::take(&mut self.freeing) {
            self.release(allocation);
        }
        Ok(())
    }

    fn release(&mut self, allocation: Allocation) {
        let at = self
            .free
            .partition_point(|&(offset, _)| offset < allocation.offset);
        self.free.insert(at, (allocation.offset, allocation.words));
        // Join neighbours.
        let mut joined: Vec<(u64, u64)> = Vec::with_capacity(self.free.len());
        for &(offset, words) in &self.free {
            match joined.last_mut() {
                Some(last) if last.0 + last.1 == offset => last.1 += words,
                _ => joined.push((offset, words)),
            }
        }
        self.free = joined;
    }

    /// `words` of the heap, growing it when full (the device is idle meanwhile: what was in it
    /// is copied to the same offsets).
    fn allocate(&mut self, words: u64) -> Result<Allocation> {
        let words = words.max(1);
        if let Some(index) = self.free.iter().position(|&(_, free)| free >= words) {
            let (offset, free) = self.free[index];
            if free == words {
                self.free.remove(index);
            } else {
                self.free[index] = (offset + words, free - words);
            }
            return Ok(Allocation { offset, words });
        }
        // Grow: wait for the work in flight (it reads the old heap), copy, rebind.
        self.wait()?;
        let old_words = self.heap.words;
        let new_words = (old_words * 2).max(old_words + words * 2);
        let bigger = self.host_buffer(new_words)?;
        // SAFETY: both mappings hold at least `old_words` words; the device is idle.
        unsafe {
            std::ptr::copy_nonoverlapping(self.heap.mapped, bigger.mapped, old_words as usize);
        }
        let old = std::mem::replace(&mut self.heap, bigger);
        self.free_buffer(&old);
        self.release(Allocation {
            offset: old_words,
            words: new_words - old_words,
        });
        self.bind();
        self.allocate(words)
    }

    /// Frees an allocation once the work recorded so far is done.
    fn retire(&mut self, allocation: Allocation) {
        self.freeing.push(allocation);
    }

    /// The heap's words, for writing what is uploaded (the device is idle: `begin`).
    fn heap_words(&mut self, allocation: &Allocation) -> &mut [u32] {
        // SAFETY: the allocation lies within the heap's mapping; nothing in flight uses it
        // (callers `begin` first, which waits for the submission in flight).
        unsafe {
            std::slice::from_raw_parts_mut(
                self.heap.mapped.add(allocation.offset as usize),
                allocation.words as usize,
            )
        }
    }

    /// Recording starts after the last submission completed (the heap is the host's again).
    fn begin(&mut self) -> Result<()> {
        if self.submitted {
            self.wait()?;
        }
        Ok(())
    }

    // ---- canvases ---------------------------------------------------------------------------

    /// A canvas of `width` × `height` pixels, every one `value` (0xAARRGGBB).
    pub fn canvas(&mut self, width: u32, height: u32, value: u32) -> Result<u32> {
        self.begin()?;
        let at = self.allocate(u64::from(width) * u64::from(height))?;
        self.next_canvas += 1;
        let id = self.next_canvas;
        self.canvases.insert(id, Canvas { at, width, height });
        self.clear(
            id,
            Pixels {
                x0: 0,
                y0: 0,
                x1: width as i32,
                y1: height as i32,
            },
            value,
        )?;
        Ok(id)
    }

    pub fn canvas_size(&self, canvas: u32) -> Option<(u32, u32)> {
        self.canvases
            .get(&canvas)
            .map(|found| (found.width, found.height))
    }

    /// A canvas at another size: its pixels kept where both sizes have them, anchored at the top
    /// left, the rest `value`.
    pub fn resize(&mut self, canvas: u32, width: u32, height: u32, value: u32) -> Result<()> {
        let Some((old_width, old_height)) = self.canvas_size(canvas) else {
            return Err(Error("no such canvas".into()));
        };
        if (old_width, old_height) == (width, height) {
            return Ok(());
        }
        let fresh = self.canvas(width, height, value)?;
        let source = self.canvases[&canvas].at.offset;
        let kept = Pixels {
            x0: 0,
            y0: 0,
            x1: width.min(old_width) as i32,
            y1: height.min(old_height) as i32,
        };
        self.record(
            fresh,
            Kind::Copy,
            kept,
            &Common::default(),
            &[source as u32, old_width],
        )?;
        // The new canvas takes the old one's number.
        let moved = self.canvases.remove(&fresh).expect("made");
        let old = self.canvases.insert(canvas, moved).expect("present");
        self.retire(old.at);
        Ok(())
    }

    /// A copy of a canvas, as it is once the work recorded so far is done.
    pub fn copy(&mut self, canvas: u32) -> Result<u32> {
        let Some((width, height)) = self.canvas_size(canvas) else {
            return Err(Error("no such canvas".into()));
        };
        let copy = self.canvas(width, height, 0)?;
        let source = self.canvases[&canvas].at.offset;
        self.record(
            copy,
            Kind::Copy,
            Pixels {
                x0: 0,
                y0: 0,
                x1: width as i32,
                y1: height as i32,
            },
            &Common::default(),
            &[source as u32, width],
        )?;
        Ok(copy)
    }

    pub fn free_canvas(&mut self, canvas: u32) {
        if let Some(found) = self.canvases.remove(&canvas) {
            self.retire(found.at);
        }
    }

    /// A canvas's pixels (rows of 0xAARRGGBB), once everything recorded is done.
    pub fn read(&mut self, canvas: u32) -> Result<Vec<u32>> {
        self.flush()?;
        self.wait()?;
        let found = self
            .canvases
            .get(&canvas)
            .ok_or_else(|| Error("no such canvas".into()))?;
        let (offset, words) = (found.at.offset, found.at.words);
        if !self.heap.coherent {
            // SAFETY: the memory is mapped; the submission that wrote it completed.
            unsafe {
                self.gpu.device.invalidate_mapped_memory_ranges(&[
                    vk::MappedMemoryRange::default()
                        .memory(self.heap.memory)
                        .size(vk::WHOLE_SIZE),
                ])?;
            }
        }
        // SAFETY: the canvas lies within the mapping; the device is idle.
        Ok(unsafe {
            std::slice::from_raw_parts(self.heap.mapped.add(offset as usize), words as usize)
        }
        .to_vec())
    }

    // ---- what is uploaded ---------------------------------------------------------------

    /// Makes a picture (`width` × `height` premultiplied R, G, B, A bytes) known by `key`; false
    /// when it was known already (nothing is uploaded).
    pub fn picture(&mut self, key: u64, width: u32, height: u32, rgba: &[u8]) -> Result<bool> {
        let frame = self.frame;
        if let Some(found) = self.pictures.get_mut(&key) {
            found.3 = frame;
            return Ok(false);
        }
        if rgba.len() < width as usize * height as usize * 4 {
            return Err(Error("a picture shorter than its size".into()));
        }
        self.begin()?;
        let at = self.allocate(u64::from(width) * u64::from(height))?;
        let words = words_of(&rgba[..width as usize * height as usize * 4]);
        self.heap_words(&at).copy_from_slice(&words);
        self.pictures.insert(key, (at, width, height, frame));
        Ok(true)
    }

    pub fn forget_picture(&mut self, key: u64) {
        if let Some((at, ..)) = self.pictures.remove(&key) {
            self.retire(at);
        }
    }

    /// Makes a glyph mask (`width` × `height` bytes of coverage) known by `key`; false when known.
    pub fn glyph(&mut self, key: u64, width: u32, height: u32, mask: &[u8]) -> Result<bool> {
        let frame = self.frame;
        if let Some(found) = self.masks.get_mut(&key) {
            found.3 = frame;
            return Ok(false);
        }
        let bytes = width as usize * height as usize;
        if mask.len() < bytes {
            return Err(Error("a mask shorter than its size".into()));
        }
        self.begin()?;
        let at = self.allocate((bytes as u64).div_ceil(4))?;
        let words = words_of(&mask[..bytes]);
        self.heap_words(&at)[..words.len()].copy_from_slice(&words);
        self.masks.insert(key, (at, width, height, frame));
        Ok(true)
    }

    pub fn has_glyph(&self, key: u64) -> bool {
        self.masks.contains_key(&key)
    }

    // ---- recording ----------------------------------------------------------------------

    /// Where `i / scale` lies for every coordinate of a canvas `length` pixels on its longer side,
    /// divided as the CPU painter divides (in single precision, correctly rounded). The shader
    /// reads its quotients by the scale here: a device's division may be an ulp off (Vulkan allows
    /// 2.5 and does not promise a fused fma to correct it), and a clip edge falling exactly on a
    /// column then keeps or drops that whole column of a glyph.
    fn quotients(&mut self, scale: f32, length: u32) -> Result<u32> {
        let length = length.max(2);
        if let Some((at, held)) = self.quotients.get(&scale.to_bits()) {
            if *held >= length {
                return Ok(at.offset as u32);
            }
        }
        let at = self.allocate(u64::from(length))?;
        let words: Vec<u32> = (0..length).map(|i| (i as f32 / scale).to_bits()).collect();
        self.heap_words(&at).copy_from_slice(&words);
        let offset = at.offset as u32;
        if let Some((held, _)) = self.quotients.insert(scale.to_bits(), (at, length)) {
            self.retire(held);
        }
        Ok(offset)
    }

    fn record(
        &mut self,
        canvas: u32,
        kind: Kind,
        pixels: Pixels,
        common: &Common,
        args: &[u32],
    ) -> Result<()> {
        let Some(target) = self.canvases.get(&canvas) else {
            return Ok(());
        };
        let (at, width, height) = (target.at.offset as u32, target.width, target.height);
        let pixels = Pixels {
            x0: pixels.x0.max(0),
            y0: pixels.y0.max(0),
            x1: pixels.x1.min(width as i32),
            y1: pixels.y1.min(height as i32),
        };
        if kind != Kind::Box && (pixels.x0 >= pixels.x1 || pixels.y0 >= pixels.y1) {
            return Ok(());
        }
        let quotients = if common.scale.is_finite() && common.scale > 0.0 {
            self.quotients(common.scale, width.max(height))?
        } else {
            0
        };
        let mut record = [0u32; RECORD];
        record[0] = kind as u32;
        record[1] = at;
        record[2] = width;
        record[3] = height;
        record[4] = pixels.x0 as u32;
        record[5] = pixels.y0 as u32;
        record[6] = pixels.x1 as u32;
        record[7] = pixels.y1 as u32;
        record[8] = common.scale.to_bits();
        for (index, value) in common.clip.iter().enumerate() {
            record[9 + index] = value.to_bits();
        }
        for (index, value) in common.color.iter().enumerate() {
            record[13 + index] = value.to_bits();
        }
        record[17] = common.opacity.to_bits();
        record[18] = quotients;
        record[19..19 + args.len()].copy_from_slice(args);
        let at = self.recorded.len() as u32;
        self.recorded.extend_from_slice(&record);
        let groups = if kind == Kind::Box {
            // A line per invocation along x, a plane along y.
            (args[3].div_ceil(8), 1)
        } else {
            (
                ((pixels.x1 - pixels.x0) as u32).div_ceil(8),
                ((pixels.y1 - pixels.y0) as u32).div_ceil(8),
            )
        };
        self.dispatches.push(Dispatch { record: at, groups });
        Ok(())
    }

    fn bits(values: &[f32]) -> impl Iterator<Item = u32> + '_ {
        values.iter().map(|value| value.to_bits())
    }

    /// A box of the canvas set to `value`.
    pub fn clear(&mut self, canvas: u32, pixels: Pixels, value: u32) -> Result<()> {
        self.begin()?;
        self.record(canvas, Kind::Clear, pixels, &Common::default(), &[value])?;
        Ok(())
    }

    pub fn fill(&mut self, canvas: u32, common: &Common, fill: &Fill) -> Result<()> {
        self.begin()?;
        let mut args: Vec<u32> = Self::bits(&fill.rect)
            .chain(Self::bits(&fill.radii))
            .collect();
        let (hole, hole_radii) = fill.hole.unwrap_or(([0.0; 4], [0.0; 4]));
        args.push(u32::from(fill.hole.is_some()));
        args.extend(Self::bits(&hole));
        args.extend(Self::bits(&hole_radii));
        self.record(canvas, Kind::Fill, common.pixels, common, &args)?;
        Ok(())
    }

    pub fn image(&mut self, canvas: u32, common: &Common, image: &Image) -> Result<()> {
        self.begin()?;
        let frame = self.frame;
        let (offset, width, height, is_canvas) = match image.source {
            Source::Picture(key) => match self.pictures.get_mut(&key) {
                Some(found) => {
                    found.3 = frame;
                    (found.0.offset, found.1, found.2, 0)
                }
                None => return Err(Error("no such picture".into())),
            },
            Source::Canvas(source) => match self.canvases.get(&source) {
                Some(found) => (found.at.offset, found.width, found.height, 1),
                None => return Err(Error("no such canvas".into())),
            },
        };
        if width == 0 || height == 0 {
            return Ok(());
        }
        let mut args = vec![offset as u32, width, height, is_canvas];
        args.extend(Self::bits(&image.rect));
        args.extend(Self::bits(&image.bounds));
        args.extend(Self::bits(&image.radii));
        args.push(u32::from(image.tinted));
        self.record(canvas, Kind::Image, common.pixels, common, &args)?;
        Ok(())
    }

    pub fn layer(&mut self, canvas: u32, common: &Common, layer: &Layer) -> Result<()> {
        self.begin()?;
        let Some(source) = self.canvases.get(&layer.canvas) else {
            return Err(Error("no such canvas".into()));
        };
        let mut args = vec![source.at.offset as u32, source.width, source.height];
        args.push(layer.layer_scale.to_bits());
        args.extend(Self::bits(&layer.origin));
        args.extend(Self::bits(&layer.inverse));
        self.record(canvas, Kind::Layer, common.pixels, common, &args)?;
        Ok(())
    }

    /// A run of glyphs in `common`'s colour (its alpha with the opacity in it), each mask known.
    pub fn glyphs(&mut self, canvas: u32, common: &Common, glyphs: &[Glyph]) -> Result<()> {
        self.begin()?;
        let frame = self.frame;
        let mut list = Vec::with_capacity(glyphs.len() * 5);
        for glyph in glyphs {
            let Some(found) = self.masks.get_mut(&glyph.mask) else {
                return Err(Error("no such glyph mask".into()));
            };
            found.3 = frame;
            list.extend_from_slice(&[
                glyph.x as u32,
                glyph.y as u32,
                found.1,
                found.2,
                found.0.offset as u32,
            ]);
        }
        if list.is_empty() {
            return Ok(());
        }
        let at = self.allocate(list.len() as u64)?;
        self.heap_words(&at).copy_from_slice(&list);
        let offset = at.offset as u32;
        self.retire(at);
        self.record(
            canvas,
            Kind::Glyphs,
            common.pixels,
            common,
            &[offset, glyphs.len() as u32],
        )?;
        Ok(())
    }

    /// blurPlanes on four planes of `width` × `height` levels at `planes`, scratch beside them.
    fn blur_planes(
        &mut self,
        canvas: u32,
        planes: u64,
        scratch: u64,
        width: u32,
        height: u32,
        sigma: f32,
    ) -> Result<()> {
        if width == 0 || height == 0 || sigma < 0.5 {
            return Ok(());
        }
        let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
        let mut lower = ideal.floor() as i32;
        if lower % 2 == 0 {
            lower -= 1;
        }
        let upper = lower + 2;
        let lower_count =
            ((12.0 * sigma * sigma - 3.0 * (lower * lower) as f32 - 12.0 * lower as f32 - 9.0)
                / (-4.0 * lower as f32 - 4.0))
                .round() as i32;
        let size = width * height;
        for pass in 0..3 {
            let r = (if pass < lower_count { lower } else { upper } - 1) / 2;
            let r = r.max(0) as u32;
            // Rows into the scratch, then columns back.
            for (from, to, lines, length, stride, step) in [
                (planes, scratch, height, width, width, 1),
                (scratch, planes, width, height, 1, width),
            ] {
                self.record(
                    canvas,
                    Kind::Box,
                    Pixels::default(),
                    &Common::default(),
                    &[from as u32, to as u32, r, lines, length, stride, step, size],
                )?;
            }
        }
        Ok(())
    }

    /// gaussianBlur: the canvas's pixels blurred in place by `sigma` device pixels.
    pub fn blur(&mut self, canvas: u32, sigma: f32) -> Result<()> {
        self.begin()?;
        let Some((width, height)) = self.canvas_size(canvas) else {
            return Err(Error("no such canvas".into()));
        };
        if width == 0 || height == 0 || sigma < 0.5 {
            return Ok(());
        }
        let size = u64::from(width) * u64::from(height);
        let planes = self.allocate(size * 8)?;
        let whole = Pixels {
            x0: 0,
            y0: 0,
            x1: width as i32,
            y1: height as i32,
        };
        let source = self.canvases[&canvas].at.offset as u32;
        self.record(
            canvas,
            Kind::Levels,
            whole,
            &Common::default(),
            &[source, width, height, planes.offset as u32],
        )?;
        self.blur_planes(
            canvas,
            planes.offset,
            planes.offset + size * 4,
            width,
            height,
            sigma,
        )?;
        self.record(
            canvas,
            Kind::FromLevels,
            whole,
            &Common::default(),
            &[planes.offset as u32, width, height],
        )?;
        self.retire(planes);
        Ok(())
    }

    /// The levels of `smooth.source`, blurred, drawn onto `canvas` at full precision, dithered.
    pub fn smooth(&mut self, canvas: u32, common: &Common, smooth: &Smooth) -> Result<()> {
        self.begin()?;
        let Some((width, height)) = self.canvas_size(smooth.source) else {
            return Err(Error("no such canvas".into()));
        };
        if width == 0 || height == 0 {
            return Ok(());
        }
        let size = u64::from(width) * u64::from(height);
        let planes = self.allocate(size * 8)?;
        let source = self.canvases[&smooth.source].at.offset as u32;
        self.record(
            smooth.source,
            Kind::Levels,
            Pixels {
                x0: 0,
                y0: 0,
                x1: width as i32,
                y1: height as i32,
            },
            &Common::default(),
            &[source, width, height, planes.offset as u32],
        )?;
        self.blur_planes(
            smooth.source,
            planes.offset,
            planes.offset + size * 4,
            width,
            height,
            smooth.sigma,
        )?;
        let mut args = vec![planes.offset as u32, width, height];
        args.extend(Self::bits(&smooth.rect));
        args.extend(Self::bits(&smooth.bounds));
        args.extend(Self::bits(&smooth.radii));
        self.record(canvas, Kind::Smooth, common.pixels, common, &args)?;
        self.retire(planes);
        Ok(())
    }

    /// drawMasked: `content` kept by the alpha of `mask` (a canvas of its size).
    pub fn mask(&mut self, content: u32, mask: u32) -> Result<()> {
        self.begin()?;
        let (Some((width, height)), Some(size)) =
            (self.canvas_size(content), self.canvas_size(mask))
        else {
            return Err(Error("no such canvas".into()));
        };
        if (width, height) != size {
            return Err(Error("a mask of another size".into()));
        }
        let offset = self.canvases[&mask].at.offset as u32;
        self.record(
            content,
            Kind::Mask,
            Pixels {
                x0: 0,
                y0: 0,
                x1: width as i32,
                y1: height as i32,
            },
            &Common::default(),
            &[offset],
        )?;
        Ok(())
    }

    /// What the recorded operations run as: runs of consecutive operations a tile can work out on
    /// one canvas, binned by tile into the heap (each tile's operations in order), and the others
    /// one by one. A step reads what the steps before it wrote (a barrier between each).
    fn plan(&mut self) -> Result<()> {
        self.steps.clear();
        let mut index = 0;
        while index < self.dispatches.len() {
            let record = self.dispatches[index].record as usize;
            let (kind, canvas) = (self.recorded[record], self.recorded[record + 1]);
            let mut end = index + 1;
            if tiled(kind) {
                while end < self.dispatches.len() {
                    let next = self.dispatches[end].record as usize;
                    if !tiled(self.recorded[next]) || self.recorded[next + 1] != canvas {
                        break;
                    }
                    end += 1;
                }
            }
            if end - index < 2 {
                self.steps.push(Step::One(index));
                index = end;
                continue;
            }
            // The run's tiles: those its operations' pixels cover.
            let boxes: Vec<(u32, u32, u32, u32)> = (index..end)
                .map(|at| {
                    let r = self.dispatches[at].record as usize;
                    (
                        self.recorded[r + 4] / TILE,
                        self.recorded[r + 5] / TILE,
                        self.recorded[r + 6].div_ceil(TILE),
                        self.recorded[r + 7].div_ceil(TILE),
                    )
                })
                .collect();
            let bounds = |at: usize| boxes[at - index];
            let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
            for at in index..end {
                let (a, b, c, d) = bounds(at);
                (x0, y0, x1, y1) = (x0.min(a), y0.min(b), x1.max(c), y1.max(d));
            }
            let (width, height) = (x1 - x0, y1 - y0);
            let tiles = (width * height) as usize;
            let mut counts = vec![0u32; tiles];
            for at in index..end {
                let (a, b, c, d) = bounds(at);
                for y in b..d {
                    for x in a..c {
                        counts[((y - y0) * width + (x - x0)) as usize] += 1;
                    }
                }
            }
            let listed: u32 = counts.iter().sum();
            let at = self.allocate(tiles as u64 + 1 + u64::from(listed))?;
            let base = at.offset as u32;
            // Each tile's range of the list (absolute heap words), then the list: records in order.
            let mut words = Vec::with_capacity(tiles + 1 + listed as usize);
            let mut next = base + tiles as u32 + 1;
            for count in &counts {
                words.push(next);
                next += count;
            }
            words.push(next);
            let records: Vec<u32> = (index..end).map(|at| self.dispatches[at].record).collect();
            let mut fill: Vec<u32> = words[..tiles].to_vec();
            words.resize(tiles + 1 + listed as usize, 0);
            for item in index..end {
                let (a, b, c, d) = bounds(item);
                let record = records[item - index];
                for y in b..d {
                    for x in a..c {
                        let tile = ((y - y0) * width + (x - x0)) as usize;
                        words[(fill[tile] - base) as usize] = record;
                        fill[tile] += 1;
                    }
                }
            }
            self.heap_words(&at).copy_from_slice(&words);
            self.retire(at);
            self.steps.push(Step::Tiles {
                bins: base,
                origin: (x0, y0),
                grid: (width, height),
            });
            index = end;
        }
        Ok(())
    }

    /// Records the steps into the command buffer, in order, each after the one before.
    fn commands(&self) {
        let device = &self.gpu.device;
        let barrier = vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::HOST_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE);
        // SAFETY: the command buffer is recording (the caller began it); every handle is live.
        unsafe {
            device.cmd_pipeline_barrier(
                self.command,
                vk::PipelineStageFlags::HOST,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[barrier],
                &[],
                &[],
            );
            device.cmd_bind_descriptor_sets(
                self.command,
                vk::PipelineBindPoint::COMPUTE,
                self.layout,
                0,
                &[self.set],
                &[],
            );
            for step in &self.steps {
                let (pipeline, constants, groups) = match *step {
                    Step::One(index) => {
                        let dispatch = &self.dispatches[index];
                        (
                            self.pipeline,
                            [dispatch.record, 0, 0, 0, 0, 0],
                            dispatch.groups,
                        )
                    }
                    Step::Tiles { bins, origin, grid } => {
                        (self.tiles, [0, bins, grid.0, origin.0, origin.1, 0], grid)
                    }
                };
                let bytes: Vec<u8> = constants
                    .iter()
                    .flat_map(|word| word.to_ne_bytes())
                    .collect();
                device.cmd_bind_pipeline(self.command, vk::PipelineBindPoint::COMPUTE, pipeline);
                device.cmd_push_constants(
                    self.command,
                    self.layout,
                    vk::ShaderStageFlags::COMPUTE,
                    0,
                    &bytes,
                );
                device.cmd_dispatch(self.command, groups.0, groups.1, 1);
                device.cmd_pipeline_barrier(
                    self.command,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[vk::MemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                        .dst_access_mask(
                            vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                        )],
                    &[],
                    &[],
                );
            }
        }
    }

    /// Writes the records, and begins the command buffer with their dispatches.
    fn begin_commands(&mut self) -> Result<()> {
        self.plan()?;
        if self.recorded.len() as u64 > self.records.words {
            let bigger = self.host_buffer((self.recorded.len() as u64 * 2).max(1 << 16))?;
            let old = std::mem::replace(&mut self.records, bigger);
            self.free_buffer(&old);
            self.bind();
        }
        // SAFETY: the records' mapping holds them; the device is idle (begin waited).
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.recorded.as_ptr(),
                self.records.mapped,
                self.recorded.len(),
            );
        }
        if !self.heap.coherent || !self.records.coherent {
            let ranges = [
                vk::MappedMemoryRange::default()
                    .memory(self.heap.memory)
                    .size(vk::WHOLE_SIZE),
                vk::MappedMemoryRange::default()
                    .memory(self.records.memory)
                    .size(vk::WHOLE_SIZE),
            ];
            // SAFETY: both are mapped whole.
            unsafe { self.gpu.device.flush_mapped_memory_ranges(&ranges)? };
        }
        // SAFETY: the command buffer is not in use (the last submission was waited for).
        unsafe {
            self.gpu
                .device
                .reset_command_buffer(self.command, vk::CommandBufferResetFlags::empty())?;
            self.gpu.device.begin_command_buffer(
                self.command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        self.commands();
        Ok(())
    }

    /// Submits the command buffer begun, signalling `semaphore` too.
    fn submit(&mut self, signal: &[vk::Semaphore]) -> Result<()> {
        // SAFETY: the command buffer is recording; the queue and fence are live.
        unsafe {
            self.gpu.device.end_command_buffer(self.command)?;
            let commands = [self.command];
            self.gpu.device.queue_submit(
                self.gpu.queue,
                &[vk::SubmitInfo::default()
                    .command_buffers(&commands)
                    .signal_semaphores(signal)],
                self.fence,
            )?;
        }
        self.submitted = true;
        self.recorded.clear();
        self.dispatches.clear();
        self.frame += 1;
        self.collect();
        Ok(())
    }

    /// Runs what is recorded (returns once submitted).
    pub fn flush(&mut self) -> Result<()> {
        if self.dispatches.is_empty() {
            return Ok(());
        }
        self.begin()?;
        self.begin_commands()?;
        self.submit(&[])
    }

    /// Pictures and masks unused for a while go (decoded again, uploaded again if seen again).
    fn collect(&mut self) {
        let frame = self.frame;
        let stale = |last: u64| frame > last + 600;
        let pictures: Vec<u64> = self
            .pictures
            .iter()
            .filter(|(_, found)| stale(found.3))
            .map(|(key, _)| *key)
            .collect();
        for key in pictures {
            self.forget_picture(key);
        }
        let masks: Vec<u64> = self
            .masks
            .iter()
            .filter(|(_, found)| stale(found.3))
            .map(|(key, _)| *key)
            .collect();
        for key in masks {
            if let Some((at, ..)) = self.masks.remove(&key) {
                self.retire(at);
            }
        }
    }

    // ---- presenting -----------------------------------------------------------------------

    /// Presents `canvas` on `surface` in answer to `configure`, as `format` (Argb8888
    /// premultiplied or Xrgb8888): what changed since a buffer of its last held it (`damage`,
    /// canvas pixels; none: everything) copied into one of the surface's GPU buffers, submitted
    /// with its acquire and release points. Returns the frame's sequence number. An error means
    /// the GPU path cannot go on (a buffer the compositor failed to import, the device lost): the
    /// caller presents in shared memory from then on.
    pub fn present(
        &mut self,
        client: &mut Client,
        surface: u32,
        configure: u64,
        canvas: u32,
        format: Format,
        damage: &[Rect],
    ) -> Result<u64> {
        present::present(self, client, surface, configure, canvas, format, damage)
    }

    /// Where the last present's time went.
    pub fn last_present(&self) -> PresentTimes {
        self.last_present
    }

    /// Forgets a surface's buffers (it is destroyed).
    pub fn forget_surface(&mut self, client: &mut Client, surface: u32) {
        if let Some(chain) = self.surfaces.remove(&surface) {
            let _ = self.wait();
            chain.destroy(&self.gpu, client);
        }
    }

    pub(crate) fn canvas_offset(&self, canvas: u32) -> Option<(u64, u32, u32)> {
        self.canvases
            .get(&canvas)
            .map(|found| (found.at.offset, found.width, found.height))
    }
}

impl Drop for Paint {
    fn drop(&mut self) {
        let _ = self.wait();
        // SAFETY: the device is idle; each object is this device's and destroyed once.
        unsafe {
            let _ = self.gpu.device.device_wait_idle();
        }
        for (_, chain) in std::mem::take(&mut self.surfaces) {
            chain.free(&self.gpu);
        }
        self.free_buffer(&self.heap);
        self.free_buffer(&self.records);
        // SAFETY: as above.
        unsafe {
            let device = &self.gpu.device;
            device.destroy_fence(self.fence, None);
            device.destroy_command_pool(self.commands, None);
            device.destroy_descriptor_pool(self.pool, None);
            device.destroy_pipeline(self.pipeline, None);
            device.destroy_pipeline(self.tiles, None);
            device.destroy_pipeline_layout(self.layout, None);
            device.destroy_descriptor_set_layout(self.set_layout, None);
        }
    }
}

/// The device's name from Vulkan's array.
pub(crate) fn name(raw: &[std::ffi::c_char]) -> String {
    // SAFETY: Vulkan's names are NUL-terminated within their arrays.
    unsafe { CStr::from_ptr(raw.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests;
