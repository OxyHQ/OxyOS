//! The index's writer (docs/adr/0051-search.mdx, decision 13): the one thread that changes the
//! index, apart from the crawler that reads the files, so what the session and the applications
//! ask (the applications listed, an application's donations) is written at once, whatever the
//! crawler is doing and however busy the machine is.
//!
//! - **Its class:** the scheduler's normal one, at nice 10, which the writer's own threads
//!   (tantivy's) inherit: below the person's work, never starved by it, as the crawler in the idle
//!   class could be. The crawler cannot hand its writes to a thread of a better class than its own
//!   and take it back (an unprivileged thread never leaves the idle class), so the two are apart.
//! - **What it is told** ([`Write`]), in order, from the crawler and from the service, on one
//!   channel: items put and removed, and when to commit.
//! - **Commits:** at once for what the service asked ([`Write::Commit`], [`Write::Done`]); for the
//!   crawler's writes, once it says it has nothing waiting ([`Write::Settled`]) or every
//!   [`COMMIT_EVERY`] while it goes on, so results appear as a long crawl goes. After a commit
//!   with the crawler settled, the tantivy writer is closed: its threads end and its memory is
//!   given back.
//! - **Idle:** with nothing written and not committed, it waits on its channel with no deadline.

use crate::index::{Index, Record, Source};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often a long crawl's writes are committed.
pub const COMMIT_EVERY: Duration = Duration::from_secs(2);

/// A change to the index, or when to commit.
pub enum Write {
    Put(Box<Record>, Option<String>),
    Remove(String),
    /// Every item inside a folder.
    RemoveInside(PathBuf),
    /// Every item of a source (an owner's, if given).
    RemoveSource(Source, Option<String>),
    /// Commit what was written, now (what the service asked).
    Commit,
    /// Commit, then call this (an answer given once the index holds what was asked).
    Done(Box<dyn FnOnce() + Send>),
    /// The crawler has nothing waiting: commit what it wrote and close the tantivy writer.
    Settled,
    /// The crawler has work again (a change said, a crawl begun).
    Busy,
}

/// What the crawler and the writer say of themselves, read by the service.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The crawler has nothing waiting (its first crawl ended).
    pub crawler_idle: bool,
    /// Everything written is committed.
    pub committed: bool,
    /// The first crawl ended.
    pub crawled: bool,
    /// Some folders are not followed: the system's limit of watches was reached.
    pub partial: bool,
    /// Files read for their text since the start.
    pub read: u64,
}

impl Status {
    /// Nothing is being indexed: the crawler has nothing waiting and all is committed.
    pub fn idle(&self) -> bool {
        self.crawler_idle && self.committed
    }
}

/// Puts the calling thread at nice 10.
fn background() {
    if let Err(error) = rustix::process::setpriority_process(None, 10) {
        eprintln!("[search] the index's writer keeps its priority: {error}");
    }
}

/// Runs the writer on the calling thread until `writes` ends; `committed` is called after each
/// commit (the service reloads its reader and tells its watchers).
pub fn run(
    mut index: Index,
    writes: Receiver<Write>,
    status: Arc<Mutex<Status>>,
    committed: Box<dyn Fn() + Send>,
) {
    background();
    let mut first_uncommitted: Option<Instant> = None;
    let mut asked = false;
    let mut settled = false;
    let mut done: Vec<Box<dyn FnOnce() + Send>> = Vec::new();
    loop {
        // Wait: for anything, with nothing uncommitted; until the commit is due otherwise.
        let first = match first_uncommitted {
            None if !asked => match writes.recv() {
                Ok(write) => Some(write),
                Err(_) => return,
            },
            _ => {
                let due = first_uncommitted.map_or(Instant::now(), |first| first + COMMIT_EVERY);
                let left = if asked || settled {
                    Duration::ZERO
                } else {
                    due.saturating_duration_since(Instant::now())
                };
                match writes.recv_timeout(left) {
                    Ok(write) => Some(write),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => {
                        let _ = index.commit(true);
                        return;
                    }
                }
            }
        };
        // Everything already sent, at once.
        let mut next = first;
        while let Some(write) = next.take() {
            match write {
                Write::Put(record, text) => {
                    let _ = index.put(&record, text.as_deref());
                    first_uncommitted.get_or_insert_with(Instant::now);
                }
                Write::Remove(id) => {
                    let _ = index.remove(&id);
                    first_uncommitted.get_or_insert_with(Instant::now);
                }
                Write::RemoveInside(folder) => {
                    let _ = index.remove_inside(&folder);
                    first_uncommitted.get_or_insert_with(Instant::now);
                }
                Write::RemoveSource(source, owner) => {
                    let _ = index.remove_source(&source, owner.as_deref());
                    first_uncommitted.get_or_insert_with(Instant::now);
                }
                Write::Commit => asked = true,
                Write::Done(then) => {
                    done.push(then);
                    asked = true;
                }
                Write::Settled => settled = true,
                Write::Busy => settled = false,
            }
            status.lock().expect("lock").committed = false;
            next = match writes.try_recv() {
                Ok(write) => Some(write),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    let _ = index.commit(true);
                    return;
                }
            };
        }
        let due = first_uncommitted.is_some_and(|first| first.elapsed() >= COMMIT_EVERY);
        if !(asked || settled || due) {
            continue;
        }
        let close = settled;
        let wrote = first_uncommitted.is_some();
        if wrote || close {
            if let Err(error) = index.commit(close) {
                eprintln!("[search] the index could not be written: {error}");
            }
            if close {
                crate::sched::give_back_memory();
            }
        }
        first_uncommitted = None;
        asked = false;
        // `settled` stays: the crawler says when it has work again. Nothing to close now.
        settled = false;
        status.lock().expect("lock").committed = true;
        // What waited on the commit is said before the service is woken to say it; the service is
        // woken only when there is something to say.
        let answered = !done.is_empty();
        for then in done.drain(..) {
            then();
        }
        if wrote || answered {
            committed();
        }
    }
}
