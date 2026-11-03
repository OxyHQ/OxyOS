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
//! quarter of it is not stored. Each entry is a file named by its URL's SHA-256 (its headers as
//! text, then its body), written beside and renamed into place, so two processes of the same
//! application share it and a reader never sees half of one.
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

    fn encode(&self) -> Vec<u8> {
        let millis = |time: SystemTime| since(time, UNIX_EPOCH).as_millis();
        let mut text = format!(
            "{MAGIC}\nurl {}\nstatus {}\nrequested {}\nreceived {}\n",
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
        text.push('\n');
        let mut bytes = text.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Entry> {
        let end = bytes.windows(2).position(|pair| pair == b"\n\n")?;
        let text = std::str::from_utf8(&bytes[..end]).ok()?;
        let mut lines = text.lines();
        if lines.next()? != MAGIC {
            return None;
        }
        let time = |millis: &str| Some(UNIX_EPOCH + Duration::from_millis(millis.parse().ok()?));
        let mut entry = Entry {
            url: String::new(),
            status: 0,
            headers: Vec::new(),
            varied: Vec::new(),
            requested: UNIX_EPOCH,
            received: UNIX_EPOCH,
            body: bytes[end + 2..].to_vec(),
        };
        for line in lines {
            let (key, rest) = line.split_once(' ')?;
            match key {
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
        Some(entry)
    }
}

/// The first line of an entry's file: its format.
const MAGIC: &str = "OXYOS-HTTP-CACHE 1";

/// What the cache holds, by file name: each entry's size and last use.
#[derive(Default)]
struct Index {
    entries: HashMap<String, (u64, SystemTime)>,
    total: u64,
}

/// A private HTTP cache in a directory of its own.
pub struct Cache {
    directory: PathBuf,
    limit: u64,
    index: Mutex<Index>,
    /// Numbers the files written beside an entry before they take its place.
    written: AtomicU64,
}

/// An entry's file name: its URL's SHA-256, in hexadecimal.
fn name(url: &str) -> String {
    use std::fmt::Write as _;
    Sha256::digest(url.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut name, byte| {
            let _ = write!(name, "{byte:02x}");
            name
        })
}

impl Cache {
    /// The cache in `directory` (made if it is not there), holding at most `limit` bytes.
    pub fn open(directory: &Path, limit: u64) -> io::Result<Cache> {
        fs::create_dir_all(directory)?;
        let mut index = Index::default();
        for item in fs::read_dir(directory)? {
            let item = item?;
            let file = item.file_name().to_string_lossy().into_owned();
            let Ok(metadata) = item.metadata() else {
                continue;
            };
            if file.len() == 64 && file.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                let used = metadata.modified().unwrap_or(UNIX_EPOCH);
                index.total += metadata.len();
                index.entries.insert(file, (metadata.len(), used));
            } else if file.starts_with(".writing-") {
                // What a process ended while writing left; another's still writing is younger.
                let old = metadata.modified().is_ok_and(|modified| {
                    since(SystemTime::now(), modified) > Duration::from_secs(600)
                });
                if old {
                    let _ = fs::remove_file(item.path());
                }
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

    /// The entry stored for `url` that a request with `headers` may use, if any; it is used now.
    pub fn get(&self, url: &str, headers: &[(String, String)]) -> Option<Entry> {
        let file = name(url);
        if !self.index.lock().expect("lock").entries.contains_key(&file) {
            return None;
        }
        let path = self.directory.join(&file);
        let Some(entry) = fs::read(&path).ok().and_then(|bytes| Entry::decode(&bytes)) else {
            // Gone (another process evicted it) or damaged: forgotten, and a damaged one removed.
            let _ = fs::remove_file(&path);
            self.forget(&file);
            return None;
        };
        if entry.url != url || !entry.serves(headers) {
            return None;
        }
        let now = SystemTime::now();
        if let Some(used) = self.index.lock().expect("lock").entries.get_mut(&file) {
            used.1 = now;
        }
        let _ = fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|opened| opened.set_modified(now));
        Some(entry)
    }

    /// Whether an entry is stored for `url`, without reading it.
    pub fn contains(&self, url: &str) -> bool {
        self.index
            .lock()
            .expect("lock")
            .entries
            .contains_key(&name(url))
    }

    /// Stores `entry` for `url` (the URL asked for, which may differ from the one answered after
    /// redirects), in place of any before; one larger than a quarter of the cache is not.
    pub fn put(&self, url: &str, entry: &Entry) -> io::Result<()> {
        let bytes = entry.encode();
        let size = bytes.len() as u64;
        if size > self.limit / 4 {
            return Ok(());
        }
        let file = name(url);
        let writing = self.directory.join(format!(
            ".writing-{}-{}",
            std::process::id(),
            self.written.fetch_add(1, Ordering::Relaxed)
        ));
        let written = fs::File::create(&writing)
            .and_then(|mut out| out.write_all(&bytes))
            .and_then(|()| fs::rename(&writing, self.directory.join(&file)));
        if let Err(error) = written {
            let _ = fs::remove_file(&writing);
            return Err(error);
        }
        let mut index = self.index.lock().expect("lock");
        if let Some((before, _)) = index.entries.insert(file, (size, SystemTime::now())) {
            index.total -= before;
        }
        index.total += size;
        self.evict(&mut index);
        Ok(())
    }

    /// Forgets the entry for `url`.
    pub fn remove(&self, url: &str) {
        let file = name(url);
        let _ = fs::remove_file(self.directory.join(&file));
        self.forget(&file);
    }

    /// Forgets every entry.
    pub fn clear(&self) {
        let files: Vec<String> = {
            let mut index = self.index.lock().expect("lock");
            index.total = 0;
            index.entries.drain().map(|(file, _)| file).collect()
        };
        for file in files {
            let _ = fs::remove_file(self.directory.join(file));
        }
    }

    /// The bytes the entries take.
    pub fn size(&self) -> u64 {
        self.index.lock().expect("lock").total
    }

    fn forget(&self, file: &str) {
        let mut index = self.index.lock().expect("lock");
        if let Some((size, _)) = index.entries.remove(file) {
            index.total -= size;
        }
    }

    /// Removes the entries least recently used until the rest fit.
    fn evict(&self, index: &mut Index) {
        while index.total > self.limit {
            let Some(oldest) = index
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(file, _)| file.clone())
            else {
                break;
            };
            let (size, _) = index.entries.remove(&oldest).expect("listed");
            index.total -= size;
            let _ = fs::remove_file(self.directory.join(oldest));
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
    pub fn fetch_cached(
        &self,
        request: Request,
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
            .then(|| cache.get(&request.url, &request.headers))
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
        let (url, id) = (request.url.clone(), request.id);
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
            let _ = cache.put(&url, &entry);
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
            let _ = cache.put(&url, &entry);
        }
        Ok(Source::Network)
    }
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
