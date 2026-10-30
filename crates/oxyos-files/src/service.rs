//! The file service the Bloom host gives an application (`OxyOSFiles`, docs/adr/0027-files.mdx,
//! decision 1): requests as JSON, run in the order they came on a thread of the service's own (so
//! a slow disk never stalls the application's frames), and answers and changes as JSON, read
//! from a queue whose descriptor is readable while anything is in it.
//!
//! A request is `{"id": N, "op": OP, ...}`; its answer `{"id": N, "ok": true, "value": ...}` or
//! `{"id": N, "ok": false, "error": CODE, "message": TEXT}` ([`Error::code`]). A watched change is
//! `{"changed": WATCH}`. The operations:
//!
//! | `op` | Arguments | Value |
//! | --- | --- | --- |
//! | `places` | | `{home, favourites: [{key, path}], volumes: [{name, path}], trash: COUNT}` |
//! | `list` | `path` | `{path, entries: [ENTRY]}` (the folder canonical) |
//! | `stat` | `path` | `ENTRY` (the item itself) |
//! | `createFolder` | `parent`, `name` | `ENTRY` |
//! | `rename` | `path`, `name` | `ENTRY` |
//! | `move` | `paths`, `to` | `[{path, ok, to | error}]` |
//! | `trash` | `paths` | `[{path, ok, id | error}]` |
//! | `trashList` | | `[TRASHED]` |
//! | `restore` | `ids` | `[{id, ok, path | error}]` |
//! | `deleteFromTrash` | `ids` | `[{id, ok | error}]` |
//! | `emptyTrash` | | the count removed |
//! | `watch` | `path`, or `trash: true`, or `places: true` | the watch's number |
//! | `unwatch` | `watch` | null |
//!
//! `ENTRY` is `{name, path, folder, link, size, modified, hidden}` ([`Entry::json`]); `TRASHED`
//! adds `id`, `original` and `deleted` ([`trash::Trashed::json`]).

use crate::entries::{self, Entry};
use crate::places::{self, Favourite};
use crate::trash;
use crate::watch::Watcher;
use crate::{Error, Scope};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// The longest request taken, in bytes.
pub const MAX_REQUEST_BYTES: usize = 1 << 20;

struct Shared {
    answers: Mutex<VecDeque<String>>,
    wake: Mutex<UnixStream>,
}

impl Shared {
    fn push(&self, answer: String) {
        self.answers.lock().expect("lock").push_back(answer);
        let _ = self.wake.lock().expect("lock").write_all(&[1]);
    }
}

/// The service: its thread runs requests; its queue holds what it answered.
pub struct Service {
    shared: Arc<Shared>,
    readable: UnixStream,
    requests: Option<mpsc::Sender<String>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Service {
    /// The service for `scope`.
    pub fn new(scope: Scope) -> io::Result<Service> {
        let (readable, wake) = UnixStream::pair()?;
        readable.set_nonblocking(true)?;
        wake.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            answers: Mutex::new(VecDeque::new()),
            wake: Mutex::new(wake),
        });
        let watcher = {
            let shared = shared.clone();
            Watcher::new(scope.mountinfo(), move |ids| {
                for id in ids {
                    shared.push(json!({ "changed": id }).to_string());
                }
            })?
        };
        let (requests, received) = mpsc::channel::<String>();
        let thread = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("oxyos-files".into())
                .spawn(move || {
                    for request in received {
                        shared.push(answer(&scope, &watcher, &request));
                    }
                })?
        };
        Ok(Service {
            shared,
            readable,
            requests: Some(requests),
            thread: Some(thread),
        })
    }

    /// Queues a request; it is answered in order.
    pub fn request(&self, json: &str) {
        if let Some(requests) = &self.requests {
            let _ = requests.send(json.to_string());
        }
    }

    /// The next answer or change, if any; never blocks.
    pub fn take(&self) -> Option<String> {
        let mut answers = self.shared.answers.lock().expect("lock");
        let answer = answers.pop_front();
        if answers.is_empty() {
            let mut drained = [0u8; 64];
            while matches!((&self.readable).read(&mut drained), Ok(n) if n > 0) {}
        }
        answer
    }

    /// Puts an answer taken back at the front (the host had no room for it yet).
    pub fn untake(&self, answer: String) {
        self.shared.answers.lock().expect("lock").push_front(answer);
        let _ = self.shared.wake.lock().expect("lock").write_all(&[1]);
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        // The thread ends once the requests' channel closes.
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl AsFd for Service {
    /// Readable while answers are queued.
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.readable.as_fd()
    }
}

/// The answer to one request, as JSON.
pub fn answer(scope: &Scope, watcher: &Watcher, request: &str) -> String {
    let parsed: Result<Value, _> = if request.len() > MAX_REQUEST_BYTES {
        Ok(Value::Null)
    } else {
        serde_json::from_str(request)
    };
    let request = parsed.unwrap_or(Value::Null);
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    match run(scope, watcher, &request) {
        Ok(value) => json!({ "id": id, "ok": true, "value": value }),
        Err(error) => json!({
            "id": id, "ok": false, "error": error.code(), "message": error.to_string(),
        }),
    }
    .to_string()
}

fn text<'a>(request: &'a Value, key: &str) -> Result<&'a str, Error> {
    request
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Refused(format!("`{key}` is missing")))
}

fn texts<'a>(request: &'a Value, key: &str) -> Result<Vec<&'a str>, Error> {
    request
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Refused(format!("`{key}` is missing")))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| Error::Refused(format!("`{key}` holds paths")))
        })
        .collect()
}

/// One item's result in a request about several: what `done` gave under `key`, or its error.
fn each<T: Into<Value>>(name: &str, item: &str, key: &str, done: Result<T, Error>) -> Value {
    match done {
        Ok(value) => json!({ name: item, "ok": true, key: value.into() }),
        Err(error) => json!({
            name: item, "ok": false, "error": error.code(), "message": error.to_string(),
        }),
    }
}

fn run(scope: &Scope, watcher: &Watcher, request: &Value) -> Result<Value, Error> {
    let op = request
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Refused("not a request".into()))?;
    match op {
        "places" => Ok(json!({
            "home": scope.home().to_string_lossy(),
            "favourites": places::favourites(scope)
                .iter()
                .map(|(favourite, path): &(Favourite, _)| {
                    json!({ "key": favourite.key(), "path": path.to_string_lossy() })
                })
                .collect::<Vec<_>>(),
            "volumes": scope
                .volumes()
                .iter()
                .map(|volume| json!({ "name": volume.name, "path": volume.path.to_string_lossy() }))
                .collect::<Vec<_>>(),
            "trash": trash::count(scope),
        })),
        "list" => {
            let folder = scope.folder(text(request, "path")?)?;
            let entries = entries::list(&folder)?;
            Ok(json!({
                "path": folder.to_string_lossy(),
                "entries": entries.iter().map(Entry::json).collect::<Vec<_>>(),
            }))
        }
        "stat" => {
            let path = text(request, "path")?;
            // The home and a volume's root are folders to show, not items to change.
            let item = scope.item(path).or_else(|_| scope.folder(path))?;
            Ok(entries::entry(&item)?.json())
        }
        "createFolder" => {
            let parent = scope.folder(text(request, "parent")?)?;
            Ok(entries::create_folder(&parent, text(request, "name")?)?.json())
        }
        "rename" => {
            let item = scope.item(text(request, "path")?)?;
            Ok(entries::rename(&item, text(request, "name")?)?.json())
        }
        "move" => {
            let folder = scope.folder(text(request, "to")?)?;
            Ok(texts(request, "paths")?
                .into_iter()
                .map(|path| {
                    let moved = scope
                        .item(path)
                        .and_then(|item| entries::move_into(&item, &folder))
                        .map(|to| to.to_string_lossy().into_owned());
                    each("path", path, "to", moved)
                })
                .collect())
        }
        "trash" => Ok(texts(request, "paths")?
            .into_iter()
            .map(|path| {
                let trashed = scope
                    .item(path)
                    .and_then(|item| trash::trash(scope, &item))
                    .map(|id| id.to_string_lossy().into_owned());
                each("path", path, "id", trashed)
            })
            .collect()),
        "trashList" => Ok(trash::list(scope)
            .iter()
            .map(trash::Trashed::json)
            .collect()),
        "restore" => Ok(texts(request, "ids")?
            .into_iter()
            .map(|id| {
                let restored =
                    trash::restore(scope, id).map(|path| path.to_string_lossy().into_owned());
                each("id", id, "path", restored)
            })
            .collect()),
        "deleteFromTrash" => Ok(texts(request, "ids")?
            .into_iter()
            .map(|id| each("id", id, "removed", trash::remove(scope, id).map(|()| true)))
            .collect()),
        "emptyTrash" => Ok(trash::empty(scope)?.into()),
        "watch" => {
            let id = if request.get("places").and_then(Value::as_bool) == Some(true) {
                watcher.watch(&[], true)?
            } else if request.get("trash").and_then(Value::as_bool) == Some(true) {
                // The home's Trash is made so it can be watched before anything is in it.
                trash::home_trash(scope).make()?;
                let folders: Vec<_> = trash::trashes(scope)
                    .iter()
                    .map(trash::TrashDir::files)
                    .collect();
                watcher.watch(&folders, true)?
            } else {
                let folder = scope.folder(text(request, "path")?)?;
                watcher.watch(&[folder], false)?
            };
            Ok(id.into())
        }
        "unwatch" => {
            let id = request
                .get("watch")
                .and_then(Value::as_u64)
                .and_then(|id| u32::try_from(id).ok())
                .ok_or_else(|| Error::Refused("`watch` is missing".into()))?;
            watcher.unwatch(id);
            Ok(Value::Null)
        }
        other => Err(Error::Refused(format!("no operation {other:?}"))),
    }
}
