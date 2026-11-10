//! Applications that quit unexpectedly, and resetting one
//! (docs/adr/0061-crashes-and-resetting-an-application.mdx), for the program running the
//! compositor.
//!
//! - **An unexpected quit** is a process the launcher started ending on its own with a status that
//!   is not success: an exit status other than 0, or a signal the session did not send
//!   ([`oxyos_apps::Exited::abnormal`]). One the session asked to end (a force quit, a quit before a
//!   reset, a file chooser withdrawn, the session ending) never is.
//! - **Its report** is written to the person's own folder ([`Reports`]:
//!   `$XDG_STATE_HOME/oxyos/crash-reports`), private, at most [`MAX_REPORTS_PER_APPLICATION`] for an
//!   application and [`MAX_REPORTS`] in all, the oldest going first, each at most
//!   [`oxyos_apps::REPORT_BYTES`] of what the application said of its end. Nothing is ever sent
//!   anywhere: the person reads, copies and shares a report themselves.
//! - **Telling the person.** The first unexpected quit in [`REPEATED`] is a notification about the
//!   application (its icon and name, *quit unexpectedly*) with Reopen and Report…, which nags no
//!   one; a second within [`REPEATED`] is the system's sheet (`crash`): Reopen, Reset…, Report… and
//!   Ignore, focused on Reopen. One sheet at a time for an application.
//! - **Reset** (`{"resetApplication": {"id", "reopen"?}}`, the session's vocabulary): asked by the
//!   shell (from the sheet) or Settings (its Storage), citing the person's input, and confirmed in
//!   the system's sheet (`reset`). Accepted, the application is held (nothing starts it), every
//!   process the session started for it is asked to quit and, after [`QUIT_GRACE`], ended; then its
//!   own data ([`oxyos_apps::OwnData`]) is set aside, its notifications withdrawn, and the asker
//!   told `{"reset": {"id"}}`; with `reopen`, it is opened again, as new. What was set aside is
//!   deleted off the turn.

use oxyos_apps::{Exited, Launcher, Origin, OwnData};
use oxyos_compositor_core::{ClientId, Scene, SessionActivation};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The screen reader as its reports name it, a part of OxyOS the session keeps running
/// (decision 6, amended: docs/adr/0014-screen-reader.mdx).
pub const READER: &str = "so.oxy.reader";
/// The screen reader's name in its reports.
pub const READER_NAME: &str = "Screen reader";

/// Two unexpected quits of one application within this are repeated: the second is the sheet.
pub const REPEATED: Duration = Duration::from_secs(10 * 60);
/// How long an application being reset has to quit when asked, before it is ended.
pub const QUIT_GRACE: Duration = Duration::from_secs(3);
/// The most reports kept in all, and for one application.
pub const MAX_REPORTS: usize = 50;
pub const MAX_REPORTS_PER_APPLICATION: usize = 10;
/// The identifier of the session's notification about an application that quit.
pub const NOTIFICATION: &str = "oxyos.quit-unexpectedly";
/// A report's file name ends so.
const EXTENSION: &str = ".crash";
/// `YYYY-MM-DD-HHMMSS`, when a report's application ended, in its name.
const STAMP_LENGTH: usize = "1970-01-01-000000".len();

/// Each application's unexpected quits in the last [`REPEATED`].
#[derive(Debug, Default)]
pub struct Counter(BTreeMap<String, VecDeque<Instant>>);

impl Counter {
    /// `id` quit unexpectedly at `now`: how many times it has in the [`REPEATED`] up to now, this
    /// one among them.
    pub fn quit(&mut self, id: &str, now: Instant) -> u32 {
        let times = self.0.entry(id.to_string()).or_default();
        while times
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= REPEATED)
        {
            times.pop_front();
        }
        times.push_back(now);
        times.len() as u32
    }

    /// `id` starts over: it was reset.
    pub fn forget(&mut self, id: &str) {
        self.0.remove(id);
    }
}

/// What a report says.
#[derive(Clone, Debug)]
pub struct Report<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub origin: &'a Origin,
    /// When it was found ended, in the session's time zone.
    pub ended: jiff::Zoned,
    pub ran: Duration,
    pub status: Option<std::process::ExitStatus>,
    /// Its unexpected quits in the last [`REPEATED`], this one among them.
    pub times: u32,
    /// The system it ran on (`/etc/os-release`'s name and the kernel's release).
    pub system: Option<&'a str>,
    /// What it said of its end (its report file), if anything.
    pub said: Option<&'a str>,
}

/// A signal's name, as the kernel numbers them on x86-64 and AArch64.
fn signal_name(signal: i32) -> Option<&'static str> {
    Some(match signal {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        10 => "SIGUSR1",
        11 => "SIGSEGV",
        12 => "SIGUSR2",
        13 => "SIGPIPE",
        14 => "SIGALRM",
        15 => "SIGTERM",
        24 => "SIGXCPU",
        25 => "SIGXFSZ",
        31 => "SIGSYS",
        _ => return None,
    })
}

/// How a process ended, as a report says it.
pub fn reason(status: Option<std::process::ExitStatus>) -> String {
    match status {
        None => "it could not be waited for".into(),
        Some(status) => match (status.code(), status.signal()) {
            (Some(code), _) => format!("exit status {code}"),
            (None, Some(signal)) => match signal_name(signal) {
                Some(name) => format!("signal {signal} ({name})"),
                None => format!("signal {signal}"),
            },
            (None, None) => "an unknown status".into(),
        },
    }
}

/// A length of time as a report says it: "13 seconds", "4 minutes 2 seconds", "2 hours 5 minutes".
fn lasted(ran: Duration) -> String {
    let seconds = ran.as_secs();
    let plural = |n: u64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    match seconds {
        0..60 => plural(seconds, "second"),
        60..3600 => format!(
            "{} {}",
            plural(seconds / 60, "minute"),
            plural(seconds % 60, "second")
        ),
        _ => format!(
            "{} {}",
            plural(seconds / 3600, "hour"),
            plural(seconds / 60 % 60, "minute")
        ),
    }
}

/// A report's text: what quit and how, in English as every system's crash reports are, then what
/// the application said of its end.
pub fn report_text(report: &Report) -> String {
    let kind = match report.origin {
        Origin::System => "a system application".to_string(),
        Origin::Installed { version_code } => format!("installed, version {version_code}"),
        Origin::Development => "a development entry".to_string(),
    };
    let mut text = format!(
        "{name} quit unexpectedly.\n\nApplication:  {name}\nIdentifier:   {id}\nKind:         {kind}\nEnded:        {ended}\nRunning for:  {ran}\nReason:       {reason}\n",
        name = report.name,
        id = report.id,
        ended = report.ended.strftime("%Y-%m-%d %H:%M:%S %:z"),
        ran = lasted(report.ran),
        reason = reason(report.status),
    );
    if report.times > 1 {
        text.push_str(&format!(
            "Repeated:     {} times in the last {} minutes\n",
            report.times,
            REPEATED.as_secs() / 60
        ));
    }
    if let Some(system) = report.system {
        text.push_str(&format!("System:       {system}\n"));
    }
    match report.said {
        Some(said) => {
            text.push_str("\nWhat it reported:\n");
            text.push_str(said);
            text.push('\n');
        }
        None => text.push_str("\nIt reported nothing of its end.\n"),
    }
    text
}

/// The system an application runs on, for its reports: `/etc/os-release`'s `PRETTY_NAME` and the
/// kernel's release (`/proc/sys/kernel/osrelease`).
pub fn system_description() -> Option<String> {
    let name = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                let value = line.strip_prefix("PRETTY_NAME=")?;
                Some(value.trim_matches('"').to_string())
            })
        });
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|release| format!("Linux {}", release.trim()));
    match (name, kernel) {
        (Some(name), Some(kernel)) => Some(format!("{name}, {kernel}")),
        (name, kernel) => name.or(kernel),
    }
}

/// The person's crash reports: a folder of their own, the files theirs alone.
#[derive(Clone, Debug)]
pub struct Reports {
    directory: PathBuf,
}

impl Reports {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    /// `$XDG_STATE_HOME/oxyos/crash-reports` (`~/.local/state`); none without a home.
    pub fn of_user() -> Option<Self> {
        let var = |name| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        let state =
            var("XDG_STATE_HOME").or_else(|| var("HOME").map(|home| home.join(".local/state")))?;
        Some(Self::new(state.join("oxyos/crash-reports")))
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Writes `text`, a report of `id` that ended at `ended`, as `<id>_<when>.crash` (`-2`... when
    /// one is there by that name), private; then keeps the newest [`MAX_REPORTS_PER_APPLICATION`]
    /// of `id`'s and [`MAX_REPORTS`] in all. Its path.
    pub fn write(&self, id: &str, ended: &jiff::Zoned, text: &str) -> io::Result<PathBuf> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.directory)?;
        let stamp = ended.strftime("%Y-%m-%d-%H%M%S").to_string();
        let mut written = None;
        for n in 1..=100u32 {
            let name = if n == 1 {
                format!("{id}_{stamp}{EXTENSION}")
            } else {
                format!("{id}_{stamp}-{n}{EXTENSION}")
            };
            let path = self.directory.join(name);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(mut file) => {
                    file.write_all(text.as_bytes())?;
                    written = Some(path);
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        let path = written.ok_or_else(|| io::Error::other("no name is free for the report"))?;
        self.prune();
        Ok(path)
    }

    /// The reports kept, oldest first, each with the application it is of. Oldest by when each
    /// was written; written at the same time as far as the file system tells (its timestamps are
    /// the kernel's coarse clock, a few milliseconds, in which many are written), by the time its
    /// name says it ended and then its number at that second, never by the application's id.
    pub fn list(&self) -> Vec<(PathBuf, String)> {
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return Vec::new();
        };
        let mut reports: Vec<(std::time::SystemTime, String, u32, PathBuf, String)> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let (id, when) = name.strip_suffix(EXTENSION)?.rsplit_once('_')?;
                let metadata = entry.metadata().ok().filter(|m| m.is_file())?;
                // `<stamp>` or `<stamp>-<n>`: the stamp's own dashes come before its time.
                let (stamp, number) = when
                    .rsplit_once('-')
                    .filter(|(stamp, _)| stamp.len() == STAMP_LENGTH)
                    .and_then(|(stamp, n)| Some((stamp, n.parse().ok()?)))
                    .unwrap_or((when, 1));
                Some((
                    metadata.modified().ok()?,
                    stamp.to_string(),
                    number,
                    entry.path(),
                    id.to_string(),
                ))
            })
            .collect();
        reports.sort();
        reports
            .into_iter()
            .map(|(_, _, _, path, id)| (path, id))
            .collect()
    }

    /// Removes the oldest reports past the bounds.
    fn prune(&self) {
        let mut reports = self.list();
        let mut by_id: BTreeMap<String, usize> = BTreeMap::new();
        for (_, id) in &reports {
            *by_id.entry(id.clone()).or_default() += 1;
        }
        let mut kept = Vec::new();
        for (path, id) in reports.drain(..) {
            let count = by_id.get_mut(&id).expect("counted");
            if *count > MAX_REPORTS_PER_APPLICATION {
                *count -= 1;
                let _ = std::fs::remove_file(&path);
            } else {
                kept.push(path);
            }
        }
        let over = kept.len().saturating_sub(MAX_REPORTS);
        for path in kept.into_iter().take(over) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The session's words for its notification, in the session's language.
struct Words {
    quit: fn(&str) -> String,
    reopen: &'static str,
    report: &'static str,
}

fn words(spanish: bool) -> Words {
    if spanish {
        Words {
            quit: |name| format!("{name} se cerró inesperadamente"),
            reopen: "Volver a abrir",
            report: "Informe…",
        }
    } else {
        Words {
            quit: |name| format!("{name} quit unexpectedly"),
            reopen: "Reopen",
            report: "Report…",
        }
    }
}

/// The request, read strictly.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Ask {
    ResetApplication {
        id: String,
        #[serde(default)]
        reopen: bool,
    },
}

/// Whether a request's text is this part's: an object whose one key is `resetApplication`.
fn ours(text: &str) -> bool {
    serde_json::from_str::<std::collections::HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.contains_key("resetApplication"))
}

/// What a turn of [`Crashes::serve`] leaves the program to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Served {
    /// The applications to open now (Reopen, a reset with `reopen`), started as any launch is.
    pub open: Vec<String>,
    /// The applications reset: what the program's other parts keep of theirs goes too (the search
    /// index's donated items, `Search::forget_application`).
    pub reset: Vec<String>,
}

/// An application's latest unexpected quit, for its notice and its Report….
#[derive(Clone, Debug)]
struct Latest {
    name: String,
    times: u32,
    report: String,
    file: Option<PathBuf>,
}

/// A reset asked for: of what, by whom (its request's client and serial), whether to open it again.
#[derive(Clone, Debug)]
struct Asked {
    id: String,
    name: String,
    asker: Option<(ClientId, u32)>,
    reopen: bool,
}

/// A reset accepted, waiting for its application's processes to end.
#[derive(Debug)]
struct Quitting {
    asked: Asked,
    deadline: Instant,
    killed: bool,
}

/// Where an application's own data is: [`OwnData::of`] in a session.
type OwnDataOf = Box<dyn Fn(&str) -> Option<OwnData>>;

/// The part (see the module's documentation).
pub struct Crashes {
    own_data: OwnDataOf,
    counter: Counter,
    reports: Option<Reports>,
    system: Option<String>,
    spanish: bool,
    latest: BTreeMap<String, Latest>,
    /// The crash sheets shown and not answered, by number: their application.
    notices: BTreeMap<u32, String>,
    /// The reset sheets shown and not answered, by number.
    confirming: BTreeMap<u32, Asked>,
    quitting: Vec<Quitting>,
    /// What is being deleted, off the turn.
    deleting: Vec<std::thread::JoinHandle<()>>,
}

impl Crashes {
    /// Reports written into `reports` (none: kept in memory only, for the sheet), naming `system`.
    pub fn new(reports: Option<Reports>, system: Option<String>) -> Self {
        Self {
            own_data: Box::new(OwnData::of),
            counter: Counter::default(),
            reports,
            system,
            spanish: false,
            latest: BTreeMap::new(),
            notices: BTreeMap::new(),
            confirming: BTreeMap::new(),
            quitting: Vec::new(),
            deleting: Vec::new(),
        }
    }

    /// The same part with an application's own data where `own_data` says (the tests' own folders).
    pub fn with_own_data(mut self, own_data: impl Fn(&str) -> Option<OwnData> + 'static) -> Self {
        self.own_data = Box::new(own_data);
        self
    }

    /// The session's language: its notification's words.
    pub fn set_locale(&mut self, locale: &str) {
        self.spanish = locale == "es" || locale.starts_with("es-") || locale.starts_with("es_");
    }

    /// When a reset waiting for its application to quit ends what is left of it: the turn wakes
    /// then.
    pub fn deadline(&self) -> Option<Instant> {
        self.quitting
            .iter()
            .filter(|quitting| !quitting.killed)
            .map(|quitting| quitting.deadline)
            .min()
    }

    /// The processes the launcher reaped this turn.
    pub fn exited(&mut self, scene: &mut Scene, exits: &[Exited]) {
        let zone = jiff::tz::TimeZone::system();
        let now = jiff::Timestamp::now().to_zoned(zone);
        self.exited_at(scene, exits, Instant::now(), now);
    }

    /// [`Crashes::exited`], at `now` (the counter's clock) and `wall` (the report's).
    pub fn exited_at(
        &mut self,
        scene: &mut Scene,
        exits: &[Exited],
        now: Instant,
        wall: jiff::Zoned,
    ) {
        for exit in exits.iter().filter(|exit| exit.abnormal()) {
            let id = exit.launched.id.as_str();
            let name = exit.launched.name.as_str();
            let times = self.counter.quit(id, now);
            let text = report_text(&Report {
                id,
                name,
                origin: &exit.launched.origin,
                ended: wall.clone(),
                ran: exit.ran,
                status: exit.status,
                times,
                system: self.system.as_deref(),
                said: exit.report.as_deref(),
            });
            let file =
                self.reports
                    .as_ref()
                    .and_then(|reports| match reports.write(id, &wall, &text) {
                        Ok(path) => Some(path),
                        Err(error) => {
                            eprintln!("[crashes] {id}'s report is not written: {error}");
                            None
                        }
                    });
            eprintln!(
                "[crashes] {id} quit unexpectedly ({}), {times} in {} minutes{}",
                reason(exit.status),
                REPEATED.as_secs() / 60,
                file.as_ref()
                    .map(|path| format!(": {}", path.display()))
                    .unwrap_or_default()
            );
            self.latest.insert(
                id.to_string(),
                Latest {
                    name: name.to_string(),
                    times,
                    report: text,
                    file,
                },
            );
            // Its sheet already asks: nothing more is said.
            if self.notices.values().any(|shown| shown == id) {
                continue;
            }
            if times < 2 {
                let words = words(self.spanish);
                if let Err(outcome) = scene.post_session_notification(
                    id,
                    NOTIFICATION,
                    &(words.quit)(name),
                    "",
                    &[("reopen", words.reopen), ("report", words.report)],
                ) {
                    eprintln!("[crashes] {id}'s notification is not posted: {outcome:?}");
                }
            } else {
                // The sheet takes the notification's place.
                scene.withdraw_session_notification(id, NOTIFICATION);
                self.show_notice(scene, id, false);
            }
        }
    }

    /// A part of OxyOS the session keeps running quit unexpectedly (its shell, its screen
    /// reader): its report is written, and the person is told nothing, the part being started
    /// again (decision 6) and the shell being what would tell them.
    pub fn note_part(&mut self, exit: &Exited) {
        let zone = jiff::tz::TimeZone::system();
        let wall = jiff::Timestamp::now().to_zoned(zone);
        let id = exit.launched.id.as_str();
        let times = self.counter.quit(id, Instant::now());
        let text = report_text(&Report {
            id,
            name: &exit.launched.name,
            origin: &exit.launched.origin,
            ended: wall.clone(),
            ran: exit.ran,
            status: exit.status,
            times,
            system: self.system.as_deref(),
            said: exit.report.as_deref(),
        });
        if let Some(reports) = &self.reports {
            match reports.write(id, &wall, &text) {
                Ok(path) => eprintln!("[crashes] {id}'s report: {}", path.display()),
                Err(error) => eprintln!("[crashes] {id}'s report is not written: {error}"),
            }
        }
    }

    /// Shows `id`'s crash sheet, at its report when `report`.
    fn show_notice(&mut self, scene: &mut Scene, id: &str, report: bool) {
        let Some(latest) = self.latest.get(id) else {
            return;
        };
        let mut sheet = json!({
            "kind": "crash",
            "id": id,
            "name": latest.name,
            "times": latest.times,
            "report": latest.report,
            "show": if report { "report" } else { "notice" },
        });
        if let Some(file) = &latest.file {
            sheet["file"] = json!(file.to_string_lossy());
        }
        match scene.show_sheet(None, &sheet.to_string()) {
            Some(number) => {
                self.notices.insert(number, id.to_string());
            }
            None => eprintln!("[crashes] {id}'s sheet is not shown: too many are"),
        }
    }

    /// A turn: the requests to reset, the sheets' answers, the notifications pressed, the resets
    /// under way; what the program is left to do ([`Served`]). `greeter`: the login screen's,
    /// which resets nothing.
    pub fn serve(&mut self, scene: &mut Scene, launcher: &mut Launcher, greeter: bool) -> Served {
        self.serve_at(scene, launcher, greeter, Instant::now())
    }

    /// [`Crashes::serve`] at `now`.
    pub fn serve_at(
        &mut self,
        scene: &mut Scene,
        launcher: &mut Launcher,
        greeter: bool,
        now: Instant,
    ) -> Served {
        let mut served = Served::default();
        for request in scene.take_system_requests_where(|request| ours(&request.text)) {
            self.handle(scene, launcher, greeter, request);
        }
        // The crash sheets: Reopen, or nothing.
        let numbers: Vec<u32> = self.notices.keys().copied().collect();
        for number in numbers {
            if let Some(accepted) = scene.take_sheet_answer(number) {
                let id = self.notices.remove(&number).expect("shown");
                // Reopen: unless the person opened it meanwhile.
                if accepted && !launcher.runs(&id) {
                    served.open.push(id);
                }
            }
        }
        // The reset sheets.
        let numbers: Vec<u32> = self.confirming.keys().copied().collect();
        for number in numbers {
            let Some(accepted) = scene.take_sheet_answer(number) else {
                continue;
            };
            let asked = self.confirming.remove(&number).expect("asked");
            if !accepted {
                answer(scene, asked.asker, &refused("cancelled"));
                continue;
            }
            // Its crash sheet, if one asks, is answered by the reset.
            let shown: Vec<u32> = self
                .notices
                .iter()
                .filter(|(_, id)| **id == asked.id)
                .map(|(number, _)| *number)
                .collect();
            for number in shown {
                self.notices.remove(&number);
                scene.dismiss_sheet(number);
            }
            launcher.hold(&asked.id);
            let pids = launcher.quit(&asked.id);
            if !pids.is_empty() {
                eprintln!(
                    "[crashes] {} asked to quit before its reset: {pids:?}",
                    asked.id
                );
            }
            self.quitting.push(Quitting {
                asked,
                deadline: now + QUIT_GRACE,
                killed: false,
            });
        }
        // The notifications pressed: Reopen (or the notification itself), or its report.
        for SessionActivation {
            application,
            identifier,
            action,
        } in scene.take_session_activations()
        {
            if identifier != NOTIFICATION {
                continue;
            }
            match action.as_deref() {
                None | Some("reopen") if !launcher.runs(&application) => {
                    served.open.push(application)
                }
                None | Some("reopen") => {}
                Some("report") => {
                    if !self.notices.values().any(|shown| *shown == application) {
                        self.show_notice(scene, &application, true);
                    }
                }
                Some(_) => {}
            }
        }
        // Resets whose applications quit, or whose time to is up.
        let mut index = 0;
        while index < self.quitting.len() {
            let quitting = &mut self.quitting[index];
            if launcher.runs(&quitting.asked.id) {
                if now >= quitting.deadline && !quitting.killed {
                    quitting.killed = true;
                    for pid in launcher.quit(&quitting.asked.id) {
                        launcher.kill(pid);
                    }
                    eprintln!(
                        "[crashes] {} did not quit in time: ended",
                        quitting.asked.id
                    );
                }
                index += 1;
                continue;
            }
            let quitting = self.quitting.remove(index);
            if let Some(reset) = self.reset(scene, launcher, quitting.asked) {
                if reset.reopen {
                    served.open.push(reset.id.clone());
                }
                served.reset.push(reset.id);
            }
        }
        served
    }

    /// A request to reset (`{"resetApplication": {"id", "reopen"?}}`): checked, then asked of the
    /// person in the system's sheet, or refused.
    pub fn handle(
        &mut self,
        scene: &mut Scene,
        launcher: &Launcher,
        greeter: bool,
        request: oxyos_compositor_core::scene::SystemRequest,
    ) {
        let asker = Some((request.client, request.serial));
        match self.ask(scene, launcher, greeter, &request) {
            Ok(asked) => {
                let sheet = json!({
                    "kind": "reset",
                    "id": asked.id,
                    "name": asked.name,
                    "running": launcher.runs(&asked.id),
                });
                match scene.show_sheet(Some(request.client), &sheet.to_string()) {
                    Some(number) => {
                        self.confirming.insert(number, Asked { asker, ..asked });
                    }
                    None => answer(scene, asker, &refused("busy")),
                }
            }
            Err(reason) => answer(scene, asker, &refused(reason)),
        }
    }

    /// Checks a request to reset: who asks, citing what, for which application.
    fn ask(
        &self,
        scene: &Scene,
        launcher: &Launcher,
        greeter: bool,
        request: &oxyos_compositor_core::scene::SystemRequest,
    ) -> Result<Asked, &'static str> {
        let Ok(Ask::ResetApplication { id, reopen }) = serde_json::from_str::<Ask>(&request.text)
        else {
            return Err("invalidRequest");
        };
        let grants = scene.grants_of(request.client).unwrap_or_default();
        if greeter || !(grants.shell || grants.settings) {
            return Err("notPermitted");
        }
        if !request.with_input {
            return Err("needsInput");
        }
        // An application is never reset at its own request: its asking would be cut short.
        if scene.application(request.client) == Some(id.as_str()) {
            return Err("notPermitted");
        }
        let application = launcher
            .application(&id)
            .filter(|_| (self.own_data)(&id).is_some())
            .ok_or("notFound")?;
        let busy = self.confirming.values().any(|asked| asked.id == id)
            || self.quitting.iter().any(|quitting| quitting.asked.id == id);
        if busy {
            return Err("busy");
        }
        Ok(Asked {
            id,
            name: application.name,
            asker: None,
            reopen,
        })
    }

    /// Erases `asked`'s own data, now that nothing of it runs; what was asked, once it is reset.
    fn reset(&mut self, scene: &mut Scene, launcher: &mut Launcher, asked: Asked) -> Option<Asked> {
        let erased = (self.own_data)(&asked.id)
            .ok_or_else(|| io::Error::other("it has no data directory"))
            .and_then(|own| own.set_aside());
        launcher.release(&asked.id);
        match erased {
            Ok(aside) => {
                eprintln!("[crashes] {} was reset", asked.id);
                let id = asked.id.clone();
                self.deleting.push(std::thread::spawn(move || {
                    if let Err(error) = aside.delete() {
                        eprintln!("[crashes] what {id} kept is not all deleted: {error}");
                    }
                }));
                self.counter.forget(&asked.id);
                scene.withdraw_application_notifications(&asked.id);
                scene.withdraw_session_notification(&asked.id, NOTIFICATION);
                answer(
                    scene,
                    asked.asker,
                    &json!({ "reset": { "id": asked.id } }).to_string(),
                );
                Some(asked)
            }
            Err(error) => {
                eprintln!("[crashes] {} was not reset: {error}", asked.id);
                answer(
                    scene,
                    asked.asker,
                    &json!({ "failed": { "reason": error.to_string() } }).to_string(),
                );
                None
            }
        }
    }

    /// Waits until what resets set aside is deleted (tests; a session leaves it to the next
    /// session's sweep).
    pub fn finish_deleting(&mut self) {
        for thread in self.deleting.drain(..) {
            let _ = thread.join();
        }
    }
}

fn refused(reason: &str) -> String {
    json!({ "refused": { "reason": reason } }).to_string()
}

fn answer(scene: &mut Scene, asker: Option<(ClientId, u32)>, text: &str) {
    if let Some((client, serial)) = asker {
        scene.system_text(client, serial, text);
    }
}

#[cfg(test)]
mod tests;
