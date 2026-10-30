use super::*;

const Y_TILED: u64 = 0x0100_0000_0000_0002;

fn layer(paint: oxyos_gpu::Paint<'_>) -> oxyos_gpu::Layer<'_> {
    oxyos_gpu::Layer {
        x: 0,
        y: 0,
        width: 1920,
        height: 1200,
        extent: (1920, 1200),
        paint,
        source: None,
        changed: None,
        opacity: 255,
        radius: 0,
        filter: oxyos_gpu::Filter::Box,
        clip: None,
    }
}

#[test]
fn only_a_buffer_covering_the_screen_at_its_size_whole_and_opaque_is_scanned_out() {
    let imported = oxyos_gpu::Paint::Imported {
        image: 1,
        opaque: true,
        pixels: &[],
    };
    assert!(covers_screen(&layer(imported), 1920, 1200));
    let changed = |change: fn(&mut oxyos_gpu::Layer<'_>)| {
        let mut drawn = layer(imported);
        change(&mut drawn);
        covers_screen(&drawn, 1920, 1200)
    };
    assert!(!changed(|l| l.x = 1), "moved");
    assert!(
        !changed(|l| l.extent = (960, 600)),
        "scaled up: the buffer is not the screen's size"
    );
    assert!(!changed(|l| l.width = 1919), "drawn smaller (a transition)");
    assert!(!changed(|l| l.opacity = 254), "fading");
    assert!(!changed(|l| l.radius = 16), "its corners cut");
    assert!(
        !changed(|l| {
            l.clip = Some(oxyos_gpu::Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 600,
            })
        }),
        "clipped"
    );
    assert!(changed(|l| {
        l.clip = Some(oxyos_gpu::Rect {
            x: -10,
            y: -10,
            width: 4000,
            height: 4000,
        })
    }));
    // A cover (the lock's), a shadow or the compositor's own fill is never a client's buffer.
    assert!(!covers_screen(
        &layer(oxyos_gpu::Paint::Fill(0)),
        1920,
        1200
    ));
    assert!(
        !covers_screen(&layer(imported), 1600, 1000),
        "another screen"
    );
}

#[test]
fn a_surface_is_told_the_opaque_formats_the_primary_plane_takes_of_the_offer() {
    let offer = [
        (Format::Argb8888, 0, 1),
        (Format::Xrgb8888, 0, 1),
        (Format::Argb8888, Y_TILED, 2),
        (Format::Xrgb8888, Y_TILED, 2),
    ];
    let primary = [
        (DrmFourcc::Xrgb8888, 0),
        (DrmFourcc::Argb8888, 0),
        (DrmFourcc::Xrgb8888, Y_TILED),
    ];
    // On the composing device: every modifier the plane takes; opaque only, as nothing is under
    // the plane to blend with.
    assert_eq!(
        scanout_formats(&offer, &primary, true),
        vec![(Format::Xrgb8888, 0), (Format::Xrgb8888, Y_TILED)]
    );
    // Another device's: linear alone, which any device reads.
    assert_eq!(
        scanout_formats(&offer, &primary, false),
        vec![(Format::Xrgb8888, 0)]
    );
    // A plane that takes none of the offer: nothing to tell.
    assert!(scanout_formats(&offer, &[(DrmFourcc::Rgb565, 0)], true).is_empty());
}
