//! Login items and background activity (docs/adr/0060-login-items-and-background-activity.mdx).
//!
//! An application asks the session how one of its background items stands, or registers or
//! unregisters it (`BackgroundItem`), as a Mac's application asks `SMAppService`; the scene hands
//! each ask to the program running the compositor, which decides by the client's launch record and
//! its account's decisions, never by what the client claims, and answers (`BackgroundItemState`).
//! The program also tells an application unasked (serial 0) when the person changed one of its
//! items. The scene knows no application and no capability.

use super::{ClientId, Scene};
use oxyos_protocol::{
    BackgroundItemAction, BackgroundItemKind, BackgroundStatus, ErrorCode, Event,
};

/// The first protocol version with background items: a client of an older one is never told one,
/// and could not have asked.
pub const BACKGROUND_VERSION: u16 = 32;
/// Asks taken and not yet handed to the program; past it, a client is refused.
const MAX_PENDING: usize = 32;

/// An application's ask about one of its background items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackgroundItemRequest {
    pub client: ClientId,
    pub serial: u32,
    pub kind: BackgroundItemKind,
    pub action: BackgroundItemAction,
}

#[derive(Debug, Default)]
pub(super) struct Background {
    asked: Vec<BackgroundItemRequest>,
    /// The clients whose application the person quit (the dock's Quit, the switcher's, Super+Q)
    /// since the program last took them: their windows were asked to close, and a process that
    /// would run on in the background ends with them instead.
    quit: Vec<ClientId>,
}

impl Scene {
    pub(super) fn background_item_request(
        &mut self,
        client: ClientId,
        serial: u32,
        kind: BackgroundItemKind,
        action: BackgroundItemAction,
    ) -> Result<(), ErrorCode> {
        if self.background.asked.len() >= MAX_PENDING {
            return Err(ErrorCode::QuotaExceeded);
        }
        self.background.asked.push(BackgroundItemRequest {
            client,
            serial,
            kind,
            action,
        });
        Ok(())
    }

    /// The asks made since the last call, in order.
    pub fn take_background_item_requests(&mut self) -> Vec<BackgroundItemRequest> {
        std::mem::take(&mut self.background.asked)
    }

    /// The person quit the application of `client`'s window: its windows are asked to close, as
    /// any quit asks them, and the program is told, so a process of it allowed in the background
    /// ends once they have closed rather than running on: Quit ends an application, closing its
    /// window does not.
    pub(super) fn note_quit(&mut self, client: ClientId) {
        if !self.background.quit.contains(&client) {
            self.background.quit.push(client);
        }
    }

    /// The clients whose application the person quit since the last call.
    pub fn take_quits(&mut self) -> Vec<ClientId> {
        std::mem::take(&mut self.background.quit)
    }

    /// Tells `client` how its background item `kind` stands: the answer to its ask `serial`, or,
    /// with serial 0, a change the person made. Nothing for a client gone or of an older version.
    pub fn background_item_state(
        &mut self,
        client: ClientId,
        serial: u32,
        kind: BackgroundItemKind,
        status: BackgroundStatus,
    ) {
        if !self.clients.contains_key(&client)
            || self
                .version_of(client)
                .is_some_and(|version| version < BACKGROUND_VERSION)
        {
            return;
        }
        self.emit(
            client,
            Event::BackgroundItemState {
                serial,
                kind,
                status,
            },
        );
    }

    pub(super) fn forget_background_items(&mut self, client: ClientId) {
        self.background.asked.retain(|ask| ask.client != client);
        self.background.quit.retain(|quit| *quit != client);
    }
}
