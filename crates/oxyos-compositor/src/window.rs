//! What the compositor draws of a window (docs/adr/0016-windows.mdx): its corners, cut from its
//! pixels, and its shadow beneath it. Every toplevel and dialog has them, unless it is full-screen
//! (`Scene::window_look`); the active window's shadow is wider and darker; a window zoomed to fill
//! the work area has square corners, so nothing of the desktop shows at them.
//!
//! A window in a transition (docs/adr/0011-transitions.mdx) is drawn scaled and moved: its corners
//! and its shadow are scaled with it, and fade with it.

use oxyos_compositor_core::WindowLook;
use oxyos_gpu::{Rect as Area, Shadow, ShadowProfile};
use oxyos_protocol::Rect;
pub use oxyos_protocol::WINDOW_RADIUS;
use std::rc::Rc;

/// A shadow's look, as CSS's `box-shadow` puts one: the shape casting it is the window moved by
/// `offset` and grown by `spread` on every side, blurred by `blur` (a Gaussian of σ = blur/2), at
/// `alpha` (0 to 255) black. In logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowLook {
    pub offset: (i32, i32),
    pub spread: i32,
    pub blur: u32,
    pub alpha: u8,
}

/// The active window's shadow, a Mac's: deep and soft, falling well below it, 35 % black where
/// it is darkest.
pub const ACTIVE_SHADOW: ShadowLook = ShadowLook {
    offset: (0, 18),
    spread: 0,
    blur: 48,
    alpha: 89,
};

/// Every other window's: closer and lighter, 20 % black.
pub const INACTIVE_SHADOW: ShadowLook = ShadowLook {
    offset: (0, 8),
    spread: 0,
    blur: 24,
    alpha: 51,
};

/// The shadows' profiles, computed once, with the identity the GPU keeps each by
/// (`oxyos_gpu::Source`): the last two keys (a surface's is client << 32 | id), and versions of
/// the compositor's numbering, which no surface's frame shares; so a surface with one of those
/// keys could only make the device upload a profile again, never draw it for the surface.
pub struct Shadows {
    pub active: (Rc<ShadowProfile>, oxyos_gpu::Source),
    pub inactive: (Rc<ShadowProfile>, oxyos_gpu::Source),
}

impl Shadows {
    /// The profiles, numbered `versions` and the next.
    pub fn new(versions: u64) -> Self {
        let profile = |look: &ShadowLook, key: u64, version: u64| {
            (
                Rc::new(ShadowProfile::new(look.blur, look.alpha)),
                oxyos_gpu::Source { key, version },
            )
        };
        Self {
            active: profile(&ACTIVE_SHADOW, u64::MAX, versions),
            inactive: profile(&INACTIVE_SHADOW, u64::MAX - 1, versions + 1),
        }
    }

    pub fn of(&self, look: WindowLook) -> &(Rc<ShadowProfile>, oxyos_gpu::Source) {
        if look.active {
            &self.active
        } else {
            &self.inactive
        }
    }
}

pub fn shadow_look(look: WindowLook) -> &'static ShadowLook {
    if look.active {
        &ACTIVE_SHADOW
    } else {
        &INACTIVE_SHADOW
    }
}

/// The radius of a window's corners as it is drawn at its own size, in logical pixels: square
/// when zoomed to fill the work area. A zoom's transition goes from one to the other.
pub fn corner(look: WindowLook) -> u32 {
    if look.maximized {
        0
    } else {
        WINDOW_RADIUS
    }
}

/// How a window at `rect` is drawn where its transition places it (`placed`): its corners'
/// radius, and its shadow's layer (its rectangle on the output, and where the shadow is in it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drawn {
    pub radius: u32,
    pub shadow_rect: Rect,
    pub shadow: Shadow,
}

/// The size a window is drawn at, in 256ths: its transition's scale (the same both ways).
fn scale_256(rect: Rect, placed: Rect) -> u32 {
    if rect.width == 0 {
        return 256;
    }
    ((u64::from(placed.width) * 256 + u64::from(rect.width) / 2) / u64::from(rect.width))
        .clamp(1, u64::from(oxyos_gpu::MAX_SHADOW_SCALE)) as u32
}

/// `rect` grown by `by` on every side (shrunk, negative), never below nothing.
fn grown(rect: Rect, by: i32) -> Rect {
    Rect::new(
        rect.x - by,
        rect.y - by,
        (i64::from(rect.width) + 2 * i64::from(by)).max(0) as u32,
        (i64::from(rect.height) + 2 * i64::from(by)).max(0) as u32,
    )
}

/// How the window at `rect`, placed at `placed`, is drawn with `look` and corners of `corner`
/// (logical pixels at its own size), its shadow of `profile`.
pub fn drawn(
    look: WindowLook,
    corner: u32,
    rect: Rect,
    placed: Rect,
    profile: &ShadowProfile,
) -> Drawn {
    let scale = scale_256(rect, placed);
    let scaled = |length: i64| ((length * i64::from(scale) + 128).div_euclid(256)) as i32;
    let shadow = shadow_look(look);
    let radius = scaled(i64::from(corner)) as u32;
    let spread = scaled(i64::from(shadow.spread));
    let shape = grown(
        Rect::new(
            placed.x + scaled(i64::from(shadow.offset.0)),
            placed.y + scaled(i64::from(shadow.offset.1)),
            placed.width,
            placed.height,
        ),
        spread,
    );
    // The profile's reach at this size, rounded up: all of the shadow is within its layer.
    let reach = (profile.reach() * scale).div_ceil(256) as i32;
    let shadow_rect = grown(shape, reach);
    let within = |inner: Rect| Area {
        x: inner.x - shadow_rect.x,
        y: inner.y - shadow_rect.y,
        width: inner.width,
        height: inner.height,
    };
    Drawn {
        radius,
        shadow_rect,
        shadow: Shadow {
            shape: within(shape),
            radius: (radius as i32 + spread).max(0) as u32,
            clip: within(placed),
            clip_radius: radius,
            scale,
        },
    }
}

/// All a window drawn at `placed` changes on the output with `look`: its rectangle and its
/// shadow's (ADR 0005's damage of a window that came, went, moved or changed its look).
pub fn extent(look: WindowLook, rect: Rect, placed: Rect, shadows: &Shadows) -> Area {
    let drawn = drawn(look, corner(look), rect, placed, &shadows.of(look).0);
    let area = |r: Rect| Area {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    };
    area(placed).bounding(&area(drawn.shadow_rect))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVE: WindowLook = WindowLook {
        active: true,
        maximized: false,
    };
    const INACTIVE: WindowLook = WindowLook {
        active: false,
        maximized: false,
    };

    #[test]
    fn a_window_casts_its_shadow_below_it_reaching_three_sigma() {
        let shadows = Shadows::new(0);
        let rect = Rect::new(100, 100, 400, 300);
        let drawn = drawn(ACTIVE, corner(ACTIVE), rect, rect, &shadows.active.0);
        assert_eq!(drawn.radius, 16);
        // Blur 48 (σ 24) reaches 72 px from the shape, the window 18 px lower.
        assert_eq!(drawn.shadow_rect, Rect::new(28, 46, 544, 444));
        assert_eq!(
            drawn.shadow,
            Shadow {
                shape: Area {
                    x: 72,
                    y: 72,
                    width: 400,
                    height: 300
                },
                radius: 16,
                clip: Area {
                    x: 72,
                    y: 54,
                    width: 400,
                    height: 300
                },
                clip_radius: 16,
                scale: 256,
            }
        );
        // The inactive one, blur 24 and 8 px lower: 36 px.
        let inactive = super::drawn(INACTIVE, corner(INACTIVE), rect, rect, &shadows.inactive.0);
        assert_eq!(inactive.shadow_rect, Rect::new(64, 72, 472, 372));
        // What either changes on the output: the window and all of its shadow.
        assert_eq!(
            extent(ACTIVE, rect, rect, &shadows),
            Area {
                x: 28,
                y: 46,
                width: 544,
                height: 444
            }
        );
    }

    #[test]
    fn a_zoomed_window_has_square_corners_and_keeps_its_shadow() {
        let shadows = Shadows::new(0);
        let zoomed = WindowLook {
            active: true,
            maximized: true,
        };
        assert_eq!(corner(zoomed), 0);
        let rect = Rect::new(0, 28, 1280, 636);
        let drawn = drawn(zoomed, corner(zoomed), rect, rect, &shadows.active.0);
        assert_eq!(
            (drawn.radius, drawn.shadow.radius, drawn.shadow.clip_radius),
            (0, 0, 0)
        );
        // Its shadow is the same as a round window's of its size, square: it still falls below.
        let round = super::drawn(ACTIVE, corner(ACTIVE), rect, rect, &shadows.active.0);
        assert_eq!(drawn.shadow_rect, round.shadow_rect);
        // Halfway through its zoom's corners (8), drawn at half its size: 4.
        let half = Rect::new(320, 187, 640, 318);
        assert_eq!(
            super::drawn(ACTIVE, 8, rect, half, &shadows.active.0).radius,
            4
        );
    }

    #[test]
    fn a_window_drawn_smaller_has_smaller_corners_and_shadow() {
        let shadows = Shadows::new(0);
        let rect = Rect::new(100, 100, 400, 300);
        // At a tenth (going into the dock): 26/256 of the size.
        let placed = Rect::new(280, 400, 40, 30);
        let drawn = drawn(ACTIVE, corner(ACTIVE), rect, placed, &shadows.active.0);
        assert_eq!(drawn.radius, 2);
        assert_eq!(drawn.shadow.scale, 26);
        // 18 px lower at a tenth is 2; 72 px of reach is 8.
        assert_eq!(drawn.shadow_rect, Rect::new(272, 394, 56, 46));
        assert_eq!(drawn.shadow.clip.y, 6);
    }
}
