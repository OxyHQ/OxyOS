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
//! for a typed one, `EVENT` being `{type, uri, isDirectory, flags, newUri?, newIsDirectory?}`.
//!
//! | `op` | Arguments | Value (bytes) |
//! | --- | --- | --- |
//! | `paths` | | `{documentDirectory, cacheDirectory, bundleDirectory}` |
//! | `diskSpace` | | `{total, available}` |
//! | `stat` | `uri` | `{exists, directory, size, modified, created}` |
//! | `size` | `uri` | a folder's bytes |
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
//! | `trashed` | | `[{uri, name, isDirectory, originalUri, deletedAt, size, lastModified}]` |
//! | `restore` | `uris` (in the Trash) | `[{uri, ok, result | error, message}]` |
//! | `erase` | `uris` (in the Trash) | `[{uri, ok, result | error, message}]` |
//! | `emptyTrash` | | the count removed |
//! | `watchTrash`, `watchPlaces` | | the watch |
//! | `unwatch` | `watch` | null |

use crate::access::{self, Encoding, Handles, Mode, Relocation};
use crate::places::{self, Favourite};
use crate::scope::Access;
use crate::watch::{Change, Event, Watcher};
use crate::{trash, uri, Error, Scope};
use serde_json::{json, Value};
use std::collections::VecDeque;
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

/// What every request runs against: the scope, the watches and the open files.
struct Context {
    scope: Scope,
    watcher: Watcher,
    handles: Handles,
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
    /// The service for `scope`.
    pub fn new(scope: Scope) -> io::Result<Service> {
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
        });
        let (requests, received) = mpsc::channel::<(u64, String, Vec<u8>)>();
        let thread = {
            let (context, shared) = (context.clone(), shared.clone());
            std::thread::Builder::new()
                .name("oxyos-files".into())
                .spawn(move || {
                    for (id, request, bytes) in received {
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
    match op {
        "paths" => value(access::paths(scope)),
        "diskSpace" => value(access::disk_space(scope)?),
        "stat" => value(access::stat(scope, text(request, "uri")?)?),
        "size" => value(access::size(scope, text(request, "uri")?)?.into()),
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
        "unwatch" => {
            let id = number(request, "watch")?;
            context
                .watcher
                .unwatch(u32::try_from(id).map_err(|_| Error::Invalid("no such watch".into()))?);
            done()
        }
        other => Err(Error::Invalid(format!("no operation {other:?}"))),
    }
}
