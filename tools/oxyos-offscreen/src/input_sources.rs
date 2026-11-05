//! `--input-sources-available METHOD,...`: the development compositor plays what the session
//! tells of the input sources (docs/adr/0041-input-methods.mdx, decision 6), for Settings'
//! Keyboard › Input Sources scenarios, with an engine that has the input methods named (none
//! given: none installed). Nothing types with them: it is the state alone, from the run's
//! preferences (`input_sources`, `input_source`) and the run's layout (`--layout`).
//!
//! What it tells, as the session's service will: `{"inputSources":{"layout","current","sources",
//! "available"}}`, the sources in effect with the layout first (an added method the engine lacks
//! is not in effect, and the layout is current in its place). Told unasked (serial 0) to each
//! client holding `settings` or `shell` once it holds it and whenever the state changes, and as
//! the answer to `{"inputSources":{}}`.
//!
//! And what the shell's top bar asks, as the service answers it, citing an input: a source chosen
//! (`{"selectInputSource":{"id"}}`) is kept as the `input_source` preference and answered with the
//! new state; the emoji picker asked for (`{"showEmoji":{}}`) is answered `{"emoji":{}}` and
//! counted. Refusals are the service's: `invalidRequest`, `notPermitted`, `needsInput`,
//! `noSuchInputSource`. Each step reports the source in effect, the sources chosen in it and the
//! emoji pickers asked for (`json`).

use oxyos_compositor::settings::Settings;
use oxyos_compositor::Compositor;
use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::ClientId;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub struct InputSources {
    available: Vec<String>,
    layout: String,
    /// The clients told the state so far, and the state last told.
    told: BTreeSet<ClientId>,
    last: Option<String>,
    /// The sources chosen and the emoji pickers asked for since the step began.
    pub selected: Vec<String>,
    pub emoji: usize,
}

/// The vocabulary's names, the service's: a request naming one of them is this fixture's.
const ASKS: [&str; 3] = ["inputSources", "selectInputSource", "showEmoji"];

enum Ask {
    InputSources,
    SelectInputSource { id: String },
    ShowEmoji,
}

/// A request read as strictly as the service reads it: one ask, with exactly its fields.
fn ask(text: &str) -> Option<Ask> {
    let asked = serde_json::from_str::<serde_json::Map<String, Value>>(text).ok()?;
    let (name, fields) = asked.iter().next().filter(|_| asked.len() == 1)?;
    let fields = fields.as_object()?;
    match name.as_str() {
        "inputSources" if fields.is_empty() => Some(Ask::InputSources),
        "showEmoji" if fields.is_empty() => Some(Ask::ShowEmoji),
        "selectInputSource" if fields.len() == 1 => Some(Ask::SelectInputSource {
            id: fields.get("id")?.as_str()?.to_owned(),
        }),
        _ => None,
    }
}

/// Whether a request is this fixture's: one object naming one of its asks.
fn ours(request: &SystemRequest) -> bool {
    serde_json::from_str::<serde_json::Map<String, Value>>(&request.text)
        .is_ok_and(|asked| asked.len() == 1 && asked.keys().all(|key| ASKS.contains(&key.as_str())))
}

impl InputSources {
    /// The fixture with these input methods installed (each one the preferences know, once).
    pub fn new(available: &str, layout: &str) -> Result<Self, String> {
        let mut methods: Vec<String> = Vec::new();
        for id in available.split(',').filter(|id| !id.is_empty()) {
            if !oxyos_preferences::INPUT_METHODS.contains(&id) || methods.iter().any(|m| m == id) {
                return Err(format!(
                    "--input-sources-available: {id:?} is not an input method, or is named twice"
                ));
            }
            methods.push(id.to_owned());
        }
        Ok(Self {
            available: methods,
            layout: layout.to_owned(),
            told: BTreeSet::new(),
            last: None,
            selected: Vec::new(),
            emoji: 0,
        })
    }

    /// The source in effect now, the sources chosen in the step and the emoji pickers asked for,
    /// as JSON.
    pub fn json(&self) -> String {
        let current = self
            .last
            .as_deref()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .map_or(Value::Null, |told| told["inputSources"]["current"].clone());
        json!({"current": current, "selected": self.selected, "emoji": self.emoji}).to_string()
    }

    /// The state, as told, from the preferences kept now.
    fn text(&self, kept: &[String], current: Option<&str>) -> String {
        let in_effect: Vec<&str> = kept
            .iter()
            .map(String::as_str)
            .filter(|id| self.available.iter().any(|m| m == id))
            .collect();
        let current = current
            .filter(|id| in_effect.contains(id))
            .unwrap_or(oxyos_preferences::LAYOUT_SOURCE);
        let mut sources = vec![oxyos_preferences::LAYOUT_SOURCE];
        sources.extend(in_effect);
        json!({"inputSources": {
            "layout": self.layout, "current": current, "sources": sources, "available": self.available,
        }})
        .to_string()
    }

    /// A turn: requests answered (a source chosen kept as the person's preference), new holders
    /// told, and everyone told a change.
    pub fn serve(
        &mut self,
        compositor: &mut Compositor,
        settings: &mut Settings,
        clients: &[ClientId],
        now: i64,
    ) {
        for request in compositor.scene.take_system_requests_where(ours) {
            let SystemRequest {
                client,
                serial,
                with_input,
                text,
            } = request;
            let refused = |reason: &str| json!({"refused": {"reason": reason}}).to_string();
            let kept = settings.preferences();
            let state = self.text(&kept.input_sources, kept.input_source.as_deref());
            let answer = match ask(&text).ok_or(()) {
                Err(()) => refused("invalidRequest"),
                Ok(_)
                    if !compositor
                        .scene
                        .grants_of(client)
                        .is_some_and(|g| g.settings || g.shell) =>
                {
                    refused("notPermitted")
                }
                Ok(Ask::InputSources) => state,
                Ok(_) if !with_input => refused("needsInput"),
                Ok(Ask::ShowEmoji) => {
                    self.emoji += 1;
                    json!({"emoji": {}}).to_string()
                }
                Ok(Ask::SelectInputSource { id }) => {
                    let in_effect = serde_json::from_str::<Value>(&state).is_ok_and(|told| {
                        told["inputSources"]["sources"]
                            .as_array()
                            .is_some_and(|sources| sources.contains(&json!(id)))
                    });
                    if in_effect {
                        if let Err(error) =
                            settings.set_preference(compositor, "input_source", &id, now)
                        {
                            eprintln!("[input sources] input_source={id}: {error}");
                        }
                        self.selected.push(id);
                        let kept = settings.preferences();
                        self.text(&kept.input_sources, kept.input_source.as_deref())
                    } else {
                        refused("noSuchInputSource")
                    }
                }
            };
            compositor.scene.system_text(client, serial, &answer);
        }
        let kept = settings.preferences();
        let text = self.text(&kept.input_sources, kept.input_source.as_deref());
        let scene = &mut compositor.scene;
        let changed = self.last.as_deref() != Some(text.as_str());
        let holders: BTreeSet<ClientId> = clients
            .iter()
            .copied()
            .filter(|client| {
                scene
                    .grants_of(*client)
                    .is_some_and(|g| g.settings || g.shell)
            })
            .collect();
        for client in &holders {
            if changed || !self.told.contains(client) {
                scene.system_text(*client, 0, &text);
            }
        }
        self.told = holders;
        self.last = Some(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_has_only_the_input_methods_the_preferences_know_each_once() {
        assert!(InputSources::new("pinyin,japanese,korean", "es").is_ok());
        assert!(InputSources::new("", "es").is_ok());
        assert!(InputSources::new("rime", "es").is_err());
        assert!(InputSources::new("pinyin,pinyin", "es").is_err());
    }

    #[test]
    fn the_sources_in_effect_are_those_the_engine_has_the_layout_first() {
        let fixture = InputSources::new("pinyin,korean", "es").unwrap();
        let kept = ["japanese".to_owned(), "korean".to_owned()];
        assert_eq!(
            fixture.text(&kept, Some("korean")),
            r#"{"inputSources":{"available":["pinyin","korean"],"current":"korean","layout":"es","sources":["layout","korean"]}}"#
        );
        // A current one not in effect: the layout is current.
        assert!(fixture
            .text(&kept, Some("japanese"))
            .contains(r#""current":"layout""#));
    }
}
