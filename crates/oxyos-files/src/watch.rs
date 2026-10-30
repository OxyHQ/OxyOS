//! Changes an application watches (docs/adr/0028-file-access.mdx): what changes in a folder or a
//! file, as `expo-file-system`'s watchers say it, and whether the Trash or the volumes mounted
//! changed, as the Bloom adapter's places and Trash say it.
//!
//! A watch is a number, of one of two kinds:
//!
//! - **Typed** ([`Watcher::watch_path`], `Directory.watch` and `File.watch`): inotify's events on
//!   the folder or file (an item created, written, deleted, moved in or out; the watched item
//!   itself deleted or moved), gathered until nothing happened for the watch's debounce and then
//!   said as Android's `FileSystemWatcher` says them: each item's events merged into one, a move
//!   out paired with a move in (in order) as one `renamed` with both paths, the rest each a
//!   `created`, `modified`, `deleted` or `renamed` of its own. The watched item deleted or moved
//!   is said at once, and ends the watch, what was gathered dropped.
//! - **Coalesced** ([`Watcher::watch`], the Trash's folders and the mount table): only that it
//!   changed, once per wake whatever many events it read, since the application reads it again.
//!   The volumes' is the mount table's (`/proc/self/mountinfo` is readable as urgent data when it
//!   changes, as `poll(2)` documents).
//!
//! One thread waits on inotify, the mount table and the earliest debounce. A lost queue
//! (`IN_Q_OVERFLOW`) tells every coalesced watch, and every typed one a `modified` of what it
//! watches. Paths in events are spelt as the application gave the watched one.

use rustix::event::{poll, PollFd, PollFlags, Timespec};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use std::collections::{BTreeSet, HashMap};
use std::io::{self, Read, Seek, Write};
use std::mem::MaybeUninit;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// What a coalesced folder watch is told of.
const FOLDER_EVENTS: WatchFlags = WatchFlags::CREATE
    .union(WatchFlags::DELETE)
    .union(WatchFlags::MOVED_FROM)
    .union(WatchFlags::MOVED_TO)
    .union(WatchFlags::CLOSE_WRITE)
    .union(WatchFlags::ATTRIB)
    .union(WatchFlags::DELETE_SELF)
    .union(WatchFlags::MOVE_SELF)
    .union(WatchFlags::ONLYDIR);

/// What a typed watch is told of: Android's `FileSystemWatcher` mask.
const TYPED_EVENTS: WatchFlags = WatchFlags::CREATE
    .union(WatchFlags::MODIFY)
    .union(WatchFlags::DELETE)
    .union(WatchFlags::DELETE_SELF)
    .union(WatchFlags::MOVED_FROM)
    .union(WatchFlags::MOVED_TO)
    .union(WatchFlags::MOVE_SELF);

/// inotify's events as Android's `FileObserver` gives them (`ALL_EVENTS`): an event's
/// `nativeEventFlags`.
const FLAGS: u32 = 0xfff;
const IN_MODIFY: u32 = 0x2;
const IN_MOVED_FROM: u32 = 0x40;
const IN_MOVED_TO: u32 = 0x80;
const IN_CREATE: u32 = 0x100;
const IN_DELETE: u32 = 0x200;
const IN_DELETE_SELF: u32 = 0x400;
const IN_MOVE_SELF: u32 = 0x800;
const IN_Q_OVERFLOW: u32 = 0x4000;
const IN_ISDIR: u32 = 0x4000_0000;

/// What happened to an item, as `expo-file-system` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Created,
    Modified,
    Deleted,
    Renamed,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Created => "created",
            Kind::Modified => "modified",
            Kind::Deleted => "deleted",
            Kind::Renamed => "renamed",
        }
    }
}

/// A change a typed watch says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: Kind,
    /// The item (for `renamed`, where it was).
    pub path: PathBuf,
    pub directory: bool,
    /// inotify's events merged for it (`FileObserver`'s flags).
    pub flags: u32,
    /// For a move out paired with a move in: where it went, and whether that is a folder.
    pub new_path: Option<(PathBuf, bool)>,
}

/// What the watcher's thread says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// These coalesced watches changed.
    Watches(Vec<u32>),
    /// A typed watch's changes, in order; `ended` when the watched item went and the watch with
    /// it.
    Events {
        watch: u32,
        events: Vec<Event>,
        ended: bool,
    },
}

/// A typed watch: what it watches, and what it gathered while its debounce runs.
struct Typed {
    wd: i32,
    /// The watched item, as the application named it.
    path: PathBuf,
    directory: bool,
    debounce: Duration,
    /// Each item's events (none: the watched item itself), in the order first seen.
    pending: Vec<(Option<String>, u32, bool)>,
    /// Moves out, in order, with whether each was a folder.
    moved_from: Vec<(String, bool)>,
    due: Option<Instant>,
}

#[derive(Default)]
struct Table {
    next: u32,
    /// Each coalesced watch's inotify descriptors, and whether it follows the mount table.
    coalesced: HashMap<u32, (Vec<i32>, bool)>,
    typed: HashMap<u32, Typed>,
    /// Which watches each inotify descriptor belongs to (one folder watched twice is one
    /// descriptor: inotify gives the same one for the same folder).
    by_wd: HashMap<i32, BTreeSet<u32>>,
}

impl Table {
    /// Forgets watch `id`'s claim on `wd`; true if no watch has it now.
    fn release(&mut self, wd: i32, id: u32) -> bool {
        let Some(ids) = self.by_wd.get_mut(&wd) else {
            return false;
        };
        ids.remove(&id);
        if ids.is_empty() {
            self.by_wd.remove(&wd);
            return true;
        }
        false
    }
}

/// Watches, and the thread that says when they change.
pub struct Watcher {
    inotify: Arc<OwnedFd>,
    table: Arc<Mutex<Table>>,
    stop: UnixStream,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    /// A watcher whose thread calls `changed` with what changed, from its own thread;
    /// `mountinfo` is the mount table it follows.
    pub fn new(mountinfo: &Path, changed: impl Fn(Change) + Send + 'static) -> io::Result<Watcher> {
        let inotify = Arc::new(inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?);
        let table = Arc::new(Mutex::new(Table::default()));
        let (stop, stopped) = UnixStream::pair()?;
        let mounts = std::fs::File::open(mountinfo).ok();
        let thread = {
            let (inotify, table) = (inotify.clone(), table.clone());
            std::thread::Builder::new()
                .name("oxyos-files-watch".into())
                .spawn(move || run(&inotify, mounts, &stopped, &table, &changed))?
        };
        Ok(Watcher {
            inotify,
            table,
            stop,
            thread: Some(thread),
        })
    }

    /// Adds `flags` to inotify's watch on `path` (the masks of every watch on it together).
    fn add(&self, path: &Path, flags: WatchFlags) -> io::Result<i32> {
        Ok(inotify::add_watch(
            &*self.inotify,
            path,
            flags | WatchFlags::MASK_ADD,
        )?)
    }

    /// A new coalesced watch on `folders` (each must be a folder) and, if `mounts`, on the mount
    /// table.
    pub fn watch(&self, folders: &[PathBuf], mounts: bool) -> io::Result<u32> {
        let mut wds = Vec::new();
        for folder in folders {
            match self.add(folder, FOLDER_EVENTS) {
                Ok(wd) => wds.push(wd),
                Err(error) => {
                    let table = self.table.lock().expect("lock");
                    for wd in wds {
                        if !table.by_wd.contains_key(&wd) {
                            let _ = inotify::remove_watch(&*self.inotify, wd);
                        }
                    }
                    return Err(error);
                }
            }
        }
        let mut table = self.table.lock().expect("lock");
        table.next += 1;
        let id = table.next;
        for &wd in &wds {
            table.by_wd.entry(wd).or_default().insert(id);
        }
        table.coalesced.insert(id, (wds, mounts));
        Ok(id)
    }

    /// A new typed watch on the folder or file at `reached` (canonical), its events named under
    /// `named` (as the application named it), gathered for `debounce` of quiet.
    pub fn watch_path(&self, reached: &Path, named: &Path, debounce: Duration) -> io::Result<u32> {
        let directory = std::fs::metadata(reached)?.is_dir();
        let wd = self.add(reached, TYPED_EVENTS)?;
        let mut table = self.table.lock().expect("lock");
        table.next += 1;
        let id = table.next;
        table.by_wd.entry(wd).or_default().insert(id);
        table.typed.insert(
            id,
            Typed {
                wd,
                path: named.to_path_buf(),
                directory,
                debounce,
                pending: Vec::new(),
                moved_from: Vec::new(),
                due: None,
            },
        );
        Ok(id)
    }

    /// Ends watch `id`; nothing is said of it again.
    pub fn unwatch(&self, id: u32) {
        let mut table = self.table.lock().expect("lock");
        let wds = if let Some((wds, _)) = table.coalesced.remove(&id) {
            wds
        } else if let Some(typed) = table.typed.remove(&id) {
            vec![typed.wd]
        } else {
            return;
        };
        for wd in wds {
            if table.release(wd, id) {
                let _ = inotify::remove_watch(&*self.inotify, wd);
            }
        }
    }

    /// How many watches there are.
    pub fn len(&self) -> usize {
        let table = self.table.lock().expect("lock");
        table.coalesced.len() + table.typed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.stop.write_all(&[1]);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    inotify: &OwnedFd,
    mut mounts: Option<std::fs::File>,
    stopped: &UnixStream,
    table: &Mutex<Table>,
    changed: &dyn Fn(Change),
) {
    // The table is read once first: poll then reports only changes after this reading.
    let mut table_text = Vec::new();
    if let Some(file) = mounts.as_mut() {
        let _ = file.read_to_end(&mut table_text);
    }
    let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
    loop {
        let due = table
            .lock()
            .expect("lock")
            .typed
            .values()
            .filter_map(|typed| typed.due)
            .min();
        let timeout = due.map(|due| {
            let wait = due.saturating_duration_since(Instant::now());
            Timespec {
                tv_sec: i64::try_from(wait.as_secs()).unwrap_or(i64::MAX),
                tv_nsec: i64::from(wait.subsec_nanos()),
            }
        });
        let mut fds = vec![
            PollFd::new(inotify, PollFlags::IN),
            PollFd::from_borrowed_fd(stopped.as_fd(), PollFlags::IN),
        ];
        if let Some(file) = mounts.as_ref() {
            fds.push(PollFd::from_borrowed_fd(file.as_fd(), PollFlags::PRI));
        }
        if poll(&mut fds, timeout.as_ref()).is_err() {
            continue;
        }
        let ready: Vec<PollFlags> = fds.iter().map(PollFd::revents).collect();
        drop(fds);
        if !ready[1].is_empty() {
            return;
        }
        let mut ids = BTreeSet::new();
        let mut said = Vec::new();
        if ready[0].contains(PollFlags::IN) {
            let mut reader = inotify::Reader::new(inotify, &mut buffer);
            let mut table = table.lock().expect("lock");
            while let Ok(event) = reader.next() {
                let mask = event.events().bits();
                if mask & IN_Q_OVERFLOW != 0 {
                    // Events were lost: every folder is read again.
                    let table = &mut *table;
                    ids.extend(table.coalesced.keys().copied());
                    for typed in table.typed.values_mut() {
                        typed
                            .pending
                            .push((None, IN_MODIFY | IN_Q_OVERFLOW, typed.directory));
                        typed.due = Some(Instant::now() + typed.debounce);
                    }
                    continue;
                }
                let watching: Vec<u32> = table
                    .by_wd
                    .get(&event.wd())
                    .map(|ids| ids.iter().copied().collect())
                    .unwrap_or_default();
                let name = event
                    .file_name()
                    .and_then(|name| name.to_str().ok())
                    .filter(|name| !name.is_empty())
                    .map(String::from);
                for id in watching {
                    if table.coalesced.contains_key(&id) {
                        ids.insert(id);
                    } else if let Some(typed) = table.typed.get_mut(&id) {
                        let ended = said.iter().any(|(said, _)| *said == id);
                        if let Some(event) = typed_event(typed, name.clone(), mask) {
                            if !ended {
                                said.push((id, event));
                            }
                        }
                    }
                }
                if event.events().contains(ReadFlags::IGNORED) {
                    // The folder went and the kernel dropped its watch; its watches stay, told.
                    table.by_wd.remove(&event.wd());
                }
            }
            // A typed watch whose item went is said at once and ends.
            for (id, _) in &said {
                if let Some(typed) = table.typed.remove(id) {
                    if table.release(typed.wd, *id) {
                        let _ = inotify::remove_watch(inotify, typed.wd);
                    }
                }
            }
        }
        for (watch, event) in said {
            changed(Change::Events {
                watch,
                events: vec![event],
                ended: true,
            });
        }
        if ready.get(2).is_some_and(|flags| !flags.is_empty()) {
            if let Some(file) = mounts.as_mut() {
                table_text.clear();
                let _ = file.rewind();
                let _ = file.read_to_end(&mut table_text);
            }
            let table = table.lock().expect("lock");
            ids.extend(
                table
                    .coalesced
                    .iter()
                    .filter(|(_, (_, mounts))| *mounts)
                    .map(|(id, _)| *id),
            );
        }
        if !ids.is_empty() {
            changed(Change::Watches(ids.into_iter().collect()));
        }
        // Typed watches whose debounce ran out say what they gathered.
        let now = Instant::now();
        let flushed: Vec<(u32, Vec<Event>)> = {
            let mut table = table.lock().expect("lock");
            table
                .typed
                .iter_mut()
                .filter(|(_, typed)| typed.due.is_some_and(|due| due <= now))
                .map(|(id, typed)| (*id, flush(typed)))
                .collect()
        };
        for (watch, events) in flushed {
            if !events.is_empty() {
                changed(Change::Events {
                    watch,
                    events,
                    ended: false,
                });
            }
        }
    }
}

/// Takes one of inotify's events into a typed watch. The watched item deleted or moved is the
/// event said at once (and the watch ends); anything else is gathered, its debounce started
/// again.
fn typed_event(typed: &mut Typed, name: Option<String>, mask: u32) -> Option<Event> {
    let flags = mask & FLAGS;
    if flags & (IN_DELETE_SELF | IN_MOVE_SELF) != 0 {
        return Some(Event {
            kind: if flags & IN_DELETE_SELF != 0 {
                Kind::Deleted
            } else {
                Kind::Renamed
            },
            path: typed.path.clone(),
            directory: typed.directory,
            flags,
            new_path: None,
        });
    }
    if flags & TYPED_EVENTS.bits() == 0 {
        // Another watch's events on the same folder (inotify's mask is all of theirs).
        return None;
    }
    let directory = if mask & IN_ISDIR != 0 {
        true
    } else {
        match &name {
            Some(name) => typed.path.join(name).is_dir(),
            None => typed.directory,
        }
    };
    if flags & IN_MOVED_FROM != 0 {
        if let Some(name) = &name {
            typed.moved_from.push((name.clone(), directory));
        }
    }
    typed.pending.push((name, flags, directory));
    typed.due = Some(Instant::now() + typed.debounce);
    None
}

/// What a typed watch gathered, said as Android's `FileSystemWatcher` says it: each item's events
/// merged, moves out paired in order with moves in as `renamed`, the rest each item's types.
fn flush(typed: &mut Typed) -> Vec<Event> {
    typed.due = None;
    let pending = std::mem::take(&mut typed.pending);
    let moved_from = std::mem::take(&mut typed.moved_from);
    // Each item's events merged, in the order first seen.
    let mut merged: Vec<(Option<String>, u32, bool)> = Vec::new();
    for (name, flags, directory) in pending {
        match merged.iter_mut().find(|(seen, _, _)| *seen == name) {
            Some((_, merged_flags, merged_directory)) => {
                *merged_flags |= flags;
                *merged_directory |= directory;
            }
            None => merged.push((name, flags, directory)),
        }
    }
    let path = |name: &Option<String>| match name {
        Some(name) => typed.path.join(name),
        None => typed.path.clone(),
    };
    let flags_of = |name: &str| {
        merged
            .iter()
            .find(|(seen, _, _)| seen.as_deref() == Some(name))
            .map_or(0, |(_, flags, _)| *flags)
    };
    let moved_to: Vec<(String, bool)> = merged
        .iter()
        .filter(|(_, flags, _)| flags & IN_MOVED_TO != 0)
        .filter_map(|(name, _, directory)| name.clone().map(|name| (name, *directory)))
        .collect();
    let mut events = Vec::new();
    let mut paired = BTreeSet::new();
    for ((from, from_directory), (to, to_directory)) in moved_from.iter().zip(&moved_to) {
        events.push(Event {
            kind: Kind::Renamed,
            path: typed.path.join(from),
            directory: *from_directory,
            flags: flags_of(from) | flags_of(to),
            new_path: Some((typed.path.join(to), *to_directory)),
        });
        paired.insert(from.clone());
        paired.insert(to.clone());
    }
    for (name, flags, directory) in &merged {
        if name.as_ref().is_some_and(|name| paired.contains(name)) {
            continue;
        }
        let mut kinds = Vec::new();
        if flags & IN_CREATE != 0 {
            kinds.push(Kind::Created);
        }
        if flags & IN_MODIFY != 0 {
            kinds.push(Kind::Modified);
        }
        if flags & IN_DELETE != 0 {
            kinds.push(Kind::Deleted);
        }
        if flags & (IN_MOVED_FROM | IN_MOVED_TO) != 0 {
            kinds.push(Kind::Renamed);
        }
        if kinds.is_empty() {
            kinds.push(Kind::Modified);
        }
        for kind in kinds {
            events.push(Event {
                kind,
                path: path(name),
                directory: *directory,
                flags: *flags,
                new_path: None,
            });
        }
    }
    events
}
