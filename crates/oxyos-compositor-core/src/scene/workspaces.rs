//! Workspaces (ADR 0009): each toplevel window belongs to one, and only the active workspace's
//! windows are shown, take input and can have focus. A window opens on the active workspace.
//! Workspaces are numbered from 0, at least one, at most `MAX_WORKSPACES`; removing one moves its
//! windows to its neighbour.

use super::{DismissReason, Role, Scene, SurfaceKey};
use std::cmp::Ordering;
use std::collections::BTreeSet;

/// Workspaces a session may have.
pub const MAX_WORKSPACES: u32 = 16;

#[derive(Debug)]
pub(super) struct Workspaces {
    pub(super) count: u32,
    pub(super) active: u32,
    /// Workspaces removed since the program running the compositor last asked, in order: what it
    /// keeps for each (a desktop's wallpaper, docs/adr/0048-wallpapers.mdx) follows them.
    removed: Vec<u32>,
}

impl Default for Workspaces {
    fn default() -> Self {
        Self {
            count: 1,
            active: 0,
            removed: Vec::new(),
        }
    }
}

impl Scene {
    /// How many workspaces there are, and the active one.
    pub fn workspaces(&self) -> (u32, u32) {
        (self.workspaces.count, self.workspaces.active)
    }

    /// The workspaces removed since the last call, each numbered as it was when it went.
    pub fn take_removed_workspaces(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.workspaces.removed)
    }

    /// The output that holds most of the topmost window shown of a client with the `settings`
    /// grant (docs/adr/0048-wallpapers.mdx: a wallpaper is chosen for the display Settings is
    /// on), if it shows one.
    pub fn settings_output(&self) -> Option<super::OutputId> {
        self.roots.iter().rev().copied().find_map(|key| {
            let node = &self.nodes[&key];
            (node.role == Role::Toplevel
                && self.clients.get(&key.client).is_some_and(|g| g.settings)
                && self.visible(key))
            .then_some(node.output)
        })
    }

    /// The workspace of a window (a toplevel), if it is one.
    pub fn window_workspace(&self, key: SurfaceKey) -> Option<u32> {
        let node = self.live(key)?;
        (node.role == Role::Toplevel).then_some(node.workspace)
    }

    /// The windows shown now: they are the ones `apply` compares.
    fn shown_windows(&self) -> BTreeSet<SurfaceKey> {
        self.roots
            .iter()
            .copied()
            .filter(|key| self.nodes[key].role == Role::Toplevel && self.visible(*key))
            .collect()
    }

    /// After a change of workspaces: each window no longer shown loses what it held, as a
    /// minimized one does (its popups, the pointer, focus), and focus goes to the topmost window
    /// shown if it has none.
    fn apply(&mut self, before: BTreeSet<SurfaceKey>) {
        let now = self.shown_windows();
        for &gone in before.difference(&now) {
            self.dismiss_group_popups(gone, DismissReason::ParentHidden);
            self.withdraw_input(gone);
            if self.focus.is_some_and(|f| self.root_of(f) == gone) {
                self.set_focus(None);
            }
        }
        if self.focus.is_none() {
            self.focus_next();
        }
        // The active window left while the shell's top bar had focus: none is active.
        self.update_active();
        self.refresh_pointer_focus();
    }

    /// Shows workspace `index`: its windows, its topmost one focused. False for one that does not
    /// exist, and while locked.
    pub fn activate_workspace(&mut self, index: u32) -> bool {
        self.activate_workspace_as(index, true)
    }

    /// The same; `slide`: the windows slide out and in (ADR 0011), else the compositor moves them
    /// already, as a swipe across has brought the desktop in (docs/adr/0046-interactive-gestures.mdx).
    pub(super) fn activate_workspace_as(&mut self, index: u32, slide: bool) -> bool {
        if index >= self.workspaces.count || self.locked {
            return false;
        }
        if index == self.workspaces.active {
            return true;
        }
        let before = self.shown_windows();
        let forward = index > self.workspaces.active;
        self.workspaces.active = index;
        // The windows left slide out and those of the workspace shown slide in, as a Mac's
        // spaces do (ADR 0011); they are moved aside, not gone.
        let now = self.shown_windows();
        for &out in before.difference(&now).filter(|_| slide) {
            if self.nodes[&out].mapped {
                self.appeared(out, super::appearances::Change::WorkspaceOut { forward });
            }
        }
        for &into in now.difference(&before).filter(|_| slide) {
            if self.nodes[&into].mapped {
                self.appeared(into, super::appearances::Change::WorkspaceIn { forward });
            }
        }
        // The window that had focus is on the workspace left: the new one's topmost takes it.
        if let Some(focused) = self.focus.map(|f| self.root_of(f)) {
            if before.contains(&focused) {
                self.set_focus(None);
            }
        }
        self.apply(before);
        true
    }

    /// Moves a window (a toplevel) to workspace `index`. One moved away from the active workspace
    /// goes from the screen; one moved onto it appears, without taking focus.
    pub fn move_to_workspace(&mut self, key: SurfaceKey, index: u32) -> bool {
        if index >= self.workspaces.count || self.locked || self.window_workspace(key).is_none() {
            return false;
        }
        let before = self.shown_windows();
        self.nodes.get_mut(&key).expect("live").workspace = index;
        self.apply(before);
        true
    }

    /// Adds a workspace after the others; its number, or none at the limit.
    pub fn add_workspace(&mut self) -> Option<u32> {
        if self.workspaces.count >= MAX_WORKSPACES {
            return None;
        }
        self.workspaces.count += 1;
        Some(self.workspaces.count - 1)
    }

    /// Removes workspace `index`, the last one never: its windows go to its neighbour (the one
    /// before it, or after it for the first), and those after it are numbered down.
    pub fn remove_workspace(&mut self, index: u32) -> bool {
        if index >= self.workspaces.count || self.workspaces.count == 1 || self.locked {
            return false;
        }
        let before = self.shown_windows();
        // The neighbour's number once `index` is gone.
        let neighbour = index.saturating_sub(1);
        for node in self.nodes.values_mut() {
            match node.workspace.cmp(&index) {
                Ordering::Equal => node.workspace = neighbour,
                Ordering::Greater => node.workspace -= 1,
                Ordering::Less => {}
            }
        }
        self.workspaces.count -= 1;
        self.workspaces.removed.push(index);
        let active = self.workspaces.active;
        match active.cmp(&index) {
            Ordering::Equal => self.workspaces.active = neighbour,
            Ordering::Greater => self.workspaces.active -= 1,
            Ordering::Less => {}
        }
        self.apply(before);
        true
    }
}
