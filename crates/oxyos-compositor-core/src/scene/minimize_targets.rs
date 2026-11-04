//! Where each window minimizes into (docs/adr/0011-transitions.mdx, the genie): the session's shell
//! says, on one of its shell surfaces (the dock), the rectangle a window goes into when it is
//! minimized and comes back out of when it is restored, its application's icon, as a Mac's Dock
//! tells the window server where its genie ends. Wayland's foreign-toplevel `set_rectangle` and
//! Plasma's minimized geometry are the same thing. The shell names a window by its number; the
//! compositor asks for the rectangle in the global space as the window minimizes.

use super::{ClientId, Request, Role, Scene, SurfaceKey};
use oxyos_protocol::{ErrorCode, Rect, MAX_MINIMIZE_TARGETS};

/// A shell surface's targets: the list being made since ClearMinimizeTargets, and the one held
/// since the surface's Submit after it (each a window's number and where, surface-local).
#[derive(Debug, Default)]
pub(super) struct SurfaceTargets {
    making: Option<Vec<(u32, Rect)>>,
    held: Vec<(u32, Rect)>,
}

impl Scene {
    /// ClearMinimizeTargets and PlaceMinimizeTarget: only the shell's, on its own shell surfaces.
    pub(super) fn minimize_target_request(
        &mut self,
        client: ClientId,
        request: Request,
    ) -> Result<(), ErrorCode> {
        if !self.clients.get(&client).is_some_and(|grants| grants.shell) {
            return Err(ErrorCode::Unauthorized);
        }
        let surface = match request {
            Request::ClearMinimizeTargets { surface }
            | Request::PlaceMinimizeTarget { surface, .. } => SurfaceKey {
                client,
                id: surface,
            },
            _ => unreachable!("only minimize targets are passed here"),
        };
        let node = self.live(surface).ok_or(ErrorCode::UnknownSurface)?;
        if !matches!(node.role, Role::Shell { .. }) {
            return Err(ErrorCode::InvalidRole);
        }
        let targets = self.minimize_targets.entry(surface).or_default();
        match request {
            Request::ClearMinimizeTargets { .. } => targets.making = Some(Vec::new()),
            Request::PlaceMinimizeTarget { window, rect, .. } => {
                let making = targets.making.as_mut().ok_or(ErrorCode::InvalidOrder)?;
                // A window placed again is where it was placed last.
                making.retain(|(placed, _)| *placed != window);
                if making.len() >= MAX_MINIMIZE_TARGETS {
                    return Err(ErrorCode::QuotaExceeded);
                }
                making.push((window, rect));
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// A frame of `surface` was submitted: the list made since ClearMinimizeTargets holds.
    pub(super) fn send_minimize_targets(&mut self, surface: SurfaceKey) {
        if let Some(targets) = self.minimize_targets.get_mut(&surface) {
            if let Some(making) = targets.making.take() {
                targets.held = making;
            }
        }
    }

    pub(super) fn forget_minimize_targets(&mut self, surface: SurfaceKey) {
        self.minimize_targets.remove(&surface);
    }

    /// Where `window` minimizes into and is restored out of, in the global space (logical
    /// pixels), as a shell surface shown says: none if no shell surface says, or the one that does
    /// is not shown (dismissed, not mapped) or the rectangle is empty.
    pub fn minimize_target(&self, window: SurfaceKey) -> Option<Rect> {
        let number = self.window_number(window)?;
        self.minimize_targets
            .iter()
            .filter(|(surface, _)| {
                self.nodes
                    .get(surface)
                    .is_some_and(|node| node.mapped && !node.dismissed)
            })
            .find_map(|(surface, targets)| {
                let &(_, rect) = targets.held.iter().find(|(placed, _)| *placed == number)?;
                if rect.width == 0 || rect.height == 0 {
                    return None;
                }
                let at = self.global_rect(*surface)?;
                Some(Rect::new(
                    at.x.saturating_add(rect.x),
                    at.y.saturating_add(rect.y),
                    rect.width,
                    rect.height,
                ))
            })
    }
}
