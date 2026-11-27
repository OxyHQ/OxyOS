//! Input methods for the program running the compositor (docs/adr/0041-input-methods.mdx): the
//! person's input sources, Super+Alt+Space, emoji, and the session as fcitx5's frontend.
//!
//! The frontend is a client of the scene in the program's own process, holding `input_method`
//! (ADR 0008): it is connected only while an input method is the current input source (or emoji
//! is being searched), so with the keyboard layout current, keys type with no round trip. What the
//! scene tells an input method goes to the engine ([`oxyos_input_methods::Service`], on a thread
//! of its own); what the engine does comes back as the input method's requests: a key's answer
//! after its transaction, and the candidates handed to the system, which the shell draws.
//!
//! The input sources are the person's preferences (`input_sources`, `input_source`); the shell and
//! Settings are told them (`SystemText` `inputSources`) and the shell chooses one from its menu
//! (`selectInputSource`) or asks for emoji (`showEmoji`), citing an input.

use crate::settings::Settings;
use crate::Compositor;
use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::{ClientId, Grants, InputSourceShortcut, Scene};
use oxyos_input_methods::{
    capability, fcitx_name, Change, Command, Engine, Outcome, Panel, Paths, Service, Sources,
};
use oxyos_preferences::{Preferences, LAYOUT_SOURCE};
use oxyos_protocol::{
    Event, PreeditStyle, Request, TextChunk, TextHints, TextPurpose, CANDIDATES_NEXT,
    CANDIDATES_PREVIOUS, CANDIDATES_VERTICAL, MAX_CANDIDATES_AUX_BYTES, MAX_CANDIDATE_BYTES,
    MAX_INPUT_CANDIDATES, NO_CANDIDATE,
};
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::os::fd::OwnedFd;

/// The session's input method, a client of the scene in this process: the middle of the ids,
/// which nothing else takes (applications count up from 1, shells down from the top).
pub const CLIENT: ClientId = ClientId::MAX / 2;

/// What the shell and Settings may ask (`SystemRequest`).
const ASKS: [&str; 3] = ["inputSources", "selectInputSource", "showEmoji"];

fn ours(text: &str) -> bool {
    serde_json::from_str::<HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.keys().all(|key| ASKS.contains(&key.as_str())))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Ask {
    /// The state, answered as it is told (`{"inputSources": …}`).
    InputSources {},
    SelectInputSource {
        id: String,
    },
    ShowEmoji {},
}

/// What the scene told of the field served, for the engine and for turning its character
/// offsets into the protocol's bytes.
#[derive(Debug, Default)]
struct Field {
    active: bool,
    /// Activated and not yet told the engine (its state comes before its Done).
    starting: bool,
    purpose: Option<TextPurpose>,
    hints: TextHints,
    receiving: String,
    text: String,
    anchor: u32,
    cursor: u32,
    /// The field's commit serial the engine's changes are for.
    serial: u32,
}

/// The field's state while the engine composes in it: its text, anchor and cursor, which never
/// hold the composition (ADR 0008, decision 1).
type Told = (String, u32, u32);

/// Whether the application took the engine's composition as text itself: a press elsewhere in the
/// field or an editing key ends it there, as Android's `finishComposingText` does (ADR 0037,
/// decision 2), and the engine, still holding it, would put it in again with the next key. While
/// the engine composes, the field tells the state the engine's changes leave it in: the same text
/// and caret while only the composition changes, and the text its commit and deletions made. Any
/// other state is the application's, and the engine forgets the composition (`Command::Reset`).
#[derive(Debug, Default)]
struct Composing {
    /// What the field should tell while the engine composes; none while it composes nothing.
    expected: Option<Told>,
    /// What it told before the engine's last change: a field may tell it once more, sent before
    /// that change reached it.
    before: Option<Told>,
}

impl Composing {
    /// The engine's change, sent for the field told `field`: what it deleted around the
    /// selection, in bytes, what it committed, and whether it composes after it.
    fn sent(&mut self, field: Told, (before, after): (u32, u32), commit: &str, composing: bool) {
        if !composing {
            *self = Self::default();
            return;
        }
        // A composition replaces the selection, or the composition before it, which the field
        // holds as a caret where it begins.
        let basis = self.expected.take().unwrap_or(field);
        let (text, anchor, cursor) = &basis;
        let from = (*anchor.min(cursor) as usize).min(text.len());
        let to = (*anchor.max(cursor) as usize).min(text.len());
        let start = from.saturating_sub(before as usize);
        let end = (to + after as usize).min(text.len());
        let (Some(head), Some(tail)) = (text.get(..start), text.get(end..)) else {
            // Offsets inside a character: the field refuses such a change; nothing to expect.
            self.before = None;
            return;
        };
        let caret = (start + commit.len()) as u32;
        self.expected = Some((format!("{head}{commit}{tail}"), caret, caret));
        self.before = Some(basis);
    }

    /// The field told `field`: true when the application took the composition as text.
    fn told(&mut self, field: &Told) -> bool {
        let Some(expected) = &self.expected else {
            return false;
        };
        if field == expected || self.before.as_ref() == Some(field) {
            return false;
        }
        *self = Self::default();
        true
    }
}

/// Emoji searched for in a field (Control+Super+Space).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Emoji {
    /// Asked for; QuickPhrase starts once a field is served.
    Wanted,
    /// QuickPhrase open; `shown` once its prompt or candidates were shown.
    Searching { shown: bool },
}

pub struct InputMethods {
    engine: Engine,
    service: Service,
    available: Vec<&'static str>,
    sources: Vec<String>,
    current: Option<String>,
    /// Every source, the most recently used first: what Super+Alt+Space goes back through.
    recent: Vec<String>,
    /// The order a held Super+Alt+Space goes through, as it was when the hold began.
    cycle: Vec<String>,
    layout: String,
    language: String,
    connected: bool,
    field: Field,
    /// The key symbol the scene told before its key.
    symbol: Option<(u64, u32, u32)>,
    /// The keys sent to the engine and not yet answered: their code and whether pressed.
    keys: std::collections::BTreeMap<u64, (u32, bool)>,
    /// Keys whose press the engine took: their release is taken too, so no application ever
    /// sees half of a key the engine composed with.
    taken: BTreeSet<u32>,
    /// The composition the field has, given again in a change that leaves it as it was.
    preedit: Option<oxyos_input_methods::Preedit>,
    /// Whether the field still holds that composition as the engine's.
    composing: Composing,
    emoji: Option<Emoji>,
    /// Whether the engine is running (until then keys type with the layout).
    ready: bool,
    /// Switches told the engine and not yet done: until they are, the frontend stays connected,
    /// so what the input method left commits into the field (pinyin's composition, as a Mac's).
    switching: usize,
    /// The state told, and to whom.
    told: Option<String>,
    told_to: BTreeSet<ClientId>,
}

/// The XKB layout of the session's (`layout\tvariant\toptions`): its first layout.
fn layout_of(scene: &Scene) -> String {
    scene
        .keyboard_layout()
        .and_then(|told| told.split('\t').next())
        .and_then(|layouts| layouts.split(',').next())
        .filter(|layout| !layout.is_empty())
        .unwrap_or("us")
        .to_owned()
}

/// fcitx5's keyboard engine for an XKB layout.
fn keyboard_engine(layout: &str) -> String {
    format!("keyboard-{layout}")
}

/// fcitx5's capability flags for a field's purpose and hints.
fn capabilities(purpose: Option<TextPurpose>, hints: TextHints) -> u64 {
    let mut flags = match purpose {
        Some(TextPurpose::Digits) => capability::DIGIT,
        Some(TextPurpose::Number) => capability::NUMBER,
        Some(TextPurpose::Phone) => capability::DIALABLE,
        Some(TextPurpose::Url) => capability::URL,
        Some(TextPurpose::Email) => capability::EMAIL,
        Some(TextPurpose::Name) => capability::NAME,
        Some(TextPurpose::Date) => capability::DATE,
        Some(TextPurpose::Time) => capability::TIME,
        _ => 0,
    };
    if hints.bits() & TextHints::MULTILINE != 0 {
        flags |= capability::MULTILINE;
    }
    if hints.bits() & TextHints::NO_PREDICTION != 0 {
        flags |= capability::NO_SPELL_CHECK;
    }
    flags
}

/// The characters before byte `offset` of `text` (the end, past it).
fn chars_before(text: &str, offset: u32) -> u32 {
    let offset = (offset as usize).min(text.len());
    let mut end = offset;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].chars().count() as u32
}

/// The bytes of `count` characters of `text` going back from byte `from` (backwards) or forward.
fn bytes_of(text: &str, from: u32, count: u32, backwards: bool) -> u32 {
    let from = (from as usize).min(text.len());
    let part = if backwards {
        &text[..from]
    } else {
        &text[from..]
    };
    let taken: Vec<char> = if backwards {
        part.chars().rev().take(count as usize).collect()
    } else {
        part.chars().take(count as usize).collect()
    };
    taken.iter().map(|c| c.len_utf8() as u32).sum()
}

/// A part of `text` of at most `max` bytes, at a whole character.
fn bounded(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Where the engine is installed (`tools/input-methods/build.sh`, installed by the image).
pub const INSTALLED: &str = "/usr/lib/oxyos/input-methods";

/// The language of a BCP 47 tag (`es-ES` → `es`).
pub fn language_of(locale: &str) -> &str {
    locale.split(['-', '_']).next().unwrap_or("en")
}

/// The session's input methods: the engine at `OXYOS_INPUT_METHODS`, else where the image
/// installs it, run in the session's runtime directory with the person's data. None when no
/// engine is installed (the keyboard types as before) or it cannot start.
pub fn for_session(
    compositor: &mut Compositor,
    settings: &Settings,
    runtime_dir: &std::path::Path,
    safe_mode: bool,
) -> Option<InputMethods> {
    // In Safe Mode the engine is the image's, whatever the environment names
    // (docs/adr/0054-recovery-and-erasing.mdx, decision 10).
    let prefix = std::env::var_os("OXYOS_INPUT_METHODS")
        .filter(|_| !safe_mode)
        .unwrap_or_else(|| INSTALLED.into());
    let Some(engine) = Engine::at(&prefix) else {
        eprintln!(
            "[input methods] no engine at {}: layouts only",
            std::path::Path::new(&prefix).display()
        );
        return None;
    };
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".local/share"))
        })
        .unwrap_or_else(|| runtime_dir.to_path_buf());
    let paths = Paths::for_session(runtime_dir, &data_home);
    start(compositor, settings, engine, paths)
}

/// The input methods with `engine` run in `paths`, the waker added to the compositor's turn.
pub fn start(
    compositor: &mut Compositor,
    settings: &Settings,
    engine: Engine,
    paths: Paths,
) -> Option<InputMethods> {
    let locale = settings.locale();
    match InputMethods::new(
        engine,
        paths,
        compositor,
        settings.preferences(),
        language_of(&locale),
    ) {
        Ok(methods) => match methods.wake_fd() {
            Ok(fd) => {
                compositor.wakers.push(fd);
                Some(methods)
            }
            Err(error) => {
                eprintln!("[input methods] {error}");
                None
            }
        },
        Err(error) => {
            eprintln!("[input methods] the engine: {error}");
            None
        }
    }
}

/// The person's input methods the engine has, and the current one if it is one of them.
fn chosen(preferences: &Preferences, available: &[&str]) -> (Vec<String>, Option<String>) {
    let sources: Vec<String> = preferences
        .input_sources
        .iter()
        .filter(|id| available.contains(&id.as_str()))
        .cloned()
        .collect();
    let current = preferences
        .input_source
        .clone()
        .filter(|id| sources.contains(id));
    (sources, current)
}

/// The keyboard engine emoji are searched in: the session's language's (ADR 0040), whose CLDR
/// annotations name them; English's otherwise.
fn emoji_engine(language: &str) -> String {
    match language {
        "es" => "keyboard-es".to_owned(),
        _ => "keyboard-us".to_owned(),
    }
}

/// The group fcitx5 has: the layout's keyboard engine, the language's when it is another (emoji
/// are searched in it), the person's input methods; and the one in use.
fn engine_sources(
    layout: &str,
    language: &str,
    sources: &[String],
    current: Option<&str>,
    emoji: bool,
) -> Sources {
    let mut group = vec![keyboard_engine(layout)];
    let searched = emoji_engine(language);
    if !group.contains(&searched) {
        group.push(searched.clone());
    }
    group.extend(
        sources
            .iter()
            .filter_map(|id| fcitx_name(id))
            .map(str::to_owned),
    );
    let current = if emoji {
        searched
    } else {
        current
            .and_then(fcitx_name)
            .map(str::to_owned)
            .unwrap_or_else(|| keyboard_engine(layout))
    };
    Sources {
        layout: layout.to_owned(),
        group,
        current,
    }
}

impl InputMethods {
    /// The engine at `engine`, run with `paths`, serving the person's input sources.
    pub fn new(
        engine: Engine,
        paths: Paths,
        compositor: &Compositor,
        preferences: &Preferences,
        language: &str,
    ) -> std::io::Result<Self> {
        let available = engine.available();
        let layout = layout_of(&compositor.scene);
        let (sources, current) = chosen(preferences, &available);
        // The current source first (the one Super+Alt+Space goes back from), then the layout and the
        // input methods in their order.
        let mut recent = vec![current.clone().unwrap_or_else(|| LAYOUT_SOURCE.to_owned())];
        for id in std::iter::once(LAYOUT_SOURCE.to_owned()).chain(sources.iter().cloned()) {
            if !recent.contains(&id) {
                recent.push(id);
            }
        }
        // The engine starts with the person's sources, so its first configuration is theirs.
        let first = engine_sources(&layout, language, &sources, current.as_deref(), false);
        Ok(Self {
            service: Service::spawn(engine.clone(), paths, first)?,
            engine,
            available,
            sources,
            current,
            recent,
            cycle: Vec::new(),
            layout,
            language: language.to_owned(),
            connected: false,
            field: Field::default(),
            symbol: None,
            keys: Default::default(),
            taken: BTreeSet::new(),
            preedit: None,
            composing: Composing::default(),
            emoji: None,
            ready: false,
            switching: 0,
            told: None,
            told_to: BTreeSet::new(),
        })
    }

    /// What wakes the compositor's turn when the engine answered.
    pub fn wake_fd(&self) -> std::io::Result<OwnedFd> {
        self.service.wake_fd().try_clone_to_owned()
    }

    /// Whether the seat keyboard composes dead keys itself: not while an input method is current,
    /// whose engine reads them as keys.
    pub fn composing(&self) -> bool {
        !self.connected
    }

    /// The installed engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The person's input sources as the preferences keep them now (after Settings changed
    /// them). Input methods the engine lacks are left out.
    pub fn follow_preferences(&mut self, preferences: &Preferences) {
        let (sources, current) = chosen(preferences, &self.available);
        self.sources = sources;
        // Every source in the recency order once, the ones removed gone, the ones added last.
        let mut all = vec![LAYOUT_SOURCE.to_owned()];
        all.extend(self.sources.iter().cloned());
        self.recent.retain(|id| all.contains(id));
        for id in all {
            if !self.recent.contains(&id) {
                self.recent.push(id);
            }
        }
        self.choose(current, false);
    }

    /// Makes `current` the input source (`None`: the layout), told to the engine.
    fn choose(&mut self, current: Option<String>, used: bool) {
        let id = current.clone().unwrap_or_else(|| LAYOUT_SOURCE.to_owned());
        if used {
            self.recent.retain(|kept| *kept != id);
            self.recent.insert(0, id);
        }
        self.current = current;
        self.sources_to_engine();
    }

    /// The engine told the group and the input method in use, the switch awaited.
    fn sources_to_engine(&mut self) {
        self.switching += 1;
        self.service.send(Command::Sources(self.engine_sources()));
    }

    fn engine_sources(&self) -> Sources {
        engine_sources(
            &self.layout,
            &self.language,
            &self.sources,
            self.current.as_deref(),
            self.emoji.is_some(),
        )
    }

    fn emoji_engine(&self) -> String {
        emoji_engine(&self.language)
    }

    /// The session's language changed (ADR 0040): emoji are searched in it from now. The input
    /// sources stay as they are.
    pub fn set_language(&mut self, language: &str) {
        if self.language != language {
            self.language = language.to_owned();
            self.sources_to_engine();
        }
    }

    /// Whether the frontend should be connected: an input method current, emoji wanted, or a
    /// switch away from an input method not yet done.
    fn wanted(&self) -> bool {
        self.current.is_some() || self.emoji.is_some() || (self.connected && self.switching > 0)
    }

    fn connect(&mut self, compositor: &mut Compositor) {
        if self.connected {
            return;
        }
        let grants = Grants {
            input_method: true,
            ..Grants::default()
        };
        match compositor.scene.connect(CLIENT, grants) {
            Ok(()) => {
                if std::env::var_os("OXYOS_INPUT_METHODS_TRACE").is_some() {
                    eprintln!("[input methods] connected");
                }
                compositor.local_clients.insert(CLIENT);
                self.connected = true;
                self.keys.clear();
                self.taken.clear();
                self.field = Field::default();
                self.preedit = None;
                self.composing = Composing::default();
            }
            // Another input method holds the role (the development compositor's own).
            Err(code) => eprintln!("[input methods] not connected: {code:?}"),
        }
    }

    fn disconnect(&mut self, compositor: &mut Compositor) {
        if !self.connected {
            return;
        }
        // A composition the field still holds as the engine's is committed as it shows, as a Mac
        // commits marked text when the input source changes. The engine commits it too as it
        // switches, but its commit and its Switched come back in either order: told Switched first,
        // the frontend left before the commit came, and the composition was lost (ime-sources-scaled,
        // steps 3, 8 and 12 on seldon: the source changed, the field never had ni or か). Its commit
        // coming after this finds nothing connected, so it is never doubled.
        if let Some(preedit) = self.preedit.clone().filter(|_| self.field.active) {
            let change = Change {
                delete: None,
                commit: preedit.text(),
                preedit: None,
            };
            self.apply(&mut compositor.scene, &change);
        }
        if self.field.active {
            self.service.send(Command::Deactivate);
        }
        compositor.scene.disconnect(CLIENT);
        compositor.local_clients.remove(&CLIENT);
        compositor
            .local_events
            .retain(|(client, _)| *client != CLIENT);
        self.connected = false;
        self.field = Field::default();
        self.preedit = None;
        self.composing = Composing::default();
    }

    /// Waits until the engine runs, at most `limit` (the development compositor, so a scenario's
    /// first key reaches it); false if it did not start by then.
    pub fn wait_ready(&mut self, compositor: &mut Compositor, limit: std::time::Duration) -> bool {
        let start = std::time::Instant::now();
        while !self.ready && start.elapsed() < limit {
            for outcome in self.service.outcomes() {
                self.outcome(compositor, outcome);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        self.ready
    }

    /// The session ends: the frontend leaves the scene (the engine stops when this is dropped).
    pub fn end(&mut self, compositor: &mut Compositor) {
        self.emoji = None;
        self.current = None;
        self.disconnect(compositor);
    }

    /// A turn: the input-source shortcuts, the shell's and Settings' requests, what the scene
    /// told the input method, and what the engine did.
    pub fn serve(&mut self, compositor: &mut Compositor, settings: &mut Settings, now: i64) {
        for shortcut in compositor.scene.take_input_source_shortcuts() {
            self.shortcut(compositor, settings, now, shortcut);
        }
        for request in compositor
            .scene
            .take_system_requests_where(|request| ours(&request.text))
        {
            self.request(compositor, settings, now, request);
        }
        if self.wanted() {
            self.connect(compositor);
        }
        let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut compositor.local_events)
            .into_iter()
            .partition(|(client, _)| *client == CLIENT);
        compositor.local_events = others;
        let trace = std::env::var_os("OXYOS_INPUT_METHODS_TRACE").is_some();
        for (_, event) in mine {
            if trace {
                eprintln!("[input methods] told {event:?}");
            }
            self.event(event);
        }
        for outcome in self.service.outcomes() {
            if trace {
                eprintln!("[input methods] engine {outcome:?}");
            }
            self.outcome(compositor, outcome);
        }
        if !self.wanted() {
            self.disconnect(compositor);
        }
        compositor.set_composing(self.composing());
        self.tell(&mut compositor.scene);
    }

    fn shortcut(
        &mut self,
        compositor: &mut Compositor,
        settings: &mut Settings,
        now: i64,
        shortcut: InputSourceShortcut,
    ) {
        match shortcut {
            InputSourceShortcut::Previous { step } => {
                if step == 1 {
                    self.cycle = self.recent.clone();
                }
                if self.cycle.len() < 2 {
                    return;
                }
                let id = self.cycle[step as usize % self.cycle.len()].clone();
                self.select(compositor, settings, now, &id);
            }
            InputSourceShortcut::Next => {
                let mut list = vec![LAYOUT_SOURCE.to_owned()];
                list.extend(self.sources.iter().cloned());
                if list.len() < 2 {
                    return;
                }
                let current = self
                    .current
                    .clone()
                    .unwrap_or_else(|| LAYOUT_SOURCE.to_owned());
                let index = list.iter().position(|id| *id == current).unwrap_or(0);
                let id = list[(index + 1) % list.len()].clone();
                self.select(compositor, settings, now, &id);
            }
            InputSourceShortcut::Emoji => self.emoji(),
        }
    }

    /// The person chose an input source: kept as their preference, told to the engine.
    fn select(&mut self, compositor: &mut Compositor, settings: &mut Settings, now: i64, id: &str) {
        let current = (id != LAYOUT_SOURCE).then(|| id.to_owned());
        if current
            .as_ref()
            .is_some_and(|id| !self.sources.contains(id))
        {
            return;
        }
        self.emoji = None;
        if let Err(error) = settings.set_preference(compositor, "input_source", id, now) {
            eprintln!("[input methods] input_source={id}: {error}");
        }
        self.choose(current, true);
    }

    fn emoji(&mut self) {
        if self.emoji.is_none() {
            self.emoji = Some(Emoji::Wanted);
            if self.field.active {
                self.start_emoji();
            }
        }
    }

    fn start_emoji(&mut self) {
        self.emoji = Some(Emoji::Searching { shown: false });
        self.service.send(Command::Emoji {
            keyboard: self.emoji_engine(),
        });
    }

    /// Emoji searched no more: the input source in use again.
    fn end_emoji(&mut self) {
        self.emoji = None;
        self.sources_to_engine();
    }

    fn request(
        &mut self,
        compositor: &mut Compositor,
        settings: &mut Settings,
        now: i64,
        request: SystemRequest,
    ) {
        let SystemRequest {
            client,
            serial,
            with_input,
            text,
        } = request;
        let scene = &mut compositor.scene;
        let refuse = |scene: &mut Scene, reason: &str| {
            scene.system_text(
                client,
                serial,
                &json!({"refused": {"reason": reason}}).to_string(),
            );
        };
        let Ok(ask) = serde_json::from_str::<Ask>(&text) else {
            return refuse(scene, "invalidRequest");
        };
        if !scene
            .grants_of(client)
            .is_some_and(|grants| grants.shell || grants.settings)
        {
            return refuse(scene, "notPermitted");
        }
        // What changes anything cites an input the asker received.
        if !with_input && !matches!(ask, Ask::InputSources {}) {
            return refuse(scene, "needsInput");
        }
        let text = match ask {
            Ask::InputSources {} => self.state_text(),
            Ask::SelectInputSource { id } => {
                let known = id == LAYOUT_SOURCE || self.sources.contains(&id);
                if !known {
                    return refuse(scene, "noSuchInputSource");
                }
                self.select(compositor, settings, now, &id);
                self.state_text()
            }
            Ask::ShowEmoji {} => {
                self.emoji();
                json!({"emoji": {}}).to_string()
            }
        };
        compositor.scene.system_text(client, serial, &text);
    }

    /// The input sources as the shell and Settings are told them.
    fn state_text(&self) -> String {
        let mut sources = vec![LAYOUT_SOURCE.to_owned()];
        sources.extend(self.sources.iter().cloned());
        json!({"inputSources": {
            "layout": self.layout,
            "current": self.current.clone().unwrap_or_else(|| LAYOUT_SOURCE.to_owned()),
            "sources": sources,
            "available": self.available,
        }})
        .to_string()
    }

    /// Tells the shell and Settings the input sources: each one as it connects, all of them as
    /// they change.
    fn tell(&mut self, scene: &mut Scene) {
        let text = self.state_text();
        let watchers = scene.input_source_watchers();
        let changed = self.told.as_deref() != Some(text.as_str());
        for client in &watchers {
            if changed || !self.told_to.contains(client) {
                scene.system_text(*client, 0, &text);
            }
        }
        self.told_to = watchers.into_iter().collect();
        self.told = Some(text);
    }

    /// What the scene told the input method.
    fn event(&mut self, event: Event) {
        match event {
            Event::InputMethodActivate {} => {
                self.field = Field {
                    active: true,
                    starting: true,
                    ..Field::default()
                };
                self.preedit = None;
                self.composing = Composing::default();
            }
            Event::InputMethodDeactivate {} => {
                self.field = Field::default();
                self.preedit = None;
                self.composing = Composing::default();
                self.service.send(Command::Deactivate);
                // Emoji were for that field.
                if matches!(self.emoji, Some(Emoji::Searching { .. })) {
                    self.end_emoji();
                }
            }
            Event::InputMethodPurpose { purpose, hints } => {
                self.field.purpose = Some(purpose);
                self.field.hints = hints;
            }
            Event::InputMethodSurrounding { text } => self.field.receiving.push_str(text.as_str()),
            Event::InputMethodSelection { anchor, cursor } => {
                self.field.anchor = anchor;
                self.field.cursor = cursor;
            }
            Event::InputMethodDone { serial } => {
                self.field.text = std::mem::take(&mut self.field.receiving);
                self.field.serial = serial;
                let told = (
                    self.field.text.clone(),
                    self.field.anchor,
                    self.field.cursor,
                );
                if self.preedit.is_some() && self.composing.told(&told) {
                    // The field took the composition as text: the engine forgets it.
                    self.preedit = None;
                    self.service.send(Command::Reset);
                }
                if std::mem::take(&mut self.field.starting) {
                    self.service.send(Command::Activate {
                        capability: capabilities(self.field.purpose, self.field.hints),
                    });
                    if self.emoji == Some(Emoji::Wanted) {
                        self.start_emoji();
                    }
                }
                let text = &self.field.text;
                self.service.send(Command::Surrounding {
                    text: text.clone(),
                    cursor: chars_before(text, self.field.cursor),
                    anchor: chars_before(text, self.field.anchor),
                });
            }
            Event::InputMethodKeySymbol {
                serial,
                keysym,
                modifiers,
            } => self.symbol = Some((serial, keysym, modifiers)),
            Event::InputMethodKey {
                serial,
                key,
                pressed,
                time_ns,
                ..
            } => {
                self.keys.insert(serial, (key, pressed));
                let (keysym, modifiers) = match self.symbol.take() {
                    Some((told, keysym, modifiers)) if told == serial => (keysym, modifiers),
                    _ => (0, 0),
                };
                self.service.send(Command::Key {
                    serial,
                    keysym,
                    code: key,
                    modifiers,
                    pressed,
                    time_ms: (time_ns / 1_000_000) as u32,
                });
            }
            Event::InputMethodCandidateChosen { index } => {
                self.service.send(Command::Choose(index))
            }
            Event::InputMethodCandidatesPage { forward } => {
                self.service.send(Command::Page { forward })
            }
            _ => {}
        }
    }

    /// What the engine did.
    fn outcome(&mut self, compositor: &mut Compositor, outcome: Outcome) {
        match outcome {
            Outcome::Ready => self.ready = true,
            Outcome::Switched => self.switching = self.switching.saturating_sub(1),
            Outcome::Stopped(why) => {
                eprintln!("[input methods] the engine stopped: {why}; it starts again");
                self.ready = false;
                self.preedit = None;
                self.composing = Composing::default();
            }
            Outcome::Key {
                serial,
                used,
                change,
            } => {
                let used = match self.keys.remove(&serial) {
                    Some((code, true)) => {
                        if used {
                            self.taken.insert(code);
                        } else {
                            self.taken.remove(&code);
                        }
                        used
                    }
                    Some((code, false)) => self.taken.remove(&code) || used,
                    None => used,
                };
                if self.connected {
                    self.apply(&mut compositor.scene, &change);
                    let _ = compositor
                        .scene
                        .handle(CLIENT, Request::InputMethodKeyHandled { serial, used });
                }
            }
            Outcome::Change(change) => {
                if self.connected {
                    self.apply(&mut compositor.scene, &change);
                }
            }
            Outcome::Panel(panel) => {
                if self.connected {
                    self.candidates(&mut compositor.scene, &panel);
                }
                // QuickPhrase closed (a choice made, or Escape): emoji searched no more.
                if let Some(Emoji::Searching { shown }) = self.emoji {
                    if !panel.is_empty() {
                        self.emoji = Some(Emoji::Searching { shown: true });
                    } else if shown {
                        self.end_emoji();
                    }
                }
            }
        }
    }

    /// The engine's change as one transaction for the field it was made for.
    fn apply(&mut self, scene: &mut Scene, change: &Change) {
        if change.is_empty() || !self.field.active {
            return;
        }
        let send = |scene: &mut Scene, request: Request| {
            if let Err(code) = scene.handle(CLIENT, request) {
                eprintln!("[input methods] {request:?} refused: {code:?}");
            }
        };
        let field = &self.field;
        let mut deleted = (0, 0);
        if let Some((offset, size)) = change.delete {
            // fcitx5 deletes `size` characters from `offset` characters past the caret.
            let (before, after) = if offset < 0 {
                let before = offset.unsigned_abs().min(size);
                (before, size - before)
            } else {
                (0, size + offset as u32)
            };
            deleted = (
                bytes_of(&field.text, field.cursor, before, true),
                bytes_of(&field.text, field.cursor, after, false),
            );
            send(
                scene,
                Request::InputMethodDelete {
                    before: deleted.0,
                    after: deleted.1,
                },
            );
        }
        for text in TextChunk::split(&change.commit) {
            send(scene, Request::InputMethodCommitText { text });
        }
        if let Some(preedit) = &change.preedit {
            self.preedit = Some(preedit.clone()).filter(|preedit| !preedit.text().is_empty());
        } else if !change.commit.is_empty() {
            // A commit ends the composition unless the engine says another.
            self.preedit = None;
        }
        let field = &self.field;
        self.composing.sent(
            (field.text.clone(), field.anchor, field.cursor),
            deleted,
            &change.commit,
            self.preedit.is_some(),
        );
        if let Some(preedit) = &self.preedit {
            let text = preedit.text();
            for chunk in TextChunk::split(&text) {
                send(scene, Request::InputMethodPreeditText { text: chunk });
            }
            let cursor = u32::try_from(preedit.cursor)
                .unwrap_or(text.len() as u32)
                .min(text.len() as u32);
            send(
                scene,
                Request::InputMethodPreeditCursor {
                    begin: cursor,
                    end: cursor,
                },
            );
            let mut begin = 0;
            for (part, format) in &preedit.parts {
                let end = begin + part.len() as u32;
                if end > begin {
                    let style = if format.converting() {
                        PreeditStyle::Converting
                    } else {
                        PreeditStyle::Composing
                    };
                    send(
                        scene,
                        Request::InputMethodPreeditSegment { begin, end, style },
                    );
                }
                begin = end;
            }
        }
        send(
            scene,
            Request::InputMethodCommit {
                serial: self.field.serial,
            },
        );
    }

    /// The engine's candidates, handed to the system for the shell to draw.
    fn candidates(&self, scene: &mut Scene, panel: &Panel) {
        let send = |scene: &mut Scene, request: Request| {
            if let Err(code) = scene.handle(CLIENT, request) {
                eprintln!("[input methods] {request:?} refused: {code:?}");
            }
        };
        let candidates: Vec<&String> = panel.candidates.iter().take(MAX_INPUT_CANDIDATES).collect();
        for (index, candidate) in candidates.iter().enumerate() {
            for text in TextChunk::split(bounded(candidate, MAX_CANDIDATE_BYTES)) {
                send(
                    scene,
                    Request::InputMethodCandidate {
                        index: index as u8,
                        text,
                    },
                );
            }
        }
        // With no candidates, a prompt alone (QuickPhrase's) is still shown.
        let aux = if panel.aux_down.is_empty() {
            panel.aux_up.clone()
        } else if panel.aux_up.is_empty() {
            panel.aux_down.clone()
        } else {
            format!("{} {}", panel.aux_up, panel.aux_down)
        };
        for text in TextChunk::split(bounded(&aux, MAX_CANDIDATES_AUX_BYTES)) {
            send(scene, Request::InputMethodCandidatesAux { text });
        }
        let mut flags = 0;
        if panel.previous {
            flags |= CANDIDATES_PREVIOUS;
        }
        if panel.next {
            flags |= CANDIDATES_NEXT;
        }
        if panel.vertical {
            flags |= CANDIDATES_VERTICAL;
        }
        let highlighted = panel
            .highlighted
            .filter(|index| *index < candidates.len())
            .map_or(NO_CANDIDATE, |index| index as u8);
        send(
            scene,
            Request::InputMethodCandidates {
                count: candidates.len() as u8,
                highlighted,
                flags,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_offsets_become_bytes_around_the_caret() {
        let text = "你好abc";
        assert_eq!(chars_before(text, 6), 2);
        assert_eq!(bytes_of(text, 6, 2, true), 6);
        assert_eq!(bytes_of(text, 6, 2, false), 2);
        assert_eq!(bytes_of(text, 6, 9, false), 3);
    }

    #[test]
    fn a_field_says_what_it_is_for() {
        assert_eq!(
            capabilities(Some(TextPurpose::Email), TextHints::default()),
            capability::EMAIL
        );
        assert_eq!(
            capabilities(
                Some(TextPurpose::Normal),
                TextHints::new(TextHints::MULTILINE).unwrap()
            ),
            capability::MULTILINE
        );
    }

    fn told(text: &str, caret: u32) -> Told {
        (text.to_owned(), caret, caret)
    }

    #[test]
    fn a_composition_changing_alone_leaves_the_field_as_it_was() {
        let mut composing = Composing::default();
        // ni, then nih: the field tells "ab" with its caret at 2 throughout.
        composing.sent(told("ab", 2), (0, 0), "", true);
        assert!(!composing.told(&told("ab", 2)));
        composing.sent(told("ab", 2), (0, 0), "", true);
        assert!(!composing.told(&told("ab", 2)));
    }

    #[test]
    fn a_press_elsewhere_in_the_field_takes_the_composition_as_text() {
        let mut composing = Composing::default();
        composing.sent(told("ab", 2), (0, 0), "", true);
        // The field ended it at a press at its start: "abあい" with the caret at 0.
        assert!(composing.told(&told("abあい", 0)));
        // Forgotten: nothing more is expected of the field.
        assert!(!composing.told(&told("abあい", 3)));
    }

    #[test]
    fn a_caret_moved_by_the_application_takes_it_too() {
        let mut composing = Composing::default();
        composing.sent(told("ab", 2), (0, 0), "", true);
        assert!(composing.told(&told("ab", 1)));
    }

    #[test]
    fn a_commit_with_the_next_composition_is_the_engines() {
        let mut composing = Composing::default();
        composing.sent(told("x", 1), (0, 0), "", true);
        // 你好 committed and ma composed after it: the field tells "x你好", its caret after it.
        composing.sent(told("x", 1), (0, 0), "你好", true);
        // Told before the change reached it, then after.
        assert!(!composing.told(&told("x", 1)));
        assert!(!composing.told(&told("x你好", 7)));
        // Then only the composition changes.
        composing.sent(told("x你好", 7), (0, 0), "", true);
        assert!(!composing.told(&told("x你好", 7)));
    }

    #[test]
    fn a_composition_replaces_the_selection_and_a_deletion_goes_around_it() {
        let mut composing = Composing::default();
        // "abcd" with bc selected: a composition replaces them, the caret where they were.
        composing.sent(("abcd".to_owned(), 1, 3), (0, 0), "", true);
        assert!(!composing.told(&told("ad", 1)));
        // Hangul: a byte before deleted and 가 committed, composing on.
        composing.sent(told("ad", 1), (1, 0), "가", true);
        assert!(!composing.told(&told("가d", 3)));
    }

    #[test]
    fn nothing_is_expected_once_the_engine_composes_nothing() {
        let mut composing = Composing::default();
        composing.sent(told("ab", 2), (0, 0), "", true);
        composing.sent(told("ab", 2), (0, 0), "日本", false);
        assert!(!composing.told(&told("ab日本", 8)));
        assert!(!composing.told(&told("", 0)));
    }

    #[test]
    fn only_its_own_requests_are_its() {
        assert!(ours(r#"{"selectInputSource":{"id":"pinyin"}}"#));
        assert!(ours(r#"{"showEmoji":{}}"#));
        assert!(ours(r#"{"inputSources":{}}"#));
        assert!(!ours(r#"{"wifi":{"enabled":true}}"#));
    }
}
