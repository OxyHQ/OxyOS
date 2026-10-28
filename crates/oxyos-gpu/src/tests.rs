use super::*;

/// A layer's pixels from 0xAARRGGBB words.
fn pixels(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// A frame like the compositor's: a scaled opaque window partly off the output, a premultiplied
/// popup with transparent and translucent pixels over it, and a cover.
fn frame() -> (Vec<u8>, Vec<u8>) {
    let window: Vec<u32> = (0..64 * 48)
        .map(|i: u32| 0xff00_0000 | (i.wrapping_mul(2_654_435_761) >> 8))
        .collect();
    let popup: Vec<u32> = (0..20 * 30)
        .map(|i| match i % 3 {
            0 => 0x0000_0000,   // transparent corner
            1 => 0x80_40_20_10, // translucent, premultiplied
            _ => 0xff_ff_80_00, // opaque
        })
        .collect();
    (pixels(&window), pixels(&popup))
}

fn layers<'a>(window: &'a [u8], popup: &'a [u8]) -> Vec<Layer<'a>> {
    vec![
        // 64x48 pixels shown at 96x72 (a 1.5x buffer at half size would be the reverse).
        Layer {
            x: -10,
            y: 5,
            width: 96,
            height: 72,
            extent: (64, 48),
            paint: Paint::Opaque(window),
            source: None,
            changed: None,
        },
        Layer {
            x: 70,
            y: 40,
            width: 20,
            height: 30,
            extent: (20, 30),
            paint: Paint::Premultiplied(popup),
            source: None,
            changed: None,
        },
        Layer {
            x: 100,
            y: 0,
            width: 30,
            height: 200,
            extent: (1, 1),
            paint: Paint::Fill(0x0012_3456),
            source: None,
            changed: None,
        },
    ]
}

#[test]
fn the_reference_copies_scales_blends_and_clips() {
    let (window, popup) = frame();
    let out = compose_cpu(120, 90, 0x0011_2233, &layers(&window, &popup));
    assert_eq!(out.len(), 120 * 90);
    // Outside every layer: the background.
    assert_eq!(out[0], 0x0011_2233);
    // A cover fills its rectangle, clipped to the output.
    assert_eq!(out[100], 0x0012_3456);
    assert_eq!(out[89 * 120 + 119], 0x0012_3456);
    // The window's first visible pixel is source (6, 0) scaled: (0 - -10) * 64 / 96 = 6.
    let source = u32::from_le_bytes(window[6 * 4..6 * 4 + 4].try_into().unwrap());
    assert_eq!(out[5 * 120], source & 0x00ff_ffff);
    // Popup: a transparent pixel keeps the window, an opaque one replaces it.
    let under = out[40 * 120 + 70];
    let alone = compose_cpu(120, 90, 0x0011_2233, &layers(&window, &popup)[..1]);
    assert_eq!(under, alone[40 * 120 + 70]);
    assert_eq!(out[40 * 120 + 72], 0x00ff_8000);
    // A translucent pixel: source + under * (255 - 0x80) / 255 per channel.
    let d = alone[40 * 120 + 71];
    let blend = |s: u32, shift: u32| (s + ((d >> shift) & 0xff) * 127 / 255).min(255) << shift;
    assert_eq!(
        out[40 * 120 + 71],
        blend(0x40, 16) | blend(0x20, 8) | blend(0x10, 0)
    );
}

#[test]
fn invalid_layers_are_skipped() {
    let short = [0u8; 8];
    let layers = [
        Layer {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
            extent: (4, 4),
            paint: Paint::Opaque(&short),
            source: None,
            changed: None,
        },
        Layer {
            x: 0,
            y: 0,
            width: 0,
            height: 4,
            extent: (1, 1),
            paint: Paint::Fill(1),
            source: None,
            changed: None,
        },
    ];
    assert_eq!(compose_cpu(4, 4, 7, &layers), vec![7; 16]);
}

/// The Vulkan composition matches the reference byte for byte, on whatever device is present.
/// Without a Vulkan device the test says so and passes: CI has none; `oxyos-offscreen --gpu`
/// is the end-to-end check where one exists.
#[test]
fn vulkan_matches_the_reference() {
    let mut gpu = match VulkanCompositor::new() {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            return;
        }
    };
    eprintln!(
        "composing on {} ({:?})",
        gpu.device_name(),
        gpu.device_kind()
    );
    let (window, popup) = frame();
    let layers = layers(&window, &popup);
    for (width, height) in [(120, 90), (7, 3), (1280, 720)] {
        let cpu = compose_cpu(width, height, 0x0011_2233, &layers);
        let vulkan = gpu.compose(width, height, 0x0011_2233, &layers).unwrap();
        assert_eq!(vulkan, cpu, "{width}x{height}");
    }
    // Buffers grow and are reused across frames.
    let empty = gpu.compose(16, 16, 5, &[]).unwrap();
    assert_eq!(empty, vec![5; 256]);
}

/// A frame composed into a scanout target and kept reads back as the reference composed it,
/// once; a later composition replaces it. Where the device exports no DMA-BUF, it says so.
#[cfg(target_os = "linux")]
#[test]
fn a_kept_scanout_frame_is_the_reference() {
    let mut gpu = match VulkanCompositor::new() {
        Ok(gpu) if gpu.exports_dma_buf() => gpu,
        Ok(gpu) => {
            eprintln!("{} exports no DMA-BUF, skipped", gpu.device_name());
            return;
        }
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            return;
        }
    };
    let (window, popup) = frame();
    let layers = layers(&window, &popup);
    let _targets = gpu.scanout_targets(2, 120, 90).unwrap();
    assert!(gpu.read_kept().is_err(), "nothing kept yet");
    for index in [0, 1, 0] {
        gpu.compose_to_scanout_kept(index, 0x0011_2233, &layers, None)
            .unwrap();
        assert_eq!(
            gpu.read_kept().unwrap(),
            compose_cpu(120, 90, 0x0011_2233, &layers),
            "target {index}"
        );
        assert!(gpu.read_kept().is_err(), "a kept frame is read once");
    }
    gpu.compose_to_scanout_kept(1, 0x0011_2233, &layers, None)
        .unwrap();
    gpu.compose(8, 8, 1, &[]).unwrap();
    assert!(gpu.read_kept().is_err(), "a later composition replaced it");
}

#[test]
fn rectangles_intersect_and_bound() {
    let a = Rect {
        x: -5,
        y: 0,
        width: 10,
        height: 10,
    };
    let b = Rect {
        x: 3,
        y: 4,
        width: 10,
        height: 2,
    };
    assert_eq!(
        a.intersect(&b),
        Some(Rect {
            x: 3,
            y: 4,
            width: 2,
            height: 2
        })
    );
    let apart = Rect {
        x: 5,
        y: 0,
        width: 1,
        height: 1,
    };
    assert_eq!(a.intersect(&apart), None, "touching edges share nothing");
    assert_eq!(
        a.bounding(&b),
        Rect {
            x: -5,
            y: 0,
            width: 18,
            height: 10
        }
    );
}

/// The popup of `layers` moved by (dx, dy), and what that damages: where it was and where it is.
fn popup_moved<'a>(layers: &[Layer<'a>], dx: i32, dy: i32) -> (Vec<Layer<'a>>, Vec<Rect>) {
    let mut moved = layers.to_vec();
    let before = Rect {
        x: moved[1].x,
        y: moved[1].y,
        width: moved[1].width,
        height: moved[1].height,
    };
    moved[1].x += dx;
    moved[1].y += dy;
    let after = Rect {
        x: moved[1].x,
        y: moved[1].y,
        ..before
    };
    (moved, vec![before, after])
}

/// Composing only what changed gives the frame the reference composes whole; damage that
/// leaves out where the popup was leaves it there, so the limit is real.
#[test]
fn damaged_composition_is_the_reference() {
    let mut gpu = match VulkanCompositor::new() {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            return;
        }
    };
    let (window, popup) = frame();
    let first = layers(&window, &popup);
    // No frame yet: composed whole whatever the damage says.
    assert_eq!(
        gpu.compose_damaged(120, 90, 0x0011_2233, &first, &[])
            .unwrap(),
        compose_cpu(120, 90, 0x0011_2233, &first)
    );
    let (second, damage) = popup_moved(&first, 15, -20);
    assert_eq!(
        gpu.compose_damaged(120, 90, 0x0011_2233, &second, &damage)
            .unwrap(),
        compose_cpu(120, 90, 0x0011_2233, &second)
    );
    let (third, damage) = popup_moved(&second, -30, 25);
    let too_small = gpu
        .compose_damaged(120, 90, 0x0011_2233, &third, &damage[1..])
        .unwrap();
    assert_ne!(
        too_small,
        compose_cpu(120, 90, 0x0011_2233, &third),
        "the popup's old place was not composed again"
    );
    // A frame of another size holds nothing of this one: composed whole.
    assert_eq!(
        gpu.compose_damaged(7, 3, 0x0011_2233, &first, &[]).unwrap(),
        compose_cpu(7, 3, 0x0011_2233, &first)
    );
}

/// Scanout targets alternate: each frame composes its damage and what changed since its target
/// was last written, and every frame shown is the reference's.
#[cfg(target_os = "linux")]
#[test]
fn damaged_scanout_frames_are_the_reference() {
    let mut gpu = match VulkanCompositor::new() {
        Ok(gpu) if gpu.exports_dma_buf() => gpu,
        Ok(gpu) => {
            eprintln!("{} exports no DMA-BUF, skipped", gpu.device_name());
            return;
        }
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            return;
        }
    };
    let (window, popup) = frame();
    let _targets = gpu.scanout_targets(2, 120, 90).unwrap();
    let mut frame = layers(&window, &popup);
    let mut damage: Option<Vec<Rect>> = None;
    for (index, step) in [
        (0, (0, 0)),
        (1, (12, 9)),
        (0, (-25, 14)),
        (1, (8, -30)),
        (0, (3, 3)),
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            let (moved, changed) = popup_moved(&frame, step.1 .0, step.1 .1);
            frame = moved;
            damage = Some(changed);
        }
        gpu.compose_to_scanout_kept(step.0, 0x0011_2233, &frame, damage.as_deref())
            .unwrap();
        assert_eq!(
            gpu.read_kept().unwrap(),
            compose_cpu(120, 90, 0x0011_2233, &frame),
            "frame {index} on target {}",
            step.0
        );
    }
}

/// Layers whose source the device keeps are not uploaded again, and every frame is still the
/// reference's: with the same pixels, with a new version of them, and after the source buffer
/// filled with others and was reclaimed.
#[test]
fn kept_sources_are_uploaded_once() {
    let mut gpu = match VulkanCompositor::new() {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            return;
        }
    };
    let (window, popup) = frame();
    fn sourced(layers: Vec<Layer<'_>>, version: u64) -> Vec<Layer<'_>> {
        layers
            .into_iter()
            .enumerate()
            .map(|(index, layer)| Layer {
                source: Some(Source {
                    key: index as u64,
                    version,
                }),
                ..layer
            })
            .collect()
    }
    let first = sourced(layers(&window, &popup), 1);
    let check = |gpu: &mut VulkanCompositor, frame: &[Layer<'_>]| {
        assert_eq!(
            gpu.compose(120, 90, 0x0011_2233, frame).unwrap(),
            compose_cpu(120, 90, 0x0011_2233, frame)
        );
        gpu.last_uploaded_bytes()
    };
    let all = ((64 * 48) + (20 * 30)) * 4;
    assert_eq!(
        check(&mut gpu, &first),
        all,
        "the first frame uploads everything"
    );
    assert_eq!(
        check(&mut gpu, &first),
        0,
        "the same versions upload nothing"
    );
    // The popup's pixels change: a new version of it, the window's kept.
    let changed: Vec<u32> = (0..20 * 30).map(|i| 0xff00_0000 | i).collect();
    let changed = pixels(&changed);
    let mut second = first.clone();
    second[1] = Layer {
        paint: Paint::Premultiplied(&changed),
        source: Some(Source { key: 1, version: 2 }),
        ..second[1]
    };
    assert_eq!(check(&mut gpu, &second), 20 * 30 * 4);
    // Many other sources fill the buffer: it is reclaimed, and kept ones are uploaded again.
    for key in 10..40 {
        let other = [Layer {
            source: Some(Source { key, version: 1 }),
            ..second[0]
        }];
        check(&mut gpu, &other);
    }
    check(&mut gpu, &second);
    assert_eq!(check(&mut gpu, &second), 0);
}

/// Pixels the device holds at the version they changed from are updated with only what changed:
/// the frame is the reference's and only those rows go up; a change left out keeps its old pixels
/// on the device, so only the rows named are copied.
#[test]
fn changed_rows_alone_are_uploaded() {
    let mut gpu = match VulkanCompositor::new() {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            return;
        }
    };
    let width = 16u32;
    let height = 8u32;
    let words = |value: u32| vec![0xff00_0000 | value; (width * height) as usize];
    fn window<'a>(pixels: &'a [u8], version: u64, changed: Option<(u64, &'a [Rect])>) -> Layer<'a> {
        Layer {
            x: 0,
            y: 0,
            width: 16,
            height: 8,
            extent: (16, 8),
            paint: Paint::Opaque(pixels),
            source: Some(Source { key: 7, version }),
            changed,
        }
    }
    let first = pixels(&words(1));
    let frame = [window(&first, 1, None)];
    assert_eq!(
        gpu.compose(width, height, 0, &frame).unwrap(),
        compose_cpu(width, height, 0, &frame)
    );
    // Version 2 changes a 3x2 block at (4, 5).
    let block = Rect {
        x: 4,
        y: 5,
        width: 3,
        height: 2,
    };
    let mut second = words(1);
    for y in 5..7 {
        for x in 4..7 {
            second[(y * width + x) as usize] = 0xff00_0002;
        }
    }
    let second = pixels(&second);
    let frame = [window(&second, 2, Some((1, std::slice::from_ref(&block))))];
    assert_eq!(
        gpu.compose(width, height, 0, &frame).unwrap(),
        compose_cpu(width, height, 0, &frame)
    );
    assert_eq!(
        gpu.last_uploaded_bytes(),
        3 * 2 * 4,
        "only the block's rows"
    );
    // Version 3 changes everything but says only the block did: the rest stays version 2's.
    let third = pixels(&words(3));
    let frame = [window(&third, 3, Some((2, std::slice::from_ref(&block))))];
    let composed = gpu.compose(width, height, 0, &frame).unwrap();
    assert_eq!(
        composed[0], 0x0000_0001,
        "a pixel left out of the change is the old one"
    );
    assert_eq!(composed[(5 * width + 4) as usize], 0x0000_0003);
    // From a version the device does not hold, everything goes up.
    let frame = [window(&third, 4, Some((9, std::slice::from_ref(&block))))];
    assert_eq!(
        gpu.compose(width, height, 0, &frame).unwrap(),
        compose_cpu(width, height, 0, &frame)
    );
    assert_eq!(gpu.last_uploaded_bytes(), u64::from(width * height) * 4);
}

#[test]
fn frames_round_trip_through_a_file() {
    let (window, popup) = frame();
    let layers = layers(&window, &popup);
    let mut bytes = Vec::new();
    frame_file::write(&mut bytes, 120, 90, 0x0011_2233, &layers).unwrap();
    let frame = frame_file::read(&mut bytes.as_slice()).unwrap();
    assert_eq!(
        (frame.width, frame.height, frame.background),
        (120, 90, 0x0011_2233)
    );
    assert_eq!(
        compose_cpu(120, 90, 0x0011_2233, &frame.layers()),
        compose_cpu(120, 90, 0x0011_2233, &layers)
    );
    assert!(frame_file::read(&mut &b"NOTAFRAME"[..]).is_err());
}
