//! Following the registry's sources (docs/adr/0030-installing-applications.mdx, decision 7): an
//! inotify descriptor that is readable when an application may have been installed, updated or
//! removed, so the shell lists it at once.
//!
//! Each source is watched where it is, or, while it is not there yet (the install service makes an
//! account's directory at its first install), at the nearest directory above it that is; every
//! change arms the watches again, so a directory that appeared is then watched itself.

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};

/// What changes a source: a file or directory made, removed, renamed in or out, written, or the
/// watched directory itself going.
const CHANGES: WatchFlags = WatchFlags::CREATE
    .union(WatchFlags::DELETE)
    .union(WatchFlags::MOVED_FROM)
    .union(WatchFlags::MOVED_TO)
    .union(WatchFlags::CLOSE_WRITE)
    .union(WatchFlags::ATTRIB)
    .union(WatchFlags::DELETE_SELF)
    .union(WatchFlags::MOVE_SELF);

pub struct Watch {
    fd: OwnedFd,
    sources: Vec<PathBuf>,
}

/// `path`, or the nearest directory above it that exists.
fn nearest(path: &Path) -> Option<&Path> {
    path.ancestors().find(|dir| dir.is_dir())
}

impl Watch {
    /// A watch over `sources`: directories (and files, by their directories) the registry reads.
    pub fn new(sources: Vec<PathBuf>) -> std::io::Result<Self> {
        let fd = inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC)?;
        let watch = Self { fd, sources };
        watch.arm();
        Ok(watch)
    }

    fn arm(&self) {
        for source in &self.sources {
            if let Some(dir) = nearest(source) {
                // Watched again, a directory keeps its watch: the kernel gives the same one.
                let _ = inotify::add_watch(&self.fd, dir, CHANGES);
            }
        }
    }

    /// Readable when a source may have changed (for poll).
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// Whether a source may have changed since the last call: what the descriptor holds is read,
    /// and the watches are armed again.
    pub fn changed(&mut self) -> bool {
        let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
        let mut changed = false;
        loop {
            let mut reader = inotify::Reader::new(&self.fd, &mut buffer);
            match reader.next() {
                Ok(_) => changed = true,
                Err(_) => break,
            }
            while reader.next().is_ok() {}
        }
        if changed {
            self.arm();
        }
        changed
    }
}
