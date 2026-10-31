//! File choosers for the session (docs/adr/0029-chromium-on-oxyos.mdx, decision 8): the program
//! running the compositor's side of the open and save panels.
//!
//! The scene hands on each request an application made for one of its windows, citing its input.
//! This reads it in the session's vocabulary ([`Ask`], strictly: an unknown key or a value out of
//! bounds refuses it), and asks for the chooser to be started: the file manager in its picker mode,
//! told what to show (`OXYOS_FILE_CHOOSER`, [`Shown`]). The program starts it and says which
//! process it is ([`Choosers::started`]); the compositor attaches its connection to the request as
//! it says Hello, so its window is the asking window's sheet. Its answer is checked here: a
//! cancellation, or files the person chose citing an input of the chooser's, as many as the request
//! allows, of the kind it asked for, each a `file://` URI the rule applications' files follow
//! reaches ([`oxyos_files::Scope`]: the home and the volumes mounted for the person, never a
//! Trash's insides), an opened one there and a saved one's folder there. Anything else is not an
//! answer, and the panel stays. The asker alone is told, once; then the chooser's process is ended.

use oxyos_compositor_core::{ClientId, Scene};
use oxyos_files::{uri, Access, Scope};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// The most accepted types a request may list, and the longest each may be.
pub const MAX_TYPES: usize = 128;
pub const MAX_TYPE_BYTES: usize = 128;
/// The longest title a panel shows, and the longest name a save suggests (a file's name).
pub const MAX_TITLE_BYTES: usize = 256;
pub const MAX_NAME_BYTES: usize = 255;
/// The most files one answer chooses.
pub const MAX_CHOSEN: usize = 1024;

/// What an application asks for, in the session's vocabulary.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Ask {
    /// Files (or folders, `directory`) to open: one, or several with `multiple`.
    Open {
        #[serde(default)]
        multiple: bool,
        #[serde(default)]
        directory: bool,
        #[serde(default)]
        types: Vec<String>,
        title: Option<String>,
        folder: Option<String>,
    },
    /// The file to save as: `name` suggested.
    Save {
        name: Option<String>,
        #[serde(default)]
        types: Vec<String>,
        title: Option<String>,
        folder: Option<String>,
    },
}

/// The application asking, as the program running the compositor knows it: its id and the name
/// the person knows it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asker {
    pub id: String,
    pub name: String,
}

/// A chooser to start for request `id`, told `shown` (the JSON of `OXYOS_FILE_CHOOSER`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Start {
    pub id: u32,
    pub shown: String,
}

/// What a chooser is told to show (`OXYOS_FILE_CHOOSER`): Bloom's `PlatformFileChooserRequest`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub save: bool,
    pub multiple: bool,
    pub directory: bool,
    pub types: Vec<String>,
    pub title: Option<String>,
    pub suggested_name: Option<String>,
    pub folder: Option<String>,
    pub application: Option<Asker>,
}

impl Shown {
    pub fn to_json(&self) -> String {
        json!({
            "mode": if self.save { "save" } else { "open" },
            "multiple": self.multiple,
            "directory": self.directory,
            "types": self.types,
            "title": self.title,
            "suggestedName": self.suggested_name,
            "folder": self.folder,
            "application": self.application.as_ref().map(|asker| json!({"id": asker.id, "name": asker.name})),
        })
        .to_string()
    }
}

/// A chooser's answer, in the session's vocabulary.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Answer {
    Chosen(Vec<String>),
    Cancelled {},
}

/// Text a person reads (a title, a name): bounded, with no control character.
fn readable(text: &str, limit: usize) -> bool {
    text.len() <= limit && !text.chars().any(char::is_control)
}

/// An accepted type: a media type (`image/png`, `image/*`, lowercase) or an extension (`.png`,
/// `.tar.gz`).
fn accepted_type(kind: &str) -> bool {
    let token = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"!#$&^_.+-".contains(&b))
    };
    if kind.is_empty() || kind.len() > MAX_TYPE_BYTES {
        return false;
    }
    if let Some(extension) = kind.strip_prefix('.') {
        return extension.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_+-".contains(&b))
        });
    }
    match kind.split_once('/') {
        Some((family, subtype)) => token(family) && (subtype == "*" || token(subtype)),
        None => false,
    }
}

/// A name a saved file may take: one item's name, readable.
fn file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && readable(name, MAX_NAME_BYTES)
}

impl Ask {
    /// The request read strictly, then checked: what the chooser is shown, or why it is refused.
    /// A folder to start in that the person's files do not hold is left out, not refused: it is
    /// the asker's suggestion (a browser's last folder may be gone).
    pub fn read(
        text: &str,
        application: Option<Asker>,
        scope: Option<&Scope>,
    ) -> Result<Shown, &'static str> {
        let ask: Ask = serde_json::from_str(text).map_err(|_| "invalidRequest")?;
        let (save, multiple, directory, types, title, name, folder) = match ask {
            Ask::Open {
                multiple,
                directory,
                types,
                title,
                folder,
            } => (false, multiple, directory, types, title, None, folder),
            Ask::Save {
                name,
                types,
                title,
                folder,
            } => (true, false, false, types, title, name, folder),
        };
        if types.len() > MAX_TYPES || !types.iter().all(|kind| accepted_type(kind)) {
            return Err("invalidRequest");
        }
        if title
            .as_deref()
            .is_some_and(|title| !readable(title, MAX_TITLE_BYTES))
        {
            return Err("invalidRequest");
        }
        if name.as_deref().is_some_and(|name| !file_name(name)) {
            return Err("invalidRequest");
        }
        let folder = folder.and_then(|folder| {
            let path = uri::path_of(&folder).ok()?;
            let reached = scope?.reach(&path, Access::Read).ok()?;
            reached.is_dir().then(|| uri::of(Path::new(&path), true))
        });
        Ok(Shown {
            save,
            multiple,
            directory,
            types,
            title,
            suggested_name: name,
            folder,
            application,
        })
    }
}

/// The chooser's answer to what it was `shown`, checked: the asker's answer, or why it is none.
fn check(
    shown: &Shown,
    text: &str,
    with_input: bool,
    scope: Option<&Scope>,
) -> Result<String, String> {
    let answer: Answer =
        serde_json::from_str(text).map_err(|_| "not in the vocabulary".to_string())?;
    let uris = match answer {
        Answer::Cancelled {} => return Ok(json!({"cancelled": {}}).to_string()),
        Answer::Chosen(uris) => uris,
    };
    if !with_input {
        return Err("a choice citing no input of the chooser's".into());
    }
    let most = if shown.multiple { MAX_CHOSEN } else { 1 };
    if uris.is_empty() || uris.len() > most {
        return Err(format!(
            "{} files for a request of at most {most}",
            uris.len()
        ));
    }
    let scope = scope.ok_or("no home to choose in")?;
    let mut chosen: Vec<Value> = Vec::with_capacity(uris.len());
    for given in &uris {
        let path = uri::path_of(given).map_err(|error| error.to_string())?;
        let item = if shown.save {
            // The file to write: in a folder of the person's, a file if anything is there.
            let reached = scope
                .reach(&path, Access::Write)
                .map_err(|error| error.to_string())?;
            let folder = reached.parent().ok_or("no folder")?;
            if !folder.is_dir() || reached.is_dir() {
                return Err(format!("{given} is not a file in a folder"));
            }
            uri::of(Path::new(&path), false)
        } else {
            let reached = scope
                .reach(&path, Access::Read)
                .map_err(|error| error.to_string())?;
            let metadata = std::fs::metadata(&reached).map_err(|error| error.to_string())?;
            if metadata.is_dir() != shown.directory {
                return Err(format!(
                    "{given} is not a {}",
                    if shown.directory { "folder" } else { "file" }
                ));
            }
            uri::of(Path::new(&path), shown.directory)
        };
        if !chosen.iter().any(|seen| seen == &item) {
            chosen.push(Value::String(item));
        }
    }
    Ok(json!({ "chosen": chosen }).to_string())
}

/// The session's open requests and their choosers.
pub struct Choosers {
    /// What the person's files are, to check answers by; none without a home (no answer chooses).
    scope: Option<Scope>,
    /// Each open request's chooser: what it was shown, and its process once started.
    open: BTreeMap<u32, (Shown, Option<u32>)>,
    /// Choosers whose requests ended: their processes, for the program to end.
    ended: Vec<u32>,
}

impl Choosers {
    /// The session's side of file choosers, checking answers against `scope` (the person's files,
    /// `Scope::for_user`).
    pub fn new(scope: Option<Scope>) -> Self {
        Self {
            scope,
            open: BTreeMap::new(),
            ended: Vec::new(),
        }
    }

    /// A turn: the requests the scene handed on, read (each refused or a chooser to start, in
    /// order), and the choosers' answers, checked and given to their askers. `asker` says who a
    /// client is. Returns the choosers to start; the program starts each and answers with
    /// [`Choosers::started`].
    pub fn serve(
        &mut self,
        scene: &mut Scene,
        asker: &dyn Fn(ClientId) -> Option<Asker>,
    ) -> Vec<Start> {
        let mut starts = Vec::new();
        for request in scene.take_file_requests() {
            match Ask::read(&request.text, asker(request.client), self.scope.as_ref()) {
                Ok(shown) => {
                    starts.push(Start {
                        id: request.id,
                        shown: shown.to_json(),
                    });
                    self.open.insert(request.id, (shown, None));
                }
                Err(reason) => scene.answer_files(
                    request.id,
                    &json!({"refused": {"reason": reason}}).to_string(),
                ),
            }
        }
        for answer in scene.take_chooser_answers() {
            let Some((shown, _)) = self.open.get(&answer.id) else {
                continue;
            };
            match check(shown, &answer.text, answer.with_input, self.scope.as_ref()) {
                Ok(text) => scene.answer_files(answer.id, &text),
                Err(why) => eprintln!("[choosers] request {}: not an answer: {why}", answer.id),
            }
        }
        self.follow(scene);
        starts
    }

    /// The chooser of request `id` was started as `pid`, or could not be (`None`: the request is
    /// refused, the file manager unavailable).
    pub fn started(&mut self, scene: &mut Scene, id: u32, pid: Option<u32>) {
        match (pid, self.open.get_mut(&id)) {
            (Some(pid), Some((_, process))) => *process = Some(pid),
            (Some(pid), None) => self.ended.push(pid),
            (None, _) => {
                self.open.remove(&id);
                scene.answer_files(
                    id,
                    &json!({"refused": {"reason": "unavailable"}}).to_string(),
                );
            }
        }
        self.follow(scene);
    }

    /// A process the program started exited: if it was a chooser whose request is open (it went
    /// before answering, or before connecting), the request is cancelled.
    pub fn exited(&mut self, scene: &mut Scene, pid: u32) {
        let gone = self
            .open
            .iter()
            .find(|(_, (_, process))| *process == Some(pid))
            .map(|(id, _)| *id);
        if let Some(id) = gone {
            self.open.remove(&id);
            scene.answer_files(id, &json!({"cancelled": {}}).to_string());
        }
        self.follow(scene);
        self.ended.retain(|ended| *ended != pid);
    }

    /// The choosers' processes whose requests ended since the last call: the program ends each
    /// (and forgets it as a chooser it expects).
    pub fn take_ended(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.ended)
    }

    /// The requests the scene ended (answered, withdrawn, their window, asker or chooser gone):
    /// their choosers' processes are to be ended.
    fn follow(&mut self, scene: &mut Scene) {
        for id in scene.take_ended_choosers() {
            if let Some((_, process)) = self.open.remove(&id) {
                self.ended.extend(process);
            }
        }
    }
}

#[cfg(test)]
mod tests;
