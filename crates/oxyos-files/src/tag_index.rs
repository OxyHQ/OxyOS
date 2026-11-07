//! The index of the person's tagged items (docs/adr/0049-file-tags.mdx): what finds every item
//! carrying a tag, and every tag there is, without reading the disk again for each question.
//!
//! - **Started when first asked** (the service's `tagged`, `tagNames` and `watchTags`), on a
//!   thread of its own: it crawls the scope's places, the home and the volumes mounted for the
//!   person, reading the tags of each item that has an attribute that may hold them (one
//!   `llistxattr` an item), and watches every folder it crawled with inotify. A question asked
//!   before the crawl ended is answered when it ends.
//! - **Kept current, never crawled again for a question:** an item's attributes changed
//!   (`IN_ATTRIB`), an item made, deleted or moved in or out (a folder moved in is crawled; a move
//!   within the places renames what the index holds, its watches kept, and reads the item again
//!   where it is, a folder crawled, since events said before the move named it where it was), the
//!   volumes mounted and unmounted (the mount table, as [`crate::watch`] follows it). Lost events
//!   (`IN_Q_OVERFLOW`) crawl everything again.
//! - **Not crawled**, as KDE Baloo's folder filters leave them out: hidden folders (a name starting
//!   with `.`), every Trash, dependency and build folders (`node_modules`, `__pycache__`, `CVS`,
//!   `_darcs`, `CMakeFiles`) and `lost+found`. Links are not followed: what a link leads to is
//!   indexed where it is, if it is in the places.
//! - **What it holds:** only the tagged items (path and tags) and the folders watched. When the
//!   system's limit of inotify watches is reached (`ENOSPC`, `fs.inotify.max_user_watches`) the
//!   folders past it are still crawled but not followed: the index logs the first such folder and
//!   says it is partial ([`TagIndex::partial`]).

use crate::tags::{self, Tag};
use crate::Scope;
use rustix::event::{poll, PollFd, PollFlags};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Read, Seek, Write};
use std::mem::MaybeUninit;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Folders the index never crawls, by name (as Baloo's default folder filters).
pub const EXCLUDED_FOLDERS: [&str; 6] = [
    "node_modules",
    "__pycache__",
    "CVS",
    "_darcs",
    "CMakeFiles",
    "lost+found",
];

/// What a crawled folder's watch is told of.
const FOLDER_EVENTS: WatchFlags = WatchFlags::ATTRIB
    .union(WatchFlags::CREATE)
    .union(WatchFlags::DELETE)
    .union(WatchFlags::MOVED_FROM)
    .union(WatchFlags::MOVED_TO)
    .union(WatchFlags::DELETE_SELF)
    .union(WatchFlags::MOVE_SELF)
    .union(WatchFlags::ONLYDIR)
    .union(WatchFlags::DONT_FOLLOW);

/// A tagged item the index found.
pub type Tagged = (PathBuf, Vec<Tag>);

/// A tag's name the index found, and how many items carry it.
pub type TagCount = (String, usize);

/// A question asked before the crawl ended.
enum Question {
    Tagged(String, Box<dyn FnOnce(Vec<Tagged>) + Send>),
    Names(Box<dyn FnOnce(Vec<TagCount>) + Send>),
}

#[derive(Default)]
struct State {
    /// Each tagged item's tags, by path.
    items: HashMap<PathBuf, Vec<Tag>>,
    /// The first crawl ended.
    ready: bool,
    /// Some folders could not be watched.
    partial: bool,
    /// Asked before it was ready.
    waiting: Vec<Question>,
}

impl State {
    fn tagged(&self, name: &str) -> Vec<Tagged> {
        let wanted = tags::key(name);
        let mut found: Vec<Tagged> = self
            .items
            .iter()
            .filter(|(_, tags)| tags.iter().any(|tag| tags::key(&tag.name) == wanted))
            .map(|(path, tags)| (path.clone(), tags.clone()))
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    }

    /// Every tag, case aside, spelt as the item first by path spells it, with how many items carry
    /// it; by name.
    fn names(&self) -> Vec<TagCount> {
        let mut paths: Vec<&PathBuf> = self.items.keys().collect();
        paths.sort();
        let mut names: BTreeMap<String, (String, usize)> = BTreeMap::new();
        for path in paths {
            for tag in &self.items[path] {
                names
                    .entry(tags::key(&tag.name))
                    .or_insert_with(|| (tag.name.clone(), 0))
                    .1 += 1;
            }
        }
        names.into_values().collect()
    }

    fn answer(&self, question: Question) -> Box<dyn FnOnce() + Send> {
        match question {
            Question::Tagged(name, then) => {
                let found = self.tagged(&name);
                Box::new(move || then(found))
            }
            Question::Names(then) => {
                let names = self.names();
                Box::new(move || then(names))
            }
        }
    }
}

/// The index, and its thread.
pub struct TagIndex {
    state: Arc<Mutex<State>>,
    /// The thread's own end: a byte asks it to look at the volumes again; closing it stops it.
    control: Mutex<UnixStream>,
    /// Set as the index goes: a crawl under way stops.
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TagIndex {
    /// Starts the index of `scope`'s places; its thread calls `changed` whenever what it holds
    /// changed (an item's tags, a tagged item moved, gone or come), once per wake.
    pub fn start(scope: Scope, changed: impl Fn() + Send + 'static) -> io::Result<TagIndex> {
        let inotify = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?;
        let state = Arc::new(Mutex::new(State::default()));
        let (control, controlled) = UnixStream::pair()?;
        controlled.set_nonblocking(true)?;
        let stopping = Arc::new(AtomicBool::new(false));
        let thread = {
            let (state, stopping) = (state.clone(), stopping.clone());
            std::thread::Builder::new()
                .name("oxyos-files-tags".into())
                .spawn(move || {
                    let mut crawler = Crawler {
                        scope,
                        inotify,
                        state,
                        folders: HashMap::new(),
                        wds: HashMap::new(),
                        roots: Vec::new(),
                        told_partial: false,
                        stopping,
                    };
                    crawler.run(&controlled, &changed);
                })?
        };
        Ok(TagIndex {
            state,
            control: Mutex::new(control),
            stopping,
            thread: Some(thread),
        })
    }

    /// Every item carrying a tag named `name` (case aside), by path, given to `then` once the
    /// first crawl ended (at once, on the caller's thread, after that).
    pub fn tagged(&self, name: &str, then: impl FnOnce(Vec<Tagged>) + Send + 'static) {
        self.ask(Question::Tagged(name.to_string(), Box::new(then)));
    }

    /// Every tag the index found, with how many items carry it, by name; given as [`Self::tagged`].
    pub fn names(&self, then: impl FnOnce(Vec<TagCount>) + Send + 'static) {
        self.ask(Question::Names(Box::new(then)));
    }

    fn ask(&self, question: Question) {
        let answer = {
            let mut state = self.state.lock().expect("lock");
            if !state.ready {
                state.waiting.push(question);
                return;
            }
            state.answer(question)
        };
        answer();
    }

    /// Whether the first crawl ended.
    pub fn ready(&self) -> bool {
        self.state.lock().expect("lock").ready
    }

    /// Whether some folders are not followed, the system's limit of watches reached.
    pub fn partial(&self) -> bool {
        self.state.lock().expect("lock").partial
    }

    /// Has the thread look at the mount table's volumes now (what a change of procfs's table
    /// does; a test's table is a plain file, which says nothing).
    pub fn volumes_changed(&self) {
        let _ = self.control.lock().expect("lock").write_all(&[1]);
    }
}

impl Drop for TagIndex {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        let _ = self
            .control
            .lock()
            .expect("lock")
            .shutdown(std::net::Shutdown::Both);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The index's thread: its watches and what it crawled.
struct Crawler {
    scope: Scope,
    inotify: OwnedFd,
    state: Arc<Mutex<State>>,
    /// Each watched folder's watch.
    folders: HashMap<PathBuf, i32>,
    /// Each watch's folder.
    wds: HashMap<i32, PathBuf>,
    /// The places crawled: the home and the volumes.
    roots: Vec<PathBuf>,
    /// The first folder past the limit of watches was logged.
    told_partial: bool,
    stopping: Arc<AtomicBool>,
}

/// A move out of a folder, waiting for its move in.
struct MovedFrom {
    cookie: u32,
    path: PathBuf,
}

impl Crawler {
    fn run(&mut self, controlled: &UnixStream, changed: &dyn Fn()) {
        // The mount table, read once first: poll then says only changes after this reading.
        let mut mounts = std::fs::File::open(self.scope.mountinfo()).ok();
        let mut table = Vec::new();
        if let Some(file) = mounts.as_mut() {
            let _ = file.read_to_end(&mut table);
        }
        self.crawl_all();
        let answers: Vec<_> = {
            let mut state = self.state.lock().expect("lock");
            state.ready = true;
            let waiting = std::mem::take(&mut state.waiting);
            waiting
                .into_iter()
                .map(|question| state.answer(question))
                .collect()
        };
        for answer in answers {
            answer();
        }
        let mut buffer = vec![MaybeUninit::<u8>::uninit(); 64 * 1024];
        loop {
            let mut fds = vec![
                PollFd::new(&self.inotify, PollFlags::IN),
                PollFd::from_borrowed_fd(controlled.as_fd(), PollFlags::IN),
            ];
            if let Some(file) = mounts.as_ref() {
                fds.push(PollFd::from_borrowed_fd(file.as_fd(), PollFlags::PRI));
            }
            if poll(&mut fds, None).is_err() {
                continue;
            }
            let ready: Vec<PollFlags> = fds.iter().map(PollFd::revents).collect();
            drop(fds);
            let mut moved = false;
            if !ready[1].is_empty() {
                let mut asked = [0u8; 64];
                match (&*controlled).read(&mut asked) {
                    // The index went.
                    Ok(0) => return,
                    Ok(_) => moved |= self.sync_volumes(),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(_) => return,
                }
            }
            if ready.get(2).is_some_and(|flags| !flags.is_empty()) {
                if let Some(file) = mounts.as_mut() {
                    table.clear();
                    let _ = file.rewind();
                    let _ = file.read_to_end(&mut table);
                }
                moved |= self.sync_volumes();
            }
            if ready[0].contains(PollFlags::IN) {
                moved |= self.events(&mut buffer);
            }
            if moved {
                changed();
            }
        }
    }

    /// The places now: the home and the volumes mounted for the person.
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

    /// Everything forgotten, and every place crawled.
    fn crawl_all(&mut self) {
        for wd in self.wds.keys() {
            let _ = inotify::remove_watch(&self.inotify, *wd);
        }
        self.wds.clear();
        self.folders.clear();
        self.state.lock().expect("lock").items.clear();
        self.told_partial = false;
        self.roots = self.places();
        for root in self.roots.clone() {
            self.crawl(&root);
        }
    }

    /// The volumes mounted and unmounted since they were last looked at: the new crawled, the
    /// gone forgotten. Whether what the index holds changed.
    fn sync_volumes(&mut self) -> bool {
        let places = self.places();
        let mut changed = false;
        for gone in self
            .roots
            .iter()
            .filter(|root| !places.contains(root))
            .cloned()
            .collect::<Vec<_>>()
        {
            changed |= self.forget(&gone);
        }
        let new: Vec<PathBuf> = places
            .iter()
            .filter(|place| !self.roots.contains(place))
            .cloned()
            .collect();
        for place in new {
            changed |= self.crawl(&place);
        }
        self.roots = places;
        changed
    }

    /// Whether the folder `path` (not a place's root) is one the index leaves out.
    fn excluded(&self, path: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return true;
        };
        // A volume's Trash (`.Trash`, `.Trash-UID`) is a hidden folder; the home's is wherever
        // `XDG_DATA_HOME` puts it.
        name.starts_with('.') || EXCLUDED_FOLDERS.contains(&name) || path == self.scope.home_trash()
    }

    /// Crawls the folder `top` and everything in it the index does not leave out: each folder
    /// watched (a folder watched already keeps its watch), each tagged item kept, replacing what
    /// was held inside it. Whether what the index holds changed.
    fn crawl(&mut self, top: &Path) -> bool {
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        let mut folders = vec![top.to_path_buf()];
        while let Some(folder) = folders.pop() {
            if self.stopping.load(Ordering::Relaxed) {
                break;
            }
            // Once each, whatever the mounts inside the places make of the tree.
            let Ok(metadata) = std::fs::symlink_metadata(&folder) else {
                continue;
            };
            if !metadata.is_dir() || !seen.insert((metadata.dev(), metadata.ino())) {
                continue;
            }
            // Watched before it is read, so nothing made meanwhile is missed.
            match inotify::add_watch(&self.inotify, &folder, FOLDER_EVENTS) {
                Ok(wd) => {
                    if let Some(old) = self.wds.insert(wd, folder.clone()) {
                        self.folders.remove(&old);
                    }
                    self.folders.insert(folder.clone(), wd);
                }
                Err(Errno::NOSPC) => self.past_limit(&folder),
                // A folder the person cannot read: nothing in it can be listed.
                Err(_) => continue,
            }
            let Ok(items) = std::fs::read_dir(&folder) else {
                continue;
            };
            for item in items.flatten() {
                if item.file_name().to_str().is_none() {
                    continue;
                }
                let Ok(kind) = item.file_type() else {
                    continue;
                };
                if kind.is_symlink() {
                    continue;
                }
                let path = item.path();
                if kind.is_dir() {
                    if self.excluded(&path) {
                        continue;
                    }
                    folders.push(path.clone());
                }
                if let Some(tags) = read(&path) {
                    found.push((path, tags));
                }
            }
        }
        // What was held inside `top` is what the crawl found now, in one step.
        let mut state = self.state.lock().expect("lock");
        let before = state.items.len();
        state
            .items
            .retain(|item, _| item == top || !item.starts_with(top));
        let any = !found.is_empty() || state.items.len() != before;
        state.items.extend(found);
        any
    }

    fn past_limit(&mut self, folder: &Path) {
        if !self.told_partial {
            eprintln!(
                "[files] the tag index is partial: the system's limit of inotify watches \
                 (fs.inotify.max_user_watches) is reached at {}; folders past it are not followed",
                folder.display()
            );
            self.told_partial = true;
        }
        self.state.lock().expect("lock").partial = true;
    }

    /// Forgets `path` and everything in it: its tagged items, its folders' watches. Whether it
    /// held tagged items.
    fn forget(&mut self, path: &Path) -> bool {
        let gone: Vec<PathBuf> = self
            .folders
            .keys()
            .filter(|folder| folder.starts_with(path))
            .cloned()
            .collect();
        for folder in gone {
            if let Some(wd) = self.folders.remove(&folder) {
                self.wds.remove(&wd);
                let _ = inotify::remove_watch(&self.inotify, wd);
            }
        }
        let mut state = self.state.lock().expect("lock");
        let before = state.items.len();
        state.items.retain(|item, _| !item.starts_with(path));
        state.items.len() != before
    }

    /// `from` and everything in it is at `to` now (a move within the places): what the index
    /// holds renamed, its watches kept (inotify follows a folder moved). Whether it held tagged
    /// items.
    fn rename(&mut self, from: &Path, to: &Path) -> bool {
        // `to` itself, never `to` joined with nothing (which ends in a `/`, a path a file is not
        // at).
        let moved = |path: &Path| {
            let rest = path.strip_prefix(from).ok()?;
            Some(if rest.as_os_str().is_empty() {
                to.to_path_buf()
            } else {
                to.join(rest)
            })
        };
        let folders: Vec<(PathBuf, i32)> = self
            .folders
            .iter()
            .filter(|(folder, _)| folder.starts_with(from))
            .map(|(folder, wd)| (folder.clone(), *wd))
            .collect();
        for (folder, wd) in folders {
            self.folders.remove(&folder);
            if let Some(new) = moved(&folder) {
                self.folders.insert(new.clone(), wd);
                self.wds.insert(wd, new);
            }
        }
        let mut state = self.state.lock().expect("lock");
        let items: Vec<PathBuf> = state
            .items
            .keys()
            .filter(|item| item.starts_with(from))
            .cloned()
            .collect();
        let any = !items.is_empty();
        for item in items {
            if let (Some(tags), Some(new)) = (state.items.remove(&item), moved(&item)) {
                state.items.insert(new, tags);
            }
        }
        any
    }

    /// The item at `path` read again: kept with its tags, or forgotten without. Whether that
    /// changed what the index holds.
    fn refresh(&mut self, path: &Path) -> bool {
        let tags = read(path);
        let mut state = self.state.lock().expect("lock");
        match tags {
            Some(tags) => state.items.insert(path.to_path_buf(), tags.clone()) != Some(tags),
            None => state.items.remove(path).is_some(),
        }
    }

    /// An item come into a watched folder (made, moved in from outside the places, or moved
    /// within them): a folder crawled, anything else read.
    fn came(&mut self, path: &Path) -> bool {
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return false;
        };
        if metadata.file_type().is_symlink() {
            return false;
        }
        if metadata.is_dir() {
            if self.excluded(path) {
                return false;
            }
            let own = self.refresh(path);
            return self.crawl(path) | own;
        }
        self.refresh(path)
    }

    /// What inotify said since the last wake. Whether what the index holds changed.
    fn events(&mut self, buffer: &mut [MaybeUninit<u8>]) -> bool {
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
        let mut changed = false;
        let mut moved_from: Vec<MovedFrom> = Vec::new();
        for (wd, flags, cookie, name) in said {
            if flags.contains(ReadFlags::QUEUE_OVERFLOW) {
                // Events were lost: everything is crawled again.
                self.crawl_all();
                return true;
            }
            if flags.contains(ReadFlags::IGNORED) {
                // The folder went and the kernel dropped its watch.
                if let Some(folder) = self.wds.remove(&wd) {
                    if self.folders.get(&folder) == Some(&wd) {
                        self.folders.remove(&folder);
                    }
                }
                continue;
            }
            let (Some(folder), Some(name)) = (self.wds.get(&wd).cloned(), name) else {
                // A folder's own change: its parent's watch says it, or it is a place's root.
                continue;
            };
            let path = folder.join(&name);
            if flags.contains(ReadFlags::MOVED_FROM) {
                moved_from.push(MovedFrom { cookie, path });
            } else if flags.contains(ReadFlags::MOVED_TO) {
                match moved_from.iter().position(|from| from.cookie == cookie) {
                    Some(at) => {
                        let from = moved_from.remove(at);
                        changed |= self.rename(&from.path, &path);
                        // Moved into a folder the index leaves out (made hidden): forgotten.
                        // Otherwise read again where it is: events said before the move named
                        // it where it was (a copy's staged item tagged, then put in place).
                        let left_out = std::fs::symlink_metadata(&path)
                            .is_ok_and(|found| found.is_dir())
                            && self.excluded(&path);
                        if left_out {
                            changed |= self.forget(&path);
                        } else {
                            changed |= self.came(&path);
                        }
                    }
                    None => changed |= self.came(&path),
                }
            } else if flags.contains(ReadFlags::CREATE) {
                changed |= self.came(&path);
            } else if flags.contains(ReadFlags::DELETE) {
                changed |= self.forget(&path);
            } else if flags.contains(ReadFlags::ATTRIB) {
                changed |= self.refresh(&path);
            }
        }
        // Moved out of the places (or into a folder the index leaves out): forgotten.
        for from in moved_from {
            changed |= self.forget(&from.path);
        }
        changed
    }
}

/// The tags of the item at `path` (itself), if it has any.
fn read(path: &Path) -> Option<Vec<Tag>> {
    if !tags::may_have_tags(path) {
        return None;
    }
    tags::read(path).ok().filter(|tags| !tags.is_empty())
}
