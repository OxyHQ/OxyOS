//! The windows a shell surface shows live (docs/adr/0022-window-views.mdx): the overview's cards
//! hold each window as the compositor draws it, scaled into the place the shell's layout gives
//! it, as a Mac's window server draws Mission Control's windows. The shell names a window by its
//! number and says where; the compositor draws the window's own frame there, over the surface's
//! pixels. The shell never receives a window's pixels, and nothing of them is drawn while the
//! session is locked (shell surfaces are not shown then).

use super::{ClientId, RenderItem, Request, Role, Scene, SurfaceKey};
use oxyos_protocol::{ErrorCode, Rect, Size, MAX_WINDOW_VIEWS};

/// One window shown on a shell surface, as the shell placed it (surface-local logical pixels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Placed {
    window: u32,
    rect: Rect,
    clip: Rect,
    radius: u32,
    opacity: u8,
}

/// A shell surface's windows: the list being made, the one sent with a frame, the one shown.
#[derive(Debug, Default)]
pub(super) struct SurfaceViews {
    /// Since ClearWindowViews: sent with the surface's next Submit.
    making: Option<Vec<Placed>>,
    /// Sent with the frame of this sequence: shown when it, or a later one, presents.
    sent: Option<(u64, Vec<Placed>)>,
    shown: Vec<Placed>,
}

/// The largest rectangle of `size`'s proportions within `rect`, centred in it (none if either
/// is empty).
pub fn fitted(rect: Rect, size: Size) -> Option<Rect> {
    if rect.width == 0 || rect.height == 0 || size.width == 0 || size.height == 0 {
        return None;
    }
    let (w, h) = (u64::from(rect.width), u64::from(rect.height));
    let (sw, sh) = (u64::from(size.width), u64::from(size.height));
    let (fw, fh) = if w * sh <= h * sw {
        (w, ((w * sh + sw / 2) / sw).max(1))
    } else {
        (((h * sw + sh / 2) / sh).max(1), h)
    };
    Some(Rect::new(
        rect.x + ((w - fw) / 2) as i32,
        rect.y + ((h - fh) / 2) as i32,
        fw as u32,
        fh as u32,
    ))
}

impl Scene {
    /// ClearWindowViews and PlaceWindowView: only the shell's, on its own shell surfaces.
    pub(super) fn window_view_request(
        &mut self,
        client: ClientId,
        request: Request,
    ) -> Result<(), ErrorCode> {
        if !self.clients.get(&client).is_some_and(|grants| grants.shell) {
            return Err(ErrorCode::Unauthorized);
        }
        let surface = match request {
            Request::ClearWindowViews { surface } | Request::PlaceWindowView { surface, .. } => {
                SurfaceKey {
                    client,
                    id: surface,
                }
            }
            _ => unreachable!("only window views are passed here"),
        };
        let node = self.live(surface).ok_or(ErrorCode::UnknownSurface)?;
        if !matches!(node.role, Role::Shell { .. }) {
            return Err(ErrorCode::InvalidRole);
        }
        let views = self.window_views.entry(surface).or_default();
        match request {
            Request::ClearWindowViews { .. } => views.making = Some(Vec::new()),
            Request::PlaceWindowView {
                window,
                rect,
                clip,
                radius,
                opacity,
                ..
            } => {
                let making = views.making.as_mut().ok_or(ErrorCode::InvalidOrder)?;
                if making.len() >= MAX_WINDOW_VIEWS {
                    return Err(ErrorCode::QuotaExceeded);
                }
                making.push(Placed {
                    window,
                    rect,
                    clip,
                    radius,
                    opacity,
                });
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// A frame of `surface` was submitted: the list made since ClearWindowViews goes with it.
    pub(super) fn send_window_views(&mut self, surface: SurfaceKey, sequence: u64) {
        if let Some(views) = self.window_views.get_mut(&surface) {
            if let Some(making) = views.making.take() {
                views.sent = Some((sequence, making));
            }
        }
    }

    /// A frame of `surface` began presenting: the list sent with it, or with an earlier one it
    /// superseded, is shown with it.
    pub(super) fn present_window_views(&mut self, surface: SurfaceKey, sequence: u64) {
        if let Some(views) = self.window_views.get_mut(&surface) {
            if views
                .sent
                .as_ref()
                .is_some_and(|(sent, _)| *sent <= sequence)
            {
                views.shown = views.sent.take().expect("checked").1;
            }
        }
    }

    pub(super) fn forget_window_views(&mut self, surface: SurfaceKey) {
        self.window_views.remove(&surface);
    }

    /// The windows `surface`, shown at `at` (its rectangle on the output), shows live, bottom to
    /// top: each a window still there with a frame, fitted to where the shell put it and cut to
    /// its clip and the surface. Called only for a surface being drawn, so never while locked.
    pub(super) fn push_window_views(
        &self,
        surface: SurfaceKey,
        at: Rect,
        out: &mut Vec<RenderItem>,
        in_flight: bool,
    ) {
        let Some(views) = self.window_views.get(&surface) else {
            return;
        };
        for placed in &views.shown {
            let Some(window) = self.window_by_number(placed.window) else {
                continue;
            };
            let node = &self.nodes[&window];
            if node.role != Role::Toplevel || !node.mapped || node.dismissed {
                continue;
            }
            let Some((frame, _, size)) = node.view(in_flight) else {
                continue;
            };
            let local = |rect: Rect| {
                Rect::new(
                    at.x.saturating_add(rect.x),
                    at.y.saturating_add(rect.y),
                    rect.width,
                    rect.height,
                )
            };
            let Some(rect) = fitted(local(placed.rect), size) else {
                continue;
            };
            let clip = super::intersect(&local(placed.clip), &at)
                .and_then(|clip| super::intersect(&clip, &rect));
            let Some(clip) = clip else {
                continue;
            };
            out.push(RenderItem::View {
                surface,
                window,
                frame,
                rect,
                clip,
                radius: placed.radius,
                opacity: placed.opacity,
            });
        }
    }
}
