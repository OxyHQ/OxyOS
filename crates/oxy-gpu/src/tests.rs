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
        },
        Layer {
            x: 70,
            y: 40,
            width: 20,
            height: 30,
            extent: (20, 30),
            paint: Paint::Premultiplied(popup),
        },
        Layer {
            x: 100,
            y: 0,
            width: 30,
            height: 200,
            extent: (1, 1),
            paint: Paint::Fill(0x0012_3456),
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
        },
        Layer {
            x: 0,
            y: 0,
            width: 0,
            height: 4,
            extent: (1, 1),
            paint: Paint::Fill(1),
        },
    ];
    assert_eq!(compose_cpu(4, 4, 7, &layers), vec![7; 16]);
}

/// The Vulkan composition matches the reference byte for byte, on whatever device is present.
/// Without a Vulkan device the test says so and passes: CI has none; `oxy-offscreen --gpu`
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
