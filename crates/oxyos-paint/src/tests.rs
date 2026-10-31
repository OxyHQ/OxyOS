//! Each operation on a Vulkan device, against what the CPU painter makes of it
//! (tools/rn-host-spike/renderer.cpp): a pixel covered whole takes the colour exactly, one half
//! covered is blended by half, what is outside is untouched. The two painters' frames are compared
//! whole by the Bloom host (`OXYOS_HOST_RENDER_CHECK`, docs/adr/0031, decision 13).

use super::*;

fn paint() -> Option<Paint> {
    match Paint::new(0) {
        Ok(paint) => Some(paint),
        Err(error) => {
            eprintln!("no Vulkan device for painting, skipped: {error}");
            None
        }
    }
}

fn whole(width: i32, height: i32) -> Pixels {
    Pixels {
        x0: 0,
        y0: 0,
        x1: width,
        y1: height,
    }
}

const EVERYWHERE: [f32; 4] = [-1e9, -1e9, 1e9, 1e9];

fn common(width: i32, height: i32, color: [f32; 4]) -> Common {
    Common {
        pixels: whole(width, height),
        scale: 1.0,
        clip: EVERYWHERE,
        color,
        opacity: 1.0,
    }
}

#[test]
fn a_fill_covers_whole_pixels_with_its_colour_and_half_pixels_by_half() {
    let Some(mut paint) = paint() else { return };
    let canvas = paint.canvas(8, 4, 0xffff_ffff).unwrap();
    // Red over white from x = 2 to 5.5: pixels 2..5 whole, 5 half.
    paint
        .fill(
            canvas,
            &common(8, 4, [1.0, 0.0, 0.0, 1.0]),
            &Fill {
                rect: [2.0, 0.0, 3.5, 4.0],
                radii: [0.0; 4],
                hole: None,
            },
        )
        .unwrap();
    let pixels = paint.read(canvas).unwrap();
    assert_eq!(pixels[1], 0xffff_ffff);
    assert_eq!(pixels[2], 0xffff_0000);
    assert_eq!(pixels[4], 0xffff_0000);
    // Half red over white: 255, 128, 128 (lround of 127.5).
    assert_eq!(pixels[5], 0xffff_8080);
    assert_eq!(pixels[6], 0xffff_ffff);
}

#[test]
fn a_rounded_fill_leaves_its_corners_and_a_ring_its_hole() {
    let Some(mut paint) = paint() else { return };
    let canvas = paint.canvas(32, 32, 0).unwrap();
    paint
        .fill(
            canvas,
            &common(32, 32, [0.0, 0.0, 1.0, 1.0]),
            &Fill {
                rect: [0.0, 0.0, 32.0, 32.0],
                radii: [12.0; 4],
                hole: Some(([4.0, 4.0, 24.0, 24.0], [8.0; 4])),
            },
        )
        .unwrap();
    let pixels = paint.read(canvas).unwrap();
    assert_eq!(pixels[0], 0, "a corner outside the circle");
    assert_eq!(pixels[16 * 32 + 1], 0xff00_00ff, "the ring");
    assert_eq!(pixels[16 * 32 + 16], 0, "the hole");
    // The corner's edge is partly covered: some alpha, not all.
    let edge = pixels[3 * 32 + 3] >> 24;
    assert!(edge > 0 && edge < 255, "{edge}");
}

#[test]
fn a_picture_at_its_own_size_is_its_texels_and_a_tint_keeps_their_alpha() {
    let Some(mut paint) = paint() else { return };
    let canvas = paint.canvas(4, 1, 0).unwrap();
    // R, G, B, A premultiplied: opaque green, half transparent black, nothing, opaque white.
    let rgba = [0, 255, 0, 255, 0, 0, 0, 128, 0, 0, 0, 0, 255, 255, 255, 255];
    assert!(paint.picture(7, 4, 1, &rgba).unwrap());
    assert!(
        !paint.picture(7, 4, 1, &rgba).unwrap(),
        "known: not uploaded again"
    );
    let image = Image {
        source: Source::Picture(7),
        rect: [0.0, 0.0, 4.0, 1.0],
        bounds: [0.0, 0.0, 4.0, 1.0],
        radii: [0.0; 4],
        tinted: false,
    };
    paint
        .image(canvas, &common(4, 1, [0.0; 4]), &image)
        .unwrap();
    assert_eq!(
        paint.read(canvas).unwrap(),
        vec![0xff00_ff00, 0x8000_0000, 0, 0xffff_ffff]
    );
    let tinted = paint.canvas(4, 1, 0).unwrap();
    paint
        .image(
            tinted,
            &common(4, 1, [1.0, 0.0, 0.0, 1.0]),
            &Image {
                tinted: true,
                ..image
            },
        )
        .unwrap();
    assert_eq!(
        paint.read(tinted).unwrap(),
        vec![0xffff_0000, 0x8080_0000, 0, 0xffff_0000]
    );
}

#[test]
fn glyphs_blend_their_masks_in_the_runs_colour() {
    let Some(mut paint) = paint() else { return };
    let canvas = paint.canvas(6, 2, 0xff00_0000).unwrap();
    paint.glyph(1, 2, 2, &[255, 0, 128, 255]).unwrap();
    paint
        .glyphs(
            canvas,
            &common(6, 2, [1.0, 1.0, 1.0, 1.0]),
            &[
                Glyph {
                    x: 0,
                    y: 0,
                    mask: 1,
                },
                Glyph {
                    x: 3,
                    y: 0,
                    mask: 1,
                },
            ],
        )
        .unwrap();
    let pixels = paint.read(canvas).unwrap();
    assert_eq!(pixels[0], 0xffff_ffff);
    assert_eq!(pixels[1], 0xff00_0000);
    assert_eq!(pixels[6], 0xff80_8080);
    assert_eq!(pixels[3], 0xffff_ffff);
    assert_eq!(pixels[2], 0xff00_0000, "between the glyphs");
}

#[test]
fn a_layer_mapped_one_to_one_is_its_canvas_and_moved_is_moved() {
    let Some(mut paint) = paint() else { return };
    let source = paint.canvas(4, 4, 0).unwrap();
    paint
        .fill(
            source,
            &common(4, 4, [0.0, 1.0, 0.0, 1.0]),
            &Fill {
                rect: [1.0, 1.0, 2.0, 2.0],
                radii: [0.0; 4],
                hole: None,
            },
        )
        .unwrap();
    let target = paint.canvas(8, 8, 0xffff_ffff).unwrap();
    paint
        .layer(
            target,
            &common(8, 8, [0.0; 4]),
            &Layer {
                canvas: source,
                layer_scale: 1.0,
                origin: [0.0, 0.0],
                // Moved 2 right: the target's (x, y) is the layer's (x - 2, y).
                inverse: [1.0, 0.0, 0.0, 1.0, -2.0, 0.0],
            },
        )
        .unwrap();
    let pixels = paint.read(target).unwrap();
    assert_eq!(pixels[8 + 3], 0xff00_ff00);
    assert_eq!(pixels[2 * 8 + 4], 0xff00_ff00);
    assert_eq!(pixels[8 + 1], 0xffff_ffff);
}

#[test]
fn a_flat_canvas_blurred_stays_flat_and_drawn_smooth_is_its_colour() {
    let Some(mut paint) = paint() else { return };
    let canvas = paint.canvas(16, 16, 0xff20_4060).unwrap();
    paint.blur(canvas, 3.0).unwrap();
    assert!(paint
        .read(canvas)
        .unwrap()
        .iter()
        .all(|&pixel| pixel == 0xff20_4060));
    let target = paint.canvas(16, 16, 0).unwrap();
    paint
        .smooth(
            target,
            &common(16, 16, [0.0; 4]),
            &Smooth {
                source: canvas,
                sigma: 2.0,
                rect: [0.0, 0.0, 16.0, 16.0],
                bounds: [0.0, 0.0, 16.0, 16.0],
                radii: [0.0; 4],
            },
        )
        .unwrap();
    assert!(paint
        .read(target)
        .unwrap()
        .iter()
        .all(|&pixel| pixel == 0xff20_4060));
}

#[test]
fn a_mask_keeps_content_by_its_alpha_and_a_resized_canvas_keeps_its_top_left() {
    let Some(mut paint) = paint() else { return };
    let content = paint.canvas(2, 1, 0xffff_ffff).unwrap();
    let mask = paint.canvas(2, 1, 0).unwrap();
    paint
        .clear(
            mask,
            Pixels {
                x0: 1,
                y0: 0,
                x1: 2,
                y1: 1,
            },
            0xff00_0000,
        )
        .unwrap();
    paint.mask(content, mask).unwrap();
    assert_eq!(paint.read(content).unwrap(), vec![0, 0xffff_ffff]);
    paint.resize(content, 3, 2, 0x1234_5678).unwrap();
    assert_eq!(
        paint.read(content).unwrap(),
        vec![
            0,
            0xffff_ffff,
            0x1234_5678,
            0x1234_5678,
            0x1234_5678,
            0x1234_5678
        ]
    );
    let copy = paint.copy(content).unwrap();
    assert_eq!(paint.read(copy).unwrap(), paint.read(content).unwrap());
}

#[test]
fn the_heap_grows_and_keeps_what_it_held() {
    let Some(mut paint) = paint() else { return };
    let first = paint.canvas(64, 64, 0xff11_2233).unwrap();
    // More than the heap starts with.
    let big = paint.canvas(4096, 4096, 0xff44_5566).unwrap();
    assert!(paint
        .read(first)
        .unwrap()
        .iter()
        .all(|&pixel| pixel == 0xff11_2233));
    assert_eq!(paint.read(big).unwrap()[4096 * 4096 - 1], 0xff44_5566);
    paint.free_canvas(big);
}

/// The CPU painter keeps a glyph's pixel where `px / scale`, divided in single precision, lies in
/// [clip.x0, clip.x1): a clip edge falling exactly on a column at a fractional scale keeps that
/// column on the left and drops it on the right. With the shader dividing, lavapipe got 334 of
/// these columns wrong (at 1.1 and 1.75), a glyph's column kept or dropped whole.
#[test]
fn a_glyph_clip_on_a_column_keeps_it_on_the_left_and_drops_it_on_the_right_at_every_scale() {
    let Some(mut paint) = paint() else { return };
    const WIDTH: i32 = 2048;
    paint.glyph(1, 1, 1, &[255]).unwrap();
    let mut missed = Vec::new();
    for scale in [1.25f32, 1.5, 1.75, 2.25, 1.1, 1.333_333_4] {
        let canvas = paint.canvas(WIDTH as u32, 2, 0xff00_0000).unwrap();
        for px in 0..WIDTH {
            let edge = px as f32 / scale;
            for (row, clip) in [(0, [edge, -1e9, 1e9, 1e9]), (1, [-1e9, -1e9, edge, 1e9])] {
                let common = Common {
                    pixels: Pixels {
                        x0: px,
                        y0: row,
                        x1: px + 1,
                        y1: row + 1,
                    },
                    scale,
                    clip,
                    color: [1.0, 1.0, 1.0, 1.0],
                    opacity: 1.0,
                };
                let glyph = Glyph {
                    x: px,
                    y: row,
                    mask: 1,
                };
                paint.glyphs(canvas, &common, &[glyph]).unwrap();
            }
        }
        let pixels = paint.read(canvas).unwrap();
        for px in 0..WIDTH as usize {
            if pixels[px] != 0xffff_ffff {
                missed.push(format!(
                    "{scale}: column {px} dropped by a clip starting on it"
                ));
            }
            if pixels[WIDTH as usize + px] != 0xff00_0000 {
                missed.push(format!("{scale}: column {px} kept by a clip ending on it"));
            }
        }
        paint.free_canvas(canvas);
    }
    assert!(
        missed.is_empty(),
        "{} columns: {:?}",
        missed.len(),
        &missed[..missed.len().min(12)]
    );
}

/// A buffer's modifiers, as the host offers them to its driver (docs/adr/0031-gpu-buffers.mdx,
/// decision 2): the best kind it shares with the compositor alone, tiled before linear before
/// compressed, so the driver never takes linear or a compressed one while a tiled one would do.
#[test]
fn a_buffer_is_offered_the_best_kind_of_modifier_both_sides_share() {
    let intel = |n: u64| (0x01 << 56) | n;
    let exportable = [(0, 1), (intel(1), 1), (intel(2), 1), (intel(6), 2)];
    let everything = [intel(1), intel(2), 0, intel(6)];
    assert_eq!(
        present::best_shared(&everything, &exportable),
        vec![intel(1), intel(2)],
        "the tiled ones, for the driver to pick its best"
    );
    assert_eq!(
        present::best_shared(&[0, intel(6)], &exportable),
        vec![0],
        "linear before compressed"
    );
    assert_eq!(
        present::best_shared(&[intel(6)], &exportable),
        vec![intel(6)]
    );
    assert!(present::best_shared(&[intel(9)], &exportable).is_empty());
}
