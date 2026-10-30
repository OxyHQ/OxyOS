//! Surfaces arriving on screen and going from it (docs/adr/0011-transitions.mdx), as the
//! compositor animates them: a surface's first frame, a window restored, and a surface
//! destroyed, dismissed, minimized or (a lock surface) unlocked; and the windows a switch of
//! workspace hides and shows, which are moved aside, not gone. A surface hidden under the lock
//! neither arrives nor goes.

use super::{Role, Scene, SurfaceKey};
use oxyos_protocol::ShellKind;

/// Appearances kept until the compositor takes them.
const MAX_APPEARANCES: usize = 256;

/// What a surface is, for its transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A toplevel or a dialog.
    Window,
    Popup,
    /// The shell's overlays (the launcher, the overview).
    Overlay,
    /// The shell's panels and background.
    Panel,
    /// The shell's notifications.
    Notification,
    Lock,
    /// The input method's candidates: never animated.
    Candidates,
}

/// How a surface came onto the screen or went from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Its first frame.
    Arrived,
    /// Destroyed, dismissed, its client gone, or (a lock surface) unlocked.
    Gone,
    /// A window minimized: it goes into the dock.
    Minimized,
    /// A window restored: it comes out of the dock.
    Restored,
    /// A window of the workspace switched to, coming in; `forward` when the workspace is a
    /// later one (it comes from the right).
    WorkspaceIn { forward: bool },
    /// A window of the workspace left, going out; `forward` as above (it goes to the left).
    WorkspaceOut { forward: bool },
    /// A window's frame changed at once (zoomed, restored from a zoom, maximized or tiled by the
    /// shell), not by hand (docs/adr/0016-windows.mdx): it stays on the screen, its frame
    /// animated from where it was drawn to where it is configured.
    Zoomed,
}

/// A surface arriving or going, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appearance {
    pub key: SurfaceKey,
    pub kind: Kind,
    pub change: Change,
}

impl Appearance {
    /// Whether it comes onto the screen (else it goes, or, zoomed, stays).
    pub fn arrived(&self) -> bool {
        matches!(
            self.change,
            Change::Arrived | Change::Restored | Change::WorkspaceIn { .. }
        )
    }
}

pub(super) fn kind(role: &Role) -> Kind {
    match role {
        Role::Toplevel | Role::Dialog => Kind::Window,
        Role::Popup(_) => Kind::Popup,
        Role::Shell {
            kind: ShellKind::Overlay,
            ..
        } => Kind::Overlay,
        Role::Shell {
            kind: ShellKind::Notification,
            ..
        } => Kind::Notification,
        Role::Shell { .. } => Kind::Panel,
        Role::Lock { .. } => Kind::Lock,
        Role::Candidates => Kind::Candidates,
    }
}

impl Scene {
    /// The surfaces that arrived or went since the last call, in order: what the compositor
    /// starts transitions for, before it composes the frame they first (or last) show in.
    pub fn take_appearances(&mut self) -> Vec<Appearance> {
        std::mem::take(&mut self.appearances)
    }

    /// Where a minimized window goes and a restored one comes from: the shell's bottom panel,
    /// its dock, as a Mac's Dock; none without one.
    pub fn dock(&self) -> Option<oxyos_protocol::Rect> {
        self.nodes
            .iter()
            .filter(|(_, node)| !node.dismissed && node.mapped)
            .find(|(_, node)| {
                matches!(
                    node.role,
                    Role::Shell {
                        kind: ShellKind::Panel,
                        edge: oxyos_protocol::PanelEdge::Bottom,
                        ..
                    }
                )
            })
            .and_then(|(key, _)| self.global_rect(*key))
    }

    pub(super) fn appeared(&mut self, key: SurfaceKey, change: Change) {
        let Some(node) = self.nodes.get(&key) else {
            return;
        };
        let kind = kind(&node.role);
        // A scene nothing composes (a probe, a test) keeps only the latest.
        if self.appearances.len() >= MAX_APPEARANCES {
            self.appearances.remove(0);
        }
        self.appearances.push(Appearance { key, kind, change });
    }
}
