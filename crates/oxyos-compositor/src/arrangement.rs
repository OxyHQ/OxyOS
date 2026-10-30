//! Where the outputs are and how dense (docs/adr/0018-several-displays.mdx): side by side, left
//! to right, top-aligned, with no gap, the built-in panel first (the primary), then the
//! connectors in the card's order; each at its preferred mode, at a scale from its density.
//!
//! The person may arrange them otherwise, as a Mac's Displays settings do: a display dragged
//! somewhere is placed beside the others, touching one along an edge, over none ([`snap`]), and
//! when a display's size changes (another mode or scale) the others keep where they are against
//! it ([`reflow`]). The arrangement's top-left corner is the global space's origin
//! ([`normalize`]). A display's scale may be any of its choices ([`scale_choices`]): from the most
//! space to the largest text, around the one its density recommends.

use oxyos_compositor_core::scene::SCALE_DENOMINATOR;
use oxyos_protocol::Rect;

/// The density a logical pixel is drawn for, in pixels per inch: a built-in panel's, seen from
/// closer, and a monitor's. A display denser than its reference is scaled by how much, to the
/// nearest quarter.
pub const PANEL_PPI: f64 = 135.0;
pub const MONITOR_PPI: f64 = 110.0;
/// The largest scale chosen (the scene takes up to 4).
pub const MAX_SCALE_120: u32 = 3 * SCALE_DENOMINATOR;

/// A connected display, as the arrangement sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Monitor {
    /// A panel built into the machine (eDP, LVDS, DSI).
    pub internal: bool,
    /// Its mode's size, in pixels.
    pub pixels: (u32, u32),
    /// Its physical size, in millimetres (its EDID's); 0 when unknown.
    pub size_mm: (u32, u32),
}

/// The order outputs are arranged in, left to right, as indices of `monitors` (given in the
/// card's connector order): the built-in panels first, each group in the card's order.
pub fn order(monitors: &[Monitor]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..monitors.len()).collect();
    order.sort_by_key(|&index| !monitors[index].internal);
    order
}

/// A monitor's scale, in 120ths: its density over the reference, to the nearest quarter, from 1
/// to 3. Unknown or implausible sizes (a projector's EDID gives none, some give an aspect
/// ratio in centimetres) are 1.
pub fn scale_120(monitor: &Monitor) -> u32 {
    let (width, height) = (f64::from(monitor.pixels.0), f64::from(monitor.pixels.1));
    let (width_mm, height_mm) = (f64::from(monitor.size_mm.0), f64::from(monitor.size_mm.1));
    if width_mm < 50.0 || height_mm < 30.0 {
        return SCALE_DENOMINATOR;
    }
    let ppi = width.hypot(height) / (width_mm.hypot(height_mm) / 25.4);
    if !(20.0..=600.0).contains(&ppi) {
        return SCALE_DENOMINATOR;
    }
    let reference = if monitor.internal {
        PANEL_PPI
    } else {
        MONITOR_PPI
    };
    let quarters = (ppi / reference * 4.0).round() as u32;
    (quarters * SCALE_DENOMINATOR / 4).clamp(SCALE_DENOMINATOR, MAX_SCALE_120)
}

/// The smallest logical size a scale choice leaves, as a Mac's smallest "looks like" choice of a
/// laptop's panel: what applications' windows and the shell's bars fit in.
pub const MIN_LOGICAL: (u32, u32) = (1024, 640);
/// Most scale choices offered for a display.
pub const MAX_CHOICES: usize = 5;

/// The scales a display may be shown at (in 120ths), in quarters from 1 to 3, from the most space
/// to the largest text: the one its density recommends ([`scale_120`]) and up to two each side of
/// it (more on one side when the other has fewer), each leaving at least [`MIN_LOGICAL`].
pub fn scale_choices(monitor: &Monitor) -> Vec<u32> {
    let recommended = scale_120(monitor);
    let fits = |scale: u32| {
        let (width, height) = logical(monitor.pixels, scale);
        width >= MIN_LOGICAL.0 && height >= MIN_LOGICAL.1
    };
    let all: Vec<u32> = (4..=12)
        .map(|quarters| quarters * SCALE_DENOMINATOR / 4)
        .filter(|&scale| scale <= MAX_SCALE_120 && (scale == recommended || fits(scale)))
        .collect();
    let at = all
        .iter()
        .position(|&scale| scale == recommended)
        .expect("the recommended scale is a choice");
    let (mut low, mut high) = (at, at);
    while high - low + 1 < MAX_CHOICES && (low > 0 || high + 1 < all.len()) {
        // Balanced around the recommended one while both sides have more.
        if high + 1 < all.len() && (high - at <= at - low || low == 0) {
            high += 1;
        } else {
            low -= 1;
        }
    }
    all[low..=high].to_vec()
}

/// An output's logical size: its pixels at its scale, rounded up so that it covers them.
pub fn logical((width, height): (u32, u32), scale_120: u32) -> (u32, u32) {
    let at = |pixels: u32| {
        (u64::from(pixels) * u64::from(SCALE_DENOMINATOR)).div_ceil(u64::from(scale_120)) as u32
    };
    (at(width).max(1), at(height).max(1))
}

/// Where outputs of these logical sizes go, in order: left to right from the origin,
/// top-aligned, each starting where the last ends.
pub fn arrange(sizes: &[(u32, u32)]) -> Vec<(i32, i32)> {
    let mut x = 0i32;
    sizes
        .iter()
        .map(|&(width, _)| {
            let at = (x, 0);
            x = x.saturating_add(i32::try_from(width).unwrap_or(i32::MAX));
            at
        })
        .collect()
}

/// The shortest stretch of edge two displays touching share (or the whole of the shorter's): the
/// pointer crosses where they meet, so they must meet over more than a corner.
pub const MIN_SHARED: u32 = 64;
/// How near an edge of the display touched a display dragged is drawn to it: aligned with its
/// top, bottom or centre.
pub const ALIGN: i64 = 48;

fn rect(at: (i32, i32), (width, height): (u32, u32)) -> Rect {
    Rect::new(at.0, at.1, width, height)
}

/// Whether two rectangles cover a common area (edges touching is not overlapping).
fn overlap(a: Rect, b: Rect) -> bool {
    i64::from(a.x) < b.right()
        && i64::from(b.x) < a.right()
        && i64::from(a.y) < b.bottom()
        && i64::from(b.y) < a.bottom()
}

/// Whether two rectangles touch along an edge, sharing a stretch of it.
fn touch(a: Rect, b: Rect) -> bool {
    let shared = |a0: i64, a1: i64, b0: i64, b1: i64| a1.min(b1) - a0.max(b0) > 0;
    let beside = (a.right() == i64::from(b.x) || b.right() == i64::from(a.x))
        && shared(i64::from(a.y), a.bottom(), i64::from(b.y), b.bottom());
    let stacked = (a.bottom() == i64::from(b.y) || b.bottom() == i64::from(a.y))
        && shared(i64::from(a.x), a.right(), i64::from(b.x), b.right());
    beside || stacked
}

fn clamp_coordinate(value: i64) -> i32 {
    value.clamp(i64::from(i32::MIN / 2), i64::from(i32::MAX / 2)) as i32
}

/// Where a display of `size` wanted at `wanted` goes, among `others` (the rest, where they are):
/// the nearest place to it beside one of them, touching it along at least [`MIN_SHARED`] of an
/// edge (all of the shorter one's), over none; aligned with that one's top, bottom or centre (left,
/// right or centre) when within [`ALIGN`] of it. Alone, it stays where it is wanted.
pub fn snap(others: &[Rect], size: (u32, u32), wanted: (i32, i32)) -> (i32, i32) {
    let (width, height) = (i64::from(size.0), i64::from(size.1));
    let (wx, wy) = (i64::from(wanted.0), i64::from(wanted.1));
    let mut best: Option<(i64, (i32, i32))> = None;
    // Along an edge from `start` to `end`, the display `length` long: where it may be, nearest
    // `want`, aligned when near.
    let along = |want: i64, start: i64, end: i64, length: i64| {
        let shared = i64::from(MIN_SHARED).min(length).min(end - start);
        let mut at = want.clamp(start - length + shared, end - shared);
        for aligned in [start, end - length, start + (end - start - length) / 2] {
            if (at - aligned).abs() <= ALIGN {
                at = aligned;
                break;
            }
        }
        at
    };
    for other in others {
        let (x0, y0, x1, y1) = (
            i64::from(other.x),
            i64::from(other.y),
            other.right(),
            other.bottom(),
        );
        let candidates = [
            (x1, along(wy, y0, y1, height)),
            (x0 - width, along(wy, y0, y1, height)),
            (along(wx, x0, x1, width), y1),
            (along(wx, x0, x1, width), y0 - height),
        ];
        for (x, y) in candidates {
            let at = (clamp_coordinate(x), clamp_coordinate(y));
            let placed = rect(at, size);
            if others.iter().any(|other| overlap(*other, placed)) {
                continue;
            }
            let distance = (x - wx).pow(2) + (y - wy).pow(2);
            if best.is_none_or(|(nearest, _)| distance < nearest) {
                best = Some((distance, at));
            }
        }
    }
    best.map_or(wanted, |(_, at)| at)
}

/// Where outputs go once their sizes change from `before` (their rectangles, the primary first) to
/// `sizes`: each keeps its place against the one it touched, as its edge aligned (top, bottom or
/// centre) or its offset along it, from the primary outwards, the primary where it was; one that
/// would then cover another, or touched none, is placed beside the others nearest where it was
/// ([`snap`]). Not normalized.
pub fn reflow(before: &[Rect], sizes: &[(u32, u32)]) -> Vec<(i32, i32)> {
    debug_assert_eq!(before.len(), sizes.len());
    let count = before.len();
    if count == 0 {
        return Vec::new();
    }
    let mut placed: Vec<Option<Rect>> = vec![None; count];
    placed[0] = Some(rect((before[0].x, before[0].y), sizes[0]));
    // From the primary outwards: each display touching one already placed keeps its place
    // against that one.
    let mut order = vec![0];
    while order.len() < count {
        let next = (0..count).filter(|j| placed[*j].is_none()).find_map(|j| {
            order
                .iter()
                .find(|&&i| touch(before[i], before[j]))
                .map(|&i| (i, j))
        });
        let Some((i, j)) = next else {
            break;
        };
        let at = against(before[i], before[j], placed[i].expect("placed"), sizes[j]);
        placed[j] = Some(rect(at, sizes[j]));
        order.push(j);
    }
    // What touched nothing goes last, in its order.
    let rest: Vec<usize> = (0..count).filter(|j| !order.contains(j)).collect();
    order.extend(rest);
    let mut done: Vec<Rect> = Vec::new();
    let mut origins = vec![(0, 0); count];
    for j in order {
        let kept = placed[j].map(|r| (r.x, r.y));
        let wanted = kept.unwrap_or((before[j].x, before[j].y));
        let clear = done.iter().all(|d| !overlap(*d, rect(wanted, sizes[j])));
        let at = if done.is_empty() || (kept.is_some() && clear) {
            wanted
        } else {
            snap(&done, sizes[j], wanted)
        };
        origins[j] = at;
        done.push(rect(at, sizes[j]));
    }
    origins
}

/// Where `moved` (of `size` now) goes against `anchor` (now `anchor_now`), as it was against it
/// before: on the same side, its edge aligned as it was (start, end or centre), else at the same
/// offset along it.
fn against(anchor: Rect, moved: Rect, anchor_now: Rect, size: (u32, u32)) -> (i32, i32) {
    let (width, height) = (i64::from(size.0), i64::from(size.1));
    // Along one axis, `a` the anchor's span before, `m` the moved one's, `n` the anchor's now.
    let keep = |a: (i64, i64), m: (i64, i64), n: (i64, i64), length: i64| {
        if m.0 == a.0 {
            n.0
        } else if m.1 == a.1 {
            n.1 - length
        } else if m.0 + m.1 == a.0 + a.1 {
            (n.0 + n.1 - length) / 2
        } else {
            n.0 + (m.0 - a.0)
        }
    };
    let (a, m, n) = (anchor, moved, anchor_now);
    let rows = |r: Rect| (i64::from(r.y), r.bottom());
    let columns = |r: Rect| (i64::from(r.x), r.right());
    let (x, y) = if i64::from(m.x) == a.right() {
        (n.right(), keep(rows(a), rows(m), rows(n), height))
    } else if m.right() == i64::from(a.x) {
        (
            i64::from(n.x) - width,
            keep(rows(a), rows(m), rows(n), height),
        )
    } else if i64::from(m.y) == a.bottom() {
        (keep(columns(a), columns(m), columns(n), width), n.bottom())
    } else {
        (
            keep(columns(a), columns(m), columns(n), width),
            i64::from(n.y) - height,
        )
    };
    (clamp_coordinate(x), clamp_coordinate(y))
}

/// The same origins moved together so that the arrangement's top-left corner is the global
/// space's origin.
pub fn normalize(origins: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let left = origins.iter().map(|(x, _)| *x).min().unwrap_or(0);
    let top = origins.iter().map(|(_, y)| *y).min().unwrap_or(0);
    origins
        .iter()
        .map(|(x, y)| (x.saturating_sub(left), y.saturating_sub(top)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(internal: bool, pixels: (u32, u32), size_mm: (u32, u32)) -> Monitor {
        Monitor {
            internal,
            pixels,
            size_mm,
        }
    }

    #[test]
    fn a_scale_is_the_density_over_the_reference_to_a_quarter() {
        // The ThinkPad P14s Gen 3's 14-inch panel, 1920x1200 (about 162 ppi): 1.25.
        assert_eq!(scale_120(&monitor(true, (1920, 1200), (301, 188))), 150);
        // A 13-inch 2560x1600 panel (about 227 ppi): 1.75.
        assert_eq!(scale_120(&monitor(true, (2560, 1600), (286, 179))), 210);
        // A 27-inch 4K monitor (about 163 ppi): 1.5; a 27-inch 1440p one (109 ppi): 1.
        assert_eq!(scale_120(&monitor(false, (3840, 2160), (597, 336))), 180);
        assert_eq!(scale_120(&monitor(false, (2560, 1440), (597, 336))), 120);
        // A 24-inch 1080p monitor is less dense than the reference: never below 1.
        assert_eq!(scale_120(&monitor(false, (1920, 1080), (531, 299))), 120);
        // A 15.6-inch 4K panel (about 283 ppi): 2; a 13-inch 5120x3200 one (455 ppi) is 3,
        // nothing beyond.
        assert_eq!(scale_120(&monitor(true, (3840, 2160), (344, 194))), 240);
        assert_eq!(scale_120(&monitor(true, (5120, 3200), (286, 179))), 360);
    }

    #[test]
    fn an_unknown_or_implausible_size_is_scale_one() {
        assert_eq!(scale_120(&monitor(false, (1920, 1080), (0, 0))), 120);
        // An aspect ratio given as a size (16 by 9).
        assert_eq!(scale_120(&monitor(false, (3840, 2160), (16, 9))), 120);
        // A 1080p projector reporting a wall-sized 3 m image.
        assert_eq!(scale_120(&monitor(false, (1920, 1080), (3000, 1690))), 120);
    }

    #[test]
    fn the_panel_comes_first_then_the_cards_order() {
        let monitors = [
            monitor(false, (2560, 1440), (597, 336)),
            monitor(true, (1920, 1200), (301, 188)),
            monitor(false, (1920, 1080), (531, 299)),
        ];
        assert_eq!(order(&monitors), [1, 0, 2]);
        assert_eq!(order(&[]), Vec::<usize>::new());
    }

    #[test]
    fn outputs_go_left_to_right_top_aligned() {
        // The panel at 1.25 (1536x960 logical), a 4K monitor at 1.5 (2560x1440), a 1080p one.
        let sizes = [
            logical((1920, 1200), 150),
            logical((3840, 2160), 180),
            logical((1920, 1080), 120),
        ];
        assert_eq!(sizes, [(1536, 960), (2560, 1440), (1920, 1080)]);
        assert_eq!(arrange(&sizes), [(0, 0), (1536, 0), (4096, 0)]);
        // A size that does not divide is rounded up: 2560 pixels at 1.75 are 1463 logical.
        assert_eq!(logical((2560, 1600), 210), (1463, 915));
    }

    #[test]
    fn a_display_offers_the_scales_around_its_density() {
        // The ThinkPad's panel (1.25): 1, then 1.5 and 1.75; 2 would leave 960x600.
        assert_eq!(
            scale_choices(&monitor(true, (1920, 1200), (301, 188))),
            [120, 150, 180, 210]
        );
        // A 27-inch 4K monitor (1.5): two each side, 1 to 2 (1920x1080 looks).
        assert_eq!(
            scale_choices(&monitor(false, (3840, 2160), (597, 336))),
            [120, 150, 180, 210, 240]
        );
        // A 24-inch 1080p monitor (1): more space is not to be had, larger text up to 1.5.
        assert_eq!(
            scale_choices(&monitor(false, (1920, 1080), (531, 299))),
            [120, 150, 180]
        );
        // A 13-inch 5120x3200 panel (3, the most): the four below it.
        assert_eq!(
            scale_choices(&monitor(true, (5120, 3200), (286, 179))),
            [240, 270, 300, 330, 360]
        );
        // A small display: its own scale, nothing else.
        assert_eq!(scale_choices(&monitor(false, (1280, 720), (0, 0))), [120]);
    }

    #[test]
    fn a_display_dragged_goes_beside_the_others_touching_one() {
        let panel = Rect::new(0, 0, 1536, 960);
        // Dropped above the panel, a little off: on its top edge, where it was dropped.
        assert_eq!(snap(&[panel], (2560, 1440), (-300, -1300)), (-300, -1440));
        // Near the panel's centre line: centred on it.
        assert_eq!(snap(&[panel], (2560, 1440), (-480, -1400)), (-512, -1440));
        // Dropped over the panel, nearer its right: beside it, top-aligned within reach.
        assert_eq!(snap(&[panel], (2560, 1440), (900, 30)), (1536, 0));
        // Far below and right: below it, still over MIN_SHARED of its bottom edge.
        assert_eq!(snap(&[panel], (1920, 1080), (5000, 5000)), (1536 - 64, 960));
        // Two displays already: never over either.
        let monitor = Rect::new(1536, 0, 2560, 1440);
        let at = snap(&[panel, monitor], (1920, 1080), (1000, 500));
        let placed = Rect::new(at.0, at.1, 1920, 1080);
        assert!(!overlap(placed, panel) && !overlap(placed, monitor));
        assert!(touch(placed, panel) || touch(placed, monitor));
        // Alone, a display is where it is.
        assert_eq!(snap(&[], (1920, 1080), (10, 20)), (10, 20));
    }

    #[test]
    fn displays_keep_their_places_against_one_that_changes_size() {
        // A monitor right of the panel, bottoms aligned; one more below the monitor, centred.
        let before = [
            Rect::new(0, 480, 1536, 960),
            Rect::new(1536, 0, 2560, 1440),
            Rect::new(1856, 1440, 1920, 1080),
        ];
        // The panel at 1 (1920x1200): the monitor moves right with its edge, bottoms still
        // aligned; the one below it follows the monitor.
        let after = reflow(&before, &[(1920, 1200), (2560, 1440), (1920, 1080)]);
        assert_eq!(after, [(0, 480), (1920, 240), (2240, 1680)]);
        // The monitor at 2 (1920x1080): bottoms aligned with the panel still, the third centred
        // below it.
        let after = reflow(&before, &[(1536, 960), (1920, 1080), (1920, 1080)]);
        assert_eq!(after, [(0, 480), (1536, 360), (1536, 1440)]);
        // Unplugging the one between: the last touched nothing kept, and goes beside the panel.
        let before = [Rect::new(0, 0, 1536, 960), Rect::new(3456, 0, 2560, 1440)];
        assert_eq!(
            reflow(&before, &[(1536, 960), (2560, 1440)]),
            [(0, 0), (1536, 0)]
        );
        assert_eq!(normalize(&[(0, 480), (1536, 0)]), [(0, 480), (1536, 0)]);
        assert_eq!(normalize(&[(0, 0), (-2560, -480)]), [(2560, 480), (0, 0)]);
    }
}
