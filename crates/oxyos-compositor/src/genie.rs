//! The genie (docs/adr/0011-transitions.mdx, step 7): the mesh a Mac's Dock works out for each frame
//! of a window minimized into its slot, ported from Raphael Salaja's line-by-line port of the Dock's
//! own function (<https://www.raphaelsalaja.com/work/genie>, November 2026), which he checked bit
//! for bit against the Dock's machine code in an emulator and to within 3 pixels of a 120 fps
//! recording of a real minimize.
//!
//! In order, as the page says: the move is two phases, each eased by half a cosine
//! (`0.5 − 0.5·cos(πt)`), the sides bending toward the slot and then the window sliding down
//! between them; each phase lasts `trunc(ln(d/2 + 1))·50` for its distance `d` (the sideways
//! distance for the bend, the travel for the slide); they are scheduled so the sides finish bending
//! as the window's leading edge reaches the slot; each side is a cosine from the window's edge to
//! the slot's, halved until each piece is within 0.6 points of straight (no piece under 8 points,
//! straightness checked only under 128, at most 64 splits); rows that passed into the slot are
//! clamped to its near edge, and a last row is added once the window has gone further than the gap
//! it started with. The mesh is two columns wide (two rows tall with the Dock at a side) and as many
//! rows as the sides needed.
//!
//! The thresholds are in points, so the mesh is worked out in the global space's logical pixels,
//! which are an OxyOS display's points whatever its scale, and each screen draws it at its own
//! (`Screen::layer`). Values are rounded to 32-bit floats wherever the Dock stores one
//! (`fround`), and the comparisons that choose between two values are kept as written, as the
//! page's port keeps them: they differ from `min` and `max` for NaN and −0.

use std::f64::consts::PI;

/// Which edge of the screen the slot is on: the Dock's. OxyOS's dock is at the bottom (ADR 0009);
/// the other three are the Dock's function's own, kept whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Bottom,
    Top,
    Left,
    Right,
}

/// A rectangle in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// The mesh for one frame: `columns` × `rows` vertices, row by row, each where it is on the window
/// (`[0]`, `[1]`, points from its top-left corner) and where it is drawn on the screen (`[2]`,
/// `[3]`). With the Dock at the bottom or the top, two columns (each row's left and right ends);
/// at a side, two rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub columns: usize,
    pub rows: usize,
    pub points: Vec<[f32; 4]>,
}

/// How far each phase is: the sides bent (`bent`) and the window slid (`poured`), each 0 to 1 and
/// eased; the gap between the window and the slot along the way it goes (`gap`); and the share of
/// the progress at which the bend is done (`settled`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Schedule {
    pub bent: f64,
    pub poured: f64,
    pub gap: f64,
    pub settled: f64,
}

/// The rounding of a 32-bit float, as JavaScript's `Math.fround`.
fn fround(value: f64) -> f64 {
    value as f32 as f64
}

/// `0.5 − 0.5·x`: with `x = cos(πt)`, half a cosine from 0 to 1.
fn half(x: f64) -> f64 {
    -0.5 * x + 0.5
}

/// The animation's record, as the Dock reads it, along the way the window goes: where the window's
/// far edge is (`far`, the one away from the slot) and its length that way (negative when it goes
/// toward smaller coordinates), the slot's near and far edges, how far the slot's two sides are
/// from the window's (`left`, `right`, across the way it goes), how much the window overlaps the
/// Dock's edge (`overlap`, negative or positive by the way) and how far it travels.
#[derive(Clone, Copy, Debug)]
struct Record {
    far: f64,
    length: f64,
    near: f64,
    icon_far: f64,
    left: f64,
    right: f64,
    overlap: f64,
    travel: f64,
}

fn record(window: Rect, slot: Rect, edge: Edge) -> Record {
    let Rect {
        x: r,
        y: n,
        width: i,
        height: a,
    } = window;
    let Rect {
        x: l,
        y: s,
        width: o,
        height: u,
    } = slot;
    match edge {
        Edge::Bottom => {
            let gap = s - (n + a);
            let overlap = if gap > 0.0 { 0.0 } else { gap };
            Record {
                far: n,
                length: a,
                near: s,
                icon_far: s + u,
                left: l - r,
                right: l + o - (r + i),
                overlap,
                travel: if overlap >= 0.0 { s - n } else { a },
            }
        }
        Edge::Top => {
            let near = s + u;
            let far = n + a;
            let gap = near - n;
            let overlap = if gap < 0.0 { 0.0 } else { gap };
            Record {
                far,
                length: -a,
                near,
                icon_far: s,
                left: l - r,
                right: l + o - (r + i),
                overlap,
                travel: if overlap <= 0.0 { near - far } else { -a },
            }
        }
        Edge::Right => {
            let gap = l - (r + i);
            let overlap = if gap > 0.0 { 0.0 } else { gap };
            Record {
                far: r,
                length: i,
                near: l,
                icon_far: l + o,
                left: s - n,
                right: s + u - (n + a),
                overlap,
                travel: if overlap >= 0.0 { l - r } else { i },
            }
        }
        Edge::Left => {
            let near = l + o;
            let far = r + i;
            let gap = near - r;
            let overlap = if gap < 0.0 { 0.0 } else { gap };
            Record {
                far,
                length: -i,
                near,
                icon_far: l,
                left: s - n,
                right: s + u - (n + a),
                overlap,
                travel: if overlap <= 0.0 { near - far } else { -i },
            }
        }
    }
}

/// The two phases at `progress`. The bend lasts `trunc(ln(d/2 + 1))·50` for `d` the larger of how
/// far the sides move (summed, over 8) and twice any overlap with the Dock; the slide the same for
/// the travel. The slide's cosine covers the gap at `reach` (the `acos` solves it for that), where
/// the bend must be done: a bend that would finish earlier is stretched to it (`narrow`), one that
/// would finish later delays the slide (`lead`).
fn schedule_of(progress: f64, record: &Record) -> Schedule {
    let sideways = (record.left.abs() + record.right.abs()) * 0.125;
    let overlap = (record.overlap + record.overlap).abs();
    let bend = 50.0
        * (0.5
            * (if overlap > sideways {
                overlap
            } else {
                sideways
            })
            .abs()
            + 1.0)
            .ln()
            .trunc();
    let travel = record.travel.abs();
    let slide = 50.0 * (0.5 * travel + 1.0).ln().trunc();
    let gap = (record.travel - record.length).abs();
    let reach = (-(slide * gap / travel / slide * 2.0) + 1.0).acos() / PI * slide;
    let narrow = if bend > reach { bend } else { reach };
    let lead = if bend > reach { bend - reach } else { 0.0 };
    let along = (slide + lead) * progress;
    let slid = along - lead;
    let clamp = |value: f64, most: f64| {
        if value < 0.0 {
            0.0
        } else if value > most {
            most
        } else {
            value
        }
    };
    Schedule {
        bent: half((clamp(along, narrow) / narrow * PI).cos()),
        poured: half((clamp(slid, slide) / slide * PI).cos()),
        gap,
        settled: narrow / (slide + lead),
    }
}

/// The phases of a minimize at `progress` (0 the window open, 1 in the slot).
pub fn schedule(progress: f64, window: Rect, slot: Rect, edge: Edge) -> Schedule {
    schedule_of(fround(progress), &record(window, slot, edge))
}

/// The most floats the Dock keeps of a mesh's rows (three a row: where along, and the two sides'
/// offsets): 400 rows. A write past it is lost, and a read past it is no number, as a JavaScript
/// `Float32Array`'s.
const ROW_FLOATS: usize = 1200;

struct Rows([f32; ROW_FLOATS]);

impl Rows {
    fn set(&mut self, index: usize, value: f64) {
        if let Some(slot) = self.0.get_mut(index) {
            *slot = value as f32;
        }
    }

    fn get(&self, index: usize) -> f64 {
        self.0
            .get(index)
            .map_or(f64::NAN, |&value| f64::from(value))
    }
}

/// What the side tracing runs: its rows, where the funnel starts (`origin`), the sides' offsets
/// at the slot (`left`, `right`), the cosine's period (`period`, the travel rounded) and the
/// splits made so far.
struct Trace<'a> {
    rows: &'a mut Rows,
    origin: f64,
    left: f64,
    right: f64,
    period: f64,
    calls: u32,
}

impl Trace<'_> {
    /// The sides' cosine at `at`, 0 at the funnel's start and 1 a period on.
    fn side(&self, at: f64) -> f64 {
        half(((at - self.origin) * PI / self.period).cos())
    }

    /// Traces the sides from `from` to `to` into the rows from index `index`, halving each piece
    /// until it is within 0.6 points of straight: the rows between them, in order, none at either
    /// end. How many floats it wrote.
    fn trace(&mut self, index: usize, from: f64, to: f64) -> usize {
        let at_to = self.side(to);
        self.calls += 1;
        if self.calls > 64 {
            return 0;
        }
        let right_to = fround(self.right * at_to);
        let left_to = fround(self.left * at_to);
        let mut written = 0;
        let mut start = from;
        let mut next = index;
        loop {
            let length = (to - start).abs();
            if length < 8.0 {
                return written;
            }
            let middle = (to + start) * 0.5;
            let at_start = ((start - self.origin) * PI / self.period).cos();
            let at_middle = self.side(middle);
            let left = fround(self.left * at_middle);
            let right = fround(self.right * at_middle);
            if length < 128.0 {
                let share = half(at_start);
                let left_start = fround(self.left * share);
                if 0.6 > (0.5 * fround(left_to + left_start) - left).abs() {
                    let right_start = fround(self.right * share);
                    if 0.6 > (0.5 * fround(right_to + right_start) - right).abs() {
                        return written;
                    }
                }
            }
            let before = self.trace(next, start, middle);
            let at = before + next;
            if at > 1199 {
                return written;
            }
            self.rows.set(at, middle);
            self.rows.set(at + 1, left);
            self.rows.set(at + 2, right);
            if at > 1196 {
                return written;
            }
            next = at + 3;
            written += before + 3;
            self.calls += 1;
            start = middle;
            if self.calls >= 65 {
                return written;
            }
        }
    }
}

/// The Dock's mesh for `window` going into `slot` at `progress` (0 open, 1 in the slot; restoring
/// is the same from 1 to 0), every point in points.
pub fn mesh(progress: f64, window: Rect, slot: Rect, edge: Edge) -> Mesh {
    let n = record(window, slot, edge);
    let Schedule {
        bent, poured, gap, ..
    } = schedule_of(fround(progress), &n);
    let left = n.left * bent;
    let right = n.right * bent;
    let slid = n.travel * poured;
    let period = (n.travel + 0.5).trunc();
    // The funnel starts where the window's far edge is once the overlap is bent away, and the
    // window's far and near edges are slid down it.
    let origin = n.far + n.overlap * bent;
    let far = origin + slid;
    let near = n.length + far;
    let forward = matches!(edge, Edge::Bottom | Edge::Right);
    // Its near edge, clamped to the slot's near edge once past it.
    let clamped = if forward {
        if near <= n.near {
            near
        } else {
            n.near
        }
    } else if near >= n.near {
        near
    } else {
        n.near
    };
    let last = if poured <= 0.0 { near } else { clamped };
    let mut rows = Rows([0.0; ROW_FLOATS]);
    let first = half(((far - origin) * PI / period).cos());
    rows.set(0, far);
    rows.set(1, left * first);
    rows.set(2, right * first);
    let traced = Trace {
        rows: &mut rows,
        origin,
        left,
        right,
        period,
        calls: 0,
    }
    .trace(3, far, last);
    let at_last = half(((last - origin) * PI / period).cos());
    rows.set(traced + 3, last);
    rows.set(traced + 4, left * at_last);
    rows.set(traced + 5, right * at_last);
    let mut count = traced + 6;
    // Gone further than the gap it started with: a last row at its near edge, within the slot.
    if slid.abs() > gap {
        let mut end = near;
        if forward {
            if end > n.icon_far {
                end = n.icon_far;
            }
        } else if end < n.icon_far {
            end = n.icon_far;
        }
        rows.set(count, end);
        rows.set(count + 1, left);
        rows.set(count + 2, right);
        count += 3;
    }
    build(&rows, count / 3, window, edge)
}

/// The rows as the mesh's vertices: the window's own length spread over them from the first to
/// the last, each row's ends where its sides are.
fn build(rows: &Rows, count: usize, window: Rect, edge: Edge) -> Mesh {
    let vertical = matches!(edge, Edge::Bottom | Edge::Top);
    let reversed = matches!(edge, Edge::Top | Edge::Left);
    let mut points = vec![[0.0f32; 4]; 2 * count];
    if count == 0 {
        return Mesh {
            columns: 2,
            rows: 0,
            points,
        };
    }
    let start = rows.get(3 * if reversed { count - 1 } else { 0 });
    let end = rows.get(3 * if reversed { 0 } else { count - 1 });
    let along = fround(
        if vertical {
            window.height
        } else {
            window.width
        } / (end - start),
    );
    let across = if vertical {
        window.width
    } else {
        window.height
    };
    let across_f32 = fround(across);
    let origin = if vertical { window.x } else { window.y };
    for t in 0..count {
        let r = if reversed { count - 1 - t } else { t };
        let at = rows.get(3 * r);
        let source = fround((at - start) * along);
        let low = fround(origin + rows.get(3 * r + 1));
        let high = fround(origin + across + rows.get(3 * r + 2));
        let f = |v: f64| v as f32;
        if vertical {
            points[2 * t] = [0.0, f(source), f(low), f(at)];
            points[2 * t + 1] = [f(across_f32), f(source), f(high), f(at)];
        } else {
            points[t] = [f(source), 0.0, f(at), f(low)];
            points[count + t] = [f(source), f(across_f32), f(at), f(high)];
        }
    }
    if vertical {
        Mesh {
            columns: 2,
            rows: count,
            points,
        }
    } else {
        Mesh {
            columns: count,
            rows: 2,
            points,
        }
    }
}

#[cfg(test)]
mod tests;
