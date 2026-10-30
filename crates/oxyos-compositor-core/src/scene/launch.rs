//! The shell asking for an application (docs/adr/0013-launching-applications.mdx): its id, in
//! chunks, handed whole to the program running the compositor, which starts it if its registry
//! lists it and answers. The scene starts nothing and knows no registry.

use super::{ClientId, Scene, MAX_APP_ID_BYTES};
use oxyos_protocol::{ErrorCode, Event, PreeditText};
use std::collections::BTreeMap;

/// Launches asked for and not yet taken.
const MAX_PENDING_LAUNCHES: usize = 16;

/// An application the shell asked for: whose request (`serial`) and its id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    pub client: ClientId,
    pub serial: u32,
    pub id: String,
}

#[derive(Debug, Default)]
pub(super) struct Launches {
    /// Ids being received, by the shell's request: the text so far and the next chunk's index.
    receiving: BTreeMap<(ClientId, u32), (String, u8)>,
    asked: Vec<LaunchRequest>,
}

impl Scene {
    pub(super) fn launch_request(
        &mut self,
        client: ClientId,
        serial: u32,
        index: u8,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        if !self.clients[&client].shell {
            return Err(ErrorCode::Unauthorized);
        }
        let launches = &mut self.launches;
        let entry = launches
            .receiving
            .entry((client, serial))
            .or_insert_with(|| (String::new(), 0));
        if index == 0 {
            *entry = (String::new(), 0);
        } else if index != entry.1 {
            launches.receiving.remove(&(client, serial));
            return Err(ErrorCode::InvalidOrder);
        }
        if entry.0.len() + text.as_str().len() > MAX_APP_ID_BYTES {
            launches.receiving.remove(&(client, serial));
            return Err(ErrorCode::QuotaExceeded);
        }
        entry.0.push_str(text.as_str());
        entry.1 = index.wrapping_add(1);
        if !last {
            return Ok(());
        }
        let (id, _) = launches
            .receiving
            .remove(&(client, serial))
            .expect("just received");
        if launches.asked.len() >= MAX_PENDING_LAUNCHES {
            return Err(ErrorCode::QuotaExceeded);
        }
        launches.asked.push(LaunchRequest { client, serial, id });
        Ok(())
    }

    /// The applications the shell asked for since the last call, in order: the program running
    /// the compositor starts those its registry lists and answers each with [`Scene::launched`].
    pub fn take_launches(&mut self) -> Vec<LaunchRequest> {
        std::mem::take(&mut self.launches.asked)
    }

    /// Tells the shell whether the application its request `serial` asked for was started.
    pub fn launched(&mut self, client: ClientId, serial: u32, launched: bool) {
        self.emit(client, Event::LaunchResult { serial, launched });
    }

    pub(super) fn forget_launches(&mut self, client: ClientId) {
        let launches = &mut self.launches;
        launches.receiving.retain(|(asker, _), _| *asker != client);
        launches.asked.retain(|request| request.client != client);
    }
}
