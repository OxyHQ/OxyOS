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
//! - **A rebuild** ([`Write::Rebuild`]): a second index beside the one answering, which takes what
//!   cannot be read again from the disk (the applications, the packages' declared items, what
//!   applications donated) from the old one, then the crawler's reading. While it is built, what
//!   the crawler reads goes into it alone; what the service gives goes into both, and what is
//!   removed is removed from both (a folder kept out leaves the results at once). Once the
//!   crawler has read everything ([`Write::Rebuilt`]) it takes the old one's place
//!   ([`Index::promote`]), and the questions are answered from it.
//! - **Idle:** with nothing written and not committed, it waits on its channel with no deadline.

use crate::index::{Index, Record, Source};
use std::io::Write as _;
use std::os::unix::net::UnixStream;
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
    /// A rebuild begins, into this index (the crawler's reading goes into it from now on).
    Rebuild(Box<Index>),
    /// The rebuild's crawl ended: the rebuilt index takes the old one's place.
    Rebuilt,
}

/// Wakes the service's answering thread from another: something committed (the results may have
/// changed: it reloads and tells its watchers), or only the index's state changed (it tells who
/// follows the state).
#[derive(Clone)]
pub struct Wake(std::sync::Arc<UnixStream>);

impl Wake {
    pub fn new(stream: UnixStream) -> Wake {
        Wake(std::sync::Arc::new(stream))
    }

    /// What is committed changed.
    pub fn results(&self) {
        let _ = (&*self.0).write(&[RESULTS]);
    }

    /// The index's state changed (a crawl began or went on, a rebuild began).
    pub fn status(&self) {
        let _ = (&*self.0).write(&[STATUS]);
    }
}

/// [`Wake::results`]'s byte.
pub const RESULTS: u8 = 1;
/// [`Wake::status`]'s byte.
pub const STATUS: u8 = 2;

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
    /// A rebuild is under way: from the crawler's asking until its index answers.
    pub rebuilding: bool,
    /// The items a crawl under way has gone through so far (none with no crawl going on).
    pub found: Option<u64>,
    /// Someone follows the state: the crawler says how far it went as it goes.
    pub followed: bool,
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

/// Runs the writer on the calling thread until `writes` ends; `wake` after each commit (the
/// service reloads its reader and tells its watchers) and each change of the state.
pub fn run(mut index: Index, writes: Receiver<Write>, status: Arc<Mutex<Status>>, wake: Wake) {
    background();
    let mut rebuilding: Option<Index> = None;
    let mut first_uncommitted: Option<Instant> = None;
    let mut asked = false;
    let mut settled = false;
    let mut promoted = false;
    let mut done: Vec<Box<dyn FnOnce() + Send>> = Vec::new();
    // Every index written to: the one answering, and a rebuild's.
    macro_rules! each {
        ($index:ident => $body:expr) => {{
            {
                let $index = &mut index;
                let _ = $body;
            }
            if let Some($index) = rebuilding.as_mut() {
                let _ = $body;
            }
        }};
    }
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
                        each!(index => index.commit(true));
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
                    match rebuilding.as_mut() {
                        // What the crawler reads goes into the rebuild alone; what the session
                        // and the applications give, into both.
                        Some(rebuilt) => {
                            let _ = rebuilt.put(&record, text.as_deref());
                            if record.source != Source::File {
                                let _ = index.put(&record, text.as_deref());
                            }
                        }
                        None => {
                            let _ = index.put(&record, text.as_deref());
                        }
                    }
                    first_uncommitted.get_or_insert_with(Instant::now);
                }
                Write::Remove(id) => {
                    each!(index => index.remove(&id));
                    first_uncommitted.get_or_insert_with(Instant::now);
                }
                Write::RemoveInside(folder) => {
                    each!(index => index.remove_inside(&folder));
                    first_uncommitted.get_or_insert_with(Instant::now);
                }
                Write::RemoveSource(source, owner) => {
                    each!(index => index.remove_source(&source, owner.as_deref()));
                    first_uncommitted.get_or_insert_with(Instant::now);
                }
                Write::Commit => asked = true,
                Write::Done(then) => {
                    done.push(then);
                    asked = true;
                }
                Write::Settled => settled = true,
                Write::Busy => settled = false,
                Write::Rebuild(mut rebuilt) => {
                    // What the disk cannot give again, from the index answering.
                    match index.searcher_handle().given() {
                        Ok(given) => {
                            for (record, text) in given {
                                let _ = rebuilt.put(&record, text.as_deref());
                            }
                        }
                        Err(error) => eprintln!(
                            "[search] the rebuild starts without the applications' items: {error}"
                        ),
                    }
                    rebuilding = Some(*rebuilt);
                    first_uncommitted.get_or_insert_with(Instant::now);
                }
                Write::Rebuilt => {
                    if let Some(rebuilt) = rebuilding.take() {
                        // What the old one was given since is committed with it: nothing of the
                        // new one's is lost if the layout cannot be written.
                        let _ = index.commit(false);
                        let (answering, promoted_now) = rebuilt.promote(index);
                        index = answering;
                        if let Err(error) = promoted_now {
                            eprintln!("[search] the rebuilt index could not take the old one's place: {error}");
                        }
                        status.lock().expect("lock").rebuilding = false;
                        promoted = true;
                        asked = true;
                    }
                }
            }
            status.lock().expect("lock").committed = false;
            next = match writes.try_recv() {
                Ok(write) => Some(write),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    each!(index => index.commit(true));
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
        // What answers changes only with what the answering index was given.
        let answering_changed = index.uncommitted() > 0 || promoted;
        if wrote || close {
            let mut failed = None;
            each!(index => if let Err(error) = index.commit(close) { failed = Some(error) });
            if let Some(error) = failed {
                eprintln!("[search] the index could not be written: {error}");
            }
            if close {
                crate::sched::give_back_memory();
            }
        }
        first_uncommitted = None;
        asked = false;
        promoted = false;
        // `settled` stays: the crawler says when it has work again. Nothing to close now.
        settled = false;
        status.lock().expect("lock").committed = true;
        // What waited on the commit is said before the service is woken to say it.
        let answered = !done.is_empty();
        for then in done.drain(..) {
            then();
        }
        if answering_changed || answered {
            wake.results();
        } else {
            // A rebuild's progress, or the crawler settled: the state alone changed.
            wake.status();
        }
    }
}
