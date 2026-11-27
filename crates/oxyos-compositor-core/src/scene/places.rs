//! Windows coming back where they were (docs/adr/0070-windows-come-back-where-they-were.mdx), as a
//! Mac's do: the scene's side. The program running the compositor keeps where each application's
//! windows were and says where a new one goes before it is first shown; the scene says where they
//! are now.
//!
//! - **Where they are** ([`Scene::launched_windows`]): every window shown of an application the
//!   session launched (its launch record names it, ADR 0013), back to front, with its frame (the
//!   one it goes back to while zoomed or full screen), whether it is zoomed, its desktop and
//!   whether it is minimized.
//! - **Where a new one goes** ([`Scene::take_windows_to_place`], [`Scene::place_window`]): a
//!   launched application's toplevel not shown yet is offered once, as soon as its client is known
//!   to be that application. Placed, it is configured at its frame before its first frame, so it is
//!   never seen anywhere else, kept on a display that shows it (the one most of it was on, else the
//!   one it would have opened on) and inside that display's work area, and zoomed again if it was.
//! - **What the session opens as the person logs in** ([`Placing::at_login`]) takes its desktop,
//!   whether it is minimized and its place in the order of the windows opened with it too: one
//!   that was behind another of them is shown behind it as it arrives and takes no keyboard from
//!   it, whichever starts first. What the person opens later opens on the desktop they are on,
//!   in front, as they asked.

use super::{layer_of, Layer, Role, Scene, SurfaceKey, MIN_VISIBLE};
use oxyos_protocol::{Rect, Size};
use std::collections::BTreeMap;

/// Where a window is: what the program running the compositor keeps of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowPlace {
    /// Its frame, global and logical: where it is, or, zoomed or full screen, the frame it goes
    /// back to.
    pub origin: (i32, i32),
    pub size: Size,
    /// Zoomed to fill its display's work area (ADR 0016).
    pub zoomed: bool,
    /// Its desktop (workspace, ADR 0009).
    pub desktop: u32,
    /// Minimized into the dock.
    pub minimized: bool,
}

/// A window of an application the session launched, as it is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchedWindow {
    pub key: SurfaceKey,
    /// The application its launch record names.
    pub application: String,
    pub place: WindowPlace,
}

/// Where a new window goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placing {
    pub place: WindowPlace,
    /// Opened by the session as the person logged in: its place in the order of the windows kept
    /// (0 the back-most). It takes its desktop and its minimized state too, and the order among
    /// the others opened at login.
    pub at_login: Option<u32>,
}

#[derive(Debug, Default)]
pub(super) struct Places {
    /// Toplevels made and not offered yet, in order: offered once their client is known.
    new: Vec<SurfaceKey>,
    /// Windows placed at login, by their place in the order kept: those not shown yet and those
    /// shown, so each arriving one goes behind the ones kept in front of it.
    at_login: BTreeMap<SurfaceKey, u32>,
}

impl Scene {
    /// A toplevel was made: it is offered to the program to place once its client is known.
    pub(super) fn window_made(&mut self, key: SurfaceKey) {
        self.places.new.push(key);
    }

    pub(super) fn forget_place(&mut self, key: SurfaceKey) {
        self.places.new.retain(|new| *new != key);
        self.places.at_login.remove(&key);
    }

    /// The windows of launched applications not shown yet and never offered, each with its
    /// application, in the order they were made; each is offered once. One whose client is not
    /// known yet waits for it; one shown before is never offered.
    pub fn take_windows_to_place(&mut self) -> Vec<(SurfaceKey, String)> {
        let mut offered = Vec::new();
        let mut waiting = Vec::new();
        for key in std::mem::take(&mut self.places.new) {
            let Some(node) = self.live(key) else {
                continue;
            };
            if node.mapped || node.role != Role::Toplevel {
                continue;
            }
            match self.application(key.client) {
                Some(application) => offered.push((key, application.to_owned())),
                None => waiting.push(key),
            }
        }
        self.places.new = waiting;
        offered
    }

    /// Every window shown of an application the session launched, back to front.
    pub fn launched_windows(&self) -> Vec<LaunchedWindow> {
        self.roots
            .iter()
            .filter_map(|key| {
                let node = self.live(*key)?;
                if node.role != Role::Toplevel || !node.mapped {
                    return None;
                }
                let application = self.application(key.client)?.to_owned();
                // Zoomed, the frame it goes back to; full screen, the one it had before (zoomed,
                // that one's).
                let back = if node.maximized {
                    node.unzoomed
                } else if node.fullscreen {
                    node.restore
                } else {
                    None
                };
                let (origin, size) = back.unwrap_or((node.origin, node.size));
                Some(LaunchedWindow {
                    key: *key,
                    application,
                    place: WindowPlace {
                        origin,
                        size,
                        zoomed: node.maximized,
                        desktop: node.workspace,
                        minimized: node.hidden && !self.applications_hidden(*key),
                    },
                })
            })
            .collect()
    }

    /// Puts a window not shown yet where `placing` says, before its first frame. False for one
    /// that is gone, shown already, not a toplevel or full screen.
    pub fn place_window(&mut self, key: SurfaceKey, placing: Placing) -> bool {
        let Some(node) = self.live(key) else {
            return false;
        };
        if node.role != Role::Toplevel || node.mapped || node.fullscreen {
            return false;
        }
        let place = placing.place;
        let wanted = Rect::new(
            place.origin.0,
            place.origin.1,
            place.size.width,
            place.size.height,
        );
        // The display most of it was on, if one still shows any of it.
        let shown_on = self
            .outputs
            .iter()
            .filter_map(|output| {
                let shared = overlap(output.rect, wanted);
                (shared > 0).then_some((shared, output.id))
            })
            .max_by_key(|(shared, _)| *shared)
            .map(|(_, id)| id);
        let output = shown_on.unwrap_or(node.output);
        let limits = node.limits;
        let Some(area) = self.work_area(output) else {
            return false;
        };
        let size = limits.clamp(place.size);
        let size = Size::new(size.width.min(area.width), size.height.min(area.height));
        let origin = match shown_on {
            Some(_) => kept_in(area, place.origin, size),
            // Its display is gone: where a new window of its size opens on this one.
            None => self.placed(size, output),
        };
        let node = self.nodes.get_mut(&key).expect("live");
        node.output = output;
        if let Some(rank) = placing.at_login {
            if place.desktop < self.workspaces.count {
                node.workspace = place.desktop;
            }
            node.hidden = place.minimized;
            self.places.at_login.insert(key, rank);
        }
        self.reconfigure(key, Some(origin), size);
        if place.zoomed {
            if let Some((zoomed_origin, zoomed_size)) = self.zoomed_frame(key) {
                let node = self.nodes.get_mut(&key).expect("live");
                node.maximized = true;
                node.unzoomed = Some((origin, size));
                self.reconfigure(key, Some(zoomed_origin), zoomed_size);
            }
        }
        true
    }

    /// A window placed at login arrives: it goes into the order kept among the others opened at
    /// login (behind the back-most of those that were in front of it, else in front of the
    /// front-most of those behind it), whatever their desktops. True when the usual activation is
    /// not for it: it is minimized, on another desktop, or one in front of it is shown here.
    pub(super) fn arrive_in_order(&mut self, key: SurfaceKey) -> bool {
        let Some(&rank) = self.places.at_login.get(&key) else {
            return false;
        };
        let layer: Layer = layer_of(&self.nodes[&key].role);
        let kept = |scene: &Scene, other: SurfaceKey| {
            (other != key)
                .then(|| scene.places.at_login.get(&other).copied())
                .flatten()
                .filter(|_| {
                    scene
                        .live(other)
                        .is_some_and(|node| node.mapped && layer_of(&node.role) == layer)
                })
        };
        let behind = self
            .roots
            .iter()
            .position(|other| kept(self, *other).is_some_and(|other| other > rank));
        let before = self
            .roots
            .iter()
            .rposition(|other| kept(self, *other).is_some_and(|other| other < rank));
        let at = self.roots.iter().position(|root| *root == key);
        if let Some(at) = at {
            let index = match (behind, before) {
                (Some(behind), _) => Some(if behind > at { behind - 1 } else { behind }),
                (None, Some(before)) if before > at => Some(before),
                (None, Some(before)) => Some(before + 1),
                (None, None) => None,
            };
            if let Some(index) = index {
                self.roots.remove(at);
                self.roots.insert(index, key);
            }
        }
        let node = &self.nodes[&key];
        if node.hidden || node.workspace != self.workspaces.active {
            return true;
        }
        let active = self.workspaces.active;
        self.roots.iter().any(|other| {
            kept(self, *other).is_some_and(|other_rank| other_rank > rank)
                && self
                    .live(*other)
                    .is_some_and(|node| !node.hidden && node.workspace == active)
        })
    }
}

/// How much of `b` is on `a`, in square logical pixels.
fn overlap(a: Rect, b: Rect) -> u64 {
    let width = (a.right().min(b.right()) - i64::from(a.x).max(i64::from(b.x))).max(0);
    let height = (a.bottom().min(b.bottom()) - i64::from(a.y).max(i64::from(b.y))).max(0);
    (width as u64) * (height as u64)
}

/// `origin` moved as little as keeps a window of `size` in `area`: its top edge (where its title
/// is) inside it, never under a panel, and at least [`MIN_VISIBLE`] of it across and down, as a
/// window moved by hand is kept.
fn kept_in(area: Rect, origin: (i32, i32), size: Size) -> (i32, i32) {
    let x = i64::from(origin.0).clamp(
        i64::from(area.x) - i64::from(size.width) + MIN_VISIBLE.min(i64::from(size.width)),
        area.right() - MIN_VISIBLE.min(i64::from(size.width)),
    );
    let y = i64::from(origin.1).clamp(
        i64::from(area.y),
        (area.bottom() - MIN_VISIBLE.min(i64::from(size.height))).max(i64::from(area.y)),
    );
    (x as i32, y as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_is_kept_inside_the_work_area_by_as_little_as_needed() {
        let area = Rect::new(0, 30, 1280, 690);
        let size = Size::new(600, 400);
        // Inside: where it was.
        assert_eq!(kept_in(area, (100, 200), size), (100, 200));
        // Under the top bar: brought down to the work area's top.
        assert_eq!(kept_in(area, (100, 0), size), (100, 30));
        // Off the right: kept with 64 px on the display.
        assert_eq!(kept_in(area, (2000, 200), size), (1216, 200));
        // Off the left: the same.
        assert_eq!(kept_in(area, (-900, 200), size), (-536, 200));
        // Below: its top 64 px above the work area's bottom.
        assert_eq!(kept_in(area, (100, 900), size), (100, 656));
    }

    #[test]
    fn overlap_is_the_shared_area() {
        let output = Rect::new(0, 0, 1280, 720);
        assert_eq!(overlap(output, Rect::new(1180, 620, 200, 200)), 100 * 100);
        assert_eq!(overlap(output, Rect::new(1280, 0, 200, 200)), 0);
        assert_eq!(overlap(output, Rect::new(-50, -50, 100, 100)), 50 * 50);
    }
}
