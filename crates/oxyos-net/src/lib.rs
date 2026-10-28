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
//! request's answers are dropped: a request cannot be stopped while it waits on the network, but
//! nothing of it is delivered.

use cookie_store::{CookieStore, RawCookie};
use std::collections::{HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::Agent;
use url::Url;

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
    pub body: Option<Vec<u8>>,
    /// The whole request's deadline; none if `None`.
    pub timeout: Option<Duration>,
    /// Whether the process's cookies go with it and what it sets is kept.
    pub with_credentials: bool,
    /// Whether the body is answered as it arrives ([`Answer::Chunk`]) instead of whole.
    pub incremental: bool,
}

/// What a request answered, in order: a response, then chunks or the data, then its end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
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
            Answer::Response { id, .. }
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
    wake: Mutex<UnixStream>,
    cookies: Mutex<CookieStore>,
}

impl Shared {
    /// Queues an answer, unless its request was aborted; its completion is the last one heard.
    fn answer(&self, answer: Answer) {
        let mut heard = self.heard.lock().expect("lock");
        let still = match answer {
            Answer::Complete { id, .. } => heard.remove(&id),
            _ => heard.contains(&answer.id()),
        };
        if !still {
            return;
        }
        self.answers.lock().expect("lock").push_back(answer);
        let _ = self.wake.lock().expect("lock").write_all(&[1]);
    }
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
        let spawned = std::thread::Builder::new()
            .name(format!("oxyos-net-{id}"))
            .spawn(move || {
                let answer = |answer| shared.answer(answer);
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

    /// Delivers nothing more of a request.
    pub fn abort(&self, id: i64) {
        let mut heard = self.shared.heard.lock().expect("lock");
        heard.remove(&id);
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
    answer: &dyn Fn(Answer),
) -> Result<(), ureq::Error> {
    let id = request.id;
    let deadline = request.timeout.map(|timeout| Instant::now() + timeout);
    let mut url =
        Url::parse(&request.url).map_err(|error| ureq::Error::BadUri(error.to_string()))?;
    let mut method = request.method.clone();
    let mut headers = request.headers.clone();
    let mut body = request.body.unwrap_or_default();
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
            let mut builder = ureq::http::Request::builder()
                .method(method.as_str())
                .uri(url.as_str());
            for (name, value) in &headers {
                if cookie.is_none() || !name.eq_ignore_ascii_case("cookie") {
                    builder = builder.header(name.as_str(), value.as_str());
                }
            }
            if let Some(cookie) = &cookie {
                builder = builder.header("cookie", cookie.as_str());
            }
            let http = builder.body(body.clone()).map_err(ureq::Error::from)?;
            let remaining = match deadline {
                Some(deadline) => match deadline.checked_duration_since(Instant::now()) {
                    Some(remaining) if !remaining.is_zero() => Some(remaining),
                    _ => return Err(ureq::Error::Timeout(ureq::Timeout::Global)),
                },
                None => None,
            };
            let http = agent
                .configure_request(http)
                .timeout_global(remaining)
                .build();
            match agent.run(http) {
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
            body.clear();
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
    });
    let mut reader = response
        .into_body()
        .into_with_config()
        .limit(u64::MAX)
        .reader();
    if request.incremental {
        let mut progress = 0;
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
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
            });
        }
    } else {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map_err(ureq::Error::from)?;
        answer(Answer::Data {
            id,
            bytes,
            content_type,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
