use super::*;
use crate::{
    compose_cpu, compose_cpu_damaged_with_material, compose_cpu_with_material, Filter, Layer, Paint,
};

const W: u32 = 96;
const H: u32 = 64;

fn fill(x: i32, y: i32, width: u32, height: u32, colour: u32) -> Layer<'static> {
    Layer {
        x,
        y,
        width,
        height,
        extent: (width, height),
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

/// Black on the left half, white on the right: a sharp edge at x = 48.
fn halves() -> Vec<Layer<'static>> {
    vec![
        fill(0, 0, 48, H, 0x0000_0000),
        fill(48, 0, 48, H, 0x00ff_ffff),
    ]
}

#[test]
fn a_grey_stays_that_grey() {
    let layers = [fill(0, 0, W, H, 0x0080_8080), surface()];
    let out = compose_cpu_with_material(W, H, 0, &layers, &[(1, area(10, 10, 40, 30))]);
    assert!(out.iter().all(|&pixel| pixel == 0x0080_8080));
}

#[test]
fn a_flat_colour_is_saturated_and_nothing_else() {
    let layers = [fill(0, 0, W, H, 0x0060_8040), surface()];
    let out = compose_cpu_with_material(W, H, 0, &layers, &[(1, area(10, 10, 40, 30))]);
    // (96, 128, 64) through the matrix, in 4096ths, rounded and clamped.
    let channel = |row: [i32; 3]| {
        ((row[0] * 96 + row[1] * 128 + row[2] * 64 + 2048) >> 12).clamp(0, 255) as u32
    };
    let saturated =
        (channel(SATURATION[0]) << 16) | (channel(SATURATION[1]) << 8) | channel(SATURATION[2]);
    assert_eq!(out[(20 * W + 20) as usize], saturated);
    assert_ne!(saturated, 0x0060_8040);
    // Outside the area, the colour as it is.
    assert_eq!(out[(5 * W + 5) as usize], 0x0060_8040);
}

#[test]
fn an_edge_beneath_is_blurred_within_the_area_alone() {
    let layers = halves();
    let out = compose_cpu_with_material(W, H, 0, &layers, &[(2, area(24, 16, 48, 32))]);
    let at = |x: u32, y: u32| out[(y * W + x) as usize] & 0xff;
    // Either side of the edge, inside the area, the two mix.
    assert!(at(46, 30) > 0 && at(46, 30) < 255, "{}", at(46, 30));
    assert!(at(50, 30) > 0 && at(50, 30) < 255, "{}", at(50, 30));
    assert!(at(46, 30) < at(50, 30));
    // Outside it, the edge as sharp as it was.
    assert_eq!(at(46, 5), 0);
    assert_eq!(at(50, 5), 255);
}

#[test]
fn a_regions_pixels_do_not_depend_on_the_region() {
    // The same pixel in a small region and a large one around it is blurred alike: the cells are
    // the output's, and the source reaches far enough.
    let layers = halves();
    let small = compose_cpu_with_material(W, H, 0, &layers, &[(2, area(40, 24, 16, 16))]);
    let large = compose_cpu_with_material(W, H, 0, &layers, &[(2, area(8, 4, 80, 56))]);
    for y in 24..40 {
        for x in 40..56 {
            let index = (y * W + x) as usize;
            assert_eq!(small[index], large[index], "({x}, {y})");
        }
    }
}

#[test]
fn what_is_above_the_material_is_drawn_over_it() {
    let mut layers = halves();
    // Drawn by the surface over its material: here a layer above it.
    layers.push(fill(40, 20, 16, 16, 0x00ff_0000));
    let out = compose_cpu_with_material(W, H, 0, &layers, &[(2, area(24, 16, 48, 32))]);
    assert_eq!(out[(28 * W + 48) as usize], 0x00ff_0000);
}

#[test]
fn no_opacity_or_no_area_changes_nothing() {
    let layers = halves();
    let plain = compose_cpu(W, H, 0, &layers);
    let mut faded = area(24, 16, 48, 32);
    faded.opacity = 0;
    assert_eq!(
        compose_cpu_with_material(W, H, 0, &layers, &[(2, faded)]),
        plain
    );
    assert_eq!(
        compose_cpu_with_material(W, H, 0, &layers, &[(2, area(200, 200, 10, 10))]),
        plain
    );
    assert_eq!(compose_cpu_with_material(W, H, 0, &layers, &[]), plain);
}

#[test]
fn rounded_corners_leave_what_is_beneath() {
    let layers = halves();
    let mut rounded = area(24, 16, 48, 32);
    rounded.radius = 12;
    let out = compose_cpu_with_material(W, H, 0, &layers, &[(2, rounded)]);
    let plain = compose_cpu(W, H, 0, &layers);
    // The area's corner pixel is outside its rounded corner: as it was.
    assert_eq!(out[(16 * W + 24) as usize], plain[(16 * W + 24) as usize]);
}

#[test]
fn composing_damage_gives_what_composing_whole_gives() {
    let layers = halves();
    let materials = [(2, area(24, 16, 48, 32))];
    let whole = compose_cpu_with_material(W, H, 0, &layers, &materials);
    let mut out = vec![0x00ab_cdef; (W * H) as usize];
    let damage = [
        Rect {
            x: 0,
            y: 0,
            width: 50,
            height: 30,
        },
        Rect {
            x: 40,
            y: 20,
            width: 56,
            height: 44,
        },
    ];
    compose_cpu_damaged_with_material(&mut out, W, H, 0, &layers, &materials, &damage);
    out.iter()
        .zip(&whole)
        .enumerate()
        .filter(|(index, _)| {
            let (x, y) = ((*index as u32 % W) as i32, (*index as u32 / W) as i32);
            damage.iter().any(|r| {
                x >= r.x && y >= r.y && x < r.x + r.width as i32 && y < r.y + r.height as i32
            })
        })
        .for_each(|(index, (got, want))| assert_eq!(got, want, "pixel {index}"));
}

#[test]
fn box_widths_and_reach() {
    // σ 20 output pixels is 5 cells: ⌊5 · 1.88 + ½⌋ = 9.
    assert_eq!(box_width(20), 9);
    assert_eq!(box_width(25), 12);
    assert_eq!(box_width(0), 1);
    assert_eq!(reach(20), (3 * (9 / 2 + 1) + 2) * CELL);
    assert_eq!(box_width(MAX_SIGMA * 4), box_width(MAX_SIGMA));
}
