//! The user's preferences (docs/adr/0019-settings.mdx). Settings (its `settings` grant) asks for a
//! change, `key=value` in chunks, handed whole to the program running the compositor, which
//! validates it, keeps it, applies it and answers. That program tells the scene what the
//! preferences are: every client is told the ones that change how it draws, when it connects and
//! whenever they change; a client with the grant is told every one as it is kept. The scene knows
//! no key and keeps no file.

use super::launch::assemble;
use super::{ClientId, Scene};
use oxyos_protocol::{ErrorCode, Event, PreeditText, TextChunk, MAX_PREFERENCE_BYTES};
use std::collections::BTreeMap;
/// Changes asked for and not yet taken.
const MAX_PENDING_CHANGES: usize = 16;

/// What every client is told: the preferences that change how it draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawingPreferences {
    /// The appearance, an automatic one resolved by the program running the compositor.
    pub dark: bool,
    pub reduce_motion: bool,
    pub reduce_transparency: bool,
    /// Bloom's accent colour, by name: one chunk.
    pub accent: PreeditText,
}

/// A change Settings asked for: whose request (`serial`), the preference and its new value's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreferenceChange {
    pub client: ClientId,
    pub serial: u32,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Default)]
pub(super) struct Preferences {
    /// Changes being received, by the client's request: the text so far and the next chunk's index.
    receiving: BTreeMap<(ClientId, u32), (String, u8)>,
    asked: Vec<PreferenceChange>,
    /// What the program running the compositor last said, if it said anything.
    drawing: Option<DrawingPreferences>,
    /// Every preference as it is kept, `(key, value)`, in the program's order.
    values: Vec<(String, String)>,
}

impl Scene {
    pub(super) fn preference_request(
        &mut self,
        client: ClientId,
        serial: u32,
        index: u8,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        if !self.clients[&client].settings {
            return Err(ErrorCode::Unauthorized);
        }
        let preferences = &mut self.preferences;
        let Some(text) = assemble(
            &mut preferences.receiving,
            (client, serial),
            index,
            last,
            text,
            MAX_PREFERENCE_BYTES,
        )?
        else {
            return Ok(());
        };
        // Not a change at all: refused here, with nothing for the program to judge.
        let Some((key, value)) = text.split_once('=') else {
            self.preference_set(client, serial, false);
            return Ok(());
        };
        if preferences.asked.len() >= MAX_PENDING_CHANGES {
            return Err(ErrorCode::QuotaExceeded);
        }
        preferences.asked.push(PreferenceChange {
            client,
            serial,
            key: key.to_string(),
            value: value.to_string(),
        });
        Ok(())
    }

    /// The changes Settings asked for since the last call, in order: the program running the
    /// compositor validates, keeps and applies each, answers it with [`Scene::preference_set`] and
    /// tells the scene the preferences then ([`Scene::set_preferences`]).
    pub fn take_preference_changes(&mut self) -> Vec<PreferenceChange> {
        std::mem::take(&mut self.preferences.asked)
    }

    /// Tells the client whether the change its request `serial` asked for was taken.
    pub fn preference_set(&mut self, client: ClientId, serial: u32, accepted: bool) {
        self.emit(client, Event::PreferenceResult { serial, accepted });
    }

    /// What the preferences are, from the program running the compositor, which alone keeps
    /// them: `drawing`, told to every client when it changes, and `values`, every preference as
    /// kept (`key`, `value`), each one that changes told to the clients with the `settings` grant.
    /// A `key=value` longer than [`MAX_PREFERENCE_BYTES`] is not told.
    pub fn set_preferences(&mut self, drawing: DrawingPreferences, values: &[(String, String)]) {
        if self.preferences.drawing != Some(drawing) {
            self.preferences.drawing = Some(drawing);
            let clients: Vec<ClientId> = self.clients.keys().copied().collect();
            for client in clients {
                self.tell_drawing(client);
            }
        }
        let changed: Vec<(String, String)> = values
            .iter()
            .filter(|(key, value)| {
                !self
                    .preferences
                    .values
                    .iter()
                    .any(|(k, v)| k == key && v == value)
            })
            .cloned()
            .collect();
        self.preferences.values = values.to_vec();
        let settings: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, grants)| grants.settings)
            .map(|(client, _)| *client)
            .collect();
        for client in settings {
            for (key, value) in &changed {
                self.tell_preference(client, key, value);
            }
        }
    }

    /// The drawing preferences, to `client`, once the program running the compositor said them.
    pub(super) fn tell_drawing(&mut self, client: ClientId) {
        if let Some(drawing) = self.preferences.drawing {
            self.emit(
                client,
                Event::Preferences {
                    dark: drawing.dark,
                    reduce_motion: drawing.reduce_motion,
                    reduce_transparency: drawing.reduce_transparency,
                    accent: drawing.accent,
                },
            );
        }
    }

    /// Every preference as kept, to a client given the `settings` grant.
    pub(super) fn tell_preferences(&mut self, client: ClientId) {
        for (key, value) in self.preferences.values.clone() {
            self.tell_preference(client, &key, &value);
        }
    }

    fn tell_preference(&mut self, client: ClientId, key: &str, value: &str) {
        let text = format!("{key}={value}");
        if text.len() > MAX_PREFERENCE_BYTES {
            return;
        }
        let chunks = TextChunk::split(&text);
        let count = chunks.len();
        for (index, chunk) in chunks.into_iter().enumerate() {
            self.emit(
                client,
                Event::Preference {
                    index: index as u8,
                    last: index + 1 == count,
                    text: PreeditText::new(chunk.as_str()).expect("a chunk fits"),
                },
            );
        }
    }

    pub(super) fn forget_preference_changes(&mut self, client: ClientId) {
        let preferences = &mut self.preferences;
        preferences
            .receiving
            .retain(|(asker, _), _| *asker != client);
        preferences.asked.retain(|change| change.client != client);
    }
}
