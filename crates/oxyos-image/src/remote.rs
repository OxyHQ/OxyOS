//! Remote images: what React Native's Android `Image` loads through Fresco and OkHttp, for the
//! Bloom host (docs/react-native-android-adaptation.mdx).
//!
//! An image is fetched with the application's own network (`oxyos-net`: its TLS, its proxy, its
//! cookies, which the Bloom host's `fetch` shares), through its HTTP cache on disk
//! ([`oxyos_net::cache`], in the application's cache directory) by the source's `cache` policy,
//! the headers the source gives sent with it. What arrives from the network is answered as it
//! arrives ([`Outcome::Progress`], when its length is known; none for a cache's), then decoded on the fetch's thread at the
//! size each load wants it, never on the caller's.
//!
//! Above the disk cache, the encoded images most recently fetched are kept in memory (bounded,
//! least recently used first), as Fresco's encoded memory cache: a load of one there does not
//! reach the network or the disk while its response is fresh by HTTP's rules (`default`), or
//! however old (`force-cache`, `only-if-cached`). Each load says which caches it uses, and a
//! prefetch fills those it uses as any load does: React Native's `prefetchImage` (Fresco's
//! `prefetchToDiskCache`) uses the disk cache alone, expo-image's `prefetch` with `memory-disk`
//! both. Both hold an image by its cache key, its URL unless the load gives another
//! (expo-image's `cacheKey`). What is answered says where its bytes came from ([`Origin`]).
//!
//! Loads of the same image with the same request at once share one fetch, as Fresco's do; a
//! load cancelled answers nothing more, and the fetch stops at its next step once no load waits
//! for it.

use crate::{outcome, Error, Fit, Origin, Outcome, Shared};
use oxyos_net::cache::{self, Cache, Policy, Source};
use oxyos_net::{Answer, Network, Request};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What a remote image's load wants of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    /// Its pixels, at the size `Fit` asks for.
    Picture(Fit),
    /// Its size, from its headers (`Image.getSize`).
    Size,
    /// It fetched into the caches, nothing more (`Image.prefetch`).
    Prefetch,
}

/// A remote image's load, as React Native's `Image` source describes it.
#[derive(Clone, Debug)]
pub struct Fetch {
    pub url: String,
    /// Sent with the request (the source's `headers`).
    pub headers: Vec<(String, String)>,
    /// The source's `cache`.
    pub policy: Policy,
    /// Whether the memory cache and the disk cache are used, read and written (expo-image's
    /// `cachePolicy`: both for `memory-disk`, neither for `none`; React Native's prefetch, the
    /// disk alone).
    pub memory: bool,
    pub disk: bool,
    pub want: Want,
    /// What both caches hold it by (expo-image's `cacheKey`); none: its URL.
    pub key: Option<String>,
    /// Whether an animated GIF or WebP is decoded frame by frame, else its first frame.
    pub animated: bool,
}

impl Fetch {
    /// What both caches hold it by.
    pub fn cache_key(&self) -> &str {
        self.key.as_deref().unwrap_or(&self.url)
    }
}

/// The most of a remote image's body fetched.
pub const MAX_BODY: usize = 64 << 20;
/// The encoded images kept in memory, at most, in bytes.
const MEMORY_LIMIT: usize = 32 << 20;
/// How often a fetch's progress is answered at most, as `oxyos-net` answers a request's.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// An encoded image kept in memory.
struct Kept {
    bytes: Arc<[u8]>,
    content_type: Option<String>,
    /// Until when HTTP's rules let it be used without asking (its response's freshness).
    fresh_until: Instant,
    used: u64,
}

#[derive(Default)]
struct Memory {
    kept: HashMap<String, Kept>,
    total: usize,
    clock: u64,
}

impl Memory {
    fn get(&mut self, url: &str, policy: Policy) -> Option<(Arc<[u8]>, Option<String>)> {
        self.clock += 1;
        let clock = self.clock;
        let kept = self.kept.get_mut(url)?;
        let usable = match policy {
            Policy::Default => Instant::now() < kept.fresh_until,
            Policy::ForceCache | Policy::OnlyIfCached => true,
            Policy::Reload => false,
        };
        if !usable {
            return None;
        }
        kept.used = clock;
        Some((kept.bytes.clone(), kept.content_type.clone()))
    }

    fn put(&mut self, url: &str, kept: Kept) {
        if kept.bytes.len() > MEMORY_LIMIT / 4 {
            return;
        }
        self.clock += 1;
        let size = kept.bytes.len();
        if let Some(before) = self.kept.insert(
            url.to_string(),
            Kept {
                used: self.clock,
                ..kept
            },
        ) {
            self.total -= before.bytes.len();
        }
        self.total += size;
        while self.total > MEMORY_LIMIT {
            let Some(oldest) = self
                .kept
                .iter()
                .min_by_key(|(_, kept)| kept.used)
                .map(|(url, _)| url.clone())
            else {
                break;
            };
            let gone = self.kept.remove(&oldest).expect("listed");
            self.total -= gone.bytes.len();
        }
    }
}

/// A fetch on its way, and the loads waiting for it: each one's id, what it wants, and whether
/// an animated image is decoded frame by frame for it.
struct Flight {
    waiting: Vec<(u64, Want, bool)>,
    cancel: Arc<AtomicBool>,
    /// When its progress was last answered.
    told: Option<Instant>,
}

pub(crate) struct Remote {
    network: Arc<Network>,
    cache: Option<Arc<Cache>>,
    memory: Mutex<Memory>,
    /// By the request they make (its URL, headers, cache key and how it uses the caches).
    flights: Mutex<HashMap<String, Flight>>,
}

impl Remote {
    pub(crate) fn new(network: Arc<Network>, cache: Option<Arc<Cache>>) -> Self {
        Self {
            network,
            cache,
            memory: Mutex::default(),
            flights: Mutex::default(),
        }
    }

    pub(crate) fn cached(&self, key: &str) -> (bool, bool) {
        let memory = self.memory.lock().expect("lock").kept.contains_key(key);
        let disk = self.cache.as_ref().is_some_and(|cache| cache.contains(key));
        (memory, disk)
    }

    pub(crate) fn clear(&self, memory: bool, disk: bool) {
        if memory {
            *self.memory.lock().expect("lock") = Memory::default();
        }
        if let Some(cache) = self.cache.as_ref().filter(|_| disk) {
            cache.clear();
        }
    }

    pub(crate) fn cache_path(&self, key: &str) -> Option<std::path::PathBuf> {
        self.cache.as_ref()?.body_path(key)
    }

    pub(crate) fn cancel(&self, id: u64) {
        let mut flights = self.flights.lock().expect("lock");
        flights.retain(|_, flight| {
            flight.waiting.retain(|(waiting, ..)| *waiting != id);
            if flight.waiting.is_empty() {
                flight.cancel.store(true, Ordering::Relaxed);
            }
            !flight.waiting.is_empty()
        });
    }

    pub(crate) fn fetch(remote: &Arc<Remote>, shared: &Arc<Shared>, id: u64, fetch: Fetch) {
        let memory = fetch.memory;
        if memory {
            let held = remote
                .memory
                .lock()
                .expect("lock")
                .get(fetch.cache_key(), fetch.policy);
            if let Some((bytes, content_type)) = held {
                let shared = shared.clone();
                let (want, animated) = (fetch.want, fetch.animated);
                // Decoded off the caller's thread, as a fetched one.
                let spawned = std::thread::Builder::new()
                    .name(format!("oxyos-image-{id}"))
                    .spawn(move || {
                        let content_type = content_type.as_deref();
                        let answered =
                            outcome(&bytes, content_type, want, animated, Origin::Memory);
                        shared.answer(id, answered)
                    });
                if spawned.is_ok() {
                    return;
                }
            }
        }
        let key = format!(
            "{:?} {memory} {} {}\n{}\n{:?}",
            fetch.policy,
            fetch.disk,
            fetch.url,
            fetch.cache_key(),
            fetch.headers
        );
        let cancel = {
            let mut flights = remote.flights.lock().expect("lock");
            if let Some(flight) = flights.get_mut(&key) {
                flight.waiting.push((id, fetch.want, fetch.animated));
                return;
            }
            let cancel = Arc::new(AtomicBool::new(false));
            flights.insert(
                key.clone(),
                Flight {
                    waiting: vec![(id, fetch.want, fetch.animated)],
                    cancel: cancel.clone(),
                    told: None,
                },
            );
            cancel
        };
        let (remote, shared) = (remote.clone(), shared.clone());
        let spawned = std::thread::Builder::new()
            .name(format!("oxyos-image-fetch-{id}"))
            .spawn({
                let (remote, shared, key) = (remote.clone(), shared.clone(), key.clone());
                move || remote.run(&shared, &key, fetch, memory, cancel)
            });
        if let Err(error) = spawned {
            remote.flights.lock().expect("lock").remove(&key);
            shared.answer(
                id,
                Outcome::Failed(Error::Fetch(format!("the fetch cannot start: {error}"))),
            );
        }
    }

    /// Runs a flight to its end, on its own thread.
    fn run(&self, shared: &Shared, key: &str, fetch: Fetch, memory: bool, cancel: Arc<AtomicBool>) {
        let request = Request {
            id: 0,
            method: "GET".into(),
            url: fetch.url.clone(),
            headers: fetch.headers.clone(),
            with_credentials: true,
            incremental: true,
            cancel: cancel.clone(),
            ..Request::default()
        };
        let state = Mutex::new((0u16, Vec::<(String, String)>::new(), Vec::<u8>::new()));
        let fetched = self.network.fetch_keyed(
            request,
            fetch.cache_key(),
            self.cache.as_deref().filter(|_| fetch.disk),
            fetch.policy,
            &|answer| {
                let mut state = state.lock().expect("lock");
                match answer {
                    Answer::Response {
                        status, headers, ..
                    } => {
                        state.0 = status;
                        state.1 = headers;
                    }
                    Answer::Chunk {
                        bytes,
                        progress,
                        total,
                        ..
                    } => {
                        if state.2.len() + bytes.len() > MAX_BODY {
                            return Err(format!("the image is larger than {} MiB", MAX_BODY >> 20));
                        }
                        state.2.extend_from_slice(&bytes);
                        // A successful response's progress, when its length is known.
                        if let (200, Some(total)) = (state.0, total) {
                            self.progress(shared, key, progress, total);
                        }
                    }
                    // From a cache, whole: nothing was downloaded, so no progress.
                    Answer::Data { bytes, .. } => {
                        if state.2.len() + bytes.len() > MAX_BODY {
                            return Err(format!("the image is larger than {} MiB", MAX_BODY >> 20));
                        }
                        state.2.extend_from_slice(&bytes);
                    }
                    _ => {}
                }
                Ok(())
            },
        );
        let waiting = self
            .flights
            .lock()
            .expect("lock")
            .remove(key)
            .map(|flight| flight.waiting)
            .unwrap_or_default();
        if cancel.load(Ordering::Relaxed) || waiting.is_empty() {
            return;
        }
        let (status, headers, body) = state.into_inner().expect("lock");
        let failed = |error: String| {
            for (id, ..) in &waiting {
                shared.answer(*id, Outcome::Failed(Error::Fetch(error.clone())));
            }
        };
        let from = match fetched {
            Err(failure) => return failed(failure.error),
            Ok(Source::Network) => Origin::Source,
            Ok(Source::Cache | Source::Revalidated) => Origin::Disk,
        };
        if status != 200 {
            return failed(format!("{} answered {status}", fetch.url));
        }
        let header = |wanted: &str| {
            headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
                .map(|(_, value)| value.clone())
        };
        let content_type = header("content-type");
        let bytes: Arc<[u8]> = body.into();
        if memory {
            let fresh = cache::freshness(&fetch.url, &headers);
            self.memory.lock().expect("lock").put(
                fetch.cache_key(),
                Kept {
                    bytes: bytes.clone(),
                    content_type: content_type.clone(),
                    fresh_until: Instant::now() + fresh,
                    used: 0,
                },
            );
        }
        for (id, want, animated) in waiting {
            let answered = outcome(&bytes, content_type.as_deref(), want, animated, from);
            shared.answer(id, answered);
        }
    }

    /// Answers a flight's progress to the loads waiting for it, at most every
    /// [`PROGRESS_EVERY`] and when it is complete.
    fn progress(&self, shared: &Shared, key: &str, loaded: u64, total: u64) {
        let mut flights = self.flights.lock().expect("lock");
        let Some(flight) = flights.get_mut(key) else {
            return;
        };
        let due = flight
            .told
            .is_none_or(|told| told.elapsed() >= PROGRESS_EVERY);
        if !due && loaded < total {
            return;
        }
        flight.told = Some(Instant::now());
        for (id, ..) in &flight.waiting {
            shared.answer(*id, Outcome::Progress { loaded, total });
        }
    }
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;
