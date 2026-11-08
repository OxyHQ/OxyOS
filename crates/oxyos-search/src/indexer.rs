//! The crawler (docs/adr/0051-search.mdx, decisions 3 and 4): the thread that reads the person's
//! files, at the scheduler's idle class, keeping the index current with them by inotify, as the tag
//! index does (docs/adr/0049-file-tags.mdx). What it reads it hands to the writer
//! ([`crate::writer`]), which changes the index.
//!
//! - **The first crawl** reconciles the index with the places (the home and the volumes mounted
//!   for the person): every folder is watched before it is read; an item the index holds as it is
//!   on the disk (its modification time and size) is not read again, one it lacks or holds
//!   otherwise is read, one it holds that is gone is removed. A session that starts on an index of
//!   a hundred thousand files reads their metadata, not their text.
//! - **Then, never a crawl again for a change:** inotify says an item made, written and closed,
//!   its attributes changed (tags, `touch`), deleted, or moved in, out or within (a folder moved
//!   in is crawled; one moved within is removed where it was and crawled where it is). Changes to
//!   a path are gathered for [`SETTLE`] after the last, so a file written in many steps is read
//!   once. Volumes mounted and unmounted are followed by the mount table, as the file service's
//!   watches follow it. Lost events (`IN_Q_OVERFLOW`) crawl everything again, reconciling.
//! - **Left out**, as Baloo and Spotlight leave them out: hidden files and folders (a name starting
//!   with `.`), every Trash, dependency and build folders (the tag index's list), a folder whose
//!   name ends with `.noindex` (a Mac's convention), a folder holding a `CACHEDIR.TAG` (the Cache
//!   Directory Tagging Specification, which Cargo's, pip's and other caches write), and the folders
//!   the person kept out in Settings › Spotlight.
//! - **Settled:** once nothing waits, it tells the writer, which commits and closes.
//! - **Rebuilt** when the person asks (Settings › Spotlight's *Rebuild Index…*): a new, empty
//!   index beside the one answering, and a crawl that reads every item into it, as the first
//!   crawl of a new index does, at the same idle classes; then it takes the old one's place. A
//!   rebuild the service stopped in goes on at its next start, its committed items not read
//!   again. Asked during another crawl, that crawl ends where it is and the rebuild begins.
//! - **How far a crawl went** (the items it went through) is in the state the service tells,
//!   and the service is woken to tell it at most once a second, only while someone follows it.
//! - **Idle:** with nothing to do, the thread waits on its descriptors with no deadline.

use crate::category::Category;
use crate::extract;
use crate::index::{Index, Record, Searcher, Source};
use crate::writer::{Status, Wake, Write};
use oxyos_files::{tag_index::EXCLUDED_FOLDERS, tags, uri, Scope};
use rustix::event::{poll, PollFd, PollFlags, Timespec};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek};
use std::mem::MaybeUninit;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long changes to a path settle before it is read.
pub const SETTLE: Duration = Duration::from_millis(400);
/// How many items a crawl reads between looks at its commands.
const COMMANDS_EVERY: u32 = 256;
/// How often, at most, a crawl wakes the service to tell how far it went (while followed).
const PROGRESS_EVERY: Duration = Duration::from_secs(1);

/// What a watched folder's watch is told of.
const FOLDER_EVENTS: WatchFlags = WatchFlags::ATTRIB
    .union(WatchFlags::CLOSE_WRITE)
    .union(WatchFlags::CREATE)
    .union(WatchFlags::DELETE)
    .union(WatchFlags::MOVED_FROM)
    .union(WatchFlags::MOVED_TO)
    .union(WatchFlags::DELETE_SELF)
    .union(WatchFlags::MOVE_SELF)
    .union(WatchFlags::ONLYDIR)
    .union(WatchFlags::DONT_FOLLOW);

/// What the crawler is asked by the service.
pub enum Command {
    /// The folders the person kept out (Settings › Spotlight), all of them.
    Excluded(Vec<PathBuf>),
    /// Discard the index and build it again from the files.
    Rebuild,
}

/// The crawler's thread's state.
pub struct Indexer {
    scope: Scope,
    /// The index's cache folder (where a rebuild's index is made).
    cache: PathBuf,
    /// The index as committed: what it holds of the files when the crawl begins.
    searcher: Searcher,
    /// A rebuild's index, as committed, while it is built.
    rebuilding: Option<Searcher>,
    /// A rebuild was asked and has not begun.
    rebuild_asked: bool,
    /// The service's wake, to tell it the state changed.
    wake: Wake,
    /// When the service was last woken to tell how far a crawl went.
    progress_told: Option<Instant>,
    /// Items the crawl under way went through.
    found: u64,
    /// Where what it reads goes.
    writes: Sender<Write>,
    inotify: OwnedFd,
    folders: HashMap<PathBuf, i32>,
    wds: HashMap<i32, PathBuf>,
    roots: Vec<PathBuf>,
    excluded: Vec<PathBuf>,
    /// Paths changed, each read once it has settled.
    pending: BTreeMap<PathBuf, Instant>,
    /// What the index held of the files when it started, until the first crawl reconciled them.
    known: Option<HashMap<String, (i64, u64)>>,
    /// Ids the first crawl saw.
    seen: HashSet<String>,
    status: Arc<Mutex<Status>>,
    told_partial: bool,
    /// Whether it told the writer it has nothing waiting.
    settled: bool,
    /// Items read since the commands were last looked at.
    since_commands: u32,
}

impl Indexer {
    pub fn new(
        scope: Scope,
        cache: PathBuf,
        searcher: Searcher,
        writes: Sender<Write>,
        excluded: Vec<PathBuf>,
        status: Arc<Mutex<Status>>,
        wake: Wake,
    ) -> std::io::Result<Indexer> {
        Ok(Indexer {
            scope,
            cache,
            searcher,
            rebuilding: None,
            rebuild_asked: false,
            wake,
            progress_told: None,
            found: 0,
            writes,
            inotify: inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?,
            folders: HashMap::new(),
            wds: HashMap::new(),
            roots: Vec::new(),
            excluded,
            pending: BTreeMap::new(),
            known: None,
            seen: HashSet::new(),
            status,
            told_partial: false,
            settled: false,
            since_commands: 0,
        })
    }

    /// Runs until `commands` ends: the first crawl, then the changes. `wake` is readable when a
    /// command was sent.
    pub fn run(mut self, commands: Receiver<Command>, wake: OwnedFd) {
        let mut mounts = std::fs::File::open(self.scope.mountinfo()).ok();
        let mut table = Vec::new();
        if let Some(file) = mounts.as_mut() {
            let _ = file.read_to_end(&mut table);
        }
        // The exclusions sent before the crawl first.
        if !self.commands(&commands) {
            return;
        }
        // A rebuild the service stopped in goes on; otherwise the index is reconciled.
        let resume = Index::rebuild_under_way(&self.cache);
        if resume {
            self.rebuild(&commands, true);
        } else {
            self.crawl_all(&commands);
        }
        self.status.lock().expect("lock").crawled = true;
        let mut buffer = vec![MaybeUninit::<u8>::uninit(); 64 * 1024];
        loop {
            if self.rebuild_asked {
                self.rebuild(&commands, false);
            }
            self.settle();
            let deadline = self.deadline();
            if deadline.is_none() && !self.settled {
                self.settled = true;
                self.status.lock().expect("lock").crawler_idle = true;
                let _ = self.writes.send(Write::Settled);
            }
            let timeout = deadline.map(|due| {
                let left = due.saturating_duration_since(Instant::now());
                Timespec {
                    tv_sec: left.as_secs() as i64,
                    tv_nsec: i64::from(left.subsec_nanos()),
                }
            });
            let mut fds = vec![
                PollFd::new(&self.inotify, PollFlags::IN),
                PollFd::from_borrowed_fd(wake.as_fd(), PollFlags::IN),
            ];
            if let Some(file) = mounts.as_ref() {
                fds.push(PollFd::from_borrowed_fd(file.as_fd(), PollFlags::PRI));
            }
            match poll(&mut fds, timeout.as_ref()) {
                Ok(_) | Err(Errno::INTR) => {}
                Err(_) => return,
            }
            let ready: Vec<PollFlags> = fds.iter().map(PollFd::revents).collect();
            drop(fds);
            if !ready[1].is_empty() {
                let mut bytes = [0u8; 64];
                let _ = rustix::io::read(&wake, &mut bytes);
                if !self.commands(&commands) {
                    return;
                }
            }
            if ready.get(2).is_some_and(|flags| !flags.is_empty()) {
                if let Some(file) = mounts.as_mut() {
                    table.clear();
                    let _ = file.rewind();
                    let _ = file.read_to_end(&mut table);
                }
                self.sync_volumes(&commands);
            }
            if ready[0].contains(PollFlags::IN) {
                self.events(&mut buffer, &commands);
            }
        }
    }

    /// When the thread must wake with nothing said: a pending path settling.
    fn deadline(&self) -> Option<Instant> {
        self.pending.values().min().copied()
    }

    /// It has work: the writer and the service are told, once.
    fn busy(&mut self) {
        if self.settled {
            self.settled = false;
            let followed = {
                let mut status = self.status.lock().expect("lock");
                status.crawler_idle = false;
                status.followed
            };
            let _ = self.writes.send(Write::Busy);
            if followed {
                self.wake.status();
            }
        }
    }

    fn write(&mut self, write: Write) {
        self.busy();
        let _ = self.writes.send(write);
    }

    /// Takes the commands sent; false once the service is gone.
    fn commands(&mut self, commands: &Receiver<Command>) -> bool {
        loop {
            match commands.try_recv() {
                Ok(command) => self.command(command),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            // One under way is the one asked.
            Command::Rebuild => self.rebuild_asked = self.rebuilding.is_none(),
            Command::Excluded(excluded) => {
                self.busy();
                let added: Vec<PathBuf> = excluded
                    .iter()
                    .filter(|path| !self.excluded.contains(path))
                    .cloned()
                    .collect();
                let removed: Vec<PathBuf> = self
                    .excluded
                    .iter()
                    .filter(|path| !excluded.contains(path))
                    .cloned()
                    .collect();
                self.excluded = excluded;
                for path in added {
                    self.forget(&path, true);
                }
                for path in removed {
                    if self.roots.iter().any(|root| path.starts_with(root)) && path.is_dir() {
                        self.crawl(&path);
                    }
                }
            }
        }
    }

    /// Reads every pending path that settled.
    fn settle(&mut self) {
        let now = Instant::now();
        let due: Vec<PathBuf> = self
            .pending
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(path, _)| path.clone())
            .collect();
        for path in due {
            self.pending.remove(&path);
            self.refresh(&path);
        }
    }

    /// The places: the home and the volumes mounted for the person.
    fn places(&self) -> Vec<PathBuf> {
        let mut places = vec![self.scope.home().to_path_buf()];
        places.extend(
            self.scope
                .volumes()
                .into_iter()
                .filter_map(|volume| std::fs::canonicalize(&volume.path).ok()),
        );
        places
    }

    /// Builds the index again from the files: a new index beside the one answering (the one
    /// under way when `resume`), every item read into it, then it takes the old one's place.
    /// Ends early, the new index left for the next start, only if the service is gone.
    fn rebuild(&mut self, commands: &Receiver<Command>, resume: bool) {
        self.rebuild_asked = false;
        let index = match Index::rebuild(&self.cache, resume) {
            Ok(index) => index,
            Err(error) => {
                eprintln!("[search] the index cannot be rebuilt: {error}");
                self.status.lock().expect("lock").rebuilding = false;
                self.wake.status();
                return;
            }
        };
        self.rebuilding = Some(index.searcher_handle());
        self.status.lock().expect("lock").rebuilding = true;
        self.write(Write::Rebuild(Box::new(index)));
        let ended = self.crawl_all(commands);
        self.rebuilding = None;
        if ended {
            // The writer says it is no longer rebuilding once its index answers.
            self.write(Write::Rebuilt);
        }
    }

    /// Reconciles the index with every place: what it held, by what is there now (a rebuild's
    /// index, while one is built). False if it ended early: the service gone, or a rebuild asked
    /// (what it did not reach is then not removed).
    fn crawl_all(&mut self, commands: &Receiver<Command>) -> bool {
        for wd in self.wds.keys() {
            let _ = inotify::remove_watch(&self.inotify, *wd);
        }
        self.wds.clear();
        self.folders.clear();
        self.told_partial = false;
        self.busy();
        let searcher = self.rebuilding.as_ref().unwrap_or(&self.searcher).clone();
        let _ = searcher.reload();
        self.known = searcher.files().ok();
        self.seen.clear();
        self.found = 0;
        self.progress(true);
        self.roots = self.places();
        let mut ended = true;
        for root in self.roots.clone() {
            if !self.crawl_with(&root, Some(commands)) {
                ended = false;
                break;
            }
        }
        // What the index held that is no longer there.
        if let Some(known) = self.known.take().filter(|_| ended) {
            for id in known.into_keys() {
                if !self.seen.contains(&id) {
                    self.write(Write::Remove(id));
                }
            }
        }
        self.seen = HashSet::new();
        self.status.lock().expect("lock").found = None;
        ended
    }

    /// Says how far the crawl went: in the state at once, and to the service (to tell who follows
    /// it) at once when `now`, else at most once a second.
    fn progress(&mut self, now: bool) {
        let followed = {
            let mut status = self.status.lock().expect("lock");
            status.found = Some(self.found);
            status.followed
        };
        let due = now
            || self
                .progress_told
                .is_none_or(|told| told.elapsed() >= PROGRESS_EVERY);
        if followed && due {
            self.progress_told = Some(Instant::now());
            self.wake.status();
        }
    }

    fn sync_volumes(&mut self, commands: &Receiver<Command>) {
        let places = self.places();
        for gone in self
            .roots
            .iter()
            .filter(|root| !places.contains(root))
            .cloned()
            .collect::<Vec<_>>()
        {
            self.forget(&gone, true);
        }
        for new in places
            .iter()
            .filter(|place| !self.roots.contains(place))
            .cloned()
            .collect::<Vec<_>>()
        {
            self.crawl_with(&new, Some(commands));
        }
        self.roots = places;
    }

    /// Whether the folder at `path` is left out.
    fn excluded_folder(&self, path: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return true;
        };
        name.starts_with('.')
            || name.ends_with(".noindex")
            || EXCLUDED_FOLDERS.contains(&name)
            || path == self.scope.home_trash()
            || self
                .excluded
                .iter()
                .any(|excluded| path.starts_with(excluded))
            || path.join("CACHEDIR.TAG").is_file()
    }

    /// Whether the item at `path` (a file) is left out, its folder aside.
    fn excluded_file(&self, path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| name.starts_with('.'))
            || self
                .excluded
                .iter()
                .any(|excluded| path.starts_with(excluded))
    }

    fn crawl(&mut self, top: &Path) {
        self.crawl_with(top, None);
    }

    /// Crawls the folder `top` (watched, read, each folder and file in it indexed if it is not
    /// as the index holds it). A long crawl takes commands and says how far it went as it goes;
    /// false if it ended early (the service gone, or a rebuild asked of a crawl that is not one).
    fn crawl_with(&mut self, top: &Path, commands: Option<&Receiver<Command>>) -> bool {
        let mut visited = HashSet::new();
        let mut folders = vec![top.to_path_buf()];
        while let Some(folder) = folders.pop() {
            let Ok(metadata) = std::fs::symlink_metadata(&folder) else {
                continue;
            };
            if !metadata.is_dir() || !visited.insert((metadata.dev(), metadata.ino())) {
                continue;
            }
            match inotify::add_watch(&self.inotify, &folder, FOLDER_EVENTS) {
                Ok(wd) => {
                    if let Some(old) = self.wds.insert(wd, folder.clone()) {
                        if old != folder {
                            self.folders.remove(&old);
                        }
                    }
                    self.folders.insert(folder.clone(), wd);
                }
                Err(Errno::NOSPC) => self.past_limit(&folder),
                Err(_) => continue,
            }
            if !self.roots.contains(&folder) {
                self.index_item(&folder, &metadata);
            }
            let Ok(items) = std::fs::read_dir(&folder) else {
                continue;
            };
            for item in items.flatten() {
                let Ok(kind) = item.file_type() else {
                    continue;
                };
                if kind.is_symlink() || item.file_name().to_str().is_none() {
                    continue;
                }
                let path = item.path();
                if kind.is_dir() {
                    if !self.excluded_folder(&path) {
                        folders.push(path);
                    }
                } else if kind.is_file() && !self.excluded_file(&path) {
                    if let Ok(metadata) = item.metadata() {
                        self.index_item(&path, &metadata);
                    }
                }
                if let Some(commands) = commands {
                    self.since_commands += 1;
                    if self.since_commands >= COMMANDS_EVERY {
                        self.since_commands = 0;
                        if !self.commands(commands) || self.rebuild_asked {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    fn past_limit(&mut self, folder: &Path) {
        if !self.told_partial {
            eprintln!(
                "[search] the index is partial: the system's limit of inotify watches \
                 (fs.inotify.max_user_watches) is reached at {}; folders past it are not followed",
                folder.display()
            );
            self.told_partial = true;
        }
        self.status.lock().expect("lock").partial = true;
    }

    /// Indexes the item at `path` unless the index holds it as it is.
    fn index_item(&mut self, path: &Path, metadata: &std::fs::Metadata) {
        let id = uri::of(path, false);
        let changed = metadata.mtime() * 1_000_000_000 + metadata.mtime_nsec();
        let size = if metadata.is_dir() { 0 } else { metadata.len() };
        if self.known.is_some() {
            // A crawl of the whole goes on: one more item gone through.
            self.found += 1;
            self.progress(false);
        }
        if let Some(known) = &self.known {
            self.seen.insert(id.clone());
            if known.get(&id) == Some(&(changed, size)) {
                return;
            }
        }
        let record = record(path, metadata, id, changed);
        let text = if metadata.is_dir() {
            None
        } else {
            let extension = extension(path);
            let read = extract::text(path, record.category, extension.as_deref(), size);
            if read.is_some() {
                self.status.lock().expect("lock").read += 1;
            }
            read
        };
        self.write(Write::Put(Box::new(record), text));
    }

    /// Removes `path` and, if it was a folder, everything in it and its watches.
    fn forget(&mut self, path: &Path, folder: bool) {
        self.write(Write::Remove(uri::of(path, false)));
        if folder {
            self.write(Write::RemoveInside(path.to_path_buf()));
            let gone: Vec<PathBuf> = self
                .folders
                .keys()
                .filter(|watched| watched.starts_with(path))
                .cloned()
                .collect();
            for watched in gone {
                if let Some(wd) = self.folders.remove(&watched) {
                    self.wds.remove(&wd);
                    let _ = inotify::remove_watch(&self.inotify, wd);
                }
            }
        }
        self.pending.retain(|pending, _| !pending.starts_with(path));
    }

    /// The item at `path`, settled: indexed as it is now, crawled if it is a folder not yet
    /// watched, or forgotten if it is gone or left out.
    fn refresh(&mut self, path: &Path) {
        let was_folder = self.folders.contains_key(path);
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            self.forget(path, true);
            return;
        };
        if metadata.file_type().is_symlink() {
            self.forget(path, was_folder);
        } else if metadata.is_dir() {
            if self.excluded_folder(path) {
                self.forget(path, true);
            } else if was_folder {
                self.index_item(path, &metadata);
            } else {
                self.crawl(path);
            }
        } else if metadata.is_file() && !self.excluded_file(path) {
            if was_folder {
                self.forget(path, true);
            }
            self.index_item(path, &metadata);
        } else {
            self.forget(path, was_folder);
        }
    }

    /// What inotify said since the last wake.
    fn events(&mut self, buffer: &mut [MaybeUninit<u8>], commands: &Receiver<Command>) {
        let mut said = Vec::new();
        {
            let mut reader = inotify::Reader::new(&self.inotify, buffer);
            while let Ok(event) = reader.next() {
                let name = event
                    .file_name()
                    .and_then(|name| name.to_str().ok())
                    .filter(|name| !name.is_empty())
                    .map(String::from);
                said.push((event.wd(), event.events(), event.cookie(), name));
            }
        }
        if !said.is_empty() {
            self.busy();
        }
        let due = Instant::now() + SETTLE;
        let mut moved_from: Vec<(u32, PathBuf)> = Vec::new();
        for (wd, flags, cookie, name) in said {
            if flags.contains(ReadFlags::QUEUE_OVERFLOW) {
                self.pending.clear();
                self.crawl_all(commands);
                return;
            }
            if flags.contains(ReadFlags::IGNORED) {
                if let Some(folder) = self.wds.remove(&wd) {
                    if self.folders.get(&folder) == Some(&wd) {
                        self.folders.remove(&folder);
                    }
                }
                continue;
            }
            let (Some(folder), Some(name)) = (self.wds.get(&wd).cloned(), name) else {
                continue;
            };
            let path = folder.join(&name);
            if flags.contains(ReadFlags::MOVED_FROM) {
                moved_from.push((cookie, path));
            } else if flags.contains(ReadFlags::MOVED_TO) {
                // Moved within the places: gone where it was (its watches with it), read where
                // it is.
                if let Some(at) = moved_from.iter().position(|(from, _)| *from == cookie) {
                    let (_, from) = moved_from.remove(at);
                    self.forget(&from, true);
                }
                self.pending.insert(path, due);
            } else if flags.contains(ReadFlags::DELETE) {
                self.forget(&path, true);
            } else {
                // Made, written and closed, its attributes changed.
                self.pending.insert(path, due);
            }
        }
        // Moved out of the places.
        for (_, from) in moved_from {
            self.forget(&from, true);
        }
    }
}

/// A path's extension, lowercase.
fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
}

/// Milliseconds since the epoch of a file's time.
fn millis(time: std::io::Result<SystemTime>) -> Option<i64> {
    time.ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|since| since.as_millis() as i64)
}

/// The record of the file or folder at `path`.
fn record(path: &Path, metadata: &std::fs::Metadata, id: String, changed: i64) -> Record {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let directory = metadata.is_dir();
    let media = if directory {
        None
    } else {
        oxyos_files::mime::of_name(&name).map(String::from)
    };
    let category = if directory {
        Category::Folder
    } else {
        Category::of_file(media.as_deref(), extension(path).as_deref())
    };
    let tags = if tags::may_have_tags(path) {
        tags::read(path)
            .map(|tags| tags.into_iter().map(|tag| tag.name).collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Record {
        id,
        source: Source::File,
        owner: None,
        category,
        titles: BTreeMap::from([(String::new(), name)]),
        keywords: Vec::new(),
        tags,
        path: Some(path.to_path_buf()),
        url: None,
        application: None,
        icon: None,
        media,
        size: (!directory).then_some(metadata.len()),
        allocated: (!directory).then_some(metadata.blocks().saturating_mul(512)),
        modified: millis(metadata.modified()),
        created: millis(metadata.created()),
        changed: Some(changed),
    }
}
