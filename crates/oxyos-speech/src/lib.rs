//! Speech for the screen reader (docs/adr/0014-screen-reader.mdx): what it says of an element, in
//! the session's language ([`describe`]); the order it says things in ([`Speaker`]: a polite
//! utterance after what it is saying, an assertive one at once); and the voice, behind
//! [`Backend`]: eSpeak NG ([`Espeak`], Debian's speech synthesizer, which plays through the
//! session's PipeWire), or a record of what would have been said ([`Record`], [`Memory`]).

use oxyos_protocol::{AccessibleRole, AccessibleState, Politeness};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

#[cfg(test)]
mod tests;

/// The languages the reader speaks: Spanish when the session's locale is, English otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    Spanish,
}

impl Language {
    /// The session's: `LC_ALL`, else `LC_MESSAGES`, else `LANG` (POSIX's order).
    pub fn from_env() -> Self {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
            .map_or(Self::English, |value| Self::of_locale(&value))
    }

    /// A POSIX locale (`es_ES.UTF-8`) or a BCP 47 tag (`es-MX`).
    pub fn of_locale(locale: &str) -> Self {
        let language = locale.split(['_', '-', '.', '@']).next().unwrap_or("");
        if language.eq_ignore_ascii_case("es") {
            Self::Spanish
        } else {
            Self::English
        }
    }

    /// eSpeak NG's voice for it.
    pub fn voice(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Spanish => "es",
        }
    }

    fn pick(self, english: &'static str, spanish: &'static str) -> &'static str {
        match self {
            Self::English => english,
            Self::Spanish => spanish,
        }
    }
}

/// What the reader calls a role; nothing for the ones it does not name (plain text, a group, a
/// list's item), as VoiceOver reads them by their content.
fn role(language: Language, role: AccessibleRole) -> Option<&'static str> {
    use AccessibleRole as R;
    let (english, spanish) = match role {
        R::Generic | R::Text | R::Group | R::ListItem => return None,
        R::Button => ("button", "botón"),
        R::Link => ("link", "enlace"),
        R::Heading => ("heading", "encabezado"),
        R::Image => ("image", "imagen"),
        R::TextInput => ("text field", "campo de texto"),
        R::Menu => ("menu", "menú"),
        R::MenuItem => ("menu item", "elemento de menú"),
        R::CheckBox => ("checkbox", "casilla"),
        R::Radio => ("radio button", "botón de opción"),
        R::Switch => ("switch", "interruptor"),
        R::Tab => ("tab", "pestaña"),
        R::TabList => ("tab list", "lista de pestañas"),
        R::List => ("list", "lista"),
        R::Dialog => ("dialog", "diálogo"),
        R::Alert => ("alert", "alerta"),
        R::ProgressBar => ("progress indicator", "indicador de progreso"),
        R::Slider => ("slider", "regulador"),
        R::Search => ("search field", "campo de búsqueda"),
        R::Toolbar => ("toolbar", "barra de herramientas"),
        R::ScrollView => ("scroll area", "área de desplazamiento"),
        R::ComboBox => ("combo box", "menú desplegable"),
        R::Summary => ("summary", "resumen"),
        R::Grid => ("grid", "cuadrícula"),
    };
    Some(language.pick(english, spanish))
}

/// What the reader says of a state, in VoiceOver's order: checked or not (for what can be), then
/// selected, expanded or collapsed, busy, dimmed.
fn states(language: Language, role: AccessibleRole, state: AccessibleState) -> Vec<&'static str> {
    let has = |bit| state.contains(bit);
    let l = |english, spanish| language.pick(english, spanish);
    let mut said = Vec::new();
    match role {
        AccessibleRole::CheckBox => said.push(if has(AccessibleState::MIXED) {
            l("mixed", "mixta")
        } else if has(AccessibleState::CHECKED) {
            l("checked", "marcada")
        } else {
            l("unchecked", "no marcada")
        }),
        AccessibleRole::Switch => said.push(if has(AccessibleState::CHECKED) {
            l("on", "activado")
        } else {
            l("off", "desactivado")
        }),
        AccessibleRole::Radio => said.push(if has(AccessibleState::CHECKED) {
            l("selected", "seleccionado")
        } else {
            l("not selected", "no seleccionado")
        }),
        _ => {}
    }
    if has(AccessibleState::SELECTED) && role != AccessibleRole::Radio {
        said.push(l("selected", "seleccionado"));
    }
    if has(AccessibleState::EXPANDED) {
        said.push(l("expanded", "expandido"));
    } else if has(AccessibleState::COLLAPSED) {
        said.push(l("collapsed", "contraído"));
    }
    if has(AccessibleState::BUSY) {
        said.push(l("busy", "ocupado"));
    }
    if has(AccessibleState::DISABLED) {
        said.push(l("dimmed", "atenuado"));
    }
    said
}

/// What the reader says of an element, as VoiceOver does: its name, its value, its state, its
/// role ("Recordar, no marcada, casilla"). Empty for an element with nothing to say.
pub fn describe(
    language: Language,
    role_of: AccessibleRole,
    state: AccessibleState,
    name: &str,
    value: &str,
) -> String {
    let (name, value) = (name.trim(), value.trim());
    let mut parts: Vec<&str> = Vec::new();
    if !name.is_empty() {
        parts.push(name);
    }
    if !value.is_empty() && value != name {
        parts.push(value);
    }
    if parts.is_empty() && matches!(role_of, AccessibleRole::Generic | AccessibleRole::Group) {
        return String::new();
    }
    parts.extend(states(language, role_of, state));
    parts.extend(role(language, role_of));
    parts.join(", ")
}

/// What changed in the focused text field, as the reader echoes it (TalkBack's text events,
/// VoiceOver's typing echo).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextChange {
    /// Typed or pasted at the caret.
    Inserted(String),
    /// Deleted at the caret, or the selection deleted.
    Deleted(String),
    /// Passed over by the caret: a character or a word.
    Traversed(String),
    /// The line the caret moved to.
    Line(String),
    /// Newly selected.
    Selected(String),
    /// No longer selected.
    Unselected(String),
}

/// A run of text as said: a character that sounds like nothing by its name ("espacio").
fn spoken(language: Language, text: &str) -> String {
    let mut chars = text.chars();
    let name = match (chars.next(), chars.next()) {
        (Some(' '), None) => Some(language.pick("space", "espacio")),
        (Some('\n'), None) => Some(language.pick("new line", "nueva línea")),
        (Some('\t'), None) => Some(language.pick("tab", "tabulador")),
        _ => None,
    };
    name.map_or_else(|| text.to_string(), str::to_string)
}

/// What the reader says of a change in the focused text field: what was typed, as it is; what
/// was deleted, selected or unselected, followed by what happened to it ("a, borrado"); what the
/// caret passed over; the line it moved to ("en blanco" for an empty one). Empty for nothing.
pub fn describe_change(language: Language, change: &TextChange) -> String {
    let l = |english, spanish| language.pick(english, spanish);
    let (text, said) = match change {
        TextChange::Inserted(text) | TextChange::Traversed(text) => (text, None),
        TextChange::Line(line) if line.trim().is_empty() => {
            return l("blank", "en blanco").to_string();
        }
        TextChange::Line(line) => (line, None),
        TextChange::Deleted(text) => (text, Some(l("deleted", "borrado"))),
        TextChange::Selected(text) => (text, Some(l("selected", "seleccionado"))),
        TextChange::Unselected(text) => (text, Some(l("unselected", "deseleccionado"))),
    };
    if text.is_empty() {
        return String::new();
    }
    let text = spoken(language, text);
    match said {
        Some(said) => format!("{text}, {said}"),
        None => text,
    }
}

/// A voice: says one utterance at a time.
pub trait Backend {
    /// Starts saying `text`; what it was saying has been stopped if `interrupting`.
    fn say(&mut self, text: &str, interrupting: bool) -> io::Result<()>;
    /// Stops what it is saying, at once.
    fn stop(&mut self);
    /// Whether it is still saying something.
    fn speaking(&mut self) -> bool;
}

impl<B: Backend + ?Sized> Backend for Box<B> {
    fn say(&mut self, text: &str, interrupting: bool) -> io::Result<()> {
        (**self).say(text, interrupting)
    }
    fn stop(&mut self) {
        (**self).stop()
    }
    fn speaking(&mut self) -> bool {
        (**self).speaking()
    }
}

/// The order of what the reader says: a polite utterance waits for what is being said and what
/// waits before it; an assertive one (an application's assertive announcement, the element the
/// user moved to) stops it and drops what waits.
pub struct Speaker<B: Backend> {
    backend: B,
    queue: VecDeque<String>,
}

impl<B: Backend> Speaker<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            queue: VecDeque::new(),
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn say(&mut self, text: &str, politeness: Politeness) -> io::Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        match politeness {
            Politeness::Polite => {
                self.queue.push_back(text.to_string());
                self.pump()
            }
            Politeness::Assertive => {
                self.silence();
                self.backend.say(text, true)
            }
        }
    }

    /// Stops what it is saying and drops what waits.
    pub fn silence(&mut self) {
        self.queue.clear();
        self.backend.stop();
    }

    /// Starts what waits, once the voice has finished what it was saying.
    pub fn pump(&mut self) -> io::Result<()> {
        while !self.backend.speaking() {
            let Some(next) = self.queue.pop_front() else {
                break;
            };
            self.backend.say(&next, false)?;
        }
        Ok(())
    }

    /// Whether something is being said or waits.
    pub fn busy(&mut self) -> bool {
        !self.queue.is_empty() || self.backend.speaking()
    }
}

/// eSpeak NG, run for each utterance (`espeak-ng -v VOICE`, the text on its standard input);
/// stopping kills it. It plays through libpcaudio, which reaches the session's PipeWire through
/// its PulseAudio server (`pipewire-pulse`).
pub struct Espeak {
    program: OsString,
    voice: &'static str,
    child: Option<Child>,
}

impl Espeak {
    pub fn new(program: impl Into<OsString>, language: Language) -> Self {
        Self {
            program: program.into(),
            voice: language.voice(),
            child: None,
        }
    }
}

impl Backend for Espeak {
    fn say(&mut self, text: &str, _interrupting: bool) -> io::Result<()> {
        self.stop();
        // `-b 1`: the text is UTF-8.
        let mut child = Command::new(&self.program)
            .args(["-v", self.voice, "-b", "1"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let written = child
            .stdin
            .take()
            .map_or(Ok(()), |mut stdin| stdin.write_all(text.as_bytes()));
        self.child = Some(child);
        written
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn speaking(&mut self) -> bool {
        match self.child.as_mut().map(Child::try_wait) {
            Some(Ok(None)) => true,
            Some(_) => {
                self.child = None;
                false
            }
            None => false,
        }
    }
}

impl Drop for Espeak {
    fn drop(&mut self) {
        self.stop();
    }
}

/// What would have been said, as JSON lines (`{"t":NS,"text":TEXT,"interrupting":BOOL}`, `t` on
/// CLOCK_MONOTONIC in nanoseconds), for the development compositor's scenarios. Each utterance
/// is said at once.
pub struct Record {
    file: std::fs::File,
}

impl Record {
    pub fn create(path: &std::path::Path) -> io::Result<Self> {
        Ok(Self {
            file: std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?,
        })
    }
}

fn monotonic_ns() -> i64 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    now.tv_sec * 1_000_000_000 + now.tv_nsec
}

/// `text` as a JSON string.
fn json(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Backend for Record {
    fn say(&mut self, text: &str, interrupting: bool) -> io::Result<()> {
        writeln!(
            self.file,
            "{{\"t\":{},\"text\":{},\"interrupting\":{interrupting}}}",
            monotonic_ns(),
            json(text)
        )
    }

    fn stop(&mut self) {}

    fn speaking(&mut self) -> bool {
        false
    }
}

/// What was said, kept in memory; it is saying something while `busy`.
#[derive(Debug, Default)]
pub struct Memory {
    /// Each utterance and whether it interrupted.
    pub said: Vec<(String, bool)>,
    /// How many times it was stopped.
    pub stopped: usize,
    pub busy: bool,
}

impl Backend for Memory {
    fn say(&mut self, text: &str, interrupting: bool) -> io::Result<()> {
        self.said.push((text.to_string(), interrupting));
        self.busy = true;
        Ok(())
    }

    fn stop(&mut self) {
        self.stopped += 1;
        self.busy = false;
    }

    fn speaking(&mut self) -> bool {
        self.busy
    }
}

/// The voice `OXYOS_SPEECH` names: eSpeak NG when unset or `espeak-ng` (or `espeak-ng:PROGRAM`),
/// a record with `record:PATH`.
pub fn from_env(language: Language) -> io::Result<Box<dyn Backend>> {
    let setting = std::env::var("OXYOS_SPEECH").unwrap_or_default();
    if setting.is_empty() || setting == "espeak-ng" {
        return Ok(Box::new(Espeak::new("espeak-ng", language)));
    }
    if let Some(program) = setting.strip_prefix("espeak-ng:") {
        return Ok(Box::new(Espeak::new(PathBuf::from(program), language)));
    }
    if let Some(path) = setting.strip_prefix("record:") {
        return Ok(Box::new(Record::create(std::path::Path::new(path))?));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("OXYOS_SPEECH={setting:?}: expected espeak-ng, espeak-ng:PROGRAM or record:PATH"),
    ))
}
