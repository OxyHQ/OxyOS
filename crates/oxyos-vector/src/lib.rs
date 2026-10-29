//! Vector graphics for applications (react-native-svg on OxyOS): shapes with SVG's geometry,
//! fills and strokes, rasterized with anti-aliasing into a picture the Bloom host draws as it
//! draws an image. The host flattens an SVG element tree into shapes (transforms composed,
//! inherited paint resolved, opacity folded into the colours); this crate does not know SVG's
//! document model.

use kurbo::{BezPath, PathEl, Shape as _};
use tiny_skia::{
    FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, StrokeDash, Transform,
};

#[cfg(test)]
mod tests;

/// The largest picture side, in pixels, as the compositor's buffers.
pub const MAX_SIDE: u32 = 16_384;
/// How closely curves of SVG's basic shapes are approximated, in user units.
const TOLERANCE: f64 = 0.01;

/// A shape's outline, in its own user space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Geometry<'a> {
    /// SVG path data (`d`).
    Path(&'a str),
    /// A rectangle, its corners rounded by (`rx`, `ry`), each at most half its side.
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        rx: f32,
        ry: f32,
    },
    Ellipse {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
}

/// A colour as 0xAARRGGBB, not premultiplied (as React Native's processed colours).
pub type Argb = u32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fill {
    pub color: Argb,
    /// SVG's `evenodd` fill rule; `nonzero` otherwise.
    pub even_odd: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke<'a> {
    pub color: Argb,
    pub width: f32,
    pub cap: Cap,
    pub join: Join,
    pub miter_limit: f32,
    /// Dash and gap lengths; empty for a solid line.
    pub dash: &'a [f32],
    pub dash_offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape<'a> {
    pub geometry: Geometry<'a>,
    /// From user space to the picture's pixels: SVG's matrix (a, b, c, d, e, f), so that
    /// x' = a·x + c·y + e and y' = b·x + d·y + f.
    pub transform: [f32; 6],
    pub fill: Option<Fill>,
    pub stroke: Option<Stroke<'a>>,
}

/// A rasterized picture: premultiplied R, G, B, A bytes, rows top to bottom (as decoded images).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// Shapes left out: path data SVG would reject, or a transform that cannot be drawn.
    pub skipped: usize,
}

/// Draws `shapes` in order (later ones over earlier ones, each fill under its stroke) on a
/// transparent picture. `None` for a size of zero or beyond `MAX_SIDE`.
pub fn render(shapes: &[Shape<'_>], width: u32, height: u32) -> Option<Picture> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return None;
    }
    let mut pixmap = Pixmap::new(width, height)?;
    let mut skipped = 0;
    for shape in shapes {
        let [a, b, c, d, e, f] = shape.transform;
        let transform = Transform::from_row(a, b, c, d, e, f);
        let Some(path) = outline(&shape.geometry).filter(|_| transform.is_finite()) else {
            skipped += 1;
            continue;
        };
        if let Some(fill) = shape.fill {
            let rule = if fill.even_odd {
                FillRule::EvenOdd
            } else {
                FillRule::Winding
            };
            pixmap.fill_path(&path, &paint(fill.color), rule, transform, None);
        }
        if let Some(stroke) = shape.stroke.filter(|stroke| stroke.width > 0.0) {
            let style = tiny_skia::Stroke {
                width: stroke.width,
                miter_limit: stroke.miter_limit.max(1.0),
                line_cap: match stroke.cap {
                    Cap::Butt => LineCap::Butt,
                    Cap::Round => LineCap::Round,
                    Cap::Square => LineCap::Square,
                },
                line_join: match stroke.join {
                    Join::Miter => LineJoin::Miter,
                    Join::Round => LineJoin::Round,
                    Join::Bevel => LineJoin::Bevel,
                },
                // An odd list is repeated, as SVG says; a list of zeros (or an invalid one) is solid.
                dash: dash(stroke.dash, stroke.dash_offset),
            };
            pixmap.stroke_path(&path, &paint(stroke.color), &style, transform, None);
        }
    }
    Some(Picture {
        width,
        height,
        pixels: pixmap.take(),
        skipped,
    })
}

fn paint(color: Argb) -> Paint<'static> {
    let [a, r, g, b] = color.to_be_bytes();
    let mut paint = Paint::default();
    paint.set_color_rgba8(r, g, b, a);
    paint.anti_alias = true;
    paint
}

fn dash(lengths: &[f32], offset: f32) -> Option<StrokeDash> {
    if lengths.is_empty() || lengths.iter().all(|length| *length == 0.0) {
        return None;
    }
    let mut lengths = lengths.to_vec();
    if lengths.len() % 2 == 1 {
        lengths.extend_from_within(..);
    }
    StrokeDash::new(lengths, offset)
}

/// The outline of a geometry, if it is one SVG draws.
fn outline(geometry: &Geometry<'_>) -> Option<Path> {
    let path = match *geometry {
        Geometry::Path(data) => BezPath::from_svg(data).ok()?,
        Geometry::Rect {
            x,
            y,
            width,
            height,
            rx,
            ry,
        } => rounded_rect(
            x.into(),
            y.into(),
            width.into(),
            height.into(),
            rx.into(),
            ry.into(),
        )?,
        Geometry::Ellipse { cx, cy, rx, ry } => {
            if rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            kurbo::Ellipse::new(
                (f64::from(cx), f64::from(cy)),
                (f64::from(rx), f64::from(ry)),
                0.0,
            )
            .to_path(TOLERANCE)
        }
        Geometry::Line { x1, y1, x2, y2 } => {
            let mut path = BezPath::new();
            path.move_to((f64::from(x1), f64::from(y1)));
            path.line_to((f64::from(x2), f64::from(y2)));
            path
        }
    };
    let mut builder = PathBuilder::new();
    let point = |p: kurbo::Point| (p.x as f32, p.y as f32);
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) => {
                let (x, y) = point(p);
                builder.move_to(x, y);
            }
            PathEl::LineTo(p) => {
                let (x, y) = point(p);
                builder.line_to(x, y);
            }
            PathEl::QuadTo(c, p) => {
                let ((cx, cy), (x, y)) = (point(c), point(p));
                builder.quad_to(cx, cy, x, y);
            }
            PathEl::CurveTo(c1, c2, p) => {
                let ((ax, ay), (bx, by), (x, y)) = (point(c1), point(c2), point(p));
                builder.cubic_to(ax, ay, bx, by, x, y);
            }
            PathEl::ClosePath => builder.close(),
        }
    }
    builder.finish()
}

/// SVG's rectangle: corners rounded by `rx` and `ry` (each defaulting to the other, and at most
/// half the side), none drawn when either side is not positive.
fn rounded_rect(x: f64, y: f64, width: f64, height: f64, rx: f64, ry: f64) -> Option<BezPath> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let (rx, ry) = match (rx > 0.0, ry > 0.0) {
        (false, false) => (0.0, 0.0),
        (true, false) => (rx, rx),
        (false, true) => (ry, ry),
        (true, true) => (rx, ry),
    };
    let (rx, ry) = (rx.min(width / 2.0), ry.min(height / 2.0));
    let rect = kurbo::Rect::new(x, y, x + width, y + height);
    if rx == 0.0 || ry == 0.0 {
        return Some(rect.to_path(TOLERANCE));
    }
    // Four elliptical corners joined by the sides, clockwise from the top-left.
    let mut path = BezPath::new();
    path.move_to((x + rx, y));
    path.line_to((x + width - rx, y));
    corner(&mut path, (x + width - rx, y + ry), (rx, ry), -90.0);
    path.line_to((x + width, y + height - ry));
    corner(&mut path, (x + width - rx, y + height - ry), (rx, ry), 0.0);
    path.line_to((x + rx, y + height));
    corner(&mut path, (x + rx, y + height - ry), (rx, ry), 90.0);
    path.line_to((x, y + ry));
    corner(&mut path, (x + rx, y + ry), (rx, ry), 180.0);
    path.close_path();
    Some(path)
}

/// A quarter of the ellipse around `center`, from `start` degrees, clockwise on screen.
fn corner(path: &mut BezPath, center: (f64, f64), radii: (f64, f64), start: f64) {
    let arc = kurbo::Arc::new(center, radii, start.to_radians(), 90f64.to_radians(), 0.0);
    arc.to_cubic_beziers(TOLERANCE, |c1, c2, p| path.curve_to(c1, c2, p));
}
