//! The dock's Trash, for the program running the compositor (docs/adr/0034-the-dock.mdx): how many
//! items the user's Trashes hold (freedesktop.org's, which every file manager shares,
//! docs/adr/0028-file-access.mdx), told to the shell as they change, and emptying them when the
//! person asks from the dock and accepts the system sheet that asks them. The Trash is the
//! platform's: the session empties it; what an application keeps in a Trash of its own (Oxy Files')
//! is the application's.

use oxyos_compositor_core::{Scene, TrashRequest};
use oxyos_files::trash;
use oxyos_files::watch::Watcher;
use oxyos_files::Scope;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
        })
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
        if self.changed.swap(false, Ordering::AcqRel) {
            self.follow();
            self.tell(scene);
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
            self.tell(scene);
        }
    }

    /// The person answered the sheet: accepted, every Trash is emptied for good.
    fn answered(&mut self, scene: &mut Scene, accepted: bool) {
        self.sheet = None;
        if accepted {
            match trash::empty(&self.scope) {
                Ok(removed) => eprintln!("[trash] emptied: {removed} items"),
                Err(error) => eprintln!("[trash] not emptied: {error:?}"),
            }
            self.tell(scene);
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
        // Accepted, it is emptied for good and the shell told.
        follower.ask(&mut scene, true);
        follower.answered(&mut scene, true);
        assert_eq!(trash::count(&scope), 0);
        assert!(told(&mut scene).contains(&Event::TrashItems { items: 0 }));
        // An empty Trash asks nothing.
        follower.ask(&mut scene, true);
        assert_eq!(follower.sheet, None);
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
        assert!(told(&mut scene).contains(&Event::TrashItems { items: 1 }));
        let _ = std::fs::remove_dir_all(&home);
    }
}
