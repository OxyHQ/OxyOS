//! Opens the GPU as the session and a Bloom host do (docs/adr/0031-gpu-buffers.mdx, decision 14),
//! outside a session: the compositor's Vulkan device on the GPU that drives the displays (or on
//! the node given), then a painter on the node it would name to its clients. Run it under
//! `strace -f -e trace=openat` to see that no other GPU's node is opened.
//!
//!   cargo run --release --example compositors-gpu -p oxyos-paint [-- /dev/dri/renderD129]

use std::os::unix::fs::MetadataExt;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let device = match std::env::args().nth(1) {
        // A node's number, from its metadata: the node itself is not opened.
        Some(node) => std::fs::metadata(&node)?.rdev(),
        None => oxyos_gpu_device::display_device(Path::new(oxyos_gpu_device::SYSTEM_ROOT)),
    };
    println!(
        "drivers: {}",
        oxyos_gpu_device::Drivers::for_device(device).describe()
    );
    let compositor = oxyos_gpu::VulkanCompositor::on_device(device)?;
    let named = compositor.render_node().unwrap_or(0);
    println!(
        "compositor: {} ({:?}), naming {:?} to its clients",
        compositor.device_name(),
        compositor.device_kind(),
        oxyos_gpu_device::major_minor(named)
    );
    let paint = oxyos_paint::Paint::new(named)?;
    println!(
        "painter: {}, the compositor's device: {}, exports DMA-BUF: {}",
        paint.device_name(),
        paint.same_device(),
        paint.exports()
    );
    Ok(())
}
