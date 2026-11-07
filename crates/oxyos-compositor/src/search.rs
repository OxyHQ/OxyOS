//! Search for the program running the compositor (docs/adr/0051-search.mdx): the session's search
//! index, a service of the user's own run as this program's child, and the broker between it and
//! the clients that ask it.
//!
//! The program starts the service itself, as `<the program> --search-service`, its standard input
//! one end of a socket pair whose other end is this part's: no path, nothing another process could
//! connect to. Both ways the socket carries compact JSON, one object a line. This part tells the
//! service the session's languages and the applications the shell's launcher lists (at its start
//! and as they change), each client's ask with who asked (the application its process was launched
//! as, if the session launched it, and whether it is the shell or Settings, which only the
//! compositor knows), and each asker that went; the service answers each ask, and tells a client
//! that watches what changed (serial 0).
//!
//! A thread writes what the service is told, so the compositor's turn never waits for it, and one
//! reads its answers; each waits on its own events and on the order to stop, and none wakes on a
//! clock (docs/performance.mdx, *Idle*). The service exiting answers every ask it had not answered
//! `unavailable`; it is started again on the next ask, no sooner than RESTART after its last start,
//! and an ask made before then waits for it. One that cannot be started answers every ask
//! `unavailable` until it can.

use crate::stop::Stop;
use oxyos_compositor_core::{ClientId, Scene, SearchAsk};
use rustix::event::PollFlags;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The least time between two starts of the service: one that exits at once is not started over
/// and over.
pub const RESTART: Duration = Duration::from_secs(5);
/// Asks the service has not answered, and asks waiting for it to start: one more is answered busy.
const MAX_WAITING: usize = 256;
/// The longest line the service may write: an answer's text as a JSON string, each character
/// escaped at worst, with room for what names it.
const MAX_LINE: usize = 8 * oxyos_protocol::MAX_SEARCH_TEXT_BYTES;
/// How long the service has to end once its standard input closes, before it is killed.
const GRACE: Duration = Duration::from_secs(2);

/// What an ask is answered when the service cannot answer it.
fn refused(reason: &str) -> String {
    json!({ "refused": { "reason": reason } }).to_string()
}

/// The service's process, as this part sees it: a test plays one in the same process.
pub trait Service: Send {
    /// Whether it exited, reaped if so.
    fn exited(&mut self) -> bool;
    /// Ends it: its standard input is closed already, so it is given GRACE to end, then killed;
    /// reaped either way.
    fn end(&mut self);
}

/// What starts the service with `stdin`, the service's end of the socket.
pub trait Start: Send {
    fn start(&mut self, stdin: OwnedFd) -> io::Result<Box<dyn Service>>;
}

/// The program itself, as `command` runs it.
struct Program(Command);

impl Start for Program {
    fn start(&mut self, stdin: OwnedFd) -> io::Result<Box<dyn Service>> {
        let started = self.0.stdin(Stdio::from(stdin)).spawn();
        // The command keeps what it was given for its standard input: given back now, the
        // service's end is held by the service alone, so its going closes the socket.
        self.0.stdin(Stdio::null());
        Ok(Box::new(started?))
    }
}

impl Service for Child {
    fn exited(&mut self) -> bool {
        !matches!(self.try_wait(), Ok(None))
    }

    fn end(&mut self) {
        let given = Instant::now() + GRACE;
        while matches!(self.try_wait(), Ok(None)) {
            if Instant::now() >= given {
                let _ = self.kill();
                let _ = self.wait();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// What a thread did, for the compositor's turn; each names the start of the service it was of.
enum Done {
    Answer {
        start: u64,
        client: ClientId,
        serial: u32,
        text: String,
    },
    /// The service closed its end, or wrote what is not its vocabulary.
    Ended { start: u64 },
    /// The applications may have changed.
    Applications,
}

#[derive(Clone)]
struct Back {
    done: mpsc::Sender<Done>,
    wake: Arc<UnixStream>,
}

impl Back {
    fn send(&self, done: Done) {
        if self.done.send(done).is_ok() {
            let _ = (&*self.wake).write(&[1]);
        }
    }
}

/// What the service writes, read strictly.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Told {
    Answer {
        client: ClientId,
        serial: u32,
        text: String,
    },
}

/// The service running: its process, what it is told, and the threads that write and read it.
struct Running {
    start: u64,
    service: Box<dyn Service>,
    lines: Option<mpsc::Sender<String>>,
    stop: Arc<Stop>,
    threads: Vec<JoinHandle<()>>,
}

impl Running {
    /// Tells the service `message`, a line of compact JSON.
    fn tell(&self, message: &Value) {
        if let Some(lines) = &self.lines {
            let _ = lines.send(format!("{message}\n"));
        }
    }

    /// Ends the threads, which close this part's end of the socket, then the service.
    fn end(mut self) {
        self.stop.give();
        self.lines = None;
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        self.service.end();
    }
}

/// An application as the service is told it.
fn application_json(application: &oxyos_apps::Application) -> Value {
    json!({
        "id": application.id,
        "name": application.name,
        "keywords": application.keywords,
        "icon": application.icon.as_ref().map(|icon| icon.to_string_lossy()),
        "package": oxyos_apps::package_directory(application).map(|dir| dir.to_string_lossy()),
    })
}

pub struct Search {
    starter: Box<dyn Start>,
    /// Started with the session, rather than at the first ask.
    eager: bool,
    running: Option<Running>,
    /// The starts so far, when the last was, and how long after it the next may be.
    starts: u64,
    started: Option<Instant>,
    restart: Duration,
    /// The last start failed: asks are answered `unavailable` until the next may be tried.
    failed: bool,
    /// Asks waiting for the service to start, and those it has not answered.
    held: Vec<SearchAsk>,
    answering: BTreeSet<(ClientId, u32)>,
    /// The session's languages, the most wanted first: told to the service as its languages
    /// (`es`, then English, every application's own); the registry's names are looked up in them.
    locale: Option<String>,
    /// What the applications are listed from, and the list as last told.
    listing: Option<Arc<oxyos_apps::Listing>>,
    applications: Option<Value>,
    back: Back,
    done: mpsc::Receiver<Done>,
    woken: UnixStream,
    /// The order to stop the thread that follows the applications, and the thread.
    stop: Arc<Stop>,
    following: Option<JoinHandle<()>>,
}

impl Drop for Search {
    fn drop(&mut self) {
        self.stop.give();
        if let Some(thread) = self.following.take() {
            let _ = thread.join();
        }
        if let Some(running) = self.running.take() {
            running.end();
        }
    }
}

impl Search {
    /// The search index started by `command` (the program, `--search-service`): with the session
    /// when `eager`, else at the first ask.
    pub fn new(command: Command, eager: bool) -> io::Result<Self> {
        Self::with_start(Box::new(Program(command)), eager)
    }

    /// The same, the service started by `starter` (a test's).
    pub fn with_start(starter: Box<dyn Start>, eager: bool) -> io::Result<Self> {
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let (done, received) = mpsc::channel();
        Ok(Self {
            starter,
            eager,
            running: None,
            starts: 0,
            started: None,
            restart: RESTART,
            failed: false,
            held: Vec::new(),
            answering: BTreeSet::new(),
            locale: None,
            listing: None,
            applications: None,
            back: Back {
                done,
                wake: Arc::new(wake),
            },
            done: received,
            woken,
            stop: Arc::new(Stop::new()?),
            following: None,
        })
    }

    /// What wakes the compositor's turn when a thread has something (`Compositor::wakers`).
    pub fn wake_fd(&self) -> io::Result<OwnedFd> {
        Ok(self.woken.try_clone()?.into())
    }

    /// The applications the service is told are listed from `listing`, as the shell's launcher
    /// lists them, and followed as they change while the service runs.
    pub fn list_applications(&mut self, listing: oxyos_apps::Listing) {
        self.listing = Some(Arc::new(listing));
        self.applications = None;
        if self.running.is_some() {
            self.follow_applications();
            self.tell_applications();
        }
    }

    /// The session's locale (a BCP 47 tag, `es-ES`): the service is told its languages, and the
    /// applications named in them, when it changes.
    pub fn set_locale(&mut self, locale: &str) {
        if self.locale.as_deref() == Some(locale) {
            return;
        }
        self.locale = Some(locale.to_owned());
        if self.running.is_some() {
            self.tell_languages();
            self.tell_applications();
        }
    }

    /// When the service may next be started, while an ask waits for it: the program's turn wakes
    /// then. None while nothing waits, so an idle session has no clock for it.
    pub fn deadline(&self) -> Option<Instant> {
        if self.running.is_some() || self.held.is_empty() {
            return None;
        }
        self.started.map(|started| started + self.restart)
    }

    /// A turn: what the threads did, the askers gone, the asks made, and the service started
    /// when it is needed and may be.
    pub fn serve(&mut self, scene: &mut Scene) {
        let mut buffer = [0u8; 64];
        while matches!((&self.woken).read(&mut buffer), Ok(read) if read > 0) {}
        for done in self.done.try_iter().collect::<Vec<_>>() {
            match done {
                Done::Answer {
                    start,
                    client,
                    serial,
                    text,
                } if self.is_current(start) => {
                    // Serial 0 is the service's own word to a watcher; any other answers an ask
                    // once, and only one still waiting.
                    if serial == 0 || self.answering.remove(&(client, serial)) {
                        scene.search_text(client, serial, &text);
                    }
                }
                Done::Ended { start } if self.is_current(start) => self.ended(scene),
                Done::Applications => self.tell_applications(),
                _ => {}
            }
        }
        // Reaped with the program's other children, though it has not closed its end yet.
        if self
            .running
            .as_mut()
            .is_some_and(|running| running.service.exited())
        {
            self.ended(scene);
        }
        for client in scene.take_gone_searchers() {
            self.answering.retain(|(asker, _)| *asker != client);
            self.held.retain(|ask| ask.client != client);
            if let Some(running) = &self.running {
                running.tell(&json!({ "gone": { "client": client } }));
            }
        }
        let asks = scene.take_search_asks();
        if self.running.is_none() && self.may_start(!asks.is_empty() || !self.held.is_empty()) {
            self.start(scene);
        }
        for ask in asks {
            if self.held.len() + self.answering.len() >= MAX_WAITING {
                scene.search_text(ask.client, ask.serial, &refused("busy"));
            } else if self.running.is_some() {
                self.send(scene, ask);
            } else if self.failed {
                scene.search_text(ask.client, ask.serial, &refused("unavailable"));
            } else {
                self.held.push(ask);
            }
        }
    }

    fn is_current(&self, start: u64) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.start == start)
    }

    /// Whether the service is to be started now: it is needed (eager and never started, or an
    /// ask for it) and RESTART has passed since its last start.
    fn may_start(&mut self, asked: bool) -> bool {
        let needed = asked || (self.eager && self.starts == 0);
        let due = self
            .started
            .is_none_or(|started| started.elapsed() >= self.restart);
        if due {
            self.failed = false;
        }
        needed && due
    }

    fn start(&mut self, scene: &mut Scene) {
        self.starts += 1;
        self.started = Some(Instant::now());
        let running = UnixStream::pair().and_then(|(ours, theirs)| {
            ours.set_nonblocking(true)?;
            let reading = ours.try_clone()?;
            let stop = Arc::new(Stop::new()?);
            let service = self.starter.start(theirs.into())?;
            Ok((ours, reading, stop, service))
        });
        let (writing, reading, stop, service) = match running {
            Ok(running) => running,
            Err(error) => {
                eprintln!("[search] the service did not start: {error}");
                self.failed = true;
                for ask in std::mem::take(&mut self.held) {
                    scene.search_text(ask.client, ask.serial, &refused("unavailable"));
                }
                return;
            }
        };
        let (lines, to_write) = mpsc::channel::<String>();
        let mut threads = Vec::new();
        {
            let stop = stop.clone();
            if let Ok(thread) = std::thread::Builder::new()
                .name("search-write".into())
                .spawn(move || write(&writing, &to_write, &stop))
            {
                threads.push(thread);
            }
        }
        {
            let (stop, back, start) = (stop.clone(), self.back.clone(), self.starts);
            if let Ok(thread) = std::thread::Builder::new()
                .name("search-read".into())
                .spawn(move || read(&reading, &stop, &back, start))
            {
                threads.push(thread);
            }
        }
        self.running = Some(Running {
            start: self.starts,
            service,
            lines: Some(lines),
            stop,
            threads,
        });
        self.follow_applications();
        self.tell_languages();
        self.applications = None;
        self.tell_applications();
        for ask in std::mem::take(&mut self.held) {
            self.send(scene, ask);
        }
    }

    /// The service went: what it had not answered is answered `unavailable`.
    fn ended(&mut self, scene: &mut Scene) {
        if let Some(running) = self.running.take() {
            eprintln!("[search] the service ended");
            running.end();
        }
        for (client, serial) in std::mem::take(&mut self.answering) {
            scene.search_text(client, serial, &refused("unavailable"));
        }
    }

    /// Hands `ask` to the service with who asked.
    fn send(&mut self, scene: &Scene, ask: SearchAsk) {
        let Some(running) = &self.running else {
            return;
        };
        let Some(grants) = scene.grants_of(ask.client) else {
            return;
        };
        let id = scene.application(ask.client);
        running.tell(&json!({
            "ask": {
                "client": ask.client,
                "serial": ask.serial,
                "asker": {
                    "id": id,
                    "launched": id.is_some(),
                    "shell": grants.shell,
                    "settings": grants.settings,
                },
                "text": ask.text,
            }
        }));
        self.answering.insert((ask.client, ask.serial));
    }

    fn tell_languages(&self) {
        let Some(running) = &self.running else {
            return;
        };
        let language = self
            .locale
            .as_deref()
            .and_then(|locale| locale.split(['-', '_']).next())
            .filter(|language| !language.is_empty())
            .unwrap_or("en");
        let mut languages = vec![language];
        if language != "en" {
            languages.push("en");
        }
        running.tell(&json!({ "languages": languages }));
    }

    /// Tells the service the applications, if they changed since it was last told them.
    fn tell_applications(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let languages = oxyos_apps::languages_of_tag(self.locale.as_deref().unwrap_or("en"));
        let listed: Vec<Value> = self.listing.as_ref().map_or_else(Vec::new, |listing| {
            listing
                .applications(&languages)
                .iter()
                .map(application_json)
                .collect()
        });
        let applications = Value::Array(listed);
        if self.applications.as_ref() == Some(&applications) {
            return;
        }
        running.tell(&json!({ "applications": applications }));
        self.applications = Some(applications);
    }

    /// Follows the applications' sources, once, from the first start on.
    fn follow_applications(&mut self) {
        if self.following.is_some() {
            return;
        }
        let Some(listing) = self.listing.clone() else {
            return;
        };
        let watch = match oxyos_apps::Watch::new(listing.sources()) {
            Ok(watch) => watch,
            Err(error) => {
                eprintln!("[search] the applications are not followed: {error}");
                return;
            }
        };
        let (stop, back) = (self.stop.clone(), self.back.clone());
        self.following = std::thread::Builder::new()
            .name("search-applications".into())
            .spawn(move || {
                let mut watch = watch;
                while stop.wait_for(watch.fd()) {
                    if watch.changed() {
                        back.send(Done::Applications);
                    }
                }
            })
            .ok();
    }
}

/// Writes each line it is given to the service, waiting while the socket is full, until the lines
/// end (the service is ended) or the order to stop is given.
fn write(socket: &UnixStream, lines: &mpsc::Receiver<String>, stop: &Stop) {
    while let Ok(line) = lines.recv() {
        let mut bytes = line.as_bytes();
        while !bytes.is_empty() {
            match (&*socket).write(bytes) {
                Ok(written) => bytes = &bytes[written..],
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if !stop.wait_for_any(socket.as_fd(), PollFlags::OUT) {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    }
}

/// Reads the service's lines and hands on its answers until it closes its end, writes what is not
/// its vocabulary, or the order to stop is given; then says it ended.
fn read(socket: &UnixStream, stop: &Stop, back: &Back, start: u64) {
    let mut line = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    'reading: loop {
        if !stop.wait_for(socket.as_fd()) {
            return;
        }
        let read = match (&*socket).read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                continue
            }
            Err(_) => break,
        };
        line.extend_from_slice(&chunk[..read]);
        while let Some(end) = line.iter().position(|&byte| byte == b'\n') {
            let whole: Vec<u8> = line.drain(..=end).collect();
            if whole.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            match serde_json::from_slice::<Told>(&whole[..end]) {
                Ok(Told::Answer {
                    client,
                    serial,
                    text,
                }) => back.send(Done::Answer {
                    start,
                    client,
                    serial,
                    text,
                }),
                Err(error) => {
                    eprintln!("[search] the service wrote what is not its vocabulary: {error}");
                    break 'reading;
                }
            }
        }
        if line.len() > MAX_LINE {
            eprintln!("[search] the service wrote a line too long");
            break;
        }
    }
    back.send(Done::Ended { start });
}

#[cfg(test)]
mod tests;
