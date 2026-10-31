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

/// A chunk's index: a byte for short texts, two for long ones (a list, a URL).
pub(super) trait ChunkIndex: Copy + PartialEq {
    const FIRST: Self;
    fn next(self) -> Self;
}

impl ChunkIndex for u8 {
    const FIRST: Self = 0;
    fn next(self) -> Self {
        self.wrapping_add(1)
    }
}

impl ChunkIndex for u16 {
    const FIRST: Self = 0;
    fn next(self) -> Self {
        self.wrapping_add(1)
    }
}

/// Text a client sends in chunks for one of its requests (`request`, its serial): `index` 0 begins
/// it, each next chunk follows in order, `last` completes it, and the whole text then. A chunk out
/// of order or text beyond `max` bytes is a protocol error, and what was received is dropped.
pub(super) fn assemble<K: Ord + Copy, I: ChunkIndex>(
    receiving: &mut BTreeMap<K, (String, I)>,
    request: K,
    index: I,
    last: bool,
    text: PreeditText,
    max: usize,
) -> Result<Option<String>, ErrorCode> {
    let entry = receiving
        .entry(request)
        .or_insert_with(|| (String::new(), I::FIRST));
    if index == I::FIRST {
        *entry = (String::new(), I::FIRST);
    } else if index != entry.1 {
        receiving.remove(&request);
        return Err(ErrorCode::InvalidOrder);
    }
    if entry.0.len() + text.as_str().len() > max {
        receiving.remove(&request);
        return Err(ErrorCode::QuotaExceeded);
    }
    entry.0.push_str(text.as_str());
    entry.1 = index.next();
    if !last {
        return Ok(None);
    }
    Ok(receiving.remove(&request).map(|(text, _)| text))
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
        let Some(id) = assemble(
            &mut launches.receiving,
            (client, serial),
            index,
            last,
            text,
            MAX_APP_ID_BYTES,
        )?
        else {
            return Ok(());
        };
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
