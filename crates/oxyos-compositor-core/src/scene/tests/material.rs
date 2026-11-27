//! A surface's material regions (docs/adr/0069-the-platforms-material.mdx): told in order before
//! the Submit they belong to, held from when that frame is composed, replaced by a later list's
//! frame, refused past their limits.

use super::*;
use crate::scene::MaterialRegion;
use oxyos_protocol::MAX_MATERIAL_REGIONS;

fn region(surface: u32, index: u8, last: bool, rect: Rect, radius: u32) -> Request {
    Request::MaterialRegion {
        surface,
        index,
        last,
        rect,
        radius,
    }
}

const MENU: Rect = Rect::new(0, 0, 240, 160);
const TIP: Rect = Rect::new(10, 10, 80, 24);

#[test]
fn a_list_holds_from_its_frame_displayed() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    assert!(h.scene.material_regions(window).is_empty());
    h.request(APP, region(1, 0, false, MENU, 12)).unwrap();
    h.request(APP, region(1, 1, true, TIP, 6)).unwrap();
    // Told, but no frame of its own displayed yet.
    assert!(h.scene.material_regions(window).is_empty());
    h.present(window);
    assert_eq!(
        h.scene.material_regions(window),
        &[
            MaterialRegion {
                rect: MENU,
                radius: 12
            },
            MaterialRegion {
                rect: TIP,
                radius: 6
            },
        ]
    );
    // A later frame with no list told keeps it.
    h.present(window);
    assert_eq!(h.scene.material_regions(window).len(), 2);
}

#[test]
fn an_empty_list_removes_them() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.request(APP, region(1, 0, true, MENU, 12)).unwrap();
    h.present(window);
    assert_eq!(h.scene.material_regions(window).len(), 1);
    h.request(APP, region(1, 0, true, Rect::new(0, 0, 0, 0), 0))
        .unwrap();
    h.present(window);
    assert!(h.scene.material_regions(window).is_empty());
}

#[test]
fn a_list_waits_for_its_frame_while_an_older_one_shows() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.request(APP, region(1, 0, true, MENU, 12)).unwrap();
    h.present(window);
    h.request(APP, region(1, 0, true, TIP, 6)).unwrap();
    h.submit(window);
    // Submitted, not yet displayed: the frame on screen keeps its own list.
    assert_eq!(
        h.scene.material_regions(window),
        &[MaterialRegion {
            rect: MENU,
            radius: 12
        }]
    );
    h.frame();
    assert_eq!(
        h.scene.material_regions(window),
        &[MaterialRegion {
            rect: TIP,
            radius: 6
        }]
    );
}

#[test]
fn a_frame_composes_with_its_own_list() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.request(APP, region(1, 0, true, MENU, 12)).unwrap();
    h.present(window);
    h.request(APP, region(1, 0, true, MENU, 0)).unwrap();
    h.submit(window);
    h.pump();
    // While the frame it was told for composes, its list is the one drawn: its pixels are composed
    // with it, never with the list of the frame they replace.
    h.scene.begin_frame();
    assert_eq!(
        h.scene.material_regions(window),
        &[MaterialRegion {
            rect: MENU,
            radius: 0
        }]
    );
    h.scene.end_frame(1, Clock::Synthetic);
    assert_eq!(
        h.scene.material_regions(window),
        &[MaterialRegion {
            rect: MENU,
            radius: 0
        }]
    );
}

#[test]
fn regions_out_of_order_beyond_the_limit_or_misshapen_are_refused() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    assert_eq!(
        h.request(APP, region(1, 1, true, MENU, 0)),
        Err(ErrorCode::InvalidOrder)
    );
    assert_eq!(
        h.request(APP, region(1, 0, true, MENU, 81)),
        Err(ErrorCode::InvalidGeometry),
        "a radius past half the shorter side"
    );
    assert_eq!(
        h.request(APP, region(1, 0, true, Rect::new(-1, 0, 10, 10), 0)),
        Err(ErrorCode::InvalidGeometry)
    );
    assert_eq!(
        h.request(APP, region(1, 0, false, Rect::new(0, 0, 0, 0), 0)),
        Err(ErrorCode::InvalidGeometry),
        "none is a list's one region"
    );
    for index in 0..MAX_MATERIAL_REGIONS {
        h.request(APP, region(1, index as u8, false, TIP, 0))
            .unwrap();
    }
    assert_eq!(
        h.request(APP, region(1, MAX_MATERIAL_REGIONS as u8, true, TIP, 0)),
        Err(ErrorCode::QuotaExceeded)
    );
    assert_eq!(
        h.request(APP, region(9, 0, true, TIP, 0)),
        Err(ErrorCode::UnknownSurface)
    );
    // Nothing refused reached the frame.
    h.present(window);
    assert!(h.scene.material_regions(window).is_empty());
}

#[test]
fn reduced_transparency_draws_no_material() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.request(APP, region(1, 0, true, MENU, 12)).unwrap();
    h.present(window);
    assert_eq!(h.scene.material_regions(window).len(), 1);
    let mut preferences = drawing(false, "blue");
    preferences.reduce_transparency = true;
    h.scene.set_preferences(preferences, &[]);
    assert!(h.scene.material_regions(window).is_empty());
    preferences.reduce_transparency = false;
    h.scene.set_preferences(preferences, &[]);
    assert_eq!(h.scene.material_regions(window).len(), 1);
}
