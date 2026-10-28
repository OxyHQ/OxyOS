//! Popup placement: anchor/gravity/offset relative to the parent, then per-axis constraint
//! adjustment against the usable area of one output — flip first, then slide, then resize.
//! Arithmetic is done in i64 so no peer-provided geometry can overflow.

use oxyos_protocol::{Adjust, Anchor, Gravity, Positioner, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Start,
    Center,
    End,
}

fn sides_of_anchor(anchor: Anchor) -> (Side, Side) {
    use Side::*;
    match anchor {
        Anchor::Center => (Center, Center),
        Anchor::Top => (Center, Start),
        Anchor::Bottom => (Center, End),
        Anchor::Left => (Start, Center),
        Anchor::Right => (End, Center),
        Anchor::TopLeft => (Start, Start),
        Anchor::BottomLeft => (Start, End),
        Anchor::TopRight => (End, Start),
        Anchor::BottomRight => (End, End),
    }
}

fn sides_of_gravity(gravity: Gravity) -> (Side, Side) {
    use Side::*;
    match gravity {
        Gravity::Center => (Center, Center),
        Gravity::Top => (Center, Start),
        Gravity::Bottom => (Center, End),
        Gravity::Left => (Start, Center),
        Gravity::Right => (End, Center),
        Gravity::TopLeft => (Start, Start),
        Gravity::BottomLeft => (Start, End),
        Gravity::TopRight => (End, Start),
        Gravity::BottomRight => (End, End),
    }
}

fn invert(side: Side) -> Side {
    match side {
        Side::Start => Side::End,
        Side::Center => Side::Center,
        Side::End => Side::Start,
    }
}

/// One axis of a positioner: anchor span, anchor side, gravity side, popup length and offset.
#[derive(Clone, Copy, Debug)]
struct Axis {
    anchor_start: i64,
    anchor_length: i64,
    anchor: Side,
    gravity: Side,
    length: i64,
    offset: i64,
}

impl Axis {
    fn start(self) -> i64 {
        let point = match self.anchor {
            Side::Start => self.anchor_start,
            Side::Center => self.anchor_start + self.anchor_length / 2,
            Side::End => self.anchor_start + self.anchor_length,
        };
        let start = match self.gravity {
            Side::Start => point - self.length,
            Side::Center => point - self.length / 2,
            Side::End => point,
        };
        start + self.offset
    }

    fn flipped(self) -> Axis {
        Axis {
            anchor: invert(self.anchor),
            gravity: invert(self.gravity),
            offset: -self.offset,
            ..self
        }
    }
}

fn fits(start: i64, length: i64, area_start: i64, area_length: i64) -> bool {
    start >= area_start && start + length <= area_start + area_length
}

/// Returns (start, length) after constraint adjustment, or None if resizing left nothing.
fn constrain(
    axis: Axis,
    origin: i64,
    area_start: i64,
    area_length: i64,
    flip: bool,
    slide: bool,
    resize: bool,
) -> Option<(i64, i64)> {
    let mut start = origin + axis.start();
    let mut length = axis.length;
    if fits(start, length, area_start, area_length) {
        return Some((start, length));
    }
    if flip {
        let flipped = origin + axis.flipped().start();
        if fits(flipped, length, area_start, area_length) {
            return Some((flipped, length));
        }
    }
    if slide {
        start = if length > area_length {
            area_start
        } else {
            start.clamp(area_start, area_start + area_length - length)
        };
    }
    if resize && !fits(start, length, area_start, area_length) {
        let end = (start + length).min(area_start + area_length);
        start = start.max(area_start);
        length = end - start;
        if length <= 0 {
            return None;
        }
    }
    Some((start, length))
}

/// Mirrors horizontal preferences for right-to-left layouts.
fn mirror(side: Side, rtl: bool) -> Side {
    if rtl {
        invert(side)
    } else {
        side
    }
}

/// Places a popup in global logical coordinates. `parent_origin` is the parent's global origin
/// and `area` the usable area of the output the anchor point is on. Without adjustment flags a
/// popup that does not fit keeps its requested geometry and is clipped by the output.
pub fn place(positioner: &Positioner, parent_origin: (i32, i32), area: Rect) -> Option<Rect> {
    let rtl = positioner.adjust.has(Adjust::RTL);
    let (anchor_x, anchor_y) = sides_of_anchor(positioner.anchor);
    let (gravity_x, gravity_y) = sides_of_gravity(positioner.gravity);
    let rect = positioner.anchor_rect;
    let horizontal = Axis {
        anchor_start: i64::from(rect.x),
        anchor_length: i64::from(rect.width),
        anchor: mirror(anchor_x, rtl),
        gravity: mirror(gravity_x, rtl),
        length: i64::from(positioner.size.width),
        offset: if rtl {
            -i64::from(positioner.offset_x)
        } else {
            i64::from(positioner.offset_x)
        },
    };
    let vertical = Axis {
        anchor_start: i64::from(rect.y),
        anchor_length: i64::from(rect.height),
        anchor: anchor_y,
        gravity: gravity_y,
        length: i64::from(positioner.size.height),
        offset: i64::from(positioner.offset_y),
    };
    let adjust = positioner.adjust;
    let (x, width) = constrain(
        horizontal,
        i64::from(parent_origin.0),
        i64::from(area.x),
        i64::from(area.width),
        adjust.has(Adjust::FLIP_X),
        adjust.has(Adjust::SLIDE_X),
        adjust.has(Adjust::RESIZE_X),
    )?;
    let (y, height) = constrain(
        vertical,
        i64::from(parent_origin.1),
        i64::from(area.y),
        i64::from(area.height),
        adjust.has(Adjust::FLIP_Y),
        adjust.has(Adjust::SLIDE_Y),
        adjust.has(Adjust::RESIZE_Y),
    )?;
    Some(Rect {
        x: i32::try_from(x).ok()?,
        y: i32::try_from(y).ok()?,
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
    })
}

/// The anchor point in global coordinates, used to choose the constraining output.
pub fn anchor_point(positioner: &Positioner, parent_origin: (i32, i32)) -> (i64, i64) {
    let rect = positioner.anchor_rect;
    (
        i64::from(parent_origin.0) + i64::from(rect.x) + i64::from(rect.width) / 2,
        i64::from(parent_origin.1) + i64::from(rect.y) + i64::from(rect.height) / 2,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_protocol::Size;

    const OUTPUT: Rect = Rect::new(0, 0, 1920, 1080);

    fn menu(anchor_rect: Rect, size: Size, anchor: Anchor, gravity: Gravity) -> Positioner {
        Positioner {
            anchor_rect,
            size,
            anchor,
            gravity,
            adjust: Adjust::NONE,
            offset_x: 0,
            offset_y: 0,
        }
    }

    fn ltr(mut positioner: Positioner) -> Positioner {
        positioner.adjust = Adjust::SLIDE_X
            .union(Adjust::SLIDE_Y)
            .union(Adjust::FLIP_X)
            .union(Adjust::FLIP_Y)
            .union(Adjust::RESIZE_X)
            .union(Adjust::RESIZE_Y);
        positioner
    }

    #[test]
    fn popup_extends_beyond_each_edge_of_a_small_parent() {
        // Parent window 200x100 at (800, 500); the menu is larger than the whole parent.
        let parent = (800, 500);
        let size = Size::new(300, 400);
        let below = ltr(menu(
            Rect::new(10, 90, 20, 10),
            size,
            Anchor::BottomLeft,
            Gravity::BottomRight,
        ));
        let placed = place(&below, parent, OUTPUT).unwrap();
        assert_eq!(placed, Rect::new(810, 600, 300, 400));
        assert!(
            placed.bottom() > 600 && placed.right() > 1000,
            "beyond bottom and right"
        );

        let above = ltr(menu(
            Rect::new(180, 0, 20, 10),
            size,
            Anchor::TopRight,
            Gravity::TopLeft,
        ));
        let placed = place(&above, parent, OUTPUT).unwrap();
        assert_eq!(placed, Rect::new(700, 100, 300, 400));
        assert!(placed.y < 500 && placed.x < 800, "beyond top and left");
    }

    #[test]
    fn flips_at_the_bottom_and_right_edges() {
        let parent = (1800, 1000);
        let positioner = ltr(menu(
            Rect::new(0, 40, 100, 30),
            Size::new(240, 300),
            Anchor::BottomLeft,
            Gravity::BottomRight,
        ));
        let placed = place(&positioner, parent, OUTPUT).unwrap();
        // Ends at the trigger's right edge and sits above it.
        assert_eq!(placed, Rect::new(1660, 740, 240, 300));
        assert!(OUTPUT.contains(placed));
    }

    #[test]
    fn slides_when_neither_side_fits() {
        let positioner = ltr(menu(
            Rect::new(0, 0, 10, 10),
            Size::new(1500, 100),
            Anchor::BottomLeft,
            Gravity::BottomRight,
        ));
        let placed = place(&positioner, (900, 100), OUTPUT).unwrap();
        assert_eq!(placed, Rect::new(420, 110, 1500, 100));
    }

    #[test]
    fn corner_placement_stays_inside_the_output() {
        for parent in [(0, 0), (1900, 0), (0, 1060), (1900, 1060)] {
            let positioner = ltr(menu(
                Rect::new(0, 0, 20, 20),
                Size::new(400, 500),
                Anchor::BottomRight,
                Gravity::BottomRight,
            ));
            let placed = place(&positioner, parent, OUTPUT).unwrap();
            assert!(OUTPUT.contains(placed), "{parent:?} -> {placed:?}");
            assert_eq!((placed.width, placed.height), (400, 500));
        }
    }

    #[test]
    fn resizes_only_when_the_menu_is_taller_than_the_usable_area() {
        let area = Rect::new(0, 32, 1920, 1000);
        let positioner = ltr(menu(
            Rect::new(0, 0, 50, 20),
            Size::new(200, 1500),
            Anchor::BottomLeft,
            Gravity::BottomRight,
        ));
        let placed = place(&positioner, (100, 400), area).unwrap();
        assert_eq!(placed, Rect::new(100, 32, 200, 1000));
    }

    #[test]
    fn without_adjustment_the_requested_geometry_is_kept() {
        let mut positioner = menu(
            Rect::new(0, 0, 10, 10),
            Size::new(300, 300),
            Anchor::BottomLeft,
            Gravity::BottomRight,
        );
        positioner.adjust = Adjust::NONE;
        let placed = place(&positioner, (1800, 1000), OUTPUT).unwrap();
        assert_eq!(placed, Rect::new(1800, 1010, 300, 300));
    }

    #[test]
    fn rtl_mirrors_horizontal_preferences_and_offsets() {
        let mut positioner = ltr(menu(
            Rect::new(100, 0, 40, 20),
            Size::new(200, 100),
            Anchor::BottomLeft,
            Gravity::BottomRight,
        ));
        positioner.offset_x = 8;
        assert_eq!(
            place(&positioner, (500, 500), OUTPUT).unwrap(),
            Rect::new(608, 520, 200, 100)
        );
        positioner.adjust = positioner.adjust.union(Adjust::RTL);
        // Anchored at the right edge of the trigger, extending leftwards.
        assert_eq!(
            place(&positioner, (500, 500), OUTPUT).unwrap(),
            Rect::new(432, 520, 200, 100)
        );
    }

    #[test]
    fn extreme_geometry_does_not_overflow() {
        let positioner = ltr(menu(
            Rect::new(
                oxyos_protocol::MAX_COORDINATE,
                oxyos_protocol::MAX_COORDINATE,
                1,
                1,
            ),
            Size::new(oxyos_protocol::MAX_DIMENSION, oxyos_protocol::MAX_DIMENSION),
            Anchor::BottomRight,
            Gravity::BottomRight,
        ));
        let placed = place(&positioner, (i32::MAX, i32::MAX), OUTPUT).unwrap();
        assert!(OUTPUT.contains(placed));
        assert_eq!(
            place(&positioner, (i32::MAX, i32::MAX), Rect::new(0, 0, 0, 0)),
            None
        );
    }
}
