//! The volumes mounted for the user, from the mount table, read again only when it changed.
//!
//! Every call that takes a path asks which volumes are mounted (a path in one is in scope), and
//! reading and parsing `/proc/self/mountinfo` each time was about 2 % of the Bloom host's work in
//! Stash. The table is read once and kept; it is read again only after the kernel says it changed,
//! as util-linux's libmount monitor learns it: `/proc/self/mountinfo`, kept open, polls with
//! `POLLPRI` (and `POLLERR`) once the mount namespace's table changed since the descriptor last
//! looked (`proc(5)`, `fs/proc_namespace.c`'s `mounts_poll`, which forgets the change as it says
//! it). Before each use the descriptor is polled with no timeout, so nothing waits and nothing
//! stale is answered: a change made before the poll is read now, one made after it is seen by the
//! next use. A table that is not procfs's (a test's file) is read again when its file changed (its
//! inode, size or modification time).

use crate::places::{self, Volume};
use rustix::event::{poll, PollFd, PollFlags, Timespec};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// procfs's `f_type` (`statfs(2)`).
const PROC_SUPER_MAGIC: i64 = 0x9fa0;

/// A mount table and the volumes it has mounted under the user's media folders.
#[derive(Debug)]
pub(crate) struct MountTable {
    path: PathBuf,
    media: Vec<PathBuf>,
    kept: Mutex<Kept>,
}

#[derive(Debug, Default)]
struct Kept {
    /// The table, open, while it is procfs's: polled for its changes and read from its start.
    proc: Option<File>,
    /// What the table's file was when it was read, while it is not procfs's.
    stamp: Option<Stamp>,
    /// The volumes, as the table last read had them; none before it is read.
    volumes: Option<Vec<Volume>>,
    /// How many times the table was read (tests count them).
    reads: u64,
}

/// What tells a plain file changed.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Stamp {
    dev: u64,
    ino: u64,
    size: i64,
    modified: (i64, i64),
}

impl MountTable {
    pub(crate) fn new(path: PathBuf, media: Vec<PathBuf>) -> MountTable {
        MountTable {
            path,
            media,
            kept: Mutex::new(Kept::default()),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The volumes mounted now.
    pub(crate) fn volumes(&self) -> Vec<Volume> {
        let mut kept = self
            .kept
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if kept.volumes.is_none() {
            // First use: procfs's table is kept open to be polled; any other is a plain file.
            kept.proc = File::open(&self.path).ok().filter(on_procfs);
        }
        let known = kept.volumes.is_some();
        let text = if let Some(file) = &mut kept.proc {
            // Polled before it is read: the poll forgets the change it says.
            if known && !changed(file) {
                None
            } else {
                let mut text = String::new();
                if file.seek(SeekFrom::Start(0)).is_err() || file.read_to_string(&mut text).is_err()
                {
                    text.clear();
                }
                Some(text)
            }
        } else {
            let stamp = stamp(&self.path);
            if stamp.is_some() && stamp == kept.stamp && kept.volumes.is_some() {
                None
            } else {
                kept.stamp = stamp;
                Some(std::fs::read_to_string(&self.path).unwrap_or_default())
            }
        };
        if let Some(text) = text {
            kept.reads += 1;
            kept.volumes = Some(places::volumes(&self.media, &text));
        }
        kept.volumes.clone().unwrap_or_default()
    }

    /// How many times the table was read.
    #[cfg(test)]
    pub(crate) fn reads(&self) -> u64 {
        self.kept.lock().expect("lock").reads
    }
}

/// Whether `file` is on procfs.
// `f_type`'s type is the architecture's (`__fsword_t`): i64 here, i32 or u32 on others.
#[allow(clippy::unnecessary_cast)]
fn on_procfs(file: &File) -> bool {
    rustix::fs::fstatfs(file).is_ok_and(|fs| fs.f_type as i64 == PROC_SUPER_MAGIC)
}

/// Whether procfs's mount table, open as `file`, changed since it last said so: `POLLPRI` or
/// `POLLERR` at once, without waiting. A poll that fails says nothing, so the table is read.
fn changed(file: &File) -> bool {
    let mut fds = [PollFd::new(file, PollFlags::PRI)];
    match poll(&mut fds, Some(&Timespec::default())) {
        Ok(0) => false,
        Ok(_) => fds[0].revents().intersects(PollFlags::PRI | PollFlags::ERR),
        Err(_) => true,
    }
}

fn stamp(path: &Path) -> Option<Stamp> {
    let stat = rustix::fs::stat(path).ok()?;
    Some(Stamp {
        dev: stat.st_dev as u64,
        ino: stat.st_ino as u64,
        size: stat.st_size as i64,
        modified: (stat.st_mtime as i64, stat.st_mtime_nsec as i64),
    })
}
