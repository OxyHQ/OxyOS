//! What a part's threads wait on instead of a clock: the order to stop, which wakes a thread
//! waiting on anything else at once, and a path coming to be (a service's socket made, by inotify).
//! No thread of the session wakes on a timer to see whether it should end or whether something
//! appeared: a session doing nothing does not wake (docs/performance.mdx, *Idle*).

use rustix::event::{poll, PollFd, PollFlags};
use rustix::fd::{AsFd, BorrowedFd};
use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
use std::io::{self, Write};
use std::mem::MaybeUninit;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// The order to stop, given once by the part that owns the threads (when it goes).
pub struct Stop {
    given: AtomicBool,
    /// Readable once the order is given: nothing reads it, so it stays readable for every wait.
    read: UnixStream,
    write: UnixStream,
}

/// What a path is: its device, inode and when its inode last changed (made, to the nanosecond), so
/// a socket made again is told from the one before even when its inode's number is reused.
pub type Identity = (u64, u64, i64, i64);

/// `path`'s identity, if it is there.
pub fn identity(path: &Path) -> Option<Identity> {
    std::fs::metadata(path).ok().map(|metadata| {
        (
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        )
    })
}

/// What makes a path or the directories above it: an entry made, moved in or removed, the
/// watched directory itself going.
const MADE: WatchFlags = WatchFlags::CREATE
    .union(WatchFlags::MOVED_TO)
    .union(WatchFlags::DELETE)
    .union(WatchFlags::MOVED_FROM)
    .union(WatchFlags::DELETE_SELF)
    .union(WatchFlags::MOVE_SELF);

impl Stop {
    pub fn new() -> io::Result<Self> {
        let (read, write) = UnixStream::pair()?;
        write.set_nonblocking(true)?;
        Ok(Self {
            given: AtomicBool::new(false),
            read,
            write,
        })
    }

    /// Gives the order: every thread waiting wakes, and every later wait returns at once.
    pub fn give(&self) {
        self.given.store(true, Ordering::SeqCst);
        let _ = (&self.write).write(&[1]);
    }

    pub fn given(&self) -> bool {
        self.given.load(Ordering::SeqCst)
    }

    /// Readable once the order is given (for a wait of another kind: an async one, or poll).
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.read.as_fd()
    }

    /// Waits, however long, until `fd` is readable (true) or the order is given (false).
    pub fn wait_for(&self, fd: BorrowedFd<'_>) -> bool {
        self.wait_for_any(fd, PollFlags::IN)
    }

    /// Waits until `fd` has one of `events` (or an error or hang-up: true), or the order is
    /// given (false).
    pub fn wait_for_any(&self, fd: BorrowedFd<'_>, events: PollFlags) -> bool {
        loop {
            if self.given() {
                return false;
            }
            let mut fds = [
                PollFd::from_borrowed_fd(self.read.as_fd(), PollFlags::IN),
                PollFd::from_borrowed_fd(fd, events),
            ];
            match poll(&mut fds, None) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(_) => return !self.given(),
            }
            if self.given() {
                return false;
            }
            if !fds[1].revents().is_empty() {
                return true;
            }
        }
    }

    /// Waits until the order is given.
    pub fn wait(&self) {
        while !self.given() {
            let mut fds = [PollFd::from_borrowed_fd(self.read.as_fd(), PollFlags::IN)];
            let _ = poll(&mut fds, None);
        }
    }

    /// Waits until `path` is there and is not `before` (a socket made again: the one a service
    /// that was gone, or never answered, made anew), true; or the order is given, false. The
    /// directory it is in is watched, or while it is not there the nearest one above that is,
    /// so a service whose directory is made at its start is seen as it starts.
    pub fn wait_for_path(&self, path: &Path, before: Option<Identity>) -> bool {
        let Ok(watch) = inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC) else {
            self.wait();
            return false;
        };
        let name = path
            .file_name()
            .map(|name| name.as_encoded_bytes().to_vec());
        loop {
            if self.given() {
                return false;
            }
            // Watched before it is looked at, so nothing made in between is missed.
            let own = arm(&watch, path, || {});
            if let Some(now) = identity(path) {
                if Some(now) != before {
                    return true;
                }
            }
            if !self.wait_for(watch.as_fd()) {
                return false;
            }
            // Made (or moved) there while watched: a new one, whatever it looks like.
            let mut made = false;
            let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
            let mut reader = inotify::Reader::new(&watch, &mut buffer);
            while let Ok(event) = reader.next() {
                made |= Some(event.wd()) == own
                    && event
                        .events()
                        .intersects(inotify::ReadFlags::CREATE | inotify::ReadFlags::MOVED_TO)
                    && event.file_name().map(|file| file.to_bytes().to_vec()) == name;
            }
            if made && path.exists() {
                return true;
            }
        }
    }
}

/// The directory `path` is to be in, or while it is not there the nearest one above that is.
fn nearest(path: &Path) -> Option<&Path> {
    path.parent()?.ancestors().find(|dir| dir.is_dir())
}

/// Watches where `path` will be made ([`nearest`]), and returns the watch of `path`'s own
/// directory once that is the one watched. The nearest directory is looked for, watched, then
/// looked for again until the one watched is still the nearest: a directory made below it between
/// the look and the watch (its creation came before the watch, so no event says it) is watched in
/// turn, and nothing made in it afterwards is missed. `between` runs between each look and its
/// watch (nothing in a session; a test makes a directory there).
fn arm(watch: &rustix::fd::OwnedFd, path: &Path, mut between: impl FnMut()) -> Option<i32> {
    loop {
        let dir = nearest(path)?;
        between();
        let watched = inotify::add_watch(watch, dir, MADE).ok();
        if nearest(path) == Some(dir) {
            return watched.filter(|_| Some(dir) == path.parent());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn directory(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("oxyos-stop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_path_made_below_directories_not_there_yet_is_seen() {
        let root = directory("made");
        std::fs::create_dir_all(&root).unwrap();
        let socket = root.join("run/oxyos/installd/socket");
        let stop = Arc::new(Stop::new().unwrap());
        let waiting = {
            let (stop, socket) = (stop.clone(), socket.clone());
            std::thread::spawn(move || stop.wait_for_path(&socket, None))
        };
        std::thread::sleep(Duration::from_millis(50));
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert!(waiting.join().unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_made_between_the_look_and_the_watch_is_watched_too() {
        let root = directory("between");
        std::fs::create_dir_all(&root).unwrap();
        let socket = root.join("run/oxyos/installd/socket");
        let watch = inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC).unwrap();
        // The service makes its directories just after the nearest one was looked for and
        // before it is watched: their creation comes before any watch could say it.
        let mut made = false;
        let own = arm(&watch, &socket, || {
            if !made {
                std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
                made = true;
            }
        });
        assert!(own.is_some(), "the socket's own directory is watched");
        // Its socket made then is seen.
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
        let mut reader = inotify::Reader::new(&watch, &mut buffer);
        let mut seen = false;
        while let Ok(event) = reader.next() {
            seen |= Some(event.wd()) == own
                && event.events().contains(inotify::ReadFlags::CREATE)
                && event.file_name().map(|name| name.to_bytes()) == Some(b"socket".as_slice());
        }
        assert!(seen, "the socket's creation was not seen");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_socket_made_again_is_told_from_the_one_before() {
        let root = directory("again");
        std::fs::create_dir_all(&root).unwrap();
        let socket = root.join("socket");
        let first = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let before = identity(&socket);
        let stop = Arc::new(Stop::new().unwrap());
        let waiting = {
            let (stop, socket) = (stop.clone(), socket.clone());
            std::thread::spawn(move || stop.wait_for_path(&socket, before))
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(!waiting.is_finished(), "the same socket is not a new one");
        drop(first);
        std::fs::remove_file(&socket).unwrap();
        let _again = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert!(waiting.join().unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_order_wakes_every_wait_at_once() {
        let root = directory("order");
        let stop = Arc::new(Stop::new().unwrap());
        let (never, _other) = UnixStream::pair().unwrap();
        let waits = [
            {
                let stop = stop.clone();
                std::thread::spawn(move || stop.wait_for_path(&root.join("never"), None))
            },
            {
                let stop = stop.clone();
                std::thread::spawn(move || stop.wait_for(never.as_fd()))
            },
        ];
        std::thread::sleep(Duration::from_millis(50));
        let given = Instant::now();
        stop.give();
        for wait in waits {
            assert!(!wait.join().unwrap());
        }
        assert!(given.elapsed() < Duration::from_secs(1));
        // Given, a wait returns at once.
        assert!(!stop.wait_for(stop.fd()));
    }
}
