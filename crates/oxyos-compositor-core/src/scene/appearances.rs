//! Surfaces arriving on screen and going from it (docs/adr/0011-transitions.mdx), as the
//! compositor animates them: a surface's first frame, a window restored, and a surface
//! destroyed, dismissed, minimized or (a lock surface) unlocked. A surface hidden with its
//! workspace or under the lock neither arrives nor goes: it is hidden, not gone.

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
    /// The shell's panels and background, and its notifications.
    Panel,
    Lock,
    /// The input method's candidates: never animated.
    Candidates,
}

/// A surface arriving (`arrived`) or going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appearance {
    pub key: SurfaceKey,
    pub kind: Kind,
    pub arrived: bool,
}

pub(super) fn kind(role: &Role) -> Kind {
    match role {
        Role::Toplevel | Role::Dialog => Kind::Window,
        Role::Popup(_) => Kind::Popup,
        Role::Shell {
            kind: ShellKind::Overlay,
            ..
        } => Kind::Overlay,
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

    pub(super) fn appeared(&mut self, key: SurfaceKey, arrived: bool) {
        let Some(node) = self.nodes.get(&key) else {
            return;
        };
        let kind = kind(&node.role);
        // A scene nothing composes (a probe, a test) keeps only the latest.
        if self.appearances.len() >= MAX_APPEARANCES {
            self.appearances.remove(0);
        }
        self.appearances.push(Appearance { key, kind, arrived });
    }
}
