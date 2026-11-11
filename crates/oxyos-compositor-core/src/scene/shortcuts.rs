//! The system shortcuts (docs/adr/0024-system-shortcuts.mdx): Super is OxyOS's ⌘ for the system,
//! as Ctrl is the applications' (their chords, ADR 0021), so Super and every key pressed while it
//! is held are the system's: no application or input method ever receives them, press or release.
//! Keys are matched by their place on the keyboard (Linux key codes, the US layout's names), as
//! Bloom matches applications' chords, so the two can never meet on any layout.
//!
//! The compositor acts on windows itself, so those work whatever the shell is doing:
//! Super+W closes the active window (its close request), Super+Q quits its application (a close
//! request to each of its windows), Super+M minimizes it, Super+H hides its application,
//! Super+` (Shift: backwards) moves to its application's next window, Super+Ctrl+F toggles
//! full screen and Super+Alt with F, C, R or an arrow (Shift: arrangements) tiles it (ADR 0016,
//! decision 12, `tiling.rs`); with Alt, as a Mac's ⌥⌘W and ⌥⌘M, Super+Alt+W asks every window of
//! the active application to close and Super+Alt+M minimizes every one shown on this workspace.
//! What needs a picture is the shell's (its `shortcuts` grant): Super+Tab and
//! Super+Shift+Tab open the application switcher and move in it while Super is held (Q quits and
//! H hides the application it has chosen, Escape cancels it, releasing Super activates the
//! choice), Super pressed and released alone opens the launcher's search (ADR 0041), Super+Space
//! opens Spotlight, as a Mac's ⌘Space, and Super+Return reveals what Spotlight has chosen where it
//! is, as its ⌘Return (docs/adr/0051-search.mdx), and Super+↑ and Super+↓ open Mission Control
//! and App Exposé (ADR 0044), as a touchpad's swipes up and down do (`swipes.rs`). Input sources
//! are the program's (ADR 0041, as ADR 0051 moved them): Super+Alt+Space chooses the one used
//! before (each further Space while Super and Alt are held, the one before that),
//! Super+Alt+Shift+Space the next in the list, Control+Super+Space and the emoji key emoji.
//! Screenshots are decision 10's
//! (`screenshots.rs`): Super+Shift+3 is the compositor's, Super+Shift+4 and 5 open the shell's
//! interface, whose keys are the system's while it is open. While the session is locked none acts
//! and their keys reach nobody. The screen reader's keys come first (ADR 0014); the keymap's own
//! (a terminal switch, the lock's Ctrl+Super+Q, the reader's Super+F5) never reach the scene as
//! keys.
//!
//! An application is the windows the shell sees as one: those of a process the session started
//! for a registry entry, or claiming the same id, or, claiming none, its client's (ADR 0013).
//! Each window counts how recently it was active, which the shell orders the switcher by, and
//! a window hidden with its application stays off the screen, out of the dock, until its
//! application is activated again.

use super::tiling::{Arrangement, Layout, Tile};
use super::{appearances, ClientId, OutputId, Role, Scene, SurfaceKey};
use oxyos_protocol::{
    CaptureMode, DismissReason, ErrorCode, Event, ShellKind, ShortcutIntent, SystemShortcut,
};
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
/// The digits' row: 3, 4 and 5 (screenshots, decision 10).
const KEY_3: u32 = 4;
const KEY_4: u32 = 5;
const KEY_5: u32 = 6;
const TAB: u32 = 15;
const Q: u32 = 16;
const W: u32 = 17;
/// R and C, with F: tiling's Return to Previous Size, Center and Fill (ADR 0016, decision 12).
const R: u32 = 19;
const F: u32 = 33;
const C: u32 = 46;
const H: u32 = 35;
/// The key above Tab, left of 1: ` on the US layout, º on the Spanish one.
const GRAVE: u32 = 41;
/// N: the Notification Center, as a Mac's Fn-N (ADR 0039).
const N: u32 = 49;
const M: u32 = 50;
const SPACE: u32 = 57;
/// Return: what Spotlight has chosen revealed where it is (docs/adr/0051-search.mdx).
const ENTER: u32 = 28;
/// The emoji key some keyboards have (Linux's KEY_EMOJI_PICKER).
const EMOJI_PICKER: u32 = 0x249;
const F2: u32 = 60;
const F3: u32 = 61;
const F8: u32 = 66;
const UP: u32 = 103;
const LEFT: u32 = 105;
const RIGHT: u32 = 106;
const DOWN: u32 = 108;

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
    /// Super went down and nothing else happened since (no other key, button or scroll): its
    /// release opens the launcher's search.
    super_alone: bool,
    /// Super+Alt+Space pressed since Super and Alt were last pressed or released (ADR 0041): which
    /// input source back it chooses.
    spaces: u32,
}

/// An input-source shortcut (ADR 0041), for the program running the compositor, which keeps the
/// input sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputSourceShortcut {
    /// Super+Alt+Space: the input source used `step` switches before the one current when Super
    /// and Alt went down (1, the one before it; 2, the one before that, and so on).
    Previous { step: u32 },
    /// Super+Alt+Shift+Space: the next in the list.
    Next,
    /// Control+Super+Space, or the emoji key: emoji, in the focused field.
    Emoji,
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
    /// Every window of the active application asked to close, as each by Super+W.
    CloseAll,
    Quit,
    Minimize,
    /// Every window of the active application shown on this workspace minimized.
    MinimizeAll,
    Hide,
    NextWindow,
    PreviousWindow,
    FullScreen,
}

/// The layout Super+Alt with `key` (and Shift) asks for (ADR 0016, decision 12).
fn tiling_layout(key: u32, shift: bool) -> Option<Layout> {
    let tile = |tile| Some(Layout::Tile(tile));
    let arrange = |arrangement| Some(Layout::Arrange(arrangement));
    match (key, shift) {
        (F, false) => tile(Tile::Fill),
        (C, false) => Some(Layout::Center),
        (R, false) => Some(Layout::ReturnToPreviousSize),
        (LEFT, false) => tile(Tile::Left),
        (RIGHT, false) => tile(Tile::Right),
        (UP, false) => tile(Tile::Top),
        (DOWN, false) => tile(Tile::Bottom),
        (LEFT, true) => arrange(Arrangement::LeftAndRight),
        (RIGHT, true) => arrange(Arrangement::RightAndLeft),
        (UP, true) => arrange(Arrangement::TopAndBottom),
        (DOWN, true) => arrange(Arrangement::BottomAndTop),
        _ => None,
    }
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
        // Super or Alt pressed or released: a hold of Super+Alt begins or ends, and Space goes back
        // from the input source current then.
        if is_super(key) || matches!(key, LEFT_ALT | RIGHT_ALT) {
            self.system_keys.spaces = 0;
        }
        if is_super(key) {
            let held = self.system_keys.holds([LEFT_SUPER, RIGHT_SUPER]);
            if pressed && held && self.system_keys.held.len() == 1 {
                // Super down alone, nothing else held.
                self.system_keys.super_alone = true;
            } else if pressed {
                self.system_keys.super_alone = false;
            }
            if !pressed && !held {
                // Super released: the switcher activates what it chose.
                if let Some((client, output)) = self.system_keys.switcher.take() {
                    self.tell_shortcut(client, output, SystemShortcut::SwitcherDone);
                }
                // Super alone: the launcher's search (ADR 0041).
                if std::mem::take(&mut self.system_keys.super_alone)
                    && !self.locked
                    && !self.capturing()
                {
                    self.dismiss_grab(DismissReason::OutsidePress);
                    let output = self.shortcut_output();
                    if let Some(client) = self.shortcut_shell(output) {
                        self.tell_shortcut(client, output, SystemShortcut::Search);
                    }
                }
            }
            return true;
        }
        if pressed && key == EMOJI_PICKER {
            self.system_keys.taken.insert(key);
            if !self.locked && !self.capturing() {
                self.input_source_shortcuts.push(InputSourceShortcut::Emoji);
            }
            return true;
        }
        if !pressed {
            return self.system_keys.taken.remove(&key);
        }
        // The screenshot interface open: every key is the system's, Super's shortcuts aside.
        if self.capturing() && !self.system_keys.holds([LEFT_SUPER, RIGHT_SUPER]) {
            self.system_keys.taken.insert(key);
            if !self.locked {
                self.capture_key(key);
            }
            return true;
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

    /// Something other than Super happened while it was down (a key, a button, a scroll, a key
    /// the keymap took): its release is no longer the search.
    pub(super) fn not_super_alone(&mut self) {
        self.system_keys.super_alone = false;
    }

    /// A key from either way keys come in (`keyboard`, `key`), before anything takes it: a press
    /// of any key but Super means Super is not pressed alone.
    pub(super) fn key_with_super(&mut self, key: u32, pressed: bool) {
        if pressed && !is_super(key) {
            self.not_super_alone();
        }
    }

    /// Whether a press of `key` now would be the system's, as `system_key` decides, and if so
    /// whether it repeats while held (decision 8, docs/adr/0036-key-repeat.mdx): what moves does
    /// (the switcher's Tab, \`, ← and →; Super+\` through the active application's windows), what
    /// acts never (closing, quitting, hiding, searching, Spotlight and revealing what it has
    /// chosen, a screenshot, the switcher's Q, H and Esc,
    /// any key while locked or while the screenshot interface is open).
    pub(super) fn system_repeats(&self, key: u32) -> Option<bool> {
        let keys = &self.system_keys;
        if is_super(key) {
            return Some(false);
        }
        if !keys.holds([LEFT_SUPER, RIGHT_SUPER]) {
            return self.capturing().then_some(false);
        }
        if self.locked || self.capturing() || is_modifier(key) {
            return Some(false);
        }
        let ctrl = keys.holds([LEFT_CTRL, RIGHT_CTRL]);
        let alt = keys.holds([LEFT_ALT, RIGHT_ALT]);
        Some(match keys.switcher {
            Some(_) => matches!(key, TAB | GRAVE | LEFT | RIGHT),
            None => key == GRAVE && !ctrl && !alt,
        })
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
        // Input sources (ADR 0041, on Super+Alt since docs/adr/0051-search.mdx gave Super+Space
        // to Spotlight): never while the screenshot interface is open.
        if key == SPACE && (ctrl || alt) && !self.capturing() {
            let shortcut = match (ctrl, alt, shift) {
                (false, true, false) => {
                    self.system_keys.spaces += 1;
                    InputSourceShortcut::Previous {
                        step: self.system_keys.spaces,
                    }
                }
                (false, true, true) => InputSourceShortcut::Next,
                (true, false, false) => InputSourceShortcut::Emoji,
                _ => return,
            };
            self.input_source_shortcuts.push(shortcut);
            return;
        }
        // Log out, as a Mac's ⇧⌘Q (asked) and ⌥⇧⌘Q (at once): the session decides
        // (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx). A menu open closes first.
        if key == Q && shift && !ctrl && !self.capturing() {
            self.dismiss_grab(DismissReason::OutsidePress);
            self.session_key(super::SessionKey::LogOut { confirm: !alt });
            return;
        }
        // Force Quit Applications, as a Mac's ⌥⌘⎋ (docs/adr/0067-hangs-and-force-quit.mdx,
        // decision 2): the program shows it. A menu open closes first.
        if key == ESCAPE && alt && !ctrl && !shift && !self.capturing() {
            self.dismiss_grab(DismissReason::OutsidePress);
            self.force_quit_shortcut();
            return;
        }
        // Tiling, as a Mac's Fn-Control with the same keys (ADR 0016, decision 12): Super+Alt with
        // F fills, C centres, R returns to the previous size, an arrow tiles to that half, and with
        // Shift an arrow arranges the active window and the one behind it.
        if alt && !ctrl && !self.capturing() {
            if let Some(layout) = tiling_layout(key, shift) {
                if let Some(active) = self.active.filter(|active| self.live(*active).is_some()) {
                    self.dismiss_grab(DismissReason::OutsidePress);
                    self.lay_out_window(active, layout);
                }
                return;
            }
        }
        if alt {
            // Close All and Minimize All, as a Mac's ⌥⌘W and ⌥⌘M: Super+W and Super+M for every
            // window of the active application. Never while the screenshot interface is open.
            let action = match (key, ctrl, shift) {
                (W, false, false) => WindowAction::CloseAll,
                (M, false, false) => WindowAction::MinimizeAll,
                _ => return,
            };
            if !self.capturing() {
                self.window_action(action);
            }
            return;
        }
        // Screenshots (decision 10). While the interface is open, only these act.
        match (key, ctrl, shift) {
            (KEY_3, false, true) => return self.capture_screens(),
            (KEY_4, false, true) => {
                self.dismiss_grab(DismissReason::OutsidePress);
                return self.open_capture_interface(CaptureMode::Selection);
            }
            (KEY_5, false, true) => {
                self.dismiss_grab(DismissReason::OutsidePress);
                return self.open_capture_interface(CaptureMode::Toolbar);
            }
            _ if self.capturing() => return,
            _ => {}
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
            // Spotlight, as a Mac's ⌘Space, and what it has chosen revealed where it is, as its
            // ⌘Return (docs/adr/0051-search.mdx): the shell shows them; a menu open closes first
            // as Spotlight opens, and stays as what it has chosen is revealed, which acts on what
            // the shell shows. Nothing reaches the application, and neither repeats.
            (SPACE | ENTER, false, false) => {
                if key == SPACE {
                    self.dismiss_grab(DismissReason::OutsidePress);
                }
                let output = self.shortcut_output();
                if let Some(client) = self.shortcut_shell(output) {
                    let shortcut = if key == SPACE {
                        SystemShortcut::Spotlight
                    } else {
                        SystemShortcut::Reveal
                    };
                    self.tell_shortcut(client, output, shortcut);
                }
                return;
            }
            // Mission Control and App Exposé, as a Mac's Control-↑ and Control-↓ (ADR 0045): the
            // shell shows them, as it does for the touchpad's swipes; a menu open closes first.
            (UP | DOWN, false, false) => {
                self.dismiss_grab(DismissReason::OutsidePress);
                let output = self.shortcut_output();
                if let Some(client) = self.shortcut_shell(output) {
                    let shortcut = if key == UP {
                        SystemShortcut::MissionControl
                    } else {
                        SystemShortcut::AppExpose
                    };
                    self.tell_shortcut(client, output, shortcut);
                }
                return;
            }
            // The keyboard to the top bar's menus, as a Mac's Control-F2: the shell takes it with
            // `FocusShellSurface`, citing this shortcut. A menu open in a window closes first.
            (F2 | F3 | F8 | N, false, false) => {
                self.dismiss_grab(DismissReason::OutsidePress);
                let output = self.shortcut_output();
                if let Some(client) = self.shortcut_shell(output) {
                    // The dock, as a Mac's Control-F3, the top bar's status items, as its
                    // Control-F8, and the Notification Center, as its Fn-N (ADR 0039): the shell
                    // takes the keyboard as for the menus.
                    let shortcut = match key {
                        F2 => SystemShortcut::MenuBar,
                        F3 => SystemShortcut::Dock,
                        F8 => SystemShortcut::StatusMenus,
                        _ => SystemShortcut::Notifications,
                    };
                    self.tell_shortcut(client, output, shortcut);
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
        self.window_action(action);
    }

    /// One of the compositor's own window actions, on the active window or its application.
    fn window_action(&mut self, action: WindowAction) {
        // With no active window, nothing to act on.
        let Some(active) = self.active.filter(|active| self.live(*active).is_some()) else {
            return;
        };
        // A menu open closes first, as a press elsewhere closes it; one of the window's own closes
        // with the window when it goes from the screen (its parent hidden), so the window is never
        // given the keyboard back for the instant between.
        if !matches!(
            action,
            WindowAction::Minimize | WindowAction::MinimizeAll | WindowAction::Hide
        ) {
            self.dismiss_grab(DismissReason::OutsidePress);
        }
        match action {
            WindowAction::Close => {
                self.emit(active.client, Event::CloseRequested { surface: active.id })
            }
            WindowAction::CloseAll => self.close_application_windows(active),
            WindowAction::Quit => self.quit_application(active),
            WindowAction::Minimize => {
                self.minimize(active);
                // Any other menu (the top bar's, opened from the keyboard).
                self.dismiss_grab(DismissReason::OutsidePress);
            }
            WindowAction::MinimizeAll => {
                self.minimize_application(active);
                self.dismiss_grab(DismissReason::OutsidePress);
            }
            WindowAction::Hide => {
                self.hide_application(active);
                self.dismiss_grab(DismissReason::OutsidePress);
            }
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

    /// Sends a shortcut to a shell: its serial is the shell's input, as a key press is. A key's
    /// toggles what it shows.
    pub(super) fn tell_shortcut(
        &mut self,
        client: ClientId,
        output: OutputId,
        shortcut: SystemShortcut,
    ) {
        self.tell_shortcut_intent(client, output, shortcut, ShortcutIntent::Toggle);
    }

    /// The same, asking what `intent` says of what it shows: a swipe's opening or closing it
    /// (docs/adr/0046-interactive-gestures.mdx), never sent to a shell of version 24 or older.
    pub(super) fn tell_shortcut_intent(
        &mut self,
        client: ClientId,
        output: OutputId,
        shortcut: SystemShortcut,
        intent: ShortcutIntent,
    ) {
        if !self.clients.contains_key(&client) {
            return;
        }
        let serial = self.next_serial();
        self.last_user_serial = serial;
        self.credit_input(client, serial);
        self.emit(
            client,
            Event::SystemShortcut {
                shortcut,
                serial,
                output,
                intent,
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

    /// Super+Alt+W, a Mac's ⌥⌘W (Close All): each window of `key`'s application, on any
    /// workspace, shown or minimized, is asked to close as Super+W asks one, and each may ask
    /// first or refuse. The application is not quitting: what it runs in the background goes on.
    fn close_application_windows(&mut self, key: SurfaceKey) {
        for window in self.application_windows(key) {
            self.emit(window.client, Event::CloseRequested { surface: window.id });
        }
    }

    /// Super+Alt+M, a Mac's ⌥⌘M (Minimize All): the windows of `key`'s application shown on this
    /// workspace go into the dock together, as the dock's press on the frontmost application
    /// minimizes them (docs/adr/0034-the-dock.mdx, decision 9), the active one last, so the
    /// keyboard moves once, to the window used last of another application. A window minimized
    /// already, hidden or on another workspace stays as it is.
    fn minimize_application(&mut self, key: SurfaceKey) {
        let application = self.application_windows(key);
        // Back to front, the active one last.
        let mut shown: Vec<SurfaceKey> = self
            .roots
            .iter()
            .copied()
            .filter(|root| application.contains(root) && self.visible(*root))
            .collect();
        shown.sort_by_key(|window| *window == key);
        for window in shown {
            self.minimize(window);
        }
    }

    /// Asks each window of `key`'s application to close; it may refuse.
    pub(super) fn quit_application(&mut self, key: SurfaceKey) {
        for window in self.application_windows(key) {
            self.emit(window.client, Event::CloseRequested { surface: window.id });
            // Quit ends the application, its background activity too
            // (docs/adr/0060-login-items-and-background-activity.mdx).
            self.note_quit(window.client);
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
    /// it received lately; without such an input, it is ignored. Its background (the desktop, ADR
    /// 0047) takes the keys only for the press the person made on it, cited while it is the latest
    /// press: a press on a Mac's desktop makes Finder active, the window that had the keys
    /// inactive. Never by itself, and never for any other input. A surface it never had, or one of
    /// a kind that never takes the keys, is its mistake. While a system sheet
    /// waits for the person (ADR 0030), an overlay takes the keys with no input of the shell's:
    /// the sheet is modal, the key window as a Mac's alert is, and what is typed while it asks is
    /// never the application's behind it.
    pub(super) fn focus_shell_surface(
        &mut self,
        key: SurfaceKey,
        serial: u64,
    ) -> Result<(), ErrorCode> {
        let node = self.nodes.get(&key).ok_or(ErrorCode::UnknownSurface)?;
        let sheet = self.sheets_waiting() > 0
            && matches!(
                node.role,
                Role::Shell {
                    kind: ShellKind::Overlay,
                    ..
                }
            );
        match node.role {
            Role::Shell {
                kind: ShellKind::Panel | ShellKind::Overlay | ShellKind::Notification,
                ..
            } => {}
            Role::Shell {
                kind: ShellKind::Background,
                ..
            } => {
                let pressed_here = self.last_press.is_some_and(|press| {
                    press.serial == serial
                        && press.client == key.client
                        && press.pointer
                        && self.root_of(press.surface) == key
                });
                if pressed_here
                    && !self.locked
                    && !node.dismissed
                    && self.visible(key)
                    && self.grab.is_empty()
                {
                    self.set_focus(Some(key));
                }
                return Ok(());
            }
            // The compositor gives the lock screen the keys while locked, and takes them back as it
            // unlocks; a shell whose field still had them asks for them as the lock goes (its
            // request crosses the unlock): nothing to do, as for a surface the compositor dismissed.
            Role::Lock { .. } => return Ok(()),
            _ => return Err(ErrorCode::InvalidRole),
        }
        if node.dismissed {
            return Ok(());
        }
        let recent = self
            .inputs
            .get(&key.client)
            .is_some_and(|inputs| inputs.contains(&serial));
        if (recent || sheet) && self.focusable(key) && self.grab.is_empty() {
            self.activate(key);
        }
        Ok(())
    }
}
