//! The Vulkan device a native client draws on and the surfaces' GPU buffers it presents in
//! (docs/adr/0031-gpu-buffers.mdx, decision 12): the Bloom host's Skia (Ganesh on Vulkan) is given
//! this device ([`Paint::vulkan`]) and draws each frame into an image of its own; [`Paint::present`]
//! copies what changed of it into one of the surface's exportable images and hands that to the
//! compositor as a DMA-BUF with explicit synchronization.
//!
//! - [`Paint::new`]: the device the compositor composes on (its render node, from the client's GPU
//!   table), or another one that exports linear DMA-BUFs, with a graphics queue and every core
//!   feature it has but `robustBufferAccess`.
//! - [`Paint::present`]: the source image copied (what changed since each buffer last held it) into
//!   one of the surface's exportable images, a modifier from the compositor's table, attached once,
//!   each with a timeline of its own; the copy's completion goes into its acquire point and the
//!   buffer is copied into again only once its release point has signalled.

#![allow(unsafe_code)]

/// Vulkan's types, for callers handing the painter their images.
pub use ash::vk;
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
/// release waited for, the present before waited for, the copy recorded, submitted, the acquire
/// point given its fence, the frame sent; and whether it made an image.
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
    /// The regions copied: what the buffer lacked (one, the whole frame, for a new buffer).
    pub operations: u32,
}

/// The painter's Vulkan objects, for a renderer that draws on its device (Skia's Ganesh, ADR 0031,
/// decision 12). They live as long as the painter; the queue is the painter's too, so the
/// renderer and the painter use it from one thread.
#[derive(Clone, Copy)]
pub struct Vulkan<'a> {
    pub get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    pub instance: vk::Instance,
    pub physical_device: vk::PhysicalDevice,
    pub device: vk::Device,
    pub queue: vk::Queue,
    pub queue_family: u32,
    /// The version the instance was created with.
    pub api_version: u32,
    /// The physical device's own version.
    pub device_api_version: u32,
    pub instance_extensions: &'a [&'static CStr],
    pub device_extensions: &'a [&'static CStr],
    /// What the device was created with: every core feature the device has but
    /// `robustBufferAccess`; no chain after it.
    pub features: &'a vk::PhysicalDeviceFeatures2<'static>,
}

/// A frame to present: an image of `width` × `height` (B8G8R8A8_UNORM, optimal tiling, usable as
/// a transfer source) in `TRANSFER_SRC_OPTIMAL` on the painter's queue family, its drawing already
/// submitted to the painter's queue. It is left so: in that layout, on that family.
#[derive(Clone, Copy, Debug)]
pub struct Source {
    pub image: vk::Image,
    pub width: u32,
    pub height: u32,
}

pub struct Paint {
    pub(crate) gpu: present::Gpu,
    commands: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    /// Whether the command buffer's last submission may still run (its fence not waited for).
    submitted: bool,
    /// What is presented: each surface's buffers.
    surfaces: HashMap<u32, present::Swapchain>,
    /// Where the last present's time went.
    pub(crate) last_present: PresentTimes,
}

impl Paint {
    /// A painter on the device whose render node is `device` (the compositor's, its GPU table's),
    /// else on any device that exports DMA-BUF, with linear buffers.
    pub fn new(device: u64) -> Result<Self> {
        let gpu = present::Gpu::open(device)?;
        let mut paint = Self {
            gpu,
            commands: vk::CommandPool::null(),
            command: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            submitted: false,
            surfaces: HashMap::new(),
            last_present: PresentTimes::default(),
        };
        let device = &paint.gpu.device;
        // SAFETY: the device is live; every info outlives its call; each object is stored in
        // `paint` at once, so `Drop` destroys it.
        unsafe {
            paint.commands = device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(paint.gpu.family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?;
            paint.command = device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(paint.commands)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0];
            paint.fence = device.create_fence(&vk::FenceCreateInfo::default(), None)?;
        }
        Ok(paint)
    }

    /// The device's name, as its driver gives it.
    pub fn device_name(&self) -> &str {
        &self.gpu.name
    }

    /// Whether it exports DMA-BUFs to present in.
    pub fn exports(&self) -> bool {
        !self.gpu.exportable_is_empty()
    }

    /// Whether it is the compositor's own device (tiled modifiers may be shared).
    pub fn same_device(&self) -> bool {
        self.gpu.same_device
    }

    /// The painter's Vulkan objects, for the renderer that draws the frames.
    pub fn vulkan(&self) -> Vulkan<'_> {
        self.gpu.vulkan()
    }

    /// Waits for the copy in flight.
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
        Ok(())
    }

    /// Presents `source` on `surface` in answer to `configure`, as `format` (Argb8888
    /// premultiplied or Xrgb8888): what changed since a buffer of its last held it (`damage`,
    /// source pixels; none: everything) copied into one of the surface's GPU buffers, submitted
    /// with its acquire and release points. Returns the frame's sequence number. An error means
    /// the GPU path cannot go on (a buffer the compositor failed to import, the device lost): the
    /// caller presents in shared memory from then on.
    pub fn present(
        &mut self,
        client: &mut Client,
        surface: u32,
        configure: u64,
        source: Source,
        format: Format,
        damage: &[Rect],
    ) -> Result<u64> {
        present::present(self, client, surface, configure, source, format, damage)
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
        // SAFETY: as above.
        unsafe {
            let device = &self.gpu.device;
            device.destroy_fence(self.fence, None);
            device.destroy_command_pool(self.commands, None);
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
