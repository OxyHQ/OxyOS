//! What filtered sampling costs (docs/adr/0011-transitions.mdx, step 5): a 1920×1200 output
//! with a full-size window (opaque, its corners rounded) drawn at the scales its transitions pass
//! through (a minimize into the dock down to a tenth, a window arriving or going at 96 %, an
//! overlay at 104 %, a zoom to twice), by nearest neighbour and filtered. Reports, per scale, the
//! device's own time composing the frame (GPU timestamps; the window's pixels stay on the device,
//! as a transition's do) and the CPU reference's, medians of the frames composed.
//!
//!     cargo run --release -p oxyos-gpu --example sampling [-- FRAMES]

use oxyos_gpu::{compose_cpu, Filter, Layer, Paint, Source, VulkanCompositor};
use std::time::Instant;

const OUTPUT: (u32, u32) = (1920, 1200);

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(f64::NAN)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frames: usize = std::env::args().nth(1).map_or(Ok(20), |n| n.parse())?;
    let mut gpu = VulkanCompositor::new()?;
    println!(
        "{} ({:?}), {}x{}, {frames} frames each",
        gpu.device_name(),
        gpu.device_kind(),
        OUTPUT.0,
        OUTPUT.1
    );
    let extent = OUTPUT;
    let window: Vec<u8> = (0..extent.0 * extent.1)
        .flat_map(|i: u32| (0xff00_0000 | (i.wrapping_mul(2_654_435_761) >> 8)).to_le_bytes())
        .collect();
    println!("scale  filter    device ms  CPU ms");
    for scale in [1.0, 0.96, 0.5, 0.25, 0.1, 1.04, 2.0] {
        let size = |length: u32| ((f64::from(length) * scale).round() as u32).max(1);
        let (width, height) = (size(extent.0), size(extent.1));
        for filter in [Filter::Nearest, Filter::Box] {
            let layer = Layer {
                x: (OUTPUT.0 as i32 - width as i32) / 2,
                y: (OUTPUT.1 as i32 - height as i32) / 2,
                width,
                height,
                extent,
                paint: Paint::Opaque(&window),
                source: Some(Source { key: 1, version: 1 }),
                changed: None,
                opacity: 230,
                radius: 16,
                filter,
                clip: None,
            };
            let (mut device, mut cpu) = (Vec::new(), Vec::new());
            // The first frame uploads the window; the others compose it from the device.
            gpu.compose(OUTPUT.0, OUTPUT.1, 0x0020_3040, &[layer])?;
            for _ in 0..frames {
                gpu.compose(OUTPUT.0, OUTPUT.1, 0x0020_3040, &[layer])?;
                device.extend(gpu.last_gpu_ms());
                let started = Instant::now();
                let out = compose_cpu(OUTPUT.0, OUTPUT.1, 0x0020_3040, &[layer]);
                cpu.push(started.elapsed().as_secs_f64() * 1e3);
                std::hint::black_box(out);
            }
            println!(
                "{scale:<6} {:<9} {:>9.3}  {:>6.2}",
                format!("{filter:?}"),
                median(device),
                median(cpu)
            );
        }
    }
    Ok(())
}
