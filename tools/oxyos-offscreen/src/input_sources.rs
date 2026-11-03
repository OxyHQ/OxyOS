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

use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::{ClientId, Scene};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub struct InputSources {
    available: Vec<String>,
    layout: String,
    /// The clients told the state so far, and the state last told.
    told: BTreeSet<ClientId>,
    last: Option<String>,
}

/// Whether a request is this fixture's: exactly `{"inputSources":{}}`.
fn ours(request: &SystemRequest) -> bool {
    serde_json::from_str::<Value>(&request.text)
        .is_ok_and(|asked| asked == json!({"inputSources": {}}))
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
        })
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

    /// A turn: requests answered, new holders told, and everyone told a change.
    pub fn serve(
        &mut self,
        scene: &mut Scene,
        clients: &[ClientId],
        kept: &[String],
        current: Option<&str>,
    ) {
        let text = self.text(kept, current);
        for request in scene.take_system_requests_where(ours) {
            let allowed = scene
                .grants_of(request.client)
                .is_some_and(|g| g.settings || g.shell);
            let answer = if allowed {
                text.clone()
            } else {
                r#"{"refused":{"reason":"notPermitted"}}"#.to_owned()
            };
            scene.system_text(request.client, request.serial, &answer);
        }
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
