//! The menu bar (ADR 0021): each toplevel's menus, sent by its application in chunks and
//! committed whole, as an accessibility snapshot is; the session's shell (its `shell` grant) is
//! told the active window's menus whenever they or the active window change, and chooses a
//! command of them, which reaches the window's client only if that window still declares it.
//!
//! A command is named by its index in the commit that declared it and that commit's serial,
//! which rises with every commit of the window: a choice the shell made before a recommit names
//! an older serial and is refused, so it can never run whatever entry took that index since. The
//! application keeps its own ids for each index; the compositor and the shell never see them,
//! nor anything of the application's but labels and chords.

use super::{ClientId, Role, Scene, SurfaceKey};
use oxyos_protocol::{
    ErrorCode, Event, MenuField, MenuFlags, MenuKind, TextChunk, MAX_MENU_DEPTH, MAX_MENU_ITEMS,
    MAX_MENU_LABEL_BYTES, MAX_MENU_SHORTCUT_BYTES, NO_MENU_PARENT,
};
use std::collections::BTreeMap;

/// One entry of a window's menus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    /// Index of the menu or submenu it is in, earlier in the list; `NO_MENU_PARENT` for a menu
    /// of the bar.
    pub parent: u32,
    pub kind: MenuKind,
    pub flags: MenuFlags,
    pub label: String,
    /// A command's key chord, as the bar shows it (empty: none).
    pub shortcut: String,
}

/// A window's menus as one commit declared them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menus {
    /// The commit's serial, above every earlier commit's of the window.
    pub serial: u32,
    pub entries: Vec<MenuEntry>,
}

/// Menus being received: the commit's serial, how many entries it has, those sent so far and
/// each one's depth.
#[derive(Debug)]
struct Pending {
    serial: u32,
    count: u32,
    entries: Vec<MenuEntry>,
    depths: Vec<u32>,
}

#[derive(Debug, Default)]
struct SurfaceMenus {
    committed: Option<Menus>,
    pending: Option<Pending>,
}

#[derive(Debug, Default)]
pub(super) struct MenuBars {
    surfaces: BTreeMap<SurfaceKey, SurfaceMenus>,
    /// What each shell client was last told: the active window's number (0: none) and the serial
    /// of its menus (0: none).
    told: BTreeMap<ClientId, (u32, u32)>,
}

impl Scene {
    pub(super) fn menus_begin(
        &mut self,
        key: SurfaceKey,
        serial: u32,
        items: u32,
    ) -> Result<(), ErrorCode> {
        let node = self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        if node.role != Role::Toplevel {
            return Err(ErrorCode::InvalidRole);
        }
        if items > MAX_MENU_ITEMS {
            return Err(ErrorCode::QuotaExceeded);
        }
        let entry = self.menu_bars.surfaces.entry(key).or_default();
        // Serials rise with every commit: an older one would let a stale choice through.
        let last = entry.committed.as_ref().map_or(0, |menus| menus.serial);
        if serial <= last {
            return Err(ErrorCode::InvalidMessage);
        }
        entry.pending = Some(Pending {
            serial,
            count: items,
            entries: Vec::with_capacity(items as usize),
            depths: Vec::with_capacity(items as usize),
        });
        Ok(())
    }

    pub(super) fn menu_item(
        &mut self,
        key: SurfaceKey,
        index: u32,
        parent: u32,
        kind: MenuKind,
        flags: MenuFlags,
    ) -> Result<(), ErrorCode> {
        let pending = self.pending_menus(key)?;
        // In order, each once, no more than were announced.
        if index as usize != pending.entries.len() || index >= pending.count {
            return Err(ErrorCode::InvalidMessage);
        }
        let depth = if parent == NO_MENU_PARENT {
            // A menu of the bar, and only a menu, has no parent.
            if kind != MenuKind::Menu {
                return Err(ErrorCode::InvalidMessage);
            }
            1
        } else {
            let Some(container) = pending.entries.get(parent as usize) else {
                return Err(ErrorCode::InvalidMessage);
            };
            if !matches!(container.kind, MenuKind::Menu | MenuKind::Submenu)
                || kind == MenuKind::Menu
            {
                return Err(ErrorCode::InvalidMessage);
            }
            pending.depths[parent as usize] + 1
        };
        if depth > MAX_MENU_DEPTH {
            return Err(ErrorCode::QuotaExceeded);
        }
        // What each kind may say of itself: a command may be disabled and checked, a submenu
        // disabled, a menu or a separator nothing; a check only on a command that has one.
        let allowed = match kind {
            MenuKind::Item => MenuFlags::DISABLED | MenuFlags::CHECKABLE | MenuFlags::CHECKED,
            MenuKind::Submenu => MenuFlags::DISABLED,
            MenuKind::Menu | MenuKind::Separator => 0,
        };
        if flags.bits() & !allowed != 0
            || (flags.contains(MenuFlags::CHECKED) && !flags.contains(MenuFlags::CHECKABLE))
        {
            return Err(ErrorCode::InvalidMessage);
        }
        pending.entries.push(MenuEntry {
            parent,
            kind,
            flags,
            label: String::new(),
            shortcut: String::new(),
        });
        pending.depths.push(depth);
        Ok(())
    }

    pub(super) fn menu_text(
        &mut self,
        key: SurfaceKey,
        index: u32,
        field: MenuField,
        text: TextChunk,
    ) -> Result<(), ErrorCode> {
        let pending = self.pending_menus(key)?;
        let entry = pending
            .entries
            .get_mut(index as usize)
            .ok_or(ErrorCode::InvalidOrder)?;
        let (target, limit) = match (field, entry.kind) {
            (_, MenuKind::Separator) => return Err(ErrorCode::InvalidMessage),
            (MenuField::Label, _) => (&mut entry.label, MAX_MENU_LABEL_BYTES),
            (MenuField::Shortcut, MenuKind::Item) => (&mut entry.shortcut, MAX_MENU_SHORTCUT_BYTES),
            (MenuField::Shortcut, _) => return Err(ErrorCode::InvalidMessage),
        };
        if target.len() + text.as_str().len() > limit {
            return Err(ErrorCode::QuotaExceeded);
        }
        target.push_str(text.as_str());
        Ok(())
    }

    pub(super) fn menus_commit(&mut self, key: SurfaceKey) -> Result<(), ErrorCode> {
        self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        let entry = self
            .menu_bars
            .surfaces
            .get_mut(&key)
            .ok_or(ErrorCode::InvalidOrder)?;
        let pending = entry.pending.take().ok_or(ErrorCode::InvalidOrder)?;
        if pending.entries.len() != pending.count as usize {
            return Err(ErrorCode::InvalidOrder);
        }
        // Everything but a separator is named.
        if pending
            .entries
            .iter()
            .any(|entry| entry.kind != MenuKind::Separator && entry.label.is_empty())
        {
            return Err(ErrorCode::InvalidMessage);
        }
        entry.committed = Some(Menus {
            serial: pending.serial,
            entries: pending.entries,
        });
        Ok(())
    }

    fn pending_menus(&mut self, key: SurfaceKey) -> Result<&mut Pending, ErrorCode> {
        self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        self.menu_bars
            .surfaces
            .get_mut(&key)
            .and_then(|entry| entry.pending.as_mut())
            .ok_or(ErrorCode::InvalidOrder)
    }

    /// The shell chose a command: it reaches the window's client only if the window is the
    /// active one, `serial` is its menus' commit and `index` is a command there that it and
    /// everything it is in let be chosen. Anything else (a window that went or stopped being
    /// active, menus committed since, a menu, a separator, a disabled entry) is answered with
    /// nothing: the shell may have chosen from what it was told before.
    pub(super) fn activate_menu_item(
        &mut self,
        client: ClientId,
        window: u32,
        serial: u32,
        index: u32,
    ) -> Result<(), ErrorCode> {
        if !self.clients.get(&client).is_some_and(|grants| grants.shell) {
            return Err(ErrorCode::Unauthorized);
        }
        if self.locked {
            return Ok(());
        }
        let Some(key) = self
            .active
            .filter(|key| self.window_number(*key) == Some(window))
        else {
            return Ok(());
        };
        self.choose_menu_item(key, serial, index);
        Ok(())
    }

    /// The session's bar chooses the entry `index` of the commit `serial` of the active
    /// window's menus, as the shell's choice is delivered (a development compositor's played
    /// bar): false, and nothing delivered, for anything the shell's choice would not deliver.
    pub fn choose_active_menu_item(&mut self, serial: u32, index: u32) -> bool {
        match self.active {
            Some(key) if !self.locked => self.choose_menu_item(key, serial, index),
            _ => false,
        }
    }

    fn choose_menu_item(&mut self, key: SurfaceKey, serial: u32, index: u32) -> bool {
        let Some(menus) = self
            .menu_bars
            .surfaces
            .get(&key)
            .and_then(|entry| entry.committed.as_ref())
            .filter(|menus| menus.serial == serial)
        else {
            return false;
        };
        let Some(entry) = menus.entries.get(index as usize) else {
            return false;
        };
        if entry.kind != MenuKind::Item {
            return false;
        }
        let mut at = Some(entry);
        while let Some(entry) = at {
            if entry.flags.contains(MenuFlags::DISABLED) {
                return false;
            }
            at = menus.entries.get(entry.parent as usize);
        }
        // The choice is the person's input, as a key press is: what the command does (a paste,
        // a popup) may cite its serial.
        let input = self.next_serial();
        self.last_user_serial = input;
        self.record_press(input, key, false);
        self.emit(
            key.client,
            Event::MenuItemActivated {
                surface: key.id,
                serial: input,
                menus: serial,
                index,
            },
        );
        true
    }

    /// A surface went away: its menus with it.
    pub(super) fn forget_menus(&mut self, key: SurfaceKey) {
        self.menu_bars.surfaces.remove(&key);
    }

    pub(super) fn forget_menu_bar_client(&mut self, client: ClientId) {
        self.menu_bars.told.remove(&client);
    }

    /// The active window's number and its committed menus, if any.
    fn active_menus(&self) -> (u32, Option<&Menus>) {
        let Some(key) = self
            .active
            .filter(|key| self.live(*key).is_some_and(|node| node.mapped))
        else {
            return (0, None);
        };
        let Some(number) = self.window_number(key) else {
            return (0, None);
        };
        let menus = self
            .menu_bars
            .surfaces
            .get(&key)
            .and_then(|entry| entry.committed.as_ref())
            .filter(|menus| !menus.entries.is_empty());
        (number, menus)
    }

    /// Tells each shell client the active window's menus, if they or the active window changed
    /// since it was last told (before events are sent, after the windows themselves).
    pub(super) fn tell_menu_bar(&mut self) {
        let shells: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, grants)| grants.shell)
            .map(|(client, _)| *client)
            .collect();
        if shells.is_empty() {
            return;
        }
        let (window, menus) = self.active_menus();
        let now = (window, menus.map_or(0, |menus| menus.serial));
        let entries = menus.map(|menus| menus.entries.clone()).unwrap_or_default();
        for client in shells {
            if self.menu_bars.told.get(&client) == Some(&now) {
                continue;
            }
            self.menu_bars.told.insert(client, now);
            self.emit(
                client,
                Event::MenuBar {
                    window,
                    serial: now.1,
                    items: entries.len() as u32,
                },
            );
            for (index, entry) in (0..).zip(&entries) {
                self.emit(
                    client,
                    Event::MenuBarItem {
                        window,
                        index,
                        parent: entry.parent,
                        kind: entry.kind,
                        flags: entry.flags,
                    },
                );
                for (field, text) in [
                    (MenuField::Label, &entry.label),
                    (MenuField::Shortcut, &entry.shortcut),
                ] {
                    for text in TextChunk::split(text) {
                        self.emit(
                            client,
                            Event::MenuBarText {
                                window,
                                index,
                                field,
                                text,
                            },
                        );
                    }
                }
            }
            self.emit(client, Event::MenuBarCommitted { window });
        }
    }

    /// A window's committed menus, as the shell is told them when it is active.
    pub fn menus(&self, key: SurfaceKey) -> Option<&Menus> {
        self.menu_bars
            .surfaces
            .get(&key)
            .and_then(|entry| entry.committed.as_ref())
    }
}
