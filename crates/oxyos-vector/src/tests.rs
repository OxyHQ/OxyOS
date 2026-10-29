use super::*;

const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
const RED: Argb = 0xffff_0000;

fn filled(geometry: Geometry<'_>) -> Shape<'_> {
    Shape {
        geometry,
        transform: IDENTITY,
        fill: Some(Fill {
            color: RED,
            even_odd: false,
        }),
        stroke: None,
    }
}

/// A pixel as 0xAARRGGBB, premultiplied.
fn at(picture: &Picture, x: u32, y: u32) -> u32 {
    let i = ((y * picture.width + x) * 4) as usize;
    let p = &picture.pixels[i..i + 4];
    u32::from_be_bytes([p[3], p[0], p[1], p[2]])
}

#[test]
fn a_rectangle_on_whole_pixels_covers_exactly_them() {
    let picture = render(
        &[filled(Geometry::Rect {
            x: 2.0,
            y: 3.0,
            width: 4.0,
            height: 2.0,
            rx: 0.0,
            ry: 0.0,
        })],
        10,
        10,
    )
    .unwrap();
    let covered: Vec<(u32, u32)> = (0..10)
        .flat_map(|y| (0..10).map(move |x| (x, y)))
        .filter(|&(x, y)| at(&picture, x, y) != 0)
        .collect();
    let expected: Vec<(u32, u32)> = (3..5).flat_map(|y| (2..6).map(move |x| (x, y))).collect();
    assert_eq!(covered, expected);
    assert_eq!(at(&picture, 2, 3), 0xffff_0000);
}

#[test]
fn the_fill_rule_decides_whether_an_inner_ring_is_a_hole() {
    // Two squares, the inner one drawn in the same direction: nonzero fills it, evenodd does not.
    let ring = "M0 0H20V20H0Z M5 5H15V15H5Z";
    for (even_odd, centre) in [(false, 0xffff_0000), (true, 0)] {
        let shape = Shape {
            fill: Some(Fill {
                color: RED,
                even_odd,
            }),
            ..filled(Geometry::Path(ring))
        };
        let picture = render(&[shape], 20, 20).unwrap();
        assert_eq!(at(&picture, 10, 10), centre, "even_odd {even_odd}");
        assert_eq!(at(&picture, 2, 2), 0xffff_0000);
    }
}

#[test]
fn the_transform_maps_user_space_to_pixels() {
    // A 1×1 square scaled by 4 and moved by (2, 2): it covers (2..6, 2..6).
    let shape = Shape {
        transform: [4.0, 0.0, 0.0, 4.0, 2.0, 2.0],
        ..filled(Geometry::Path("M0 0H1V1H0Z"))
    };
    let picture = render(&[shape], 10, 10).unwrap();
    assert_ne!(at(&picture, 2, 2), 0);
    assert_ne!(at(&picture, 5, 5), 0);
    assert_eq!(at(&picture, 6, 6), 0);
    assert_eq!(at(&picture, 1, 1), 0);
}

#[test]
fn arcs_in_path_data_are_drawn() {
    // A circle of radius 8 at (10, 10) as two arcs.
    let picture = render(
        &[filled(Geometry::Path(
            "M2 10A8 8 0 1 0 18 10A8 8 0 1 0 2 10Z",
        ))],
        20,
        20,
    )
    .unwrap();
    assert_eq!(at(&picture, 10, 10), 0xffff_0000);
    assert_eq!(at(&picture, 0, 0), 0);
    assert_eq!(at(&picture, 19, 19), 0);
}

#[test]
fn a_stroke_has_its_width_and_a_dash_its_gaps() {
    let line = Geometry::Line {
        x1: 0.0,
        y1: 10.0,
        x2: 20.0,
        y2: 10.0,
    };
    let stroke = Stroke {
        color: RED,
        width: 4.0,
        cap: Cap::Butt,
        join: Join::Miter,
        miter_limit: 4.0,
        dash: &[],
        dash_offset: 0.0,
    };
    let shape = |stroke| Shape {
        geometry: line,
        transform: IDENTITY,
        fill: None,
        stroke: Some(stroke),
    };
    let picture = render(&[shape(stroke)], 20, 20).unwrap();
    // 4 pixels wide around y = 10: rows 8 to 11.
    assert!((8..12).all(|y| at(&picture, 5, y) == 0xffff_0000));
    assert_eq!(at(&picture, 5, 7), 0);
    assert_eq!(at(&picture, 5, 12), 0);
    // Dashes of 5 with gaps of 5 (one length, repeated): x 0..5 drawn, 5..10 not.
    let dashed = render(
        &[shape(Stroke {
            dash: &[5.0],
            ..stroke
        })],
        20,
        20,
    )
    .unwrap();
    assert_ne!(at(&dashed, 2, 10), 0);
    assert_eq!(at(&dashed, 7, 10), 0);
    assert_ne!(at(&dashed, 12, 10), 0);
}

#[test]
fn colours_come_out_premultiplied() {
    let shape = Shape {
        fill: Some(Fill {
            color: 0x80ff_0000,
            even_odd: false,
        }),
        ..filled(Geometry::Rect {
            x: 0.0,
            y: 0.0,
            width: 2.0,
            height: 2.0,
            rx: 0.0,
            ry: 0.0,
        })
    };
    let picture = render(&[shape], 2, 2).unwrap();
    assert_eq!(at(&picture, 0, 0), 0x8080_0000);
}

#[test]
fn rounded_corners_leave_the_corner_pixel_clear() {
    let rect = |rx| {
        render(
            &[filled(Geometry::Rect {
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 20.0,
                rx,
                ry: 0.0,
            })],
            20,
            20,
        )
        .unwrap()
    };
    assert_eq!(at(&rect(0.0), 0, 0), 0xffff_0000);
    let rounded = rect(8.0);
    assert_eq!(at(&rounded, 0, 0), 0);
    assert_eq!(at(&rounded, 10, 0), 0xffff_0000);
    assert_eq!(at(&rounded, 10, 10), 0xffff_0000);
}

#[test]
fn what_svg_would_not_draw_is_skipped_and_counted() {
    let shapes = [
        filled(Geometry::Path("M0 0 L")),
        filled(Geometry::Ellipse {
            cx: 5.0,
            cy: 5.0,
            rx: 0.0,
            ry: 3.0,
        }),
        filled(Geometry::Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 5.0,
            rx: 0.0,
            ry: 0.0,
        }),
        filled(Geometry::Ellipse {
            cx: 5.0,
            cy: 5.0,
            rx: 3.0,
            ry: 3.0,
        }),
    ];
    let picture = render(&shapes, 10, 10).unwrap();
    assert_eq!(picture.skipped, 3);
    assert_eq!(at(&picture, 5, 5), 0xffff_0000);
    assert!(render(&shapes, 0, 10).is_none());
    assert!(render(&shapes, MAX_SIDE + 1, 1).is_none());
}

#[test]
fn a_bloom_icon_is_drawn_inside_its_view_box() {
    // Remix Icon `wifi-line`, as Bloom draws it: a 24×24 view box at twice its size.
    let wifi = "M0.689453 6.99659C3.78027 4.49704 7.71526 3 11.9999 3C16.2845 3 20.2195 4.49704 23.3104 6.99659L22.0536 8.55252C19.3062 6.3307 15.8085 5 11.9999 5C8.19133 5 4.69356 6.3307 1.94617 8.55252L0.689453 6.99659ZM10.1148 18.6661C10.63 18.2495 11.2858 18 11.9999 18C12.714 18 13.3698 18.2495 13.885 18.6661L11.9999 21L10.1148 18.6661Z";
    let shape = Shape {
        transform: [2.0, 0.0, 0.0, 2.0, 0.0, 0.0],
        fill: Some(Fill {
            color: RED,
            even_odd: true,
        }),
        ..filled(Geometry::Path(wifi))
    };
    let picture = render(&[shape], 48, 48).unwrap();
    assert_eq!(picture.skipped, 0);
    // The outer arc's band at the top centre, the dot at the bottom, nothing in between.
    assert_eq!(at(&picture, 24, 8), 0xffff_0000);
    assert_eq!(at(&picture, 24, 39), 0xffff_0000);
    assert_eq!(at(&picture, 24, 24), 0);
}
