//! Fetching the store's files (docs/adr/0030-installing-applications.mdx, decision 5).
//!
//! The install service has no network. What it needs from the repository it asks of the
//! fetcher, `oxyos-installd --fetch`, a service of its own run as a dynamic user with the network
//! and nothing else: the install service opens each destination file itself, in its own staging
//! directory, and hands the fetcher only that descriptor with the file's path in the repository
//! and the most it may be. The fetcher can write those bytes and reach nothing of the system's;
//! the install service then checks what was written against the verified metadata before the
//! file goes anywhere.
//!
//! - **The fetcher's socket**, `/run/oxyos/app-fetch/socket` (its own runtime directory, the dynamic
//!   user's): `SOCK_SEQPACKET`, one JSON message
//!   per packet, the descriptor with the request (`SCM_RIGHTS`):
//!
//!   ```text
//!   → {"fetch": {"path", "limit"}}  with the destination's descriptor
//!   ← {"fetched": {"length"}} | "notFound" | "tooLarge" | {"failed": {"reason"}}
//!   ```
//!
//!   Only root (the install service) may ask; the connection serves requests until it closes.
//! - **Paths** are relative, of `/`-separated components of ASCII letters, digits, `.`, `_`, `-`
//!   and `+`, none empty, `.` or `..`.
//! - **The repository** is a local directory (`file:///…`) in this step, laid out as the store
//!   serves it: `metadata/` (TUF's), `targets/`, `objects/<aa>/<sha256>`. HTTPS to the store's
//!   CDN, through `oxyos-net`'s client, is a later step; the install service's side does not
//!   change with it.

use crate::socket::{Connection, Peer};
use rustix::fd::{AsFd, BorrowedFd};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the fetcher listens.
pub const FETCH_SOCKET: &str = "/run/oxyos/app-fetch/socket";

/// How long a fetch may take, or a connection wait.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    NotFound,
    TooLarge,
    Failed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::NotFound => f.write_str("not in the repository"),
            FetchError::TooLarge => f.write_str("larger than it may be"),
            FetchError::Failed(reason) => f.write_str(reason),
        }
    }
}

/// What reads the repository's files.
pub trait Fetch {
    /// Writes the repository's file `path` into `out` (empty, written from its start), at most
    /// `limit` bytes: its length.
    fn fetch(&self, path: &str, limit: u64, out: BorrowedFd) -> Result<u64, FetchError>;

    /// The repository's file `path` in memory, at most `limit` bytes (metadata, a manifest).
    fn fetch_to_vec(&self, path: &str, limit: u64) -> Result<Vec<u8>, FetchError> {
        let memory = rustix::fs::memfd_create("oxyos-fetch", rustix::fs::MemfdFlags::CLOEXEC)
            .map_err(|e| FetchError::Failed(e.to_string()))?;
        let length = self.fetch(path, limit, memory.as_fd())?;
        let mut file = File::from(memory);
        file.rewind()
            .map_err(|e| FetchError::Failed(e.to_string()))?;
        let mut bytes = Vec::with_capacity(length.min(limit) as usize);
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| FetchError::Failed(e.to_string()))?;
        if bytes.len() as u64 > limit {
            return Err(FetchError::TooLarge);
        }
        Ok(bytes)
    }
}

/// Whether `path` is a path of the repository.
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-+".contains(&c))
        })
}

/// A repository in a local directory.
#[derive(Clone, Debug)]
pub struct LocalRepository {
    root: PathBuf,
}

impl LocalRepository {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// The repository a URL names: `file:///absolute/directory` alone, in this step.
    pub fn from_url(url: &str) -> Result<Self, String> {
        match url.strip_prefix("file://") {
            Some(path) if path.starts_with('/') => Ok(Self::new(Path::new(path))),
            _ => Err(format!(
                "{url}: only a file:/// repository is read in this step (HTTPS comes later)"
            )),
        }
    }
}

impl Fetch for LocalRepository {
    fn fetch(&self, path: &str, limit: u64, out: BorrowedFd) -> Result<u64, FetchError> {
        use std::os::unix::fs::OpenOptionsExt;
        if !valid_path(path) {
            return Err(FetchError::Failed(format!(
                "{path:?} is not a repository path"
            )));
        }
        let mut file = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(self.root.join(path))
        {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(FetchError::NotFound),
            Err(e) => return Err(FetchError::Failed(format!("{path}: {e}"))),
        };
        let mut out = File::from(
            out.try_clone_to_owned()
                .map_err(|e| FetchError::Failed(e.to_string()))?,
        );
        let mut buffer = vec![0; 64 << 10];
        let mut total: u64 = 0;
        loop {
            let n = match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(FetchError::Failed(format!("{path}: {e}"))),
            };
            total += n as u64;
            if total > limit {
                return Err(FetchError::TooLarge);
            }
            out.write_all(&buffer[..n])
                .map_err(|e| FetchError::Failed(format!("{path}: {e}")))?;
        }
        Ok(total)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Request {
    Fetch { path: String, limit: u64 },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Answer {
    Fetched { length: u64 },
    NotFound,
    TooLarge,
    Failed { reason: String },
}

/// The fetcher: serves one connection's requests from `repository` until it closes, if
/// `allowed` lets its peer ask (the install service's user, root). A peer that may not ask is
/// answered `failed` once, after its first message is read.
pub fn serve(
    connection: &Connection,
    repository: &dyn Fetch,
    allowed: impl Fn(&Peer) -> bool,
) -> io::Result<()> {
    let peer = connection.peer()?;
    connection.set_timeout(FETCH_TIMEOUT)?;
    loop {
        let Some((request, fd)) = connection.receive_with_fd::<Request>()? else {
            return Ok(());
        };
        let answer = if !allowed(&peer) {
            Answer::Failed {
                reason: "not permitted".into(),
            }
        } else {
            match (request, fd) {
                (Some(Request::Fetch { path, limit }), Some(fd)) => {
                    match repository.fetch(&path, limit, fd.as_fd()) {
                        Ok(length) => Answer::Fetched { length },
                        Err(FetchError::NotFound) => Answer::NotFound,
                        Err(FetchError::TooLarge) => Answer::TooLarge,
                        Err(FetchError::Failed(reason)) => Answer::Failed { reason },
                    }
                }
                _ => Answer::Failed {
                    reason: "not a request with its destination".into(),
                },
            }
        };
        let refused = !allowed(&peer);
        connection.send(&answer)?;
        if refused {
            return Ok(());
        }
    }
}

/// The install service's side: asks the fetcher at `socket`.
#[derive(Clone, Debug)]
pub struct FetcherClient {
    socket: PathBuf,
}

impl FetcherClient {
    pub fn new(socket: &Path) -> Self {
        Self {
            socket: socket.to_path_buf(),
        }
    }
}

impl Fetch for FetcherClient {
    fn fetch(&self, path: &str, limit: u64, out: BorrowedFd) -> Result<u64, FetchError> {
        let failed = |e: io::Error| FetchError::Failed(format!("the fetcher: {e}"));
        let connection = Connection::connect(&self.socket).map_err(failed)?;
        connection.set_timeout(FETCH_TIMEOUT).map_err(failed)?;
        connection
            .send_with_fd(
                &Request::Fetch {
                    path: path.to_string(),
                    limit,
                },
                out,
            )
            .map_err(failed)?;
        let answer = connection.receive::<Answer>().map_err(failed)?;
        let written = rustix::fs::fstat(out)
            .map_err(|e| FetchError::Failed(e.to_string()))?
            .st_size as u64;
        match answer {
            Some(Some(Answer::Fetched { length })) if length == written && length <= limit => {
                Ok(length)
            }
            Some(Some(Answer::Fetched { .. })) => Err(FetchError::Failed(
                "the fetcher said another length than it wrote".into(),
            )),
            Some(Some(Answer::NotFound)) => Err(FetchError::NotFound),
            Some(Some(Answer::TooLarge)) => Err(FetchError::TooLarge),
            Some(Some(Answer::Failed { reason })) => Err(FetchError::Failed(reason)),
            _ => Err(FetchError::Failed("the fetcher did not answer".into())),
        }
    }
}

/// A fetcher served on a thread from `repository`, over a socket pair: what the tests use, so
/// they pass descriptors as the service does.
pub struct ThreadFetcher {
    socket: PathBuf,
    _thread: std::thread::JoinHandle<()>,
}

impl ThreadFetcher {
    /// Serves `repository` at a socket in `directory`, for this process's own user.
    pub fn start(directory: &Path, repository: LocalRepository) -> io::Result<Self> {
        let socket = directory.join("fetch.socket");
        let listener = crate::socket::Listener::bind(&socket, 0o600)?;
        let uid = rustix::process::getuid().as_raw();
        let thread = std::thread::spawn(move || {
            while let Ok(connection) = listener.accept() {
                let _ = serve(&connection, &repository, |peer| peer.uid == uid);
            }
        });
        Ok(Self {
            socket,
            _thread: thread,
        })
    }

    pub fn client(&self) -> FetcherClient {
        FetcherClient::new(&self.socket)
    }
}

/// A fetch that counts what it was asked for, around another: for tests that check an update
/// fetches only what changed.
pub struct Counting<F> {
    pub inner: F,
    pub asked: std::sync::Mutex<Vec<String>>,
}

impl<F: Fetch> Counting<F> {
    pub fn new(inner: F) -> Self {
        Self {
            inner,
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl<F: Fetch> Fetch for Counting<F> {
    fn fetch(&self, path: &str, limit: u64, out: BorrowedFd) -> Result<u64, FetchError> {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push(path.to_string());
        }
        self.inner.fetch(path, limit, out)
    }
}
