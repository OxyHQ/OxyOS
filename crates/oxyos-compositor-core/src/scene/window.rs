//! A toplevel's own window (ADR 0016): the sizes it can take, zooming it to fill the work area,
//! the compositor resizing it by its edges, and which window is the active one.

use super::{Role, Scene, SurfaceKey};
use oxyos_protocol::{ErrorCode, Event, PointerShape, Rect, ShellKind, Size};

/// How far outside a window's edges a press resizes it, in logical pixels: the band is in the
/// window's shadow, so it takes nothing from the window's own content.
pub const RESIZE_BAND: i64 = 6;
/// How far along an edge from a corner the band resizes from the corner.
pub const RESIZE_CORNER: i64 = 12;
/// No window is resized smaller than this, whatever its limits.
pub const MIN_WINDOW: Size = Size {
    width: 64,
    height: 64,
};

/// The sizes a window can take; a zero in `max` is no bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Limits {
    pub min: Size,
    pub max: Size,
}

impl Limits {
    pub const NONE: Limits = Limits {
        min: Size {
            width: 0,
            height: 0,
        },
        max: Size {
            width: 0,
            height: 0,
        },
    };

    pub fn clamp(&self, size: Size) -> Size {
        let axis = |value: u32, min: u32, max: u32, floor: u32| {
            let value = value.max(min).max(floor);
            if max == 0 {
                value
            } else {
                value.min(max.max(floor))
            }
        };
        Size::new(
            axis(size.width, self.min.width, self.max.width, MIN_WINDOW.width),
            axis(
                size.height,
                self.min.height,
                self.max.height,
                MIN_WINDOW.height,
            ),
        )
    }
}

/// The edges of a window a resize moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Edges {
    /// The edges of `rect` a point in its band is beside, or none if it is inside `rect` or
    /// beyond the band.
    fn at(rect: Rect, x: i64, y: i64) -> Option<Edges> {
        let (left, top) = (i64::from(rect.x), i64::from(rect.y));
        let (right, bottom) = (rect.right(), rect.bottom());
        let inside = |v: i64, lo: i64, hi: i64| v >= lo && v < hi;
        if !inside(x, left - RESIZE_BAND, right + RESIZE_BAND)
            || !inside(y, top - RESIZE_BAND, bottom + RESIZE_BAND)
            || inside(x, left, right) && inside(y, top, bottom)
        {
            return None;
        }
        let mut edges = Edges {
            left: x < left,
            right: x >= right,
            top: y < top,
            bottom: y >= bottom,
        };
        if edges.left || edges.right {
            edges.top |= y < top + RESIZE_CORNER;
            edges.bottom |= y >= bottom - RESIZE_CORNER;
        }
        if edges.top || edges.bottom {
            edges.left |= x < left + RESIZE_CORNER;
            edges.right |= x >= right - RESIZE_CORNER;
        }
        Some(edges)
    }

    pub fn pointer_shape(self) -> PointerShape {
        match self {
            Edges {
                left: true,
                top: true,
                ..
            }
            | Edges {
                right: true,
                bottom: true,
                ..
            } => PointerShape::ResizeTopLeft,
            Edges {
                right: true,
                top: true,
                ..
            }
            | Edges {
                left: true,
                bottom: true,
                ..
            } => PointerShape::ResizeTopRight,
            Edges { left, right, .. } if left || right => PointerShape::ResizeHorizontal,
            _ => PointerShape::ResizeVertical,
        }
    }
}

/// A resize the compositor is doing: the window, the edges the press was beside, and where the
/// pointer, the window and its size were when it began.
#[derive(Clone, Copy, Debug)]
pub(super) struct Resize {
    pub surface: SurfaceKey,
    pub edges: Edges,
    pub pointer: (i32, i32),
    pub origin: (i32, i32),
    pub size: Size,
}

impl Scene {
    /// The window and edges a press at (x, y) would resize, if it is in a window's band and
    /// over nothing else: what is drawn above the band there takes the press instead.
    pub(super) fn resize_edges_at(&self, x: i32, y: i32) -> Option<(SurfaceKey, Edges)> {
        if self.locked || !self.grab.is_empty() || self.moving.is_some() {
            return None;
        }
        let (x, y) = (i64::from(x), i64::from(y));
        for item in self.render_list().into_iter().rev() {
            let super::RenderItem::Surface { key, rect, .. } = item else {
                return None;
            };
            let node = &self.nodes[&key];
            if rect.contains_point(x as i32, y as i32) {
                let local = (x - i64::from(rect.x), y - i64::from(rect.y));
                let accepts = node
                    .input_region
                    .is_none_or(|region| region.contains_point(local.0 as i32, local.1 as i32));
                if accepts {
                    return None;
                }
                continue;
            }
            if node.role == Role::Toplevel && !node.fullscreen {
                if let Some(edges) = Edges::at(rect, x, y) {
                    return Some((key, edges));
                }
            }
        }
        None
    }

    /// A press in a window's band: the window is raised and focused and resized as the pointer
    /// moves, until every button is up. No client sees the press, and it authorizes nothing.
    pub(super) fn begin_resize(&mut self, key: SurfaceKey, edges: Edges) {
        self.activate(key);
        let node = self
            .nodes
            .get_mut(&key)
            .expect("resized windows are present");
        let (origin, size) = node.upcoming();
        // A window resized by hand is no longer zoomed.
        node.maximized = false;
        node.unzoomed = None;
        self.resizing = Some(Resize {
            surface: key,
            edges,
            pointer: self.pointer,
            origin,
            size,
        });
        self.last_press = None;
        self.set_pointer_focus(None);
    }

    pub(super) fn resize_to_pointer(&mut self, resize: Resize, x: i32, y: i32) {
        let Some(node) = self.live(resize.surface) else {
            self.resizing = None;
            return;
        };
        let dx = i64::from(x) - i64::from(resize.pointer.0);
        let dy = i64::from(y) - i64::from(resize.pointer.1);
        let grow = |length: u32, delta: i64, more: bool, less: bool| {
            let delta = if more {
                delta
            } else if less {
                -delta
            } else {
                0
            };
            (i64::from(length) + delta).clamp(1, i64::from(u32::MAX)) as u32
        };
        let edges = resize.edges;
        let wanted = Size::new(
            grow(resize.size.width, dx, edges.right, edges.left),
            grow(resize.size.height, dy, edges.bottom, edges.top),
        );
        let size = node.limits.clamp(wanted);
        // The edges not pressed stay where they were.
        let origin = (
            if edges.left {
                resize.origin.0 + resize.size.width as i32 - size.width as i32
            } else {
                resize.origin.0
            },
            if edges.top {
                resize.origin.1 + resize.size.height as i32 - size.height as i32
            } else {
                resize.origin.1
            },
        );
        if node.upcoming() == (origin, size) {
            return;
        }
        // One configure in flight, as a Wayland compositor resizes: while the client has not
        // shown a frame for the last one, only the latest geometry is kept, and sent when it
        // does (`end_frame`). The window shows its last frame where it was meanwhile, so the
        // edges not dragged stay put frame by frame.
        if node.pending.is_some() {
            let node = self.nodes.get_mut(&resize.surface).expect("live");
            node.wanted = Some((origin, size));
        } else {
            self.reconfigure(resize.surface, Some(origin), size);
        }
    }

    pub(super) fn end_resize(&mut self) {
        if let Some(resize) = self.resizing.take() {
            if self.nodes.contains_key(&resize.surface) {
                self.reconstrain_children(resize.surface);
            }
            self.refresh_pointer_focus();
        }
    }

    pub(super) fn set_size_limits(
        &mut self,
        key: SurfaceKey,
        min: Size,
        max: Size,
    ) -> Result<(), ErrorCode> {
        let node = self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        if node.role != Role::Toplevel {
            return Err(ErrorCode::InvalidRole);
        }
        let ordered = |min: u32, max: u32| max == 0 || max >= min;
        if !ordered(min.width, max.width) || !ordered(min.height, max.height) {
            return Err(ErrorCode::InvalidGeometry);
        }
        let node = self.nodes.get_mut(&key).expect("present");
        node.limits = Limits { min, max };
        if node.fullscreen {
            return Ok(());
        }
        let size = node.upcoming().1;
        let clamped = node.limits.clamp(size);
        if clamped != size {
            self.reconfigure(key, None, clamped);
        }
        Ok(())
    }

    /// Zooms a window to fill its output's work area (its largest size within that, centred on
    /// it, if its limits are smaller) or restores the frame it had. A full-screen window keeps
    /// its state until it leaves full screen.
    pub(super) fn set_maximized(
        &mut self,
        key: SurfaceKey,
        enabled: bool,
    ) -> Result<(), ErrorCode> {
        let node = self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        if node.role != Role::Toplevel {
            return Err(ErrorCode::InvalidRole);
        }
        if node.maximized == enabled || node.fullscreen || self.locked {
            return Ok(());
        }
        let current = node.upcoming();
        let (origin, size) = if enabled {
            let area = self
                .work_area(node.output)
                .ok_or(ErrorCode::InvalidGeometry)?;
            let size = node.limits.clamp(Size::new(area.width, area.height));
            let size = Size::new(size.width.min(area.width), size.height.min(area.height));
            let origin = (
                area.x + ((area.width - size.width) / 2) as i32,
                area.y + ((area.height - size.height) / 2) as i32,
            );
            (origin, size)
        } else {
            node.unzoomed.unwrap_or(current)
        };
        let node = self.nodes.get_mut(&key).expect("present");
        node.maximized = enabled;
        node.unzoomed = enabled.then_some(current);
        self.reconfigure(key, Some(origin), size);
        // Its frame goes from where it is to where it is configured, animated (ADR 0016).
        self.appeared(key, super::appearances::Change::Zoomed);
        Ok(())
    }

    /// A window minimizing itself, as the shell minimizes it.
    pub(super) fn minimize_request(&mut self, key: SurfaceKey) -> Result<(), ErrorCode> {
        let node = self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        if node.role != Role::Toplevel {
            return Err(ErrorCode::InvalidRole);
        }
        if !self.locked {
            self.minimize(key);
        }
        Ok(())
    }

    /// After focus changed, or the active window stopped being shown: the active window is the
    /// toplevel that has the focus itself or through its popups and dialogs. Focus on one of the
    /// shell's panels (the top bar's menus act for the active application) leaves it, while it is
    /// shown (minimized or on another workspace, it is not active); anything else (an overlay, the
    /// lock, nothing) leaves no window active.
    pub(super) fn update_active(&mut self) {
        let root = self.focus.map(|focus| self.root_of(focus));
        let active = match root.and_then(|root| self.nodes.get(&root).map(|node| (root, node))) {
            Some((root, node)) if node.role == Role::Toplevel => Some(root),
            Some((_, node))
                if matches!(
                    node.role,
                    Role::Shell {
                        kind: ShellKind::Panel,
                        ..
                    }
                ) =>
            {
                self.active.filter(|active| self.visible(*active))
            }
            _ => None,
        };
        if active == self.active {
            return;
        }
        if let Some(old) = self.active.take() {
            if self.live(old).is_some() {
                self.emit(
                    old.client,
                    Event::Activated {
                        surface: old.id,
                        active: false,
                    },
                );
            }
        }
        if let Some(new) = active {
            self.emit(
                new.client,
                Event::Activated {
                    surface: new.id,
                    active: true,
                },
            );
        }
        self.active = active;
    }

    /// The window the compositor is moving with the pointer (`BeginMove`), if any.
    pub fn moving_window(&self) -> Option<SurfaceKey> {
        self.moving.map(|moving| moving.surface)
    }

    /// The window that is active (ADR 0016), if any.
    pub fn active_window(&self) -> Option<SurfaceKey> {
        self.active
    }

    /// Whether a window is zoomed.
    pub fn is_maximized(&self, key: SurfaceKey) -> bool {
        self.live(key).is_some_and(|node| node.maximized)
    }

    /// Where a surface is going to be once its configures are presented (global), as a zoom's
    /// transition ends there.
    pub fn upcoming_rect(&self, key: SurfaceKey) -> Option<Rect> {
        self.live(key)?;
        self.global_rect_pending(key)
    }

    /// The serial of the latest configure a surface was sent.
    pub fn configure_serial(&self, key: SurfaceKey) -> Option<u64> {
        self.live(key)
            .map(|node| node.presentation.configuration().0)
    }
}
