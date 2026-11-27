//! The material on the Vulkan device (docs/adr/0069-the-platforms-material.mdx, step 3): every
//! frame with material areas composes byte for byte as `compose_cpu_with_material` composes it,
//! whole and from damage, into the output and into a display's buffers.

use super::*;
use crate::material::{box_width, reach, MaterialArea};
use crate::{
    compose_cpu, compose_cpu_damaged_with_material, compose_cpu_with_material, Filter, Layer,
    Paint, Rect, VulkanCompositor,
};

/// The device, or `None` (said) where there is none: CI has none; seldon's lavapipe runs these.
fn device() -> Option<VulkanCompositor> {
    match VulkanCompositor::new() {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("no Vulkan device, skipped: {error}");
            None
        }
    }
}

/// σ at scales 1, 1.25 and 2: 20 logical pixels (`material_area` in the compositor).
const SIGMAS: [u32; 3] = [20, 25, 40];

fn area(x: i32, y: i32, width: u32, height: u32) -> MaterialArea {
    MaterialArea {
        rect: Rect {
            x,
            y,
            width,
            height,
        },
        radius: 0,
        sigma: 20,
        opacity: 255,
    }
}

fn fill(x: i32, y: i32, width: u32, height: u32, colour: u32) -> Layer<'static> {
    Layer {
        x,
        y,
        width,
        height,
        extent: (1, 1),
        paint: Paint::Fill(colour),
        source: None,
        changed: None,
        opacity: 255,
        radius: 0,
        filter: Filter::Nearest,
        clip: None,
        warp: None,
    }
}

/// A menu's pixels, premultiplied: its tint, translucent, with clear pixels where it draws
/// nothing over its material.
fn menu_pixels(width: u32, height: u32) -> Vec<u8> {
    pixels(
        &(0..width * height)
            .map(|i| match i % 5 {
                0 | 1 => 0x0000_0000,
                2 => 0x6030_3030,
                3 => 0xc0b0_a090,
                _ => 0xffff_ffff,
            })
            .collect::<Vec<_>>(),
    )
}

/// `layers` (a scaled window, a premultiplied popup and a fill) with a menu over them at index 3,
/// whose material the tests draw.
fn with_menu<'a>(window: &'a [u8], popup: &'a [u8], menu: &'a [u8]) -> Vec<Layer<'a>> {
    let mut frame = layers(window, popup);
    frame.push(Layer {
        x: 20,
        y: 10,
        width: 60,
        height: 50,
        extent: (60, 50),
        paint: Paint::Premultiplied(menu),
        source: None,
        changed: None,
        opacity: 255,
        radius: 12,
        filter: Filter::Nearest,
        clip: None,
        warp: None,
    });
    frame
}

fn assert_agrees(
    gpu: &mut VulkanCompositor,
    (width, height): (u32, u32),
    layers: &[Layer<'_>],
    materials: &[(usize, MaterialArea)],
    what: &str,
) {
    let cpu = compose_cpu_with_material(width, height, 0x0011_2233, layers, materials);
    let vulkan = gpu
        .compose_with_material(width, height, 0x0011_2233, layers, materials)
        .unwrap();
    if let Some(at) = vulkan.iter().zip(&cpu).position(|(a, b)| a != b) {
        panic!(
            "{what} at {width}x{height}: pixel ({}, {}) is {:06x} on the device, {:06x} on the CPU",
            at as u32 % width,
            at as u32 / width,
            vulkan[at],
            cpu[at]
        );
    }
}

/// At each scale's σ, square and rounded, opaque and half faded, the device's material is the
/// reference's.
#[test]
fn vulkan_draws_the_material_as_the_reference_does() {
    let Some(mut gpu) = device() else {
        return;
    };
    let (window, popup) = frame();
    let menu = menu_pixels(60, 50);
    let layers = with_menu(&window, &popup, &menu);
    for sigma in SIGMAS {
        for radius in [0, 12] {
            for opacity in [255, 128] {
                let materials = [(
                    3,
                    MaterialArea {
                        radius,
                        sigma,
                        opacity,
                        ..area(20, 10, 60, 50)
                    },
                )];
                for size in [(120, 90), (320, 200)] {
                    let what = format!("σ {sigma}, radius {radius}, opacity {opacity}");
                    assert_agrees(&mut gpu, size, &layers, &materials, &what);
                }
            }
        }
    }
    // What the material changes is real: the frame is not the one without it.
    let materials = [(3, area(20, 10, 60, 50))];
    assert_ne!(
        gpu.compose_with_material(120, 90, 0x0011_2233, &layers, &materials)
            .unwrap(),
        compose_cpu(120, 90, 0x0011_2233, &layers)
    );
}

/// Areas under different layers, overlapping, under the bottom layer (nothing beneath but the
/// background), under a premultiplied layer, and ones that are never drawn (beyond the layers,
/// off the output, faded out): each blurs what is beneath it alone, as the reference does.
#[test]
fn several_areas_under_different_layers_are_the_reference() {
    let Some(mut gpu) = device() else {
        return;
    };
    let (window, popup) = frame();
    let menu = menu_pixels(60, 50);
    let mut layers = with_menu(&window, &popup, &menu);
    // A rounded, clipped, translucent fill above everything, with an area of its own.
    layers.push(Layer {
        opacity: 200,
        radius: 6,
        clip: Some(Rect {
            x: 50,
            y: 30,
            width: 60,
            height: 50,
        }),
        ..fill(40, 25, 70, 60, 0x0030_c050)
    });
    let materials = [
        (0, area(5, 5, 30, 20)),
        (1, area(60, 30, 40, 40)),
        (
            3,
            MaterialArea {
                radius: 12,
                sigma: 25,
                ..area(20, 10, 60, 50)
            },
        ),
        (
            3,
            MaterialArea {
                opacity: 128,
                sigma: 40,
                ..area(30, 40, 50, 30)
            },
        ),
        (
            4,
            MaterialArea {
                radius: 6,
                ..area(50, 30, 60, 50)
            },
        ),
        (5, area(0, 0, 120, 90)),
        (4, area(500, 500, 20, 20)),
        (
            2,
            MaterialArea {
                opacity: 0,
                ..area(10, 50, 40, 30)
            },
        ),
    ];
    for size in [(120, 90), (320, 200)] {
        assert_agrees(&mut gpu, size, &layers, &materials, "several areas");
    }
}

/// Areas at the output's edges and past them, on outputs whose sides are not whole cells: the
/// cells there take fewer pixels and the blur stops at the edge, as the reference's.
#[test]
fn areas_at_the_outputs_edge_are_the_reference() {
    let Some(mut gpu) = device() else {
        return;
    };
    let (window, popup) = frame();
    let menu = menu_pixels(60, 50);
    let layers = with_menu(&window, &popup, &menu);
    for sigma in SIGMAS {
        let edge = |x, y, width, height| MaterialArea {
            sigma,
            radius: 12,
            ..area(x, y, width, height)
        };
        let materials = [
            (3, edge(-10, -5, 50, 40)),
            (3, edge(90, 60, 50, 50)),
            (2, edge(0, 70, 130, 30)),
        ];
        for size in [(120, 90), (118, 87), (7, 3)] {
            let what = format!("σ {sigma} at the edges");
            assert_agrees(&mut gpu, size, &layers, &materials, &what);
        }
    }
}

/// The reference's own cases (`material::tests`) on the device: a grey stays that grey, a flat
/// colour is saturated, an edge is blurred within the area alone, small and large regions agree,
/// what is above is drawn over it, rounded corners leave what is beneath.
#[test]
fn the_references_cases_compose_alike_on_the_device() {
    let Some(mut gpu) = device() else {
        return;
    };
    let (width, height) = (96, 64);
    static CLEAR: [u8; 4] = [0; 4];
    let surface = Layer {
        x: 0,
        y: 0,
        width,
        height,
        extent: (1, 1),
        paint: Paint::Premultiplied(&CLEAR),
        ..fill(0, 0, 1, 1, 0)
    };
    let halves = vec![
        fill(0, 0, 48, height, 0x0000_0000),
        fill(48, 0, 48, height, 0x00ff_ffff),
        surface,
    ];
    let mut above = halves.clone();
    above.push(fill(40, 20, 16, 16, 0x00ff_0000));
    let rounded = MaterialArea {
        radius: 12,
        ..area(24, 16, 48, 32)
    };
    type Case<'a> = (&'a str, Vec<Layer<'a>>, Vec<(usize, MaterialArea)>);
    let cases: Vec<Case<'_>> = vec![
        (
            "grey",
            vec![fill(0, 0, width, height, 0x0080_8080), surface],
            vec![(1, area(10, 10, 40, 30))],
        ),
        (
            "flat",
            vec![fill(0, 0, width, height, 0x0060_8040), surface],
            vec![(1, area(10, 10, 40, 30))],
        ),
        ("edge", halves.clone(), vec![(2, area(24, 16, 48, 32))]),
        ("small", halves.clone(), vec![(2, area(40, 24, 16, 16))]),
        ("large", halves.clone(), vec![(2, area(8, 4, 80, 56))]),
        ("above", above, vec![(2, area(24, 16, 48, 32))]),
        ("rounded", halves.clone(), vec![(2, rounded)]),
        ("none", halves.clone(), vec![]),
    ];
    for (what, layers, materials) in &cases {
        assert_agrees(&mut gpu, (width, height), layers, materials, what);
    }
    // A grey stays that grey on the device too.
    let grey = gpu
        .compose_with_material(width, height, 0, &cases[0].1, &cases[0].2)
        .unwrap();
    assert!(grey.iter().all(|&pixel| pixel == 0x0080_8080));
}

/// What a frame damages, as the compositor works it out: `changed`, and every area whose source
/// it touches, whole.
fn damage_with_areas(changed: &[Rect], materials: &[(usize, MaterialArea)]) -> Vec<Rect> {
    let mut damage = changed.to_vec();
    for (_, area) in materials {
        let grow = reach(area.sigma) as i32;
        let around = Rect {
            x: area.rect.x - grow,
            y: area.rect.y - grow,
            width: area.rect.width + 2 * grow as u32,
            height: area.rect.height + 2 * grow as u32,
        };
        if changed.iter().any(|rect| rect.intersect(&around).is_some()) {
            damage.push(area.rect);
        }
    }
    damage
}

/// Composing damage with material areas gives each pixel what composing whole gives: a popup
/// moving beneath an area, within its reach and beyond it, and damage that covers part of an
/// area while nothing changed.
#[test]
fn damaged_composition_with_material_is_the_reference() {
    let Some(mut gpu) = device() else {
        return;
    };
    let (window, popup) = frame();
    let menu = menu_pixels(60, 50);
    let first = with_menu(&window, &popup, &menu);
    let materials = [
        (
            3,
            MaterialArea {
                radius: 12,
                sigma: 25,
                ..area(20, 10, 60, 50)
            },
        ),
        (1, area(0, 60, 40, 30)),
    ];
    let (width, height) = (160, 120);
    let whole = |layers: &[Layer<'_>]| {
        compose_cpu_with_material(width, height, 0x0011_2233, layers, &materials)
    };
    // No frame yet: composed whole whatever the damage says.
    assert_eq!(
        gpu.compose_damaged_with_material(width, height, 0x0011_2233, &first, &materials, &[])
            .unwrap(),
        whole(&first)
    );
    let mut cpu = whole(&first);
    let mut frame = first.clone();
    for (dx, dy) in [(15, -20), (-30, 25), (40, 40), (-50, -10)] {
        let (moved, changed) = popup_moved(&frame, dx, dy);
        frame = moved;
        let damage = damage_with_areas(&changed, &materials);
        let vulkan = gpu
            .compose_damaged_with_material(width, height, 0x0011_2233, &frame, &materials, &damage)
            .unwrap();
        assert_eq!(vulkan, whole(&frame), "the popup moved by ({dx}, {dy})");
        compose_cpu_damaged_with_material(
            &mut cpu,
            width,
            height,
            0x0011_2233,
            &frame,
            &materials,
            &damage,
        );
        assert_eq!(
            cpu, vulkan,
            "the reference from damage, moved by ({dx}, {dy})"
        );
    }
    // Damage over parts of the areas, nothing changed: those parts composed again alike.
    let partial = [
        Rect {
            x: 10,
            y: 5,
            width: 30,
            height: 20,
        },
        Rect {
            x: 50,
            y: 40,
            width: 70,
            height: 35,
        },
        Rect {
            x: 0,
            y: 80,
            width: 20,
            height: 10,
        },
    ];
    assert_eq!(
        gpu.compose_damaged_with_material(width, height, 0x0011_2233, &frame, &materials, &partial)
            .unwrap(),
        whole(&frame)
    );
    // The damage limits what is composed: a material area moved without damaging its old place
    // leaves it there.
    let moved = [(materials[0].0, area(70, 60, 60, 50)), materials[1]];
    let too_small = gpu
        .compose_damaged_with_material(
            width,
            height,
            0x0011_2233,
            &frame,
            &moved,
            &[moved[0].1.rect],
        )
        .unwrap();
    assert_ne!(
        too_small,
        compose_cpu_with_material(width, height, 0x0011_2233, &frame, &moved),
        "the area's old place was not composed again"
    );
}

/// An area over a client's buffer the device does not hold blurs the pixels the CPU read of it,
/// as the reference does; a premultiplied one, translucent, too.
#[test]
fn an_area_over_a_client_buffer_blurs_its_pixels() {
    let Some(mut gpu) = device() else {
        return;
    };
    let (window, popup) = frame();
    let menu = menu_pixels(60, 50);
    let mut layers = with_menu(&window, &popup, &menu);
    // Neither was imported: drawn from their pixels.
    layers[0].paint = Paint::Imported {
        image: 0xdead_beef,
        opaque: true,
        pixels: &window,
    };
    layers[1].paint = Paint::Imported {
        image: 0xfeed_f00d,
        opaque: false,
        pixels: &popup,
    };
    layers[1].opacity = 160;
    let materials = [
        (3, area(20, 10, 60, 50)),
        (
            2,
            MaterialArea {
                sigma: 40,
                radius: 12,
                ..area(60, 35, 40, 40)
            },
        ),
    ];
    assert_agrees(&mut gpu, (120, 90), &layers, &materials, "client buffers");
}

/// A display's buffers take the material too, whole and from damage, as the reference composes
/// it at their size.
#[cfg(target_os = "linux")]
#[test]
fn scanout_frames_with_material_are_the_reference() {
    let Some(mut gpu) = device() else {
        return;
    };
    if !gpu.exports_dma_buf() {
        eprintln!("{} exports no DMA-BUF, skipped", gpu.device_name());
        return;
    }
    let (window, popup) = frame();
    let menu = menu_pixels(60, 50);
    let _targets = gpu.scanout_targets(2, 120, 90).unwrap();
    let materials = [(
        3,
        MaterialArea {
            radius: 12,
            sigma: 40,
            ..area(20, 10, 60, 50)
        },
    )];
    let mut frame = with_menu(&window, &popup, &menu);
    let mut damage: Option<Vec<Rect>> = None;
    for (index, (target, step)) in [(0, (0, 0)), (1, (12, 9)), (0, (-25, 14)), (1, (8, -30))]
        .into_iter()
        .enumerate()
    {
        if index > 0 {
            let (moved, changed) = popup_moved(&frame, step.0, step.1);
            frame = moved;
            damage = Some(damage_with_areas(&changed, &materials));
        }
        gpu.compose_to_scanout_kept_with_material(
            target,
            0x0011_2233,
            &frame,
            &materials,
            damage.as_deref(),
        )
        .unwrap();
        assert_eq!(
            gpu.read_kept().unwrap(),
            compose_cpu_with_material(120, 90, 0x0011_2233, &frame, &materials),
            "frame {index} on target {target}"
        );
    }
}

/// The widest box the device sums (σ at its limit) still agrees: no channel's sum overflows.
#[test]
fn the_widest_blur_agrees() {
    let Some(mut gpu) = device() else {
        return;
    };
    let (window, popup) = frame();
    let menu = menu_pixels(60, 50);
    let layers = with_menu(&window, &popup, &menu);
    let sigma = crate::material::MAX_SIGMA;
    assert!(box_width(sigma) > 100);
    let materials = [(
        3,
        MaterialArea {
            sigma,
            ..area(20, 10, 60, 50)
        },
    )];
    assert_agrees(&mut gpu, (320, 200), &layers, &materials, "σ at its limit");
}
