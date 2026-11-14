//! Window tiling, as macOS 15's (docs/adr/0016-windows.mdx, decision 12): a window dragged to the
//! left or right edge of a display takes that half of it, to a corner a quarter, to the menu bar
//! (the top edge) the whole of it (Fill), a translucent preview showing where before the release;
//! with Alt (a Mac's Option) held, the preview follows the pointer anywhere on the display. Dragged
//! away, a tiled window takes back the size it had before. Super+Alt with F, C, R or an arrow (a
//! Mac's Fn-Control with the same keys) fills, centres, returns to the previous size or tiles to
//! a half, and with Shift an arrow arranges the active window and the one behind it. Tiles keep
//! a margin from the work area's edges and from each other, as a Mac's "Tiled windows have
//! margins", its default.

use super::{Move, OutputId, Role, Scene, SurfaceKey};
use oxyos_protocol::{ErrorCode, Rect, Size, MODIFIER_ALT};

/// The margin tiles keep from the work area's edges and from each other, in logical pixels.
pub const TILE_MARGIN: u32 = 8;
/// How close to a display's outer edge the pointer is when it reaches it, in logical pixels.
pub const TILE_EDGE: i64 = 4;
/// Along an edge, the share of the work area's side at each end that tiles to a corner (an
/// eighth of it).
const CORNER_SHARE: u32 = 8;
/// With Alt held, the share of the work area's side at each end that is a side's column or row
/// (a quarter of it); between, the middle.
const ALT_SHARE: u32 = 4;
/// The preview's colour (0x00RRGGBB) and opacity: a translucent light pane under the window, as a
/// Mac's highlighted area.
pub const PREVIEW_COLOR: u32 = 0x00ff_ffff;
pub const PREVIEW_OPACITY: u8 = 77;

/// Where a window is tiled on its display's work area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tile {
    /// The whole work area, within its margins (a Mac's Fill).
    Fill,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Windows arranged together (a Mac's Window › Move & Resize › Arrange): the active window takes
/// the first tile, the windows behind it on its display the others, front to back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrangement {
    LeftAndRight,
    RightAndLeft,
    TopAndBottom,
    BottomAndTop,
    Quarters,
    LeftAndQuarters,
    RightAndQuarters,
    TopAndQuarters,
    BottomAndQuarters,
}

/// What is asked of a window's place: a tile, the middle of the work area at its size, an
/// arrangement with the windows behind it, or the frame it had before any of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Tile(Tile),
    Center,
    Arrange(Arrangement),
    ReturnToPreviousSize,
}

/// The scene's layout for the shell's `LayOutWindow` (the Window menu, decision 12); none for
/// Zoom, which is the window's zoom and not a layout.
pub(super) fn layout_of(asked: oxyos_protocol::WindowLayout) -> Option<Layout> {
    use oxyos_protocol::WindowLayout as Asked;
    Some(match asked {
        Asked::Fill => Layout::Tile(Tile::Fill),
        Asked::Center => Layout::Center,
        Asked::Left => Layout::Tile(Tile::Left),
        Asked::Right => Layout::Tile(Tile::Right),
        Asked::Top => Layout::Tile(Tile::Top),
        Asked::Bottom => Layout::Tile(Tile::Bottom),
        Asked::TopLeft => Layout::Tile(Tile::TopLeft),
        Asked::TopRight => Layout::Tile(Tile::TopRight),
        Asked::BottomLeft => Layout::Tile(Tile::BottomLeft),
        Asked::BottomRight => Layout::Tile(Tile::BottomRight),
        Asked::LeftAndRight => Layout::Arrange(Arrangement::LeftAndRight),
        Asked::RightAndLeft => Layout::Arrange(Arrangement::RightAndLeft),
        Asked::TopAndBottom => Layout::Arrange(Arrangement::TopAndBottom),
        Asked::BottomAndTop => Layout::Arrange(Arrangement::BottomAndTop),
        Asked::Quarters => Layout::Arrange(Arrangement::Quarters),
        Asked::ReturnToPreviousSize => Layout::ReturnToPreviousSize,
        Asked::Zoom => return None,
    })
}

/// A window tiled or centred (`tile`, none when centred), and the frame it had before, which
/// Return to Previous Size and a drag away give back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Placed {
    pub tile: Option<Tile>,
    pub previous: ((i32, i32), Size),
}

impl Tile {
    /// The tile's rectangle in `area`, `TILE_MARGIN` from its edges and between tiles: two
    /// columns (or rows) share what the margins leave, the second taking an odd pixel.
    pub fn rect(self, area: Rect) -> Rect {
        let margin = TILE_MARGIN;
        let split = |start: i32, length: u32| {
            let room = length.saturating_sub(3 * margin);
            let first = room / 2;
            let second = room - first;
            let near = (start.saturating_add(margin as i32), first.max(1));
            let far = (
                start.saturating_add((2 * margin + first) as i32),
                second.max(1),
            );
            let whole = (
                start.saturating_add(margin as i32),
                length.saturating_sub(2 * margin).max(1),
            );
            (near, far, whole)
        };
        let (left, right, width) = split(area.x, area.width);
        let (top, bottom, height) = split(area.y, area.height);
        let (x, y) = match self {
            Tile::Fill => (width, height),
            Tile::Left => (left, height),
            Tile::Right => (right, height),
            Tile::Top => (width, top),
            Tile::Bottom => (width, bottom),
            Tile::TopLeft => (left, top),
            Tile::TopRight => (right, top),
            Tile::BottomLeft => (left, bottom),
            Tile::BottomRight => (right, bottom),
        };
        Rect::new(x.0, y.0, x.1, y.1)
    }
}

impl Arrangement {
    /// The tiles, the active window's first.
    pub fn tiles(self) -> &'static [Tile] {
        match self {
            Arrangement::LeftAndRight => &[Tile::Left, Tile::Right],
            Arrangement::RightAndLeft => &[Tile::Right, Tile::Left],
            Arrangement::TopAndBottom => &[Tile::Top, Tile::Bottom],
            Arrangement::BottomAndTop => &[Tile::Bottom, Tile::Top],
            Arrangement::Quarters => &[
                Tile::TopLeft,
                Tile::TopRight,
                Tile::BottomLeft,
                Tile::BottomRight,
            ],
            Arrangement::LeftAndQuarters => &[Tile::Left, Tile::TopRight, Tile::BottomRight],
            Arrangement::RightAndQuarters => &[Tile::Right, Tile::TopLeft, Tile::BottomLeft],
            Arrangement::TopAndQuarters => &[Tile::Top, Tile::BottomLeft, Tile::BottomRight],
            Arrangement::BottomAndQuarters => &[Tile::Bottom, Tile::TopLeft, Tile::TopRight],
        }
    }
}

/// The tile a drag's pointer at (`x`, `y`) on a display asks for: at its outer edges (`edges`:
/// left, top, right, bottom, each where no other display continues it) or, with `alt`, anywhere.
/// `area` is the display's work area; `top_bar` whether the pointer is over the menu bar, which
/// is the top edge's. None: no tile.
fn target(
    area: Rect,
    (x, y): (i64, i64),
    edges: [bool; 4],
    top_bar: bool,
    alt: bool,
) -> Option<Tile> {
    let (left, top) = (i64::from(area.x), i64::from(area.y));
    let (right, bottom) = (area.right(), area.bottom());
    let (width, height) = (i64::from(area.width), i64::from(area.height));
    if alt {
        let column = |share: u32| {
            let side = width / i64::from(share);
            if x < left + side {
                0
            } else if x >= right - side {
                2
            } else {
                1
            }
        };
        let row = {
            let side = height / i64::from(ALT_SHARE);
            if y < top + side {
                0
            } else if y >= bottom - side {
                2
            } else {
                1
            }
        };
        return Some(match (column(ALT_SHARE), row) {
            (0, 0) => Tile::TopLeft,
            (0, 1) => Tile::Left,
            (0, _) => Tile::BottomLeft,
            (2, 0) => Tile::TopRight,
            (2, 1) => Tile::Right,
            (2, _) => Tile::BottomRight,
            _ => Tile::Fill,
        });
    }
    let [at_left, at_top, at_right, at_bottom] = edges;
    let (corner_x, corner_y) = (
        width / i64::from(CORNER_SHARE),
        height / i64::from(CORNER_SHARE),
    );
    let upper = y < top + corner_y;
    let lower = y >= bottom - corner_y;
    if at_left || at_right {
        return Some(match (at_left, upper, lower) {
            (true, true, _) => Tile::TopLeft,
            (true, _, true) => Tile::BottomLeft,
            (true, ..) => Tile::Left,
            (false, true, _) => Tile::TopRight,
            (false, _, true) => Tile::BottomRight,
            (false, ..) => Tile::Right,
        });
    }
    let leftmost = x < left + corner_x;
    let rightmost = x >= right - corner_x;
    if at_top || top_bar {
        return Some(match (leftmost, rightmost) {
            (true, _) => Tile::TopLeft,
            (_, true) => Tile::TopRight,
            _ => Tile::Fill,
        });
    }
    if at_bottom {
        return match (leftmost, rightmost) {
            (true, _) => Some(Tile::BottomLeft),
            (_, true) => Some(Tile::BottomRight),
            // The bottom edge's middle is the dock's: no tile, as on a Mac.
            _ => None,
        };
    }
    None
}

/// A window's frame for `rect` (a tile): its size within its limits and the work area, centred on
/// the tile and kept within the area.
fn fitted(rect: Rect, area: Rect, limits: &super::window::Limits) -> ((i32, i32), Size) {
    let size = limits.clamp(Size::new(rect.width, rect.height));
    let size = Size::new(size.width.min(area.width), size.height.min(area.height));
    let origin = (
        (i64::from(rect.x) + (i64::from(rect.width) - i64::from(size.width)) / 2) as i32,
        (i64::from(rect.y) + (i64::from(rect.height) - i64::from(size.height)) / 2) as i32,
    );
    super::within(area, origin, size)
}

impl Scene {
    /// The tile a window being dragged would take if released now, on the display under the
    /// pointer, and that display: none unless the compositor is moving a window (`BeginMove`)
    /// that has moved, and the pointer is at a display's outer edge or Alt is held.
    pub(super) fn tile_target(&self) -> Option<(SurfaceKey, OutputId, Tile)> {
        let moving = self.moving.filter(|moving| moving.moved)?;
        if self.locked || self.live(moving.surface).is_none() {
            return None;
        }
        let (x, y) = (i64::from(self.pointer.0), i64::from(self.pointer.1));
        let on = |x: i64, y: i64| {
            self.outputs.iter().any(|o| {
                x >= i64::from(o.rect.x)
                    && x < o.rect.right()
                    && y >= i64::from(o.rect.y)
                    && y < o.rect.bottom()
            })
        };
        let output = self.outputs.iter().copied().find(|o| {
            x >= i64::from(o.rect.x)
                && x < o.rect.right()
                && y >= i64::from(o.rect.y)
                && y < o.rect.bottom()
        })?;
        let area = self.work_area(output.id)?;
        let rect = output.rect;
        // An edge another display continues is crossed, not tiled to.
        let edges = [
            x < i64::from(rect.x) + TILE_EDGE && !on(i64::from(rect.x) - 1, y),
            y < i64::from(rect.y) + TILE_EDGE && !on(x, i64::from(rect.y) - 1),
            x >= rect.right() - TILE_EDGE && !on(rect.right(), y),
            y >= rect.bottom() - TILE_EDGE && !on(x, rect.bottom()),
        ];
        let top_bar = y < i64::from(area.y);
        let alt = self.modifiers() & MODIFIER_ALT != 0;
        let tile = target(area, (x, y), edges, top_bar, alt)?;
        Some((moving.surface, output.id, tile))
    }

    /// Where the preview of the tile a drag would take is drawn (global, logical), if any.
    pub fn tile_preview(&self) -> Option<Rect> {
        let (_, output, tile) = self.tile_target()?;
        Some(tile.rect(self.work_area(output)?))
    }

    /// Whether a window is tiled (or centred) and has a frame to return to.
    pub fn tile_of(&self, key: SurfaceKey) -> Option<Option<Tile>> {
        self.live(key)?.placed.map(|placed| placed.tile)
    }

    /// Lays out a window as `layout` asks (Super+Alt's keys, the Window menu's Move & Resize):
    /// a toplevel shown, not full-screen, while unlocked; anything else stays as it is.
    pub fn lay_out_window(&mut self, key: SurfaceKey, layout: Layout) {
        let Some(node) = self.live(key) else {
            return;
        };
        if node.role != Role::Toplevel || node.fullscreen || node.hidden || self.locked {
            return;
        }
        let output = node.output;
        match layout {
            Layout::Tile(tile) => self.tile_window(key, output, tile),
            Layout::Center => self.center_window(key),
            Layout::ReturnToPreviousSize => self.return_to_previous_size(key),
            Layout::Arrange(arrangement) => {
                let behind = self.windows_behind(key, output);
                let tiles = arrangement.tiles();
                self.tile_window(key, output, tiles[0]);
                for (window, tile) in behind.into_iter().zip(&tiles[1..]) {
                    self.tile_window(window, output, *tile);
                }
            }
        }
    }

    /// A toplevel's own request for a layout (`LayOutSurface`, ADR 0016, decision 12: its zoom
    /// control's menu, a Mac's green button): laid out as the shell's LayOutWindow lays a window
    /// out, Zoom zooming it or back as its zoom control does. Only a toplevel: anything else of
    /// the client's is an error, as `SetMaximized` is.
    pub(super) fn lay_out_surface(
        &mut self,
        key: SurfaceKey,
        layout: oxyos_protocol::WindowLayout,
    ) -> Result<(), ErrorCode> {
        let node = self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        if node.role != Role::Toplevel {
            return Err(ErrorCode::InvalidRole);
        }
        match layout_of(layout) {
            Some(layout) => {
                self.lay_out_window(key, layout);
                Ok(())
            }
            None => {
                let zoomed = self.is_maximized(key);
                self.set_maximized(key, !zoomed)
            }
        }
    }

    /// The windows shown behind `key` on its display and workspace that can be tiled, front to
    /// back.
    fn windows_behind(&self, key: SurfaceKey, output: OutputId) -> Vec<SurfaceKey> {
        self.roots
            .iter()
            .rev()
            .copied()
            .filter(|root| *root != key && self.visible(*root))
            .filter(|root| {
                let node = &self.nodes[root];
                node.role == Role::Toplevel
                    && node.output == output
                    && node.mapped
                    && !node.fullscreen
                    && !node.hidden
            })
            .collect()
    }

    /// The frame a window had before it was laid out, remembered once: a zoomed window's is the
    /// one it had before its zoom.
    fn remember_frame(&mut self, key: SurfaceKey) -> ((i32, i32), Size) {
        let node = self.nodes.get_mut(&key).expect("present");
        let previous = match node.placed {
            Some(placed) => placed.previous,
            None if node.maximized => node.unzoomed.unwrap_or_else(|| node.upcoming()),
            None => node.upcoming(),
        };
        node.maximized = false;
        node.unzoomed = None;
        previous
    }

    /// Tiles a window on `output`'s work area: its frame goes there on the zoom's spring, its
    /// client told the new size once.
    pub(super) fn tile_window(&mut self, key: SurfaceKey, output: OutputId, tile: Tile) {
        let Some(area) = self.work_area(output) else {
            return;
        };
        let (origin, size) = fitted(tile.rect(area), area, &self.nodes[&key].limits);
        let previous = self.remember_frame(key);
        self.nodes.get_mut(&key).expect("present").placed = Some(Placed {
            tile: Some(tile),
            previous,
        });
        if self.nodes[&key].output != output {
            self.move_to_output(key, output);
        }
        self.place(key, origin, size);
    }

    /// Centres a window on its display's work area at its size (a Mac's Center).
    fn center_window(&mut self, key: SurfaceKey) {
        let node = &self.nodes[&key];
        let Some(area) = self.work_area(node.output) else {
            return;
        };
        let size = if node.maximized {
            node.unzoomed.unwrap_or_else(|| node.upcoming()).1
        } else {
            node.upcoming().1
        };
        let origin = (
            (i64::from(area.x) + (i64::from(area.width) - i64::from(size.width)) / 2) as i32,
            (i64::from(area.y) + (i64::from(area.height) - i64::from(size.height)) / 2) as i32,
        );
        let (origin, size) = super::within(area, origin, size);
        let previous = self.remember_frame(key);
        self.nodes.get_mut(&key).expect("present").placed = Some(Placed {
            tile: None,
            previous,
        });
        self.place(key, origin, size);
    }

    /// Gives a laid-out window back the frame it had before, within its display's work area.
    fn return_to_previous_size(&mut self, key: SurfaceKey) {
        let node = self.nodes.get_mut(&key).expect("present");
        let Some(placed) = node.placed.take() else {
            return;
        };
        let output = node.output;
        let (origin, size) = placed.previous;
        let (origin, size) = match self.work_area(output) {
            Some(area) => super::within(area, origin, size),
            None => (origin, size),
        };
        self.place(key, origin, size);
    }

    /// Configures a window's new frame, animated on the zoom's spring (ADR 0016, step 6), unless
    /// it is already there: then its client is told nothing.
    fn place(&mut self, key: SurfaceKey, origin: (i32, i32), size: Size) {
        // A zoomed window was told so: it is told it no longer is, even where it stays.
        let node = &self.nodes[&key];
        if node.upcoming() == (origin, size) && !node.pending_framing.maximized {
            return;
        }
        self.reconfigure(key, Some(origin), size);
        self.appeared(key, super::appearances::Change::Zoomed);
        self.reconstrain_children(key);
        self.refresh_pointer_focus();
    }

    /// A drag of a laid-out window that has begun to move: the window takes back the size it had
    /// before, the pointer at the same share of its width and the same height down it, and the
    /// drag goes on from there. Its client is told the size once.
    pub(super) fn untile_for_drag(&mut self, moving: Move) -> Move {
        let Some(node) = self.nodes.get_mut(&moving.surface) else {
            return moving;
        };
        let Some(placed) = node.placed.take() else {
            return moving;
        };
        let (_, size) = node.upcoming();
        let (_, wanted) = placed.previous;
        let wanted = node.limits.clamp(wanted);
        let (px, py) = (i64::from(self.pointer.0), i64::from(self.pointer.1));
        // Where the window was grabbed, not where the pointer is now: the share of its width and
        // the height down it it keeps under the hand.
        let across = (i64::from(moving.pointer.0) - i64::from(moving.origin.0))
            .clamp(0, i64::from(size.width));
        let down = (i64::from(moving.pointer.1) - i64::from(moving.origin.1))
            .clamp(0, i64::from(wanted.height.saturating_sub(1)));
        let x = px - across * i64::from(wanted.width) / i64::from(size.width.max(1));
        let y = py - down;
        let displayed = node.origin;
        self.reconfigure(moving.surface, Some((x as i32, y as i32)), wanted);
        // The drag goes on from the window as displayed: its new frame moves with it.
        Move {
            surface: moving.surface,
            pointer: self.pointer,
            origin: displayed,
            moved: true,
        }
    }

    /// The display's work area changed (another rectangle or scale): a tiled window takes its
    /// tile there. Its frame, if it is tiled.
    pub(super) fn retiled(&self, key: SurfaceKey, area: Rect) -> Option<((i32, i32), Size)> {
        let node = self.nodes.get(&key)?;
        let tile = node.placed?.tile?;
        Some(fitted(tile.rect(area), area, &node.limits))
    }

    /// A window's frame changed by hand (an edge resized, zoomed, the shell's resize): it is no
    /// longer laid out.
    pub(super) fn forget_layout(&mut self, key: SurfaceKey) {
        if let Some(node) = self.nodes.get_mut(&key) {
            node.placed = None;
        }
    }

    /// A drag released with a tile previewed: the window takes it, on the display under the
    /// pointer.
    pub(super) fn tile_on_release(&mut self, target: Option<(SurfaceKey, OutputId, Tile)>) {
        if let Some((key, output, tile)) = target {
            if self.live(key).is_some() {
                self.tile_window(key, output, tile);
            }
        }
    }
}

#[cfg(test)]
mod geometry {
    use super::*;

    const AREA: Rect = Rect::new(0, 0, 1280, 720);

    #[test]
    fn tiles_keep_their_margins_and_share_the_rest() {
        assert_eq!(Tile::Fill.rect(AREA), Rect::new(8, 8, 1264, 704));
        assert_eq!(Tile::Left.rect(AREA), Rect::new(8, 8, 628, 704));
        assert_eq!(Tile::Right.rect(AREA), Rect::new(644, 8, 628, 704));
        assert_eq!(Tile::Top.rect(AREA), Rect::new(8, 8, 1264, 348));
        assert_eq!(Tile::Bottom.rect(AREA), Rect::new(8, 364, 1264, 348));
        assert_eq!(Tile::TopLeft.rect(AREA), Rect::new(8, 8, 628, 348));
        assert_eq!(Tile::BottomRight.rect(AREA), Rect::new(644, 364, 628, 348));
        // An odd pixel goes to the second, and an area away from the origin offsets every tile.
        let area = Rect::new(-1281, 28, 1281, 716);
        assert_eq!(Tile::Left.rect(area), Rect::new(-1273, 36, 628, 700));
        assert_eq!(Tile::Right.rect(area), Rect::new(-637, 36, 629, 700));
        assert_eq!(Tile::Top.rect(area), Rect::new(-1273, 36, 1265, 346));
        assert_eq!(Tile::Bottom.rect(area), Rect::new(-1273, 390, 1265, 346));
    }

    #[test]
    fn a_drag_tiles_at_the_edges_and_anywhere_with_alt() {
        let none = [false; 4];
        let left = [true, false, false, false];
        let right = [false, false, true, false];
        let top = [false, true, false, false];
        let bottom = [false, false, false, true];
        // The left edge: its upper and lower eighths are the corners.
        assert_eq!(target(AREA, (0, 360), left, false, false), Some(Tile::Left));
        assert_eq!(
            target(AREA, (0, 89), left, false, false),
            Some(Tile::TopLeft)
        );
        assert_eq!(target(AREA, (0, 90), left, false, false), Some(Tile::Left));
        assert_eq!(
            target(AREA, (0, 630), left, false, false),
            Some(Tile::BottomLeft)
        );
        assert_eq!(
            target(AREA, (1279, 360), right, false, false),
            Some(Tile::Right)
        );
        assert_eq!(
            target(AREA, (1279, 0), right, false, false),
            Some(Tile::TopRight)
        );
        // The top edge (or the menu bar) fills, its ends the upper corners.
        assert_eq!(target(AREA, (640, 0), top, false, false), Some(Tile::Fill));
        assert_eq!(target(AREA, (640, 10), none, true, false), Some(Tile::Fill));
        assert_eq!(
            target(AREA, (159, 0), top, false, false),
            Some(Tile::TopLeft)
        );
        assert_eq!(
            target(AREA, (1120, 0), top, false, false),
            Some(Tile::TopRight)
        );
        // The bottom edge's middle is the dock's; its ends are the lower corners.
        assert_eq!(target(AREA, (640, 719), bottom, false, false), None);
        assert_eq!(
            target(AREA, (10, 719), bottom, false, false),
            Some(Tile::BottomLeft)
        );
        assert_eq!(
            target(AREA, (1270, 719), bottom, false, false),
            Some(Tile::BottomRight)
        );
        // Away from the edges, nothing, but with Alt held: the sides' quarters, else Fill.
        assert_eq!(target(AREA, (640, 360), none, false, false), None);
        assert_eq!(
            target(AREA, (640, 360), none, false, true),
            Some(Tile::Fill)
        );
        assert_eq!(
            target(AREA, (319, 360), none, false, true),
            Some(Tile::Left)
        );
        assert_eq!(
            target(AREA, (320, 360), none, false, true),
            Some(Tile::Fill)
        );
        assert_eq!(
            target(AREA, (100, 100), none, false, true),
            Some(Tile::TopLeft)
        );
        assert_eq!(
            target(AREA, (1000, 600), none, false, true),
            Some(Tile::BottomRight)
        );
        assert_eq!(
            target(AREA, (1000, 300), none, false, true),
            Some(Tile::Right)
        );
    }

    #[test]
    fn arrangements_give_the_active_window_the_first_tile() {
        assert_eq!(
            Arrangement::LeftAndRight.tiles(),
            &[Tile::Left, Tile::Right]
        );
        assert_eq!(
            Arrangement::BottomAndTop.tiles(),
            &[Tile::Bottom, Tile::Top]
        );
        assert_eq!(Arrangement::Quarters.tiles().len(), 4);
        assert_eq!(
            Arrangement::RightAndQuarters.tiles(),
            &[Tile::Right, Tile::TopLeft, Tile::BottomLeft]
        );
    }
}
