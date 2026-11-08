//! The file service the Bloom host gives an application (`OxyOSFiles`,
//! docs/adr/0028-file-access.mdx): `expo-file-system`'s operations and the Bloom adapter's places
//! and Trash, as JSON requests with bytes beside them.
//!
//! A request is answered either at once on the caller's thread ([`Service::call`]: what
//! `expo-file-system` does synchronously on the JavaScript thread, as on Android) or in the order
//! requests came on a thread of the service's own ([`Service::request`], its asynchronous calls,
//! so a slow disk never stalls the application's frames), the answer then taken from a queue whose
//! descriptor is readable while anything is in it ([`Service::take`]). The same queue says what
//! watches saw.
//!
//! A request is `{"op": OP, ...}` and its bytes (what `write` writes); an answer
//! `{"ok": true, "value": ...}` with its bytes (what `read` read), or
//! `{"ok": false, "error": CODE, "message": TEXT}` ([`Error::code`]). An event, a reply with id 0,
//! is `{"watch": N}` for a coalesced watch, or `{"watch": N, "events": [EVENT], "ended": BOOL}`
//! for a typed one, `EVENT` being `{type, uri, isDirectory, flags, newUri?, newIsDirectory?}`;
//! `{"transfer": ID, "progress": {...}}` for a download or an upload running as request `ID`.
//!
//! | `op` | Arguments | Value (bytes) |
//! | --- | --- | --- |
//! | `paths` | | `{documentDirectory, cacheDirectory, bundleDirectory}` |
//! | `diskSpace` | | `{total, available}` |
//! | `stat` | `uri` | `{exists, directory, size, modified, created}` |
//! | `size` | `uri` | a folder's bytes |
//! | `mimeType` | `uri` | its name's media type, or null |
//! | `read` | `uri`, `encoding` | null (the content) |
//! | `write` | `uri`, `encoding`, `append` (bytes: the content) | null |
//! | `createFile` | `uri`, `intermediates`, `overwrite` | null |
//! | `createDirectory` | `uri`, `intermediates`, `overwrite`, `idempotent` | null |
//! | `createChild` | `uri`, `name`, `directory` | the child's URI |
//! | `delete` | `uri` | null |
//! | `copy`, `move` | `uri`, `directory`, `to`, `toDirectory`, `overwrite` | the URI there |
//! | `rename` | `uri`, `directory`, `name` | the new URI |
//! | `list` | `uri` | `[{uri, isDirectory}]` |
//! | `digest` | `uri`, `algorithm` | lowercase hexadecimal |
//! | `open` | `uri`, `mode` | the handle |
//! | `handleRead` | `handle`, `length` | null (the bytes) |
//! | `handleWrite` | `handle` (bytes) | null |
//! | `handleOffset`, `handleSize` | `handle` | a number |
//! | `handleSeek` | `handle`, `offset` | null |
//! | `handleClose` | `handle` | null |
//! | `watchPath` | `uri`, `debounce` (ms) | the watch |
//! | `places` | | `{home, favourites: [{key, uri}], volumes: [{name, uri}], trash: COUNT}` |
//! | `trash` | `uris` | `[{uri, ok, result | error, message}]`, `result` its URI in the Trash |
//! | `trashed` | | `[{uri, name, isDirectory, originalUri, deletedAt, size, lastModified, tags}]` |
//! | `restore` | `uris` (in the Trash) | `[{uri, ok, result | error, message}]` |
//! | `erase` | `uris` (in the Trash) | `[{uri, ok, result | error, message}]` |
//! | `emptyTrash` | | the count removed |
//! | `watchTrash`, `watchPlaces` | | the watch |
//! | `tagsIn` | `uri` (a folder) | `{ITEM_URI: [TAG]}`: each item with tags, its URI as `list` writes it |
//! | `tagsOf` | `uris` | `[[TAG] | null]`: null for an item not there or not reached |
//! | `changeTags` | `uris`, `add: [TAG]`, `remove: [NAME]` | `[{uri, ok, result | error, message}]`, `result` its URI |
//! | `tagged` (requested) | `name` | `[{uri, name, isDirectory, size, lastModified, tags}]`, from the index |
//! | `tagNames` (requested) | | `[{name, count}]`, from the index |
//! | `watchTags` | | the watch: the index's items changed |
//! | `unwatch` | `watch` | null |
//! | `download` (requested) | `url`, `to`, `toDirectory`, `headers`, `idempotent`, `resume` | the file's URI, null if paused |
//! | `upload` (requested) | `uri`, `url`, `method`, `headers`, `uploadType`, `fieldName`, `mimeType`, `parameters` | `{body, status, headers}` |
//! | `cancelTransfer` | `transfer` (its request's id) | null |
//! | `pauseTransfer` | `transfer` | `{resumeData}`: the bytes written, or null |
//!
//! A `TAG` is `{name, color}`, `color` one of Finder's seven (`red`, `orange`, `yellow`, `green`,
//! `blue`, `purple`, `grey`) or null (docs/adr/0049-file-tags.mdx). `changeTags` removes, then
//! adds (a name removed and added back keeps its place, and a new one takes a removed one's: a
//! rename in place); a name that cannot be a tag's refuses the whole request (`invalid`).
//! `tagged` and `tagNames` start the index of the person's tagged items ([`TagIndex`]) and are
//! answered once its first crawl ended, never holding the requests behind them; `watchTags`
//! starts it too.

use crate::access::{self, Encoding, Handles, Mode, Relocation};
use crate::places::{self, Favourite};
use crate::scope::Access;
use crate::tag_index::{TagIndex, Tagged};
use crate::tags::{self, Color, Tag};
use crate::transfer::{self, Live};
use crate::watch::{Change, Event, Watcher};
use crate::{trash, uri, Error, Scope};
use oxyos_net::Network;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The longest request taken, in bytes (its JSON; the bytes beside it are the content).
pub const MAX_REQUEST_BYTES: usize = 1 << 20;

/// An answer, or an event (id 0), with its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub id: u64,
    pub json: String,
    pub bytes: Vec<u8>,
}

struct Shared {
    replies: Mutex<VecDeque<Reply>>,
    wake: Mutex<UnixStream>,
}

impl Shared {
    fn push(&self, reply: Reply) {
        self.replies.lock().expect("lock").push_back(reply);
        let _ = self.wake.lock().expect("lock").write_all(&[1]);
    }

    fn event(&self, json: Value) {
        self.push(Reply {
            id: 0,
            json: json.to_string(),
            bytes: Vec::new(),
        });
    }
}

/// Whether the application the service is for may reach the network: an installed version only if
/// its account granted `network` (docs/adr/0030-installing-applications.mdx, decision 8), which its
/// downloads and uploads need as its own requests do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkAccess {
    Granted,
    Refused,
}

/// Why a transfer is refused to an application without the network: the words the host's own
/// requests, sockets and images are refused with.
pub const NOT_GRANTED_THE_NETWORK: &str = "this application was not granted the network";

/// What every request runs against: the scope, the watches, the open files and the transfers.
struct Context {
    scope: Scope,
    watcher: Watcher,
    handles: Handles,
    /// None for an application without the network: nothing of it is made.
    network: Option<Network>,
    /// Downloads and uploads running, by their request's id.
    transfers: Mutex<HashMap<u64, Arc<Live>>>,
    /// The index of tagged items, once an application asked it something.
    tag_index: Mutex<Option<Arc<TagIndex>>>,
    /// The watches told when it changes (`watchTags`).
    tag_watches: Arc<Mutex<BTreeSet<u32>>>,
    shared: Arc<Shared>,
}

impl Context {
    /// The index of tagged items, started the first time it is wanted.
    fn tag_index(&self) -> Result<Arc<TagIndex>, Error> {
        // The index crawls every one of the person's files, never only items given.
        self.scope.reaches_the_persons_files()?;
        let mut index = self.tag_index.lock().expect("lock");
        if let Some(index) = index.as_ref() {
            return Ok(index.clone());
        }
        let (shared, watches) = (self.shared.clone(), self.tag_watches.clone());
        let started = Arc::new(TagIndex::start(self.scope.clone(), move || {
            for id in watches.lock().expect("lock").iter() {
                shared.event(json!({ "watch": id }));
            }
        })?);
        *index = Some(started.clone());
        Ok(started)
    }
}

/// The service: its thread runs requests; its queue holds what it answered and saw.
pub struct Service {
    context: Arc<Context>,
    shared: Arc<Shared>,
    readable: UnixStream,
    requests: Option<mpsc::Sender<(u64, String, Vec<u8>)>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// A typed watch's event as JSON.
fn event_json(event: &Event) -> Value {
    let mut value = json!({
        "type": event.kind.name(),
        "uri": uri::of(&event.path, false),
        "isDirectory": event.directory,
        "flags": event.flags,
    });
    if let Some((path, directory)) = &event.new_path {
        value["newUri"] = uri::of(path, false).into();
        value["newIsDirectory"] = (*directory).into();
    }
    value
}

impl Service {
    /// The service for `scope`, its downloads and uploads reaching the network only with `network`.
    pub fn new(scope: Scope, network: NetworkAccess) -> io::Result<Service> {
        let (readable, wake) = UnixStream::pair()?;
        readable.set_nonblocking(true)?;
        wake.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            replies: Mutex::new(VecDeque::new()),
            wake: Mutex::new(wake),
        });
        let watcher = {
            let shared = shared.clone();
            Watcher::new(scope.mountinfo(), move |change| match change {
                Change::Watches(ids) => {
                    for id in ids {
                        shared.event(json!({ "watch": id }));
                    }
                }
                Change::Events {
                    watch,
                    events,
                    ended,
                } => shared.event(json!({
                    "watch": watch,
                    "events": events.iter().map(event_json).collect::<Vec<_>>(),
                    "ended": ended,
                })),
            })?
        };
        let context = Arc::new(Context {
            scope,
            watcher,
            handles: Handles::default(),
            network: match network {
                NetworkAccess::Granted => Some(Network::new()?),
                NetworkAccess::Refused => None,
            },
            transfers: Mutex::new(HashMap::new()),
            tag_index: Mutex::new(None),
            tag_watches: Arc::new(Mutex::new(BTreeSet::new())),
            shared: shared.clone(),
        });
        let (requests, received) = mpsc::channel::<(u64, String, Vec<u8>)>();
        let thread = {
            let (context, shared) = (context.clone(), shared.clone());
            std::thread::Builder::new()
                .name("oxyos-files".into())
                .spawn(move || {
                    for (id, request, bytes) in received {
                        // Downloads, uploads and the index's questions are answered when they
                        // end, never holding the requests behind them.
                        let parsed = (request.len() <= MAX_REQUEST_BYTES)
                            .then(|| serde_json::from_str::<Value>(&request).ok())
                            .flatten();
                        let op = parsed
                            .as_ref()
                            .and_then(|parsed| parsed.get("op")?.as_str());
                        let (a_transfer, a_question) = (
                            matches!(op, Some("download" | "upload")),
                            matches!(op, Some("tagged" | "tagNames")),
                        );
                        if let (true, Some(parsed)) = (a_transfer || a_question, parsed) {
                            if a_transfer {
                                transfer(&context, &shared, id, parsed);
                            } else {
                                indexed(&context, &shared, id, &parsed);
                            }
                            continue;
                        }
                        let (json, bytes) = answer(&context, &request, &bytes);
                        shared.push(Reply { id, json, bytes });
                    }
                })?
        };
        Ok(Service {
            context,
            shared,
            readable,
            requests: Some(requests),
            thread: Some(thread),
        })
    }

    /// Answers a request at once, on the caller's thread.
    pub fn call(&self, request: &str, bytes: &[u8]) -> Reply {
        let (json, bytes) = answer(&self.context, request, bytes);
        Reply { id: 0, json, bytes }
    }

    /// Queues a request; it is answered in order, as `id`.
    pub fn request(&self, id: u64, request: &str, bytes: Vec<u8>) {
        if let Some(requests) = &self.requests {
            let _ = requests.send((id, request.to_string(), bytes));
        }
    }

    /// The person's files, granted to the application: its account granted `files` at first use
    /// (docs/adr/0030-installing-applications.mdx, decision 8). Holds for every request after.
    pub fn grant_the_persons_files(&self) {
        self.context.scope.grant_the_persons_files();
    }

    /// The item `uri` (a `file://` URI) of the person's files, given to the application by the
    /// person: chosen for it in the system's panel, dropped on it, opened with it.
    pub fn give(&self, uri: &str) -> Result<(), Error> {
        self.context.scope.give(Path::new(&uri::path_of(uri)?))
    }

    /// A file the scope allows opened for reading, at once: a request's body (`Networking`'s
    /// multipart files).
    pub fn open_read(&self, uri: &str) -> Result<std::fs::File, Error> {
        access::open_read(&self.context.scope, uri)
    }

    /// The next answer or event, if any; never blocks.
    pub fn take(&self) -> Option<Reply> {
        let mut replies = self.shared.replies.lock().expect("lock");
        let reply = replies.pop_front();
        if replies.is_empty() {
            let mut drained = [0u8; 64];
            while matches!((&self.readable).read(&mut drained), Ok(n) if n > 0) {}
        }
        reply
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
    /// Readable while replies are queued.
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.readable.as_fd()
    }
}

/// Starts a download or an upload (`parsed`) on a thread of its own: it is answered when it ends,
/// as `id`, its progress said as events (`{"transfer": ID, "progress": {...}}`).
fn transfer(context: &Arc<Context>, shared: &Arc<Shared>, id: u64, parsed: Value) {
    // An application without the network is refused before anything is made or asked.
    if context.network.is_none() {
        shared.push(Reply {
            id,
            json: ended_json(Err(Error::Network(NOT_GRANTED_THE_NETWORK.into()))),
            bytes: Vec::new(),
        });
        return;
    }
    let live = Arc::new(Live::default());
    context
        .transfers
        .lock()
        .expect("lock")
        .insert(id, live.clone());
    let (context, replies) = (context.clone(), shared.clone());
    let started = std::thread::Builder::new()
        .name(format!("oxyos-files-transfer-{id}"))
        .spawn(move || {
            let shared = replies;
            let progress = |progress: Value| {
                shared.event(json!({ "transfer": id, "progress": progress }));
            };
            let run = if parsed["op"] == "download" {
                transfer::download
            } else {
                transfer::upload
            };
            let ended = match &context.network {
                Some(network) => run(network, &context.scope, &live, &parsed, &progress),
                // Refused before it started (below); never reached.
                None => Err(Error::Network(NOT_GRANTED_THE_NETWORK.into())),
            };
            context.transfers.lock().expect("lock").remove(&id);
            shared.push(Reply {
                id,
                json: ended_json(ended),
                bytes: Vec::new(),
            });
        });
    if let Err(error) = started {
        shared.push(Reply {
            id,
            json: ended_json(Err(error.into())),
            bytes: Vec::new(),
        });
    }
}

/// Asks the index of tagged items `tagged` or `tagNames` (`parsed`): answered as `id` once its
/// first crawl ended, from whichever thread has the answer then, so the requests behind it never
/// wait for the crawl.
fn indexed(context: &Context, shared: &Arc<Shared>, id: u64, parsed: &Value) {
    let op = parsed.get("op").and_then(Value::as_str);
    let reply = {
        let shared = shared.clone();
        move |ended: Result<Value, Error>| {
            shared.push(Reply {
                id,
                json: ended_json(ended),
                bytes: Vec::new(),
            });
        }
    };
    let index = match context.tag_index() {
        Ok(index) => index,
        Err(error) => return reply(Err(error)),
    };
    if op == Some("tagNames") {
        index.names(move |names| {
            reply(Ok(names
                .into_iter()
                .map(|(name, count)| json!({ "name": name, "count": count }))
                .collect()));
        });
    } else {
        match text(parsed, "name").and_then(tags::valid) {
            Ok(name) => index.tagged(name, move |found| reply(Ok(tagged_json(found)))),
            Err(error) => reply(Err(error)),
        }
    }
}

/// The items the index found, as `tagged` answers them: those still there.
fn tagged_json(found: Vec<Tagged>) -> Value {
    found
        .into_iter()
        .filter_map(|(path, tags)| {
            let entry = crate::entries::entry(&path).ok()?;
            Some(json!({
                "uri": uri::of(&path, entry.folder),
                "name": entry.name,
                "isDirectory": entry.folder,
                "size": (!entry.folder).then_some(entry.size),
                "lastModified": entry.modified_ms,
                "tags": tags::json(&tags),
            }))
        })
        .collect()
}

/// The tags a request gives (`add`): `[{name, color}]`, each name one a tag can have.
fn tags_given(request: &Value, key: &str) -> Result<Vec<Tag>, Error> {
    let Some(given) = request.get(key) else {
        return Ok(Vec::new());
    };
    let invalid = || Error::Invalid(format!("`{key}` holds tags: [{{name, color}}]"));
    given
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|tag| {
            let name = tag
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let color = match tag.get("color") {
                None | Some(Value::Null) => None,
                Some(color) => Some(
                    color
                        .as_str()
                        .and_then(Color::parse)
                        .ok_or_else(|| Error::Invalid(format!("no tag colour {color}")))?,
                ),
            };
            Ok(Tag::new(tags::valid(name)?, color))
        })
        .collect()
}

/// The tags of what a folder holds, by each item's URI as `list` writes it; items with none, or
/// whose link leads out of the scope, left out.
fn tags_in(scope: &Scope, folder_uri: &str) -> Result<Value, Error> {
    let folder = scope.reach(&uri::path_of(folder_uri)?, Access::Read)?;
    match std::fs::metadata(&folder) {
        Ok(found) if !found.is_dir() => return Err(Error::NotAFolder),
        Ok(_) => {}
        Err(error) => return Err(error.into()),
    }
    let mut found = serde_json::Map::new();
    for item in std::fs::read_dir(&folder)? {
        let Ok(item) = item else { continue };
        let Some(name) = item.file_name().to_str().map(String::from) else {
            continue;
        };
        let mut path = item.path();
        if item.file_type().is_ok_and(|kind| kind.is_symlink()) {
            match path.to_str().map(|link| scope.reach(link, Access::Read)) {
                Some(Ok(reached)) => path = reached,
                _ => continue,
            }
        }
        let Ok(tags) = tags::read(&path) else {
            continue;
        };
        if !tags.is_empty() {
            let directory = path.is_dir();
            found.insert(uri::child(folder_uri, &name, directory), tags::json(&tags));
        }
    }
    Ok(Value::Object(found))
}

fn ended_json(ended: Result<Value, Error>) -> String {
    match ended {
        Ok(value) => json!({ "ok": true, "value": value }),
        Err(error) => json!({ "ok": false, "error": error.code(), "message": error.to_string() }),
    }
    .to_string()
}

/// The answer to one request, as JSON, and its bytes.
fn answer(context: &Context, request: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    let parsed: Result<Value, _> = if request.len() > MAX_REQUEST_BYTES {
        Ok(Value::Null)
    } else {
        serde_json::from_str(request)
    };
    let request = parsed.unwrap_or(Value::Null);
    match run(context, &request, bytes) {
        Ok((value, bytes)) => (json!({ "ok": true, "value": value }).to_string(), bytes),
        Err(error) => (
            json!({ "ok": false, "error": error.code(), "message": error.to_string() }).to_string(),
            Vec::new(),
        ),
    }
}

fn text<'a>(request: &'a Value, key: &str) -> Result<&'a str, Error> {
    request
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Invalid(format!("`{key}` is missing")))
}

fn flag(request: &Value, key: &str) -> bool {
    request.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn number(request: &Value, key: &str) -> Result<u64, Error> {
    request
        .get(key)
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_f64().filter(|n| *n >= 0.0).map(|n| n as u64))
        })
        .ok_or_else(|| Error::Invalid(format!("`{key}` is missing")))
}

fn texts<'a>(request: &'a Value, key: &str) -> Result<Vec<&'a str>, Error> {
    request
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Invalid(format!("`{key}` is missing")))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| Error::Invalid(format!("`{key}` holds URIs")))
        })
        .collect()
}

/// One item's outcome in a request about several: `{uri, ok, result}` or its error.
fn outcome(item: &str, done: Result<String, Error>) -> Value {
    match done {
        Ok(result) => json!({ "uri": item, "ok": true, "result": result }),
        Err(error) => json!({
            "uri": item, "ok": false, "error": error.code(), "message": error.to_string(),
        }),
    }
}

fn path_uri(path: &Path) -> String {
    uri::of(path, path.is_dir())
}

fn run(context: &Context, request: &Value, bytes: &[u8]) -> Result<(Value, Vec<u8>), Error> {
    let scope = &context.scope;
    let op = request
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Invalid("not a request".into()))?;
    let value = |value: Value| Ok((value, Vec::new()));
    let done = || Ok((Value::Null, Vec::new()));
    let encoding = || {
        request
            .get("encoding")
            .and_then(Value::as_str)
            .map_or(Ok(Encoding::Text), Encoding::parse)
    };
    let relocation = || -> Result<Relocation, Error> {
        Ok(Relocation {
            uri: text(request, "uri")?,
            directory: flag(request, "directory"),
            to: text(request, "to")?,
            to_directory: flag(request, "toDirectory"),
            overwrite: flag(request, "overwrite"),
        })
    };
    // What lists or changes the person's files as a whole (their places, the Trash) needs them
    // all, never only items the person gave the application.
    if matches!(
        op,
        "places" | "watchPlaces" | "trashed" | "watchTrash" | "restore" | "erase" | "emptyTrash"
    ) {
        scope.reaches_the_persons_files()?;
    }
    match op {
        "paths" => value(access::paths(scope)),
        "diskSpace" => value(access::disk_space(scope)?),
        "stat" => value(access::stat(scope, text(request, "uri")?)?),
        "size" => value(access::size(scope, text(request, "uri")?)?.into()),
        "mimeType" => {
            let path = uri::path_of(text(request, "uri")?)?;
            let name = path.rsplit('/').next().unwrap_or_default();
            value(crate::mime::of_name(name).map_or(Value::Null, Value::from))
        }
        "read" => Ok((
            Value::Null,
            access::read(scope, text(request, "uri")?, encoding()?)?,
        )),
        "write" => {
            access::write(
                scope,
                text(request, "uri")?,
                bytes,
                encoding()?,
                flag(request, "append"),
            )?;
            done()
        }
        "createFile" => {
            access::create_file(
                scope,
                text(request, "uri")?,
                flag(request, "intermediates"),
                flag(request, "overwrite"),
            )?;
            done()
        }
        "createDirectory" => {
            access::create_directory(
                scope,
                text(request, "uri")?,
                flag(request, "intermediates"),
                flag(request, "overwrite"),
                flag(request, "idempotent"),
            )?;
            done()
        }
        "createChild" => value(
            access::create_child(
                scope,
                text(request, "uri")?,
                text(request, "name")?,
                flag(request, "directory"),
            )?
            .into(),
        ),
        "delete" => {
            access::delete(scope, text(request, "uri")?)?;
            done()
        }
        "copy" => value(access::copy(scope, &relocation()?)?.into()),
        "move" => value(access::relocate(scope, &relocation()?)?.into()),
        "rename" => value(
            access::rename(
                scope,
                text(request, "uri")?,
                flag(request, "directory"),
                text(request, "name")?,
            )?
            .into(),
        ),
        "list" => value(access::list(scope, text(request, "uri")?)?),
        "digest" => value(
            access::digest(scope, text(request, "uri")?, text(request, "algorithm")?)?.into(),
        ),
        "open" => {
            let mode = request
                .get("mode")
                .and_then(Value::as_str)
                .map_or(Ok(Mode::ReadWrite), Mode::parse)?;
            value(
                context
                    .handles
                    .open(scope, text(request, "uri")?, mode)?
                    .into(),
            )
        }
        "handleRead" => Ok((
            Value::Null,
            context
                .handles
                .read(number(request, "handle")?, number(request, "length")?)?,
        )),
        "handleWrite" => {
            context.handles.write(number(request, "handle")?, bytes)?;
            done()
        }
        "handleOffset" => value(context.handles.offset(number(request, "handle")?)?.into()),
        "handleSeek" => {
            context
                .handles
                .seek(number(request, "handle")?, number(request, "offset")?)?;
            done()
        }
        "handleSize" => value(context.handles.size(number(request, "handle")?)?.into()),
        "handleClose" => {
            context.handles.close(number(request, "handle")?);
            done()
        }
        "watchPath" => {
            let named = uri::path_of(text(request, "uri")?)?;
            let reached = scope.reach(&named, Access::Read)?;
            if std::fs::symlink_metadata(&reached).is_err() {
                return Err(Error::NotFound);
            }
            let debounce = Duration::from_millis(number(request, "debounce").unwrap_or(100));
            value(
                context
                    .watcher
                    .watch_path(&reached, Path::new(&named), debounce)?
                    .into(),
            )
        }
        "places" => value(json!({
            "home": uri::of(scope.home(), true),
            "favourites": places::favourites(scope)
                .iter()
                .map(|(favourite, path): &(Favourite, _)| {
                    json!({ "key": favourite.key(), "uri": uri::of(path, true) })
                })
                .collect::<Vec<_>>(),
            "volumes": scope
                .volumes()
                .iter()
                .map(|volume| json!({ "name": volume.name, "uri": uri::of(&volume.path, true) }))
                .collect::<Vec<_>>(),
            "trash": trash::count(scope),
        })),
        "trash" => value(
            texts(request, "uris")?
                .into_iter()
                .map(|item| {
                    let trashed = uri::path_of(item)
                        .and_then(|path| scope.item(&path))
                        .and_then(|path| trash::trash(scope, &path))
                        .map(|id| path_uri(&id));
                    outcome(item, trashed)
                })
                .collect(),
        ),
        "trashed" => value(
            trash::list(scope)
                .iter()
                .map(|item| {
                    json!({
                        "uri": uri::of(&item.id, item.entry.folder),
                        "name": item.name,
                        "isDirectory": item.entry.folder,
                        "originalUri": item.original.as_ref().map(|path| uri::of(path, item.entry.folder)),
                        "deletedAt": item.deleted_ms,
                        "size": (!item.entry.folder).then_some(item.entry.size),
                        "lastModified": item.entry.modified_ms,
                        "tags": tags::json(&tags::read(&item.id).unwrap_or_default()),
                    })
                })
                .collect(),
        ),
        "restore" => value(
            texts(request, "uris")?
                .into_iter()
                .map(|item| {
                    let restored = uri::path_of(item)
                        .and_then(|id| trash::restore(scope, &id))
                        .map(|path| path_uri(&path));
                    outcome(item, restored)
                })
                .collect(),
        ),
        "erase" => value(
            texts(request, "uris")?
                .into_iter()
                .map(|item| {
                    let erased = uri::path_of(item)
                        .and_then(|id| trash::remove(scope, &id))
                        .map(|()| item.to_string());
                    outcome(item, erased)
                })
                .collect(),
        ),
        "emptyTrash" => value(trash::empty(scope)?.into()),
        "watchPlaces" => value(context.watcher.watch(&[], true)?.into()),
        "watchTrash" => {
            // The home's Trash is made so it can be watched before anything is in it.
            trash::home_trash(scope).make()?;
            let folders: Vec<_> = trash::trashes(scope)
                .iter()
                .map(trash::TrashDir::files)
                .collect();
            value(context.watcher.watch(&folders, true)?.into())
        }
        "tagsIn" => value(tags_in(scope, text(request, "uri")?)?),
        "tagsOf" => value(
            texts(request, "uris")?
                .into_iter()
                .map(|item| {
                    uri::path_of(item)
                        .and_then(|path| scope.reach(&path, Access::Read))
                        .and_then(|path| tags::read(&path))
                        .map_or(Value::Null, |tags| tags::json(&tags))
                })
                .collect(),
        ),
        "changeTags" => {
            let add = tags_given(request, "add")?;
            let remove = match request.get("remove") {
                None => Vec::new(),
                Some(_) => texts(request, "remove")?
                    .into_iter()
                    .map(|name| tags::valid(name).map(String::from))
                    .collect::<Result<Vec<_>, _>>()?,
            };
            value(
                texts(request, "uris")?
                    .into_iter()
                    .map(|item| {
                        let changed = uri::path_of(item)
                            .and_then(|path| scope.reach(&path, Access::Write))
                            .and_then(|path| tags::change(&path, &add, &remove))
                            .map(|_| item.to_string());
                        outcome(item, changed)
                    })
                    .collect(),
            )
        }
        "watchTags" => {
            let _index = context.tag_index()?;
            // A watch of the watcher's own numbering, which the index's changes tell.
            let watch = context.watcher.watch(&[], false)?;
            context.tag_watches.lock().expect("lock").insert(watch);
            value(watch.into())
        }
        "download" | "upload" | "tagged" | "tagNames" => Err(Error::Invalid(format!(
            "{op} is answered when it ends: it is requested, never called"
        ))),
        "cancelTransfer" => {
            let id = number(request, "transfer")?;
            if let Some(live) = context.transfers.lock().expect("lock").get(&id) {
                live.cancel();
            }
            done()
        }
        "pauseTransfer" => {
            let id = number(request, "transfer")?;
            let live = context.transfers.lock().expect("lock").get(&id).cloned();
            let Some(live) = live else {
                return Err(Error::Invalid("the download is not running".into()));
            };
            value(json!({ "resumeData": live.pause() }))
        }
        "unwatch" => {
            let id = number(request, "watch")?;
            let id = u32::try_from(id).map_err(|_| Error::Invalid("no such watch".into()))?;
            context.tag_watches.lock().expect("lock").remove(&id);
            context.watcher.unwatch(id);
            done()
        }
        other => Err(Error::Invalid(format!("no operation {other:?}"))),
    }
}
