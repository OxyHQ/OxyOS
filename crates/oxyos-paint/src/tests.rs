//! The painter's device as Skia is told of it, and frames presented through a connection: the
//! compositor's side reads each buffer's DMA-BUF once its acquire point has signalled and finds
//! the source image's pixels, a reused buffer given what it lacked (docs/adr/0031-gpu-buffers.mdx,
//! decision 12). Without a Vulkan device, DMA-BUF export or timelines, what needs them is skipped
//! and says so; `tools/gpu-vm` has them all.

use super::*;
use oxyos_dmabuf::{DmaBuf, Mapping, RenderNode, Timeline};
use oxyos_ipc::Connection;
use oxyos_protocol::{Event, Request, ServerConnection};
use std::collections::VecDeque;
use std::os::fd::OwnedFd;
use std::sync::Arc;

fn paint() -> Option<Paint> {
    match Paint::new(0) {
        Ok(paint) => Some(paint),
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            None
        }
    }
}

/// The painter's handles, loaded through its own `vkGetInstanceProcAddr` as Skia loads them.
fn loaded(vulkan: &Vulkan<'_>) -> (ash::Instance, ash::Device) {
    let entry = ash::StaticFn {
        get_instance_proc_addr: vulkan.get_instance_proc_addr,
    };
    // SAFETY: the handles are the painter's, live while it is.
    unsafe {
        let instance = ash::Instance::load(&entry, vulkan.instance);
        let device = ash::Device::load(instance.fp_v1_0(), vulkan.device);
        (instance, device)
    }
}

#[test]
fn skia_is_given_a_graphics_queue_and_the_exact_features_and_extensions_enabled() {
    let Some(paint) = paint() else { return };
    let vulkan = paint.vulkan();
    assert_ne!(vulkan.instance, vk::Instance::null());
    assert_ne!(vulkan.physical_device, vk::PhysicalDevice::null());
    assert_ne!(vulkan.device, vk::Device::null());
    assert_ne!(vulkan.queue, vk::Queue::null());
    assert_eq!(vulkan.api_version, vk::API_VERSION_1_1);
    assert!(vulkan.device_api_version >= vk::API_VERSION_1_1);
    let (instance, _) = loaded(&vulkan);
    // SAFETY: the physical device is the painter's.
    let (families, supported) = unsafe {
        (
            instance.get_physical_device_queue_family_properties(vulkan.physical_device),
            instance.get_physical_device_features(vulkan.physical_device),
        )
    };
    assert!(families[vulkan.queue_family as usize]
        .queue_flags
        .contains(vk::QueueFlags::GRAPHICS));
    let enabled = vulkan.features;
    assert!(enabled.p_next.is_null(), "no chain after the core features");
    assert_eq!(enabled.features.robust_buffer_access, vk::FALSE);
    let expected = vk::PhysicalDeviceFeatures {
        robust_buffer_access: vk::FALSE,
        ..supported
    };
    // Every other core feature as the device has it (compared as the struct's words).
    let words = |features: &vk::PhysicalDeviceFeatures| -> Vec<vk::Bool32> {
        // SAFETY: VkPhysicalDeviceFeatures is a struct of VkBool32 alone.
        unsafe {
            std::slice::from_raw_parts(
                (features as *const vk::PhysicalDeviceFeatures).cast::<vk::Bool32>(),
                size_of::<vk::PhysicalDeviceFeatures>() / size_of::<vk::Bool32>(),
            )
        }
        .to_vec()
    };
    assert_eq!(words(&enabled.features), words(&expected));
    assert!(vulkan.instance_extensions.is_empty());
    let exports = [
        ash::khr::external_memory_fd::NAME,
        ash::ext::external_memory_dma_buf::NAME,
        ash::ext::image_drm_format_modifier::NAME,
    ]
    .iter()
    .all(|name| vulkan.device_extensions.contains(name));
    assert!(exports || !paint.exports());
    eprintln!(
        "{}: Vulkan {}.{} (the instance's {}.{}), family {}, device extensions {:?}",
        paint.device_name(),
        vk::api_version_major(vulkan.device_api_version),
        vk::api_version_minor(vulkan.device_api_version),
        vk::api_version_major(vulkan.api_version),
        vk::api_version_minor(vulkan.api_version),
        vulkan.queue_family,
        vulkan.device_extensions
    );
}

/// A buffer's modifiers, as the host offers them to its driver (docs/adr/0031-gpu-buffers.mdx,
/// decision 2): the best kind it shares with the compositor alone, tiled before linear before
/// compressed, so the driver never takes linear or a compressed one while a tiled one would do.
#[test]
fn a_buffer_is_offered_the_best_kind_of_modifier_both_sides_share() {
    let intel = |n: u64| (0x01 << 56) | n;
    let exportable = [(0, 1), (intel(1), 1), (intel(2), 1), (intel(6), 2)];
    let everything = [intel(1), intel(2), 0, intel(6)];
    assert_eq!(
        present::best_shared(&everything, &exportable),
        vec![intel(1), intel(2)],
        "the tiled ones, for the driver to pick its best"
    );
    assert_eq!(
        present::best_shared(&[0, intel(6)], &exportable),
        vec![0],
        "linear before compressed"
    );
    assert_eq!(
        present::best_shared(&[intel(6)], &exportable),
        vec![intel(6)]
    );
    assert!(present::best_shared(&[intel(9)], &exportable).is_empty());
}

/// What a buffer lacks (ADR 0005's age): everything when it holds nothing, when the frame names
/// no damage, when the history does not reach back to its frame or a frame since had another size;
/// else this frame's damage and every frame's since, clipped.
#[test]
fn a_reused_buffer_lacks_the_damage_of_every_frame_since_it_was_written() {
    let size = (16, 8);
    let whole = vec![Rect::new(0, 0, 16, 8)];
    let a = Rect::new(2, 1, 5, 3);
    let b = Rect::new(9, 4, 4, 3);
    let history: VecDeque<_> = [
        (1, size, None),
        (2, size, Some(vec![a])),
        (3, size, Some(vec![b])),
    ]
    .into();
    let c = Rect::new(14, 6, 10, 10);
    let clipped = Rect::new(14, 6, 2, 2);
    assert_eq!(present::missed(&history, 0, size, &[c]), whole, "new");
    assert_eq!(present::missed(&history, 1, size, &[]), whole, "no damage");
    assert_eq!(
        present::missed(&history, 1, size, &[c]),
        vec![clipped, a, b]
    );
    assert_eq!(present::missed(&history, 2, size, &[c]), vec![clipped, b]);
    assert_eq!(
        present::missed(&history, 3, size, &[a]),
        vec![a],
        "the frame before"
    );
    let resized: VecDeque<_> = [(1, size, None), (2, (8, 8), Some(vec![a]))].into();
    assert_eq!(present::missed(&resized, 1, size, &[b]), whole, "resized");
    let forgotten: VecDeque<_> = [(5, size, Some(vec![a]))].into();
    assert_eq!(present::missed(&forgotten, 1, size, &[b]), whole, "too old");
}

const WIDTH: u32 = 16;
const HEIGHT: u32 = 8;

/// The renderer's part, as Skia plays it: an image of its own on the painter's device, drawn into
/// (here: rectangles copied from host memory) and left in TRANSFER_SRC_OPTIMAL, each drawing
/// submitted to the painter's queue and not waited for.
struct Drawer {
    instance: ash::Instance,
    device: ash::Device,
    physical: vk::PhysicalDevice,
    queue: vk::Queue,
    pool: vk::CommandPool,
    image: vk::Image,
    memory: vk::DeviceMemory,
    /// Each drawing's host buffer, freed at the end.
    uploads: Vec<(vk::Buffer, vk::DeviceMemory)>,
    drawn: bool,
    /// What the image holds once its drawings are done (0xAARRGGBB).
    pixels: Vec<u32>,
}

impl Drawer {
    fn new(vulkan: &Vulkan<'_>) -> Self {
        let (instance, device) = loaded(vulkan);
        // SAFETY: the painter's live device; each object is destroyed in `finish`.
        unsafe {
            let pool = device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default().queue_family_index(vulkan.queue_family),
                    None,
                )
                .unwrap();
            let image = device
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(vk::Format::B8G8R8A8_UNORM)
                        .extent(vk::Extent3D {
                            width: WIDTH,
                            height: HEIGHT,
                            depth: 1,
                        })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(
                            vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST,
                        ),
                    None,
                )
                .unwrap();
            let mut drawer = Self {
                instance,
                device,
                physical: vulkan.physical_device,
                queue: vulkan.queue,
                pool,
                image,
                memory: vk::DeviceMemory::null(),
                uploads: Vec::new(),
                drawn: false,
                pixels: vec![0; (WIDTH * HEIGHT) as usize],
            };
            let requirements = drawer.device.get_image_memory_requirements(image);
            drawer.memory = drawer.allocate(requirements, vk::MemoryPropertyFlags::empty());
            drawer
                .device
                .bind_image_memory(image, drawer.memory, 0)
                .unwrap();
            drawer
        }
    }

    fn allocate(
        &self,
        requirements: vk::MemoryRequirements,
        flags: vk::MemoryPropertyFlags,
    ) -> vk::DeviceMemory {
        // SAFETY: the physical device and the device are live.
        unsafe {
            let memory = self
                .instance
                .get_physical_device_memory_properties(self.physical);
            let kind = (0..memory.memory_type_count)
                .find(|&index| {
                    requirements.memory_type_bits & (1 << index) != 0
                        && memory.memory_types[index as usize]
                            .property_flags
                            .contains(flags)
                })
                .unwrap();
            self.device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(requirements.size)
                        .memory_type_index(kind),
                    None,
                )
                .unwrap()
        }
    }

    /// Draws `color(x, y)` into `rect` and submits it.
    fn draw(&mut self, rect: Rect, color: impl Fn(u32, u32) -> u32) {
        let (x0, y0) = (rect.x as u32, rect.y as u32);
        let texels: Vec<u32> = (y0..y0 + rect.height)
            .flat_map(|y| (x0..x0 + rect.width).map(move |x| (x, y)))
            .map(|(x, y)| color(x, y))
            .collect();
        for (index, &texel) in texels.iter().enumerate() {
            let (x, y) = (
                x0 + index as u32 % rect.width,
                y0 + index as u32 / rect.width,
            );
            self.pixels[(y * WIDTH + x) as usize] = texel;
        }
        let range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        let layers = vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        };
        // SAFETY: the painter's live device; the upload is written before the submission reads
        // it and freed after the device is idle; the queue is used from this thread alone.
        unsafe {
            let device = &self.device;
            let bytes = (texels.len() * 4) as u64;
            let buffer = device
                .create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(bytes)
                        .usage(vk::BufferUsageFlags::TRANSFER_SRC),
                    None,
                )
                .unwrap();
            let memory = self.allocate(
                device.get_buffer_memory_requirements(buffer),
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            );
            device.bind_buffer_memory(buffer, memory, 0).unwrap();
            let mapped = device
                .map_memory(memory, 0, bytes, vk::MemoryMapFlags::empty())
                .unwrap();
            std::ptr::copy_nonoverlapping(texels.as_ptr(), mapped.cast(), texels.len());
            device.unmap_memory(memory);
            self.uploads.push((buffer, memory));
            let command = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(self.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .unwrap()[0];
            device
                .begin_command_buffer(command, &vk::CommandBufferBeginInfo::default())
                .unwrap();
            let old = if self.drawn {
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL
            } else {
                vk::ImageLayout::UNDEFINED
            };
            device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .old_layout(old)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(self.image)
                    .subresource_range(range)],
            );
            device.cmd_copy_buffer_to_image(
                command,
                buffer,
                self.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[vk::BufferImageCopy::default()
                    .image_subresource(layers)
                    .image_offset(vk::Offset3D {
                        x: rect.x,
                        y: rect.y,
                        z: 0,
                    })
                    .image_extent(vk::Extent3D {
                        width: rect.width,
                        height: rect.height,
                        depth: 1,
                    })],
            );
            // Left as the painter takes it; its own barrier makes the write visible.
            device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(self.image)
                    .subresource_range(range)],
            );
            device.end_command_buffer(command).unwrap();
            let commands = [command];
            device
                .queue_submit(
                    self.queue,
                    &[vk::SubmitInfo::default().command_buffers(&commands)],
                    vk::Fence::null(),
                )
                .unwrap();
        }
        self.drawn = true;
    }

    fn source(&self) -> Source {
        Source {
            image: self.image,
            width: WIDTH,
            height: HEIGHT,
        }
    }

    fn finish(self) {
        // SAFETY: the device is idle; each object is destroyed once.
        unsafe {
            self.device.device_wait_idle().unwrap();
            for (buffer, memory) in &self.uploads {
                self.device.destroy_buffer(*buffer, None);
                self.device.free_memory(*memory, None);
            }
            self.device.destroy_image(self.image, None);
            self.device.free_memory(self.memory, None);
            self.device.destroy_command_pool(self.pool, None);
        }
    }
}

/// The compositor's side: the buffers and timelines a client attached, and its frames.
struct Compositor {
    connection: Connection,
    node: Arc<RenderNode>,
    planes: Vec<(u64, OwnedFd, u32, u32)>,
    timelines: Vec<(u32, Timeline)>,
}

/// A frame as the compositor received it: its buffer, acquire and release points, damage.
struct Received {
    buffer: u64,
    timeline: u32,
    acquire: u64,
    release: u64,
    damage: Vec<Rect>,
}

impl Compositor {
    /// The requests up to the next Submit.
    fn frame(&mut self) -> Received {
        let mut damage = Vec::new();
        let mut sync = None;
        loop {
            let (request, fd) = self.connection.recv_request().unwrap();
            match request {
                Request::AttachDmabufPlane {
                    buffer,
                    offset,
                    stride,
                    ..
                } => self
                    .planes
                    .push((buffer, fd.expect("a plane's descriptor"), offset, stride)),
                Request::AttachDmabuf { modifier, .. } => assert_eq!(modifier, LINEAR),
                Request::ImportTimeline { timeline } => {
                    let fd = fd.expect("a timeline's descriptor");
                    self.timelines.push((
                        timeline,
                        Timeline::import(&self.node, std::os::fd::AsFd::as_fd(&fd)).unwrap(),
                    ));
                }
                Request::Damage { rect, .. } => damage.push(rect),
                Request::SetSync {
                    acquire_timeline,
                    acquire_point,
                    release_timeline,
                    release_point,
                    ..
                } => {
                    assert_eq!(acquire_timeline, release_timeline);
                    sync = Some((acquire_timeline, acquire_point, release_point));
                }
                Request::Submit { buffer, .. } => {
                    let (timeline, acquire, release) = sync.expect("SetSync before Submit");
                    return Received {
                        buffer,
                        timeline,
                        acquire,
                        release,
                        damage,
                    };
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    fn timeline(&self, id: u32) -> &Timeline {
        &self
            .timelines
            .iter()
            .find(|(known, _)| *known == id)
            .expect("an imported timeline")
            .1
    }

    /// The frame's buffer as the compositor reads it, once its acquire point has signalled.
    fn pixels(&self, frame: &Received) -> Vec<u32> {
        let timeline = self.timeline(frame.timeline);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while timeline.signalled().unwrap() < frame.acquire {
            assert!(
                std::time::Instant::now() < deadline,
                "the acquire point never signalled"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let (_, fd, offset, stride) = self
            .planes
            .iter()
            .find(|(buffer, ..)| *buffer == frame.buffer)
            .expect("an attached buffer");
        let mapping = Mapping::new(DmaBuf::from_fd(fd.try_clone().unwrap()).unwrap()).unwrap();
        let mut bytes = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
        mapping
            .read_rows(*offset, *stride, WIDTH as usize * 4, 0..HEIGHT, &mut bytes)
            .unwrap();
        bytes
            .chunks_exact(4)
            .map(|texel| u32::from_le_bytes([texel[0], texel[1], texel[2], texel[3]]))
            .collect()
    }
}

#[test]
fn frames_presented_are_the_source_pixels_and_a_reused_buffer_gets_what_it_lacked() {
    let Some(mut paint) = paint() else { return };
    if !paint.exports() {
        eprintln!("no DMA-BUF export here, skipped; tools/gpu-vm runs it");
        return;
    }
    let Some(node) = RenderNode::with_timelines(None) else {
        eprintln!("no render node with timeline syncobjs here, skipped; tools/gpu-vm runs it");
        return;
    };
    let node = Arc::new(node);
    let (client, server) = Connection::pair().unwrap();
    let handshake = std::thread::spawn(move || {
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        let welcome = protocol.receive(&hello).unwrap().expect("welcome");
        server.send_event(&welcome, None).unwrap();
        server
    });
    let mut client = Client::handshake(client).unwrap();
    let mut compositor = Compositor {
        connection: handshake.join().unwrap(),
        node: node.clone(),
        planes: Vec::new(),
        timelines: Vec::new(),
    };
    for event in [
        Event::GpuDevice {
            device: node.dev(),
            formats: 1,
        },
        Event::GpuFormat {
            index: 0,
            format: Format::Xrgb8888,
            modifier: LINEAR,
        },
    ] {
        compositor.connection.send_event(&event, None).unwrap();
        assert!(matches!(
            client.poll().unwrap(),
            oxyos_client::Poll::Event(_)
        ));
    }
    let mut drawer = Drawer::new(&paint.vulkan());
    let whole = Rect::new(0, 0, WIDTH, HEIGHT);
    let surface = 1;

    // The first frame: a new buffer holding the whole of it.
    drawer.draw(whole, |x, y| 0xff00_0000 | (x << 16) | (y << 8) | (x ^ y));
    paint
        .present(
            &mut client,
            surface,
            1,
            drawer.source(),
            Format::Xrgb8888,
            &[],
        )
        .unwrap();
    let first = compositor.frame();
    assert!(paint.last_present().new_image);
    assert_eq!(paint.last_present().operations, 1);
    assert!(first.damage.is_empty());
    assert_eq!(compositor.pixels(&first), drawer.pixels);

    // The second: the first buffer is still the compositor's, so a second, new, holds it whole.
    let a = Rect::new(2, 1, 5, 3);
    drawer.draw(a, |_, _| 0xff11_2233);
    paint
        .present(
            &mut client,
            surface,
            2,
            drawer.source(),
            Format::Xrgb8888,
            &[a],
        )
        .unwrap();
    let second = compositor.frame();
    assert_ne!(second.buffer, first.buffer);
    assert!(paint.last_present().new_image);
    assert_eq!(second.damage, vec![a]);
    assert_eq!(compositor.pixels(&second), drawer.pixels);

    // The first buffer released: the third frame reuses it, copying the second's damage and its
    // own, and nothing else.
    compositor
        .timeline(first.timeline)
        .signal(first.release)
        .unwrap();
    let b = Rect::new(9, 4, 4, 3);
    drawer.draw(b, |x, y| 0xff40_0000 | (x << 8) | y);
    paint
        .present(
            &mut client,
            surface,
            3,
            drawer.source(),
            Format::Xrgb8888,
            &[b],
        )
        .unwrap();
    let third = compositor.frame();
    assert_eq!(third.buffer, first.buffer);
    assert!(!paint.last_present().new_image);
    assert_eq!(paint.last_present().operations, 2, "{b:?} and {a:?}");
    assert_eq!(third.acquire, first.release + 1);
    assert_eq!(compositor.pixels(&third), drawer.pixels);

    // No image: refused before anything is sent.
    let none = Source {
        image: vk::Image::null(),
        width: WIDTH,
        height: HEIGHT,
    };
    assert!(paint
        .present(&mut client, surface, 4, none, Format::Xrgb8888, &[])
        .is_err());
    paint.forget_surface(&mut client, surface);
    for _ in 0..2 {
        assert!(matches!(
            compositor.connection.recv_request().unwrap().0,
            Request::DestroyBuffer { .. }
        ));
        assert!(matches!(
            compositor.connection.recv_request().unwrap().0,
            Request::DestroyTimeline { .. }
        ));
    }
    drawer.finish();
}
