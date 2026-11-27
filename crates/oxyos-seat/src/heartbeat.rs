//! The session's heartbeat to the login program's worker that started it
//! (docs/adr/0065-the-session-ending-because-of-a-problem.mdx), as a Mac's watchdogd watches the
//! window server: a session that stops answering is ended, the login screen comes back, and the
//! power key's inhibitor goes with it, so a hung session is never a hung machine.
//!
//! - **The channel** is a datagram socket the worker binds for the session it starts, in the
//!   login program's runtime directory and the session's user's alone, named to the session by
//!   `OXYOS_HEARTBEAT`, which it takes out of its environment, so nothing it starts is told it.
//!   One byte a message: `.` alive, `E` ending (an orderly end, which may take its applications'
//!   time to quit).
//! - **The session** ([`Heartbeat`]) says it is alive at the end of its turns, at most every
//!   [`BEAT`], and its turn waits at most [`WAIT`], so a session that runs says it at least that
//!   often. Sent without blocking: a full socket (a worker not reading) drops the beat, never the
//!   session's turn.
//! - **The worker** ([`Watch`]) arms on the first beat (the session's start has its own
//!   deadlines), and holds the session hung after [`HANG`] without one, [`ENDING`] once it said it
//!   is ending. The clock is the monotonic one, which stands still while the machine sleeps, so a
//!   suspend is never a hang. The compositor's longest stalls on a real machine (a display
//!   hot-plug's modeset, 1.3 s on the ThinkPad) are far inside [`HANG`].

use std::io;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The environment variable naming the worker's socket.
pub const VARIABLE: &str = "OXYOS_HEARTBEAT";
/// How often the session says it is alive, at most.
pub const BEAT: Duration = Duration::from_millis(500);
/// The longest a session's turn waits while it has a heartbeat: it says it at least this often.
pub const WAIT: Duration = Duration::from_secs(2);
/// Silence after which a running session is hung.
pub const HANG: Duration = Duration::from_secs(10);
/// Silence after which a session that said it is ending is hung.
pub const ENDING: Duration = Duration::from_secs(45);

const ALIVE: u8 = b'.';
const ENDS: u8 = b'E';

/// The session's end of the heartbeat.
#[derive(Debug)]
pub struct Heartbeat {
    socket: UnixDatagram,
    last: Option<Instant>,
}

impl Heartbeat {
    /// The worker's socket the environment names, taken out of it so that what the session
    /// starts is never told it. None when the session was not started by the login program (a
    /// terminal login, a development run) or the socket cannot be reached.
    pub fn take() -> Option<Self> {
        let named = std::env::var_os(VARIABLE);
        std::env::remove_var(VARIABLE);
        let path = PathBuf::from(named?);
        match Self::connect(&path) {
            Ok(heartbeat) => Some(heartbeat),
            Err(error) => {
                eprintln!("[heartbeat] {}: {error}", path.display());
                None
            }
        }
    }

    /// Connected to the worker's socket at `path`, never blocking.
    pub fn connect(path: &Path) -> io::Result<Self> {
        let socket = UnixDatagram::unbound()?;
        socket.connect(path)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket, last: None })
    }

    /// The session's turn ended at `now`: it says it is alive, if it has not within [`BEAT`].
    pub fn beat(&mut self, now: Instant) {
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) < BEAT)
        {
            return;
        }
        self.last = Some(now);
        self.send(ALIVE);
    }

    /// The session is ending in order: its applications may take their time to quit.
    pub fn ending(&mut self) {
        self.send(ENDS);
    }

    fn send(&self, byte: u8) {
        // A full socket or a worker gone: the beat is dropped, never waited for.
        let _ = self.socket.send(&[byte]);
    }
}

/// The worker's end: a datagram socket bound at `path`, never blocking, for the session's user
/// alone (`uid`, mode 0600), which the worker removes once the session ended.
pub fn listen(path: &Path, uid: u32) -> io::Result<UnixDatagram> {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let socket = UnixDatagram::bind(path)?;
    socket.set_nonblocking(true)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    if rustix::process::geteuid().as_raw() != uid {
        rustix::fs::chown(path, Some(rustix::fs::Uid::from_raw(uid)), None)?;
    }
    Ok(socket)
}

/// What the session sent, without blocking: the worker's side.
pub fn read_beats(socket: &UnixDatagram) -> io::Result<Vec<u8>> {
    let mut all = Vec::new();
    let mut buffer = [0u8; 64];
    loop {
        match socket.recv(&mut buffer) {
            Ok(length) => all.extend_from_slice(&buffer[..length]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(all),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

/// What the worker holds of the session's heartbeat.
#[derive(Debug)]
pub struct Watch {
    /// The last word heard, once the session first said it.
    last: Option<Instant>,
    ending: bool,
    hang: Duration,
    ends: Duration,
}

impl Default for Watch {
    fn default() -> Self {
        Self::with_limits(HANG, ENDING)
    }
}

impl Watch {
    /// A watch holding a session hung after `hang` of silence, `ending` once it said it ends
    /// (tests use shorter ones than [`HANG`] and [`ENDING`]).
    pub fn with_limits(hang: Duration, ending: Duration) -> Self {
        Self {
            last: None,
            ending: false,
            hang,
            ends: ending,
        }
    }

    /// What the session wrote, heard at `now`.
    pub fn heard(&mut self, bytes: &[u8], now: Instant) {
        if bytes.is_empty() {
            return;
        }
        self.last = Some(now);
        if bytes.contains(&ENDS) {
            self.ending = true;
        }
    }

    /// Whether the session is hung at `now`: armed, and silent past [`HANG`] (or [`ENDING`]).
    pub fn hung(&self, now: Instant) -> bool {
        let limit = if self.ending { self.ends } else { self.hang };
        self.last
            .is_some_and(|last| now.saturating_duration_since(last) > limit)
    }

    /// How long until the session would be hung, for the worker's wait.
    pub fn until(&self, now: Instant) -> Option<Duration> {
        let limit = if self.ending { self.ends } else { self.hang };
        self.last
            .map(|last| (last + limit).saturating_duration_since(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn socket_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("oxyos-heartbeat-{name}-{}", std::process::id()))
    }

    #[test]
    fn a_session_is_hung_only_once_armed_and_silent_past_its_limit() {
        let start = Instant::now();
        let mut watch = Watch::default();
        // Not armed: its start has its own deadlines.
        assert!(!watch.hung(start + Duration::from_secs(600)));
        assert_eq!(watch.until(start), None);
        watch.heard(b".", start);
        assert!(!watch.hung(start + HANG));
        assert!(watch.hung(start + HANG + Duration::from_millis(1)));
        // A modeset of 3 s on the turn, then its beat: never hung.
        watch.heard(b".", start + Duration::from_secs(3));
        assert!(!watch.hung(start + Duration::from_secs(12)));
        assert_eq!(
            watch.until(start + Duration::from_secs(4)),
            Some(Duration::from_secs(9))
        );
        // Nothing read is no word.
        watch.heard(b"", start + Duration::from_secs(12));
        assert!(watch.hung(start + Duration::from_secs(14)));
    }

    #[test]
    fn an_ending_session_has_its_applications_time_to_quit() {
        let start = Instant::now();
        let mut watch = Watch::default();
        watch.heard(b"..E", start);
        assert!(!watch.hung(start + Duration::from_secs(30)));
        assert!(watch.hung(start + ENDING + Duration::from_millis(1)));
    }

    #[test]
    fn the_session_beats_at_most_every_half_second_and_never_blocks() {
        let path = socket_path("beats");
        let worker = listen(&path, rustix::process::geteuid().as_raw()).unwrap();
        let mut heartbeat = Heartbeat::connect(&path).unwrap();
        let start = Instant::now();
        heartbeat.beat(start);
        heartbeat.beat(start + Duration::from_millis(100));
        heartbeat.beat(start + BEAT);
        assert_eq!(read_beats(&worker).unwrap(), b"..");
        // A worker that never reads: the socket fills, and the session goes on.
        for n in 0..200_000u32 {
            heartbeat.beat(start + BEAT * (n + 2));
        }
        heartbeat.ending();
        assert!(!read_beats(&worker).unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nothing_reachable_is_no_heartbeat() {
        let path = socket_path("gone");
        let _ = std::fs::remove_file(&path);
        assert!(Heartbeat::connect(&path).is_err());
    }
}
