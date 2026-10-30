//! Changes an application watches (docs/adr/0027-files.mdx, decision 6): a folder's items, the
//! Trash's, and the volumes mounted.
//!
//! A watch is a number. A folder's is inotify's on the folder (an item created, removed, moved in
//! or out, written and closed, or its attributes changed; the folder itself removed or moved);
//! the volumes' is the mount table's (`/proc/self/mountinfo` is readable as urgent data when it
//! changes, as `poll(2)` documents). One thread waits on both and says which watches changed,
//! each once per wake, whatever many events it read: the application reads the folder again,
//! so a burst of changes is one reading.

use rustix::event::{poll, PollFd, PollFlags};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use std::collections::{BTreeSet, HashMap};
use std::io::{self, Read, Seek, Write};
use std::mem::MaybeUninit;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// What a folder's watch is told of.
const FOLDER_EVENTS: WatchFlags = WatchFlags::CREATE
    .union(WatchFlags::DELETE)
    .union(WatchFlags::MOVED_FROM)
    .union(WatchFlags::MOVED_TO)
    .union(WatchFlags::CLOSE_WRITE)
    .union(WatchFlags::ATTRIB)
    .union(WatchFlags::DELETE_SELF)
    .union(WatchFlags::MOVE_SELF)
    .union(WatchFlags::ONLYDIR);

#[derive(Default)]
struct Table {
    next: u32,
    /// Each watch's inotify descriptors, and whether it follows the mount table.
    watches: HashMap<u32, (Vec<i32>, bool)>,
    /// Which watches each inotify descriptor belongs to (one folder watched twice is one
    /// descriptor: inotify gives the same one for the same folder).
    by_wd: HashMap<i32, BTreeSet<u32>>,
}

/// Watches, and the thread that says when they change.
pub struct Watcher {
    inotify: Arc<OwnedFd>,
    table: Arc<Mutex<Table>>,
    stop: UnixStream,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    /// A watcher whose thread calls `changed` with the watches that changed, from its own thread;
    /// `mountinfo` is the mount table it follows.
    pub fn new(
        mountinfo: &Path,
        changed: impl Fn(Vec<u32>) + Send + 'static,
    ) -> io::Result<Watcher> {
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

    /// A new watch on `folders` (each must be a folder) and, if `mounts`, on the mount table.
    pub fn watch(&self, folders: &[PathBuf], mounts: bool) -> io::Result<u32> {
        let mut wds = Vec::new();
        for folder in folders {
            match inotify::add_watch(&*self.inotify, folder.as_path(), FOLDER_EVENTS) {
                Ok(wd) => wds.push(wd),
                Err(errno) => {
                    let table = self.table.lock().expect("lock");
                    for wd in wds {
                        if !table.by_wd.contains_key(&wd) {
                            let _ = inotify::remove_watch(&*self.inotify, wd);
                        }
                    }
                    drop(table);
                    return Err(errno.into());
                }
            }
        }
        let mut table = self.table.lock().expect("lock");
        table.next += 1;
        let id = table.next;
        for &wd in &wds {
            table.by_wd.entry(wd).or_default().insert(id);
        }
        table.watches.insert(id, (wds, mounts));
        Ok(id)
    }

    /// Ends watch `id`; nothing is said of it again.
    pub fn unwatch(&self, id: u32) {
        let mut table = self.table.lock().expect("lock");
        let Some((wds, _)) = table.watches.remove(&id) else {
            return;
        };
        for wd in wds {
            if let Some(ids) = table.by_wd.get_mut(&wd) {
                ids.remove(&id);
                if ids.is_empty() {
                    table.by_wd.remove(&wd);
                    let _ = inotify::remove_watch(&*self.inotify, wd);
                }
            }
        }
    }

    /// How many watches there are.
    pub fn len(&self) -> usize {
        self.table.lock().expect("lock").watches.len()
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
    changed: &dyn Fn(Vec<u32>),
) {
    // The table is read once first: poll then reports only changes after this reading.
    let mut table_text = Vec::new();
    if let Some(file) = mounts.as_mut() {
        let _ = file.read_to_end(&mut table_text);
    }
    let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
    loop {
        let mut fds = vec![
            PollFd::new(inotify, PollFlags::IN),
            PollFd::from_borrowed_fd(stopped.as_fd(), PollFlags::IN),
        ];
        if let Some(file) = mounts.as_ref() {
            fds.push(PollFd::from_borrowed_fd(file.as_fd(), PollFlags::PRI));
        }
        if poll(&mut fds, None).is_err() {
            continue;
        }
        let ready: Vec<PollFlags> = fds.iter().map(PollFd::revents).collect();
        drop(fds);
        if !ready[1].is_empty() {
            return;
        }
        let mut ids = BTreeSet::new();
        if ready[0].contains(PollFlags::IN) {
            let mut reader = inotify::Reader::new(inotify, &mut buffer);
            let mut table = table.lock().expect("lock");
            while let Ok(event) = reader.next() {
                if event.events().contains(ReadFlags::QUEUE_OVERFLOW) {
                    // Events were lost: every folder is read again.
                    ids.extend(table.by_wd.values().flatten().copied());
                    continue;
                }
                if let Some(watching) = table.by_wd.get(&event.wd()) {
                    ids.extend(watching.iter().copied());
                }
                if event.events().contains(ReadFlags::IGNORED) {
                    // The folder went and the kernel dropped its watch; its watches stay, told.
                    table.by_wd.remove(&event.wd());
                }
            }
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
                    .watches
                    .iter()
                    .filter(|(_, (_, mounts))| *mounts)
                    .map(|(id, _)| *id),
            );
        }
        if !ids.is_empty() {
            changed(ids.into_iter().collect());
        }
    }
}
