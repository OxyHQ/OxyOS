//! Search (docs/adr/0051-search.mdx).
//!
//! Any client asks the session's search index something in the index's vocabulary (JSON text in
//! chunks, `SearchRequest`); the scene assembles it and hands it whole to the program running the
//! compositor, which attaches who asked (only the compositor knows a client's grants and the
//! launch that started it) and passes it to the index; the index's answers come back as
//! `SearchText` naming the request's serial, and serial 0 tells a client unasked that what it asked
//! to watch changed. The program is told the clients that asked and went, so the index forgets
//! what it kept for them. The scene knows no vocabulary.

use super::{ClientId, Scene};
use oxyos_protocol::{ErrorCode, Event, PreeditText, TextChunk, MAX_SEARCH_TEXT_BYTES};
use std::collections::{BTreeMap, BTreeSet};

/// The first protocol version with search: a client of an older one is never sent its answers,
/// and could not have asked.
pub const SEARCH_VERSION: u16 = 30;
/// Asks assembled and not yet taken by the program; one more is answered busy.
pub const MAX_SEARCH_ASKS: usize = 64;
/// Asks one client may be sending in chunks at once.
const MAX_RECEIVING: usize = 16;
/// What an ask the scene cannot take now is answered.
const BUSY: &str = r#"{"refused":{"reason":"busy"}}"#;

/// A client's request to the search index, whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchAsk {
    pub client: ClientId,
    pub serial: u32,
    pub text: String,
}

#[derive(Debug, Default)]
pub(super) struct Search {
    /// The asks arriving in chunks: what came so far and the next chunk's index.
    receiving: BTreeMap<(ClientId, u32), (String, u16)>,
    asked: Vec<SearchAsk>,
    /// The connected clients that asked anything: when one goes, the program is told.
    askers: BTreeSet<ClientId>,
    gone: Vec<ClientId>,
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
    pub(super) fn search_request(
        &mut self,
        client: ClientId,
        serial: u32,
        index: u16,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        let receiving = &mut self.search.receiving;
        if index == 0
            && !receiving.contains_key(&(client, serial))
            && receiving
                .keys()
                .filter(|(asker, _)| *asker == client)
                .count()
                >= MAX_RECEIVING
        {
            return Err(ErrorCode::QuotaExceeded);
        }
        let entry = receiving
            .entry((client, serial))
            .or_insert_with(|| (String::new(), 0));
        if index == 0 {
            *entry = (String::new(), 0);
        } else if index != entry.1 {
            receiving.remove(&(client, serial));
            return Err(ErrorCode::InvalidOrder);
        }
        if entry.0.len() + text.as_str().len() > MAX_SEARCH_TEXT_BYTES {
            receiving.remove(&(client, serial));
            return Err(ErrorCode::QuotaExceeded);
        }
        entry.0.push_str(text.as_str());
        entry.1 = index.wrapping_add(1);
        if !last {
            return Ok(());
        }
        let (text, _) = receiving.remove(&(client, serial)).expect("received");
        // Many clients asking within one turn is no client's fault: the one past the limit is
        // answered busy and asks again, and none is cut off.
        if self.search.asked.len() >= MAX_SEARCH_ASKS {
            self.search_text(client, serial, BUSY);
            return Ok(());
        }
        self.search.askers.insert(client);
        self.search.asked.push(SearchAsk {
            client,
            serial,
            text,
        });
        Ok(())
    }

    /// The asks made since the last call, in order.
    pub fn take_search_asks(&mut self) -> Vec<SearchAsk> {
        std::mem::take(&mut self.search.asked)
    }

    /// Tells `client` the index's answer to its ask `serial` (0: what it watches changed,
    /// unasked), if it is still connected and speaks search.
    pub fn search_text(&mut self, client: ClientId, serial: u32, text: &str) {
        if !self.clients.contains_key(&client) || text.len() > MAX_SEARCH_TEXT_BYTES {
            return;
        }
        if self
            .version_of(client)
            .is_some_and(|version| version < SEARCH_VERSION)
        {
            return;
        }
        let parts = chunks(text);
        let count = parts.len();
        for (index, text) in parts.into_iter().enumerate() {
            self.emit(
                client,
                Event::SearchText {
                    serial,
                    index: index as u16,
                    last: index + 1 == count,
                    text,
                },
            );
        }
    }

    /// The clients that asked the index anything and disconnected since the last call: the index
    /// forgets what it kept for each (what it watched, its asks not answered).
    pub fn take_gone_searchers(&mut self) -> Vec<ClientId> {
        std::mem::take(&mut self.search.gone)
    }

    pub(super) fn forget_searcher(&mut self, client: ClientId) {
        let search = &mut self.search;
        search.receiving.retain(|(asker, _), _| *asker != client);
        search.asked.retain(|ask| ask.client != client);
        if search.askers.remove(&client) {
            search.gone.push(client);
        }
    }
}
