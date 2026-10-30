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
            opacity: 255,
            radius: 0,
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
    assert_eq!(&bytes[..9], b"OXYFRAME3");
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
