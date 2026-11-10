//! Resume after any end of the session (docs/adr/0065-the-session-ending-because-of-a-problem.mdx,
//! decision 3): what ran is opened again at the next login whatever ended the session — the
//! person's Log Out, Restart or Shut Down, and also a crash of the session, its hang, a power cut
//! or a kernel panic — as a Mac's Resume brings back the same applications after a forced restart.
//!
//! The session keeps, in the person's state folder (`$XDG_STATE_HOME/oxyos/session`), written whole
//! as it changes:
//! - `running`: the applications it launched that run, by registry id, once each, in the order
//!   they were launched (a file chooser is no application of its own);
//! - `reopened`: those it opened again as it started;
//! - `steady`: it ran a minute.
//!
//! At the next login ([`Resume::start`]):
//! - the person's own end: ADR 0064's `reopen-at-login`, as before — what ran when they asked, if
//!   they kept *Reopen windows when logging back in* (its question writes the file, and removes it
//!   when the end is cancelled);
//! - any other end: what ran, if the person's latest choice of *Reopen windows* is on (a Mac's
//!   rule after a forced restart or a power cut: "the most recent choice applies");
//! - an end that was not asked for, within a minute of a start that reopened applications: the
//!   person is asked first ([`AtLogin::Ask`]), as a Mac asks "Do you want to try to reopen its
//!   windows again?", since what was reopened may be what ended it.
//!
//! Never at the login screen and never in Safe Mode (nothing is opened there).

use oxyos_apps::Launcher;
use oxyos_compositor_core::Scene;
use oxyos_preferences::Preferences;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A session that ran this long was no crash of its reopening.
pub const STEADY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    running: Vec<String>,
    #[serde(default)]
    reopened: Vec<String>,
    #[serde(default)]
    steady: bool,
}

/// What the session opens as it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtLogin {
    /// These, in order (none: nothing).
    Reopen(Vec<String>),
    /// These, once the person says so: the last session ended within a minute of reopening.
    Ask(Vec<String>),
}

/// The session's record of what runs.
#[derive(Debug)]
pub struct Resume {
    path: PathBuf,
    state: State,
    started: Instant,
    /// The person asked whether to reopen again: the sheet's number and what it would open.
    question: Option<(u32, Vec<String>)>,
}

/// The session's record in the person's state folder, from the environment.
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
    Some(state.join("oxyos").join("session"))
}

fn read(path: &Path) -> Option<State> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Once each, in order.
fn once_each(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for id in ids {
        if !id.is_empty() && !kept.contains(&id) {
            kept.push(id);
        }
    }
    kept
}

/// Opens `ids` again, each once, but those the person opens at login, which were opened already:
/// what was opened.
pub fn reopen(launcher: &mut Launcher, ids: &[String], preferences: &Preferences) -> Vec<String> {
    let mut opened = Vec::new();
    for id in ids {
        if preferences.open_at_login.contains(id) {
            continue;
        }
        match launcher.launch(id) {
            Ok(pid) => {
                eprintln!("[resume] reopened {id} at login ({pid})");
                opened.push(id.clone());
            }
            Err(error) => eprintln!("[resume] {id} not reopened at login: {error}"),
        }
    }
    opened
}

/// Asks the person whether to open `ids` again: the system's sheet `reopen-again`
/// (`{"kind": "reopen-again", "applications": [NAME]}`), drawn by the shell and answered by its
/// press alone, *Reopen* its acceptance and *Don't Reopen* its refusal. Its number, to take the
/// answer by; none if the scene has too many sheets asked already (nothing is reopened then).
pub fn ask(
    scene: &mut Scene,
    ids: &[String],
    name_of: &dyn Fn(&str) -> Option<String>,
) -> Option<u32> {
    let names: Vec<String> = ids
        .iter()
        .map(|id| name_of(id).unwrap_or_else(|| id.clone()))
        .collect();
    scene.show_sheet(
        None,
        &json!({ "kind": "reopen-again", "applications": names }).to_string(),
    )
}

impl Resume {
    /// The session starts at `now`: what the last one leaves to open, and the record of this one
    /// begun (nothing running yet). `reopen_file` is ADR 0064's, read and removed here;
    /// `reopen_windows` the person's latest choice.
    pub fn start(
        path: PathBuf,
        reopen_file: Option<&Path>,
        reopen_windows: bool,
        now: Instant,
    ) -> (Self, AtLogin) {
        let asked = reopen_file
            .filter(|file| file.exists())
            .map(crate::session_end::take_reopen);
        let at_login = match (asked, read(&path)) {
            // The person's own end: its question decided (ADR 0064).
            (Some(ids), _) => AtLogin::Reopen(ids),
            // No record: nothing is known to have run.
            (None, None) => AtLogin::Reopen(Vec::new()),
            // Any other end: what ran, as the person's latest choice says.
            (None, Some(last)) => {
                let running = once_each(if last.running.is_empty() {
                    last.reopened.clone()
                } else {
                    last.running.clone()
                });
                if !reopen_windows || running.is_empty() {
                    AtLogin::Reopen(Vec::new())
                } else if !last.steady && !last.reopened.is_empty() {
                    AtLogin::Ask(running)
                } else {
                    AtLogin::Reopen(running)
                }
            }
        };
        let resume = Self {
            path,
            state: State::default(),
            started: now,
            question: None,
        };
        resume.write();
        (resume, at_login)
    }

    /// A session's start (the session's and the development compositor's, one set of
    /// decisions): the record begun, and, unless in Safe Mode, what the last session leaves
    /// opened (after the person's login items, never twice) or the person asked first. Whether
    /// anything was reopened.
    pub fn at_login(
        path: PathBuf,
        reopen_file: Option<&Path>,
        preferences: &Preferences,
        safe_mode: bool,
        launcher: &mut Launcher,
        scene: &mut Scene,
        now: Instant,
    ) -> (Self, bool) {
        let (mut resume, at_login) =
            Self::start(path, reopen_file, preferences.reopen_windows, now);
        if safe_mode {
            return (resume, false);
        }
        match at_login {
            AtLogin::Reopen(ids) => {
                let opened = reopen(launcher, &ids, preferences);
                resume.reopened(&opened);
                (resume, !opened.is_empty())
            }
            AtLogin::Ask(ids) => {
                resume.question = ask(scene, &ids, &|id| {
                    launcher.application(id).map(|application| application.name)
                })
                .map(|sheet| (sheet, ids));
                (resume, false)
            }
        }
    }

    /// A turn: the person's answer to reopening again taken, and the record following what runs
    /// (the launcher's applications, a file chooser no application of its own).
    pub fn serve(
        &mut self,
        scene: &mut Scene,
        launcher: &mut Launcher,
        preferences: &Preferences,
        now: Instant,
    ) {
        if let Some((sheet, ids)) = &self.question {
            if let Some(accepted) = scene.take_sheet_answer(*sheet) {
                let opened = if accepted {
                    reopen(launcher, ids, preferences)
                } else {
                    eprintln!("[resume] not reopened, as the person asked");
                    Vec::new()
                };
                self.question = None;
                self.reopened(&opened);
            }
        }
        let running: Vec<String> = launcher
            .running()
            .filter(|(_, launched)| !launched.chooser)
            .map(|(_, launched)| launched.id.clone())
            .collect();
        self.follow(running, now);
    }

    /// What this start opened again: kept at once, so that this session ending before anything
    /// runs still counts as a crash of its reopening.
    pub fn reopened(&mut self, ids: &[String]) {
        self.state.reopened = once_each(ids.iter().cloned());
        self.write();
    }

    /// The session's turn: what runs now (by id, in launch order), at `now`. Written only when it
    /// changed, and once when the session has run a minute.
    pub fn follow(&mut self, running: impl IntoIterator<Item = String>, now: Instant) {
        let running = once_each(running);
        let steady = now.saturating_duration_since(self.started) >= STEADY;
        if running != self.state.running || steady != self.state.steady {
            self.state.running = running;
            self.state.steady = steady;
            self.write();
        }
    }

    /// Written whole: a crash or a power cut leaves the last record or this one, never a part.
    fn write(&self) {
        let result = (|| {
            if let Some(folder) = self.path.parent() {
                std::fs::create_dir_all(folder)?;
            }
            let partial = self.path.with_extension("new");
            let text = serde_json::to_vec(&self.state).map_err(std::io::Error::other)?;
            // Not synced: the write is on the compositor's turn, which never waits on the disk;
            // renamed over the last one, which the file system orders after the data (ext4's
            // auto_da_alloc), so a power cut leaves one record or the other.
            std::fs::write(&partial, text)?;
            std::fs::rename(&partial, &self.path)
        })();
        if let Err(error) = result {
            eprintln!("[resume] {}: {error}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Home(PathBuf);

    impl Home {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("oxyos-resume-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn record(&self) -> PathBuf {
            self.0.join("session")
        }
        fn reopen(&self) -> PathBuf {
            self.0.join("reopen-at-login")
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| id.to_string()).collect()
    }

    const ALLO: &str = "so.oxy.allo";
    const INBOX: &str = "so.oxy.inbox";

    #[test]
    fn what_ran_is_reopened_after_a_crash_or_a_power_cut() {
        let home = Home::new("crash");
        let now = Instant::now();
        let (mut first, at) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        assert_eq!(at, AtLogin::Reopen(Vec::new()));
        first.follow(ids(&[ALLO, INBOX, ALLO]), now + STEADY);
        // It ends with no word: a crash, a hang, a power cut.
        drop(first);
        let (_, at) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        assert_eq!(at, AtLogin::Reopen(ids(&[ALLO, INBOX])));
    }

    #[test]
    fn the_persons_latest_choice_of_reopen_windows_applies_to_an_end_not_asked_for() {
        let home = Home::new("choice");
        let now = Instant::now();
        let (mut first, _) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        first.follow(ids(&[ALLO]), now + STEADY);
        let (_, at) = Resume::start(home.record(), Some(&home.reopen()), false, now);
        assert_eq!(at, AtLogin::Reopen(Vec::new()));
    }

    #[test]
    fn the_persons_own_end_reopens_what_its_question_kept() {
        let home = Home::new("asked");
        let now = Instant::now();
        let (mut first, _) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        first.follow(ids(&[ALLO, INBOX]), now);
        // Log Out with Reopen windows: ADR 0064 wrote what ran when asked, and each quit.
        std::fs::write(home.reopen(), format!("{ALLO}\n")).unwrap();
        first.follow(ids(&[]), now + STEADY);
        let (mut second, at) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        assert_eq!(at, AtLogin::Reopen(ids(&[ALLO])));
        assert!(!home.reopen().exists(), "read once");
        // Log Out without it: the question kept no file, and the choice is the preference's.
        second.follow(ids(&[INBOX]), now);
        let (_, at) = Resume::start(home.record(), Some(&home.reopen()), false, now);
        assert_eq!(at, AtLogin::Reopen(Vec::new()));
    }

    #[test]
    fn an_end_within_a_minute_of_reopening_asks_before_reopening_again() {
        let home = Home::new("loop");
        let now = Instant::now();
        let (mut first, _) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        first.follow(ids(&[ALLO, INBOX]), now + STEADY);
        let (mut second, at) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        let AtLogin::Reopen(reopened) = at else {
            panic!("{at:?}")
        };
        second.reopened(&reopened);
        // It ends before anything ran: the reopening ended it.
        let (mut third, at) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        assert_eq!(at, AtLogin::Ask(ids(&[ALLO, INBOX])));
        // Don't Reopen: nothing ran; a later end reopens nothing.
        third.follow(ids(&[]), now + STEADY);
        let (_, at) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        assert_eq!(at, AtLogin::Reopen(Vec::new()));
    }

    #[test]
    fn a_session_that_ran_a_minute_after_reopening_is_no_crash_of_it() {
        let home = Home::new("steady");
        let now = Instant::now();
        let (mut first, _) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        first.reopened(&ids(&[ALLO]));
        first.follow(ids(&[ALLO]), now + Duration::from_secs(59));
        let (_, at) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        assert_eq!(at, AtLogin::Ask(ids(&[ALLO])));
        let (mut again, _) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        again.reopened(&ids(&[ALLO]));
        again.follow(ids(&[ALLO]), now + STEADY);
        let (_, at) = Resume::start(home.record(), Some(&home.reopen()), true, now);
        assert_eq!(at, AtLogin::Reopen(ids(&[ALLO])));
    }

    #[test]
    fn a_first_start_with_no_record_keeps_the_reopen_file_as_before() {
        let home = Home::new("first");
        std::fs::write(home.reopen(), format!("{ALLO}\n{ALLO}\n")).unwrap();
        let (_, at) = Resume::start(home.record(), Some(&home.reopen()), true, Instant::now());
        assert_eq!(at, AtLogin::Reopen(ids(&[ALLO])));
        // No record and no file: nothing.
        let _ = std::fs::remove_file(home.record());
        let (_, at) = Resume::start(home.record(), Some(&home.reopen()), true, Instant::now());
        assert_eq!(at, AtLogin::Reopen(Vec::new()));
        // A record unreadable (a crash between its write and its rename cannot make one, but a
        // person's editor can): as no record.
        std::fs::write(home.record(), b"{not json").unwrap();
        let (_, at) = Resume::start(home.record(), None, true, Instant::now());
        assert_eq!(at, AtLogin::Reopen(Vec::new()));
    }
}
