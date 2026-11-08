//! The search service (docs/adr/0051-search.mdx, decisions 1, 6 and 7): the process the program
//! running the compositor starts as its child, `<program> --search-service`, its standard input
//! one end of a socket pair. The compositor knows every client and brokers their questions; the
//! service trusts what it is told on that socket alone.
//!
//! **The control socket** carries one compact JSON object a line, both ways:
//!
//! ```text
//! → {"languages": ["es", "en"]}
//! → {"applications": [{"id", "name", "keywords": [..], "icon": PATH|null, "package": DIR|null}]}
//! → {"ask": {"client", "serial", "asker": {"id": ID|null, "launched", "shell", "settings"}, "text"}}
//! → {"gone": {"client"}}
//! → {"forget": {"application"}}       (it was reset, docs/adr/0061: what it donated goes)
//! ← {"answer": {"client", "serial", "text"}}          serial 0: told unasked ({"changed": {}})
//! ```
//!
//! **What a client asks** (`text`, one JSON object naming its ask), answered with one object:
//!
//! ```text
//! {"query": {"text", "tokens"?, "within"?: [URI], "categories"?, "limit"?, "language"?}}
//!                                   ← {"results": {"items": [Item], "topHit": ID|null, "indexing"}}
//! {"watch": {}}                     ← {"watching": {}}, then {"changed": {}} unasked after each change
//! {"unwatch": {}}                   ← {"unwatched": {}}
//! {"used": {"id"}}                  ← {"noted": {}}
//! {"donate": {"items": [{"id", "title", "category", "text"?, "keywords"?, "url", "modified"?}]}}
//! {"remove": {"ids": [ID]}}  {"removeAll": {}}          ← {"done": {}}, once the index holds it
//! {"preferences": {}}  {"setPreferences": {"categories"?, "excluded"?: [URI]}}
//!                                   ← {"preferences": {"categories": {NAME: bool}, "excluded": [URI]}}
//! {"status": {}}                    ← {"status": {"indexing", "items", "partial", "bytes", "readsPdf",
//!                                                "read", "rebuilding", "found"}}
//!                                   (`read`: files read for their text since it started; `found`:
//!                                    the items a crawl under way went through, null with none)
//! {"followStatus": {}}              ← {"status": {…}}, then {"status": {…}} unasked as it changes
//! {"unfollowStatus": {}}            ← {"unfollowed": {}}
//! {"rebuild": {}}                   ← {"rebuilding": {}}: the index discarded and built again from
//!                                     the files, the old one answering until the new one is ready
//! {"storage": {}}                   ← {"storage": {"home": URI, "kinds": {NAME: {"files", "bytes"}},
//!                                                 "indexing", "partial"}}
//! {"largest": {"limit"?, "language"?}}  ← {"results": {"items": [Item], "topHit": null, "indexing"}}
//! anything refused                  ← {"refused": {"reason": "notPermitted" | "invalid"}}
//! ```
//!
//! **Who sees what** (decision 6): the shell sees every item but the categories the person turned
//! off for Spotlight (a question naming its categories, the launcher's, is not Spotlight's); any
//! other client sees the files, and an application the session launched its own items
//! too. Donating and removing are a launched application's, its own items alone; the preferences
//! and rebuilding the index are Settings' (the `settings` grant), and so are what the home's files
//! occupy by kind and its largest files (`storage`, `largest`: Settings › Storage,
//! docs/adr/0062-storage.mdx), read from the index's columns, never by walking the disk.
//!
//! **Threads:** this one answers, at the normal priority, so a question is answered at once while
//! the index is being built; the writer ([`crate::writer`]), at nice 10, changes the index, so
//! what the session and the applications give it is written at once; the crawler
//! ([`crate::indexer`]) reads the files at the scheduler's idle class and the disk's, with the
//! processes it starts (a PDF's reader). None wakes on a clock while nothing happens.

use crate::category::Category;
use crate::extract;
use crate::index::{Index, Question, Record, Searcher, Source, Usage, Viewer};
use crate::indexer::{Command, Indexer};
use crate::query::{self, Token};
use crate::writer::{self, Status, Wake};
use oxyos_files::{uri, Scope};
use rustix::event::{poll, PollFd, PollFlags};
use rustix::fd::{AsFd, OwnedFd};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// The most items a question gives.
const MAX_LIMIT: usize = 200;
/// The largest files Settings › Storage's Review Files lists unless it says.
const DEFAULT_LARGEST: usize = 50;
/// The items a question gives unless it says.
const DEFAULT_LIMIT: usize = 60;
/// Of a question's items, how many are given a snippet of their text.
const SNIPPETS: usize = 10;
/// The most items one donation holds.
const MAX_DONATED: usize = 1000;
/// The longest text of a donated item, in bytes.
const MAX_DONATED_TEXT: usize = 64 * 1024;
/// The most items whose use is remembered.
const MAX_USAGE: usize = 10_000;
/// The largest `searchable.json` read.
const MAX_DECLARED: u64 = 1 << 20;
/// The system's Settings, whose package alone declares Settings' pages (ADR 0030, decision 7:
/// no other source can shadow a system application's id).
pub const SETTINGS_ID: &str = "so.oxy.settings";

/// Where the service keeps what it keeps.
#[derive(Clone, Debug)]
pub struct Paths {
    /// The index (a cache: made again when lost), `$XDG_CACHE_HOME/oxyos/search`.
    pub cache: PathBuf,
    /// What the person opened (`usage.json`), `$XDG_DATA_HOME/oxyos/search`.
    pub data: PathBuf,
    /// The person's choices in Settings › Spotlight, `$XDG_CONFIG_HOME/oxyos/search.json`.
    pub config: PathBuf,
}

impl Paths {
    /// The user's, from the environment.
    pub fn for_user(scope: &Scope) -> Paths {
        let var = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        let home = scope.home();
        Paths {
            cache: var("XDG_CACHE_HOME")
                .unwrap_or_else(|| home.join(".cache"))
                .join("oxyos/search"),
            data: var("XDG_DATA_HOME")
                .unwrap_or_else(|| home.join(".local/share"))
                .join("oxyos/search"),
            config: scope.config_home().join("oxyos/search.json"),
        }
    }
}

/// The person's choices in Settings › Spotlight.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preferences {
    /// Categories the shell's results leave out.
    pub hidden: Vec<Category>,
    /// Folders kept out of the index.
    pub excluded: Vec<PathBuf>,
}

impl Preferences {
    /// Reads them; anything missing or unreadable is the default (every category, no folder
    /// kept out), so the preferences never stop the service.
    pub fn load(path: &Path) -> Preferences {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Preferences::default();
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return Preferences::default();
        };
        let hidden = value["hidden"]
            .as_array()
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| Category::parse(name.as_str()?))
                    .collect()
            })
            .unwrap_or_default();
        let excluded = value["excluded"]
            .as_array()
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(|path| Some(PathBuf::from(path.as_str()?)))
                    .filter(|path| path.is_absolute())
                    .collect()
            })
            .unwrap_or_default();
        Preferences { hidden, excluded }
    }

    fn save(&self, path: &Path) -> std::io::Result<()> {
        let value = json!({
            "hidden": self.hidden.iter().map(|category| category.name()).collect::<Vec<_>>(),
            "excluded": self.excluded,
        });
        write_atomically(path, &serde_json::to_vec_pretty(&value).expect("JSON"))
    }

    fn json(&self) -> Value {
        let categories: serde_json::Map<String, Value> = Category::ALL
            .iter()
            .map(|category| {
                (
                    category.name().to_string(),
                    Value::Bool(!self.hidden.contains(category)),
                )
            })
            .collect();
        json!({
            "categories": categories,
            "excluded": self.excluded.iter().map(|path| uri::of(path, true)).collect::<Vec<_>>(),
        })
    }
}

/// Writes `bytes` to `path` whole or not at all.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let staged = path.with_extension("staged");
    std::fs::write(&staged, bytes)?;
    std::fs::rename(&staged, path)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}

/// Who asks, as the compositor says.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Asker {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub launched: bool,
    #[serde(default)]
    pub shell: bool,
    #[serde(default)]
    pub settings: bool,
}

impl Asker {
    /// The launched application's id: whose own items it sees and changes.
    fn application(&self) -> Option<&str> {
        self.id.as_deref().filter(|_| self.launched)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Asked {
    text: String,
    #[serde(default)]
    tokens: Vec<Token>,
    #[serde(default)]
    within: Vec<String>,
    #[serde(default)]
    categories: Vec<Category>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    language: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Donated {
    id: String,
    title: String,
    category: Category,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
    url: String,
    #[serde(default)]
    modified: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Ask {
    Query(Asked),
    Watch {},
    Unwatch {},
    Used {
        id: String,
    },
    Donate {
        items: Vec<Donated>,
    },
    Remove {
        ids: Vec<String>,
    },
    RemoveAll {},
    Preferences {},
    Rebuild {},
    FollowStatus {},
    UnfollowStatus {},
    SetPreferences {
        #[serde(default)]
        categories: Option<BTreeMap<Category, bool>>,
        #[serde(default)]
        excluded: Option<Vec<String>>,
    },
    Status {},
    /// What the home's files occupy, by kind.
    Storage {},
    /// The home's largest files, the largest first.
    Largest {
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        language: Option<String>,
    },
}

/// An application the session lists.
#[derive(Clone, Debug, Deserialize)]
pub struct Listed {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub icon: Option<PathBuf>,
    #[serde(default)]
    pub package: Option<PathBuf>,
}

/// An item a package declares (`searchable.json`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Declared {
    id: String,
    url: String,
    titles: BTreeMap<String, String>,
    #[serde(default)]
    keywords: BTreeMap<String, Vec<String>>,
}

/// The records of the applications listed, and of the items their packages declare.
pub fn listed_records(listed: &[Listed]) -> (Vec<Record>, Vec<Record>) {
    let mut applications = Vec::new();
    let mut declared = Vec::new();
    for application in listed {
        applications.push(Record {
            id: format!("application:{}", application.id),
            source: Source::Application,
            owner: None,
            category: Category::Application,
            titles: BTreeMap::from([(String::new(), application.name.clone())]),
            keywords: application.keywords.clone(),
            tags: Vec::new(),
            path: None,
            url: None,
            application: Some(application.id.clone()),
            icon: application.icon.as_deref().map(|icon| uri::of(icon, false)),
            media: None,
            size: None,
            allocated: None,
            modified: None,
            created: None,
            changed: None,
        });
        let Some(package) = &application.package else {
            continue;
        };
        let file = package.join("searchable.json");
        let Ok(metadata) = std::fs::metadata(&file) else {
            continue;
        };
        if metadata.len() > MAX_DECLARED {
            continue;
        }
        let Some(items) = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<Declared>>(&bytes).ok())
        else {
            eprintln!(
                "[search] {} is not a list of searchable items",
                file.display()
            );
            continue;
        };
        let settings = application.id == SETTINGS_ID;
        for item in items {
            let id = match item.id.strip_prefix("settings:") {
                Some(page) if settings => format!("settings:{page}"),
                _ => format!("{}:{}", application.id, item.id),
            };
            declared.push(Record {
                id,
                source: Source::Declared,
                owner: Some(application.id.clone()),
                category: if settings {
                    Category::Settings
                } else {
                    Category::Other
                },
                titles: item.titles,
                keywords: item.keywords.into_values().flatten().collect(),
                tags: Vec::new(),
                path: None,
                url: Some(item.url),
                application: Some(application.id.clone()),
                icon: application.icon.as_deref().map(|icon| uri::of(icon, false)),
                media: None,
                size: None,
                allocated: None,
                modified: None,
                created: None,
                changed: None,
            });
        }
    }
    (applications, declared)
}

/// Where a file is, as a person reads it: its folder's path from the home (`Documents › Work`),
/// the home by its name.
fn location(path: &Path, home: &Path) -> Option<String> {
    let parent = path.parent()?;
    let shown: Vec<String> = match parent.strip_prefix(home) {
        Ok(inside) if inside.as_os_str().is_empty() => {
            vec![home.file_name()?.to_string_lossy().into_owned()]
        }
        Ok(inside) => inside
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect(),
        Err(_) => parent
            .components()
            .skip(1)
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect(),
    };
    Some(shown.join(" › "))
}

/// The service's state on its answering thread.
pub struct Service {
    scope: Scope,
    paths: Paths,
    searcher: Searcher,
    /// The crawler's commands (the folders kept out).
    commands: Sender<Command>,
    /// Written to when a command is sent: the crawler's wake.
    wake: UnixStream,
    /// The writer's channel: what the session and the applications give the index.
    writes: Sender<writer::Write>,
    status: Arc<Mutex<Status>>,
    preferences: Preferences,
    usage: HashMap<String, Usage>,
    languages: Vec<String>,
    watchers: HashSet<u32>,
    /// Who follows the index's state, with the state each was last told.
    status_followers: HashMap<u32, Value>,
    /// What to say on the control socket.
    out: Vec<String>,
    /// Answers given when the indexer has committed what was asked (from its thread).
    later: Arc<Mutex<Vec<String>>>,
}

impl Service {
    /// The service over `scope`, its index at `paths`: its writer started on a thread of its own at
    /// nice 10, its crawler on another at the idle class. `told` is written a byte after each
    /// commit and each change of the index's state ([`writer::Wake`]).
    pub fn start(scope: Scope, paths: Paths, told: UnixStream) -> std::io::Result<Service> {
        let index = Index::open(&paths.cache).map_err(std::io::Error::other)?;
        let searcher = index.searcher_handle();
        let preferences = Preferences::load(&paths.config);
        let usage = std::fs::read(paths.data.join("usage.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let status = Arc::new(Mutex::new(Status::default()));
        let (commands, received) = std::sync::mpsc::channel();
        let (wake, woken) = UnixStream::pair()?;
        woken.set_nonblocking(true)?;
        told.set_nonblocking(true)?;
        let told = Wake::new(told);
        let (writes, written) = std::sync::mpsc::channel();
        {
            let status = status.clone();
            let told = told.clone();
            std::thread::Builder::new()
                .name("search-writer".into())
                .spawn(move || writer::run(index, written, status, told))?;
        }
        let indexer = Indexer::new(
            scope.clone(),
            paths.cache.clone(),
            searcher.clone(),
            writes.clone(),
            preferences.excluded.clone(),
            status.clone(),
            told,
        )?;
        std::thread::Builder::new()
            .name("search-crawler".into())
            .spawn(move || {
                crate::sched::idle_thread();
                indexer.run(received, OwnedFd::from(woken));
            })?;
        Ok(Service {
            scope,
            paths,
            searcher,
            commands,
            wake,
            writes,
            status,
            preferences,
            usage,
            languages: vec!["en".into()],
            watchers: HashSet::new(),
            status_followers: HashMap::new(),
            out: Vec::new(),
            later: Arc::new(Mutex::new(Vec::new())),
        })
    }

    fn send(&mut self, command: Command) {
        let _ = self.commands.send(command);
        let _ = (&self.wake).write(&[1]);
    }

    /// Replaces every item of `source` (an owner's) by `records`, committed at once; `done` once
    /// the index holds them.
    fn replace(
        &mut self,
        source: Source,
        owner: Option<String>,
        records: Vec<(Record, Option<String>)>,
        done: Option<Box<dyn FnOnce() + Send>>,
    ) {
        let _ = self.writes.send(writer::Write::RemoveSource(source, owner));
        self.put(records, done);
    }

    /// Puts `records`, committed at once; `done` once the index holds them.
    fn put(
        &mut self,
        records: Vec<(Record, Option<String>)>,
        done: Option<Box<dyn FnOnce() + Send>>,
    ) {
        for (record, text) in records {
            let _ = self.writes.send(writer::Write::Put(Box::new(record), text));
        }
        let _ = self.writes.send(match done {
            Some(done) => writer::Write::Done(done),
            None => writer::Write::Commit,
        });
    }

    /// The indexer committed: the reader takes it, and every watcher is told, and who follows the
    /// state.
    pub fn committed(&mut self) {
        let _ = self.searcher.reload();
        let watchers: Vec<u32> = self.watchers.iter().copied().collect();
        for client in watchers {
            self.answer(client, 0, &json!({"changed": {}}));
        }
        let later = std::mem::take(&mut *self.later.lock().expect("lock"));
        self.out.extend(later);
        self.state_changed();
    }

    /// The index's state may have changed: each follower is told it, if it is not what it was
    /// last told.
    pub fn state_changed(&mut self) {
        if self.status_followers.is_empty() {
            return;
        }
        let status = self.status();
        let told: Vec<u32> = self
            .status_followers
            .iter()
            .filter(|(_, last)| **last != status)
            .map(|(client, _)| *client)
            .collect();
        for client in told {
            self.status_followers.insert(client, status.clone());
            self.answer(client, 0, &json!({ "status": status }));
        }
    }

    /// The index's state.
    fn status(&self) -> Value {
        let status = *self.status.lock().expect("lock");
        json!({
            "indexing": !status.idle(),
            "items": self.searcher.count(),
            "partial": status.partial,
            "bytes": self.searcher.disk_size(),
            "readsPdf": extract::reads_pdf(),
            "read": status.read,
            "rebuilding": status.rebuilding,
            "found": status.found,
        })
    }

    fn set_followed(&self) {
        self.status.lock().expect("lock").followed = !self.status_followers.is_empty();
    }

    /// What to write on the control socket now.
    pub fn take_out(&mut self) -> Vec<String> {
        let mut out = std::mem::take(&mut self.out);
        out.extend(std::mem::take(&mut *self.later.lock().expect("lock")));
        out
    }

    fn answer(&mut self, client: u32, serial: u32, value: &Value) {
        self.out.push(answer_line(client, serial, value));
    }

    /// Handles one line from the program.
    pub fn line(&mut self, line: &str) {
        let Ok(Value::Object(message)) = serde_json::from_str::<Value>(line) else {
            eprintln!("[search] not a message: {line:.80}");
            return;
        };
        let Some((name, body)) = message.into_iter().next() else {
            return;
        };
        match name.as_str() {
            "languages" => {
                if let Ok(languages) = serde_json::from_value::<Vec<String>>(body) {
                    if !languages.is_empty() {
                        self.languages = languages;
                    }
                }
            }
            "applications" => match serde_json::from_value::<Vec<Listed>>(body) {
                Ok(listed) => self.applications(&listed),
                Err(error) => eprintln!("[search] applications: {error}"),
            },
            "gone" => {
                if let Some(client) = body["client"].as_u64() {
                    self.watchers.remove(&(client as u32));
                    if self.status_followers.remove(&(client as u32)).is_some() {
                        self.set_followed();
                    }
                }
            }
            // An application reset (docs/adr/0061-crashes-and-resetting-an-application.mdx,
            // decision 5): what it donated is its own data, and goes with it.
            "forget" => {
                if let Some(application) = body["application"].as_str().filter(|id| !id.is_empty())
                {
                    self.replace(
                        Source::Donated,
                        Some(application.to_string()),
                        Vec::new(),
                        None,
                    );
                }
            }
            "ask" => {
                let client = body["client"].as_u64().unwrap_or(0) as u32;
                let serial = body["serial"].as_u64().unwrap_or(0) as u32;
                let asker: Asker =
                    serde_json::from_value(body["asker"].clone()).unwrap_or_default();
                let text = body["text"].as_str().unwrap_or("");
                let answer = match serde_json::from_str::<Ask>(text) {
                    Ok(ask) => self.ask(client, serial, &asker, ask),
                    Err(_) => Some(refused("invalid")),
                };
                if let Some(answer) = answer {
                    self.answer(client, serial, &answer);
                }
            }
            _ => eprintln!("[search] unknown message {name}"),
        }
    }

    fn applications(&mut self, listed: &[Listed]) {
        let (applications, declared) = listed_records(listed);
        let only =
            |records: Vec<Record>| records.into_iter().map(|record| (record, None)).collect();
        self.replace(Source::Application, None, only(applications), None);
        self.replace(Source::Declared, None, only(declared), None);
    }

    /// An answer given once the indexer committed (its `done`).
    fn once_committed(&self, client: u32, serial: u32, value: Value) -> Box<dyn FnOnce() + Send> {
        let later = self.later.clone();
        Box::new(move || {
            later
                .lock()
                .expect("lock")
                .push(answer_line(client, serial, &value));
        })
    }

    /// Answers `ask`, now, or later (none now).
    fn ask(&mut self, client: u32, serial: u32, asker: &Asker, ask: Ask) -> Option<Value> {
        match ask {
            Ask::Query(asked) => Some(self.query(asker, asked)),
            Ask::Watch {} => {
                self.watchers.insert(client);
                Some(json!({"watching": {}}))
            }
            Ask::Unwatch {} => {
                self.watchers.remove(&client);
                Some(json!({"unwatched": {}}))
            }
            Ask::Used { id } => {
                self.used(&id);
                Some(json!({"noted": {}}))
            }
            Ask::Donate { items } => {
                let Some(owner) = asker.application().map(String::from) else {
                    return Some(refused("notPermitted"));
                };
                if items.len() > MAX_DONATED {
                    return Some(refused("invalid"));
                }
                let mut records = Vec::new();
                for item in items {
                    if !matches!(
                        item.category,
                        Category::Contact
                            | Category::Event
                            | Category::Mail
                            | Category::Message
                            | Category::Other
                    ) || item.id.is_empty()
                        || item.title.len() > 1024
                    {
                        return Some(refused("invalid"));
                    }
                    let mut text = item.text;
                    if let Some(text) = text.as_mut() {
                        if text.len() > MAX_DONATED_TEXT {
                            let mut end = MAX_DONATED_TEXT;
                            while !text.is_char_boundary(end) {
                                end -= 1;
                            }
                            text.truncate(end);
                        }
                    }
                    records.push((
                        Record {
                            id: format!("{owner}:{}", item.id),
                            source: Source::Donated,
                            owner: Some(owner.clone()),
                            category: item.category,
                            titles: BTreeMap::from([(String::new(), item.title)]),
                            keywords: item.keywords,
                            tags: Vec::new(),
                            path: None,
                            url: Some(item.url),
                            application: Some(owner.clone()),
                            icon: None,
                            media: None,
                            size: None,
                            allocated: None,
                            modified: item.modified,
                            created: None,
                            changed: None,
                        },
                        text,
                    ));
                }
                let done = self.once_committed(client, serial, json!({"done": {}}));
                self.put(records, Some(done));
                None
            }
            Ask::Remove { ids } => {
                let Some(owner) = asker.application().map(String::from) else {
                    return Some(refused("notPermitted"));
                };
                let done = self.once_committed(client, serial, json!({"done": {}}));
                for id in ids {
                    let _ = self
                        .writes
                        .send(writer::Write::Remove(format!("{owner}:{id}")));
                }
                let _ = self.writes.send(writer::Write::Done(done));
                None
            }
            Ask::RemoveAll {} => {
                let Some(owner) = asker.application().map(String::from) else {
                    return Some(refused("notPermitted"));
                };
                let done = self.once_committed(client, serial, json!({"done": {}}));
                self.replace(Source::Donated, Some(owner), Vec::new(), Some(done));
                None
            }
            Ask::Preferences {} => {
                if !asker.settings {
                    return Some(refused("notPermitted"));
                }
                Some(json!({"preferences": self.preferences.json()}))
            }
            Ask::SetPreferences {
                categories,
                excluded,
            } => {
                if !asker.settings {
                    return Some(refused("notPermitted"));
                }
                let mut preferences = self.preferences.clone();
                if let Some(categories) = categories {
                    for (category, shown) in categories {
                        preferences.hidden.retain(|hidden| *hidden != category);
                        if !shown {
                            preferences.hidden.push(category);
                        }
                    }
                    preferences.hidden.sort();
                }
                if let Some(excluded) = excluded {
                    let mut paths = Vec::new();
                    for folder in excluded {
                        let Ok(path) = uri::path_of(&folder) else {
                            return Some(refused("invalid"));
                        };
                        let path = PathBuf::from(path.trim_end_matches('/'));
                        if !path.is_absolute() || path == Path::new("") {
                            return Some(refused("invalid"));
                        }
                        if !paths.contains(&path) {
                            paths.push(path);
                        }
                    }
                    preferences.excluded = paths;
                }
                if let Err(error) = preferences.save(&self.paths.config) {
                    eprintln!("[search] the preferences are not kept: {error}");
                }
                if preferences.excluded != self.preferences.excluded {
                    self.send(Command::Excluded(preferences.excluded.clone()));
                }
                self.preferences = preferences;
                Some(json!({"preferences": self.preferences.json()}))
            }
            Ask::Status {} => Some(json!({"status": self.status()})),
            Ask::FollowStatus {} => {
                let status = self.status();
                self.status_followers.insert(client, status.clone());
                self.set_followed();
                Some(json!({ "status": status }))
            }
            Ask::UnfollowStatus {} => {
                self.status_followers.remove(&client);
                self.set_followed();
                Some(json!({"unfollowed": {}}))
            }
            Ask::Rebuild {} => {
                if !asker.settings {
                    return Some(refused("notPermitted"));
                }
                // Rebuilding from the crawler's asking: the state says so at once.
                self.status.lock().expect("lock").rebuilding = true;
                self.send(Command::Rebuild);
                Some(json!({"rebuilding": {}}))
            }
            Ask::Storage {} => {
                if !asker.settings {
                    return Some(refused("notPermitted"));
                }
                Some(self.storage())
            }
            Ask::Largest { limit, language } => {
                if !asker.settings {
                    return Some(refused("notPermitted"));
                }
                Some(self.largest(limit, language))
            }
        }
    }

    /// What the home's files occupy by kind, from the index (Settings › Storage).
    fn storage(&self) -> Value {
        let home = self.scope.home().to_path_buf();
        let occupied = match self.searcher.occupied(&home) {
            Ok(occupied) => occupied,
            Err(error) => {
                eprintln!("[search] what the files occupy could not be read: {error}");
                return refused("unavailable");
            }
        };
        let kinds: serde_json::Map<String, Value> = occupied
            .into_iter()
            .map(|(category, (files, bytes))| {
                (
                    category.name().to_string(),
                    json!({"files": files, "bytes": bytes}),
                )
            })
            .collect();
        let status = *self.status.lock().expect("lock");
        json!({"storage": {
            "home": uri::of(&home, true),
            "kinds": kinds,
            "indexing": !status.idle(),
            "partial": status.partial,
        }})
    }

    /// The home's largest files, as a question's items (Settings › Storage's Review Files).
    fn largest(&self, limit: Option<usize>, language: Option<String>) -> Value {
        let home = self.scope.home().to_path_buf();
        let limit = limit.unwrap_or(DEFAULT_LARGEST).clamp(1, MAX_LIMIT);
        let records = match self.searcher.largest(&home, limit) {
            Ok(records) => records,
            Err(error) => {
                eprintln!("[search] the largest files could not be read: {error}");
                return refused("unavailable");
            }
        };
        let language = language
            .or_else(|| self.languages.first().cloned())
            .unwrap_or_default();
        let items: Vec<Value> = records
            .iter()
            .map(|record| self.item(record, &language, &home, None))
            .collect();
        json!({"results": {
            "items": items,
            "topHit": Value::Null,
            "indexing": !self.status.lock().expect("lock").idle(),
        }})
    }

    /// An item as an answer gives it (decision 7), with its snippet if it has one.
    fn item(&self, record: &Record, language: &str, home: &Path, snippet: Option<String>) -> Value {
        let mut item = serde_json::Map::new();
        item.insert("id".into(), json!(record.id));
        item.insert("category".into(), json!(record.category.name()));
        item.insert("title".into(), json!(record.title(language)));
        if let Some(path) = &record.path {
            item.insert(
                "uri".into(),
                json!(uri::of(path, record.category == Category::Folder)),
            );
            if let Some(location) = location(path, home) {
                item.insert("location".into(), json!(location));
            }
        }
        if let Some(snippet) = snippet {
            item.insert("snippet".into(), json!(snippet));
        }
        let optional = [
            ("url", record.url.as_ref().map(|url| json!(url))),
            (
                "application",
                record
                    .application
                    .as_ref()
                    .or(record.owner.as_ref())
                    .map(|id| json!(id)),
            ),
            ("icon", record.icon.as_ref().map(|icon| json!(icon))),
            (
                "contentType",
                record.media.as_ref().map(|media| json!(media)),
            ),
            ("size", record.size.map(|size| json!(size))),
            ("modified", record.modified.map(|time| json!(time))),
            ("created", record.created.map(|time| json!(time))),
            (
                "lastUsed",
                self.usage.get(&record.id).map(|usage| json!(usage.last)),
            ),
            (
                "tags",
                (!record.tags.is_empty()).then(|| json!(record.tags)),
            ),
        ];
        for (name, value) in optional {
            if let Some(value) = value {
                item.insert(name.into(), value);
            }
        }
        Value::Object(item)
    }

    fn used(&mut self, id: &str) {
        if id.is_empty() || id.len() > 4096 {
            return;
        }
        let entry = self.usage.entry(id.to_string()).or_default();
        entry.count = entry.count.saturating_add(1);
        entry.last = now();
        if self.usage.len() > MAX_USAGE {
            // The least recently used go.
            let mut by_last: Vec<(i64, String)> = self
                .usage
                .iter()
                .map(|(id, usage)| (usage.last, id.clone()))
                .collect();
            by_last.sort();
            for (_, id) in by_last.into_iter().take(self.usage.len() - MAX_USAGE) {
                self.usage.remove(&id);
            }
        }
        let bytes = serde_json::to_vec(&self.usage).expect("JSON");
        if let Err(error) = write_atomically(&self.paths.data.join("usage.json"), &bytes) {
            eprintln!("[search] what was opened is not kept: {error}");
        }
    }

    fn query(&mut self, asker: &Asker, asked: Asked) -> Value {
        let now = now();
        let mut within = Vec::new();
        for folder in &asked.within {
            let Ok(path) = uri::path_of(folder) else {
                return refused("invalid");
            };
            let path = PathBuf::from(path.trim_end_matches('/'));
            within.push(std::fs::canonicalize(&path).unwrap_or(path));
        }
        let parsed = query::parse(&asked.text, &asked.tokens, now);
        let asked_categories = asked.categories.clone();
        let question = Question {
            parsed: parsed.clone(),
            within,
            categories: asked.categories,
            limit: asked.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
        };
        let viewer = Viewer {
            shell: asker.shell,
            application: asker.application().map(String::from),
            // The person's switches are Spotlight's: a question that names its categories (the
            // launcher's, for applications) is not Spotlight's, as a Mac's Launchpad finds every
            // application whatever Spotlight shows.
            hidden: if asker.shell {
                self.preferences
                    .hidden
                    .iter()
                    .filter(|category| !asked_categories.contains(category))
                    .copied()
                    .collect()
            } else {
                Vec::new()
            },
        };
        let language = asked
            .language
            .or_else(|| self.languages.first().cloned())
            .unwrap_or_default();
        let (found, top) = match self.searcher.search(&question, &viewer, &self.usage, now) {
            Ok(answer) => answer,
            Err(error) => {
                eprintln!("[search] a question failed: {error}");
                (Vec::new(), None)
            }
        };
        let home = self.scope.home().to_path_buf();
        let items: Vec<Value> = found
            .iter()
            .enumerate()
            .map(|(at, found)| {
                let record = &found.record;
                let snippet = record
                    .path
                    .as_ref()
                    .filter(|_| at < SNIPPETS && parsed.has_text())
                    .and_then(|path| snippet_of(path, record, &parsed));
                self.item(record, &language, &home, snippet)
            })
            .collect();
        let top_hit = top.map(|at| found[at].record.id.clone());
        json!({"results": {
            "items": items,
            "topHit": top_hit,
            "indexing": !self.status.lock().expect("lock").idle(),
        }})
    }
}

/// A snippet of a text file's or an office document's text around what matched; none for a
/// PDF, whose reader is too slow for a question's time.
fn snippet_of(path: &Path, record: &Record, parsed: &query::Parsed) -> Option<String> {
    if !matches!(
        record.category,
        Category::Text
            | Category::Source
            | Category::Document
            | Category::Presentation
            | Category::Spreadsheet
    ) {
        return None;
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let mut text = extract::text(path, record.category, extension.as_deref(), record.size?)?;
    // The first 256 KiB are enough to find a word in.
    if text.len() > 256 * 1024 {
        let mut end = 256 * 1024;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    Searcher::snippet(&text, parsed)
}

fn refused(reason: &str) -> Value {
    json!({"refused": {"reason": reason}})
}

fn answer_line(client: u32, serial: u32, value: &Value) -> String {
    json!({"answer": {"client": client, "serial": serial, "text": value.to_string()}}).to_string()
}

/// The service's process: `<program> --search-service`, run by the program running the
/// compositor with its standard input the control socket. Ends when the program closes it.
pub fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[search] the search service stopped: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> std::io::Result<()> {
    // It ends with the program that started it, whatever ends that.
    rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::TERM))?;
    let _ = rustix::thread::set_name(c"oxyos-search");
    let control = UnixStream::from(std::io::stdin().as_fd().try_clone_to_owned()?);
    serve(control, Scope::for_user()?, None)
}

/// Serves the control socket `control` over `scope`'s places, its files at `paths` (the user's
/// when none), until the socket closes.
pub fn serve(control: UnixStream, scope: Scope, paths: Option<Paths>) -> std::io::Result<()> {
    let paths = paths.unwrap_or_else(|| Paths::for_user(&scope));
    let (told, woken) = UnixStream::pair()?;
    woken.set_nonblocking(true)?;
    let mut service = Service::start(scope, paths, told)?;
    let mut reader = BufReader::new(control.try_clone()?);
    let mut writer = control;
    let mut line = String::new();
    loop {
        // What is buffered first: one read may have brought several lines.
        if reader.buffer().is_empty() {
            let mut fds = [
                PollFd::new(reader.get_ref(), PollFlags::IN),
                PollFd::new(&woken, PollFlags::IN),
            ];
            match poll(&mut fds, None) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(error.into()),
            }
            if !fds[1].revents().is_empty() {
                let mut bytes = [0u8; 256];
                let mut results = false;
                while let Ok(read @ 1..) = std::io::Read::read(&mut &woken, &mut bytes) {
                    results |= bytes[..read].contains(&writer::RESULTS);
                }
                if results {
                    service.committed();
                } else {
                    service.state_changed();
                }
            }
            if fds[0].revents().is_empty() {
                flush(&mut service, &mut writer)?;
                continue;
            }
        }
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        service.line(line.trim_end());
        flush(&mut service, &mut writer)?;
    }
}

fn flush(service: &mut Service, writer: &mut UnixStream) -> std::io::Result<()> {
    for line in service.take_out() {
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}
