//! The dock's Trash, for the program running the compositor (docs/adr/0034-the-dock.mdx): how many
//! items the user's Trashes hold (freedesktop.org's, which every file manager shares,
//! docs/adr/0028-file-access.mdx), told to the shell as they change, and emptying them when the
//! person asks from the dock and accepts the system sheet that asks them. The Trash is the
//! platform's: the session empties it; what an application keeps in a Trash of its own (Oxy Files')
//! is the application's.
//!
//! **Empty Trash Automatically** (docs/adr/0059-storage.mdx, decision 5): while the person has it
//! on, the session erases what has been in the Trash for more than 30 days, by each item's
//! deletion date. It looks when the session starts, when the person turns it on, when the Trash
//! changes, and when the earliest item left reaches its 30 days: the turn waits no longer than
//! that ([`Trash::deadline`]), and nothing is read on a clock.

use oxyos_compositor_core::{Scene, TrashRequest};
use oxyos_files::trash;
use oxyos_files::watch::Watcher;
use oxyos_files::Scope;
use oxyos_protocol::SystemSound;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// What the system sheet that asks says, as the shell reads it (apps/shell/src/sheet.ts).
fn sheet_text(items: usize) -> String {
    serde_json::json!({ "kind": "empty-trash", "items": items }).to_string()
}

pub struct Trash {
    scope: Scope,
    watcher: Watcher,
    /// The watch on the Trashes' folders and the mount table, and the folders it watches.
    watch: Option<(u32, Vec<PathBuf>)>,
    /// Set from the watcher's thread when anything it watches changed.
    changed: Arc<AtomicBool>,
    /// Readable once the watcher's thread said something changed: wakes the turn.
    wake: UnixStream,
    /// The sheet asking the person to empty it, while it is shown.
    sheet: Option<u32>,
    /// *Empty Trash Automatically*, as the session's preferences say.
    empty_automatically: bool,
    /// The Trash is to be looked at for what has been in it too long, at the next turn: the
    /// person turned it on.
    sweep_asked: bool,
    /// When the earliest item left reaches its time (milliseconds since 1970), while it is on.
    next_due: Option<i64>,
    /// How many times the Trash was looked at for what has been in it too long (the tests' count
    /// of the work done).
    sweeps: u32,
}

/// Now, in milliseconds since 1970.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}

impl Trash {
    /// Follows the Trashes of `scope`'s user. The home's is made if it is not there (with its
    /// folders, private to the user), so that it can be watched; the program adds [`Trash::wake_fd`]
    /// to the compositor's wakers.
    pub fn new(scope: Scope) -> io::Result<Self> {
        trash::home_trash(&scope)
            .make()
            .map_err(|error| io::Error::other(format!("the home's Trash: {error:?}")))?;
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let changed = Arc::new(AtomicBool::new(true));
        let flag = changed.clone();
        let watcher = Watcher::new(scope.mountinfo(), move |_| {
            flag.store(true, Ordering::Release);
            let _ = (&woken).write_all_nonblocking();
        })?;
        Ok(Self {
            scope,
            watcher,
            watch: None,
            changed,
            wake,
            sheet: None,
            empty_automatically: false,
            sweep_asked: false,
            next_due: None,
            sweeps: 0,
        })
    }

    /// *Empty Trash Automatically* as the person has it (the `empty_trash_automatically`
    /// preference): turned on, the Trash is looked at in the next turn.
    pub fn set_empty_automatically(&mut self, on: bool) {
        if on == self.empty_automatically {
            return;
        }
        self.empty_automatically = on;
        self.sweep_asked = on;
        if !on {
            self.next_due = None;
        }
    }

    /// When the earliest item in the Trash will have been there 30 days, while the Trash empties
    /// itself: the program's turn waits no longer than that.
    pub fn deadline(&self) -> Option<SystemTime> {
        let due = self.next_due?;
        Some(UNIX_EPOCH + Duration::from_millis(u64::try_from(due).unwrap_or(0)))
    }

    /// A descriptor readable when the Trash may have changed: only to wake the turn.
    pub fn wake_fd(&self) -> io::Result<OwnedFd> {
        Ok(self.wake.try_clone()?.into())
    }

    /// One turn: the Trashes counted again if they changed (the shell told), the shell's requests
    /// to empty them asked of the person, and the person's answer acted on.
    pub fn serve(&mut self, scene: &mut Scene) {
        let mut drained = [0u8; 64];
        while matches!((&self.wake).read(&mut drained), Ok(n) if n > 0) {}
        let changed = self.changed.swap(false, Ordering::AcqRel);
        if changed {
            self.follow();
            self.tell(scene);
        }
        let due = self.next_due.is_some_and(|due| due <= now_ms());
        if self.empty_automatically && (changed || self.sweep_asked || due) {
            self.sweep(scene);
        }
        for with_input in scene.take_trash_empties() {
            self.ask(scene, with_input);
        }
        for request in scene.take_trash_files() {
            self.put(scene, &request);
        }
        if let Some(accepted) = self.sheet.and_then(|sheet| scene.take_sheet_answer(sheet)) {
            self.answered(scene, accepted);
        }
    }

    fn tell(&self, scene: &mut Scene) {
        scene.set_trash_items(trash::count(&self.scope).try_into().unwrap_or(u32::MAX));
    }

    /// The shell asked to empty the Trash: only the person's own press asks them, one sheet at a
    /// time, and an empty Trash asks nothing.
    fn ask(&mut self, scene: &mut Scene, with_input: bool) {
        let items = trash::count(&self.scope);
        if !with_input || self.sheet.is_some() || items == 0 {
            return;
        }
        self.sheet = scene.show_sheet(None, &sheet_text(items));
    }

    /// The shell asked to put files in the Trash, the files dropped on the dock's (ADR 0035,
    /// decision 12): only with the person's own drop, each `file://` file the person's scope
    /// reaches, as a file manager's Move to Trash; any other is passed over and said.
    fn put(&mut self, scene: &mut Scene, request: &TrashRequest) {
        if !request.with_input {
            eprintln!("[trash] files not trashed: the request cited no input of the shell's");
            return;
        }
        let mut moved = 0;
        for uri in &request.uris {
            let outcome = oxyos_files::uri::path_of(uri)
                .and_then(|path| self.scope.item(&path))
                .and_then(|path| trash::trash(&self.scope, &path));
            match outcome {
                Ok(_) => moved += 1,
                Err(error) => eprintln!("[trash] {uri} not trashed: {error:?}"),
            }
        }
        if moved > 0 {
            eprintln!("[trash] trashed: {moved} items");
            // The Trash's sound, once for the drop (docs/adr/0044-system-sounds.mdx).
            scene.request_system_sound(SystemSound::MoveToTrash);
            self.tell(scene);
        }
    }

    /// The person answered the sheet: accepted, every Trash is emptied for good.
    fn answered(&mut self, scene: &mut Scene, accepted: bool) {
        self.sheet = None;
        if accepted {
            match trash::empty(&self.scope) {
                Ok(removed) => {
                    eprintln!("[trash] emptied: {removed} items");
                    // Emptied for good: its sound, once (docs/adr/0044-system-sounds.mdx).
                    scene.request_system_sound(SystemSound::EmptyTrash);
                }
                Err(error) => eprintln!("[trash] not emptied: {error:?}"),
            }
            self.tell(scene);
        }
    }

    /// Erases what has been in the Trash for more than 30 days, and notes when the next item will
    /// have been: the shell is told what is left. Silent, as a Mac's is.
    fn sweep(&mut self, scene: &mut Scene) {
        self.sweep_asked = false;
        self.sweeps += 1;
        match trash::erase_older(&self.scope, now_ms() - trash::KEPT_FOR_MS) {
            Ok(swept) => {
                self.next_due = swept.next.map(|deleted| deleted + trash::KEPT_FOR_MS);
                if swept.erased > 0 {
                    eprintln!(
                        "[trash] erased {} items in the Trash for more than 30 days",
                        swept.erased
                    );
                    self.tell(scene);
                }
            }
            Err(error) => eprintln!("[trash] not looked at for old items: {error:?}"),
        }
    }

    /// Watches every Trash the user has now (a volume mounted or gone changes which), and the
    /// mount table.
    fn follow(&mut self) {
        let folders: Vec<PathBuf> = trash::trashes(&self.scope)
            .iter()
            .map(|trash| trash.files())
            .filter(|folder| folder.is_dir())
            .collect();
        if self
            .watch
            .as_ref()
            .is_some_and(|(_, watched)| *watched == folders)
        {
            return;
        }
        if let Some((id, _)) = self.watch.take() {
            self.watcher.unwatch(id);
        }
        match self.watcher.watch(&folders, true) {
            Ok(id) => self.watch = Some((id, folders)),
            Err(error) => eprintln!("[trash] not watched: {error}"),
        }
    }
}

/// A byte written to a non-blocking socket, if it has room: a wake needs no more than one waiting.
trait WakeWrite {
    fn write_all_nonblocking(self) -> io::Result<()>;
}

impl WakeWrite for &UnixStream {
    fn write_all_nonblocking(self) -> io::Result<()> {
        use std::io::Write;
        let mut stream = self;
        match stream.write(&[1]) {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_compositor_core::{ClientId, Grants};
    use oxyos_protocol::Event;
    use std::time::{Duration, Instant};

    const SHELL: ClientId = 3;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "oxyos-compositor-trash-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn told(scene: &mut Scene) -> Vec<Event> {
        scene
            .drain_events()
            .into_iter()
            .filter(|(to, _)| *to == SHELL)
            .map(|(_, event)| event)
            .collect()
    }

    #[test]
    fn the_shell_is_told_what_the_trash_holds_and_it_empties_only_once_the_person_accepts() {
        let home = scratch("home");
        let mountinfo = home.join("mountinfo");
        std::fs::write(&mountinfo, "").unwrap();
        let scope = Scope::new(
            &home,
            &home.join(".local/share"),
            &home.join(".config"),
            Vec::new(),
            mountinfo,
            rustix::process::getuid().as_raw(),
        )
        .unwrap();
        let mut scene = Scene::new();
        scene
            .connect(
                SHELL,
                Grants {
                    shell: true,
                    ..Grants::default()
                },
            )
            .unwrap();
        let mut follower = Trash::new(scope.clone()).unwrap();
        follower.serve(&mut scene);
        assert!(told(&mut scene).contains(&Event::TrashItems { items: 0 }));
        // A file manager puts an item in it: the shell is told, once the watcher saw it.
        let item = home.join("report.txt");
        std::fs::write(&item, "draft").unwrap();
        trash::trash(&scope, &item).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            follower.serve(&mut scene);
            if told(&mut scene).contains(&Event::TrashItems { items: 1 }) {
                break;
            }
            assert!(Instant::now() < deadline, "the shell was not told");
            std::thread::sleep(Duration::from_millis(20));
        }
        // A request citing no input asks nothing (the scene says which did, scene/dock.rs).
        follower.ask(&mut scene, false);
        assert_eq!(follower.sheet, None);
        // The person's: a sheet asks, saying how many items go; refused, nothing goes.
        let sheet_texts = |scene: &mut Scene| -> String {
            told(scene)
                .iter()
                .filter_map(|event| match event {
                    Event::SheetText { text, .. } => Some(text.as_str().to_string()),
                    _ => None,
                })
                .collect()
        };
        follower.ask(&mut scene, true);
        assert!(follower.sheet.is_some());
        assert_eq!(
            sheet_texts(&mut scene),
            r#"{"items":1,"kind":"empty-trash"}"#
        );
        // One at a time.
        follower.ask(&mut scene, true);
        assert!(sheet_texts(&mut scene).is_empty());
        follower.answered(&mut scene, false);
        assert_eq!(trash::count(&scope), 1);
        // Refused: no sound.
        assert!(scene.take_system_sounds().is_empty());
        // Accepted, it is emptied for good and the shell told.
        follower.ask(&mut scene, true);
        follower.answered(&mut scene, true);
        assert_eq!(trash::count(&scope), 0);
        // Emptied: the empty sound, once.
        assert_eq!(scene.take_system_sounds(), [SystemSound::EmptyTrash]);
        assert!(told(&mut scene).contains(&Event::TrashItems { items: 0 }));
        // An empty Trash asks nothing.
        follower.ask(&mut scene, true);
        assert_eq!(follower.sheet, None);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// `name` put in the home's Trash `days` ago, as a file manager would have: its info's
    /// deletion date in local time.
    fn trashed_days_ago(home: &std::path::Path, name: &str, days: i64) {
        let trash = home.join(".local/share/Trash");
        std::fs::create_dir_all(trash.join("info")).unwrap();
        std::fs::create_dir_all(trash.join("files")).unwrap();
        let deleted = jiff::Zoned::now()
            .checked_sub(jiff::SignedDuration::from_hours(days * 24))
            .unwrap();
        std::fs::write(
            trash.join(format!("info/{name}.trashinfo")),
            format!(
                "[Trash Info]\nPath={}/{name}\nDeletionDate={}\n",
                home.display(),
                deleted.strftime("%Y-%m-%dT%H:%M:%S")
            ),
        )
        .unwrap();
        std::fs::write(trash.join(format!("files/{name}")), name).unwrap();
    }

    #[test]
    fn once_turned_on_the_trash_erases_what_has_been_in_it_30_days_and_waits_for_the_next() {
        let home = scratch("automatic");
        let mountinfo = home.join("mountinfo");
        std::fs::write(&mountinfo, "").unwrap();
        let scope = Scope::new(
            &home,
            &home.join(".local/share"),
            &home.join(".config"),
            Vec::new(),
            mountinfo,
            rustix::process::getuid().as_raw(),
        )
        .unwrap();
        let mut scene = Scene::new();
        scene
            .connect(
                SHELL,
                Grants {
                    shell: true,
                    ..Grants::default()
                },
            )
            .unwrap();
        trashed_days_ago(&home, "old.txt", 40);
        trashed_days_ago(&home, "recent.txt", 10);
        let mut follower = Trash::new(scope.clone()).unwrap();
        follower.serve(&mut scene);
        // Off, as a Mac ships: nothing goes, and nothing is due.
        assert_eq!(trash::count(&scope), 2);
        assert_eq!(follower.deadline(), None);
        assert_eq!(follower.sweeps, 0);
        assert!(told(&mut scene).contains(&Event::TrashItems { items: 2 }));
        // Turned on: the item there 40 days goes at the next turn, silently, the shell told.
        follower.set_empty_automatically(true);
        follower.serve(&mut scene);
        assert_eq!(trash::count(&scope), 1);
        assert!(!home.join(".local/share/Trash/files/old.txt").exists());
        assert!(scene.take_system_sounds().is_empty());
        assert!(told(&mut scene).contains(&Event::TrashItems { items: 1 }));
        // The next is due when the one put in ten days ago has been there 30: in 20 days.
        let in_20_days = SystemTime::now() + Duration::from_secs(20 * 24 * 60 * 60);
        let due = follower.deadline().expect("a deadline");
        let off_by = due
            .duration_since(in_20_days)
            .unwrap_or_else(|early| early.duration());
        assert!(off_by < Duration::from_secs(5), "{off_by:?}");
        // The erase is a change of the Trash, looked at once more; then, while nothing changes,
        // nothing is read again, however many turns run.
        std::thread::sleep(Duration::from_millis(300));
        follower.serve(&mut scene);
        let sweeps = follower.sweeps;
        for _ in 0..5 {
            std::thread::sleep(Duration::from_millis(50));
            follower.serve(&mut scene);
        }
        assert_eq!(follower.sweeps, sweeps);
        assert_eq!(trash::count(&scope), 1);
        // Off again: nothing is due, and an old item stays.
        follower.set_empty_automatically(false);
        assert_eq!(follower.deadline(), None);
        trashed_days_ago(&home, "older.txt", 90);
        std::thread::sleep(Duration::from_millis(300));
        follower.serve(&mut scene);
        assert_eq!(trash::count(&scope), 2);
        assert_eq!(follower.sweeps, sweeps);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn files_dropped_on_the_trash_go_into_it_only_with_the_shells_drop() {
        let home = scratch("drop");
        let mountinfo = home.join("mountinfo");
        std::fs::write(&mountinfo, "").unwrap();
        let scope = Scope::new(
            &home,
            &home.join(".local/share"),
            &home.join(".config"),
            Vec::new(),
            mountinfo,
            rustix::process::getuid().as_raw(),
        )
        .unwrap();
        let mut scene = Scene::new();
        scene
            .connect(
                SHELL,
                Grants {
                    shell: true,
                    ..Grants::default()
                },
            )
            .unwrap();
        let mut follower = Trash::new(scope.clone()).unwrap();
        follower.serve(&mut scene);
        told(&mut scene);
        let item = home.join("dropped.txt");
        std::fs::write(&item, "dragged").unwrap();
        let uri = oxyos_files::uri::of(&item, false);
        // Without the shell's drop, nothing moves.
        follower.put(
            &mut scene,
            &TrashRequest {
                uris: vec![uri.clone()],
                with_input: false,
            },
        );
        assert!(item.exists());
        assert!(scene.take_system_sounds().is_empty());
        // With it, the file is in the Trash and the shell told; a URI out of the scope and one
        // gone are passed over.
        follower.put(
            &mut scene,
            &TrashRequest {
                uris: vec![
                    uri,
                    "file:///etc/hostname".into(),
                    oxyos_files::uri::of(&home.join("gone.txt"), false),
                ],
                with_input: true,
            },
        );
        assert!(!item.exists());
        assert_eq!(trash::count(&scope), 1);
        // One sound for the drop, whatever it held.
        assert_eq!(scene.take_system_sounds(), [SystemSound::MoveToTrash]);
        assert!(told(&mut scene).contains(&Event::TrashItems { items: 1 }));
        let _ = std::fs::remove_dir_all(&home);
    }
}
