//! Safe Mode (docs/adr/0054-recovery-and-erasing.mdx, decision 10): the program running the
//! compositor says the session started in it; its shell and its login screen (a client with the
//! `shell` or the `lock` grant) are told, now and as each connects, and show it until the session
//! ends. Nothing else is: an application's session is as any other.

use super::{ClientId, Scene};
use oxyos_protocol::Event;

/// The first protocol version with Safe Mode's event: a client of an older one is never told it.
pub const SAFE_MODE_VERSION: u16 = 34;

impl Scene {
    /// The session started in Safe Mode, for its whole life: every shell and lock screen is told.
    pub fn set_safe_mode(&mut self) {
        if self.safe_mode {
            return;
        }
        self.safe_mode = true;
        let told: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, grants)| grants.shell || grants.lock)
            .map(|(client, _)| *client)
            .collect();
        for client in told {
            self.tell_safe_mode(client);
        }
    }

    /// Whether the session started in Safe Mode.
    pub fn safe_mode(&self) -> bool {
        self.safe_mode
    }

    /// Tells a shell or a lock screen that the session is in Safe Mode, if it is and it speaks it.
    pub(super) fn tell_safe_mode(&mut self, client: ClientId) {
        if !self.safe_mode
            || self
                .version_of(client)
                .is_some_and(|version| version < SAFE_MODE_VERSION)
        {
            return;
        }
        self.emit(client, Event::SafeMode {});
    }
}
