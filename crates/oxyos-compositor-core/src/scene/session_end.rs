//! Sleep, restart, shut down and log out (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx):
//! what the compositor takes and hands to the program running it, which alone decides.
//!
//! - **The power key** (`KEY_POWER`) is the system's, as the hardware keys are (ADR 0025): no
//!   application, input method or shortcut ever sees it, press or release, locked or not; a press
//!   is handed over ([`SessionKey::Power`]).
//! - **Super+Shift+Q and Super+Alt+Shift+Q** log out, as a Mac's ⇧⌘Q and ⌥⇧⌘Q, asked and at
//!   once ([`SessionKey::LogOut`]); taken in `system_shortcut`, so never while locked.
//! - **Asking applications to quit.** [`Scene::ask_to_quit`] sends an application of protocol
//!   version 38 or newer `QuitRequested`, and each window of an older one `CloseRequested` (it
//!   cannot refuse). A `QuitReply` is taken only from a client asked with that serial, once.
//!
//! The scene decides nothing else: who is asked, the deadline and logind are the program's.

use super::{ClientId, Role, Scene};
use oxyos_protocol::{Event, SessionEnd, QUIT_REQUESTS_VERSION};
use std::collections::{BTreeMap, BTreeSet};

/// The kernel's power key (`KEY_POWER`).
pub const KEY_POWER: u32 = 116;
/// Keys handed over and not yet taken.
const MAX_PENDING: usize = 16;

/// A key of the session's end, for the program running the compositor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKey {
    /// The power key was pressed.
    Power,
    /// Super+Shift+Q (`confirm`), or Super+Alt+Shift+Q (not).
    LogOut { confirm: bool },
}

/// An application's answer to the session's request to quit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuitReplied {
    pub client: ClientId,
    pub serial: u32,
    pub quit: bool,
}

#[derive(Debug, Default)]
pub(super) struct SessionEnding {
    keys: Vec<SessionKey>,
    /// The applications that said they may be ended at once (`SuddenTermination`, decision 10).
    sudden: BTreeSet<ClientId>,
    /// Super+Alt+Escape pressed and not yet taken (docs/adr/0067-hangs-and-force-quit.mdx).
    force_quits: u32,
    /// The serial each application was asked to quit with and has not answered.
    asked: BTreeMap<ClientId, u32>,
    replies: Vec<QuitReplied>,
}

impl Scene {
    /// Whether `code` is the power key: then nobody else sees it, and a press is handed over.
    pub(super) fn power_key(&mut self, code: u32, pressed: bool) -> bool {
        if code != KEY_POWER {
            return false;
        }
        if pressed {
            self.session_key(SessionKey::Power);
        }
        true
    }

    pub(super) fn session_key(&mut self, key: SessionKey) {
        let keys = &mut self.session_ending.keys;
        if keys.len() < MAX_PENDING {
            keys.push(key);
        }
    }

    /// The session's keys pressed since the last call, in order.
    pub fn take_session_keys(&mut self) -> Vec<SessionKey> {
        std::mem::take(&mut self.session_ending.keys)
    }

    /// Super+Alt+Escape (a Mac's ⌥⌘⎋) was pressed since the last call: the program shows Force
    /// Quit Applications (docs/adr/0067-hangs-and-force-quit.mdx, decision 2). The scene only
    /// takes the key, as the compositor's own, so it works whatever the shell or an application
    /// is doing.
    pub fn take_force_quit_shortcut(&mut self) -> bool {
        std::mem::take(&mut self.session_ending.force_quits) > 0
    }

    pub(super) fn force_quit_shortcut(&mut self) {
        self.session_ending.force_quits = self.session_ending.force_quits.saturating_add(1);
    }

    /// The clients that are applications, asked to quit before the session ends: none of the
    /// session's own (the shell, the lock, the input method, the assistive technology).
    pub fn applications(&self) -> Vec<ClientId> {
        self.clients
            .iter()
            .filter(|(_, grants)| {
                !(grants.shell
                    || grants.lock
                    || grants.input_method
                    || grants.accessibility
                    || grants.screen_reader)
            })
            .map(|(client, _)| *client)
            .collect()
    }

    /// The clients holding `shell`: the session's shells, and the login screen at the login
    /// screen's session.
    pub fn shells(&self) -> Vec<ClientId> {
        self.shell_clients()
    }

    /// Whether `client` is still connected.
    pub fn connected(&self, client: ClientId) -> bool {
        self.clients.contains_key(&client)
    }

    /// Asks `client` to quit because the session ends for `reason`: true when it was sent
    /// `QuitRequested` and its answer is awaited, false when it is of an older version and each of
    /// its windows was sent `CloseRequested` instead (it cannot refuse).
    pub fn ask_to_quit(&mut self, client: ClientId, serial: u32, reason: SessionEnd) -> bool {
        if !self.clients.contains_key(&client) {
            return false;
        }
        if self
            .version_of(client)
            .is_some_and(|version| version >= QUIT_REQUESTS_VERSION)
        {
            self.session_ending.asked.insert(client, serial);
            self.emit(client, Event::QuitRequested { serial, reason });
            return true;
        }
        let windows: Vec<u32> = self
            .nodes
            .iter()
            .filter(|(key, node)| key.client == client && node.role == Role::Toplevel)
            .map(|(key, _)| key.id)
            .collect();
        for surface in windows {
            self.emit(client, Event::CloseRequested { surface });
        }
        self.note_quit(client);
        false
    }

    /// An application's `QuitReply`: taken once, only for the serial it was asked with.
    pub(super) fn quit_reply(&mut self, client: ClientId, serial: u32, quit: bool) {
        let ending = &mut self.session_ending;
        if ending.asked.get(&client) != Some(&serial) {
            return;
        }
        ending.asked.remove(&client);
        ending.replies.push(QuitReplied {
            client,
            serial,
            quit,
        });
    }

    /// The applications' answers since the last call, in order.
    pub fn take_quit_replies(&mut self) -> Vec<QuitReplied> {
        std::mem::take(&mut self.session_ending.replies)
    }

    /// Forgets what was asked: the end was cancelled or went on; a late answer is ignored.
    pub fn forget_quit_requests(&mut self) {
        self.session_ending.asked.clear();
    }

    pub(super) fn forget_session_end_client(&mut self, client: ClientId) {
        // Its answer, if it gave one before it went, stays: a cancel still cancels.
        self.session_ending.asked.remove(&client);
        self.session_ending.sudden.remove(&client);
    }

    /// An application's `SuddenTermination` (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx,
    /// decision 10): whether it may be ended at once as the session ends, without being asked.
    pub(super) fn set_sudden_termination(&mut self, client: ClientId, enabled: bool) {
        if enabled {
            self.session_ending.sudden.insert(client);
        } else {
            self.session_ending.sudden.remove(&client);
        }
    }

    /// Whether `client` said it may be ended at once, with nothing unsaved: the program ends it
    /// instead of asking it to quit, as a Mac's Sudden Termination.
    pub fn may_end_at_once(&self, client: ClientId) -> bool {
        self.session_ending.sudden.contains(&client)
    }
}
