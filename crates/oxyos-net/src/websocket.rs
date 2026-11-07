//! WebSockets for applications on OxyOS: the engine behind React Native's `WebSocketModule` in the
//! Bloom host (docs/react-native-android-adaptation.mdx), through the C ABI (`oxyos-client-ffi`).
//!
//! The protocol is tungstenite's (RFC 6455: the opening handshake, framing and masking, fragments,
//! control frames, the close handshake); nothing of it is written here. Under it, TLS is rustls with
//! the verifier ureq's requests use ([`Options::system`]: the system's certificates, through
//! `rustls-platform-verifier`), and the connection goes through the proxy the process's environment
//! names, as ureq's requests do (`ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY`; an HTTP or
//! HTTPS proxy, by `CONNECT`). OxyOS has no proxy settings of its own.
//!
//! As React Native's Android module opens one (OkHttp's `newWebSocket`, `WebSocketModule.kt`): the
//! cookies of the application's network for the URL's origin, the caller's headers, an `Origin` of
//! the URL's own unless the caller gave one, the protocols asked for in one
//! `Sec-WebSocket-Protocol`; ten seconds to open it and to send what is sent, no limit on reading;
//! a queue of at most 16 MiB to send (more closes it with 1001, as OkHttp does); the close
//! handshake given 60 s before the connection is failed. A ping is a ping frame (Android's sends an
//! empty binary message instead).
//!
//! Each socket runs on a thread of its own, which sleeps in `poll` on its connection, on what the
//! application asks of it (an eventfd) and on the machine's addresses (rtnetlink): an open socket
//! with no traffic wakes nothing, and no timer runs but while something waits to be sent or the
//! close handshake is under way. What it hears is queued as [`Event`]s, in order, which the caller
//! takes from its own thread ([`WebSockets::event`]); the queue's descriptor is readable while any
//! wait.
//!
//! **The network's state.** Opening one while the machine is offline (the connectivity NetInfo
//! reports: no default route) fails at once, but to the machine itself (`localhost`, a loopback
//! address). An open socket whose local address leaves the machine (its network went away) fails
//! then, as Android destroys the sockets of a network it lost.
//!
//! **Teardown.** Dropping [`WebSockets`] closes every open socket with 1001 (going away), as
//! Android's module does when its React instance ends, and waits a second at most for the close
//! frames to be sent.

use crate::connectivity;
use crate::Network;
use rustix::event::{eventfd, poll, EventfdFlags, PollFd, PollFlags, Timespec};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::client::{Request, Response};
use tungstenite::handshake::HandshakeError;
use tungstenite::http::{HeaderName, HeaderValue};
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tungstenite::{Message, WebSocket};
use url::{Host, Url};

/// Opening a socket (resolving, connecting, TLS, a proxy's tunnel, the handshake), as Android's
/// module sets OkHttp's `connectTimeout`.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
/// Sending with no progress, as Android's module sets OkHttp's `writeTimeout`.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// The close handshake, from the close frame sent to the peer's, as OkHttp waits for it.
pub const CLOSE_TIMEOUT: Duration = Duration::from_secs(60);
/// What may wait to be sent, as OkHttp's `MAX_QUEUE_SIZE`.
pub const MAX_QUEUE: usize = 16 * 1024 * 1024;
/// How long a teardown waits for the sockets' close frames to be sent.
const GOING_AWAY_WAIT: Duration = Duration::from_secs(1);

/// What a socket heard, in order: it opened, messages, then its end (closed or failed), the last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The handshake succeeded; the protocol the server chose (empty if none).
    Open {
        id: i64,
        protocol: String,
    },
    Text {
        id: i64,
        text: String,
    },
    Binary {
        id: i64,
        bytes: Vec<u8>,
    },
    /// The close handshake completed: the code and reason the peer's close frame carried (1005, no
    /// status, if it had none). A code of 0 is a socket that was not open when it was used.
    Closed {
        id: i64,
        code: u16,
        reason: String,
    },
    /// It failed, or could not open: why.
    Failed {
        id: i64,
        message: String,
    },
}

impl Event {
    pub fn id(&self) -> i64 {
        match self {
            Event::Open { id, .. }
            | Event::Text { id, .. }
            | Event::Binary { id, .. }
            | Event::Closed { id, .. }
            | Event::Failed { id, .. } => *id,
        }
    }

    fn ends(&self) -> bool {
        matches!(self, Event::Closed { .. } | Event::Failed { .. })
    }
}

/// A socket to open, as `WebSocketModule.connect` takes it.
#[derive(Clone, Debug, Default)]
pub struct Connect {
    pub id: i64,
    /// `ws:`, `wss:`, or `http:` and `https:` for them, as OkHttp takes.
    pub url: String,
    pub protocols: Vec<String>,
    pub headers: Vec<(String, String)>,
}

/// What a socket is opened with.
#[derive(Clone)]
pub struct Options {
    pub tls: Arc<ClientConfig>,
    /// Whether the machine is on a network; asked when a socket opens.
    pub online: Arc<dyn Fn() -> bool + Send + Sync>,
    pub proxy: Option<ureq::Proxy>,
}

impl Options {
    /// The system's: certificates verified as ureq's requests verify them, the connectivity NetInfo
    /// reports, and the environment's proxy as ureq reads it.
    pub fn system() -> Result<Self, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = rustls_platform_verifier::Verifier::new(provider.clone())
            .map_err(|error| format!("the system's certificates: {error}"))?;
        Ok(Self {
            tls: tls_config(provider, verifier)?,
            online: Arc::new(|| connectivity::current(Path::new("/")).connected()),
            proxy: ureq::Proxy::try_from_env(),
        })
    }
}

/// A client configuration verifying servers with `verifier`.
pub fn tls_config(
    provider: Arc<rustls::crypto::CryptoProvider>,
    verifier: rustls_platform_verifier::Verifier,
) -> Result<Arc<ClientConfig>, String> {
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| error.to_string())?
        // Not dangerous: the platform verifier verifies, as ureq's PlatformVerifier does.
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// What the application asks of an open socket, in order.
enum Command {
    Text(String),
    Binary(Vec<u8>),
    Ping,
    Close(u16, String),
    /// The application's runtime is ending: close with 1001 and stop.
    GoingAway,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Connecting,
    Open,
    /// Closed by the application: nothing more may be sent.
    Closing,
}

/// The caller's side of a socket.
struct Socket {
    state: State,
    commands: Sender<Command>,
    wake: Arc<OwnedFd>,
    /// Set to stop an opening socket.
    abort: Arc<AtomicBool>,
    /// The opening socket's connection, shut down to stop it.
    connection: Arc<Mutex<Option<TcpStream>>>,
}

impl Socket {
    fn tell(&self, command: Command) {
        // A thread already ended takes nothing; its end is queued.
        if self.commands.send(command).is_ok() {
            let _ = rustix::io::write(&*self.wake, &1u64.to_ne_bytes());
        }
    }

    fn stop_opening(&self) {
        self.abort.store(true, Ordering::Relaxed);
        if let Some(connection) = &*self.connection.lock().expect("lock") {
            let _ = connection.shutdown(Shutdown::Both);
        }
    }
}

/// Events, the sockets still heard and the threads still running, shared with the threads.
struct Shared {
    events: Mutex<VecDeque<Event>>,
    /// Sockets whose events are queued: from their start to their end. Locked before `events`.
    heard: Mutex<HashSet<i64>>,
    wake: Mutex<UnixStream>,
    running: Mutex<usize>,
    ended: Condvar,
}

impl Shared {
    /// Queues an event of a socket still heard; its end is the last one heard.
    fn emit(&self, event: Event) {
        let mut heard = self.heard.lock().expect("lock");
        let still = if event.ends() {
            heard.remove(&event.id())
        } else {
            heard.contains(&event.id())
        };
        if still {
            self.queue(event);
        }
    }

    fn queue(&self, event: Event) {
        self.events.lock().expect("lock").push_back(event);
        let _ = self.wake.lock().expect("lock").write_all(&[1]);
    }
}

/// A running thread, counted until it ends.
struct Running(Arc<Shared>);

impl Drop for Running {
    fn drop(&mut self) {
        *self.0.running.lock().expect("lock") -= 1;
        self.0.ended.notify_all();
    }
}

/// An application's WebSockets: used by one thread, the caller's.
pub struct WebSockets {
    network: Arc<Network>,
    options: Options,
    shared: Arc<Shared>,
    readable: UnixStream,
    sockets: HashMap<i64, Socket>,
}

impl WebSockets {
    /// Sockets sending `network`'s cookies.
    pub fn new(network: Arc<Network>, options: Options) -> io::Result<Self> {
        let (readable, wake) = UnixStream::pair()?;
        readable.set_nonblocking(true)?;
        wake.set_nonblocking(true)?;
        Ok(Self {
            network,
            options,
            shared: Arc::new(Shared {
                events: Mutex::new(VecDeque::new()),
                heard: Mutex::new(HashSet::new()),
                wake: Mutex::new(wake),
                running: Mutex::new(0),
                ended: Condvar::new(),
            }),
            readable,
            sockets: HashMap::new(),
        })
    }

    /// Starts opening a socket: it opens ([`Event::Open`]) or fails. An error (a malformed URL or
    /// header, an id in use) opens nothing and queues nothing, as `new WebSocket` throws.
    pub fn connect(&mut self, connect: Connect) -> Result<(), String> {
        if self.sockets.contains_key(&connect.id) {
            return Err(format!("WebSocket {} is in use", connect.id));
        }
        let (url, request) = request(&connect, &self.network)?;
        let (commands, received) = mpsc::channel();
        let wake = Arc::new(
            eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)
                .map_err(|error| format!("the WebSocket cannot start: {error}"))?,
        );
        let socket = Socket {
            state: State::Connecting,
            commands,
            wake: wake.clone(),
            abort: Arc::default(),
            connection: Arc::default(),
        };
        let job = Job {
            id: connect.id,
            url,
            request,
            options: self.options.clone(),
            commands: received,
            wake,
            abort: socket.abort.clone(),
            connection: socket.connection.clone(),
            shared: self.shared.clone(),
        };
        self.shared.heard.lock().expect("lock").insert(connect.id);
        *self.shared.running.lock().expect("lock") += 1;
        let running = Running(self.shared.clone());
        let spawned = std::thread::Builder::new()
            .name(format!("oxyos-ws-{}", connect.id))
            .spawn(move || {
                let _running = running;
                job.run();
            });
        if let Err(error) = spawned {
            self.shared.emit(Event::Failed {
                id: connect.id,
                message: format!("the WebSocket cannot start: {error}"),
            });
            return Ok(());
        }
        self.sockets.insert(connect.id, socket);
        Ok(())
    }

    /// Sends a text message. A socket that is not open is ended at once, as Android's module ends
    /// it: failed, then closed with code 0.
    pub fn send_text(&mut self, id: i64, text: String) {
        self.command(id, Command::Text(text));
    }

    /// Sends a binary message, as [`Self::send_text`].
    pub fn send_binary(&mut self, id: i64, bytes: Vec<u8>) {
        self.command(id, Command::Binary(bytes));
    }

    /// Sends a ping frame, as [`Self::send_text`].
    pub fn ping(&mut self, id: i64) {
        self.command(id, Command::Ping);
    }

    fn command(&mut self, id: i64, command: Command) {
        match self.sockets.get(&id) {
            Some(socket) if socket.state == State::Open => socket.tell(command),
            _ => self.not_open(id),
        }
    }

    /// Used while it was not open: its events are no longer heard (but these), and an opening one
    /// stops.
    fn not_open(&mut self, id: i64) {
        if let Some(socket) = self.sockets.remove(&id) {
            if socket.state == State::Connecting {
                socket.stop_opening();
            }
        }
        let mut heard = self.shared.heard.lock().expect("lock");
        heard.remove(&id);
        let message = "the WebSocket is not open".to_string();
        self.shared.queue(Event::Failed {
            id,
            message: message.clone(),
        });
        self.shared.queue(Event::Closed {
            id,
            code: 0,
            reason: message,
        });
    }

    /// Starts the close handshake with `code` and `reason`. One opening fails at once ("closed
    /// before it opened"), as a browser's does; one closed or closing is left alone. An error is a
    /// code or a reason no close frame may carry, which changes nothing.
    pub fn close(&mut self, id: i64, code: u16, reason: &str) -> Result<(), String> {
        if let Some(problem) = close_code_problem(code) {
            return Err(problem);
        }
        if reason.len() > 123 {
            return Err(format!(
                "a close reason is at most 123 bytes, not {}",
                reason.len()
            ));
        }
        let Some(socket) = self.sockets.get_mut(&id) else {
            return Ok(());
        };
        match socket.state {
            State::Connecting => {
                socket.stop_opening();
                self.sockets.remove(&id);
                self.shared.emit(Event::Failed {
                    id,
                    message: "closed before it opened".into(),
                });
            }
            State::Open => {
                socket.state = State::Closing;
                socket.tell(Command::Close(code, reason.to_string()));
            }
            State::Closing => {}
        }
        Ok(())
    }

    /// The next event, if any; never blocks.
    pub fn event(&mut self) -> Option<Event> {
        let mut events = self.shared.events.lock().expect("lock");
        let event = events.pop_front();
        if events.is_empty() {
            let mut drained = [0u8; 64];
            while matches!((&self.readable).read(&mut drained), Ok(n) if n > 0) {}
        }
        drop(events);
        match &event {
            Some(Event::Open { id, .. }) => {
                if let Some(socket) = self.sockets.get_mut(id) {
                    if socket.state == State::Connecting {
                        socket.state = State::Open;
                    }
                }
            }
            Some(ended) if ended.ends() => {
                self.sockets.remove(&ended.id());
            }
            _ => {}
        }
        event
    }
}

impl AsFd for WebSockets {
    /// Readable while events are queued.
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.readable.as_fd()
    }
}

impl Drop for WebSockets {
    fn drop(&mut self) {
        self.shared.heard.lock().expect("lock").clear();
        for socket in self.sockets.values() {
            match socket.state {
                State::Connecting => socket.stop_opening(),
                State::Open | State::Closing => socket.tell(Command::GoingAway),
            }
        }
        self.sockets.clear();
        let deadline = Instant::now() + GOING_AWAY_WAIT;
        let mut running = self.shared.running.lock().expect("lock");
        while *running > 0 {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            running = self
                .shared
                .ended
                .wait_timeout(running, left)
                .expect("lock")
                .0;
        }
    }
}

/// Why `code` may not close a socket, if it may not: as OkHttp checks it (RFC 6455, 7.4).
pub fn close_code_problem(code: u16) -> Option<String> {
    if !(1000..5000).contains(&code) {
        Some(format!("{code} is not a close code (1000 to 4999)"))
    } else if (1004..=1006).contains(&code) || (1015..=2999).contains(&code) {
        Some(format!("{code} is a reserved close code"))
    } else {
        None
    }
}

/// The URL and the handshake's request.
fn request(connect: &Connect, network: &Network) -> Result<(Url, Request), String> {
    let mut url = Url::parse(&connect.url).map_err(|error| format!("{}: {error}", connect.url))?;
    // OkHttp takes http: and https: for ws: and wss:.
    let scheme = match url.scheme() {
        "ws" | "http" => "ws",
        "wss" | "https" => "wss",
        other => return Err(format!("{}: {other}: is not a WebSocket URL", connect.url)),
    };
    url.set_scheme(scheme)
        .map_err(|()| format!("{}: not a WebSocket URL", connect.url))?;
    if url.host().is_none() {
        return Err(format!("{}: no host", connect.url));
    }
    // A fragment is not sent.
    url.set_fragment(None);
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|error| format!("{}: {error}", connect.url))?;
    let headers = request.headers_mut();
    let header = |name: &str, value: &str| -> Result<(HeaderName, HeaderValue), String> {
        Ok((
            HeaderName::try_from(name).map_err(|_| format!("{name:?} is not a header name"))?,
            HeaderValue::try_from(value)
                .map_err(|_| format!("the {name} header's value is not one a header may have"))?,
        ))
    };
    // The application's cookies for the URL's origin, as Android's module sends them.
    let origin = origin(&url);
    let cookies = Url::parse(&format!("{origin}/"))
        .ok()
        .and_then(|origin| network.cookie_header(&origin));
    if let Some(cookies) = cookies {
        let (name, value) = header("cookie", &cookies)?;
        headers.append(name, value);
    }
    let mut has_origin = false;
    for (name, value) in &connect.headers {
        // The handshake's own headers are the handshake's, as OkHttp sets them; the extensions too,
        // none being supported.
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "connection"
                | "upgrade"
                | "sec-websocket-key"
                | "sec-websocket-version"
                | "sec-websocket-extensions"
        ) {
            continue;
        }
        has_origin |= lower == "origin";
        let (name, value) = header(name, value)?;
        if lower == "host" {
            headers.insert(name, value);
        } else {
            headers.append(name, value);
        }
    }
    if !has_origin {
        let (name, value) = header("origin", &origin)?;
        headers.insert(name, value);
    }
    let protocols: Vec<&str> = connect
        .protocols
        .iter()
        .map(|protocol| protocol.trim())
        .filter(|protocol| !protocol.is_empty() && !protocol.contains(','))
        .collect();
    if !protocols.is_empty() {
        let (name, value) = header("sec-websocket-protocol", &protocols.join(","))?;
        headers.insert(name, value);
    }
    Ok((url, request))
}

/// `http[s]://host[:port]` of a WebSocket URL, as Android's module makes its default origin.
fn origin(url: &Url) -> String {
    let scheme = if url.scheme() == "wss" {
        "https"
    } else {
        "http"
    };
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    }
}

/// Whether `host` is this machine, reachable with no network.
fn this_machine(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            name == "localhost" || name.ends_with(".localhost")
        }
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    }
}

/// Whether `local` is still one of the machine's addresses: binding it fails with
/// `EADDRNOTAVAIL` once it is not.
pub fn still_local(local: SocketAddr) -> bool {
    let mut probe = local;
    probe.set_port(0);
    match UdpSocket::bind(probe) {
        Ok(_) => true,
        Err(error) => error.kind() != io::ErrorKind::AddrNotAvailable,
    }
}

/// The connection under a socket: TCP, or TLS over a connection (a server's, or a proxy's).
enum Transport {
    Tcp(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, Transport>>),
}

impl Transport {
    fn tcp(&self) -> &TcpStream {
        match self {
            Transport::Tcp(tcp) => tcp,
            Transport::Tls(tls) => tls.sock.tcp(),
        }
    }

    /// TLS over this connection, its handshake done; a certificate not verified fails it.
    fn tls(self, config: &Arc<ClientConfig>, host: &str) -> Result<Self, String> {
        let name = ServerName::try_from(host.trim_start_matches('[').trim_end_matches(']'))
            .map_err(|error| format!("{host}: {error}"))?
            .to_owned();
        let connection = ClientConnection::new(config.clone(), name)
            .map_err(|error| format!("TLS with {host}: {error}"))?;
        let mut tls = StreamOwned::new(connection, self);
        while tls.conn.is_handshaking() {
            tls.conn
                .complete_io(&mut tls.sock)
                .map_err(|error| failure(&format!("TLS with {host}"), &error))?;
        }
        Ok(Transport::Tls(Box::new(tls)))
    }
}

impl Read for Transport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Transport::Tcp(tcp) => tcp.read(buffer),
            Transport::Tls(tls) => tls.read(buffer),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Transport::Tcp(tcp) => tcp.write(bytes),
            Transport::Tls(tls) => tls.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Transport::Tcp(tcp) => tcp.flush(),
            Transport::Tls(tls) => tls.flush(),
        }
    }
}

/// An I/O error as said to the application; a timeout (a blocking socket's `EAGAIN`) as one.
fn failure(doing: &str, error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => format!("{doing}: timed out"),
        _ => format!("{doing}: {error}"),
    }
}

/// The time left before `deadline`, or a timeout.
fn left(deadline: Instant, doing: &str) -> Result<Duration, String> {
    match deadline.checked_duration_since(Instant::now()) {
        Some(left) if !left.is_zero() => Ok(left),
        _ => Err(format!("{doing}: timed out")),
    }
}

/// A socket's thread: opens it, then converses until it ends.
struct Job {
    id: i64,
    url: Url,
    request: Request,
    options: Options,
    commands: Receiver<Command>,
    wake: Arc<OwnedFd>,
    abort: Arc<AtomicBool>,
    connection: Arc<Mutex<Option<TcpStream>>>,
    shared: Arc<Shared>,
}

/// How a socket's conversation ended.
enum End {
    Closed(u16, String),
    /// The application's runtime ended: nothing to say.
    GoneAway,
}

impl Job {
    fn run(mut self) {
        let id = self.id;
        let opened = self.open();
        if self.abort.load(Ordering::Relaxed) {
            return;
        }
        let (socket, response) = match opened {
            Ok(opened) => opened,
            Err(message) => return self.shared.emit(Event::Failed { id, message }),
        };
        let protocol = response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        self.shared.emit(Event::Open { id, protocol });
        match self.converse(socket) {
            Ok(End::Closed(code, reason)) => self.shared.emit(Event::Closed { id, code, reason }),
            Ok(End::GoneAway) => {}
            Err(message) => self.shared.emit(Event::Failed { id, message }),
        }
    }

    /// Opens the connection (through the proxy, if any), TLS for `wss:`, then the handshake.
    fn open(&mut self) -> Result<(WebSocket<Transport>, Response), String> {
        let deadline = Instant::now() + OPEN_TIMEOUT;
        let host = self.url.host().ok_or("no host")?;
        let name = self.url.host_str().unwrap_or_default().to_string();
        if !this_machine(&host) && !(self.options.online)() {
            return Err("the network is offline".into());
        }
        let port = self.url.port_or_known_default().unwrap_or(80);
        let proxy = self.options.proxy.clone().filter(|proxy| {
            let uri = self.request.uri();
            !proxy.is_no_proxy(uri)
        });
        let mut transport = match &proxy {
            None => Transport::Tcp(self.tcp(&name, port, deadline)?),
            Some(proxy) => self.tunnel(proxy, &name, port, deadline)?,
        };
        if self.url.scheme() == "wss" {
            set_timeouts(transport.tcp(), deadline, "opening the WebSocket")?;
            transport = transport.tls(&self.options.tls, &name)?;
        }
        set_timeouts(transport.tcp(), deadline, "opening the WebSocket")?;
        let config = WebSocketConfig::default().max_write_buffer_size(MAX_QUEUE);
        let mut handshake =
            tungstenite::client::client_with_config(self.request.clone(), transport, Some(config));
        let (mut socket, response) = loop {
            match handshake {
                Ok(done) => break done,
                // The socket's timeout: the time left, then the handshake goes on.
                Err(HandshakeError::Interrupted(mid)) => {
                    left(deadline, "the WebSocket's handshake")?;
                    handshake = mid.handshake();
                }
                Err(HandshakeError::Failure(error)) => return Err(handshake_failure(error)),
            }
        };
        let tcp = socket.get_mut().tcp();
        tcp.set_read_timeout(None).map_err(|e| e.to_string())?;
        tcp.set_write_timeout(None).map_err(|e| e.to_string())?;
        tcp.set_nonblocking(true).map_err(|e| e.to_string())?;
        let _ = tcp.set_nodelay(true);
        Ok((socket, response))
    }

    /// A TCP connection to `host`'s first address that answers.
    fn tcp(&self, host: &str, port: u16, deadline: Instant) -> Result<TcpStream, String> {
        let addresses: Vec<SocketAddr> = (host.trim_start_matches('[').trim_end_matches(']'), port)
            .to_socket_addrs()
            .map_err(|error| format!("{host}: {error}"))?
            .collect();
        let mut last = format!("{host}: no address");
        for address in addresses {
            if self.abort.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let wait = left(deadline, &format!("connecting to {host}"))?;
            match TcpStream::connect_timeout(&address, wait) {
                Ok(tcp) => {
                    // Kept where closing it before it opens can shut it down.
                    let clone = tcp.try_clone().map_err(|e| e.to_string())?;
                    *self.connection.lock().expect("lock") = Some(clone);
                    if self.abort.load(Ordering::Relaxed) {
                        return Err("cancelled".into());
                    }
                    return Ok(tcp);
                }
                Err(error) => last = failure(&format!("connecting to {host} ({address})"), &error),
            }
        }
        Err(last)
    }

    /// A connection to `host` through `proxy`'s tunnel (`CONNECT`), as ureq goes through it.
    fn tunnel(
        &self,
        proxy: &ureq::Proxy,
        host: &str,
        port: u16,
        deadline: Instant,
    ) -> Result<Transport, String> {
        use ureq::ProxyProtocol;
        let secure = match proxy.protocol() {
            ProxyProtocol::Http => false,
            ProxyProtocol::Https => true,
            other => return Err(format!("a {other:?} proxy is not supported")),
        };
        let mut transport = Transport::Tcp(self.tcp(proxy.host(), proxy.port(), deadline)?);
        set_timeouts(transport.tcp(), deadline, "the proxy's tunnel")?;
        if secure {
            transport = transport.tls(&self.options.tls, proxy.host())?;
        }
        let authority = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let mut asked = format!(
            "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: Keep-Alive\r\n"
        );
        if let Some(user) = proxy.username() {
            let credentials = format!("{user}:{}", proxy.password().unwrap_or_default());
            asked += &format!(
                "Proxy-Authorization: Basic {}\r\n",
                data_encoding::BASE64.encode(credentials.as_bytes())
            );
        }
        asked += "\r\n";
        transport
            .write_all(asked.as_bytes())
            .and_then(|()| transport.flush())
            .map_err(|error| failure("the proxy's tunnel", &error))?;
        // Its answer, read a byte at a time: what follows it is the server's.
        let mut answer = Vec::new();
        let mut byte = [0u8];
        while !answer.ends_with(b"\r\n\r\n") {
            if answer.len() > 16 * 1024 {
                return Err("the proxy's answer is too long".into());
            }
            match transport.read(&mut byte) {
                Ok(0) => return Err("the proxy closed the connection".into()),
                Ok(_) => answer.push(byte[0]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(failure("the proxy's tunnel", &error)),
            }
        }
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut response = httparse::Response::new(&mut headers);
        response
            .parse(&answer)
            .map_err(|error| format!("the proxy's answer: {error}"))?;
        match response.code {
            Some(200..=299) => Ok(transport),
            code => Err(format!(
                "the proxy refused the tunnel: {} {}",
                code.unwrap_or_default(),
                response.reason.unwrap_or_default()
            )),
        }
    }

    /// The socket, open, until it ends: what the application asks is sent, what arrives is told.
    fn converse(&mut self, mut socket: WebSocket<Transport>) -> Result<End, String> {
        let local = socket
            .get_ref()
            .tcp()
            .local_addr()
            .map_err(|e| e.to_string())?;
        // The machine's addresses changing; without it, a network lost is found when the
        // connection is used.
        let addresses = connectivity::watch_addresses()
            .map_err(|error| eprintln!("[net] a WebSocket cannot watch the network: {error}"))
            .ok();
        // When what waits to be sent last moved, while some does.
        let mut sending: Option<Instant> = None;
        let mut closing: Option<Instant> = None;
        // The peer's close frame, once it came.
        let mut closed: Option<(u16, String)> = None;
        // What the handshake read past its answer (the server's first messages) is in the socket's
        // buffer, not the connection's: read before waiting.
        let mut waits = false;
        loop {
            let mut out = PollFlags::IN;
            if sending.is_some() {
                out |= PollFlags::OUT;
            }
            let deadline = [
                sending.map(|since| since + SEND_TIMEOUT),
                closing.map(|since| since + CLOSE_TIMEOUT),
            ]
            .into_iter()
            .flatten()
            .min();
            let network_changed = {
                let mut fds = vec![
                    PollFd::new(socket.get_ref().tcp(), out),
                    PollFd::new(&*self.wake, PollFlags::IN),
                ];
                if let Some(addresses) = &addresses {
                    fds.push(PollFd::new(addresses, PollFlags::IN));
                }
                let timeout = deadline.map(|deadline| {
                    let wait = deadline.saturating_duration_since(Instant::now());
                    Timespec::try_from(wait).unwrap_or(Timespec {
                        tv_sec: i64::MAX,
                        tv_nsec: 0,
                    })
                });
                if waits {
                    match poll(&mut fds, timeout.as_ref()) {
                        Ok(_) | Err(rustix::io::Errno::INTR) => {}
                        Err(error) => return Err(format!("waiting on the WebSocket: {error}")),
                    }
                }
                waits = true;
                fds.get(2).is_some_and(|fd| !fd.revents().is_empty())
            };
            let now = Instant::now();
            if sending.is_some_and(|since| now >= since + SEND_TIMEOUT) {
                return Err("sending timed out".into());
            }
            if closing.is_some_and(|since| now >= since + CLOSE_TIMEOUT) {
                return Err("the server did not answer the close in time".into());
            }
            if network_changed {
                if let Some(addresses) = &addresses {
                    let mut buffer = [0u8; 8192];
                    while matches!(
                        rustix::net::recv(addresses, &mut buffer, rustix::net::RecvFlags::DONTWAIT),
                        Ok((n, _)) if n > 0
                    ) {}
                }
                if !still_local(local) {
                    return Err("the network it used went away".into());
                }
            }
            // What the application asked, in order.
            let mut drained = [0u8; 8];
            let _ = rustix::io::read(&*self.wake, &mut drained);
            loop {
                let command = match self.commands.try_recv() {
                    Ok(command) => command,
                    Err(TryRecvError::Empty) => break,
                    // The caller's side is gone: as a runtime ending.
                    Err(TryRecvError::Disconnected) => Command::GoingAway,
                };
                let message = match command {
                    Command::Text(text) => Message::text(text),
                    Command::Binary(bytes) => Message::binary(bytes),
                    Command::Ping => Message::Ping(Default::default()),
                    Command::Close(code, reason) => {
                        closing.get_or_insert(now);
                        Message::Close(Some(CloseFrame {
                            code: CloseCode::from(code),
                            reason: reason.into(),
                        }))
                    }
                    Command::GoingAway => return Ok(self.go_away(socket)),
                };
                // Nothing is sent after a close frame, as OkHttp sends nothing then.
                if !socket.can_write() {
                    continue;
                }
                match socket.write(message) {
                    Ok(()) => {}
                    Err(tungstenite::Error::Io(error))
                        if error.kind() == io::ErrorKind::WouldBlock => {}
                    // More than may wait to be sent: closed as OkHttp closes it.
                    Err(tungstenite::Error::WriteBufferFull(_)) => {
                        closing.get_or_insert(now);
                        let frame = CloseFrame {
                            code: CloseCode::Away,
                            reason: "too much to send".into(),
                        };
                        match socket.close(Some(frame)) {
                            Ok(()) => {}
                            Err(tungstenite::Error::Io(error))
                                if error.kind() == io::ErrorKind::WouldBlock => {}
                            Err(error) => return Err(error.to_string()),
                        }
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            // What arrived, until there is no more for now.
            loop {
                match socket.read() {
                    Ok(Message::Text(text)) => self.shared.emit(Event::Text {
                        id: self.id,
                        text: text.as_str().to_string(),
                    }),
                    Ok(Message::Binary(bytes)) => self.shared.emit(Event::Binary {
                        id: self.id,
                        bytes: bytes.to_vec(),
                    }),
                    Ok(Message::Close(frame)) => {
                        closed = Some(match frame {
                            Some(frame) => (frame.code.into(), frame.reason.as_str().to_string()),
                            None => (u16::from(CloseCode::Status), String::new()),
                        });
                    }
                    Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
                    Err(tungstenite::Error::Io(error))
                        if error.kind() == io::ErrorKind::WouldBlock =>
                    {
                        break
                    }
                    Err(error) => {
                        return match closed.take() {
                            // The connection closed once the peer's close came: closed.
                            Some((code, reason)) => Ok(End::Closed(code, reason)),
                            None => Err(read_failure(error)),
                        };
                    }
                }
                if closed.is_some() {
                    break;
                }
            }
            // Sent: what was asked, a pong, the close's answer.
            match socket.flush() {
                Ok(()) => sending = None,
                Err(tungstenite::Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    sending.get_or_insert(now);
                }
                Err(_) if closed.is_some() => sending = None,
                Err(error) => return Err(error.to_string()),
            }
            // Both close frames exchanged: closed, as OkHttp's onClosed, without waiting for the
            // server to end the connection.
            if sending.is_none() {
                if let Some((code, reason)) = closed.take() {
                    return Ok(End::Closed(code, reason));
                }
            }
        }
    }

    /// Closes with 1001 (going away), sending the close frame for a moment at most.
    fn go_away(&self, mut socket: WebSocket<Transport>) -> End {
        let frame = CloseFrame {
            code: CloseCode::Away,
            reason: "".into(),
        };
        let deadline = Instant::now() + GOING_AWAY_WAIT;
        let mut sent = socket.close(Some(frame));
        loop {
            match sent {
                Err(tungstenite::Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    let wait = deadline.saturating_duration_since(Instant::now());
                    if wait.is_zero() {
                        break;
                    }
                    let mut fds = [PollFd::new(socket.get_ref().tcp(), PollFlags::OUT)];
                    let timeout = Timespec::try_from(wait).ok();
                    let _ = poll(&mut fds, timeout.as_ref());
                    sent = socket.flush();
                }
                _ => break,
            }
        }
        let _ = socket.get_ref().tcp().shutdown(Shutdown::Both);
        End::GoneAway
    }
}

/// Read and write timeouts of what is left before `deadline`.
fn set_timeouts(tcp: &TcpStream, deadline: Instant, doing: &str) -> Result<(), String> {
    let wait = left(deadline, doing)?;
    tcp.set_read_timeout(Some(wait))
        .and_then(|()| tcp.set_write_timeout(Some(wait)))
        .map_err(|error| error.to_string())
}

fn handshake_failure(error: tungstenite::Error) -> String {
    match error {
        // As OkHttp says it.
        tungstenite::Error::Http(response) => format!(
            "Expected HTTP 101 response but was '{} {}'",
            response.status().as_u16(),
            response.status().canonical_reason().unwrap_or_default()
        ),
        tungstenite::Error::Io(error) => failure("the WebSocket's handshake", &error),
        error => format!("the WebSocket's handshake: {error}"),
    }
}

fn read_failure(error: tungstenite::Error) -> String {
    match error {
        tungstenite::Error::ConnectionClosed
        | tungstenite::Error::AlreadyClosed
        | tungstenite::Error::Protocol(
            tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
        ) => "the connection closed without a close frame".into(),
        tungstenite::Error::Io(error) => failure("the WebSocket", &error),
        error => error.to_string(),
    }
}

#[cfg(test)]
#[path = "websocket_tests.rs"]
mod tests;
