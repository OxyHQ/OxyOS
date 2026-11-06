use super::*;
use std::collections::HashMap;

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
        warp: None,
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

fn rect(x: i32, y: i32, width: u32, height: u32) -> oxyos_gpu::Rect {
    oxyos_gpu::Rect {
        x,
        y,
        width,
        height,
    }
}

/// A layer of a frame on a 1920 × 1200 screen: an opaque client's buffer at its own size.
fn window(at: oxyos_gpu::Rect) -> oxyos_gpu::Layer<'static> {
    oxyos_gpu::Layer {
        x: at.x,
        y: at.y,
        width: at.width,
        height: at.height,
        extent: (at.width, at.height),
        paint: oxyos_gpu::Paint::Imported {
            image: 7,
            opaque: true,
            pixels: &[],
        },
        source: None,
        changed: None,
        opacity: 255,
        radius: 0,
        filter: oxyos_gpu::Filter::Box,
        clip: None,
        warp: None,
    }
}

#[test]
fn only_a_plain_opaque_rectangle_within_the_screen_can_go_on_a_plane() {
    // A maximized window: under the menu bar, over the dock's row.
    let maximized = window(rect(0, 36, 1920, 1100));
    assert!(plain(&maximized, 1920, 1200));
    let changed = |change: fn(&mut oxyos_gpu::Layer<'static>)| {
        let mut layer = maximized;
        change(&mut layer);
        plain(&layer, 1920, 1200)
    };
    assert!(
        !changed(|l| l.radius = 12),
        "rounded corners show what is under them"
    );
    assert!(!changed(|l| l.opacity = 200), "fading");
    assert!(!changed(|l| l.extent = (960, 550)), "scaled");
    assert!(!changed(|l| l.y = 150), "past the screen's bottom");
    assert!(!changed(|l| l.x = -1), "past its left");
    assert!(
        !changed(|l| l.clip = Some(rect(0, 36, 900, 1100))),
        "cut by a clip"
    );
    assert!(
        changed(|l| l.clip = Some(rect(-5, 0, 2000, 1200))),
        "a clip around it all"
    );
    assert!(
        !changed(|l| l.paint = oxyos_gpu::Paint::Imported {
            image: 7,
            opaque: false,
            pixels: &[],
        }),
        "translucent: what is under it shows through"
    );
    assert!(
        !changed(|l| l.paint = oxyos_gpu::Paint::Opaque(&[])),
        "pixels in shared memory: no buffer a plane can show"
    );
}

#[test]
fn the_topmost_plain_layer_goes_on_the_plane_while_nothing_over_it_touches_it() {
    let background = window(rect(0, 0, 1920, 1200));
    let maximized = window(rect(0, 36, 1920, 1100));
    let bar = oxyos_gpu::Layer {
        paint: oxyos_gpu::Paint::Premultiplied(&[]),
        ..window(rect(0, 0, 1920, 36))
    };
    // The menu bar beside it, not over it: the window goes on the plane.
    assert_eq!(
        overlay_layer(&[background, maximized, bar], 1920, 1200),
        Some((1, true))
    );
    // A menu over it: still the candidate (told the plane's formats), but composed.
    let menu = oxyos_gpu::Layer {
        paint: oxyos_gpu::Paint::Premultiplied(&[]),
        ..window(rect(100, 30, 200, 300))
    };
    assert_eq!(
        overlay_layer(&[background, maximized, bar, menu], 1920, 1200),
        Some((1, false))
    );
    // One clipped away from it does not touch it.
    let clipped = oxyos_gpu::Layer {
        clip: Some(rect(0, 0, 400, 30)),
        ..menu
    };
    assert_eq!(
        overlay_layer(&[background, maximized, bar, clipped], 1920, 1200),
        Some((1, true))
    );
    // With rounded corners, the window is no candidate; the background under it is, covered.
    let rounded = oxyos_gpu::Layer {
        radius: 12,
        ..maximized
    };
    assert_eq!(
        overlay_layer(&[background, rounded], 1920, 1200),
        Some((0, false))
    );
}

#[test]
fn under_the_plane_only_what_changed_around_it_and_what_it_stops_covering_is_composed() {
    let screen = rect(0, 0, 1920, 1200);
    let window = rect(0, 36, 1920, 1100);
    let area = |rects: &[oxyos_gpu::Rect]| -> u64 {
        rects
            .iter()
            .map(|r| u64::from(r.width) * u64::from(r.height))
            .sum()
    };
    // The window's own frame, on the plane: nothing composed.
    let inside = [rect(200, 300, 400, 100)];
    assert_eq!(
        under_overlay(Some(&inside), Some(window), Some(window), screen),
        Some(vec![])
    );
    // The clock in the menu bar: that alone.
    let clock = [rect(1800, 8, 80, 20)];
    assert_eq!(
        under_overlay(Some(&clock), Some(window), Some(window), screen),
        Some(clock.to_vec())
    );
    // A whole frame with the window on the plane: all but the window.
    let around = under_overlay(None, None, Some(window), screen).unwrap();
    assert_eq!(area(&around), 1920 * 1200 - 1920 * 1100);
    assert!(around.iter().all(|r| r.intersect(&window).is_none()));
    // Off the plane again: the window's place composed, as it is under it.
    assert_eq!(
        under_overlay(Some(&[]), Some(window), None, screen),
        Some(vec![window])
    );
    // Moved down by 10: the strip it left composed, nothing under where it is now.
    let moved = rect(0, 46, 1920, 1100);
    let left = under_overlay(Some(&[]), Some(window), Some(moved), screen).unwrap();
    assert_eq!(left, [rect(0, 36, 1920, 10)]);
    // With no plane before or now, the damage is as it was, whole or not.
    assert_eq!(under_overlay(None, None, None, screen), None);
    assert_eq!(
        under_overlay(Some(&clock), None, None, screen),
        Some(clock.to_vec())
    );
}

#[test]
fn the_card_is_asked_once_for_each_buffer_and_place_and_a_refusal_composes() {
    let mut tested = HashMap::new();
    let at = rect(0, 36, 1920, 1100);
    let mut asked = 0;
    let refused = || -> std::io::Result<()> { Err(std::io::Error::other("EINVAL")) };
    assert!(!taken(&mut tested, (1, at), || {
        asked += 1;
        refused()
    }));
    // Refused once, never asked again: composed every frame after.
    assert!(!taken(&mut tested, (1, at), || {
        asked += 1;
        Ok(())
    }));
    assert_eq!(asked, 1);
    // Another buffer, or the same at another place, is asked.
    assert!(taken(&mut tested, (2, at), || Ok(())));
    assert!(taken(&mut tested, (1, rect(0, 40, 1920, 1100)), || Ok(())));
}
