//! The system shortcuts (docs/adr/0024-system-shortcuts.mdx): Super is OxyOS's ⌘ for the system,
//! as Ctrl is the applications' (their chords, ADR 0021), so Super and every key pressed while it
//! is held are the system's: no application or input method ever receives them, press or release.
//! Keys are matched by their place on the keyboard (Linux key codes, the US layout's names), as
//! Bloom matches applications' chords, so the two can never meet on any layout.
//!
//! The compositor acts on windows itself, so those work whatever the shell is doing:
//! Super+W closes the active window (its close request), Super+Q quits its application (a close
//! request to each of its windows), Super+M minimizes it, Super+H hides its application,
//! Super+` (Shift: backwards) moves to its application's next window and Super+Ctrl+F toggles
//! full screen. What needs a picture is the shell's (its `shortcuts` grant): Super+Tab and
//! Super+Shift+Tab open the application switcher and move in it while Super is held (Q quits and
//! H hides the application it has chosen, Escape cancels it, releasing Super activates the
//! choice), and Super+Space opens the launcher's search. While the session is locked none acts
//! and their keys reach nobody. The screen reader's keys come first (ADR 0014); the keymap's own
//! (a terminal switch, the lock's Ctrl+Super+Q, the reader's Super+F5) never reach the scene as
//! keys.
//!
//! An application is the windows the shell sees as one: those of a process the session started
//! for a registry entry, or claiming the same id, or, claiming none, its client's (ADR 0013).
//! Each window counts how recently it was active, which the shell orders the switcher by, and
//! a window hidden with its application stays off the screen, out of the dock, until its
//! application is activated again.

use super::{appearances, ClientId, OutputId, Role, Scene, SurfaceKey};
use oxyos_protocol::{DismissReason, Event, ShellKind, SystemShortcut};
use std::collections::{BTreeMap, BTreeSet};

const LEFT_SUPER: u32 = 125;
const RIGHT_SUPER: u32 = 126;
const LEFT_CTRL: u32 = 29;
const RIGHT_CTRL: u32 = 97;
const LEFT_SHIFT: u32 = 42;
const RIGHT_SHIFT: u32 = 54;
const LEFT_ALT: u32 = 56;
const RIGHT_ALT: u32 = 100;
const ESCAPE: u32 = 1;
const TAB: u32 = 15;
const Q: u32 = 16;
const W: u32 = 17;
const F: u32 = 33;
const H: u32 = 35;
/// The key above Tab, left of 1: ` on the US layout, º on the Spanish one.
const GRAVE: u32 = 41;
const M: u32 = 50;
const SPACE: u32 = 57;
const LEFT: u32 = 105;
const RIGHT: u32 = 106;

fn is_super(key: u32) -> bool {
    matches!(key, LEFT_SUPER | RIGHT_SUPER)
}

fn is_modifier(key: u32) -> bool {
    is_super(key)
        || matches!(
            key,
            LEFT_CTRL | RIGHT_CTRL | LEFT_SHIFT | RIGHT_SHIFT | LEFT_ALT | RIGHT_ALT
        )
}

#[derive(Debug, Default)]
pub(super) struct SystemKeys {
    /// The modifiers held, by their codes.
    held: BTreeSet<u32>,
    /// Keys whose press was the system's: their release is too.
    taken: BTreeSet<u32>,
    /// The application switcher, from Super+Tab until Super is released: the shell showing it
    /// and its output.
    switcher: Option<(ClientId, OutputId)>,
}

impl SystemKeys {
    fn holds(&self, keys: [u32; 2]) -> bool {
        keys.iter().any(|key| self.held.contains(key))
    }
}

#[derive(Debug, Default)]
pub(super) struct Applications {
    /// Activations so far.
    uses: u32,
    /// Each window's latest activation.
    used: BTreeMap<SurfaceKey, u32>,
    /// Windows hidden with their application.
    hidden: BTreeSet<SurfaceKey>,
}

/// A window action of the compositor's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowAction {
    Close,
    Quit,
    Minimize,
    Hide,
    NextWindow,
    PreviousWindow,
    FullScreen,
}

impl Scene {
    /// Whether a key from the keyboard is the system's (Super, or a key pressed while it is
    /// held): then nobody else sees it, and its press acted if it is a shortcut.
    pub(super) fn system_key(&mut self, key: u32, pressed: bool) -> bool {
        if is_modifier(key) {
            if pressed {
                self.system_keys.held.insert(key);
            } else {
                self.system_keys.held.remove(&key);
            }
        }
        if is_super(key) {
            if !pressed && !self.system_keys.holds([LEFT_SUPER, RIGHT_SUPER]) {
                // Super released: the switcher activates what it chose.
                if let Some((client, output)) = self.system_keys.switcher.take() {
                    self.tell_shortcut(client, output, SystemShortcut::SwitcherDone);
                }
            }
            return true;
        }
        if !pressed {
            return self.system_keys.taken.remove(&key);
        }
        if !self.system_keys.holds([LEFT_SUPER, RIGHT_SUPER]) {
            return false;
        }
        self.system_keys.taken.insert(key);
        if !is_modifier(key) && !self.locked {
            self.system_shortcut(key);
        }
        true
    }

    /// Whether Shift and Alt are held: a hardware key moves a quarter of a step (ADR 0025).
    pub(super) fn fine_step(&self) -> bool {
        let keys = &self.system_keys;
        keys.holds([LEFT_SHIFT, RIGHT_SHIFT]) && keys.holds([LEFT_ALT, RIGHT_ALT])
    }

    /// A key pressed with Super held, while unlocked.
    fn system_shortcut(&mut self, key: u32) {
        let keys = &self.system_keys;
        let ctrl = keys.holds([LEFT_CTRL, RIGHT_CTRL]);
        let shift = keys.holds([LEFT_SHIFT, RIGHT_SHIFT]);
        let alt = keys.holds([LEFT_ALT, RIGHT_ALT]);
        if let Some((client, output)) = keys.switcher {
            let shortcut = match key {
                TAB if shift => SystemShortcut::SwitcherPrevious,
                TAB | RIGHT => SystemShortcut::SwitcherNext,
                GRAVE if shift => SystemShortcut::SwitcherNext,
                GRAVE | LEFT => SystemShortcut::SwitcherPrevious,
                Q if !ctrl && !alt => SystemShortcut::SwitcherQuit,
                H if !ctrl && !alt => SystemShortcut::SwitcherHide,
                ESCAPE => {
                    self.system_keys.switcher = None;
                    SystemShortcut::SwitcherCancel
                }
                _ => return,
            };
            self.tell_shortcut(client, output, shortcut);
            return;
        }
        if alt {
            return;
        }
        let action = match (key, ctrl, shift) {
            (TAB, false, _) => {
                self.dismiss_grab(DismissReason::OutsidePress);
                let output = self.shortcut_output();
                if let Some(client) = self.shortcut_shell(output) {
                    self.system_keys.switcher = Some((client, output));
                    let first = if shift {
                        SystemShortcut::SwitcherPrevious
                    } else {
                        SystemShortcut::SwitcherNext
                    };
                    self.tell_shortcut(client, output, first);
                }
                return;
            }
            (SPACE, false, false) => {
                self.dismiss_grab(DismissReason::OutsidePress);
                let output = self.shortcut_output();
                if let Some(client) = self.shortcut_shell(output) {
                    self.tell_shortcut(client, output, SystemShortcut::Search);
                }
                return;
            }
            (W, false, false) => WindowAction::Close,
            (Q, false, false) => WindowAction::Quit,
            (M, false, false) => WindowAction::Minimize,
            (H, false, false) => WindowAction::Hide,
            (GRAVE, false, false) => WindowAction::NextWindow,
            (GRAVE, false, true) => WindowAction::PreviousWindow,
            (F, true, false) => WindowAction::FullScreen,
            _ => return,
        };
        // With no active window, nothing to act on.
        let Some(active) = self.active.filter(|active| self.live(*active).is_some()) else {
            return;
        };
        self.dismiss_grab(DismissReason::OutsidePress);
        match action {
            WindowAction::Close => {
                self.emit(active.client, Event::CloseRequested { surface: active.id })
            }
            WindowAction::Quit => self.quit_application(active),
            WindowAction::Minimize => self.minimize(active),
            WindowAction::Hide => self.hide_application(active),
            WindowAction::NextWindow => self.cycle_windows(active, false),
            WindowAction::PreviousWindow => self.cycle_windows(active, true),
            WindowAction::FullScreen => {
                let fullscreen = self.nodes[&active].fullscreen;
                // A window on no output stays as it is.
                let _ = self.set_fullscreen(active, !fullscreen);
            }
        }
    }

    /// Where the shell shows a shortcut: the active window's output, else the pointer's.
    pub(super) fn shortcut_output(&self) -> OutputId {
        self.active
            .and_then(|active| self.live(active).map(|node| node.output))
            .or_else(|| {
                self.output_at(i64::from(self.pointer.0), i64::from(self.pointer.1))
                    .map(|output| output.id)
            })
            .or_else(|| self.outputs.first().map(|output| output.id))
            .unwrap_or(0)
    }

    /// The shell client granted `shortcuts` that shows `output` (it has a shell surface there),
    /// else any that has a shell surface.
    pub(super) fn shortcut_shell(&self, output: OutputId) -> Option<ClientId> {
        let shells: Vec<(ClientId, OutputId)> = self
            .nodes
            .iter()
            .filter(|(key, node)| {
                !node.dismissed
                    && self
                        .clients
                        .get(&key.client)
                        .is_some_and(|grants| grants.shell && grants.shortcuts)
            })
            .filter_map(|(key, node)| match node.role {
                Role::Shell { output, .. } => Some((key.client, output)),
                _ => None,
            })
            .collect();
        shells
            .iter()
            .find(|(_, on)| *on == output)
            .or(shells.first())
            .map(|(client, _)| *client)
    }

    /// Sends a shortcut to a shell: its serial is the shell's input, as a key press is.
    fn tell_shortcut(&mut self, client: ClientId, output: OutputId, shortcut: SystemShortcut) {
        if !self.clients.contains_key(&client) {
            return;
        }
        let serial = self.next_serial();
        self.last_user_serial = serial;
        let inputs = self.inputs.entry(client).or_default();
        inputs.push_back(serial);
        if inputs.len() > super::RECENT_INPUTS {
            inputs.pop_front();
        }
        self.activation.insert(client, serial);
        self.emit(
            client,
            Event::SystemShortcut {
                shortcut,
                serial,
                output,
            },
        );
    }

    /// The switcher ends without a choice: the session locked.
    pub(super) fn cancel_switcher(&mut self) {
        if let Some((client, output)) = self.system_keys.switcher.take() {
            self.tell_shortcut(client, output, SystemShortcut::SwitcherCancel);
        }
    }

    /// A client gone, or no longer the shell: the switcher it showed is no more.
    pub(super) fn forget_switcher(&mut self, client: ClientId) {
        if self
            .system_keys
            .switcher
            .is_some_and(|(shell, _)| shell == client)
        {
            self.system_keys.switcher = None;
        }
    }

    /// Whether the application switcher is open (Super+Tab, Super still held).
    pub fn switching(&self) -> bool {
        self.system_keys.switcher.is_some()
    }

    // ---- applications ---------------------------------------------------------------------

    /// A window became the active one.
    pub(super) fn used(&mut self, key: SurfaceKey) {
        let applications = &mut self.applications;
        applications.uses += 1;
        applications.used.insert(key, applications.uses);
    }

    /// How recently a window was active (a rising count; 0: never), and whether it is hidden
    /// with its application.
    pub(super) fn window_use(&self, key: SurfaceKey) -> (u32, bool) {
        (
            self.applications.used.get(&key).copied().unwrap_or(0),
            self.applications.hidden.contains(&key),
        )
    }

    pub(super) fn forget_application_window(&mut self, key: SurfaceKey) {
        self.applications.used.remove(&key);
        self.applications.hidden.remove(&key);
    }

    /// Every window of `key`'s application, on any workspace, shown or not.
    pub(super) fn application_windows(&self, key: SurfaceKey) -> Vec<SurfaceKey> {
        let of = self.application_identity(key);
        self.nodes
            .iter()
            .filter(|(window, node)| {
                node.role == Role::Toplevel
                    && !node.dismissed
                    && node.mapped
                    && self.window_number(**window).is_some()
                    && self.application_identity(**window) == of
            })
            .map(|(window, _)| *window)
            .collect()
    }

    /// Asks each window of `key`'s application to close; it may refuse.
    pub(super) fn quit_application(&mut self, key: SurfaceKey) {
        for window in self.application_windows(key) {
            self.emit(window.client, Event::CloseRequested { surface: window.id });
        }
    }

    /// Hides `key`'s application: its windows shown leave the screen (not into the dock) until
    /// it is activated again. Its minimized windows stay in the dock.
    pub(super) fn hide_application(&mut self, key: SurfaceKey) {
        if self.locked {
            return;
        }
        for window in self.application_windows(key) {
            if !self.nodes[&window].hidden {
                self.applications.hidden.insert(window);
                self.conceal(window, appearances::Change::Gone);
            }
        }
    }

    /// Whether `key` is hidden with its application.
    pub(super) fn applications_hidden(&self, key: SurfaceKey) -> bool {
        self.applications.hidden.contains(&key)
    }

    /// `key`'s application's windows hidden with it, which come back with it.
    pub(super) fn reveal_application(&mut self, key: SurfaceKey) {
        if !self.applications.hidden.contains(&key) {
            return;
        }
        for window in self.application_windows(key) {
            if self.applications.hidden.remove(&window) {
                let node = self
                    .nodes
                    .get_mut(&window)
                    .expect("an application's window");
                node.hidden = false;
                if node.mapped {
                    self.appeared(window, appearances::Change::Arrived);
                }
            }
        }
    }

    /// Super+`: the active application's next window shown on this workspace becomes the active
    /// one, the active one going behind its others, so pressing again goes through them all, as
    /// a Mac's ⌘` does; backwards, its window furthest behind comes to the front.
    fn cycle_windows(&mut self, active: SurfaceKey, backwards: bool) {
        let application = self.application_windows(active);
        let windows: Vec<SurfaceKey> = self
            .roots
            .iter()
            .rev()
            .copied()
            .filter(|root| application.contains(root) && self.visible(*root))
            .collect();
        if windows.len() < 2 {
            return;
        }
        if backwards {
            self.activate(*windows.last().expect("two or more"));
            return;
        }
        let front = windows[0];
        let back = *windows.last().expect("two or more");
        self.activate(windows[1]);
        self.roots.retain(|root| *root != front);
        let below = self
            .roots
            .iter()
            .position(|root| *root == back)
            .unwrap_or(0);
        self.roots.insert(below, front);
        self.refresh_pointer_focus();
    }

    /// A shell asks for the keyboard focus on one of its own panels or overlays, citing an input
    /// it received lately; anything else is ignored.
    pub(super) fn focus_shell_surface(&mut self, key: SurfaceKey, serial: u64) -> bool {
        let Some(node) = self.live(key) else {
            return false;
        };
        if !matches!(
            node.role,
            Role::Shell {
                kind: ShellKind::Panel | ShellKind::Overlay | ShellKind::Notification,
                ..
            }
        ) {
            return false;
        }
        let recent = self
            .inputs
            .get(&key.client)
            .is_some_and(|inputs| inputs.contains(&serial));
        if recent && self.focusable(key) && self.grab.is_empty() {
            self.activate(key);
        }
        true
    }
}
