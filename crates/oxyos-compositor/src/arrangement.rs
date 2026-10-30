//! Where the outputs are and how dense (docs/adr/0018-several-displays.mdx): side by side, left
//! to right, top-aligned, with no gap, the built-in panel first (the primary), then the
//! connectors in the card's order; each at its preferred mode, at a scale from its density.

use oxyos_compositor_core::scene::SCALE_DENOMINATOR;

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
}
