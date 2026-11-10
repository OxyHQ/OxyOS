//! Windows coming back where they were (docs/adr/0070-windows-come-back-where-they-were.mdx), as a
//! Mac's do: the person's record of where each application's windows were, and how many desktops
//! they had, kept by the program running the compositor (the session, and the development
//! compositor alike) and given to the scene as windows open.
//!
//! - **The record** is the person's (`$XDG_STATE_HOME/oxyos/windows`, JSON, private): the desktops,
//!   and every window kept, back to front, each its application's, its frame, whether it is
//!   zoomed, its desktop and whether it is minimized. It follows the windows as they change and
//!   is written whole a second after a change ([`WRITE_AFTER`]), never on the turn that changed
//!   it, and at once as the session ends ([`Places::flush`]). An application's windows are kept
//!   as they were when its last one went (it quit, crashed or was ended with the session), so
//!   they come back the same; a window the person closes while others of its application stay
//!   open is forgotten.
//! - **Where a new window goes**: an application's n-th window opened since it had none takes the
//!   n-th of its windows kept as its first one opened: its frame, zoomed if it was. What the session opens as the person logs in (their login
//!   items, what reopens) also takes its desktop, its minimized state and its place in the order
//!   of the windows opened with it ([`Places::opened_at_login`]); what the person opens later
//!   opens in front, on the desktop they are on.
//! - **The desktops** the person had come back as the session starts ([`Places::start`]).
//! - **Reset** (ADR 0061) forgets an application's windows: they are its own state
//!   ([`Places::forget`]).

use oxyos_compositor_core::scene::{Placing, Scene, WindowPlace};
use oxyos_compositor_core::SurfaceKey;
use oxyos_protocol::Size;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long after a change the record is written: a window dragged writes once, as it stops.
pub const WRITE_AFTER: Duration = Duration::from_secs(1);
/// The most windows kept: past it, those of applications not open go, the back-most first.
pub const MAX_KEPT: usize = 64;

/// One window kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    application: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    zoomed: bool,
    #[serde(default)]
    desktop: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    minimized: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

impl Kept {
    fn of(application: &str, place: &WindowPlace) -> Self {
        Self {
            application: application.to_owned(),
            x: place.origin.0,
            y: place.origin.1,
            width: place.size.width,
            height: place.size.height,
            zoomed: place.zoomed,
            desktop: place.desktop,
            minimized: place.minimized,
        }
    }

    fn place(&self) -> WindowPlace {
        WindowPlace {
            origin: (self.x, self.y),
            size: Size::new(self.width.max(1), self.height.max(1)),
            zoomed: self.zoomed,
            desktop: self.desktop,
            minimized: self.minimized,
        }
    }
}

/// The person's record.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    #[serde(default = "one")]
    desktops: u32,
    /// Back to front.
    #[serde(default)]
    windows: Vec<Kept>,
}

fn one() -> u32 {
    1
}

/// The record's file, from the environment: the person's state folder.
pub fn path() -> Option<PathBuf> {
    let state = match std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        Some(state) => state,
        None => PathBuf::from(std::env::var_os("HOME")?)
            .join(".local")
            .join("state"),
    };
    Some(state.join("oxyos").join("windows"))
}

/// The part (see the module's documentation).
#[derive(Debug)]
pub struct Places {
    /// None: nothing is kept (the login screen, whose user is no one).
    path: Option<PathBuf>,
    record: Record,
    /// The order as the session started: what opens at login takes its place in it.
    started: Vec<Kept>,
    write_at: Option<Instant>,
    /// Applications the session opened at login whose windows have not opened yet: their first
    /// opening is placed whole.
    at_login: BTreeSet<String>,
    /// Windows offered and their applications, while they are open.
    placed: BTreeMap<SurfaceKey, String>,
    /// Each application opening windows since it had none.
    opening: BTreeMap<String, Opening>,
}

/// An application opening windows since it had none.
#[derive(Debug)]
struct Opening {
    /// Its windows as kept when its first one opened.
    kept: Vec<Kept>,
    /// How many it opened since.
    opened: usize,
    /// Opened by the session at login: placed whole.
    at_login: bool,
}

impl Places {
    /// The record at `path`, read once; one that cannot be read is none.
    pub fn at(path: Option<PathBuf>) -> Self {
        let record = path
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Record>(&bytes).ok())
            .unwrap_or(Record {
                desktops: 1,
                windows: Vec::new(),
            });
        Self {
            path,
            started: record.windows.clone(),
            record,
            write_at: None,
            at_login: BTreeSet::new(),
            placed: BTreeMap::new(),
            opening: BTreeMap::new(),
        }
    }

    /// The person's own, from the environment.
    pub fn of_user() -> Self {
        Self::at(path())
    }

    /// As the session starts, before any window opens: the desktops the person had.
    pub fn start(&self, scene: &mut Scene) {
        scene.restore_desktops(self.record.desktops);
    }

    /// The session opens `application` as the person logs in: its windows are placed whole.
    pub fn opened_at_login(&mut self, application: &str) {
        self.at_login.insert(application.to_owned());
    }

    /// An application reset: its windows are forgotten.
    pub fn forget(&mut self, application: &str, now: Instant) {
        let before = self.record.windows.len();
        self.record
            .windows
            .retain(|kept| kept.application != application);
        self.started.retain(|kept| kept.application != application);
        self.opening.remove(application);
        if self.record.windows.len() != before {
            self.write_at.get_or_insert(now + WRITE_AFTER);
        }
    }

    /// A turn at `now`: new windows placed, the record following the windows, written when due.
    pub fn serve(&mut self, scene: &mut Scene, now: Instant) {
        self.placed
            .retain(|key, _| scene.window_workspace(*key).is_some());
        for (key, application) in scene.take_windows_to_place() {
            // Its first window open: what is kept of its windows now is what they come back to,
            // however the record follows them as they open; opened by the session at login, this
            // opening alone is placed whole.
            if !self.placed.values().any(|placed| *placed == application) {
                let kept = self
                    .record
                    .windows
                    .iter()
                    .filter(|kept| kept.application == application)
                    .cloned()
                    .collect();
                let at_login = self.at_login.remove(&application);
                self.opening.insert(
                    application.clone(),
                    Opening {
                        kept,
                        opened: 0,
                        at_login,
                    },
                );
            }
            self.placed.insert(key, application.clone());
            let Some(opening) = self.opening.get_mut(&application) else {
                continue;
            };
            let nth = opening.opened;
            opening.opened += 1;
            let at_login = opening.at_login;
            let Some(place) = opening.kept.get(nth).map(Kept::place) else {
                continue;
            };
            let at_login = at_login
                .then(|| {
                    self.started
                        .iter()
                        .enumerate()
                        .filter(|(_, started)| started.application == application)
                        .nth(nth)
                        .and_then(|(index, _)| u32::try_from(index).ok())
                })
                .flatten();
            scene.place_window(key, Placing { place, at_login });
        }
        let live: Vec<Kept> = scene
            .launched_windows()
            .iter()
            .map(|window| Kept::of(&window.application, &window.place))
            .collect();
        let record = Record {
            desktops: scene.workspaces().0,
            windows: follow(&self.record.windows, &live),
        };
        if record != self.record {
            self.record = record;
            self.write_at.get_or_insert(now + WRITE_AFTER);
        }
        if self.write_at.is_some_and(|at| now >= at) {
            self.flush();
        }
    }

    /// When the record is due to be written, for the turn to wake then.
    pub fn deadline(&self) -> Option<Instant> {
        self.write_at
    }

    /// Writes the record now if it changed: as the session ends, and when due.
    pub fn flush(&mut self) {
        if self.write_at.take().is_none() {
            return;
        }
        let Some(path) = &self.path else {
            return;
        };
        if let Err(error) = write(path, &self.record) {
            eprintln!("[places] {}: {error}", path.display());
        }
    }
}

/// The record's windows following those open now (`live`, back to front): each open
/// application's windows take its places in the order, in the order they are in now, and those
/// of the applications not open stay where they were. More windows than places go in front; fewer,
/// the places left go (the person closed a window). At most [`MAX_KEPT`].
fn follow(kept: &[Kept], live: &[Kept]) -> Vec<Kept> {
    let open: BTreeSet<&str> = live.iter().map(|w| w.application.as_str()).collect();
    let mut live = live.iter();
    let mut windows: Vec<Kept> = Vec::with_capacity(kept.len());
    for window in kept {
        if open.contains(window.application.as_str()) {
            windows.extend(live.next().cloned());
        } else {
            windows.push(window.clone());
        }
    }
    windows.extend(live.cloned());
    while windows.len() > MAX_KEPT {
        match windows
            .iter()
            .position(|window| !open.contains(window.application.as_str()))
        {
            Some(index) => windows.remove(index),
            None => windows.remove(0),
        };
    }
    windows
}

/// Written whole beside it and renamed over it: never half of one. Private: where the person's
/// windows were is theirs.
fn write(path: &Path, record: &Record) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let text = serde_json::to_vec(record).map_err(std::io::Error::other)?;
    let beside = path.with_extension("new");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&beside)?;
    file.write_all(&text)?;
    drop(file);
    std::fs::rename(&beside, path)
}

#[cfg(test)]
mod tests;
