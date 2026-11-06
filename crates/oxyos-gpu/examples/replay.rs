//! Composes frames written by `oxyos-offscreen --launch --dump-frames DIR` again, on the CPU
//! reference and on the best Vulkan device here, and checks that every frame matches byte for
//! byte. Reports the device and the composition times: the CPU's, Vulkan's from the host
//! (upload, composition and readback), an empty frame's, and the device's own composing time
//! (GPU timestamps).
//!
//!     cargo run --release -p oxyos-gpu --example replay -- DIR [--repeat N]
//!
//! With `--repeat N`, each frame is also composed N times more, whole and with the damage a
//! session gives a frame (a button's, a window's, a window dragged, a few scattered changes),
//! back to back and once a refresh (16.7 ms apart, as a 60 Hz session composes, the device's
//! clock left to fall between frames), every damaged frame checked against the reference: one
//! line of the device's time (median, ms) for each frame and each, and their medians over the
//! frames.

use oxyos_gpu::{Layer, Rect, Source};
use std::time::{Duration, Instant};

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}

/// The damage a session's frames have, on a `width` × `height` frame whose topmost window-like
/// layer (smaller than the frame) is `window`: (name, rectangles).
fn damages(width: u32, height: u32, window: Option<Rect>) -> Vec<(&'static str, Vec<Rect>)> {
    let rect = |x: i32, y: i32, width: u32, height: u32| Rect {
        x,
        y,
        width,
        height,
    };
    let (cx, cy) = (width as i32 / 2, height as i32 / 2);
    let window = window.unwrap_or(rect(cx - 400, cy - 250, 800, 500));
    let moved = Rect {
        x: window.x + 6,
        y: window.y + 4,
        ..window
    };
    vec![
        ("button", vec![rect(cx - 48, cy - 20, 96, 40)]),
        ("window", vec![window]),
        ("drag", vec![window, moved]),
        (
            "scattered",
            vec![
                rect(40, 8, 120, 24),
                rect(width as i32 - 200, 8, 160, 24),
                rect(cx - 60, cy - 20, 120, 40),
                rect(cx + 200, cy + 100, 64, 64),
                rect(cx - 300, height as i32 - 90, 600, 80),
            ],
        ),
    ]
}

/// Where `out` first differs from the reference `cpu`, and in how many pixels.
fn difference(out: &[u32], cpu: &[u32], width: u32) -> String {
    let differing: Vec<usize> = (0..out.len().min(cpu.len()))
        .filter(|&at| out[at] != cpu[at])
        .collect();
    match differing.first() {
        Some(&at) => format!(
            "{} pixels, the first at ({}, {}): {:#08x}, not {:#08x}",
            differing.len(),
            at % width as usize,
            at / width as usize,
            out[at],
            cpu[at]
        ),
        None => format!("{} pixels, not {}", out.len(), cpu.len()),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().ok_or("usage: replay DIR [--repeat N]")?;
    let repeat: usize = match (args.next().as_deref(), args.next()) {
        (Some("--repeat"), Some(n)) => n.parse()?,
        (None, _) => 0,
        _ => return Err("usage: replay DIR [--repeat N]".into()),
    };
    let mut files: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "oxyframe"))
        .collect();
    files.sort();
    let mut gpu = oxyos_gpu::VulkanCompositor::new()?;
    let (mut cpu_ms, mut gpu_ms, mut mismatches) = (Vec::new(), Vec::new(), Vec::new());
    // The device's own time composing (GPU timestamps), without uploads and readback.
    let mut device_ms = Vec::new();
    // Each way of composing again: the median of every frame's median.
    let mut repeated: Vec<(String, Vec<f64>)> = Vec::new();
    for (number, path) in files.iter().enumerate() {
        let frame =
            oxyos_gpu::frame_file::read(&mut std::io::BufReader::new(std::fs::File::open(path)?))?;
        // Kept on the device from one composition to the next, as a session's are: each frame's
        // layers their own sources.
        let layers: Vec<Layer<'_>> = frame
            .layers()
            .into_iter()
            .enumerate()
            .map(|(index, layer)| Layer {
                source: Some(Source {
                    key: ((number as u64) << 32) | index as u64,
                    version: 1,
                }),
                ..layer
            })
            .collect();
        let started = Instant::now();
        let cpu = oxyos_gpu::compose_cpu(frame.width, frame.height, frame.background, &layers);
        cpu_ms.push(started.elapsed().as_secs_f64() * 1e3);
        let started = Instant::now();
        let vulkan = gpu.compose(frame.width, frame.height, frame.background, &layers)?;
        gpu_ms.push(started.elapsed().as_secs_f64() * 1e3);
        device_ms.extend(gpu.last_gpu_ms());
        if vulkan != cpu {
            mismatches.push(path.display().to_string());
        }
        if repeat == 0 {
            continue;
        }
        let whole = Rect {
            x: 0,
            y: 0,
            width: frame.width,
            height: frame.height,
        };
        let window = layers
            .iter()
            .rev()
            .map(|layer| Rect {
                x: layer.x,
                y: layer.y,
                width: layer.width,
                height: layer.height,
            })
            .filter_map(|rect| rect.intersect(&whole))
            .find(|rect| rect.width >= 200 && rect.height >= 150 && *rect != whole);
        let mut ways: Vec<(String, Option<Vec<Rect>>)> = vec![("whole".into(), None)];
        ways.extend(
            damages(frame.width, frame.height, window)
                .into_iter()
                .map(|(name, rects)| (name.to_string(), Some(rects))),
        );
        let mut line = format!(
            "{{\"frame\":{:?},\"layers\":{}",
            path.file_name().unwrap_or_default(),
            layers.len()
        );
        for (paced, apart) in [
            ("", Duration::ZERO),
            (" paced", Duration::from_micros(16_667)),
        ] {
            for (name, damage) in &ways {
                let mut times = Vec::new();
                for _ in 0..repeat {
                    let out = match damage {
                        None => {
                            gpu.compose(frame.width, frame.height, frame.background, &layers)?
                        }
                        Some(rects) => gpu.compose_damaged(
                            frame.width,
                            frame.height,
                            frame.background,
                            &layers,
                            rects,
                        )?,
                    };
                    if out != cpu {
                        mismatches.push(format!(
                            "{} ({name}): {}",
                            path.display(),
                            difference(&out, &cpu, frame.width)
                        ));
                    }
                    times.extend(gpu.last_gpu_ms());
                    std::thread::sleep(apart);
                }
                let name = format!("{name}{paced}");
                let ms = median(&mut times);
                line += &format!(",{name:?}:{ms:.3}");
                match repeated.iter_mut().find(|(way, _)| *way == name) {
                    Some((_, all)) => all.push(ms),
                    None => repeated.push((name, vec![ms])),
                }
            }
        }
        println!("{line}}}");
    }
    // The same output with no layers: what upload of the background, an empty submission and
    // readback cost alone.
    let mut empty_ms = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        gpu.compose(1280, 720, 0, &[])?;
        empty_ms.push(started.elapsed().as_secs_f64() * 1e3);
    }
    let mut summary = format!(
        "{{\"device\":{:?},\"kind\":\"{:?}\",\"frames\":{},\"mismatches\":{},\"cpu_ms_median\":{:.2},\"vulkan_ms_median\":{:.2},\"vulkan_empty_frame_ms_median\":{:.2},\"device_compose_ms_median\":{:.3}",
        gpu.device_name(),
        gpu.device_kind(),
        files.len(),
        mismatches.len(),
        median(&mut cpu_ms),
        median(&mut gpu_ms),
        median(&mut empty_ms),
        median(&mut device_ms)
    );
    for (name, mut all) in repeated {
        summary += &format!(",\"device_ms {name}\":{:.3}", median(&mut all));
    }
    println!("{summary}}}");
    for path in &mismatches {
        eprintln!("differs: {path}");
    }
    if mismatches.is_empty() && !files.is_empty() {
        Ok(())
    } else {
        Err("frames differ or none were found".into())
    }
}
