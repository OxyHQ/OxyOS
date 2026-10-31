//! What the dock's application menu needs of the compositor (docs/adr/0034-the-dock.mdx):
//!
//! - **Whether each client still answers.** A client of version 8 or newer given input (a press, a
//!   key), asked to close a window or activated is sent a Ping when none to it is unanswered; one
//!   that has not answered (Pong) within [`PING_TIMEOUT`] is not responding, and its windows say so
//!   to the shell (`WindowState`'s `responding`) until it answers. Its library answers as it reads
//!   events, so a process stuck in its own work does not.
//! - **Force quitting.** The shell names a window; the scene hands the program running it the
//!   clients of that window's application that the session started (their launch records), and
//!   the program ends their processes. A client the session did not start is never named: the
//!   shell, the login screen, the lock screen and any client claiming an id.
//! - **The person's lists of applications** (kept in the dock, opened at login): the shell's change
//!   handed whole to the program, which validates it, keeps it in the user's preferences and tells
//!   the scene, which tells every shell.
//! - **URLs the shell opens**, handed to the program, which starts the application whose entry
//!   declares the scheme and answers as it answers a launch.
//! - **The Trash**: how many items it holds, as the program says, told to every shell; and the
//!   shell's request to empty it, handed to the program, which asks the person in a system sheet.
//!
//! The scene ends no process, keeps no list and opens nothing.

use super::launch::assemble;
use super::{ClientId, Scene, SurfaceKey};
use oxyos_protocol::{
    ApplicationListKind, ErrorCode, Event, PreeditText, TextChunk, MAX_APPLICATION_LIST_BYTES,
    MAX_URL_BYTES,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// How long a client has to answer a Ping before it is not responding, as Mutter's and close to a
/// Mac's: a frame that takes this long is not one the person waits for.
pub const PING_TIMEOUT: Duration = Duration::from_secs(5);
/// The first version whose clients answer a Ping.
const PING_VERSION: u16 = 8;
/// Requests asked for and not yet taken, of each kind.
const MAX_PENDING: usize = 16;

/// The person's change of one of their lists, as the shell sent it: the program parses and judges
/// it (one id a line).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationListChange {
    pub client: ClientId,
    pub serial: u32,
    pub list: ApplicationListKind,
    pub text: String,
}

/// A URL the shell asked the session to open, citing an input it received lately or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlRequest {
    pub client: ClientId,
    pub serial: u32,
    pub url: String,
    pub with_input: bool,
}

/// One of the person's lists as the program keeps it: whether they chose it, and its ids in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplicationList {
    pub chosen: bool,
    pub ids: Vec<String>,
}

#[derive(Debug, Default)]
pub(super) struct Dock {
    /// Each client's protocol version, as the program running the compositor negotiated it.
    versions: BTreeMap<ClientId, u16>,
    /// The Ping each client has not answered: its serial and when it was sent.
    pings: BTreeMap<ClientId, (u32, Duration)>,
    next_ping: u32,
    not_responding: BTreeSet<ClientId>,
    /// The clients of each application the shell asked to force-quit, not yet taken.
    force_quits: Vec<Vec<ClientId>>,
    list_receiving: BTreeMap<(ClientId, u32), (String, u16)>,
    list_changes: Vec<ApplicationListChange>,
    lists: BTreeMap<ApplicationListKind, ApplicationList>,
    url_receiving: BTreeMap<(ClientId, u32), (String, u16)>,
    urls: Vec<UrlRequest>,
    /// How many items the Trash holds, as the program last said.
    trash_items: Option<u32>,
    /// The shell asked to empty the Trash, citing an input it received lately or not.
    trash_empties: Vec<bool>,
}

/// `text` in chunks of at most a chunk's bytes, cut on character boundaries; one (empty) for none.
fn chunks(text: &str) -> Vec<PreeditText> {
    if text.is_empty() {
        return vec![PreeditText::default()];
    }
    TextChunk::split(text)
        .into_iter()
        .map(|chunk| PreeditText::new(chunk.as_str()).expect("a chunk fits"))
        .collect()
}

impl Scene {
    /// The protocol version `client` speaks (ADR 0033): only a client of version 8 or newer is
    /// sent a Ping, and so only one can be not responding.
    pub fn set_version(&mut self, client: ClientId, version: u16) {
        if self.clients.contains_key(&client) {
            self.dock.versions.insert(client, version);
        }
    }

    /// Sends `client` a Ping, unless one to it is unanswered or its version does not answer.
    fn ping(&mut self, client: ClientId) {
        let dock = &mut self.dock;
        if dock.pings.contains_key(&client)
            || dock.versions.get(&client).is_none_or(|v| *v < PING_VERSION)
        {
            return;
        }
        dock.next_ping = dock.next_ping.wrapping_add(1).max(1);
        let serial = dock.next_ping;
        dock.pings.insert(client, (serial, self.clock));
        self.events.push((client, Event::Ping { serial }));
    }

    /// An event about to be sent: what asks a client to answer (input, a close request, its
    /// activation) pings it.
    pub(super) fn pings_for(&mut self, client: ClientId, event: &Event) {
        use oxyos_protocol::{PointerKind, TouchKind};
        let asks = match event {
            Event::Pointer { kind, .. } => *kind == PointerKind::Press,
            Event::Key { pressed, .. } | Event::KeyText { pressed, .. } => *pressed,
            Event::Touch { kind, .. } => *kind == TouchKind::Down,
            Event::CloseRequested { .. } => true,
            Event::Activated { active, .. } => *active,
            _ => false,
        };
        if asks {
            self.ping(client);
        }
    }

    pub(super) fn pong(&mut self, client: ClientId, serial: u32) {
        if self
            .dock
            .pings
            .get(&client)
            .is_some_and(|(sent, _)| *sent == serial)
        {
            self.dock.pings.remove(&client);
            self.dock.not_responding.remove(&client);
        }
    }

    /// Marks the clients that did not answer a Ping within [`PING_TIMEOUT`] of the clock (the
    /// program running the compositor calls it every turn, after `set_clock`).
    pub fn check_responding(&mut self) {
        let now = self.clock;
        let late: Vec<ClientId> = self
            .dock
            .pings
            .iter()
            .filter(|(_, (_, sent))| now.saturating_sub(*sent) >= PING_TIMEOUT)
            .map(|(client, _)| *client)
            .collect();
        self.dock.not_responding.extend(late);
    }

    /// When the next unanswered Ping is late, after the clock: the program's turn waits no longer.
    pub fn next_ping_deadline(&self) -> Option<Duration> {
        self.dock
            .pings
            .iter()
            .filter(|(client, _)| !self.dock.not_responding.contains(client))
            .map(|(_, (_, sent))| (*sent + PING_TIMEOUT).saturating_sub(self.clock))
            .min()
    }

    /// Whether `client` answers (a client never pinged does).
    pub fn responding(&self, client: ClientId) -> bool {
        !self.dock.not_responding.contains(&client)
    }

    /// The shell asks to force-quit the application of the window `key`: its clients the session
    /// started, for the program to end. A window of a client the session did not start is not
    /// one it may end, and nothing is asked.
    pub(super) fn force_quit_application(&mut self, key: SurfaceKey) -> Result<(), ErrorCode> {
        if !self.shell_windows_launched(key.client) {
            return Ok(());
        }
        let mut clients: Vec<ClientId> = self
            .application_windows(key)
            .into_iter()
            .map(|window| window.client)
            .filter(|client| self.shell_windows_launched(*client))
            .collect();
        clients.push(key.client);
        clients.sort();
        clients.dedup();
        if self.dock.force_quits.len() >= MAX_PENDING {
            return Err(ErrorCode::QuotaExceeded);
        }
        self.dock.force_quits.push(clients);
        Ok(())
    }

    /// The clients the shell asked to force-quit since the last call, each application's
    /// together: the program running the compositor ends the processes it started, and none other.
    pub fn take_force_quits(&mut self) -> Vec<Vec<ClientId>> {
        std::mem::take(&mut self.dock.force_quits)
    }

    pub(super) fn application_list_request(
        &mut self,
        client: ClientId,
        serial: u32,
        list: ApplicationListKind,
        index: u16,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        if !self.clients[&client].shell {
            return Err(ErrorCode::Unauthorized);
        }
        let dock = &mut self.dock;
        let Some(text) = assemble(
            &mut dock.list_receiving,
            (client, serial),
            index,
            last,
            text,
            MAX_APPLICATION_LIST_BYTES,
        )?
        else {
            return Ok(());
        };
        if dock.list_changes.len() >= MAX_PENDING {
            return Err(ErrorCode::QuotaExceeded);
        }
        dock.list_changes.push(ApplicationListChange {
            client,
            serial,
            list,
            text,
        });
        Ok(())
    }

    /// The changes of the person's lists the shell asked for since the last call, in order: the
    /// program judges and keeps each, answers it (`preference_set`) and tells the scene the lists
    /// then ([`Scene::set_application_list`]).
    pub fn take_application_list_changes(&mut self) -> Vec<ApplicationListChange> {
        std::mem::take(&mut self.dock.list_changes)
    }

    /// One of the person's lists as the program keeps it; every shell is told it when it changes.
    pub fn set_application_list(&mut self, kind: ApplicationListKind, list: ApplicationList) {
        if self.dock.lists.get(&kind) == Some(&list) {
            return;
        }
        self.dock.lists.insert(kind, list);
        for shell in self.shell_clients() {
            self.tell_application_list(shell, kind);
        }
    }

    /// Tells a shell that connects every list the program said.
    pub(super) fn tell_application_lists(&mut self, client: ClientId) {
        let kinds: Vec<ApplicationListKind> = self.dock.lists.keys().copied().collect();
        for kind in kinds {
            self.tell_application_list(client, kind);
        }
    }

    fn tell_application_list(&mut self, client: ClientId, kind: ApplicationListKind) {
        let Some(list) = self.dock.lists.get(&kind) else {
            return;
        };
        let chosen = list.chosen;
        let parts = chunks(&list.ids.join("\n"));
        let count = parts.len();
        for (index, text) in parts.into_iter().enumerate() {
            self.emit(
                client,
                Event::ApplicationList {
                    list: kind,
                    chosen,
                    index: index as u16,
                    last: index + 1 == count,
                    text,
                },
            );
        }
    }

    pub(super) fn url_request(
        &mut self,
        client: ClientId,
        serial: u32,
        input: u64,
        index: u16,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        if !self.clients[&client].shell {
            return Err(ErrorCode::Unauthorized);
        }
        let dock = &mut self.dock;
        let Some(url) = assemble(
            &mut dock.url_receiving,
            (client, serial),
            index,
            last,
            text,
            MAX_URL_BYTES,
        )?
        else {
            return Ok(());
        };
        if dock.urls.len() >= MAX_PENDING {
            return Err(ErrorCode::QuotaExceeded);
        }
        let with_input = input != 0 && self.recent_input(client, input);
        self.dock.urls.push(UrlRequest {
            client,
            serial,
            url,
            with_input,
        });
        Ok(())
    }

    /// The URLs the shell asked to open since the last call, in order: the program opens each it
    /// may and answers each with [`Scene::launched`], as a launch.
    pub fn take_url_requests(&mut self) -> Vec<UrlRequest> {
        std::mem::take(&mut self.dock.urls)
    }

    /// How many items the user's Trashes hold, from the program; every shell is told it changing.
    pub fn set_trash_items(&mut self, items: u32) {
        if self.dock.trash_items == Some(items) {
            return;
        }
        self.dock.trash_items = Some(items);
        for shell in self.shell_clients() {
            self.emit(shell, Event::TrashItems { items });
        }
    }

    /// Tells a shell that connects what the Trash holds, if the program said.
    pub(super) fn tell_trash(&mut self, client: ClientId) {
        if let Some(items) = self.dock.trash_items {
            self.emit(client, Event::TrashItems { items });
        }
    }

    pub(super) fn empty_trash_request(
        &mut self,
        client: ClientId,
        input: u64,
    ) -> Result<(), ErrorCode> {
        if !self.clients[&client].shell {
            return Err(ErrorCode::Unauthorized);
        }
        if self.dock.trash_empties.len() >= MAX_PENDING {
            return Err(ErrorCode::QuotaExceeded);
        }
        let with_input = input != 0 && self.recent_input(client, input);
        self.dock.trash_empties.push(with_input);
        Ok(())
    }

    /// The shell's requests to empty the Trash since the last call, each whether it cited an input
    /// it received: the program asks the person (a system sheet) for one that did, and ignores the
    /// rest.
    pub fn take_trash_empties(&mut self) -> Vec<bool> {
        std::mem::take(&mut self.dock.trash_empties)
    }

    fn shell_windows_launched(&self, client: ClientId) -> bool {
        self.application(client).is_some()
    }

    pub(super) fn forget_dock_client(&mut self, client: ClientId) {
        let dock = &mut self.dock;
        dock.versions.remove(&client);
        dock.pings.remove(&client);
        dock.not_responding.remove(&client);
        for clients in &mut dock.force_quits {
            clients.retain(|c| *c != client);
        }
        dock.force_quits.retain(|clients| !clients.is_empty());
        dock.list_receiving.retain(|(asker, _), _| *asker != client);
        dock.list_changes.retain(|change| change.client != client);
        dock.url_receiving.retain(|(asker, _), _| *asker != client);
        dock.urls.retain(|request| request.client != client);
    }
}
