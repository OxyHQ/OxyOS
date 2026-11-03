use super::*;

/// A `size` × `size` picture whose alpha is `inside(x, y)` (the pixel's centre, from -1 to 1 across
/// the picture), opaque navy where it is, transparent elsewhere; each pixel's edge anti-aliased by
/// sampling it 4 × 4 times.
fn picture(size: u32, inside: impl Fn(f64, f64) -> bool) -> Image {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let mut hits = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let at = |p: u32, s: u32| {
                        ((p as f64 + (s as f64 + 0.5) / 4.0) / size as f64) * 2.0 - 1.0
                    };
                    hits += u32::from(inside(at(x, sx), at(y, sy)));
                }
            }
            let alpha = (hits * 255 / 16) as u8;
            // Premultiplied navy (#002b64).
            let scale = |c: u32| (c * u32::from(alpha) / 255) as u8;
            pixels.extend([0, scale(0x2b), scale(0x64), alpha]);
        }
    }
    Image::still(size, size, (size, size), pixels)
}

/// A superellipse of exponent `n` filling `share` of the picture: 2 a circle, 3 Dropsea's tile.
fn superellipse(n: f64, share: f64) -> impl Fn(f64, f64) -> bool {
    move |x, y| (x / share).abs().powf(n) + (y / share).abs().powf(n) <= 1.0
}

/// Apple's icon grid's body: a rounded square filling `share` of the picture, its corners 208 of
/// its 896.
fn grid_body(share: f64) -> impl Fn(f64, f64) -> bool {
    let radius = share * 208.0 / 896.0;
    move |x, y| {
        let (dx, dy) = (
            (x.abs() - (share - radius)).max(0.0),
            (y.abs() - (share - radius)).max(0.0),
        );
        x.abs() <= share && y.abs() <= share && dx * dx + dy * dy <= radius * radius
    }
}

#[test]
fn a_rounded_square_edge_to_edge_is_its_own_shape() {
    // Dropsea's squircle, at its 512 and at a small icon's 64.
    assert_eq!(own_shape(&picture(512, superellipse(3.0, 1.0))), Some(1.0));
    assert_eq!(own_shape(&picture(64, superellipse(3.0, 1.0))), Some(1.0));
    // The grid's own rounded square, edge to edge.
    assert_eq!(own_shape(&picture(256, grid_body(1.0))), Some(1.0));
}

#[test]
fn a_rounded_square_on_the_grid_with_its_margins_says_its_body_share() {
    // Drawn on Apple's 1024 grid: the body 896 of it, 64 from each edge.
    let body = own_shape(&picture(256, grid_body(896.0 / 1024.0))).unwrap();
    assert!((body - 0.875).abs() <= 1.0 / 256.0, "{body}");
}

#[test]
fn a_square_a_circle_or_a_body_with_a_hole_is_not() {
    // A square to its corners (Atlas's, Inbox's): its corners are not transparent.
    assert_eq!(own_shape(&picture(128, |_, _| true)), None);
    // A round logo (Allo's): transparent corners and opaque within, but round.
    assert_eq!(own_shape(&picture(192, superellipse(2.0, 1.0))), None);
    // A rounded square with a hole through it (a gear, a ring), or see-through within.
    let ring = |x: f64, y: f64| superellipse(3.0, 1.0)(x, y) && x * x + y * y > 0.1;
    assert_eq!(own_shape(&picture(128, ring)), None);
    let mut glass = picture(128, superellipse(3.0, 1.0));
    let middle = (64 * 128 + 64) * 4;
    glass.pixels[middle + 3] = 200;
    assert_eq!(own_shape(&glass), None);
    // Off centre, or not square.
    assert_eq!(
        own_shape(&picture(128, |x, y| superellipse(3.0, 0.8)(x - 0.15, y))),
        None
    );
    assert_eq!(
        own_shape(&picture(128, |x, y| superellipse(3.0, 0.9)(x, y * 1.5))),
        None
    );
    // Nothing at all, or a picture that is not square.
    assert_eq!(own_shape(&picture(32, |_, _| false)), None);
    assert_eq!(own_shape(&Image::still(2, 1, (2, 1), vec![0; 8])), None);
}

#[test]
fn a_file_is_measured_as_its_picture() {
    let image = picture(64, superellipse(3.0, 1.0));
    // Straight from premultiplied: navy where opaque, nothing elsewhere.
    let straight: Vec<u8> = image
        .pixels
        .chunks_exact(4)
        .flat_map(|p| {
            if p[3] == 0 {
                [0, 0, 0, 0]
            } else {
                [0, 0x2b, 0x64, p[3]]
            }
        })
        .collect();
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 64, 64);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&straight)
        .unwrap();
    let dir = std::env::temp_dir().join(format!("oxyos-image-shape-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("icon.png");
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(own_shape_of_file(&path), Ok(Some(1.0)));
    assert!(own_shape_of_file(&dir.join("missing.png")).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
