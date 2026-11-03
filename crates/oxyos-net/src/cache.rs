//! A private HTTP cache on disk (RFC 9111), as OkHttp's `Cache` is for an Android application's
//! client: React Native's remote images go through it (the Bloom host's `Image`), and any request
//! [`Network::fetch_cached`] runs.
//!
//! What is stored: a GET's `200` response, whole, unless the request or the response says
//! `no-store` or the response varies by everything (`Vary: *`). A stored response is used while it
//! is fresh: younger than its `Cache-Control: max-age`, else than its `Expires` after its `Date`,
//! else (no query in its URL, as OkHttp) than a tenth of the time from its `Last-Modified` to its
//! `Date`; its age counts its `Age` and the time it took to arrive (§4.2.3). One that is stale, or
//! says `no-cache`, or is asked for with `no-cache` or `max-age=0`, is revalidated: asked for again
//! with its `ETag` (`If-None-Match`) and its `Last-Modified` (`If-Modified-Since`), and a `304`
//! answer refreshes its headers and serves its body (§4.3.4). The request's values of the headers
//! a response varies by are stored with it, and a request whose differ does not use it.
//!
//! Its size is bounded: the entries least recently used are removed beyond it (each one's file's
//! modification time is its last use, so the order outlives the process); one larger than a
//! quarter of it is not stored. An entry is stored by a key, the URL asked for unless the caller
//! gives another (expo-image's `cacheKey`), as two files, the way OkHttp's `DiskLruCache` keeps an
//! entry's `.0` and `.1`: its metadata (its key, its headers as text and the name and length of
//! its body's file), named by its key's SHA-256, and its body, exactly the bytes that arrived, in a
//! file of its own named by the same and a generation (`<sha256>.<generation>`), which an
//! application may open or share as the image itself ([`Cache::body_path`]). Each is written beside
//! and renamed into place, the body first and the metadata last, the commit point; a replaced
//! entry's body is removed after. So two processes of the same application share the directory and
//! a reader never sees half of an entry; one whose body is missing or short is a miss, and removed.
//!
//! [`Policy`] is how a request uses it, as React Native's `Image` source's `cache` says
//! (`default`, `reload`, `force-cache`, `only-if-cached`).

use crate::{Answer, Failure, Network, Request};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How a request uses the cache, as React Native's `Image` source's `cache`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Policy {
    /// HTTP's rules: a fresh response from the cache, a stale one revalidated, else the network.
    #[default]
    Default,
    /// The network, whatever the cache holds; what it answers is stored.
    Reload,
    /// What the cache holds, however old; else the network.
    ForceCache,
    /// What the cache holds, however old; else the request fails without reaching the network.
    OnlyIfCached,
}

/// Where a response [`Network::fetch_cached`] gave came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Network,
    /// The cache, without asking the network.
    Cache,
    /// The cache, after the server answered that it had not changed (`304`).
    Revalidated,
}

/// A stored response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The URL answered (after redirects).
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// The request's values of the headers the response varies by (`Vary`); none where it had none.
    pub varied: Vec<(String, Option<String>)>,
    /// When the request was sent and its response received.
    pub requested: SystemTime,
    pub received: SystemTime,
    pub body: Vec<u8>,
}

fn header<'a>(headers: &'a [(String, String)], wanted: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
        .map(|(_, value)| value.as_str())
}

/// The directives of `Cache-Control` headers, names in lower case, values unquoted.
fn directives(headers: &[(String, String)]) -> Vec<(String, Option<String>)> {
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
        .flat_map(|(_, value)| value.split(','))
        .filter_map(|directive| {
            let directive = directive.trim();
            if directive.is_empty() {
                return None;
            }
            Some(match directive.split_once('=') {
                Some((name, value)) => (
                    name.trim().to_ascii_lowercase(),
                    Some(value.trim().trim_matches('"').to_string()),
                ),
                None => (directive.to_ascii_lowercase(), None),
            })
        })
        .collect()
}

fn has(directives: &[(String, Option<String>)], wanted: &str) -> bool {
    directives.iter().any(|(name, _)| name == wanted)
}

fn seconds(directives: &[(String, Option<String>)], wanted: &str) -> Option<Duration> {
    directives
        .iter()
        .find(|(name, _)| name == wanted)
        .and_then(|(_, value)| value.as_deref()?.parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// An HTTP date (RFC 9110 §5.6.7: its IMF-fixdate, which every server sends today); none for
/// another form or none at all.
fn date(text: Option<&str>) -> Option<SystemTime> {
    let timestamp = jiff::fmt::rfc2822::DateTimeParser::new()
        .parse_timestamp(text?.trim())
        .ok()?;
    let seconds = u64::try_from(timestamp.as_second()).ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(seconds))
}

/// `later - earlier`, or zero when it is not later.
fn since(later: SystemTime, earlier: SystemTime) -> Duration {
    later.duration_since(earlier).unwrap_or_default()
}

/// The names of the headers a response varies by, in lower case; `["*"]` for everything.
fn vary(headers: &[(String, String)]) -> Vec<String> {
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("vary"))
        .flat_map(|(_, value)| value.split(','))
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Whether a response to a request may be stored (RFC 9111 §3, for a private cache).
pub fn storable(request: &[(String, String)], status: u16, response: &[(String, String)]) -> bool {
    status == 200
        && !has(&directives(request), "no-store")
        && !has(&directives(response), "no-store")
        && !vary(response).iter().any(|name| name == "*")
}

impl Entry {
    /// How long it is fresh for after it was made (RFC 9111 §4.2.1); zero with `no-cache`.
    pub fn lifetime(&self) -> Duration {
        let directives = directives(&self.headers);
        if has(&directives, "no-cache") {
            return Duration::ZERO;
        }
        if let Some(age) = seconds(&directives, "max-age") {
            return age;
        }
        let served = date(header(&self.headers, "date")).unwrap_or(self.received);
        if let Some(expires) = header(&self.headers, "expires") {
            // An Expires that is not a date has already passed (§5.3).
            return date(Some(expires)).map_or(Duration::ZERO, |expires| since(expires, served));
        }
        // OkHttp's heuristic (§4.2.2): a tenth of how long it had not changed, never for a URL
        // with a query, which may name an answer computed afresh.
        match date(header(&self.headers, "last-modified")) {
            Some(modified) if !self.url.contains('?') => since(served, modified) / 10,
            _ => Duration::ZERO,
        }
    }

    /// Its age at `now` (RFC 9111 §4.2.3).
    pub fn age(&self, now: SystemTime) -> Duration {
        let served = date(header(&self.headers, "date")).unwrap_or(self.received);
        let apparent = since(self.received, served);
        let said = header(&self.headers, "age")
            .and_then(|age| age.trim().parse::<u64>().ok())
            .map_or(Duration::ZERO, Duration::from_secs);
        let corrected = said + since(self.received, self.requested);
        apparent.max(corrected) + since(now, self.received)
    }

    /// Whether it may be used without asking the server, at `now`.
    pub fn fresh(&self, now: SystemTime) -> bool {
        self.age(now) < self.lifetime()
    }

    /// The headers that ask the server whether it changed: `If-None-Match` with its ETag,
    /// `If-Modified-Since` with its Last-Modified, as it gave them; none if it has neither.
    pub fn validators(&self) -> Vec<(String, String)> {
        let mut asked = Vec::new();
        if let Some(tag) = header(&self.headers, "etag") {
            asked.push(("If-None-Match".to_string(), tag.to_string()));
        }
        if let Some(modified) = header(&self.headers, "last-modified") {
            asked.push(("If-Modified-Since".to_string(), modified.to_string()));
        }
        asked
    }

    /// Whether a request with `headers` may use it: the headers it varies by have the same values.
    fn serves(&self, headers: &[(String, String)]) -> bool {
        self.varied
            .iter()
            .all(|(name, value)| header(headers, name) == value.as_deref())
    }

    /// Its headers updated by a `304`'s (RFC 9111 §4.3.4), but those that describe its body as
    /// stored (OkHttp keeps them too).
    fn refreshed(
        &self,
        headers: &[(String, String)],
        requested: SystemTime,
        received: SystemTime,
    ) -> Entry {
        const KEPT: [&str; 3] = ["content-length", "content-encoding", "content-type"];
        let mut entry = self.clone();
        for (name, value) in headers {
            let lower = name.to_ascii_lowercase();
            if KEPT.contains(&lower.as_str()) {
                continue;
            }
            match entry
                .headers
                .iter_mut()
                .find(|(stored, _)| stored.eq_ignore_ascii_case(name))
            {
                Some(stored) => stored.1 = value.clone(),
                None => entry.headers.push((name.clone(), value.clone())),
            }
        }
        entry.requested = requested;
        entry.received = received;
        entry
    }

    /// Its metadata stored by `key`, its body in the file `body`: its key, the body's file and
    /// length, then its URL, status, times and headers, a line each.
    fn metadata(&self, key: &str, body: &str) -> Vec<u8> {
        let millis = |time: SystemTime| since(time, UNIX_EPOCH).as_millis();
        let mut text = format!(
            "{MAGIC}\nkey {}\nbody {body} {}\nurl {}\nstatus {}\nrequested {}\nreceived {}\n",
            escape(key),
            self.body.len(),
            self.url,
            self.status,
            millis(self.requested),
            millis(self.received)
        );
        for (name, value) in &self.varied {
            match value {
                Some(value) => text.push_str(&format!("varied {name}: {value}\n")),
                None => text.push_str(&format!("varied {name}\n")),
            }
        }
        for (name, value) in &self.headers {
            text.push_str(&format!("header {name}: {value}\n"));
        }
        text.into_bytes()
    }

    /// An entry's metadata: what it was stored by, its body's file and length, and the entry
    /// without its body. An entry stored without a key was stored by its URL.
    fn decode(bytes: &[u8]) -> Option<Stored> {
        let text = std::str::from_utf8(bytes).ok()?;
        let mut lines = text.lines();
        if lines.next()? != MAGIC {
            return None;
        }
        let time = |millis: &str| Some(UNIX_EPOCH + Duration::from_millis(millis.parse().ok()?));
        let (mut key, mut body) = (None, None);
        let mut entry = Entry {
            url: String::new(),
            status: 0,
            headers: Vec::new(),
            varied: Vec::new(),
            requested: UNIX_EPOCH,
            received: UNIX_EPOCH,
            body: Vec::new(),
        };
        for line in lines {
            let (field, rest) = line.split_once(' ')?;
            match field {
                "key" => key = Some(unescape(rest)?),
                "body" => {
                    let (file, length) = rest.split_once(' ')?;
                    body = Some((file.to_string(), length.parse().ok()?));
                }
                "url" => entry.url = rest.to_string(),
                "status" => entry.status = rest.parse().ok()?,
                "requested" => entry.requested = time(rest)?,
                "received" => entry.received = time(rest)?,
                "varied" => entry.varied.push(match rest.split_once(": ") {
                    Some((name, value)) => (name.to_string(), Some(value.to_string())),
                    None => (rest.to_string(), None),
                }),
                "header" => {
                    let (name, value) = rest.split_once(": ")?;
                    entry.headers.push((name.to_string(), value.to_string()));
                }
                _ => return None,
            }
        }
        let (body, length) = body?;
        if !is_body(&body) {
            return None;
        }
        Some(Stored {
            key: key.unwrap_or_else(|| entry.url.clone()),
            body,
            length,
            entry,
        })
    }
}

/// An entry's metadata as read from its file.
struct Stored {
    /// What it was stored by.
    key: String,
    /// Its body's file name and length.
    body: String,
    length: u64,
    /// The entry, its body not yet read.
    entry: Entry,
}

/// A key on a line of its own: `%`, line feeds and carriage returns escaped as in a URL.
fn escape(key: &str) -> String {
    key.replace('%', "%25")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
}

fn unescape(text: &str) -> Option<String> {
    let mut key = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('%') {
        key.push_str(&rest[..at]);
        key.push(match rest.get(at + 1..at + 3)? {
            "25" => '%',
            "0A" => '\n',
            "0D" => '\r',
            _ => return None,
        });
        rest = &rest[at + 3..];
    }
    key.push_str(rest);
    Some(key)
}

/// The first line of an entry's metadata: its format. Files of another (an entry of the format
/// before, its headers and body in one file) are removed when the cache opens.
const MAGIC: &str = "OXYOS-HTTP-CACHE 2";

/// How old a file written beside the entries, or a body no metadata names, must be before the
/// cache takes it for one a process ended while writing left: another's still writing is younger.
const ABANDONED: Duration = Duration::from_secs(600);

/// An entry the cache holds, by its metadata's file name.
struct Held {
    /// The bytes its files take: its metadata and its body.
    size: u64,
    used: SystemTime,
    /// The files of its body known here: the one its metadata names, and any it replaced that
    /// another process has not removed yet.
    bodies: Vec<String>,
}

/// What the cache holds, by the file name of each entry's metadata.
#[derive(Default)]
struct Index {
    entries: HashMap<String, Held>,
    total: u64,
}

/// A private HTTP cache in a directory of its own.
pub struct Cache {
    directory: PathBuf,
    limit: u64,
    index: Mutex<Index>,
    /// Numbers the files written beside an entry before they take its place, and its bodies.
    written: AtomicU64,
}

/// The file name of an entry's metadata: its key's SHA-256, in hexadecimal.
fn name(key: &str) -> String {
    use std::fmt::Write as _;
    Sha256::digest(key.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut name, byte| {
            let _ = write!(name, "{byte:02x}");
            name
        })
}

fn hex(text: &str, length: usize) -> bool {
    text.len() == length && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Whether a file name is an entry's metadata's.
fn is_metadata(file: &str) -> bool {
    hex(file, 64)
}

/// The entry a body's file belongs to (its metadata's file name), if the name is a body's:
/// `<sha256>.<generation>`, the generation 32 hexadecimal digits.
fn body_of(file: &str) -> Option<&str> {
    let (entry, generation) = file.split_once('.')?;
    (is_metadata(entry) && hex(generation, 32)).then_some(entry)
}

fn is_body(file: &str) -> bool {
    body_of(file).is_some()
}

impl Cache {
    /// The cache in `directory` (made if it is not there), holding at most `limit` bytes.
    pub fn open(directory: &Path, limit: u64) -> io::Result<Cache> {
        fs::create_dir_all(directory)?;
        let mut metadata = HashMap::new();
        let mut bodies: HashMap<String, Vec<(String, u64, PathBuf, SystemTime)>> = HashMap::new();
        for item in fs::read_dir(directory)? {
            let item = item?;
            let file = item.file_name().to_string_lossy().into_owned();
            let Ok(about) = item.metadata() else {
                continue;
            };
            let modified = about.modified().unwrap_or(UNIX_EPOCH);
            if is_metadata(&file) {
                metadata.insert(file, (about.len(), modified, item.path()));
            } else if let Some(entry) = body_of(&file) {
                bodies.entry(entry.to_string()).or_default().push((
                    file.clone(),
                    about.len(),
                    item.path(),
                    modified,
                ));
            } else if file.starts_with(".writing-")
                && since(SystemTime::now(), modified) > ABANDONED
            {
                let _ = fs::remove_file(item.path());
            }
        }
        let mut index = Index::default();
        for (file, (length, used, path)) in metadata {
            // A body is written before the metadata that names it, and an entry's metadata is
            // removed before its body: metadata without one is damaged, or of the format before.
            let Some(files) = bodies.remove(&file) else {
                let _ = fs::remove_file(path);
                continue;
            };
            let size = length + files.iter().map(|(_, length, ..)| length).sum::<u64>();
            index.total += size;
            index.entries.insert(
                file,
                Held {
                    size,
                    used,
                    bodies: files.into_iter().map(|(file, ..)| file).collect(),
                },
            );
        }
        // Bodies no metadata names: what a process ended while storing left.
        for (_, _, path, modified) in bodies.into_values().flatten() {
            if since(SystemTime::now(), modified) > ABANDONED {
                let _ = fs::remove_file(path);
            }
        }
        let cache = Cache {
            directory: directory.to_path_buf(),
            limit,
            index: Mutex::new(index),
            written: AtomicU64::new(0),
        };
        cache.evict(&mut cache.index.lock().expect("lock"));
        Ok(cache)
    }

    /// The metadata stored for `key`, if the cache holds it and it is whole; a damaged one is
    /// removed.
    fn stored(&self, key: &str) -> Option<(String, Stored)> {
        let file = name(key);
        if !self.index.lock().expect("lock").entries.contains_key(&file) {
            return None;
        }
        let path = self.directory.join(&file);
        let Some(stored) = fs::read(&path).ok().and_then(|bytes| Entry::decode(&bytes)) else {
            // Gone (another process evicted it) or damaged: forgotten, and a damaged one removed.
            self.discard(&file);
            return None;
        };
        if let Some(held) = self.index.lock().expect("lock").entries.get_mut(&file) {
            if !held.bodies.contains(&stored.body) {
                held.bodies.push(stored.body.clone());
            }
        }
        (stored.key == key).then_some((file, stored))
    }

    /// The body `stored` names, whole. Missing or short, it is a miss: the entry is removed if
    /// its metadata still names it, else (another process replaced the entry since, and removed
    /// the body it replaced) left as the other made it.
    fn body(&self, file: &str, stored: &Stored) -> Option<Vec<u8>> {
        let read = fs::read(self.directory.join(&stored.body)).ok();
        if let Some(body) = read.filter(|body| body.len() as u64 == stored.length) {
            return Some(body);
        }
        let current = fs::read(self.directory.join(file))
            .ok()
            .and_then(|bytes| Entry::decode(&bytes));
        if current.is_none_or(|current| current.body == stored.body) {
            self.discard(file);
        }
        None
    }

    /// The entry stored for `key` that a request with `headers` may use, if any; it is used now.
    pub fn get(&self, key: &str, headers: &[(String, String)]) -> Option<Entry> {
        let (file, stored) = self.stored(key)?;
        if !stored.entry.serves(headers) {
            return None;
        }
        let body = self.body(&file, &stored)?;
        let now = SystemTime::now();
        if let Some(held) = self.index.lock().expect("lock").entries.get_mut(&file) {
            held.used = now;
        }
        let _ = fs::File::options()
            .write(true)
            .open(self.directory.join(&file))
            .and_then(|opened| opened.set_modified(now));
        Some(Entry {
            body,
            ..stored.entry
        })
    }

    /// The file holding the body of the entry stored for `key`, exactly the bytes that arrived
    /// (expo-image's `getCachePathAsync`), if the cache holds it whole. It stays while the entry
    /// does: until it is replaced, evicted or removed.
    pub fn body_path(&self, key: &str) -> Option<PathBuf> {
        let (file, stored) = self.stored(key)?;
        let path = self.directory.join(&stored.body);
        let whole = fs::metadata(&path).is_ok_and(|about| about.len() == stored.length);
        if !whole {
            self.body(&file, &stored)?;
        }
        Some(path)
    }

    /// Whether an entry is stored for `key`, without reading it.
    pub fn contains(&self, key: &str) -> bool {
        self.index
            .lock()
            .expect("lock")
            .entries
            .contains_key(&name(key))
    }

    /// A new file name beside the entries, unique to this process and this cache.
    fn writing(&self) -> PathBuf {
        self.directory.join(format!(
            ".writing-{}-{}",
            std::process::id(),
            self.written.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// Writes `bytes` beside the entries and renames the file to `path`.
    fn write(&self, bytes: &[u8], path: &Path) -> io::Result<()> {
        let writing = self.writing();
        let written = fs::File::create(&writing)
            .and_then(|mut out| out.write_all(bytes))
            .and_then(|()| fs::rename(&writing, path));
        if written.is_err() {
            let _ = fs::remove_file(&writing);
        }
        written
    }

    /// Stores `entry` by `key` (the URL asked for, which may differ from the one answered after
    /// redirects, or the caller's own key), in place of any before; one larger than a quarter of
    /// the cache is not. Its body is written first, then its metadata, which takes the place of
    /// any before; the body that one named is removed after.
    pub fn put(&self, key: &str, entry: &Entry) -> io::Result<()> {
        let file = name(key);
        // Unique to this process, this write and the time: never another's, even across a
        // restart that gives the process the same id.
        let nanos = since(SystemTime::now(), UNIX_EPOCH).as_nanos() as u64;
        let body = format!(
            "{file}.{:08x}{:08x}{nanos:016x}",
            std::process::id(),
            self.written.fetch_add(1, Ordering::Relaxed) as u32
        );
        let metadata = entry.metadata(key, &body);
        let size = (metadata.len() + entry.body.len()) as u64;
        if size > self.limit / 4 {
            return Ok(());
        }
        self.write(&entry.body, &self.directory.join(&body))?;
        let path = self.directory.join(&file);
        let replaced = fs::read(&path)
            .ok()
            .and_then(|bytes| Entry::decode(&bytes))
            .map(|stored| stored.body);
        if let Err(error) = self.write(&metadata, &path) {
            let _ = fs::remove_file(self.directory.join(&body));
            return Err(error);
        }
        let mut index = self.index.lock().expect("lock");
        let before = index.entries.insert(
            file,
            Held {
                size,
                used: SystemTime::now(),
                bodies: vec![body.clone()],
            },
        );
        let mut old = replaced.into_iter().collect::<Vec<_>>();
        if let Some(before) = before {
            index.total -= before.size;
            old.extend(before.bodies);
        }
        index.total += size;
        for old in old.iter().filter(|old| **old != body) {
            let _ = fs::remove_file(self.directory.join(old));
        }
        self.evict(&mut index);
        Ok(())
    }

    /// Forgets the entry for `key`.
    pub fn remove(&self, key: &str) {
        self.discard(&name(key));
    }

    /// Forgets every entry.
    pub fn clear(&self) {
        let entries: Vec<(String, Held)> = {
            let mut index = self.index.lock().expect("lock");
            index.total = 0;
            index.entries.drain().collect()
        };
        for (file, held) in entries {
            self.delete(&file, &held.bodies);
        }
    }

    /// The bytes the entries take.
    pub fn size(&self) -> u64 {
        self.index.lock().expect("lock").total
    }

    /// Forgets an entry, leaving its files.
    fn forget(&self, file: &str) -> Option<Held> {
        let mut index = self.index.lock().expect("lock");
        let held = index.entries.remove(file)?;
        index.total -= held.size;
        Some(held)
    }

    /// Forgets an entry and removes its files.
    fn discard(&self, file: &str) {
        let bodies = self
            .forget(file)
            .map(|held| held.bodies)
            .unwrap_or_default();
        self.delete(file, &bodies);
    }

    /// Removes an entry's files: its metadata first, so no reader finds it naming a body that is
    /// gone, then its bodies (those known here, and the one its metadata names).
    fn delete(&self, file: &str, bodies: &[String]) {
        let path = self.directory.join(file);
        let named = fs::read(&path)
            .ok()
            .and_then(|bytes| Entry::decode(&bytes))
            .map(|stored| stored.body);
        let _ = fs::remove_file(path);
        for body in bodies.iter().chain(&named) {
            let _ = fs::remove_file(self.directory.join(body));
        }
    }

    /// Removes the entries least recently used until the rest fit.
    fn evict(&self, index: &mut Index) {
        while index.total > self.limit {
            let Some(oldest) = index
                .entries
                .iter()
                .min_by_key(|(_, held)| held.used)
                .map(|(file, _)| file.clone())
            else {
                break;
            };
            let held = index.entries.remove(&oldest).expect("listed");
            index.total -= held.size;
            self.delete(&oldest, &held.bodies);
        }
    }
}

/// How much longer a response with `headers`, received now from `url`, stays fresh (zero for one
/// that must be revalidated): what a cache of decoded images above this one keeps it for.
pub fn freshness(url: &str, headers: &[(String, String)]) -> Duration {
    let now = SystemTime::now();
    let entry = Entry {
        url: url.to_string(),
        status: 200,
        headers: headers.to_vec(),
        varied: Vec::new(),
        requested: now,
        received: now,
        body: Vec::new(),
    };
    entry.lifetime().saturating_sub(entry.age(now))
}

/// A response's status, headers and the URL answered.
type Head = (u16, Vec<(String, String)>, String);

/// What a response being received is, for [`Network::fetch_cached`].
#[derive(Default)]
struct Receiving {
    response: Option<Head>,
    body: Vec<u8>,
    /// Whether the server answered that the stored response has not changed.
    not_modified: bool,
}

/// Gives a stored response as the network would have answered it, its body whole (nothing of it
/// arrives over time, so an incremental request's is not chunks).
fn replay(
    id: i64,
    entry: &Entry,
    answer: &dyn Fn(Answer) -> Result<(), String>,
) -> Result<(), Failure> {
    let failed = |error: String| Failure {
        error,
        timed_out: false,
    };
    answer(Answer::Response {
        id,
        status: entry.status,
        headers: entry.headers.clone(),
        url: entry.url.clone(),
    })
    .map_err(failed)?;
    answer(Answer::Data {
        id,
        bytes: entry.body.clone(),
        content_type: header(&entry.headers, "content-type").map(str::to_string),
    })
    .map_err(failed)
}

impl Network {
    /// Runs a request as [`Network::fetch`] does, through `cache` by `policy`: what it answers is
    /// the same, whether it came from the network, from the cache, or from the cache after the
    /// server said it had not changed (a `304`, which is never answered itself), but that a body
    /// from the cache is answered whole ([`Answer::Data`]) even to an incremental request. Only a GET with
    /// no body and no conditions of its own uses the cache. With no cache, `only-if-cached` fails.
    /// The response is stored by the request's URL.
    pub fn fetch_cached(
        &self,
        request: Request,
        cache: Option<&Cache>,
        policy: Policy,
        answer: &dyn Fn(Answer) -> Result<(), String>,
    ) -> Result<Source, Failure> {
        let key = request.url.clone();
        self.fetch_keyed(request, &key, cache, policy, answer)
    }

    /// As [`Network::fetch_cached`], the response stored by `key` rather than the request's URL
    /// (expo-image's `cacheKey`: one image whose URL changes, a signed one say, cached as one).
    pub fn fetch_keyed(
        &self,
        request: Request,
        key: &str,
        cache: Option<&Cache>,
        policy: Policy,
        answer: &dyn Fn(Answer) -> Result<(), String>,
    ) -> Result<Source, Failure> {
        let not_cached = || Failure {
            error: format!("{} is not in the cache (only-if-cached)", request.url),
            timed_out: false,
        };
        let conditional = [
            "if-none-match",
            "if-modified-since",
            "if-match",
            "if-unmodified-since",
        ]
        .iter()
        .any(|name| header(&request.headers, name).is_some());
        let asked = directives(&request.headers);
        let cache = cache.filter(|_| {
            request.method.eq_ignore_ascii_case("GET")
                && request.body.is_none()
                && !conditional
                && !has(&asked, "no-store")
        });
        let Some(cache) = cache else {
            if policy == Policy::OnlyIfCached {
                return Err(not_cached());
            }
            self.fetch(request, answer)?;
            return Ok(Source::Network);
        };
        let stored = (policy != Policy::Reload)
            .then(|| cache.get(key, &request.headers))
            .flatten();
        if let Some(entry) = &stored {
            let revalidate =
                has(&asked, "no-cache") || seconds(&asked, "max-age") == Some(Duration::ZERO);
            let usable = match policy {
                Policy::ForceCache | Policy::OnlyIfCached => true,
                Policy::Default => !revalidate && entry.fresh(SystemTime::now()),
                Policy::Reload => false,
            };
            if usable {
                replay(request.id, entry, answer)?;
                return Ok(Source::Cache);
            }
        }
        if policy == Policy::OnlyIfCached {
            return Err(not_cached());
        }
        let validators = stored.as_ref().map(Entry::validators).unwrap_or_default();
        let revalidating = !validators.is_empty();
        let id = request.id;
        let mut sent = request.clone();
        sent.headers.extend(validators);
        let receiving = RefCell::new(Receiving::default());
        // What is answered goes to the caller as it comes, and is kept to be stored; a `304`
        // and its empty body stay here.
        let forward = |given: Answer| -> Result<(), String> {
            let mut state = receiving.borrow_mut();
            match &given {
                Answer::Response { status: 304, .. } if revalidating => {
                    state.not_modified = true;
                    if let Answer::Response { headers, url, .. } = given {
                        state.response = Some((304, headers, url));
                    }
                    return Ok(());
                }
                Answer::Response {
                    status,
                    headers,
                    url,
                    ..
                } => {
                    state.response = Some((*status, headers.clone(), url.clone()));
                }
                Answer::Chunk { bytes, .. } | Answer::Data { bytes, .. } => {
                    if state.not_modified {
                        return Ok(());
                    }
                    state.body.extend_from_slice(bytes);
                }
                Answer::Sent { .. } | Answer::Complete { .. } => {}
            }
            drop(state);
            answer(given)
        };
        let requested = SystemTime::now();
        self.fetch(sent, &forward)?;
        let received = SystemTime::now();
        let Receiving {
            response,
            body,
            not_modified,
        } = receiving.into_inner();
        let Some((status, headers, answered)) = response else {
            return Ok(Source::Network);
        };
        if not_modified {
            let entry = stored
                .expect("revalidated")
                .refreshed(&headers, requested, received);
            let _ = cache.put(key, &entry);
            replay(id, &entry, answer)?;
            return Ok(Source::Revalidated);
        }
        if storable(&request.headers, status, &headers) {
            let varied = vary(&headers)
                .into_iter()
                .map(|name| {
                    let value = header(&request.headers, &name).map(str::to_string);
                    (name, value)
                })
                .collect();
            let entry = Entry {
                url: answered,
                status,
                headers,
                varied,
                requested,
                received,
                body,
            };
            let _ = cache.put(key, &entry);
        }
        Ok(Source::Network)
    }
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
