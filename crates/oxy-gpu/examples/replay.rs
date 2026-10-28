//! Composes frames written by `oxy-offscreen --launch --dump-frames DIR` again, on the CPU
//! reference and on the best Vulkan device here, and checks that every frame matches byte for
//! byte. Reports the device and the composition times (Vulkan: upload, dispatch and readback).
//!
//!     cargo run --release -p oxy-gpu --example replay -- DIR

use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args().nth(1).ok_or("usage: replay DIR")?;
    let mut files: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "oxyframe"))
        .collect();
    files.sort();
    let mut gpu = oxy_gpu::VulkanCompositor::new()?;
    let (mut cpu_ms, mut gpu_ms, mut mismatches) = (Vec::new(), Vec::new(), Vec::new());
    for path in &files {
        let frame =
            oxy_gpu::frame_file::read(&mut std::io::BufReader::new(std::fs::File::open(path)?))?;
        let layers = frame.layers();
        let started = Instant::now();
        let cpu = oxy_gpu::compose_cpu(frame.width, frame.height, frame.background, &layers);
        cpu_ms.push(started.elapsed().as_secs_f64() * 1e3);
        let started = Instant::now();
        let vulkan = gpu.compose(frame.width, frame.height, frame.background, &layers)?;
        gpu_ms.push(started.elapsed().as_secs_f64() * 1e3);
        if vulkan != cpu {
            mismatches.push(path.display().to_string());
        }
    }
    // The same output with no layers: what upload of the background, an empty submission and
    // readback cost alone.
    let mut empty_ms = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        gpu.compose(1280, 720, 0, &[])?;
        empty_ms.push(started.elapsed().as_secs_f64() * 1e3);
    }
    let median = |values: &mut Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values.get(values.len() / 2).copied().unwrap_or(0.0)
    };
    println!(
        "{{\"device\":{:?},\"kind\":\"{:?}\",\"frames\":{},\"mismatches\":{},\"cpu_ms_median\":{:.2},\"vulkan_ms_median\":{:.2},\"vulkan_empty_frame_ms_median\":{:.2}}}",
        gpu.device_name(),
        gpu.device_kind(),
        files.len(),
        mismatches.len(),
        median(&mut cpu_ms),
        median(&mut gpu_ms),
        median(&mut empty_ms)
    );
    for path in &mismatches {
        eprintln!("differs: {path}");
    }
    if mismatches.is_empty() && !files.is_empty() {
        Ok(())
    } else {
        Err("frames differ or none were found".into())
    }
}
