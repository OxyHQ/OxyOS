//! Installing applications (docs/adr/0030-installing-applications.mdx, decisions 8, 9 and 16).
//!
//! A client with the `store`, `updates` or `privacy` grant asks the session for something in the
//! session's vocabulary (JSON text in chunks, `SystemRequest`); the scene checks the grant, notes
//! whether the request cites an input the client received, and hands it whole to the program
//! running the compositor, which parses it strictly, acts, and answers (`SystemText`). Whatever
//! needs the person's confirmation is asked in a system sheet the scene sends the shell alone
//! (`SheetText`); only the shell answers it (`SheetAnswer`), and an acceptance counts only citing
//! an input the shell received, so no application can confirm one. An application asks for a
//! capability asked at first use (`RequestCapability`); the program decides and answers. The scene
//! knows no vocabulary, no application and no capability.

use super::{ClientId, Scene};
use oxyos_protocol::{ErrorCode, Event, PreeditText, TextChunk, MAX_SYSTEM_TEXT_BYTES};
use std::collections::{BTreeMap, BTreeSet};

/// Requests asked for and not yet taken.
const MAX_PENDING: usize = 32;
/// Sheets shown and not yet answered.
const MAX_SHEETS: usize = 8;

/// A request of a client with the `store`, `updates` or `privacy` grant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemRequest {
    pub client: ClientId,
    pub serial: u32,
    /// Whether it cited an input the client received lately: what changes anything needs one.
    pub with_input: bool,
    pub text: String,
}

/// The person's answer to a sheet, from the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SheetAnswered {
    pub sheet: u32,
    pub accepted: bool,
}

/// An application's request for a capability asked at first use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityRequest {
    pub client: ClientId,
    pub serial: u32,
    /// 1 notifications, 2 files, 3 camera, 4 microphone, 5 location.
    pub capability: u8,
    pub with_input: bool,
}

#[derive(Debug, Default)]
pub(super) struct System {
    receiving: BTreeMap<(ClientId, u32), (String, u16)>,
    asked: Vec<SystemRequest>,
    /// Sheets the shell was sent and not answered nor dismissed.
    sheets: BTreeSet<u32>,
    next_sheet: u32,
    answered: Vec<SheetAnswered>,
    capabilities: Vec<CapabilityRequest>,
}

/// `text` in chunks, each of at most a chunk's bytes, cut on character boundaries.
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
    pub(super) fn system_request(
        &mut self,
        client: ClientId,
        serial: u32,
        input: u64,
        index: u16,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        let grants = self.clients[&client];
        // Without a grant that asks, the request is refused and said, not an error: an
        // application cannot know its grants before it asks (Settings run as a development entry,
        // Atlas outside a session), and shows that what it asked is not available here.
        if !(grants.store || grants.updates || grants.privacy) {
            if last {
                self.refuse_system_request(client, serial);
            }
            return Ok(());
        }
        let receiving = &mut self.system.receiving;
        let entry = receiving
            .entry((client, serial))
            .or_insert_with(|| (String::new(), 0));
        if index == 0 {
            *entry = (String::new(), 0);
        } else if index != entry.1 {
            receiving.remove(&(client, serial));
            return Err(ErrorCode::InvalidOrder);
        }
        if entry.0.len() + text.as_str().len() > MAX_SYSTEM_TEXT_BYTES {
            receiving.remove(&(client, serial));
            return Err(ErrorCode::QuotaExceeded);
        }
        entry.0.push_str(text.as_str());
        entry.1 = index.wrapping_add(1);
        if !last {
            return Ok(());
        }
        let (text, _) = receiving.remove(&(client, serial)).expect("received");
        if self.system.asked.len() >= MAX_PENDING {
            return Err(ErrorCode::QuotaExceeded);
        }
        let with_input = input != 0 && self.recent_input(client, input);
        self.system.asked.push(SystemRequest {
            client,
            serial,
            with_input,
            text,
        });
        Ok(())
    }

    /// What `client` holds, if it is connected.
    pub fn grants_of(&self, client: ClientId) -> Option<super::Grants> {
        self.clients.get(&client).copied()
    }

    fn refuse_system_request(&mut self, client: ClientId, serial: u32) {
        let text = r#"{"refused":{"reason":"notPermitted"}}"#;
        let parts = chunks(text);
        let count = parts.len();
        for (index, text) in parts.into_iter().enumerate() {
            self.emit(
                client,
                Event::SystemText {
                    serial,
                    index: index as u16,
                    last: index + 1 == count,
                    text,
                },
            );
        }
    }

    /// The requests asked for since the last call, in order.
    pub fn take_system_requests(&mut self) -> Vec<SystemRequest> {
        std::mem::take(&mut self.system.asked)
    }

    /// Tells `client` the answer to its request `serial` (0: the update state, unasked), if it
    /// still holds a grant that asks.
    pub fn system_text(&mut self, client: ClientId, serial: u32, text: &str) {
        let Some(grants) = self.clients.get(&client) else {
            return;
        };
        if !(grants.store || grants.updates || grants.privacy) || text.len() > MAX_SYSTEM_TEXT_BYTES
        {
            return;
        }
        let parts = chunks(text);
        let count = parts.len();
        for (index, text) in parts.into_iter().enumerate() {
            self.emit(
                client,
                Event::SystemText {
                    serial,
                    index: index as u16,
                    last: index + 1 == count,
                    text,
                },
            );
        }
    }

    /// The clients holding `store` or `updates`: those the update state is told to.
    pub fn update_watchers(&self) -> Vec<ClientId> {
        self.clients
            .iter()
            .filter(|(_, grants)| grants.store || grants.updates)
            .map(|(client, _)| *client)
            .collect()
    }

    /// Sends the shell a sheet to show, described by `text`; its number, or none if no shell is
    /// connected, the session is locked, or too many are asked.
    pub fn show_sheet(&mut self, text: &str) -> Option<u32> {
        if self.locked
            || self.system.sheets.len() >= MAX_SHEETS
            || text.len() > MAX_SYSTEM_TEXT_BYTES
        {
            return None;
        }
        let shells: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, grants)| grants.shell)
            .map(|(client, _)| *client)
            .collect();
        if shells.is_empty() {
            return None;
        }
        self.system.next_sheet = self.system.next_sheet.wrapping_add(1).max(1);
        let sheet = self.system.next_sheet;
        self.system.sheets.insert(sheet);
        let parts = chunks(text);
        let count = parts.len();
        for shell in shells {
            for (index, text) in parts.iter().enumerate() {
                self.emit(
                    shell,
                    Event::SheetText {
                        sheet,
                        index: index as u16,
                        last: index + 1 == count,
                        text: *text,
                    },
                );
            }
        }
        Some(sheet)
    }

    /// Withdraws a sheet the shell was sent and not answered.
    pub fn dismiss_sheet(&mut self, sheet: u32) {
        if !self.system.sheets.remove(&sheet) {
            return;
        }
        let shells: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, grants)| grants.shell)
            .map(|(client, _)| *client)
            .collect();
        for shell in shells {
            self.emit(shell, Event::SheetDismissed { sheet });
        }
    }

    pub(super) fn sheet_answer(
        &mut self,
        client: ClientId,
        sheet: u32,
        accepted: bool,
        serial: u64,
    ) -> Result<(), ErrorCode> {
        if !self.clients[&client].shell {
            return Err(ErrorCode::Unauthorized);
        }
        // An acceptance is the person's only if it cites an input the shell received, and never
        // while locked; a refusal stands whatever it cites.
        if accepted && (self.locked || !self.recent_input(client, serial)) {
            return Ok(());
        }
        if self.system.sheets.remove(&sheet) {
            self.system.answered.push(SheetAnswered { sheet, accepted });
        }
        Ok(())
    }

    /// The sheets answered since the last call.
    pub fn take_sheet_answers(&mut self) -> Vec<SheetAnswered> {
        std::mem::take(&mut self.system.answered)
    }

    pub(super) fn capability_request(
        &mut self,
        client: ClientId,
        serial: u32,
        input: u64,
        capability: u8,
    ) -> Result<(), ErrorCode> {
        if !(1..=5).contains(&capability) {
            return Err(ErrorCode::InvalidMessage);
        }
        if self.system.capabilities.len() >= MAX_PENDING {
            return Err(ErrorCode::QuotaExceeded);
        }
        let with_input = input != 0 && self.recent_input(client, input);
        self.system.capabilities.push(CapabilityRequest {
            client,
            serial,
            capability,
            with_input,
        });
        Ok(())
    }

    /// The capability requests asked for since the last call.
    pub fn take_capability_requests(&mut self) -> Vec<CapabilityRequest> {
        std::mem::take(&mut self.system.capabilities)
    }

    /// Answers an application's capability request.
    pub fn capability_result(&mut self, client: ClientId, serial: u32, granted: bool) {
        if self.clients.contains_key(&client) {
            self.emit(client, Event::CapabilityResult { serial, granted });
        }
    }

    pub(super) fn forget_system_requests(&mut self, client: ClientId) {
        let system = &mut self.system;
        system.receiving.retain(|(asker, _), _| *asker != client);
        system.asked.retain(|request| request.client != client);
        system
            .capabilities
            .retain(|request| request.client != client);
    }
}
