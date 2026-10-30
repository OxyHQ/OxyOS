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
            opacity: 255,
            radius: 0,
            filter: Filter::Nearest,
            clip: None,
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
            opacity: 255,
            radius: 0,
            filter: Filter::Nearest,
            clip: None,
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
            opacity: 255,
            radius: 0,
            filter: Filter::Nearest,
            clip: None,
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
            opacity: 255,
            radius: 0,
            filter: Filter::Nearest,
            clip: None,
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
            opacity: 255,
            radius: 0,
            filter: Filter::Nearest,
            clip: None,
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

/// A frame composed deferred returns once submitted, with its completion as a sync file where
/// the device exports one (as a display's frames are, `compose_to_scanout`): the file becomes
/// readable once the device is done, and the frame then reads back as the reference. A
/// composition after it waits for it first, deferred or not.
#[test]
fn a_deferred_frame_completes_as_its_sync_file_says() {
    let mut gpu = match VulkanCompositor::new() {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            return;
        }
    };
    eprintln!(
        "{} exports sync files: {}",
        gpu.device_name(),
        gpu.exports_sync_file()
    );
    let (window, popup) = frame();
    let layers = layers(&window, &popup);
    let cpu = compose_cpu(320, 200, 0x0011_2233, &layers);
    for _ in 0..3 {
        let completion = gpu
            .compose_deferred(320, 200, 0x0011_2233, &layers)
            .unwrap();
        assert_eq!(completion.is_some(), gpu.exports_sync_file());
        if let Some(completion) = &completion {
            let mut fds = [rustix::event::PollFd::new(
                completion,
                rustix::event::PollFlags::IN,
            )];
            let timeout = rustix::event::Timespec {
                tv_sec: 10,
                tv_nsec: 0,
            };
            assert_eq!(rustix::event::poll(&mut fds, Some(&timeout)).unwrap(), 1);
        }
        assert_eq!(gpu.read_deferred().unwrap(), cpu);
    }
    // Deferred, then another composition at once: the first is done before the second begins.
    gpu.compose_deferred(320, 200, 0x0011_2233, &layers)
        .unwrap();
    assert_eq!(gpu.compose(16, 16, 5, &[]).unwrap(), vec![5; 256]);
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

/// The CPU composition from damage: a frame composed whole, then each next one composing only
/// what changed onto it, is the reference's every time, with corners, a shadow, filtering and
/// clips; any damage over an unchanged frame changes nothing; and damage that leaves out where
/// the popup was leaves it there, so the limit is real.
#[test]
fn the_cpu_composition_from_damage_is_the_reference() {
    let (window, popup) = frame();
    let profile = ShadowProfile::new(24, 90);
    let mut first = shadowed(&window, &popup, &profile, 16, 255, 256);
    // A window drawn smaller (filtered) and clipped, as the overview shows one.
    first.push(Layer {
        x: 70,
        y: 50,
        width: 40,
        height: 30,
        filter: Filter::Box,
        clip: Some(Rect {
            x: 75,
            y: 55,
            width: 30,
            height: 20,
        }),
        radius: 6,
        ..first[1]
    });
    let mut out = compose_cpu(120, 90, 0x0011_2233, &first);
    let mut shown = first.clone();
    for (dx, dy) in [(15, -20), (-30, 25), (1, 1), (40, 0)] {
        // The popup is the second layer after the shadow.
        let mut moved = shown.clone();
        let before = Rect {
            x: moved[2].x,
            y: moved[2].y,
            width: moved[2].width,
            height: moved[2].height,
        };
        moved[2].x += dx;
        moved[2].y += dy;
        let after = Rect {
            x: moved[2].x,
            y: moved[2].y,
            ..before
        };
        compose_cpu_damaged(&mut out, 120, 90, 0x0011_2233, &moved, &[before, after]);
        assert_eq!(
            out,
            compose_cpu(120, 90, 0x0011_2233, &moved),
            "moved by ({dx}, {dy})"
        );
        shown = moved;
    }
    let whole = out.clone();
    for damage in [
        Rect {
            x: -10,
            y: -10,
            width: 30,
            height: 200,
        },
        Rect {
            x: 59,
            y: 33,
            width: 1,
            height: 1,
        },
        Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 90,
        },
    ] {
        compose_cpu_damaged(&mut out, 120, 90, 0x0011_2233, &shown, &[damage]);
        assert_eq!(out, whole, "{damage:?}");
    }
    let mut moved = shown.clone();
    moved[2].x -= 25;
    let after = Rect {
        x: moved[2].x,
        y: moved[2].y,
        width: moved[2].width,
        height: moved[2].height,
    };
    compose_cpu_damaged(&mut out, 120, 90, 0x0011_2233, &moved, &[after]);
    assert_ne!(out, compose_cpu(120, 90, 0x0011_2233, &moved));
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

/// Two displays of different sizes, each with its own pair of targets (docs/adr/0018): their
/// frames interleave, each composing only its own damage, and every frame shown is the
/// reference's. A display removed leaves the other's targets where they were, and its indices
/// go to the next display's.
#[cfg(target_os = "linux")]
#[test]
fn each_displays_targets_keep_their_own_damage() {
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
    let first: Vec<usize> = gpu
        .add_scanout_targets(2, 120, 90)
        .unwrap()
        .into_iter()
        .map(|(index, _)| index)
        .collect();
    let second: Vec<usize> = gpu
        .add_scanout_targets(2, 96, 64)
        .unwrap()
        .into_iter()
        .map(|(index, _)| index)
        .collect();
    assert_eq!((first.clone(), second.clone()), (vec![0, 1], vec![2, 3]));
    let size = |display: usize| if display == 0 { (120, 90) } else { (96, 64) };
    let mut frames = [layers(&window, &popup), layers(&window, &popup)];
    let mut shown = [0usize; 2];
    for (step, (display, moved)) in [
        (0, None),
        (1, None),
        (0, Some((12, 9))),
        (1, Some((-25, 14))),
        (1, Some((8, -3))),
        (0, Some((-6, 20))),
        (1, Some((3, 3))),
        (0, Some((5, -11))),
    ]
    .into_iter()
    .enumerate()
    {
        let damage = moved.map(|(dx, dy)| {
            let (layers, changed) = popup_moved(&frames[display], dx, dy);
            frames[display] = layers;
            changed
        });
        let targets = if display == 0 { &first } else { &second };
        let index = targets[shown[display] % 2];
        shown[display] += 1;
        gpu.compose_to_scanout_kept(index, 0x0011_2233, &frames[display], damage.as_deref())
            .unwrap();
        let (width, height) = size(display);
        assert_eq!(
            gpu.read_kept().unwrap(),
            compose_cpu(width, height, 0x0011_2233, &frames[display]),
            "step {step}, display {display}"
        );
    }
    gpu.remove_scanout_targets(&first);
    assert!(gpu
        .compose_to_scanout(first[0], 0x0011_2233, &frames[0], None)
        .is_err());
    let (dx, dy) = (4, 4);
    let (moved, changed) = popup_moved(&frames[1], dx, dy);
    gpu.compose_to_scanout_kept(second[0], 0x0011_2233, &moved, Some(&changed))
        .unwrap();
    assert_eq!(
        gpu.read_kept().unwrap(),
        compose_cpu(96, 64, 0x0011_2233, &moved)
    );
    let third: Vec<usize> = gpu
        .add_scanout_targets(1, 64, 48)
        .unwrap()
        .into_iter()
        .map(|(index, _)| index)
        .collect();
    assert_eq!(third, [0]);
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
            opacity: 255,
            radius: 0,
            filter: Filter::Nearest,
            clip: None,
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

/// A layer of one pixel at (0, 0) with `paint` at `opacity`.
fn dot(paint: Paint<'_>, opacity: u8) -> Layer<'_> {
    Layer {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        extent: (1, 1),
        paint,
        source: None,
        changed: None,
        opacity,
        radius: 0,
        filter: Filter::Nearest,
        clip: None,
    }
}

#[test]
fn a_layer_at_an_opacity_is_scaled_then_blended() {
    const UNDER: u32 = 0x0010_2030;
    let opaque = pixels(&[0xff_ff_80_00]);
    let compose = |layer: Layer<'_>| compose_cpu(1, 1, UNDER, &[layer])[0];
    // Half: the pixel scaled to 128/255, over the background kept at 127/255:
    // 255·128/255 + 16·127/255 = 128 + 7, 128·128/255 + 32·127/255 = 64 + 15, 0 + 48·127/255.
    assert_eq!(compose(dot(Paint::Opaque(&opaque), 128)), 0x0087_4f17);
    assert_eq!(compose(dot(Paint::Fill(0x00ff_8000), 128)), 0x0087_4f17);
    // A translucent premultiplied pixel: its alpha too is scaled (0x80·128/255 = 64, so 191 of
    // the background is kept): 0x40·128/255 + 16·191/255 = 32 + 11, 0x20·128/255 + 32·191/255
    // = 16 + 23, 0x10·128/255 + 48·191/255 = 8 + 35.
    let translucent = pixels(&[0x80_40_20_10]);
    assert_eq!(
        compose(dot(Paint::Premultiplied(&translucent), 128)),
        0x002b_272b
    );
    // Nothing at 0; at 255 as without an opacity.
    assert_eq!(compose(dot(Paint::Opaque(&opaque), 0)), UNDER);
    assert_eq!(compose(dot(Paint::Opaque(&opaque), 255)), 0x00ff_8000);
    assert_eq!(
        compose(dot(Paint::Premultiplied(&translucent), 255)),
        0x0040_2010 + 0x0007_0f17
    );
}

#[test]
fn vulkan_matches_the_reference_at_every_opacity() {
    let mut gpu = match VulkanCompositor::new() {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            return;
        }
    };
    let (window, popup) = frame();
    for opacity in [0u8, 1, 64, 127, 128, 200, 254, 255] {
        let layers: Vec<Layer<'_>> = layers(&window, &popup)
            .into_iter()
            .map(|layer| Layer { opacity, ..layer })
            .collect();
        let cpu = compose_cpu(120, 90, 0x0011_2233, &layers);
        let vulkan = gpu.compose(120, 90, 0x0011_2233, &layers).unwrap();
        assert_eq!(vulkan, cpu, "opacity {opacity}");
    }
}

#[test]
fn a_frame_file_keeps_each_layers_opacity_and_reads_the_first_format() {
    let (window, popup) = frame();
    let layers: Vec<Layer<'_>> = layers(&window, &popup)
        .into_iter()
        .enumerate()
        .map(|(index, layer)| Layer {
            opacity: 90 + index as u8 * 40,
            ..layer
        })
        .collect();
    let mut bytes = Vec::new();
    frame_file::write(&mut bytes, 120, 90, 0x0011_2233, &layers).unwrap();
    let frame = frame_file::read(&mut bytes.as_slice()).unwrap();
    assert_eq!(
        frame.layers().iter().map(|l| l.opacity).collect::<Vec<_>>(),
        layers.iter().map(|l| l.opacity).collect::<Vec<_>>()
    );
    // A file from before opacities: the same without the byte, every layer whole.
    let mut first = b"OXYFRAME1".to_vec();
    for value in [1u32, 1, 0, 1] {
        first.extend(value.to_le_bytes());
    }
    first.extend(0i32.to_le_bytes());
    first.extend(0i32.to_le_bytes());
    for value in [1u32, 1, 0, 0] {
        first.extend(value.to_le_bytes());
    }
    first.push(2);
    first.extend(0x00ab_cdefu32.to_le_bytes());
    first.extend(0u64.to_le_bytes());
    let frame = frame_file::read(&mut first.as_slice()).unwrap();
    assert_eq!(frame.layers()[0].opacity, 255);
    assert_eq!(compose_cpu(1, 1, 0, &frame.layers()), vec![0x00ab_cdef]);
}

/// FNV-1a over a composed frame's pixels.
fn digest(out: &[u32]) -> u64 {
    out.iter()
        .flat_map(|word| word.to_le_bytes())
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}

/// Square layers are composed as before corners: the frames of `layers` at three opacities and
/// two sizes, as the reference composed them at 8dc1af5 (before corners and shadows).
#[test]
fn square_layers_are_composed_as_before_corners() {
    let (window, popup) = frame();
    let before = [
        (255, (120, 90), 0x046c_1995_ba91_3957),
        (255, (1280, 720), 0xb8ad_d202_7dad_bce7),
        (128, (120, 90), 0xcd9c_1816_f23d_2ca4),
        (128, (1280, 720), 0x7e96_56d3_d140_6de4),
        (64, (120, 90), 0x0139_769c_82c5_e30f),
        (64, (1280, 720), 0x6113_2a51_1dc7_7847),
    ];
    for (opacity, (width, height), composed) in before {
        let layers: Vec<Layer<'_>> = layers(&window, &popup)
            .into_iter()
            .map(|layer| Layer { opacity, ..layer })
            .collect();
        assert_eq!(
            digest(&compose_cpu(width, height, 0x0011_2233, &layers)),
            composed,
            "opacity {opacity} at {width}x{height}"
        );
    }
}

#[test]
fn the_integer_square_root_is_exact() {
    for n in (0..70_000u32).chain([u32::MAX, 1 << 31, (1 << 31) - 1, 65_535 * 65_535]) {
        let root = shape::isqrt(n);
        assert!(
            u64::from(root) * u64::from(root) <= u64::from(n)
                && (u64::from(root) + 1) * (u64::from(root) + 1) > u64::from(n),
            "isqrt({n}) = {root}"
        );
    }
}

/// White, 8 × 8 with corners of radius 4, over black: each output pixel is its coverage.
fn rounded_square(radius: u32, opacity: u8) -> Vec<u32> {
    compose_cpu(
        8,
        8,
        0,
        &[Layer {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
            extent: (1, 1),
            paint: Paint::Fill(0x00ff_ffff),
            source: None,
            changed: None,
            opacity,
            radius,
            filter: Filter::Nearest,
            clip: None,
        }],
    )
}

#[test]
fn a_corner_covers_its_pixels_by_their_distance_from_its_circle() {
    let out = rounded_square(4, 255);
    let blue = |x: usize, y: usize| out[y * 8 + x] & 0xff;
    // The top-left corner, its circle's centre at (4, 4). Pixel (1, 0)'s centre (1.5, 0.5) is
    // √(2.5² + 3.5²) = 4.30 from it: 64 × that is isqrt(74 · 1024) = 275, 19/64 beyond the
    // edge, so (32 − 19)·255/64 = 51. (0, 0) is 60/64 beyond: nothing. (1, 1): 226, 30/64 inside,
    // (32 + 30)·255/64 = 247. (2, 0): 243, 13/64 inside, 45·255/64 = 179. (3, 0) is (1, 1)'s
    // distance. (2, 1) is 70/64 inside: all of it.
    let corner = [
        [0, 51, 179, 247],
        [51, 247, 255, 255],
        [179, 255, 255, 255],
        [247, 255, 255, 255],
    ];
    for (y, row) in corner.iter().enumerate() {
        for (x, &coverage) in row.iter().enumerate() {
            assert_eq!(blue(x, y), coverage, "({x}, {y})");
            // The other corners are its mirror images.
            assert_eq!(blue(7 - x, y), coverage, "({}, {y})", 7 - x);
            assert_eq!(blue(x, 7 - y), coverage, "({x}, {})", 7 - y);
            assert_eq!(blue(7 - x, 7 - y), coverage, "({}, {})", 7 - x, 7 - y);
        }
    }
    // Grey, as a coverage is the same in every channel.
    assert_eq!(out[1], 0x0033_3333);
    // With an opacity, the two multiply: 128 · 51/255 = 25.
    assert_eq!(rounded_square(4, 128)[1] & 0xff, 25);
    assert_eq!(rounded_square(4, 128)[3 * 8 + 3] & 0xff, 128);
    // A radius larger than half the side is half the side; 0 is square.
    assert_eq!(rounded_square(40, 255), out);
    assert_eq!(rounded_square(0, 255), vec![0x00ff_ffff; 64]);
}

#[test]
fn a_shadows_profile_is_the_shape_blurred() {
    // Blur 8 (σ 4): it reaches 12 pixels, 8 entries a pixel each way.
    let profile = ShadowProfile::new(8, 255);
    assert_eq!((profile.entries(), profile.reach()), (192, 12));
    let alpha = |entry: usize| profile.pixels()[entry * 4 + 3];
    assert!(profile.pixels().chunks(4).all(|p| p[..3] == [0, 0, 0]));
    // Whole inside, nothing outside, half at the edge (the entries either side of it are 1/16 px
    // in and out, 255 · Φ(±0.0156) = 129.1 and 125.9), falling throughout.
    assert_eq!((alpha(0), alpha(191)), (255, 0));
    assert_eq!((alpha(95), alpha(96)), (129, 126));
    assert!((1..192).all(|entry| alpha(entry) <= alpha(entry - 1)));
    // 4.5625 px out (the entry's middle), 1.14σ: 255 · Φ(−1.1406) = 32.4.
    assert_eq!(alpha(96 + 36), 32);
    // At an alpha: scaled.
    let light = ShadowProfile::new(8, 51);
    assert_eq!(light.pixels()[96 * 4 + 3], 25);
}

/// A shadow of `profile` cast by a 20 × 20 square at (10, 10) of a 40 × 40 output (its layer
/// the whole output), of `radius`, beneath the square moved by `beneath`, over grey.
fn shadow_frame(profile: &ShadowProfile, radius: u32, beneath: (i32, i32), scale: u32) -> Vec<u32> {
    let square = Rect {
        x: 10,
        y: 10,
        width: 20,
        height: 20,
    };
    compose_cpu(
        40,
        40,
        0x0080_8080,
        &[Layer {
            x: 0,
            y: 0,
            width: 40,
            height: 40,
            extent: (profile.entries(), 1),
            paint: Paint::Shadow(
                profile.pixels(),
                Shadow {
                    shape: square,
                    radius,
                    clip: Rect {
                        x: square.x + beneath.0,
                        y: square.y + beneath.1,
                        ..square
                    },
                    clip_radius: radius,
                    scale,
                },
            ),
            source: None,
            changed: None,
            opacity: 255,
            radius: 0,
            filter: Filter::Nearest,
            clip: None,
        }],
    )
}

#[test]
fn a_shadow_darkens_by_its_profile_outside_what_it_is_beneath() {
    let profile = ShadowProfile::new(8, 255);
    let alpha = |entry: usize| u32::from(profile.pixels()[entry * 4 + 3]);
    let grey = |a: u32| {
        let channel = 0x80 * (255 - a) / 255;
        (channel << 16) | (channel << 8) | channel
    };
    let out = shadow_frame(&profile, 0, (0, -6), 256);
    // Pixel (5, 20) is 4.5 px left of the square's side: entry 96 + 36.
    assert_eq!(out[20 * 40 + 5], grey(alpha(96 + 36)));
    // Within what it is beneath (the square 6 px higher), nothing; below it, the shadow under it,
    // all but whole 2 px inside the shape's bottom (entry 96 − 20).
    assert_eq!(out[15 * 40 + 20], 0x0080_8080);
    assert_eq!(out[27 * 40 + 20], grey(alpha(96 - 20)));
    // Darker below than above, where the shape is further.
    assert!(out[31 * 40 + 20] < out[3 * 40 + 20]);
    // Far beyond its reach: nothing.
    assert_eq!(out[0], 0x0080_8080);
    // Past a corner it is taken from the distance to the corner's circle: (4, 4) is √2·6.5 px
    // from the square's corner, and with a radius of 4 its distance is to the circle centred at
    // (14, 14): √2·9.5 − 4 = 9.43 px, entry 96 + 75 (isqrt((19² + 19²)·16) = 107, less 32).
    assert_eq!(
        shadow_frame(&profile, 4, (0, -6), 256)[4 * 40 + 4],
        grey(alpha(96 + 75))
    );
    // Drawn at half its size, the same distance is twice as far along the profile.
    assert_eq!(
        shadow_frame(&profile, 0, (0, -6), 128)[20 * 40 + 5],
        grey(alpha(96 + 72))
    );
}

/// The frame of `layers` with its window rounded to `radius`, and a shadow beneath it of
/// `profile` at `scale`, offset 6 px down, every layer at `opacity`.
fn shadowed<'a>(
    window: &'a [u8],
    popup: &'a [u8],
    profile: &'a ShadowProfile,
    radius: u32,
    opacity: u8,
    scale: u32,
) -> Vec<Layer<'a>> {
    let mut layers = layers(window, popup);
    let caster = layers[0];
    let reach = (profile.reach() * scale).div_ceil(256) as i32;
    let shadow = Layer {
        x: caster.x - reach,
        y: caster.y + 6 - reach,
        width: caster.width + 2 * reach as u32,
        height: caster.height + 2 * reach as u32,
        extent: (profile.entries(), 1),
        paint: Paint::Shadow(
            profile.pixels(),
            Shadow {
                shape: Rect {
                    x: reach,
                    y: reach,
                    width: caster.width,
                    height: caster.height,
                },
                radius,
                clip: Rect {
                    x: reach,
                    y: reach - 6,
                    width: caster.width,
                    height: caster.height,
                },
                clip_radius: radius,
                scale,
            },
        ),
        source: Some(Source {
            key: 99,
            version: 1,
        }),
        changed: None,
        opacity,
        radius: 0,
        filter: Filter::Nearest,
        clip: None,
    };
    layers[0].radius = radius;
    layers[1].radius = radius / 2;
    layers.insert(0, shadow);
    layers
        .into_iter()
        .map(|layer| Layer { opacity, ..layer })
        .collect()
}

#[test]
fn vulkan_matches_the_reference_with_corners_and_shadows() {
    let mut gpu = match VulkanCompositor::new() {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            return;
        }
    };
    let (window, popup) = frame();
    let active = ShadowProfile::new(24, 90);
    let hard = ShadowProfile::new(0, 255);
    for profile in [&active, &hard] {
        for radius in [0, 1, 4, 16, 33, 1000] {
            for opacity in [0, 77, 128, 255] {
                for scale in [256, 246, 26, 1024] {
                    let layers = shadowed(&window, &popup, profile, radius, opacity, scale);
                    let cpu = compose_cpu(120, 90, 0x00e0_e0e0, &layers);
                    if opacity > 0 {
                        assert_ne!(
                            cpu,
                            compose_cpu(120, 90, 0x00e0_e0e0, &layers[1..]),
                            "the shadow is drawn"
                        );
                    }
                    let vulkan = gpu.compose(120, 90, 0x00e0_e0e0, &layers).unwrap();
                    assert_eq!(
                        vulkan, cpu,
                        "radius {radius}, opacity {opacity}, scale {scale}"
                    );
                }
            }
        }
    }
    // Composing from damage, as the compositor does: the frame is still the reference's.
    let layers = shadowed(&window, &popup, &active, 16, 255, 256);
    let damage = [Rect {
        x: 0,
        y: 60,
        width: 50,
        height: 30,
    }];
    gpu.compose(120, 90, 0x00e0_e0e0, &layers).unwrap();
    assert_eq!(
        gpu.compose_damaged(120, 90, 0x00e0_e0e0, &layers, &damage)
            .unwrap(),
        compose_cpu(120, 90, 0x00e0_e0e0, &layers)
    );
}

#[test]
fn a_frame_file_keeps_corners_and_shadows_and_reads_the_second_format() {
    let (window, popup) = frame();
    let profile = ShadowProfile::new(24, 90);
    let layers = shadowed(&window, &popup, &profile, 16, 200, 246);
    let mut bytes = Vec::new();
    frame_file::write(&mut bytes, 120, 90, 0x00e0_e0e0, &layers).unwrap();
    assert_eq!(&bytes[..9], b"OXYFRAME4");
    let frame = frame_file::read(&mut bytes.as_slice()).unwrap();
    assert_eq!(
        frame.layers().iter().map(|l| l.radius).collect::<Vec<_>>(),
        [0, 16, 8, 0]
    );
    assert!(matches!(frame.layers()[0].paint, Paint::Shadow(_, shadow) if shadow.scale == 246));
    assert_eq!(
        compose_cpu(120, 90, 0x00e0_e0e0, &frame.layers()),
        compose_cpu(120, 90, 0x00e0_e0e0, &layers)
    );
    // A file from before corners: square layers.
    let mut second = b"OXYFRAME2".to_vec();
    for value in [2u32, 2, 0, 1] {
        second.extend(value.to_le_bytes());
    }
    second.extend(0i32.to_le_bytes());
    second.extend(0i32.to_le_bytes());
    for value in [2u32, 2, 0, 0] {
        second.extend(value.to_le_bytes());
    }
    second.extend([2, 255]);
    second.extend(0x00ab_cdefu32.to_le_bytes());
    second.extend(0u64.to_le_bytes());
    let frame = frame_file::read(&mut second.as_slice()).unwrap();
    assert_eq!(frame.layers()[0].radius, 0);
    assert_eq!(compose_cpu(2, 2, 0, &frame.layers()), vec![0x00ab_cdef; 4]);
    // Shadows came with the third format.
    let index = second.len() - 8 - 4 - 2;
    second[index] = 3;
    assert!(frame_file::read(&mut second.as_slice()).is_err());
}

/// A window's pixels drawn smaller, as the overview draws it (docs/adr/0022-window-views.mdx).
fn reduced(paint: Paint<'_>, extent: (u32, u32), size: (u32, u32), filter: Filter) -> Layer<'_> {
    Layer {
        x: 0,
        y: 0,
        width: size.0,
        height: size.1,
        extent,
        paint,
        source: None,
        changed: None,
        opacity: 255,
        radius: 0,
        filter,
        clip: None,
    }
}

/// Drawn at a quarter of its size, a one-pixel checkerboard is grey with the box filter: each
/// output pixel is the mean of the 4 × 4 pixels it covers, worked by hand. Nearest neighbour
/// picks one of them, black or white: the pixelation the filter is for.
#[test]
fn a_box_filter_averages_the_pixels_each_output_pixel_covers() {
    let board = pixels(
        &(0..16 * 16)
            .map(|i| {
                if (i % 16 + i / 16) % 2 == 0 {
                    0xff_ff_ff_ff
                } else {
                    0xff_00_00_00
                }
            })
            .collect::<Vec<u32>>(),
    );
    let boxed = compose_cpu(
        4,
        4,
        0,
        &[reduced(
            Paint::Opaque(&board),
            (16, 16),
            (4, 4),
            Filter::Box,
        )],
    );
    // 8 white and 8 black: (8 × 255 + 8) / 16 = 128.
    assert_eq!(boxed, vec![0x0080_8080; 16]);
    let nearest = compose_cpu(
        4,
        4,
        0,
        &[reduced(
            Paint::Opaque(&board),
            (16, 16),
            (4, 4),
            Filter::Nearest,
        )],
    );
    assert!(nearest.iter().all(|&p| p == 0x00ff_ffff || p == 0));

    // Premultiplied pixels are averaged with their alpha: opaque red and nothing make red at
    // half alpha, blended over the background as such.
    let half = pixels(&[0xff_ff_00_00, 0x0000_0000]);
    let out = compose_cpu(
        1,
        1,
        0x0000_00ff,
        &[reduced(
            Paint::Premultiplied(&half),
            (2, 1),
            (1, 1),
            Filter::Box,
        )],
    );
    // Alpha (255 + 1) / 2 = 128 and red 128; the blue background kept at 255 - 128 = 127/255.
    assert_eq!(out, vec![0x0080_007f]);

    // However many pixels an output pixel covers (up to `filter::MAX_RATIO`), every one counts:
    // 32 greys in a row, 0 to 248, into one, their mean, 124.
    let ramp = pixels(
        &(0..32u32)
            .map(|x| 0xff00_0000 | (x * 8))
            .collect::<Vec<u32>>(),
    );
    let out = compose_cpu(
        1,
        1,
        0,
        &[reduced(Paint::Opaque(&ramp), (32, 1), (1, 1), Filter::Box)],
    );
    assert_eq!(out, vec![(0..32).map(|x| x * 8).sum::<u32>() / 32]);

    // At its own size the filter reads the one pixel nearest neighbour does.
    let (window, _) = frame();
    let size = (64, 48);
    assert_eq!(
        compose_cpu(
            96,
            72,
            7,
            &[reduced(Paint::Opaque(&window), (64, 48), size, Filter::Box)]
        ),
        compose_cpu(
            96,
            72,
            7,
            &[reduced(
                Paint::Opaque(&window),
                (64, 48),
                size,
                Filter::Nearest
            )]
        )
    );
}

/// A clipped layer touches nothing outside its clip, and inside it is drawn as without one.
#[test]
fn a_clipped_layer_is_drawn_only_within_its_clip() {
    let (window, popup) = frame();
    let whole = compose_cpu(120, 90, 0x0011_2233, &layers(&window, &popup));
    let clip = Rect {
        x: 20,
        y: 10,
        width: 40,
        height: 30,
    };
    let clipped: Vec<Layer<'_>> = layers(&window, &popup)
        .into_iter()
        .map(|layer| Layer {
            clip: Some(clip),
            ..layer
        })
        .collect();
    let out = compose_cpu(120, 90, 0x0011_2233, &clipped);
    for y in 0..90 {
        for x in 0..120 {
            let inside = clip.intersect(&Rect {
                x,
                y,
                width: 1,
                height: 1,
            });
            let index = (y * 120 + x) as usize;
            let expected = if inside.is_some() {
                whole[index]
            } else {
                0x0011_2233
            };
            assert_eq!(out[index], expected, "at {x}, {y}");
        }
    }
}

/// Layers reduced with the box filter, opaque and premultiplied, at every scale the overview
/// draws windows at and some it never does, clipped and not, whole and from damage: the Vulkan
/// composition matches the reference byte for byte.
#[test]
fn vulkan_matches_the_reference_filtered_and_clipped() {
    let mut gpu = match VulkanCompositor::new() {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            return;
        }
    };
    let window: Vec<u32> = (0..320 * 200)
        .map(|i: u32| (i.wrapping_mul(2_654_435_761) >> 8) | 0xff00_0000)
        .collect();
    let window = pixels(&window);
    // Premultiplied: each channel at most its alpha.
    let translucent: Vec<u32> = (0..320 * 200)
        .map(|i: u32| {
            let alpha = i.wrapping_mul(40_503) >> 24;
            let channel = |shift: u32| ((i >> shift) & 0xff) * alpha / 255;
            (alpha << 24) | (channel(16) << 16) | (channel(8) << 8) | channel(0)
        })
        .collect();
    let translucent = pixels(&translucent);
    for (width, height) in [
        (160, 100),
        (75, 47),
        (37, 23),
        (13, 9),
        (320, 200),
        (401, 260),
    ] {
        for clip in [
            None,
            Some(Rect {
                x: 9,
                y: 5,
                width: 50,
                height: 31,
            }),
        ] {
            let layers = [
                Layer {
                    x: 3,
                    y: 2,
                    clip,
                    ..reduced(
                        Paint::Opaque(&window),
                        (320, 200),
                        (width, height),
                        Filter::Box,
                    )
                },
                Layer {
                    x: 11,
                    y: 7,
                    radius: 6,
                    opacity: 200,
                    clip,
                    ..reduced(
                        Paint::Premultiplied(&translucent),
                        (320, 200),
                        (width, height),
                        Filter::Box,
                    )
                },
            ];
            let cpu = compose_cpu(180, 120, 0x0011_2233, &layers);
            let vulkan = gpu.compose(180, 120, 0x0011_2233, &layers).unwrap();
            assert_eq!(vulkan, cpu, "{width}x{height} clipped {clip:?}");
            let damage = [Rect {
                x: 20,
                y: 15,
                width: 60,
                height: 40,
            }];
            let damaged = gpu
                .compose_damaged(180, 120, 0x0011_2233, &layers, &damage)
                .unwrap();
            assert_eq!(damaged, cpu, "{width}x{height} clipped {clip:?}, damaged");
        }
    }
}

#[test]
fn a_frame_file_keeps_filters_and_clips_and_reads_the_third_format() {
    let (window, popup) = frame();
    let mut layers = layers(&window, &popup);
    layers[0].filter = Filter::Box;
    layers[1].clip = Some(Rect {
        x: 72,
        y: 41,
        width: 9,
        height: 12,
    });
    let mut bytes = Vec::new();
    frame_file::write(&mut bytes, 120, 90, 0x0011_2233, &layers).unwrap();
    let frame = frame_file::read(&mut bytes.as_slice()).unwrap();
    let read = frame.layers();
    assert_eq!(
        read.iter().map(|l| (l.filter, l.clip)).collect::<Vec<_>>(),
        layers
            .iter()
            .map(|l| (l.filter, l.clip))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        compose_cpu(120, 90, 0x0011_2233, &read),
        compose_cpu(120, 90, 0x0011_2233, &layers)
    );
    // A file from before filters: the same without them, every layer nearest and unclipped.
    let mut third = b"OXYFRAME3".to_vec();
    for value in [2u32, 2, 0, 1] {
        third.extend(value.to_le_bytes());
    }
    third.extend(0i32.to_le_bytes());
    third.extend(0i32.to_le_bytes());
    for value in [2u32, 2, 0, 0] {
        third.extend(value.to_le_bytes());
    }
    third.extend([2, 255]);
    // Radius, the shadow's shape and clip with theirs, and its scale.
    third.extend([0u8; 12 * 4]);
    third.extend(0x00ab_cdefu32.to_le_bytes());
    third.extend(0u64.to_le_bytes());
    let frame = frame_file::read(&mut third.as_slice()).unwrap();
    assert_eq!(
        (frame.layers()[0].filter, frame.layers()[0].clip),
        (Filter::Nearest, None)
    );
    assert_eq!(compose_cpu(2, 2, 0, &frame.layers()), vec![0x00ab_cdef; 4]);
}

/// A layer of `extent` source pixels drawn at `size`, filtered, at the origin.
fn drawn(paint: Paint<'_>, extent: (u32, u32), size: (u32, u32)) -> Layer<'_> {
    reduced(paint, extent, size, Filter::Box)
}

/// Grey levels as opaque pixels, and the red channel of each output pixel composed from them.
fn greys(levels: &[u32]) -> Vec<u8> {
    pixels(
        &levels
            .iter()
            .map(|&l| 0xff00_0000 | (l << 16))
            .collect::<Vec<_>>(),
    )
}

fn reds(out: &[u32]) -> Vec<u32> {
    out.iter().map(|pixel| pixel >> 16).collect()
}

#[test]
fn filtered_sampling_is_bilinear_drawn_larger_and_an_average_drawn_smaller() {
    let compose = |levels: &[u32], extent, size: (u32, u32)| {
        let source = greys(levels);
        reds(&compose_cpu(
            size.0,
            size.1,
            0,
            &[drawn(Paint::Opaque(&source), extent, size)],
        ))
    };
    // Twice as wide: each output pixel's centre falls at (x + ½)/2 − ½ between the source
    // pixels' centres, the edges continued: at −¼ (all of the first), ¼, ¾ (a quarter and three
    // quarters of 255: 63.75 and 191.25), 1¼ (all of the last).
    assert_eq!(compose(&[0, 255], (2, 1), (4, 1)), [0, 64, 191, 255]);
    // Half as wide: each output pixel the average of the two it covers.
    assert_eq!(compose(&[10, 20, 30, 50], (4, 1), (2, 1)), [15, 40]);
    // A third: each pixel a third (1365, 1366 and 1365 of 4096): 255·1365/4096 = 84.98.
    assert_eq!(compose(&[0, 0, 255], (3, 1), (1, 1)), [85]);
    // Both ways, a one-pixel checkerboard at half its size: each output pixel two of its four
    // source pixels white, 127.5, rounded up.
    let checkers: Vec<u32> = (0..16).map(|i| 255 * ((i % 4 + i / 4) % 2)).collect();
    assert_eq!(compose(&checkers, (4, 4), (2, 2)), [128; 4]);
    // 1.5 times as wide: centres at −⅙, ½, 1⅙, 1⅚, 2½ and 3⅙ source pixels from the first's
    // centre: all of the first, halfway from 90 to 30, a sixth of the way from 30 to 0 (25), five
    // sixths (5), halfway from 0 to 60, all of the last.
    assert_eq!(
        compose(&[90, 30, 0, 60], (4, 1), (6, 1)),
        [90, 60, 25, 5, 30, 60]
    );
    // Two thirds as wide: each output pixel covers one and a half source pixels (2731 and 1365 of
    // 4096): (90 + 30/2)/1.5 = 70, (30/2 + 0)/1.5 = 10, (60 + 120/2)/1.5 = 80 and
    // (120/2 + 240)/1.5 = 200.
    assert_eq!(
        compose(&[90, 30, 0, 60, 120, 240], (6, 1), (4, 1)),
        [70, 10, 80, 200]
    );
    // Within a pixel of its size, an axis is its pixels, by nearest neighbour: a 602-pixel row at
    // 601 drops one of them, where filtering would blend half of them with their neighbours.
    let row: Vec<u32> = (0..602).map(|i| (i * 7) % 256).collect();
    let nearest: Vec<u32> = (0..601).map(|x| row[x * 602 / 601]).collect();
    assert_eq!(compose(&row, (602, 1), (601, 1)), nearest);
    let tall: Vec<u32> = (0..2 * 602).map(|i| row[i % 602]).collect();
    assert_eq!(
        compose(&tall, (602, 2), (601, 9))[601 * 4..601 * 5],
        nearest[..]
    );
    // Premultiplied: colour and alpha averaged alike. Transparent and opaque red, half as wide:
    // half-transparent red, 128 of each, over black and over white.
    let premultiplied = pixels(&[0x0000_0000, 0xffff_0000, 0x0000_0000, 0xffff_0000]);
    let layer = drawn(Paint::Premultiplied(&premultiplied), (4, 1), (2, 1));
    assert_eq!(compose_cpu(2, 1, 0, &[layer]), [0x0080_0000; 2]);
    assert_eq!(compose_cpu(2, 1, 0x00ff_ffff, &[layer]), [0x00ff_7f7f; 2]);
    // An opaque layer is opaque, whatever its fourth byte.
    let unset = pixels(&[0x0040_0000; 4]);
    let layer = drawn(Paint::Opaque(&unset), (4, 1), (2, 1));
    assert_eq!(compose_cpu(2, 1, 0x00ff_ffff, &[layer]), [0x0040_0000; 2]);
}

#[test]
fn filtered_sampling_keeps_a_flat_colour_and_draws_one_to_one_as_it_is() {
    let (window, _) = frame();
    for (size, extent) in [
        ((96, 72), (64, 48)),
        ((7, 5), (64, 48)),
        ((640, 3), (64, 48)),
    ] {
        let flat = pixels(&vec![0xff_9a_31_c4; (extent.0 * extent.1) as usize]);
        let out = compose_cpu(
            size.0,
            size.1,
            0,
            &[drawn(Paint::Opaque(&flat), extent, size)],
        );
        assert!(out.iter().all(|&p| p == 0x009a_31c4), "{size:?}");
    }
    // At 1:1, filtered is the pixels as they are: as nearest neighbour.
    let one = drawn(Paint::Opaque(&window), (64, 48), (64, 48));
    let nearest = Layer {
        filter: Filter::Nearest,
        ..one
    };
    assert_eq!(
        compose_cpu(80, 60, 7, &[one]),
        compose_cpu(80, 60, 7, &[nearest])
    );
    // An axis 1:1 and the other twice as long: the first row is the pixels' own first row
    // (the edge continued above it), the second a quarter of the way to their second.
    let tall = compose_cpu(
        64,
        96,
        0,
        &[drawn(Paint::Opaque(&window), (64, 48), (64, 96))],
    );
    let source: Vec<u32> = (0..48 * 64)
        .map(|i| u32::from_le_bytes(window[i * 4..i * 4 + 4].try_into().unwrap()) & 0x00ff_ffff)
        .collect();
    assert_eq!(tall[..64], source[..64]);
    let red = |pixel: u32| pixel >> 16;
    let quarter = (red(source[0]) * 3072 + red(source[64]) * 1024) * 4096;
    assert_eq!(red(tall[64]), (quarter + (1 << 23)) >> 24);
    // Beyond the largest ratio, nearest neighbour, as before filtering.
    let far = noise(130, 2);
    let wide = drawn(Paint::Opaque(&far), (130, 2), (2, 2));
    let nearest = Layer {
        filter: Filter::Nearest,
        ..wide
    };
    assert_eq!(
        compose_cpu(2, 2, 0, &[wide]),
        compose_cpu(2, 2, 0, &[nearest])
    );
    let within = drawn(Paint::Opaque(&far), (128, 2), (2, 2));
    assert_ne!(
        compose_cpu(2, 2, 0, &[within]),
        compose_cpu(
            2,
            2,
            0,
            &[Layer {
                filter: Filter::Nearest,
                ..within
            }]
        )
    );
}

/// The output pixels a source span is read by are exactly those whose taps reach it.
#[test]
fn the_span_a_layers_pixels_are_read_by_is_exact() {
    for extent in 1..24u32 {
        for length in 1..24u32 {
            if !filter::filters(extent, length) {
                continue;
            }
            for from in 0..extent {
                for to in from + 1..=extent {
                    let reads: Vec<u32> = (0..length)
                        .filter(|&at| {
                            let taps = filter::taps(at, length, extent);
                            taps.first < to && taps.last >= from
                        })
                        .collect();
                    let (start, end) = filter::span(from, to, extent, length);
                    assert_eq!(
                        (start..end).collect::<Vec<_>>(),
                        reads,
                        "{from}..{to} of {extent} at {length}"
                    );
                }
            }
        }
    }
    // Long axes a pixel apart, sampled by nearest neighbour.
    for (extent, length) in [(64, 63), (63, 64), (602, 601), (601, 602), (70, 70)] {
        for from in [0, 1, 30, extent - 2] {
            for to in [from + 1, from + 2, extent] {
                let reads: Vec<u32> = (0..length)
                    .filter(|&at| (from..to).contains(&(at * extent / length)))
                    .collect();
                let (start, end) = filter::span(from, to, extent, length);
                assert_eq!(
                    (start..end).collect::<Vec<_>>(),
                    reads,
                    "{from}..{to} of {extent} at {length}"
                );
            }
        }
    }
    assert_eq!(filter::span(3, 6, 10, 15), (4, 10));
    assert_eq!(filter::span(3, 6, 10, 10), (3, 6));
}

/// A window of `extent` with corners, drawn at `scale` about its centre, a premultiplied popup
/// drawn at the same scale over it, both at `opacity`, as a transition draws them.
fn scaled<'a>(
    window: &'a [u8],
    popup: &'a [u8],
    extent: (u32, u32),
    scale: f64,
    opacity: u8,
    radius: u32,
) -> Vec<Layer<'a>> {
    let size = |length: u32| ((f64::from(length) * scale).round() as u32).max(1);
    let (width, height) = (size(extent.0), size(extent.1));
    vec![
        Layer {
            x: 160 - width as i32 / 2,
            y: 120 - height as i32 / 2,
            radius,
            opacity,
            ..drawn(Paint::Opaque(window), extent, (width, height))
        },
        Layer {
            x: 150,
            y: 100,
            radius: radius / 2,
            opacity,
            ..drawn(Paint::Premultiplied(popup), (20, 30), (size(20), size(30)))
        },
    ]
}

/// A window's pixels: noise, so every tap counts.
fn noise(width: u32, height: u32) -> Vec<u8> {
    pixels(
        &(0..width * height)
            .map(|i: u32| 0xff00_0000 | (i.wrapping_mul(2_654_435_761) >> 8))
            .collect::<Vec<_>>(),
    )
}

#[test]
fn vulkan_matches_the_reference_filtered_at_every_scale() {
    let mut gpu = match VulkanCompositor::new() {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            return;
        }
    };
    let (_, popup) = frame();
    let window = noise(400, 300);
    for scale in [0.1, 0.5, 0.96, 1.0, 1.04, 1.5, 2.0] {
        for opacity in [0u8, 77, 128, 255] {
            for radius in [0, 16] {
                let layers = scaled(&window, &popup, (400, 300), scale, opacity, radius);
                let cpu = compose_cpu(320, 240, 0x00e0_e0e0, &layers);
                let vulkan = gpu.compose(320, 240, 0x00e0_e0e0, &layers).unwrap();
                assert_eq!(
                    vulkan, cpu,
                    "scale {scale}, opacity {opacity}, radius {radius}"
                );
            }
        }
    }
    // Each axis by its own factor (a zoom), past the output's edges; one within a pixel of its
    // size, by nearest neighbour, the other filtered; and drawn far smaller than the ratio
    // filtered, by nearest neighbour.
    for (x, y, width, height) in [
        (-37, 11, 611, 97),
        (5, -20, 33, 290),
        (3, 4, 399, 150),
        (-2, 1, 171, 301),
        (0, 0, 6, 4),
    ] {
        let layers = [Layer {
            x,
            y,
            radius: 8,
            ..drawn(Paint::Opaque(&window), (400, 300), (width, height))
        }];
        let cpu = compose_cpu(320, 240, 0x00e0_e0e0, &layers);
        let vulkan = gpu.compose(320, 240, 0x00e0_e0e0, &layers).unwrap();
        assert_eq!(vulkan, cpu, "{width}x{height} at {x},{y}");
    }
    // Composing from damage: the frame is still the reference's.
    let layers = scaled(&window, &popup, (400, 300), 0.73, 200, 16);
    gpu.compose(320, 240, 0x00e0_e0e0, &layers).unwrap();
    let damage = [Rect {
        x: 100,
        y: 90,
        width: 70,
        height: 50,
    }];
    assert_eq!(
        gpu.compose_damaged(320, 240, 0x00e0_e0e0, &layers, &damage)
            .unwrap(),
        compose_cpu(320, 240, 0x00e0_e0e0, &layers)
    );
}

/// Layers drawn 1:1 are composed as before filtering, filtered or not: the frames of `layers`
/// with its window at its own size, and of `shadowed` (corners, a shadow) likewise, at three
/// opacities, as the reference composed them at fba8630 (before filtering).
#[test]
fn one_to_one_layers_are_composed_as_before_filtering() {
    let (window, popup) = frame();
    let profile = ShadowProfile::new(24, 90);
    let before = [
        (
            255,
            0x3717_8a26_896b_2cdf,
            0xb2b6_7b14_0d74_ed3f,
            0x3e09_675d_3f62_ff62,
            0xa892_4a9b_1509_ca29,
        ),
        (
            128,
            0xe3ed_ce86_0b92_5479,
            0x8e05_2db2_47f7_8779,
            0x5dd7_130c_bd71_5d01,
            0xfa58_4dc9_6a15_2eb2,
        ),
        (
            64,
            0x5c05_2082_64d5_0e41,
            0x72f2_08bf_9de8_1829,
            0xc5f5_3b94_7b61_926c,
            0x9013_7832_36fc_89b2,
        ),
    ];
    for (opacity, small, large, square, rounded) in before {
        for sampling in [Filter::Nearest, Filter::Box] {
            let layers: Vec<Layer<'_>> = layers(&window, &popup)
                .into_iter()
                .map(|layer| Layer {
                    width: if layer.extent == (64, 48) {
                        64
                    } else {
                        layer.width
                    },
                    height: if layer.extent == (64, 48) {
                        48
                    } else {
                        layer.height
                    },
                    opacity,
                    filter: sampling,
                    ..layer
                })
                .collect();
            for ((width, height), composed) in [((120, 90), small), ((1280, 720), large)] {
                assert_eq!(
                    digest(&compose_cpu(width, height, 0x0011_2233, &layers)),
                    composed,
                    "opacity {opacity} at {width}x{height}, {sampling:?}"
                );
            }
            for (radius, composed) in [(0, square), (16, rounded)] {
                let mut layers = shadowed(&window, &popup, &profile, radius, opacity, 256);
                for layer in &mut layers {
                    layer.filter = sampling;
                }
                layers[1].width = 64;
                layers[1].height = 48;
                assert_eq!(
                    digest(&compose_cpu(120, 90, 0x00e0_e0e0, &layers)),
                    composed,
                    "opacity {opacity}, radius {radius}, {sampling:?}"
                );
            }
        }
    }
}

/// A pattern of `width` × `height` grey levels finer than a tenth of it can show: a zone plate
/// (rings ever closer toward the edges) and one-pixel lines.
fn fine_pattern(width: u32, height: u32) -> Vec<u32> {
    (0..width * height)
        .map(|i| {
            let (x, y) = (f64::from(i % width), f64::from(i / width));
            if y < f64::from(height) / 2.0 {
                let (dx, dy) = (x - f64::from(width) / 2.0, y - f64::from(height) / 2.0);
                ((1.0 + (std::f64::consts::PI * (dx * dx + dy * dy) / 400.0).cos()) * 127.5).round()
                    as u32
            } else {
                255 * (((i % width) % 2) ^ ((i / width) % 3 / 2))
            }
        })
        .collect()
}

/// The pattern drawn at `size`, as an ideal display pixel shows it: the exact average of the
/// source it covers (in 64-bit floating point).
fn area_average(levels: &[u32], extent: (u32, u32), size: (u32, u32)) -> Vec<f64> {
    // Per output pixel of an axis, the source pixels it covers and by how much.
    let covers = |length: u32, extent: u32| -> Vec<Vec<(usize, f64)>> {
        let ratio = f64::from(extent) / f64::from(length);
        (0..length)
            .map(|at| {
                let (start, end) = (f64::from(at) * ratio, f64::from(at + 1) * ratio);
                (start.floor() as u32..(end.ceil() as u32).min(extent))
                    .map(|source| {
                        let s = f64::from(source);
                        (source as usize, (end.min(s + 1.0) - start.max(s)) / ratio)
                    })
                    .collect()
            })
            .collect()
    };
    let (columns, rows) = (covers(size.0, extent.0), covers(size.1, extent.1));
    let mut out = Vec::with_capacity((size.0 * size.1) as usize);
    for row in &rows {
        for column in &columns {
            let mut sum = 0.0;
            for &(sy, wy) in row {
                for &(sx, wx) in column {
                    sum += wx * wy * f64::from(levels[sy * extent.0 as usize + sx]);
                }
            }
            out.push(sum);
        }
    }
    out
}

/// A window minimized shows what it holds, not what its sampling happens to hit: at a tenth, a
/// pattern finer than that drawn by nearest neighbour is far from the area it covers (the
/// rings alias into new rings, the lines into solid colour), filtered it is that within
/// rounding, as at the scales a minimize passes through.
#[test]
fn a_window_minimized_into_the_dock_is_not_aliased() {
    let extent = (640, 400);
    let levels = fine_pattern(extent.0, extent.1);
    let source = greys(&levels);
    for (width, height) in [(64, 40), (128, 80), (467, 292)] {
        let ideal = area_average(&levels, extent, (width, height));
        let error = |sampling| {
            let layer = Layer {
                filter: sampling,
                ..drawn(Paint::Opaque(&source), extent, (width, height))
            };
            let out = reds(&compose_cpu(width, height, 0, &[layer]));
            let squares: f64 = out
                .iter()
                .zip(&ideal)
                .map(|(&shown, &ideal)| (f64::from(shown) - ideal).powi(2))
                .sum();
            (squares / out.len() as f64).sqrt()
        };
        let (nearest, filtered) = (error(Filter::Nearest), error(Filter::Box));
        eprintln!(
            "{width}x{height}: RMS error from the area covered, of 255: \
             nearest {nearest:.2}, filtered {filtered:.2}"
        );
        assert!(filtered < 0.6, "filtered {filtered}");
        assert!(nearest > 40.0 * filtered.max(0.5), "nearest {nearest}");
    }
}

/// A layer's changed pixels change the output only within their span (`filter::span`), the
/// damage the compositor gives them: drawn larger, smaller and each axis its own way.
#[test]
fn changed_pixels_change_the_output_only_within_their_span() {
    let before = noise(40, 30);
    for (width, height) in [(60, 45), (40, 30), (17, 29), (4, 3), (97, 13)] {
        for (x, y, w, h) in [(0, 0, 1, 1), (13, 7, 5, 9), (39, 29, 1, 1), (20, 0, 20, 30)] {
            let mut after = before.clone();
            for row in y..y + h {
                for column in x..x + w {
                    let at = (row * 40 + column) as usize * 4;
                    after[at..at + 4].copy_from_slice(&0xff12_3456u32.to_le_bytes());
                }
            }
            let compose = |pixels: &[u8]| {
                compose_cpu(
                    120,
                    80,
                    0,
                    &[Layer {
                        x: 3,
                        y: 2,
                        radius: 6,
                        opacity: 200,
                        ..drawn(Paint::Opaque(pixels), (40, 30), (width, height))
                    }],
                )
            };
            let (old, new) = (compose(&before), compose(&after));
            let columns = filter::span(x, x + w, 40, width);
            let rows = filter::span(y, y + h, 30, height);
            for (index, (old, new)) in old.iter().zip(&new).enumerate() {
                let (ox, oy) = (index as u32 % 120, index as u32 / 120);
                if old != new {
                    assert!(
                        (columns.0 + 3..columns.1 + 3).contains(&ox)
                            && (rows.0 + 2..rows.1 + 2).contains(&oy),
                        "({ox}, {oy}) changed outside {columns:?} × {rows:?} at {width}x{height}"
                    );
                }
            }
        }
    }
}

#[test]
fn an_opaque_layer_at_its_own_size_is_its_pixels_as_drawn_from_any_other() {
    // A window with rounded corners, partly off the output and clipped, over another: at its
    // pixels' size each pixel is copied where it is covered whole; drawn from pixels twice the
    // size by nearest neighbour (the general way) it must be the same, corners and all.
    let (width, height) = (61u32, 43u32);
    let words: Vec<u32> = (0..width * height)
        .map(|i| 0xff00_0000 | (i.wrapping_mul(2_654_435_761) & 0x00ff_ffff))
        .collect();
    let doubled: Vec<u32> = (0..height * 2)
        .flat_map(|y| {
            let words = &words;
            (0..width * 2).map(move |x| words[((y / 2) * width + x / 2) as usize])
        })
        .collect();
    let (own, twice) = (pixels(&words), pixels(&doubled));
    let window = |paint, extent, x: i32, opacity: u8, clip: Option<Rect>| Layer {
        x,
        y: 5,
        width,
        height,
        extent,
        paint,
        source: None,
        changed: None,
        opacity,
        radius: 9,
        filter: Filter::Nearest,
        clip,
    };
    let under = pixels(&vec![0xff20_4060; 80 * 60]);
    let below = drawn(Paint::Opaque(&under), (80, 60), (80, 60));
    for (x, opacity, clip) in [
        (10, 255, None),
        (-7, 255, None),
        (
            30,
            255,
            Some(Rect {
                x: 35,
                y: 0,
                width: 20,
                height: 30,
            }),
        ),
        (10, 200, None),
    ] {
        let at_size = window(Paint::Opaque(&own), (width, height), x, opacity, clip);
        let scaled = window(
            Paint::Opaque(&twice),
            (width * 2, height * 2),
            x,
            opacity,
            clip,
        );
        assert_eq!(
            compose_cpu(80, 60, 3, &[below, at_size]),
            compose_cpu(80, 60, 3, &[below, scaled]),
            "at {x}, opacity {opacity}, clip {clip:?}"
        );
    }
}

/// Clients' GPU buffers (docs/adr/0031-gpu-buffers.mdx): a DMA-BUF imported with the linear
/// modifier and sampled in place composes exactly as the same pixels uploaded do, opaque or
/// premultiplied, at its size or filtered, rounded, translucent and clipped; a frame naming an
/// import the device dropped draws the pixels the CPU read instead.
#[test]
fn a_dma_buf_sampled_in_place_composes_as_its_pixels_do() {
    use std::os::fd::AsFd;
    let mut gpu = match VulkanCompositor::new() {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            return;
        }
    };
    let argb = 0x3432_5241;
    let xrgb = 0x3432_5258;
    if !gpu
        .dma_buf_formats()
        .iter()
        .any(|format| format.fourcc == argb && format.modifier == 0 && format.planes == 1)
    {
        eprintln!("{} imports no linear DMA-BUF, skipped", gpu.device_name());
        return;
    }
    let (window, popup) = frame();
    // Each buffer in a DMA-BUF of its own, its rows padded to a stride of 80 pixels beyond an
    // offset, as a GPU's allocation may be.
    let stride = 80 * 4;
    let made = |pixels: &[u8], width: u32, height: u32| {
        let offset = 256u32;
        let size = u64::from(offset) + u64::from(stride) * u64::from(height);
        let (memfd, buffer) = match oxyos_dmabuf::shared_dmabuf(size) {
            Ok(made) => made,
            Err(error) => {
                eprintln!("no udmabuf ({error}), skipped; tools/gpu-vm runs it");
                return None;
            }
        };
        for row in 0..height {
            let start = (row * width * 4) as usize;
            rustix::io::pwrite(
                &memfd,
                &pixels[start..start + width as usize * 4],
                u64::from(offset + row * stride),
            )
            .unwrap();
        }
        Some((buffer, offset))
    };
    let Some((window_buffer, window_offset)) = made(&window, 64, 48) else {
        return;
    };
    let (popup_buffer, popup_offset) = made(&popup, 20, 30).expect("udmabuf");
    let import = |gpu: &mut VulkanCompositor,
                  fourcc,
                  buffer: &oxyos_dmabuf::DmaBuf,
                  offset,
                  width,
                  height| {
        gpu.import_dma_buf(
            fourcc,
            0,
            width,
            height,
            &[DmaBufPlane {
                fd: buffer.as_fd(),
                offset,
                stride,
            }],
        )
        .unwrap()
    };
    let window_image = import(&mut gpu, xrgb, &window_buffer, window_offset, 64, 48);
    let popup_image = import(&mut gpu, argb, &popup_buffer, popup_offset, 20, 30);
    let reference = layers(&window, &popup);
    let imported: Vec<Layer<'_>> = reference
        .iter()
        .map(|layer| match layer.paint {
            Paint::Opaque(pixels) => Layer {
                paint: Paint::Imported {
                    image: window_image,
                    opaque: true,
                    pixels,
                },
                ..*layer
            },
            Paint::Premultiplied(pixels) => Layer {
                paint: Paint::Imported {
                    image: popup_image,
                    opaque: false,
                    pixels,
                },
                ..*layer
            },
            _ => *layer,
        })
        .collect();
    // Filtered smaller, rounded, translucent and clipped.
    let mut drawn = imported.clone();
    drawn[0].width = 40;
    drawn[0].height = 30;
    drawn[0].filter = Filter::Box;
    drawn[0].radius = 8;
    drawn[1].opacity = 180;
    drawn[1].clip = Some(Rect {
        x: 72,
        y: 44,
        width: 10,
        height: 20,
    });
    for layers in [&imported, &drawn] {
        for (width, height) in [(120, 90), (1280, 720)] {
            let cpu = compose_cpu(width, height, 0x0011_2233, layers);
            let vulkan = gpu.compose(width, height, 0x0011_2233, layers).unwrap();
            assert_eq!(vulkan, cpu, "{width}x{height}");
        }
    }
    // The GPU samples the buffer itself: with no pixels read, it composes what they would.
    let unread: Vec<Layer<'_>> = imported
        .iter()
        .map(|layer| match layer.paint {
            Paint::Imported { image, opaque, .. } => Layer {
                paint: Paint::Imported {
                    image,
                    opaque,
                    pixels: &[],
                },
                ..*layer
            },
            _ => *layer,
        })
        .collect();
    assert_eq!(
        gpu.compose(120, 90, 0x0011_2233, &unread).unwrap(),
        compose_cpu(120, 90, 0x0011_2233, &imported)
    );
    // A frame waited for read them and is done.
    assert!(!gpu.reads_dma_buf(window_image));
    gpu.forget_dma_buf(window_image);
    // Forgotten, the window is drawn from the pixels the CPU read.
    assert_eq!(
        gpu.compose(120, 90, 0x0011_2233, &imported).unwrap(),
        compose_cpu(120, 90, 0x0011_2233, &imported)
    );
    let refused = gpu.import_dma_buf(
        argb,
        0,
        64,
        4096,
        &[DmaBufPlane {
            fd: window_buffer.as_fd(),
            offset: 0,
            stride,
        }],
    );
    assert!(
        refused.is_err(),
        "a DMA-BUF smaller than its image is refused"
    );
    assert!(
        gpu.import_dma_buf(argb, 0x0100_0000_0000_0001, 64, 48, &[])
            .is_err(),
        "a modifier not in the table is refused"
    );
}
