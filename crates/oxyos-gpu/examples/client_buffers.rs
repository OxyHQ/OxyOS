//! What composing clients' GPU buffers costs (docs/adr/0031-gpu-buffers.mdx): a 1920×1200 desktop
//! (a wallpaper, a window with rounded corners, a top bar and a dock) composed from shared memory,
//! then from DMA-BUFs exported by another Vulkan device on this GPU, as a client exports them, in
//! each modifier both can share, and in the one the driver picks from what a client offers it:
//! all it shares (as `oxyos-paint` offered before) and the best kind alone (as it does now, ADR
//! 0031, decision 2). Each is composed whole
//! and then with a small damage, as a window's frames are; and each with every frame showing the
//! client's buffers anew (acquired from the queue that wrote them, as every frame did before they
//! changed hands per frame shown) and with frames showing them unchanged. Reports the device's own
//! time (GPU timestamps) before composing (uploads, buffers acquired) and composing, medians of
//! the frames composed.
//!
//!     cargo run --release -p oxyos-gpu --example client_buffers [-- FRAMES]

use ash::vk;
use oxyos_gpu::{DmaBufPlane, Filter, Layer, Paint, Rect, Source, VulkanCompositor};
use std::os::fd::{AsFd, FromRawFd, OwnedFd};

const OUTPUT: (u32, u32) = (1920, 1200);
const XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
const ARGB8888: u32 = u32::from_le_bytes(*b"AR24");

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(f64::NAN)
}

/// A modifier's name, for those of Intel's and the linear one.
fn name(modifier: u64) -> String {
    let intel = |n: u64| (0x01 << 56) | n;
    let known = [
        (0, "LINEAR"),
        (intel(1), "X_TILED"),
        (intel(2), "Y_TILED"),
        (intel(4), "Y_TILED_CCS"),
        (intel(6), "Y_TILED_GEN12_RC_CCS"),
        (intel(7), "Y_TILED_GEN12_MC_CCS"),
        (intel(8), "Y_TILED_GEN12_RC_CCS_CC"),
        (intel(9), "4_TILED"),
    ];
    known
        .iter()
        .find(|(value, _)| *value == modifier)
        .map_or_else(|| format!("{modifier:#018x}"), |(_, name)| (*name).into())
}

/// One layer's pixels: a gradient, opaque or with a translucent band.
fn pixels(width: u32, height: u32, seed: u32, opaque: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let alpha = if opaque || y < height / 2 { 255 } else { 160 };
            let channel = |v: u32| ((v.wrapping_mul(seed) >> 3) & 0xff) * alpha / 255;
            out.extend_from_slice(&[
                channel(x) as u8,
                channel(y) as u8,
                channel(x + y) as u8,
                alpha as u8,
            ]);
        }
    }
    out
}

/// An image exported as a DMA-BUF, and what importing it needs.
struct Exported {
    fd: OwnedFd,
    modifier: u64,
    planes: Vec<(u32, u32)>,
    image: vk::Image,
    memory: vk::DeviceMemory,
}

/// A Vulkan device of its own on this GPU, exporting images as a client does.
struct Exporter {
    _entry: ash::Entry,
    instance: ash::Instance,
    physical: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    family: u32,
    pool: vk::CommandPool,
    memory_fd: ash::khr::external_memory_fd::Device,
    modifiers: ash::ext::image_drm_format_modifier::Device,
}

impl Exporter {
    #[allow(unsafe_code)]
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // SAFETY: loading the system's Vulkan loader; nothing else in this process races with it.
        let entry = unsafe { ash::Entry::load() }?;
        let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_2);
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
        let physicals = unsafe { instance.enumerate_physical_devices() }?;
        // A GPU, as the compositor picks one; a CPU device only if there is nothing else.
        let mut candidates: Vec<(vk::PhysicalDevice, bool)> = physicals
            .into_iter()
            .filter(|&physical| {
                // SAFETY: `physical` was enumerated from this live instance.
                let available = unsafe { instance.enumerate_device_extension_properties(physical) }
                    .unwrap_or_default();
                wanted.iter().all(|name| {
                    available
                        .iter()
                        .any(|e| e.extension_name_as_c_str() == Ok(*name))
                })
            })
            .map(|physical| {
                // SAFETY: as above.
                let properties = unsafe { instance.get_physical_device_properties(physical) };
                (
                    physical,
                    properties.device_type == vk::PhysicalDeviceType::CPU,
                )
            })
            .collect();
        candidates.sort_by_key(|(_, cpu)| *cpu);
        let (physical, _) = *candidates
            .first()
            .ok_or("no device here exports DMA-BUFs with modifiers")?;
        // SAFETY: as above.
        let families = unsafe { instance.get_physical_device_queue_family_properties(physical) };
        let family = families
            .iter()
            .position(|family| {
                family.queue_flags.contains(vk::QueueFlags::TRANSFER)
                    || family.queue_flags.contains(vk::QueueFlags::GRAPHICS)
            })
            .ok_or("no queue family that copies")? as u32;
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        let names: Vec<*const std::ffi::c_char> = wanted.iter().map(|name| name.as_ptr()).collect();
        // SAFETY: the create info and what it points to outlive the call.
        let device = unsafe {
            instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queues)
                    .enabled_extension_names(&names),
                None,
            )
        }?;
        // SAFETY: the device was made with one queue of `family`.
        let queue = unsafe { device.get_device_queue(family, 0) };
        // SAFETY: the device is live.
        let pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default().queue_family_index(family),
                None,
            )
        }?;
        Ok(Self {
            memory_fd: ash::khr::external_memory_fd::Device::new(&instance, &device),
            modifiers: ash::ext::image_drm_format_modifier::Device::new(&instance, &device),
            _entry: entry,
            instance,
            physical,
            device,
            queue,
            family,
            pool,
        })
    }

    /// The modifiers of B8G8R8A8_UNORM this device exports as copy destinations, in its order.
    #[allow(unsafe_code)]
    fn exportable(&self) -> Vec<u64> {
        let format = vk::Format::B8G8R8A8_UNORM;
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
        let mut properties = vk::FormatProperties2::default().push_next(&mut list);
        // SAFETY: the physical device is live; the first query fills only the count.
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
            .filter(|m| {
                m.drm_format_modifier_tiling_features
                    .contains(vk::FormatFeatureFlags::TRANSFER_DST)
            })
            .filter(|m| {
                let mut drm = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
                    .drm_format_modifier(m.drm_format_modifier)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE);
                let mut external = vk::PhysicalDeviceExternalImageFormatInfo::default()
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
                let info = vk::PhysicalDeviceImageFormatInfo2::default()
                    .format(format)
                    .ty(vk::ImageType::TYPE_2D)
                    .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                    .usage(vk::ImageUsageFlags::TRANSFER_DST)
                    .push_next(&mut drm)
                    .push_next(&mut external);
                let mut exported = vk::ExternalImageFormatProperties::default();
                let mut out = vk::ImageFormatProperties2::default().push_next(&mut exported);
                // SAFETY: the physical device is live; the chains outlive the call.
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
            .map(|m| m.drm_format_modifier)
            .collect()
    }

    /// An image of `width` × `height` holding `pixels`, in the modifier the driver picks from
    /// `modifiers`, exported, released to the foreign queue as a client's frame is.
    #[allow(unsafe_code)]
    fn export(
        &self,
        width: u32,
        height: u32,
        modifiers: &[u64],
        pixels: &[u8],
    ) -> Result<Exported, Box<dyn std::error::Error>> {
        let device = &self.device;
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
            .usage(vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut list)
            .push_next(&mut external);
        // SAFETY: the device is live; the create info outlives the call.
        let image = unsafe { device.create_image(&info, None) }?;
        // SAFETY: `image` is live.
        let requirements = unsafe { device.get_image_memory_requirements(image) };
        // SAFETY: the physical device is live.
        let memory_properties = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical)
        };
        let pick = |want: vk::MemoryPropertyFlags, bits: u32| {
            (0..memory_properties.memory_type_count).find(|&i| {
                bits & (1 << i) != 0
                    && memory_properties.memory_types[i as usize]
                        .property_flags
                        .contains(want)
            })
        };
        let index = pick(
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            requirements.memory_type_bits,
        )
        .or_else(|| {
            pick(
                vk::MemoryPropertyFlags::empty(),
                requirements.memory_type_bits,
            )
        })
        .ok_or("no memory type for the image")?;
        let mut export = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        // SAFETY: the device is live; the allocate info outlives the call.
        let memory = unsafe {
            device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(index)
                    .push_next(&mut export)
                    .push_next(&mut dedicated),
                None,
            )
        }?;
        // SAFETY: the memory was allocated for this image, dedicated to it.
        unsafe { device.bind_image_memory(image, memory, 0) }?;

        // The pixels through a staging buffer.
        let size = pixels.len() as u64;
        // SAFETY: the device is live.
        let staging = unsafe {
            device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::TRANSFER_SRC),
                None,
            )
        }?;
        // SAFETY: `staging` is live.
        let staging_requirements = unsafe { device.get_buffer_memory_requirements(staging) };
        let visible =
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        let staging_index =
            pick(visible, staging_requirements.memory_type_bits).ok_or("no host-visible memory")?;
        // SAFETY: the device is live.
        let staging_memory = unsafe {
            device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(staging_requirements.size)
                    .memory_type_index(staging_index),
                None,
            )
        }?;
        // SAFETY: the memory was allocated for this buffer; it is mapped whole, coherent, and
        // `pixels` fits in it.
        unsafe {
            device.bind_buffer_memory(staging, staging_memory, 0)?;
            let mapped = device.map_memory(staging_memory, 0, size, vk::MemoryMapFlags::empty())?;
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), mapped.cast::<u8>(), pixels.len());
            device.unmap_memory(staging_memory);
        }
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(1)
            .layer_count(1);
        // SAFETY: the pool is live; the command buffer is recorded, submitted and waited for
        // before anything it uses is destroyed.
        unsafe {
            let command = device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.pool)
                    .command_buffer_count(1),
            )?[0];
            device.begin_command_buffer(command, &vk::CommandBufferBeginInfo::default())?;
            device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .image(image)
                    .subresource_range(range)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)],
            );
            device.cmd_copy_buffer_to_image(
                command,
                staging,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[vk::BufferImageCopy::default()
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    })],
            );
            // Released to whoever imports it, as a client's presented frame is.
            device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .image(image)
                    .subresource_range(range)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .src_queue_family_index(self.family)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)],
            );
            device.end_command_buffer(command)?;
            let commands = [command];
            device.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default().command_buffers(&commands)],
                vk::Fence::null(),
            )?;
            device.queue_wait_idle(self.queue)?;
            device.free_command_buffers(self.pool, &commands);
            device.destroy_buffer(staging, None);
            device.free_memory(staging_memory, None);
        }
        let mut chosen = vk::ImageDrmFormatModifierPropertiesEXT::default();
        // SAFETY: `image` was made with a modifier list.
        unsafe {
            self.modifiers
                .get_image_drm_format_modifier_properties(image, &mut chosen)
        }?;
        let modifier = chosen.drm_format_modifier;
        let plane_count = self
            .plane_count(modifier)
            .ok_or("the chosen modifier is not the device's")?;
        let aspects = [
            vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
            vk::ImageAspectFlags::MEMORY_PLANE_1_EXT,
            vk::ImageAspectFlags::MEMORY_PLANE_2_EXT,
            vk::ImageAspectFlags::MEMORY_PLANE_3_EXT,
        ];
        let planes = aspects[..plane_count as usize]
            .iter()
            .map(|&aspect| {
                // SAFETY: `image` has this memory plane (its modifier's plane count).
                let layout = unsafe {
                    device.get_image_subresource_layout(
                        image,
                        vk::ImageSubresource::default().aspect_mask(aspect),
                    )
                };
                (layout.offset as u32, layout.row_pitch as u32)
            })
            .collect();
        // SAFETY: the memory was allocated exportable as a DMA-BUF.
        let raw = unsafe {
            self.memory_fd.get_memory_fd(
                &vk::MemoryGetFdInfoKHR::default()
                    .memory(memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT),
            )
        }?;
        // SAFETY: vkGetMemoryFdKHR returns a new descriptor the caller owns.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        Ok(Exported {
            fd,
            modifier,
            planes,
            image,
            memory,
        })
    }

    /// How many memory planes images of `modifier` have here.
    #[allow(unsafe_code)]
    fn plane_count(&self, modifier: u64) -> Option<u32> {
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
        let mut properties = vk::FormatProperties2::default().push_next(&mut list);
        // SAFETY: the physical device is live; the first query fills only the count.
        unsafe {
            self.instance.get_physical_device_format_properties2(
                self.physical,
                vk::Format::B8G8R8A8_UNORM,
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
                vk::Format::B8G8R8A8_UNORM,
                &mut properties,
            )
        };
        modifiers
            .iter()
            .find(|m| m.drm_format_modifier == modifier)
            .map(|m| m.drm_format_modifier_plane_count)
    }

    #[allow(unsafe_code)]
    fn free(&self, exported: Exported) {
        // SAFETY: nothing of this device uses the image any more (each export waited for its
        // copy); the importer holds its own reference to the memory through the descriptor.
        unsafe {
            self.device.destroy_image(exported.image, None);
            self.device.free_memory(exported.memory, None);
        }
    }
}

impl Drop for Exporter {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: every submission was waited for; the objects are destroyed child first.
        unsafe {
            self.device.destroy_command_pool(self.pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

/// The desktop's layers: where each is, its size, whether it is opaque, and its corners' radius.
const SCENE: [(i32, i32, u32, u32, bool, u32); 4] = [
    (0, 0, 1920, 1200, true, 0),     // the wallpaper
    (360, 200, 1200, 800, true, 12), // a window
    (0, 0, 1920, 36, false, 0),      // the top bar
    (640, 1110, 640, 72, false, 18), // the dock
];

/// The scene's layers, each from `paint(index)`.
fn layers<'a>(paint: impl Fn(usize) -> Paint<'a>) -> Vec<Layer<'a>> {
    SCENE
        .iter()
        .enumerate()
        .map(|(index, &(x, y, width, height, _, radius))| Layer {
            x,
            y,
            width,
            height,
            extent: (width, height),
            paint: paint(index),
            source: Some(Source {
                key: index as u64 + 1,
                version: 1,
            }),
            changed: None,
            opacity: 255,
            radius,
            filter: Filter::Nearest,
            clip: None,
        })
        .collect()
}

/// The device's time per frame for `frames` frames of `layers`, whole and with a window's damage:
/// before composing (uploads, buffers acquired from their client) and composing, medians. Newly
/// shown, every frame shows each client buffer anew, acquired from the queue that wrote it (as
/// every frame did before buffers changed hands only per frame shown); else frames show them
/// unchanged.
fn measure(
    gpu: &mut VulkanCompositor,
    layers: &[Layer<'_>],
    images: &[u64],
    frames: usize,
    newly: bool,
) -> Result<[(f64, f64); 2], Box<dyn std::error::Error>> {
    let whole = [Rect {
        x: 0,
        y: 0,
        width: OUTPUT.0,
        height: OUTPUT.1,
    }];
    // A frame of a window: a list's selection moved.
    let small = [Rect {
        x: 500,
        y: 300,
        width: 400,
        height: 300,
    }];
    let mut out = [(0.0, 0.0); 2];
    for (damage, out) in [&whole[..], &small[..]].into_iter().zip(out.iter_mut()) {
        // The first frame uploads what is in shared memory and composes everything.
        gpu.compose_damaged(OUTPUT.0, OUTPUT.1, 0x0020_3040, layers, &whole)?;
        let (mut before, mut composing) = (Vec::new(), Vec::new());
        for _ in 0..frames {
            if newly {
                for &image in images {
                    gpu.release_dma_buf(image);
                }
                gpu.submit_releases()?;
            }
            gpu.compose_damaged(OUTPUT.0, OUTPUT.1, 0x0020_3040, layers, damage)?;
            let handed = gpu.last_handovers();
            assert_eq!(handed.acquired, if newly { images.len() } else { 0 });
            before.extend(gpu.last_gpu_phases().map(|(before, _)| before));
            composing.extend(gpu.last_gpu_ms());
        }
        *out = (median(before), median(composing));
    }
    Ok(out)
}

/// The modifiers a client offers its driver for a buffer now: the best kind both share, as
/// `oxyos-paint` does (tiled of one plane, else linear, else with auxiliary planes).
fn best_shared(table: &[u64], exportable: &[(u64, u32)]) -> Vec<u64> {
    let kind = |&(modifier, planes): &(u64, u32)| match (modifier, planes) {
        (_, planes) if planes > 1 => 2,
        (0, _) => 1,
        _ => 0,
    };
    let shared: Vec<&(u64, u32)> = exportable
        .iter()
        .filter(|(m, _)| table.contains(m))
        .collect();
    let Some(best) = shared.iter().map(|entry| kind(entry)).min() else {
        return Vec::new();
    };
    shared
        .into_iter()
        .filter(|entry| kind(entry) == best)
        .map(|(modifier, _)| *modifier)
        .collect()
}

fn row(label: &str, newly: [(f64, f64); 2], unchanged: [(f64, f64); 2]) {
    let cell = |(before, composing): (f64, f64)| format!("{before:>6.2} {composing:>6.2}");
    println!(
        "{label:<36} {}  {}   {}  {}",
        cell(newly[0]),
        cell(newly[1]),
        cell(unchanged[0]),
        cell(unchanged[1])
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frames: usize = std::env::args().nth(1).map_or(Ok(60), |n| n.parse())?;
    let mut gpu = VulkanCompositor::new()?;
    println!(
        "{} ({:?}), {}x{}, {frames} frames each",
        gpu.device_name(),
        gpu.device_kind(),
        OUTPUT.0,
        OUTPUT.1
    );
    let contents: Vec<Vec<u8>> = SCENE
        .iter()
        .enumerate()
        .map(|(index, &(_, _, width, height, opaque, _))| {
            pixels(width, height, index as u32 + 3, opaque)
        })
        .collect();
    let exporter = Exporter::new()?;
    // The compositor's table (its order), as a client is told it.
    let table: Vec<u64> = gpu
        .dma_buf_formats()
        .iter()
        .filter(|format| format.fourcc == ARGB8888)
        .map(|format| format.modifier)
        .collect();
    let exportable: Vec<(u64, u32)> = exporter
        .exportable()
        .into_iter()
        .filter_map(|modifier| Some((modifier, exporter.plane_count(modifier)?)))
        .collect();
    let names = |modifiers: &mut dyn Iterator<Item = u64>| {
        modifiers.map(name).collect::<Vec<_>>().join(", ")
    };
    println!(
        "the compositor's table: {}",
        names(&mut table.iter().copied())
    );
    println!(
        "the client exports: {}",
        names(&mut exportable.iter().map(|(modifier, _)| *modifier))
    );
    println!(
        "\nms per frame, medians; each: before composing (uploads, buffers acquired), composing\n\
         {:<36} {:^13}  {:^13}   {:^13}  {:^13}\n{:<36} {:^13}  {:^13}   {:^13}  {:^13}",
        "",
        "newly shown",
        "",
        "unchanged",
        "",
        "layers from",
        "whole",
        "damaged",
        "whole",
        "damaged"
    );
    let shm = layers(|index| {
        if SCENE[index].4 {
            Paint::Opaque(&contents[index])
        } else {
            Paint::Premultiplied(&contents[index])
        }
    });
    let unchanged = measure(&mut gpu, &shm, &[], frames, false)?;
    row("shared memory", unchanged, unchanged);

    // Each modifier both share; then what the driver picks from the list a client offered it
    // before (all it shares, linear and compressed ones included) and now (the best kind).
    let shared: Vec<u64> = exportable
        .iter()
        .map(|(modifier, _)| *modifier)
        .filter(|modifier| table.contains(modifier))
        .collect();
    let mut cases: Vec<(String, Vec<u64>)> = shared
        .iter()
        .map(|&modifier| (name(modifier), vec![modifier]))
        .collect();
    cases.push(("picked from all shared (before)".into(), shared.clone()));
    cases.push((
        "picked from the best kind (now)".into(),
        best_shared(&table, &exportable),
    ));
    for (label, modifiers) in cases {
        let mut exported = Vec::new();
        let mut images = Vec::new();
        let mut failed = None;
        for (index, &(_, _, width, height, opaque, _)) in SCENE.iter().enumerate() {
            let result = exporter
                .export(width, height, &modifiers, &contents[index])
                .and_then(|buffer| {
                    let planes: Vec<DmaBufPlane<'_>> = buffer
                        .planes
                        .iter()
                        .map(|&(offset, stride)| DmaBufPlane {
                            fd: buffer.fd.as_fd(),
                            offset,
                            stride,
                        })
                        .collect();
                    let fourcc = if opaque { XRGB8888 } else { ARGB8888 };
                    let image =
                        gpu.import_dma_buf(fourcc, buffer.modifier, width, height, &planes)?;
                    Ok((buffer, image))
                });
            match result {
                Ok((buffer, image)) => {
                    images.push(image);
                    exported.push(buffer);
                }
                Err(error) => {
                    failed = Some(error.to_string());
                    break;
                }
            }
        }
        let chosen = exported.first().map(|buffer| name(buffer.modifier));
        let label = match &chosen {
            Some(chosen) if modifiers.len() > 1 => format!("{label}: {chosen}"),
            _ => label,
        };
        if let Some(error) = failed {
            println!("{label:<36} not composed: {error}");
        } else {
            let imported = layers(|index| Paint::Imported {
                image: images[index],
                opaque: SCENE[index].4,
                pixels: &[],
            });
            let newly = measure(&mut gpu, &imported, &images, frames, true)?;
            let unchanged = measure(&mut gpu, &imported, &images, frames, false)?;
            row(&label, newly, unchanged);
        }
        for &image in &images {
            gpu.release_dma_buf(image);
        }
        gpu.submit_releases()?;
        for image in images {
            gpu.forget_dma_buf(image);
        }
        for buffer in exported {
            exporter.free(buffer);
        }
    }
    Ok(())
}
