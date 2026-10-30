//! HTTP for applications on OxyOS: the engine behind React Native's networking in the Bloom host
//! (docs/react-native-android-adaptation.mdx), through the C ABI (`oxyos-client-ffi`).
//!
//! Each request runs on a thread of its own with ureq: TLS in Rust (rustls, verified against
//! the system's certificates), gzip and brotli, a timeout over the whole request. Redirects are
//! followed here, not by ureq, so that the process's cookie jar keeps what each hop sets and sends
//! each hop its own cookies, as a browser does; the jar holds no public suffix list, so a cookie
//! for a whole public suffix (`Domain=co.uk`) is not refused.
//!
//! What a request answers is queued as [`Answer`]s, which the caller takes from its own thread
//! ([`Network::answer`]); the network's descriptor is readable while any are queued. An aborted
//! request's answers are dropped and it stops at its next step (its body's next part sent, its
//! response's next bytes read): while it waits on the network it cannot be stopped, but nothing of
//! it is delivered. [`Network::fetch`] runs a request on the caller's thread instead, its answers
//! to a function of the caller's (the file service's downloads and uploads).
//!
//! A body is parts ([`Body`]): bytes, or ranges of open files read as the request is sent (a
//! multipart form's files, a file uploaded), never wholly in memory; its length is sent as
//! `Content-Length`, as Android's OkHttp sends a body's whose length it knows. With
//! `upload_progress`, what was sent is answered as it goes ([`Answer::Sent`]).

pub mod connectivity;

use cookie_store::{CookieStore, RawCookie};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Agent, SendBody};
use url::Url;

/// How often a request's progress is answered at most, as Android throttles it.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// A part of a request's body.
#[derive(Clone, Debug)]
pub enum Part {
    Bytes(Arc<[u8]>),
    /// `length` bytes of an open file from `offset`, read as they are sent.
    File {
        file: Arc<std::fs::File>,
        offset: u64,
        length: u64,
    },
}

impl Part {
    pub fn len(&self) -> u64 {
        match self {
            Part::Bytes(bytes) => bytes.len() as u64,
            Part::File { length, .. } => *length,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A request's body: its parts, one after another.
#[derive(Clone, Debug, Default)]
pub struct Body {
    pub parts: Vec<Part>,
}

impl Body {
    pub fn len(&self) -> u64 {
        self.parts.iter().map(Part::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl From<Vec<u8>> for Body {
    fn from(bytes: Vec<u8>) -> Self {
        Body {
            parts: vec![Part::Bytes(bytes.into())],
        }
    }
}

/// The error a cancelled request ends with.
const CANCELLED: &str = "cancelled";

/// A body as it is sent: its parts read in turn, what was sent counted.
struct Sending<'a> {
    parts: &'a [Part],
    part: usize,
    /// How far into the current part.
    at: u64,
    sent: u64,
    total: u64,
    cancel: &'a AtomicBool,
    progress: Option<&'a dyn Fn(u64, u64) -> io::Result<()>>,
    told: Option<Instant>,
}

impl Read for Sending<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other(CANCELLED));
        }
        while let Some(part) = self.parts.get(self.part) {
            let left = part.len() - self.at;
            if left == 0 {
                self.part += 1;
                self.at = 0;
                continue;
            }
            let room = buffer
                .len()
                .min(usize::try_from(left).unwrap_or(usize::MAX));
            let read = match part {
                Part::Bytes(bytes) => {
                    let start = usize::try_from(self.at).unwrap_or(usize::MAX);
                    buffer[..room].copy_from_slice(&bytes[start..start + room]);
                    room
                }
                Part::File { file, offset, .. } => {
                    let read = file.read_at(&mut buffer[..room], offset + self.at)?;
                    if read == 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "a file of the body ended before its length: it changed while sent",
                        ));
                    }
                    read
                }
            };
            self.at += read as u64;
            self.sent += read as u64;
            if let Some(progress) = self.progress {
                let due = self
                    .told
                    .is_none_or(|told| told.elapsed() >= PROGRESS_EVERY);
                if due || self.sent == self.total {
                    self.told = Some(Instant::now());
                    progress(self.sent, self.total)?;
                }
            }
            return Ok(read);
        }
        Ok(0)
    }
}

/// Redirects followed before a request fails, as browsers do.
const MAX_REDIRECTS: usize = 20;
/// Attempts after the first whose connection was lost before any response.
const CONNECTION_RETRIES: usize = 3;

/// Whether a request's connection was lost (as opposed to refused, timed out or misread).
fn lost(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
    )
}

/// A request, as React Native's XMLHttpRequest describes it.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub id: i64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Body>,
    /// The whole request's deadline; none if `None`.
    pub timeout: Option<Duration>,
    /// Whether the process's cookies go with it and what it sets is kept.
    pub with_credentials: bool,
    /// Whether the body is answered as it arrives ([`Answer::Chunk`]) instead of whole.
    pub incremental: bool,
    /// Whether what was sent of the body is answered as it goes ([`Answer::Sent`]).
    pub upload_progress: bool,
    /// Set to stop the request at its next step (it ends with the error "cancelled").
    pub cancel: Arc<AtomicBool>,
}

/// What a request answered, in order: what it sent of its body, a response, then chunks or the
/// data, then its end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// What was sent of the body so far, and its length.
    Sent { id: i64, sent: u64, total: u64 },
    /// The status and headers (duplicates joined with ", "), and the URL answered after
    /// redirects.
    Response {
        id: i64,
        status: u16,
        headers: Vec<(String, String)>,
        url: String,
    },
    /// Part of an incremental body, with the bytes received so far and the total if known.
    Chunk {
        id: i64,
        bytes: Vec<u8>,
        progress: u64,
        total: Option<u64>,
    },
    /// The whole body, and its `Content-Type`.
    Data {
        id: i64,
        bytes: Vec<u8>,
        content_type: Option<String>,
    },
    /// The end: the reason it failed, if it did, and whether that was the timeout.
    Complete {
        id: i64,
        error: Option<String>,
        timed_out: bool,
    },
}

impl Answer {
    pub fn id(&self) -> i64 {
        match self {
            Answer::Sent { id, .. }
            | Answer::Response { id, .. }
            | Answer::Chunk { id, .. }
            | Answer::Data { id, .. }
            | Answer::Complete { id, .. } => *id,
        }
    }
}

/// Answers, the requests still heard and the cookie jar, shared with the requests' threads.
struct Shared {
    answers: Mutex<VecDeque<Answer>>,
    /// Requests started and neither completed nor aborted: only their answers are queued. Locked
    /// before `answers`.
    heard: Mutex<HashSet<i64>>,
    /// Each request started and not ended: what stops it.
    cancels: Mutex<HashMap<i64, Arc<AtomicBool>>>,
    wake: Mutex<UnixStream>,
    cookies: Mutex<CookieStore>,
}

impl Shared {
    /// Queues an answer, unless its request was aborted; its completion is the last one heard.
    fn answer(&self, answer: Answer) {
        let mut heard = self.heard.lock().expect("lock");
        let still = match answer {
            Answer::Complete { id, .. } => {
                self.cancels.lock().expect("lock").remove(&id);
                heard.remove(&id)
            }
            _ => heard.contains(&answer.id()),
        };
        if !still {
            return;
        }
        self.answers.lock().expect("lock").push_back(answer);
        let _ = self.wake.lock().expect("lock").write_all(&[1]);
    }
}

/// Why a request [`Network::fetch`] ran failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub error: String,
    /// Whether that was its timeout.
    pub timed_out: bool,
}

pub struct Network {
    agent: Agent,
    shared: Arc<Shared>,
    readable: UnixStream,
}

fn agent() -> Agent {
    Agent::config_builder()
        // A status is an answer, not an error: XMLHttpRequest reports it.
        .http_status_as_error(false)
        // Redirects are [`run`]'s: it keeps and sends each hop's cookies.
        .max_redirects(0)
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}

impl Network {
    pub fn new() -> io::Result<Self> {
        let (readable, wake) = UnixStream::pair()?;
        readable.set_nonblocking(true)?;
        wake.set_nonblocking(true)?;
        Ok(Self {
            agent: agent(),
            shared: Arc::new(Shared {
                answers: Mutex::new(VecDeque::new()),
                heard: Mutex::new(HashSet::new()),
                cancels: Mutex::new(HashMap::new()),
                wake: Mutex::new(wake),
                cookies: Mutex::new(CookieStore::default()),
            }),
            readable,
        })
    }

    /// Starts a request on a thread of its own.
    pub fn send(&self, request: Request) {
        let agent = self.agent.clone();
        let shared = self.shared.clone();
        let id = request.id;
        self.shared.heard.lock().expect("lock").insert(id);
        self.shared
            .cancels
            .lock()
            .expect("lock")
            .insert(id, request.cancel.clone());
        let spawned = std::thread::Builder::new()
            .name(format!("oxyos-net-{id}"))
            .spawn(move || {
                let answer = |answer| {
                    shared.answer(answer);
                    Ok(())
                };
                match run(&agent, &shared.cookies, request, &answer) {
                    Ok(()) => answer(Answer::Complete {
                        id,
                        error: None,
                        timed_out: false,
                    }),
                    Err(error) => answer(Answer::Complete {
                        id,
                        timed_out: matches!(error, ureq::Error::Timeout(_)),
                        error: Some(error.to_string()),
                    }),
                }
            });
        if let Err(error) = spawned {
            self.shared.answer(Answer::Complete {
                id,
                error: Some(format!("the request cannot start: {error}")),
                timed_out: false,
            });
        }
    }

    /// Runs a request on the caller's thread, to its end, giving `answer` what it answers as it
    /// goes (never its completion: what this returns). An error `answer` returns stops it, with
    /// that reason; so does its `cancel` set, from another thread. Its cookies are the network's.
    pub fn fetch(
        &self,
        request: Request,
        answer: &dyn Fn(Answer) -> Result<(), String>,
    ) -> Result<(), Failure> {
        let answer = |given| answer(given).map_err(io::Error::other);
        run(&self.agent, &self.shared.cookies, request, &answer).map_err(|error| Failure {
            timed_out: matches!(error, ureq::Error::Timeout(_)),
            error: error.to_string(),
        })
    }

    /// Delivers nothing more of a request, and stops it at its next step.
    pub fn abort(&self, id: i64) {
        let mut heard = self.shared.heard.lock().expect("lock");
        heard.remove(&id);
        if let Some(cancel) = self.shared.cancels.lock().expect("lock").remove(&id) {
            cancel.store(true, Ordering::Relaxed);
        }
        self.shared
            .answers
            .lock()
            .expect("lock")
            .retain(|answer| answer.id() != id);
    }

    /// Forgets every cookie.
    pub fn clear_cookies(&self) {
        self.shared.cookies.lock().expect("lock").clear();
    }

    /// The next answer, if any; never blocks.
    pub fn answer(&self) -> Option<Answer> {
        let mut answers = self.shared.answers.lock().expect("lock");
        let answer = answers.pop_front();
        if answers.is_empty() {
            let mut drained = [0u8; 64];
            while matches!((&self.readable).read(&mut drained), Ok(n) if n > 0) {}
        }
        answer
    }
}

impl AsFd for Network {
    /// Readable while answers are queued.
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.readable.as_fd()
    }
}

/// Runs a request to its end, answering as it goes; the end itself is the caller's to answer.
fn run(
    agent: &Agent,
    cookies: &Mutex<CookieStore>,
    request: Request,
    answer: &dyn Fn(Answer) -> io::Result<()>,
) -> Result<(), ureq::Error> {
    let id = request.id;
    let cancel = request.cancel.clone();
    let cancelled = || -> Result<(), ureq::Error> {
        if cancel.load(Ordering::Relaxed) {
            return Err(ureq::Error::Io(io::Error::other(CANCELLED)));
        }
        Ok(())
    };
    let deadline = request.timeout.map(|timeout| Instant::now() + timeout);
    let mut url =
        Url::parse(&request.url).map_err(|error| ureq::Error::BadUri(error.to_string()))?;
    let mut method = request.method.clone();
    let mut headers = request.headers.clone();
    let mut body = request.body.clone();
    // What was sent of the body, as it goes (each attempt sends it from its start).
    let sent = |sent: u64, total: u64| answer(Answer::Sent { id, sent, total });
    let progress: Option<&dyn Fn(u64, u64) -> io::Result<()>> =
        request.upload_progress.then_some(&sent);
    let mut redirects = 0;
    let response = loop {
        // The jar's cookies stand in for any the request set, as on Android (OkHttp).
        let cookie = if request.with_credentials {
            let jar = cookies.lock().expect("lock");
            let values: Vec<String> = jar
                .get_request_values(&url)
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            (!values.is_empty()).then(|| values.join("; "))
        } else {
            None
        };
        let mut retries = 0;
        let response = loop {
            cancelled()?;
            let mut builder = ureq::http::Request::builder()
                .method(method.as_str())
                .uri(url.as_str());
            for (name, value) in &headers {
                let length = body.is_some() && name.eq_ignore_ascii_case("content-length");
                if (cookie.is_none() || !name.eq_ignore_ascii_case("cookie")) && !length {
                    builder = builder.header(name.as_str(), value.as_str());
                }
            }
            if let Some(cookie) = &cookie {
                builder = builder.header("cookie", cookie.as_str());
            }
            let remaining = match deadline {
                Some(deadline) => match deadline.checked_duration_since(Instant::now()) {
                    Some(remaining) if !remaining.is_zero() => Some(remaining),
                    _ => return Err(ureq::Error::Timeout(ureq::Timeout::Global)),
                },
                None => None,
            };
            let answered = match &body {
                // The body is read from its parts as it is sent, its length said first.
                Some(body) => {
                    let total = body.len();
                    let mut sending = Sending {
                        parts: &body.parts,
                        part: 0,
                        at: 0,
                        sent: 0,
                        total,
                        cancel: &cancel,
                        progress,
                        told: None,
                    };
                    let http = builder
                        .header("content-length", total)
                        .body(SendBody::from_reader(&mut sending))
                        .map_err(ureq::Error::from)?;
                    agent.run(
                        agent
                            .configure_request(http)
                            .timeout_global(remaining)
                            .build(),
                    )
                }
                None => {
                    let http = builder.body(Vec::new()).map_err(ureq::Error::from)?;
                    agent.run(
                        agent
                            .configure_request(http)
                            .timeout_global(remaining)
                            .build(),
                    )
                }
            };
            match answered {
                // A connection lost before any response, most often a kept-alive one the server
                // had closed, is tried again, as Android does (OkHttp's retryOnConnectionFailure).
                Err(ureq::Error::Io(error)) if retries < CONNECTION_RETRIES && lost(&error) => {
                    retries += 1;
                }
                answered => break answered?,
            }
        };
        if request.with_credentials {
            let set = response
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|value| value.to_str().ok())
                .filter_map(|value| RawCookie::parse(value.to_string()).ok());
            cookies
                .lock()
                .expect("lock")
                .store_response_cookies(set, &url);
        }
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok());
        let next = match (status, location) {
            (301 | 302 | 303 | 307 | 308, Some(location)) => location,
            // A redirect without a place to go is the answer.
            _ => break response,
        };
        if redirects == MAX_REDIRECTS {
            return Err(ureq::Error::TooManyRedirects);
        }
        redirects += 1;
        let next = url
            .join(next)
            .map_err(|error| ureq::Error::BadUri(format!("redirect to {next}: {error}")))?;
        if !matches!(next.scheme(), "http" | "https") {
            return Err(ureq::Error::BadUri(format!("redirect to {next}")));
        }
        // As the Fetch standard: 303, and 301 or 302 after a POST, go on as a GET without a body.
        let as_get = (status == 303 && method != "HEAD")
            || (matches!(status, 301 | 302) && method == "POST");
        if as_get {
            method = "GET".to_string();
            body = None;
            headers.retain(|(name, _)| {
                ![
                    "content-type",
                    "content-length",
                    "content-encoding",
                    "content-language",
                    "content-location",
                ]
                .iter()
                .any(|dropped| name.eq_ignore_ascii_case(dropped))
            });
        }
        // Credentials the request carried are not handed to another origin.
        if next.origin() != url.origin() {
            headers.retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
        }
        url = next;
    };
    let url = url.to_string();
    let status = response.status().as_u16();
    let mut headers: Vec<(String, String)> = Vec::new();
    for (name, value) in response.headers() {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        match headers.iter_mut().find(|(seen, _)| seen == name.as_str()) {
            Some((_, joined)) => {
                joined.push_str(", ");
                joined.push_str(&value);
            }
            None => headers.push((name.as_str().to_string(), value)),
        }
    }
    let header = |wanted: &str| {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value.clone())
    };
    let content_type = header("content-type");
    let total = header("content-length").and_then(|length| length.parse().ok());
    answer(Answer::Response {
        id,
        status,
        headers: headers.clone(),
        url,
    })?;
    let mut reader = response
        .into_body()
        .into_with_config()
        .limit(u64::MAX)
        .reader();
    if request.incremental {
        let mut progress = 0;
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            cancelled()?;
            let read = reader.read(&mut buffer).map_err(ureq::Error::from)?;
            if read == 0 {
                break;
            }
            progress += read as u64;
            answer(Answer::Chunk {
                id,
                bytes: buffer[..read].to_vec(),
                progress,
                total,
            })?;
        }
    } else {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map_err(ureq::Error::from)?;
        cancelled()?;
        answer(Answer::Data {
            id,
            bytes,
            content_type,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
