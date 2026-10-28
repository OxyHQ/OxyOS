//! Shows frames on a real display through DRM/KMS: the frames `oxy-offscreen --launch
//! --dump-frames DIR` wrote (the Bloom regression's), each composed with the compositor's
//! reference and centred on the first connected output, one per refresh, page flip after page
//! flip, for a few seconds. Reports the mode and the presentation measured by the kernel's flip
//! timestamps: intervals and missed refreshes.
//!
//! Needs DRM master: run it from a free virtual terminal (the desktop session releases the
//! display when its terminal is left) or as root after switching to one.
//!
//! With `--gpu` the frames are composed on the Vulkan device straight into two buffers exported
//! as DMA-BUF and imported as framebuffers: no copy through the CPU.
//!
//!     oxy-kms-show [--gpu] FRAMES_DIR [/dev/dri/cardN] [SECONDS]

use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1).peekable();
    let gpu = args.next_if(|arg| arg == "--gpu").is_some();
    let dir = PathBuf::from(
        args.next()
            .ok_or("usage: oxy-kms-show FRAMES_DIR [CARD] [SECONDS]")?,
    );
    let card_path = PathBuf::from(args.next().unwrap_or_else(|| "/dev/dri/card1".into()));
    let seconds: u64 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(5);

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "oxyframe"))
        .collect();
    files.sort();
    let frames = files
        .iter()
        .map(|path| {
            oxy_gpu::frame_file::read(&mut std::io::BufReader::new(std::fs::File::open(path)?))
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    if frames.is_empty() {
        return Err("no frames".into());
    }

    let card = oxy_kms::Card::open(&card_path)?;
    let driver = card.driver_name()?;
    let mut display = oxy_kms::Display::open(&card)?;
    let (width, height) = display.size();
    let refresh = Duration::from_secs_f64(1.0 / f64::from(display.refresh_hz().max(1)));

    // On the GPU: two scanout targets, imported as framebuffers.
    let mut vulkan = None;
    let mut targets = Vec::new();
    if gpu {
        let mut compositor = oxy_gpu::VulkanCompositor::new()?;
        for target in compositor.scanout_targets(2, width, height)? {
            use std::os::fd::AsFd;
            targets.push(card.import_dmabuf(
                target.fd.as_fd(),
                target.width,
                target.height,
                target.pitch,
            )?);
        }
        vulkan = Some(compositor);
    }
    let (mut flips, mut compose_ms, mut device_ms) = (Vec::new(), Vec::new(), Vec::new());
    let started = Instant::now();
    let mut index = 0;
    while started.elapsed() < Duration::from_secs(seconds) {
        let frame = &frames[index % frames.len()];
        // Centred: the frame's output was frame.width × frame.height.
        let dx = (width as i32 - frame.width as i32) / 2;
        let dy = (height as i32 - frame.height as i32) / 2;
        let layers: Vec<oxy_gpu::Layer<'_>> = frame
            .layers()
            .into_iter()
            .map(|layer| oxy_gpu::Layer {
                x: layer.x + dx,
                y: layer.y + dy,
                ..layer
            })
            .collect();
        let composing = Instant::now();
        if let Some(compositor) = &mut vulkan {
            let back = index % 2;
            compositor.compose_to_scanout(back, frame.background, &layers)?;
            compose_ms.push(composing.elapsed().as_secs_f64() * 1e3);
            device_ms.extend(compositor.last_gpu_ms());
            display.present_framebuffer(targets[back])?;
        } else {
            let pixels = oxy_gpu::compose_cpu(width, height, frame.background, &layers);
            compose_ms.push(composing.elapsed().as_secs_f64() * 1e3);
            display.draw(&pixels)?;
            display.present()?;
        }
        flips.push(display.wait_flip()?);
        index += 1;
    }
    drop(display);
    let device = vulkan.as_ref().map(|c| format!("{:?}", c.device_name()));
    drop(vulkan);

    let mut intervals: Vec<f64> = flips
        .windows(2)
        .map(|w| (w[1] - w[0]).as_secs_f64() * 1e3)
        .collect();
    let missed = intervals
        .iter()
        .filter(|&&ms| ms > refresh.as_secs_f64() * 1e3 * 1.5)
        .count();
    let median = |values: &mut Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values.get(values.len() / 2).copied().unwrap_or(0.0)
    };
    let max = intervals.iter().copied().fold(0.0, f64::max);
    println!(
        "{{\"card\":{:?},\"driver\":{driver:?},\"mode\":{:?},\"presented\":{},\"distinct_frames\":{},\"flip_interval_ms_median\":{:.3},\"flip_interval_ms_max\":{:.3},\"missed_refreshes\":{missed},\"composition\":{},\"compose_ms_median\":{:.2},\"gpu_compose_ms_median\":{}}}",
        card_path.display().to_string(),
        oxy_kms_mode(width, height, refresh),
        flips.len(),
        frames.len(),
        median(&mut intervals),
        max,
        device.map_or("\"cpu\"".to_string(), |name| format!("{{\"vulkan\":{name},\"zero_copy\":true}}")),
        median(&mut compose_ms),
        if device_ms.is_empty() { "null".to_string() } else { format!("{:.3}", median(&mut device_ms)) },
    );
    Ok(())
}

fn oxy_kms_mode(width: u32, height: u32, refresh: Duration) -> String {
    format!("{width}x{height}@{:.0}", 1.0 / refresh.as_secs_f64())
}
