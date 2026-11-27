//! Automatically hiding and showing the dock (docs/adr/0034-the-dock.mdx, 10 November 2026), as a
//! Mac's *Automatically hide and show the Dock* and its ⌥⌘D (Super+Alt+D here). The preference is
//! the program's (`dock_autohide`, which it keeps and applies with [`Scene::set_dock_hiding`]); the
//! hiding is the window server's, as a Mac's Dock is moved by its window server: the shell draws
//! its dock as always and knows nothing of it.
//!
//! While it is on, the shell's bottom panel (its dock) keeps no strip of its output out of where
//! windows go (`work_area`), and is off the screen but where the person wants it: the pointer
//! reaching the output's bottom edge brings it up, and it stays while the pointer is over it, while
//! it has the keyboard (Super+F3, docs/adr/0039-keyboard-access.mdx), while a menu or tooltip of it
//! is open, and for a moment after a window went into it (the genie's half second and a little
//! more), and slides down again once none holds. Hidden, it is not drawn and takes no input, as a
//! window minimized; it slides in and out on its own transition (`Change::EdgeShown`,
//! `Change::EdgeHidden`). Locked, nothing changes beneath the lock.

use super::{appearances, Role, Scene, SurfaceKey};
use oxyos_protocol::{PanelEdge, ShellKind};
use std::time::Duration;

/// How long the dock stays up after a window went into it while it hides: the genie's half second
/// (docs/adr/0043-the-genie.mdx) and as long again, so the window is seen arriving in it.
const MINIMIZE_HOLD: Duration = Duration::from_millis(1000);

#[derive(Debug, Default)]
pub(super) struct DockHiding {
    /// The person's preference, as the program applied it.
    on: bool,
    /// Until when a window minimized into it holds it up (the scene's clock).
    hold_until: Option<Duration>,
    /// Super+Alt+D pressed and not yet taken by the program.
    toggles: u32,
}

impl Scene {
    /// The person's *Automatically hide and show the Dock*, as the program keeps it: on, the dock
    /// keeps no strip of the screen and goes off it until it is wanted; off, it is shown and
    /// keeps its strip again.
    pub fn set_dock_hiding(&mut self, on: bool) {
        if self.dock_hiding.on == on {
            return;
        }
        self.dock_hiding.on = on;
        self.follow_dock_hiding();
    }

    /// Whether the dock hides automatically.
    pub fn dock_hiding(&self) -> bool {
        self.dock_hiding.on
    }

    /// Super+Alt+D: the program turns the preference on or off, keeps it and applies it.
    pub(super) fn toggle_dock_hiding(&mut self) {
        self.dock_hiding.toggles = self.dock_hiding.toggles.saturating_add(1);
    }

    /// How many times Super+Alt+D was pressed since the last call: each turns the preference
    /// over once.
    pub fn take_dock_hiding_toggles(&mut self) -> u32 {
        std::mem::take(&mut self.dock_hiding.toggles)
    }

    /// When the dock held up by a window minimized into it may go down: a turn should follow the
    /// hiding again then ([`Scene::follow_dock_hiding`]).
    pub fn next_dock_deadline(&self) -> Option<Duration> {
        self.dock_hiding
            .hold_until
            .filter(|_| self.dock_hiding.on)
            .map(|until| until.saturating_sub(self.clock))
    }

    /// Whether `key`, the shell's dock, is off the screen now.
    pub fn dock_hidden(&self, key: SurfaceKey) -> bool {
        self.dock_panels().contains(&key) && self.nodes[&key].hidden
    }

    /// A window went into the dock: it comes up for the genie, and stays a moment.
    pub(super) fn hold_dock(&mut self) {
        if self.dock_hiding.on {
            self.dock_hiding.hold_until = Some(self.clock + MINIMIZE_HOLD);
            self.follow_dock_hiding();
        }
    }

    /// `key`, a dock that is hidden, comes up for the keyboard the shell asked it to take
    /// (Super+F3): it stays up while it has it.
    pub(super) fn bring_up_dock(&mut self, key: SurfaceKey) {
        if self.locked || !self.dock_panels().contains(&key) || !self.nodes[&key].hidden {
            return;
        }
        self.nodes.get_mut(&key).expect("a dock").hidden = false;
        self.appeared(key, appearances::Change::EdgeShown);
    }

    /// The shell's docks: its bottom panels, shown on their outputs.
    fn dock_panels(&self) -> Vec<SurfaceKey> {
        self.nodes
            .iter()
            .filter(|(_, node)| {
                node.mapped
                    && !node.dismissed
                    && matches!(
                        node.role,
                        Role::Shell {
                            kind: ShellKind::Panel,
                            edge: PanelEdge::Bottom,
                            ..
                        }
                    )
            })
            .map(|(key, _)| *key)
            .collect()
    }

    /// Whether `dock` is wanted on the screen now: the pointer at its output's bottom edge, or
    /// over it while it is up; the keyboard in it; a popup of it open; a window just minimized
    /// into it.
    fn dock_wanted(&self, dock: SurfaceKey) -> bool {
        let node = &self.nodes[&dock];
        let (x, y) = (i64::from(self.pointer.0), i64::from(self.pointer.1));
        let pointer = match (node.hidden, self.output(node.output)) {
            // Hidden: its output's last row of pixels, as a Mac's Dock comes up.
            (true, Some(output)) => {
                let rect = output.rect;
                x >= i64::from(rect.x)
                    && x < rect.right()
                    && y >= rect.bottom() - 1
                    && y < rect.bottom()
            }
            (false, _) => self.global_rect(dock).is_some_and(|rect| {
                x >= i64::from(rect.x)
                    && x < rect.right()
                    && y >= i64::from(rect.y)
                    && y < rect.bottom()
            }),
            (true, None) => false,
        };
        let keyboard = self.focus.is_some_and(|focus| self.root_of(focus) == dock);
        let popup = self.nodes.iter().any(|(key, other)| {
            matches!(other.role, Role::Popup(_))
                && other.mapped
                && !other.dismissed
                && self.root_of(*key) == dock
        });
        let held = self
            .dock_hiding
            .hold_until
            .is_some_and(|until| self.clock < until);
        pointer || keyboard || popup || held
    }

    /// Puts each dock where the preference and the person want it now: up or down, sliding.
    /// Followed as the pointer moves, and once a turn for the rest (the keyboard, its popups,
    /// the moment after a window went into it).
    pub fn follow_dock_hiding(&mut self) {
        if self.locked {
            return;
        }
        if self
            .dock_hiding
            .hold_until
            .is_some_and(|until| self.clock >= until)
        {
            self.dock_hiding.hold_until = None;
        }
        let mut moved = false;
        for dock in self.dock_panels() {
            let shown = !self.dock_hiding.on || self.dock_wanted(dock);
            let node = self.nodes.get_mut(&dock).expect("a dock");
            if node.hidden != shown {
                continue;
            }
            node.hidden = !shown;
            moved = true;
            if shown {
                self.appeared(dock, appearances::Change::EdgeShown);
            } else {
                self.appeared(dock, appearances::Change::EdgeHidden);
                self.withdraw_input(dock);
            }
        }
        if moved {
            self.refresh_pointer_focus();
        }
    }
}
