//! Offscreen development compositor: real client processes, real shared-memory buffers, the real
//! protocol transport and scene policy, composed on the CPU into PNG snapshots.
//!
//! This is the development path the architecture allows for headless machines. It is not
//! DRM/KMS, not Vulkan and not a session; input is injected by a scripted director, and feedback
//! timestamps are labeled synthetic because nothing is displayed.
//!
//! Without arguments it runs two first-party test clients through the popup scenario.
//! `--launch -- COMMAND` instead launches one application process (such as the native Bloom
//! host), passing the socket in `OXYOS_COMPOSITOR_SOCKET`, and presses the points `--press` names.

mod autofill;
mod client;
mod displays;
mod ime;
mod input_sources;
mod launch;
mod lock;
mod passkeys;
mod pasteboard;
mod played_network;
mod png;
mod reader;
mod uinput;

use client::*;
use oxyos_compositor::Gpu;
use oxyos_compositor_core::{ClientId, Grants, Output, OutputId, RenderItem, Scene, SurfaceKey};
use oxyos_ipc::Listener;
use oxyos_protocol::{Rect, Request};
use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use oxyos_compositor::{Compositor, Result};

/// The output, when `--outputs` names none: the primary's rectangle, at scale 1.
const OUTPUT: Rect = Rect::new(0, 0, 1280, 720);
const OUTPUT_ID: OutputId = 1;
const DEADLINE: Duration = Duration::from_secs(10);
const BUTTON: u32 = 272;
/// The secondary (right) button's Linux code: a context menu's.
const SECONDARY_BUTTON: u32 = 273;

/// The primary's composed frame as a PNG in `directory` (`NAME.png`), and every output's by its id
/// beside it (`NAME.output-ID.png`, in its own pixels), but `OUTPUT_ID`'s while it is the primary:
/// that one is `NAME.png` alone. Another output made the primary (Settings, ADR 0018) is both.
fn snapshot(compositor: &Compositor, directory: &Path, name: &str) -> Result<PathBuf> {
    let path = directory.join(name);
    for (index, screen) in compositor.screens.iter().enumerate() {
        let encoded = png::encode(screen.width, screen.height, &screen.framebuffer);
        if index == 0 {
            fs::write(&path, &encoded)?;
        }
        if index != 0 || screen.id != OUTPUT_ID {
            fs::write(
                path.with_extension(format!("output-{}.png", screen.id)),
                &encoded,
            )?;
        }
    }
    Ok(path)
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Children(Vec<Child>);

impl Children {
    /// The application's exit, if it ended: the first child, when there is one (a login screen
    /// has none).
    fn application_exited(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        match self.0.first_mut() {
            Some(application) => application.try_wait(),
            None => Ok(None),
        }
    }
}

impl Drop for Children {
    fn drop(&mut self) {
        for child in &mut self.0 {
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

fn open_descriptors() -> Result<usize> {
    Ok(fs::read_dir("/proc/self/fd")?.count())
}

fn spawn(role: &str, socket: &Path) -> Result<Child> {
    Ok(Command::new(std::env::current_exe()?)
        .arg(role)
        .arg(socket)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .spawn()?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    WaitOther,
    WaitApp,
    WaitMenu,
    WaitHighlight,
    WaitDismissed,
    Done,
}

/// A compositor listening on a private socket, with one output.
struct Session {
    compositor: Compositor,
    listener: Listener,
    socket: PathBuf,
    baseline_fds: usize,
    _cleanup: TempDir,
    /// With `--input-method`: the input method the session plays (ime.rs).
    ime: Option<ime::Ime>,
    /// With `--launch`: another application's clipboard the session plays (pasteboard.rs).
    pasteboard: Option<pasteboard::Pasteboard>,
    /// With `--launch`: the assistive technology the session plays (reader.rs).
    reader: reader::Reader,
    /// With `--launch --shell`: the applications the shell asks for, started from the registry
    /// (docs/adr/0013-launching-applications.mdx), as the session starts them.
    launcher: Option<oxyos_apps::Launcher>,
    /// Applications that quit unexpectedly and resetting one (docs/adr/0061-crashes-and-resetting-
    /// an-application.mdx), as the session has them: the reports in the run's own state folder.
    pub crashes: oxyos_compositor::crashes::Crashes,
    /// Resume (docs/adr/0065-the-session-ending-because-of-a-problem.mdx, decision 3), as the
    /// session keeps it, in the run's own state folder.
    pub resume: Option<oxyos_compositor::resume::Resume>,
    /// At a login screen (`--login`): whether the login program said the last session ended
    /// because of a problem (`OXYOS_LAST_SESSION_ENDED`), which the login screen asks.
    pub last_session_problem: Option<bool>,
    /// Force Quit Applications and hung applications' reports, as the session has them
    /// (docs/adr/0067-hangs-and-force-quit.mdx).
    pub force_quit: oxyos_compositor::force_quit::ForceQuit,
    /// The application the director started itself, by pid, and its id (`OXYOS_APP_ID`): a
    /// launch of its own, recorded as the session records one.
    app: Option<(u32, String)>,
    /// The user's preferences, played as the session keeps them (docs/adr/0019-settings.mdx):
    /// from `--preferences FILE`, never the developer's own; the defaults without it.
    settings: oxyos_compositor::settings::Settings,
    /// `--grant-settings ID`: the development compositor's fixture for the image's policy of
    /// system applications (a session gives `settings` only to the application that policy names
    /// it for): the application `ID`, launched, is given `settings`.
    grant_settings: Option<String>,
    /// `--grant ID=GRANT,...`: system grants given to an application as the image's policy would.
    grants: Vec<(String, oxyos_apps::Grants)>,
    /// `--installd SOCKET`: the session's side of installing (ADR 0030).
    installs: Option<oxyos_compositor::installs::Installs>,
    /// With `--launch --shell`: the applications' login items and background activity, as the
    /// session serves them (docs/adr/0060-login-items-and-background-activity.mdx).
    pub background: Option<oxyos_compositor::background::BackgroundActivity>,
    /// With `--shell`: the session's shells, watched and started again as the session keeps its
    /// own (docs/adr/0061, decision 6).
    pub shells: Option<launch::Shells>,
    /// With `--launch --shell`: windows coming back where they were, the record in the run's own
    /// state folder (docs/adr/0070-windows-come-back-where-they-were.mdx).
    pub places: Option<oxyos_compositor::places::Places>,
    /// The processes the session ended for their background activity since a step last took them
    /// (each its application's id).
    pub background_ended: Vec<String>,
    /// The descriptor its turn is woken by, among the compositor's wakers, given back at the end.
    installs_waker: Option<std::os::fd::RawFd>,
    /// The Trash follower's waker among the compositor's, given back as the session finishes.
    pub trash_waker: Option<std::os::fd::RawFd>,
    /// The session's search (docs/adr/0051-search.mdx), its service this program's child started
    /// at the first ask, so a run that never searches never starts it; and its waker among the
    /// compositor's, given back as the session finishes.
    pub search: Option<oxyos_compositor::search::Search>,
    pub search_waker: Option<std::os::fd::RawFd>,
    /// `--controls FILE`: the machine the hardware keys and the Control Center act on, played
    /// (docs/adr/0025-hardware-keys-and-control-center.mdx); each step says what it is then.
    controls: Option<oxyos_controls::Fake>,
    /// `--network-bus ADDRESS`: the session's networks over a played daemon (ADR 0038), its waker
    /// among the compositor's, and a connection to the daemon's control interface.
    networks: Option<oxyos_compositor::networks::Networks>,
    networks_waker: Option<std::os::fd::RawFd>,
    network_control: Option<zbus::blocking::Connection>,
    /// `--bluetooth-bus ADDRESS`: the session's Bluetooth over a played BlueZ (ADR 0055), its waker
    /// among the compositor's, and a connection to the played BlueZ's control interface.
    bluetooth: Option<oxyos_compositor::bluetooth::Bluetooth>,
    bluetooth_waker: Option<std::os::fd::RawFd>,
    bluetooth_control: Option<zbus::blocking::Connection>,
    /// `--sound FILE`: the session's sound's part (docs/adr/0056-sound-devices.mdx) over a played
    /// machine, which a step changes and whose state each step's result says.
    sound: Option<oxyos_compositor::sound_devices::SoundDevices>,
    played_sound: Option<std::sync::Arc<oxyos_audio::Played>>,
    /// `--power-bus`: the session's battery and energy (ADR 0057), its waker among the
    /// compositor's, and the power services played on the run's bus.
    power: Option<oxyos_compositor::power::Power>,
    power_waker: Option<std::os::fd::RawFd>,
    /// Activity Monitor's part (docs/adr/0063-activity-monitor.mdx), and its waker among the
    /// compositor's.
    activity: Option<oxyos_compositor::activity::Activity>,
    activity_waker: Option<std::os::fd::RawFd>,
    /// A connection to the played power services' control (`oxyos-offscreen --play-power`).
    pub power_control: Option<zbus::blocking::Connection>,
    /// `--backup SOCKET VOLUMES`: the session's side of backups (docs/adr/0059-backups.mdx) over
    /// the development backup service at SOCKET (`oxyos-offscreen --play-backup`), its waker among
    /// the compositor's; VOLUMES is the service's played disks, which a step connects and takes
    /// away, and each step's result says what the service holds (`backup`).
    pub backups: Option<oxyos_compositor::backups::Backups>,
    pub backups_waker: Option<std::os::fd::RawFd>,
    pub backup_service: Option<(PathBuf, PathBuf)>,
    /// `--input-methods PREFIX`: the session's input methods, the real engine (ADR 0041), and its
    /// waker among the compositor's.
    input_methods: Option<oxyos_compositor::input_methods::InputMethods>,
    input_methods_waker: Option<std::os::fd::RawFd>,
    /// `--input-sources-available`: the input sources the session tells, played (input_sources.rs).
    input_sources: Option<input_sources::InputSources>,
    /// `--autofill-fixture ID`: AutoFill (docs/adr/0050-keychain-autofill-and-passkeys.mdx) over
    /// the fixture keychain (autofill.rs), and the director's handle on it.
    autofill: Option<oxyos_compositor::autofill::Autofill>,
    autofill_keychain: Option<std::rc::Rc<std::cell::RefCell<autofill::FixtureKeychain>>>,
    /// `--passkey-fixture ID`: passkeys' sheets (docs/adr/0050-keychain-autofill-and-passkeys.mdx,
    /// decision 7) asked by the fixture accounts' service (passkeys.rs), and the director's handle.
    passkeys: Option<oxyos_compositor::passkeys::Passkeys>,
    passkey_service: Option<std::rc::Rc<std::cell::RefCell<passkeys::FixtureService>>>,
    /// The save confirmations' waker among the compositor's (`--autofill-saves`).
    pub autofill_saves_waker: Option<std::os::fd::RawFd>,
    /// The clients of applications the shell launched (not the director's own): what they are
    /// delivered is counted as theirs, not as leaked (`delivered_json`), even once they are gone.
    launched_clients: std::collections::BTreeSet<ClientId>,
    /// Each application the shell had started, as it was launched (docs/adr/0030-installing-
    /// applications.mdx, decision 7): its id, where its entry came from, and whether it was given
    /// `settings`; the run's summary reports them (`applications`).
    pub application_launches: Vec<(String, String, bool)>,
    /// The screenshots the shell opened (docs/adr/0024-system-shortcuts.mdx, decision 10), each
    /// its file and the application started to open it, until a step reports them.
    pub screenshot_opens: Vec<(PathBuf, String)>,
    /// The URLs the shell and applications opened since a step last took them
    /// (docs/adr/0034-the-dock.mdx; an application's, the platform's Settings,
    /// docs/adr/0019-settings.mdx): each URL and the application started for it.
    pub url_opens: Vec<(String, String)>,
    /// Whether Settings signed out since a step last took it (docs/adr/0020-oxy-account-login.mdx,
    /// `oxyos_compositor::sign_out`). The session ends there; the development compositor records it
    /// and goes on, so the scenario checks it and what follows.
    pub signed_out: bool,
    /// Sleep, restart, shut down and log out as the session serves them
    /// (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx), made at the first turn over the
    /// played logind (`--power-bus`) or none: never the machine's own. The development compositor
    /// records a log out (`logged_out`) and goes on, as it does a sign out.
    pub ending: Option<oxyos_compositor::session_end::SessionEnding>,
    pub ending_logind: oxyos_compositor::session_end::Logind,
    pub logged_out: bool,
    /// Whether the session has asked the director's application to quit: its process ending is
    /// then its answer, not a failure.
    pub quit_asked: bool,
    /// Files dropped on an application's icon in the dock opened with it (ADR 0034, OpenFiles):
    /// each file's path and the application, as the session opens them.
    pub file_opens: Vec<(PathBuf, String)>,
    /// The user's Trashes, followed as the session follows them (`--trash`, a scenario's own home).
    pub trash: Option<oxyos_compositor::trash::Trash>,
    /// Settings › Storage as the session serves it (`--storage`, a played disk), and its waker
    /// among the compositor's, given back as the session finishes.
    pub storage: Option<oxyos_compositor::storage::Storage>,
    pub storage_waker: Option<std::os::fd::RawFd>,
    /// The system sounds the session would have played since a step last took them
    /// (docs/adr/0044-system-sounds.mdx).
    pub sounds: oxyos_sound::Recorder,
    /// `--installed-fixture ROOT`: the install service's state directory a scenario wrote, which
    /// the shell it starts lists from too.
    pub installed_fixture: Option<PathBuf>,
    /// `--displays FILE`: the session's displays, on virtual connectors (displays.rs), shown and
    /// changed from Settings as a session's are.
    displays: Option<(oxyos_compositor::displays::Displays, displays::Virtual)>,
    /// The session's side of file choosers (docs/adr/0029-chromium-on-oxyos.mdx, decision 8),
    /// answers checked against the run's home.
    choosers: oxyos_compositor::choosers::Choosers,
    /// `--file-chooser PROGRAM,ARG...`: the chooser started for each request; without it, the
    /// shell's registry's file manager.
    pub chooser_command: Option<Vec<std::ffi::OsString>>,
    /// The choosers `--file-chooser` started, until they exit, and how many it started.
    chooser_children: Vec<Child>,
    /// The keymap and compose table `Strokes` steps type with when there are no devices, as the
    /// session's keyboard types (made at the first such step).
    keyboard: Option<std::io::Result<oxyos_input::Keyboard>>,
    choosers_started: usize,
    /// The clients that served as a file chooser: what they are delivered is counted as the
    /// chooser's (`delivered_json`), even once they are gone.
    chooser_clients: std::collections::BTreeSet<ClientId>,
    /// The companion application's process (`--companion`, docs/adr/0035-drag-and-drop.mdx).
    pub companion: Option<u32>,
    /// The id the companion was launched for (`--companion-app`), recorded as the session records
    /// a launch: the application's own makes both windows one application's.
    pub companion_app: Option<String>,
}

impl Session {
    /// A session with one output of 1280x720 at scale 1, unpaced.
    fn start() -> Result<Self> {
        Self::with_outputs(&[launch::OutputSpec::of(OUTPUT.width, OUTPUT.height)])
    }

    /// A session with outputs of these pixels at these scales (in 120ths), arranged as a session
    /// arranges its displays (docs/adr/0018-several-displays.mdx): left to right in this order,
    /// top-aligned, numbered from `OUTPUT_ID`; the first is the primary. One given a refresh is
    /// paced as a display at it (`Screen::paced`); the others are said to be 60 Hz, and show each
    /// frame as soon as it is composed.
    fn with_outputs(outputs: &[launch::OutputSpec]) -> Result<Self> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.subsec_nanos();
        let directory =
            std::env::temp_dir().join(format!("oxyos-offscreen-{}-{nonce}", std::process::id()));
        DirBuilder::new().mode(0o700).create(&directory)?;
        let cleanup = TempDir(directory.clone());
        let socket = directory.join("s");
        let listener = Listener::bind(&socket)?;
        listener.set_nonblocking(true)?;
        // zbus's reactor (async-io's driver thread, its epoll, eventfd and timerfd) is the
        // process's, made at the first D-Bus connection and never closed: made before the count,
        // as it is in a session, so the count is of what the session opens and closes (the
        // networks', the input methods' connections). A source registered with it makes it
        // whatever the machine's load; a timer would not, if it had already fired when first
        // polled (a busy machine's thread preempted for its millisecond), and the reactor then
        // made by the session's first connection read as three descriptors leaked.
        drop(async_io::Async::new(
            std::os::unix::net::UnixDatagram::unbound()?,
        )?);
        let baseline_fds = open_descriptors()?;

        let own_uid = fs::metadata(&directory).map(|m| std::os::unix::fs::MetadataExt::uid(&m))?;
        let mut compositor = Compositor::new(Scene::new(), own_uid);
        let sizes: Vec<(u32, u32)> = outputs
            .iter()
            .map(|spec| oxyos_compositor::arrangement::logical(spec.pixels, spec.scale_120))
            .collect();
        let places = oxyos_compositor::arrangement::arrange(&sizes);
        for (index, ((spec, size), at)) in outputs.iter().zip(&sizes).zip(places).enumerate() {
            let id = OUTPUT_ID + index as OutputId;
            compositor.add_output(
                Output {
                    id,
                    rect: Rect::new(at.0, at.1, size.0, size.1),
                    scale_120: spec.scale_120,
                    refresh_mhz: spec.refresh_mhz.unwrap_or(60_000),
                },
                Some(spec.pixels),
            )?;
            if let Some(screen) = compositor.screen_mut(id) {
                screen.paced = spec.refresh_mhz.is_some();
            }
        }
        Ok(Self {
            compositor,
            listener,
            socket,
            baseline_fds,
            _cleanup: cleanup,
            ime: None,
            pasteboard: None,
            reader: reader::Reader::default(),
            launcher: None,
            crashes: oxyos_compositor::crashes::Crashes::new(
                oxyos_compositor::crashes::Reports::of_user(),
                oxyos_compositor::crashes::system_description(),
            ),
            resume: None,
            last_session_problem: None,
            force_quit: Default::default(),
            app: None,
            settings: oxyos_compositor::settings::Settings::from_file(None),
            grant_settings: None,
            grants: Vec::new(),
            installs: None,
            background: None,
            shells: None,
            places: None,
            background_ended: Vec::new(),
            installs_waker: None,
            trash_waker: None,
            search: None,
            search_waker: None,
            controls: None,
            networks: None,
            sound: None,
            played_sound: None,
            networks_waker: None,
            input_methods: None,
            input_methods_waker: None,
            network_control: None,
            bluetooth: None,
            bluetooth_waker: None,
            bluetooth_control: None,
            power: None,
            activity: None,
            activity_waker: None,
            power_waker: None,
            power_control: None,
            backups: None,
            backups_waker: None,
            backup_service: None,
            input_sources: None,
            autofill: None,
            autofill_keychain: None,
            passkeys: None,
            passkey_service: None,
            autofill_saves_waker: None,
            launched_clients: Default::default(),
            application_launches: Vec::new(),
            screenshot_opens: Vec::new(),
            url_opens: Vec::new(),
            signed_out: false,
            ending: None,
            ending_logind: oxyos_compositor::session_end::Logind::None,
            logged_out: false,
            quit_asked: false,
            file_opens: Vec::new(),
            sounds: oxyos_sound::Recorder::default(),
            trash: None,
            storage: None,
            storage_waker: None,
            installed_fixture: None,
            displays: None,
            choosers: oxyos_compositor::choosers::Choosers::new(
                oxyos_files::Scope::for_user().ok(),
            ),
            chooser_command: None,
            chooser_children: Vec::new(),
            keyboard: None,
            choosers_started: 0,
            chooser_clients: Default::default(),
            companion: None,
            companion_app: None,
        })
    }

    /// File choosers (docs/adr/0029-chromium-on-oxyos.mdx, decision 8), as the session serves
    /// them: each request read, its chooser started (the `--file-chooser` fixture, else the
    /// registry's file manager) and attached as it connects, its answer checked and given to the
    /// asker, and its process ended once the request ends.
    fn serve_choosers(&mut self) {
        let mut exited = Vec::new();
        self.chooser_children
            .retain_mut(|child| match child.try_wait() {
                Ok(None) => true,
                _ => {
                    exited.push(child.id());
                    false
                }
            });
        for pid in exited {
            self.choosers.exited(&mut self.compositor.scene, pid);
            self.compositor.forget_chooser(pid);
        }
        let askers: std::collections::BTreeMap<ClientId, oxyos_compositor::choosers::Asker> = self
            .compositor
            .peers
            .iter()
            .filter_map(|(client, peer)| {
                let pid = peer.pid as u32;
                let launched = self.launcher.as_ref().and_then(|l| l.application_of(pid));
                let asker = match (launched, &self.app) {
                    (Some(launched), _) => oxyos_compositor::choosers::Asker {
                        id: launched.id.clone(),
                        name: launched.name.clone(),
                    },
                    (None, Some((started, id))) if *started == pid => {
                        oxyos_compositor::choosers::Asker {
                            id: id.clone(),
                            name: std::env::var("OXYOS_APP_TITLE").unwrap_or_else(|_| id.clone()),
                        }
                    }
                    _ => return None,
                };
                Some((*client, asker))
            })
            .collect();
        let starts = self.choosers.serve(&mut self.compositor.scene, &|client| {
            askers.get(&client).cloned()
        });
        for start in starts {
            let pid = match (&self.chooser_command, self.launcher.as_mut()) {
                (Some(command), _) => self.spawn_chooser(command.clone(), &start.shown),
                (None, Some(launcher)) => launcher
                    .choose(oxyos_apps::FILE_MANAGER, &start.shown, true)
                    .map_err(|error| error.to_string()),
                (None, None) => Err("no file chooser".into()),
            };
            let pid = match pid {
                Ok(pid) => {
                    self.compositor.expect_chooser(pid, start.id);
                    Some(pid)
                }
                Err(error) => {
                    eprintln!("[choosers] request {}: {error}", start.id);
                    None
                }
            };
            self.choosers
                .started(&mut self.compositor.scene, start.id, pid);
        }
        for pid in self.choosers.take_ended() {
            self.compositor.forget_chooser(pid);
            if let Some(launcher) = self.launcher.as_mut() {
                launcher.terminate(pid);
            }
            if let Some(child) = self.chooser_children.iter().find(|c| c.id() == pid) {
                if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
                    let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
                }
            }
        }
        for client in self.compositor.peers.keys() {
            if self.compositor.scene.chooser_of(*client).is_some() {
                self.chooser_clients.insert(*client);
            }
        }
    }

    /// The session ends: the choosers still running are ended and waited for, and the scope their
    /// answers were checked by (which keeps the mount table open, to follow it) goes.
    pub fn end_choosers(&mut self) {
        for mut child in self.chooser_children.drain(..) {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.choosers = oxyos_compositor::choosers::Choosers::new(None);
    }

    /// Starts the `--file-chooser` fixture as a session starts the file manager to choose: on the
    /// application's socket, told what to show, its report beside the application's.
    fn spawn_chooser(
        &mut self,
        command: Vec<std::ffi::OsString>,
        shown: &str,
    ) -> std::result::Result<u32, String> {
        let program = command.first().ok_or("an empty --file-chooser")?;
        let (width, height) = oxyos_apps::CHOOSER_WINDOW;
        let mut chooser = Command::new(program);
        chooser
            .args(&command[1..])
            .args(["--window", &width.to_string(), &height.to_string()])
            .env("OXYOS_COMPOSITOR_SOCKET", &self.socket)
            .env("OXYOS_FILE_CHOOSER", shown)
            .env("OXYOS_APP_ID", oxyos_apps::FILE_MANAGER)
            .env_remove("OXYOS_APP_TITLE")
            .env_remove("OXYOS_APP_UNRECORDED")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        if let Some(bundle) = command.get(1).map(Path::new).filter(|b| b.is_file()) {
            let url = oxyos_apps::script_url(bundle);
            chooser.env("OXYOS_SCRIPT_URL", format!("file://{}", url.display()));
        }
        if let Some(mut report) = std::env::var_os("OXYOS_HOST_REPORT") {
            // Each chooser's own, numbered: a host's report starts its file afresh.
            report.push(format!(".chooser.{}", self.choosers_started));
            chooser.env("OXYOS_HOST_REPORT", report);
        }
        let child = chooser.spawn().map_err(|error| error.to_string())?;
        self.choosers_started += 1;
        let pid = child.id();
        self.chooser_children.push(child);
        Ok(pid)
    }

    /// The session's displays on the virtual connectors `file` describes (`--displays`), shown as
    /// the user's preferences keep them (docs/adr/0018-several-displays.mdx), numbered from
    /// `OUTPUT_ID`.
    fn follow_displays(&mut self, file: &Path) -> Result<()> {
        let mut heads = displays::Virtual::parse(&fs::read_to_string(file)?)?;
        let mut displays = oxyos_compositor::displays::Displays::new(OUTPUT_ID);
        displays.follow(
            &mut heads,
            &mut self.compositor,
            self.settings.preferences(),
        )?;
        if self.compositor.screens.is_empty() {
            return Err(format!("{}: no display could be shown", file.display()).into());
        }
        self.displays = Some((displays, heads));
        Ok(())
    }

    /// The played network daemon changes the machine: its control interface's `Act` (ADR 0038).
    pub fn network_act(&mut self, what: &str) -> Result<()> {
        let bus = self
            .network_control
            .as_ref()
            .ok_or("a network step needs --network-bus")?;
        bus.call_method(
            Some("org.freedesktop.NetworkManager"),
            "/so/oxy/OxyOS/FakeNetworkManager",
            Some("so.oxy.OxyOS.FakeNetworkManager"),
            "Act",
            &(what,),
        )?;
        Ok(())
    }

    /// What the played network daemon holds (its connections, with their passwords, what was
    /// tried), for a step's result; `null` without one.
    pub fn network_state(&self) -> String {
        let Some(bus) = self.network_control.as_ref() else {
            return "null".into();
        };
        bus.call_method(
            Some("org.freedesktop.NetworkManager"),
            "/so/oxy/OxyOS/FakeNetworkManager",
            Some("so.oxy.OxyOS.FakeNetworkManager"),
            "State",
            &(),
        )
        .ok()
        .and_then(|reply| reply.body().deserialize::<String>().ok())
        .unwrap_or_else(|| "null".into())
    }

    /// The played BlueZ changes the machine: its control interface's `Act` (ADR 0055).
    pub fn bluetooth_act(&mut self, what: &str) -> Result<()> {
        let bus = self
            .bluetooth_control
            .as_ref()
            .ok_or("a bluetooth step needs --bluetooth-bus")?;
        bus.call_method(
            Some("org.bluez"),
            "/so/oxy/OxyOS/FakeBlueZ",
            Some("so.oxy.OxyOS.FakeBlueZ"),
            "Act",
            &(what,),
        )?;
        Ok(())
    }

    /// What the played BlueZ holds (its adapter, agent and devices, the calls made of it), for a
    /// step's result; `null` without one.
    pub fn bluetooth_state(&self) -> String {
        let Some(bus) = self.bluetooth_control.as_ref() else {
            return "null".into();
        };
        bus.call_method(
            Some("org.bluez"),
            "/so/oxy/OxyOS/FakeBlueZ",
            Some("so.oxy.OxyOS.FakeBlueZ"),
            "State",
            &(),
        )
        .ok()
        .and_then(|reply| reply.body().deserialize::<String>().ok())
        .unwrap_or_else(|| "null".into())
    }

    /// The played power services change the machine (`--power-bus`, ADR 0057), and the session
    /// reads the battery again, as its follower would on UPower's signal.
    pub fn power_act(&mut self, what: &str) -> Result<()> {
        let bus = self
            .power_control
            .as_ref()
            .ok_or("a power step needs --power-bus")?;
        bus.call_method(
            Some("org.freedesktop.UPower"),
            oxyos_power::played::CONTROL_PATH,
            Some(oxyos_power::played::CONTROL),
            "Act",
            &(what,),
        )?;
        if let Some(controls) = self.compositor.controls.as_mut() {
            controls.read_battery();
        }
        Ok(())
    }

    /// What the played power services hold, for a step's result; `null` without them.
    pub fn power_state(&self) -> String {
        let Some(bus) = self.power_control.as_ref() else {
            return "null".into();
        };
        bus.call_method(
            Some("org.freedesktop.UPower"),
            oxyos_power::played::CONTROL_PATH,
            Some(oxyos_power::played::CONTROL),
            "Describe",
            &(),
        )
        .ok()
        .and_then(|reply| reply.body().deserialize::<String>().ok())
        .unwrap_or_else(|| "null".into())
    }

    /// The development backup service (`--backup`) acts: a played disk connected (made, or
    /// brought back from where it was taken), taken away, or a request of the person's sent to
    /// it as `oxyos-backupd --client` would. A wait for its state is the step loop's: returned
    /// here as what to wait for and for how long.
    pub fn backup_act(&mut self, what: &str) -> Result<Option<(serde_json::Value, u64)>> {
        let (socket, volumes) = self
            .backup_service
            .clone()
            .ok_or("a backup step needs --backup")?;
        let what: serde_json::Value = serde_json::from_str(what)?;
        let away = volumes.with_file_name("away");
        if let Some(uuid) = what.get("connect").and_then(|u| u.as_str()) {
            let disk = volumes.join(uuid);
            if away.join(uuid).is_dir() {
                fs::rename(away.join(uuid), &disk)?;
            } else {
                fs::create_dir_all(&disk)?;
            }
            if let Some(label) = what.get("label").and_then(|l| l.as_str()) {
                fs::write(disk.join(".oxyos-label"), label)?;
            }
        } else if let Some(uuid) = what.get("disconnect").and_then(|u| u.as_str()) {
            fs::create_dir_all(&away)?;
            fs::rename(volumes.join(uuid), away.join(uuid))?;
        } else if let Some(request) = what.get("ask") {
            let answer = backup_ask(&socket, request)?;
            if answer.get("refused").is_some() {
                return Err(format!("the backup service refused {request}: {answer}").into());
            }
        } else if let Some(until) = what.get("until") {
            let ms = what.get("ms").and_then(|ms| ms.as_u64()).unwrap_or(30_000);
            return Ok(Some((until.clone(), ms)));
        } else {
            return Err(
                format!("a backup step is connect, disconnect, ask or until: {what}").into(),
            );
        }
        Ok(None)
    }

    /// What the development backup service holds (its state, as `status` answers), for a step's
    /// result; `null` without it.
    pub fn backup_state(&self) -> String {
        self.backup_service
            .as_ref()
            .and_then(|(socket, _)| backup_ask(socket, &serde_json::json!("status")).ok())
            .and_then(|mut answer| answer.get_mut("status").map(serde_json::Value::take))
            .map_or_else(|| "null".into(), |state| state.to_string())
    }

    /// The battery and energy's turn (ADR 0057) and Activity Monitor's (ADR 0063): the
    /// applications the run started that run, the director's own among them, named by the
    /// registry as Activity Monitor finds them.
    fn serve_power(&mut self) {
        let mut running: Vec<oxyos_power::energy::Running> = self
            .launcher
            .as_ref()
            .map(|launcher| {
                launcher
                    .running()
                    .map(|(pid, launched)| oxyos_power::energy::Running {
                        pid,
                        id: launched.id.clone(),
                        name: launched.name.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(power) = self.power.as_mut() {
            power.serve(&mut self.compositor, self.settings.preferences(), &running);
        }
        if let Some(activity) = self.activity.as_mut() {
            if let Some((pid, id)) = &self.app {
                running.push(oxyos_power::energy::Running {
                    pid: *pid,
                    id: id.clone(),
                    name: id.clone(),
                });
            }
            activity.serve(&mut self.compositor, &running, self.power.as_mut());
        }
    }

    /// What Activity Monitor's part did, for a step's result (regress.py's `activity`): the
    /// samples it took and sent, whether any client follows, and the times its thread and the
    /// director's application's threads were switched out since they started (each a wakeup), so
    /// a step can hold a hidden window's to none.
    pub fn activity_state(&self) -> String {
        let Some(activity) = self.activity.as_ref() else {
            return "null".into();
        };
        let (samples, sent) = activity.counts();
        let thread = std::path::Path::new("/proc/self/task").join(activity.thread_id().to_string());
        let host: Vec<std::path::PathBuf> = self
            .app
            .as_ref()
            .and_then(|(pid, _)| {
                std::fs::read_dir(
                    std::path::Path::new("/proc")
                        .join(pid.to_string())
                        .join("task"),
                )
                .ok()
            })
            .map(|entries| entries.flatten().map(|task| task.path()).collect())
            .unwrap_or_default();
        let (host_wakeups, host_cpu) = host.iter().fold((0, 0), |(w, c), task| {
            (w + switches(task), c + run_ns(task))
        });
        format!(
            "{{\"samples\":{samples},\"sent\":{sent},\"followed\":{},\"wakeups\":{},\"cpu_ms\":{},\"host_wakeups\":{host_wakeups},\"host_cpu_ms\":{}}}",
            activity.followed(),
            switches(&thread),
            run_ns(&thread) / 1_000_000,
            host_cpu / 1_000_000,
        )
    }

    /// Sleep, restart, shut down and log out (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx):
    /// the part made at the first turn (the login screen's at a login), logind's calls made in the
    /// turn on the played bus, so a step's answer is settled with it.
    fn serve_session_end(&mut self) {
        let login = self.compositor.login;
        let directory = self.directory().join("reopen-at-login");
        let logind = self.ending_logind.clone();
        let ending = self.ending.get_or_insert_with(|| {
            oxyos_compositor::session_end::SessionEnding::inline(logind, login, Some(directory))
        });
        let launcher = self.launcher.as_ref();
        let pids: std::collections::BTreeMap<ClientId, u32> = self
            .compositor
            .peers
            .iter()
            .map(|(client, peer)| (*client, peer.pid as u32))
            .collect();
        let scene = &self.compositor.scene;
        let names: std::collections::BTreeMap<ClientId, String> = scene
            .applications()
            .into_iter()
            .map(|client| {
                let launched = pids
                    .get(&client)
                    .and_then(|pid| launcher.and_then(|launcher| launcher.application_of(*pid)))
                    .map(|launched| launched.name.clone());
                let recorded = scene.application(client).map(str::to_owned);
                let name = launched.or_else(|| {
                    let id = recorded?;
                    Some(
                        launcher
                            .and_then(|launcher| launcher.application(&id))
                            .map_or(id, |application| application.name),
                    )
                });
                (client, name.unwrap_or_default())
            })
            .collect();
        let running: Vec<String> = launcher
            .map(|launcher| launcher.running().map(|(_, l)| l.id.clone()).collect())
            .unwrap_or_default();
        // What the session started it can end at once, as an end begins (ADR 0064, decision 10).
        let endable: Vec<ClientId> = pids
            .iter()
            .filter(|(_, pid)| {
                launcher.is_some_and(|launcher| launcher.application_of(**pid).is_some())
            })
            .map(|(client, _)| *client)
            .collect();
        ending.set_endable(endable);
        let served = ending.serve(
            &mut self.compositor,
            &mut self.settings,
            &|client| names.get(&client).cloned().unwrap_or_default(),
            &running,
        );
        let at_once = ending.take_ended_at_once();
        if let Some(launcher) = self.launcher.as_mut() {
            for client in at_once {
                if let Some(pid) = pids.get(&client) {
                    if launcher.kill(*pid) {
                        eprintln!("[launch] ended {pid} at once: nothing unsaved");
                    }
                }
            }
        }
        self.logged_out |= served.log_out;
        self.quit_asked |= matches!(ending.state_name(), "quitting" | "ending" | "ended");
    }

    /// The store's, Settings' and applications' requests to the install service (ADR 0030), as
    /// the session serves them; what the store opens is launched from the registry.
    fn serve_installs(&mut self) {
        // Search's asks, after `serve_launches` named every client that asked by its launch
        // record: who asked is told the index with each, as the session tells it.
        if let Some(search) = self.search.as_mut() {
            search.set_locale(&self.settings.locale());
            search.serve(&mut self.compositor.scene);
        }
        // Settings' sign out (docs/adr/0020-oxy-account-login.mdx), recorded rather than ending
        // the run, before the install service takes what is left.
        let login = self.compositor.login;
        self.signed_out |= oxyos_compositor::sign_out::serve(&mut self.compositor.scene, login);
        // Resetting an application and the crash notices' answers (docs/adr/0061), before the
        // install service takes the requests left; what the person reopens is started as the
        // shell's launches are, and recorded so.
        if let Some(launcher) = self.launcher.as_mut() {
            self.crashes.set_locale(&self.settings.locale());
            let served = self
                .crashes
                .serve(&mut self.compositor.scene, launcher, login);
            // What a reset application donated to the index goes with its data.
            if let Some(search) = self.search.as_mut() {
                for id in &served.reset {
                    search.forget_application(id);
                }
            }
            // Where its windows were goes with it (docs/adr/0070).
            if let Some(places) = self.places.as_mut() {
                for id in &served.reset {
                    places.forget(id, std::time::Instant::now());
                }
            }
            for id in served.open {
                match launcher.launch(&id) {
                    Ok(pid) => {
                        if let Some(launched) = launcher.application_of(pid) {
                            let origin = match &launched.origin {
                                oxyos_apps::Origin::System => "system".to_string(),
                                oxyos_apps::Origin::Installed { version_code } => {
                                    format!("installed {version_code}")
                                }
                                oxyos_apps::Origin::Development => "development".to_string(),
                            };
                            let settings = launched.grants.settings
                                || self.grant_settings.as_deref() == Some(launched.id.as_str());
                            self.application_launches
                                .push((launched.id.clone(), origin, settings));
                        }
                    }
                    Err(error) => eprintln!("[launch] {id} not reopened: {error}"),
                }
            }
        }
        // AutoFill's fields, picks and the shell's presses, before the install service takes the
        // requests left.
        if let Some(autofill) = self.autofill.as_mut() {
            autofill.serve(&mut self.compositor);
        }
        // Passkeys' sheets, the fixture asking once its application has a client, before the
        // install service takes the requests left.
        if let (Some(passkeys), Some(service)) =
            (self.passkeys.as_mut(), self.passkey_service.as_ref())
        {
            if service.borrow().pid.is_none() {
                let application = service.borrow().application.clone();
                let pid = self.compositor.peers.iter().find_map(|(client, peer)| {
                    (self.compositor.scene.application(*client) == Some(application.as_str()))
                        .then_some(peer.pid)
                });
                service.borrow_mut().pid = pid;
            }
            passkeys.serve(&mut self.compositor);
        }
        // The input sources' requests, before the install service takes what is left.
        self.serve_input_sources();
        // Sleep, restart, shut down and log out (ADR 0064), before the install service takes what
        // is left.
        self.serve_session_end();
        // The login screen's question of how the last session ended (docs/adr/0065).
        if let Some(problem) = self.last_session_problem {
            oxyos_compositor::login::answer_last_session(&mut self.compositor.scene, problem);
        }
        // Battery and energy (ADR 0057), before the install service takes what is left.
        self.serve_power();
        // Backups (ADR 0059), before the install service takes what is left.
        if let Some(backups) = self.backups.as_mut() {
            backups.serve(&mut self.compositor.scene, self.settings.preferences());
        }
        // The dock's Trash, as the session follows it (docs/adr/0034-the-dock.mdx), emptying
        // itself as the person has it (docs/adr/0062-storage.mdx).
        if let Some(trash) = self.trash.as_mut() {
            trash.set_empty_automatically(self.settings.preferences().empty_trash_automatically);
            trash.serve(&mut self.compositor.scene);
            self.compositor.play_sounds();
        }
        // Settings › Storage (docs/adr/0062-storage.mdx), before the install service takes the
        // requests left: an item revealed opens the file manager as the shell's URLs do.
        if let Some(storage) = self.storage.as_mut() {
            storage.set_locale(&self.settings.locale());
            for reveal in storage.serve(&mut self.compositor.scene) {
                let opened = match self
                    .launcher
                    .as_mut()
                    .map(|launcher| launcher.open_url(&reveal.url))
                {
                    Some(Ok((id, _))) => {
                        self.url_opens.push((reveal.url.clone(), id));
                        true
                    }
                    Some(Err(error)) => {
                        eprintln!("[launch] {}: {error}", reveal.url);
                        false
                    }
                    None => false,
                };
                storage.revealed(&mut self.compositor.scene, &reveal, opened);
            }
        }
        // Input sources and the engine (ADR 0041), before the store takes the requests left.
        if let Some(methods) = self.input_methods.as_mut() {
            let now = oxyos_compositor::settings::now();
            methods.serve(&mut self.compositor, &mut self.settings, now);
        }
        // The sound's outputs and inputs (ADR 0056), before the store takes the requests left.
        if let Some(sound) = self.sound.as_mut() {
            sound.serve(
                &mut self.compositor.scene,
                self.compositor.controls.as_mut(),
            );
        }
        // Wi-Fi and networks (ADR 0038), before the store takes the requests that are its own;
        // a change of the daemon's is told to the top bar.
        if let Some(networks) = self.networks.as_mut() {
            networks.serve(&mut self.compositor.scene);
            // The top bar's network, told from what the networks' part read.
            if let (true, Some(network), Some(controls)) = (
                networks.take_changed(),
                networks.status(),
                self.compositor.controls.as_mut(),
            ) {
                controls.network_is(&mut self.compositor.scene, network);
            }
        }
        // Bluetooth (ADR 0055), before the store takes the requests left; Control Center's module
        // told from it.
        if let Some(bluetooth) = self.bluetooth.as_mut() {
            bluetooth.serve(&mut self.compositor.scene);
            if let (true, Some(status), Some(controls)) = (
                bluetooth.take_changed(),
                bluetooth.status(),
                self.compositor.controls.as_mut(),
            ) {
                controls.bluetooth_is(&mut self.compositor.scene, status);
            }
        }
        let Some(installs) = self.installs.as_mut() else {
            return;
        };
        let askers: std::collections::BTreeMap<_, _> = self
            .compositor
            .peers
            .iter()
            .map(|(client, peer)| {
                let pid = peer.pid as u32;
                let launched = self.launcher.as_ref().and_then(|l| l.application_of(pid));
                let asker = match (launched, &self.app) {
                    (Some(launched), _) => match launched.origin {
                        oxyos_apps::Origin::System => oxyos_compositor::installs::Asker::System,
                        oxyos_apps::Origin::Development => {
                            oxyos_compositor::installs::Asker::Development
                        }
                        oxyos_apps::Origin::Installed { .. } => {
                            oxyos_compositor::installs::Asker::Installed {
                                id: launched.id.clone(),
                                decisions: launched.capabilities.clone(),
                            }
                        }
                    },
                    (None, Some((started, _))) if *started == pid => {
                        oxyos_compositor::installs::Asker::Development
                    }
                    _ => oxyos_compositor::installs::Asker::Unknown,
                };
                (*client, asker)
            })
            .collect();
        let unknown = oxyos_compositor::installs::Asker::Unknown;
        let opens = installs.serve(&mut self.compositor.scene, &|client| {
            askers.get(&client).cloned().unwrap_or(unknown.clone())
        });
        for open in opens {
            let pid = self
                .launcher
                .as_mut()
                .and_then(|launcher| launcher.launch(&open.id).ok());
            // What it is, for the run's summary, as the shell's launches are recorded.
            if let Some(launched) = pid.and_then(|pid| self.launcher.as_ref()?.application_of(pid))
            {
                let origin = match &launched.origin {
                    oxyos_apps::Origin::System => "system".to_string(),
                    oxyos_apps::Origin::Installed { version_code } => {
                        format!("installed {version_code}")
                    }
                    oxyos_apps::Origin::Development => "development".to_string(),
                };
                let settings = launched.grants.settings
                    || self.grant_settings.as_deref() == Some(launched.id.as_str());
                self.application_launches
                    .push((launched.id.clone(), origin, settings));
            }
            let launched = pid.is_some();
            if let Some(installs) = self.installs.as_ref() {
                installs.opened(&mut self.compositor.scene, &open, launched);
            }
        }
    }

    /// The applications' login items and background activity
    /// (docs/adr/0060-login-items-and-background-activity.mdx), as the session serves them, after
    /// the install service's answers: a process in the background without the right is ended.
    pub fn serve_background(&mut self) {
        let Some(background) = self.background.as_mut() else {
            return;
        };
        let launcher = self.launcher.as_ref();
        let end = background.serve(
            &mut self.compositor,
            &mut self.settings,
            &|pid| launcher?.application_of(pid).cloned(),
            self.installs.as_mut(),
            oxyos_compositor::settings::now(),
        );
        if let Some(launcher) = self.launcher.as_mut() {
            for pid in end {
                if let Some(launched) = launcher.application_of(pid) {
                    self.background_ended.push(launched.id.clone());
                }
                launcher.terminate(pid);
            }
        }
    }

    /// The companion application's client (`--companion`), once it has connected.
    fn companion_client(&self) -> Option<ClientId> {
        let pid = self.companion?;
        self.compositor
            .peers
            .iter()
            .find(|(_, peer)| peer.pid as u32 == pid)
            .map(|(client, _)| *client)
    }

    /// The shells' turn: watched, one that ended noticed and started again after its wait.
    fn serve_shells(&mut self) {
        if let Some(mut shells) = self.shells.take() {
            if let Err(error) = shells.serve(self) {
                eprintln!("[shell] {error}");
            }
            self.shells = Some(shells);
        }
    }

    /// Starts the applications the shell asked for that the registry lists, and answers it;
    /// names the windows of what was started (ADR 0013), and grants them what their records give.
    fn serve_launches(&mut self) {
        self.serve_shells();
        self.serve_choosers();
        let (launcher, app) = (&self.launcher, &self.app);
        let companion = self.companion.zip(self.companion_app.clone());
        let grant_settings = self.grant_settings.as_deref();
        let fixtures = &self.grants;
        self.compositor.name_clients(&|pid| {
            let (id, granted) = match (app, &companion) {
                (Some((started, id)), _) if *started == pid => {
                    (id.clone(), oxyos_apps::Grants::default())
                }
                (_, Some((started, id))) if *started == pid => {
                    (id.clone(), oxyos_apps::Grants::default())
                }
                _ => {
                    let launched = launcher.as_ref()?.application_of(pid)?;
                    (launched.id.clone(), launched.grants)
                }
            };
            let fixture = fixtures
                .iter()
                .filter(|(fixed, _)| *fixed == id)
                .fold(oxyos_apps::Grants::default(), |all, (_, g)| all.with(*g));
            Some(oxyos_compositor::LaunchRecord {
                grants: Grants {
                    settings: granted.settings
                        || fixture.settings
                        || grant_settings == Some(id.as_str()),
                    store: granted.store || fixture.store,
                    updates: granted.updates || fixture.updates,
                    privacy: granted.privacy || fixture.privacy,
                    network: granted.network || fixture.network,
                    bluetooth: granted.bluetooth || fixture.bluetooth,
                    activity: granted.activity || fixture.activity,
                    ..Grants::default()
                },
                application: id,
            })
        });
        // A window of what it started opens where its application's was, and the record follows
        // the windows, as the session does (docs/adr/0070).
        if let Some(places) = self.places.as_mut() {
            places.serve(&mut self.compositor.scene, std::time::Instant::now());
        }
        for client in self.compositor.peers.keys() {
            // The director's own application is client 1 (launch.rs's CLIENT).
            if *client != 1 && self.compositor.scene.application(*client).is_some() {
                self.launched_clients.insert(*client);
            }
        }
        let Some(launcher) = self.launcher.as_mut() else {
            // Nothing to start a notification's application with (ADR 0026).
            for id in self.compositor.scene.take_notification_launches() {
                self.compositor.scene.notification_launch_failed(&id);
            }
            return;
        };
        let exits = launcher.reap();
        for exit in &exits {
            eprintln!(
                "[launch] {} exited ({})",
                exit.launched.id,
                oxyos_compositor::crashes::reason(exit.status)
            );
            self.choosers.exited(&mut self.compositor.scene, exit.pid);
            self.compositor.forget_chooser(exit.pid);
        }
        // Those that quit unexpectedly, as the session tells the person (docs/adr/0061).
        self.crashes.exited(&mut self.compositor.scene, &exits);
        // Those force-quit while hung: their reports (docs/adr/0067).
        self.force_quit.exited(&mut self.crashes, &exits);
        // Resume follows what runs, and takes the person's answer to reopening again (ADR 0065).
        if let Some(resume) = self.resume.as_mut() {
            resume.serve(
                &mut self.compositor.scene,
                launcher,
                self.settings.preferences(),
                std::time::Instant::now(),
            );
            // Reopened on the person's answer: back whole where it was (docs/adr/0070).
            if let Some(places) = self.places.as_mut() {
                for id in resume.take_opened() {
                    places.opened_at_login(&id);
                }
            }
        }
        // A notification pressed while its application ran nowhere (ADR 0026).
        for id in self.compositor.scene.take_notification_launches() {
            if let Err(error) = launcher.launch(&id) {
                eprintln!("[launch] {error}");
                self.compositor.scene.notification_launch_failed(&id);
            }
        }
        // A screenshot's thumbnail pressed (ADR 0024, decision 10): opened as the session opens it.
        for screenshot in self.compositor.scene.take_screenshot_opens() {
            let Some(path) = self
                .compositor
                .screenshots
                .as_ref()
                .and_then(|writer| writer.path(screenshot))
                .map(Path::to_path_buf)
            else {
                continue;
            };
            match launcher.open(&path, "image/png", oxyos_apps::FILE_MANAGER) {
                Ok((id, _)) => self.screenshot_opens.push((path, id)),
                Err(error) => eprintln!("[launch] {}: {error}", path.display()),
            }
        }
        for request in self.compositor.scene.take_launches() {
            let launched = match launcher.launch(&request.id) {
                Ok(pid) => {
                    if let Some(launched) = launcher.application_of(pid) {
                        let origin = match &launched.origin {
                            oxyos_apps::Origin::System => "system".to_string(),
                            oxyos_apps::Origin::Installed { version_code } => {
                                format!("installed {version_code}")
                            }
                            oxyos_apps::Origin::Development => "development".to_string(),
                        };
                        // What it is given: its launch record's grants, and the fixture's
                        // (`--grant-settings`), as `name_clients` gives them.
                        let settings = launched.grants.settings
                            || self.grant_settings.as_deref() == Some(launched.id.as_str());
                        self.application_launches
                            .push((launched.id.clone(), origin, settings));
                    }
                    true
                }
                Err(error) => {
                    eprintln!("[launch] {error}");
                    false
                }
            };
            self.compositor
                .scene
                .launched(request.client, request.serial, launched);
        }
        // The URLs the shell and applications open, and the force quits the shell asks for
        // (docs/adr/0034-the-dock.mdx), as the session serves them.
        for request in self.compositor.scene.take_url_requests() {
            let opened = request.with_input
                && match launcher.open_url(&request.url) {
                    Ok((id, _)) => {
                        self.url_opens.push((request.url.clone(), id));
                        true
                    }
                    Err(error) => {
                        eprintln!("[launch] {}: {error}", request.url);
                        false
                    }
                };
            self.compositor
                .scene
                .launched(request.client, request.serial, opened);
        }
        // The person's force quits, as the session ends them (docs/adr/0067-hangs-and-force-
        // quit.mdx): the dock's, Super+Alt+Escape's and the Oxy menu's; a hung one asked for its
        // report first.
        self.force_quit
            .serve(&mut self.compositor, launcher, std::time::Instant::now());
        // Files dropped on an application's icon in the dock (ADR 0034), opened with it as the
        // session opens them: from the run's own home.
        let requests = self.compositor.scene.take_open_files();
        if !requests.is_empty() {
            let scope = oxyos_files::Scope::for_user().ok();
            for request in requests {
                let mut said = |line: String| eprintln!("[launch] {line}");
                for file in
                    oxyos_compositor::open_files::files_to_open(scope.as_ref(), &request, &mut said)
                {
                    match launcher.open_with(&request.application, &file.path, file.kind) {
                        Ok(_) => self
                            .file_opens
                            .push((file.path.clone(), request.application.clone())),
                        Err(error) => eprintln!("[launch] {}: {error}", file.path.display()),
                    }
                }
            }
        }
    }

    /// The input sources as the session tells them (`--input-sources-available`), from the
    /// preferences kept now.
    fn serve_input_sources(&mut self) {
        let Some(input_sources) = self.input_sources.as_mut() else {
            return;
        };
        let clients: Vec<ClientId> = self.compositor.peers.keys().copied().collect();
        let now = oxyos_compositor::settings::now();
        input_sources.serve(&mut self.compositor, &mut self.settings, &clients, now);
    }

    /// The changes Settings asked for, kept as the session keeps them. The screen reader is the
    /// scenario's (`--reader`): its preference is kept and told, not followed.
    fn serve_preferences(&mut self) {
        let now = oxyos_compositor::settings::now();
        // Each display's wallpaper is kept by its identity (docs/adr/0048-wallpapers.mdx): with
        // `--displays`, the virtual displays'; otherwise each output is `output-ID`.
        if let Some((displays, _)) = self.displays.as_ref() {
            self.settings.set_display_identities(displays.identities());
        }
        for key in self.settings.serve(&mut self.compositor, now) {
            eprintln!("[preferences] {key} changed");
            if let Some(methods) = self.input_methods.as_mut() {
                match key.as_str() {
                    "input_sources" | "input_source" => {
                        methods.follow_preferences(self.settings.preferences())
                    }
                    "language" => methods.set_language(
                        oxyos_compositor::input_methods::language_of(&self.settings.locale()),
                    ),
                    _ => {}
                }
            }
        }
        // And the displays' (ADR 0018), with `--displays`.
        if let Some((displays, heads)) = self.displays.as_mut() {
            displays.serve(
                heads,
                &mut self.compositor,
                &mut self.settings,
                Instant::now(),
            );
        }
    }

    /// The session's own directory (its socket's), removed with it.
    pub fn directory(&self) -> &Path {
        &self._cleanup.0
    }

    /// Closes every connection, waits for the clients to exit, and checks that no scene
    /// resource or descriptor remains.
    fn finish(mut self, children: &mut Children) -> Result<()> {
        // Where the windows were as it ends, written now, as the session writes it (docs/adr/0070).
        if let Some(places) = self.places.as_mut() {
            places.flush();
        }
        let clients: Vec<ClientId> = self.compositor.peers.keys().copied().collect();
        for client in clients {
            self.compositor.disconnect(client);
        }
        // The session's input method, a client of the scene in this process (ADR 0041).
        if let Some(methods) = self.input_methods.as_mut() {
            methods.end(&mut self.compositor);
        }
        // Applications the shell launched end with the session, as their connections did.
        if let Some(launcher) = self.launcher.as_mut() {
            launcher.end(std::time::Duration::from_secs(5));
        }
        for child in &mut children.0 {
            let status = child.wait()?;
            if !status.success() {
                return Err("client failed".into());
            }
        }
        let stats = self.compositor.scene.stats();
        if stats.surfaces != 0 || stats.clients != 0 || stats.registered_buffers != 0 {
            return Err(format!("scene resources left: {stats:?}").into());
        }
        // The networks' side, its threads waited for, its waker and its connections given back.
        self.networks = None;
        self.sound = None;
        if let Some(bus) = self.network_control.take() {
            let _ = bus.close();
        }
        if let Some(controls) = &self.controls {
            controls.close_network_bus();
        }
        if let Some(waker) = self.networks_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        // Bluetooth's side, its thread waited for (the agent unregistered), its waker and its
        // connection given back.
        self.bluetooth = None;
        if let Some(bus) = self.bluetooth_control.take() {
            let _ = bus.close();
        }
        if let Some(waker) = self.bluetooth_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        // The session's ending (ADR 0064): its inhibitor let go and its connection to the played
        // logind closed.
        if let Some(mut ending) = self.ending.take() {
            ending.end();
        }
        // Backups' side, its threads waited for and its waker given back.
        self.backups = None;
        if let Some(waker) = self.backups_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        // Battery and energy's side, its threads waited for, its waker given back, and the
        // played services gone.
        if let Some(power) = self.power.as_mut() {
            power.end();
        }
        self.power = None;
        if let Some(waker) = self.power_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        self.activity = None;
        if let Some(waker) = self.activity_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        if let Some(controls) = &self.controls {
            controls.close_power_bus();
        }
        if let Some(bus) = self.power_control.take() {
            let _ = bus.close();
        }
        // The input methods' engine stopped, its thread waited for and its waker given back.
        self.input_methods = None;
        if let Some(waker) = self.input_methods_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        // The install service's side, its threads waited for and its waker given back.
        self.installs = None;
        if let Some(waker) = self.installs_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        // The search service ended, its threads waited for and its waker given back.
        self.search = None;
        if let Some(waker) = self.search_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        // The Trash's follower, its watcher's thread waited for and its waker given back.
        self.trash = None;
        if let Some(waker) = self.trash_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        // AutoFill's save confirmations, their thread waited for (its connection to the accounts'
        // service closed) and their waker given back.
        self.autofill = None;
        if let Some(waker) = self.autofill_saves_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        // Storage, a measurement under way stopped and waited for, and its waker given back.
        self.storage = None;
        if let Some(waker) = self.storage_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        // The render node the offer of GPU buffers holds goes with the session.
        self.compositor.withdraw_gpu_buffers();
        // The screenshots' writer, its thread waited for (it has written all it was given).
        self.compositor.screenshots = None;
        // A socket closed is let go on the runtime's own thread a moment later (D-Bus's).
        let mut fds = open_descriptors()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while fds > self.baseline_fds && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
            fds = open_descriptors()?;
        }
        // Fewer than at the start is no leak: a descriptor another thread held for a moment as the
        // baseline was counted (the network bus's D-Bus connection being set up) has gone since.
        if fds > self.baseline_fds {
            // What each is, to say what leaked.
            let open: Vec<String> = fs::read_dir("/proc/self/fd")?
                .filter_map(|entry| fs::read_link(entry.ok()?.path()).ok())
                .map(|target| target.display().to_string())
                .collect();
            return Err(format!(
                "descriptors leaked: {} before, {fds} after ({})",
                self.baseline_fds,
                open.join(", ")
            )
            .into());
        }
        Ok(())
    }
}

fn run(out: &Path) -> Result<()> {
    fs::create_dir_all(out)?;
    let mut session = Session::start()?;
    let socket = session.socket.clone();
    let listener = &session.listener;
    let compositor = &mut session.compositor;
    let mut children = Children(vec![spawn("--other", &socket)?]);
    let (other, app) = (1, 2);
    let deadline = Instant::now() + DEADLINE;
    let mut step = Step::WaitOther;
    let mut snapshots = Vec::new();
    let mut checks = checks::Checks::default();

    while step != Step::Done {
        if Instant::now() > deadline {
            return Err(format!("deadline exceeded at {step:?}").into());
        }
        for child in &mut children.0 {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    return Err(format!("client failed at {step:?}").into());
                }
            }
        }
        compositor.serve(listener)?;

        match step {
            Step::WaitOther => {
                if compositor.surface_rect(other, 1).is_some() {
                    children.0.push(spawn("--app", &socket)?);
                    step = Step::WaitApp;
                }
            }
            Step::WaitApp => {
                // Shown whole, once it has arrived (docs/adr/0011-transitions.mdx).
                if let Some(window) = compositor
                    .surface_rect(app, 1)
                    .filter(|_| compositor.transitions.is_empty())
                {
                    snapshots.push(snapshot(compositor, out, "1-windows.png")?);
                    // Below the window, over the other application (in the window's shadow,
                    // docs/adr/0016-windows.mdx): where the menu opens, shown again once it goes.
                    let below = (window.x + 10, window.bottom() as i32 + 10);
                    checks.below_window = Some((below, compositor.pixel(below.0, below.1)));
                    // Synthetic input: a right-button press at the window's center.
                    let center = (
                        window.x + window.width as i32 / 2,
                        window.y + window.height as i32 / 2,
                    );
                    compositor.scene.pointer_motion(center.0, center.1);
                    compositor.scene.pointer_button(BUTTON + 1, true);
                    compositor.scene.pointer_button(BUTTON + 1, false);
                    step = Step::WaitMenu;
                }
            }
            Step::WaitMenu => {
                if let Some(menu) = compositor.surface_rect(app, 2) {
                    let window = compositor.surface_rect(app, 1).expect("parent shown");
                    let base = compositor.surface_rect(other, 1).expect("other shown");
                    checks.verify_menu(compositor, window, menu, base)?;
                    snapshots.push(snapshot(compositor, out, "2-menu-open.png")?);
                    // Press a row inside the menu but outside the window, over the other app.
                    let point = (menu.x + 20, menu.y + menu.height as i32 - 20);
                    if window.contains_point(point.0, point.1)
                        || !base.contains_point(point.0, point.1)
                    {
                        return Err("menu test point is not over the other application".into());
                    }
                    compositor.scene.pointer_motion(point.0, point.1);
                    compositor.scene.pointer_button(BUTTON, true);
                    compositor.scene.pointer_button(BUTTON, false);
                    checks.other_presses_before = compositor.peers[&other].presses;
                    checks.menu_sequence = compositor.surface_sequence(app, 2).expect("shown");
                    step = Step::WaitHighlight;
                }
            }
            Step::WaitHighlight => {
                if compositor
                    .surface_sequence(app, 2)
                    .is_some_and(|s| s > checks.menu_sequence)
                {
                    let menu = compositor.surface_rect(app, 2).expect("menu shown");
                    let row = (
                        menu.x + menu.width as i32 / 2,
                        menu.y + menu.height as i32 - 20,
                    );
                    if compositor.pixel(row.0, row.1) != MENU_HIGHLIGHT {
                        return Err("highlighted row not on screen".into());
                    }
                    checks.highlight_over_other = true;
                    snapshots.push(snapshot(compositor, out, "3-menu-row-pressed.png")?);
                    // Outside press over the other application: dismiss, consumed.
                    let base = compositor.surface_rect(other, 1).expect("other shown");
                    let outside = (base.x + 10, base.y + 10);
                    if menu.contains_point(outside.0, outside.1) {
                        return Err("outside point is inside the menu".into());
                    }
                    compositor.scene.pointer_motion(outside.0, outside.1);
                    compositor.scene.pointer_button(BUTTON, true);
                    compositor.scene.pointer_button(BUTTON, false);
                    step = Step::WaitDismissed;
                }
            }
            Step::WaitDismissed => {
                // Dismissed, the menu is gone from the scene at once and from the screen once
                // it has faded out (docs/adr/0011-transitions.mdx).
                if compositor.surface_rect(app, 2).is_none()
                    && compositor.scene.stats().popups == 0
                    && compositor.transitions.is_empty()
                {
                    checks.outside_presses_delivered =
                        compositor.peers[&other].presses - checks.other_presses_before;
                    let (below, before) = checks.below_window.expect("sampled with the window");
                    if compositor
                        .surface_rect(app, 1)
                        .map(|w| (w.x + 10, w.bottom() as i32 + 10))
                        != Some(below)
                        || compositor.pixel(below.0, below.1) != before
                    {
                        return Err("menu pixels remain after dismissal".into());
                    }
                    snapshots.push(snapshot(compositor, out, "4-menu-dismissed.png")?);
                    step = Step::Done;
                }
            }
            Step::Done => {}
        }
    }
    if checks.outside_presses_delivered != 0 {
        return Err("the dismissing press reached the other application".into());
    }

    // Close every connection; clients exit on EOF. Then nothing may remain.
    let frames = compositor.frames;
    session.finish(&mut children)?;
    println!(
        "{{\"mode\":\"offscreen-cpu\",\"drm\":false,\"gpu\":false,\"bloom\":false,\"clients\":2,\"frames\":{},\"popup_beyond_parent_edges\":{},\"menu_row_pressed_over_other_client\":{},\"outside_presses_delivered\":{},\"descriptors_settled\":true,\"snapshots\":{:?},\"status\":\"passed\"}}",
        frames,
        checks.edges,
        checks.highlight_over_other,
        checks.outside_presses_delivered,
        snapshots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    );
    Ok(())
}

mod checks {
    use super::*;

    #[derive(Default)]
    pub struct Checks {
        pub edges: usize,
        pub highlight_over_other: bool,
        pub other_presses_before: usize,
        pub menu_sequence: u64,
        pub outside_presses_delivered: usize,
        /// A point below the app's window, and what it showed before the menu opened.
        pub below_window: Option<((i32, i32), u32)>,
    }

    impl Checks {
        /// Samples composed pixels just outside each edge of the window, inside the menu.
        pub fn verify_menu(
            &mut self,
            compositor: &Compositor,
            window: Rect,
            menu: Rect,
            base: Rect,
        ) -> Result<()> {
            let mid_x = window.x + window.width as i32 / 2 + 7;
            let mid_y = window.y + window.height as i32 / 2 + 7;
            let samples = [
                (window.x - 10, mid_y),
                (window.right() as i32 + 10, mid_y),
                (mid_x, window.y - 10),
                (mid_x, window.bottom() as i32 + 10),
            ];
            for (x, y) in samples {
                if !menu.contains_point(x, y) || window.contains_point(x, y) {
                    return Err(
                        format!("sample ({x}, {y}) is not between window and menu edges").into(),
                    );
                }
                if !base.contains_point(x, y) {
                    return Err(format!("sample ({x}, {y}) is not over the other client").into());
                }
                match compositor.pixel(x, y) {
                    MENU | MENU_BORDER => self.edges += 1,
                    other => {
                        return Err(
                            format!("pixel ({x}, {y}) is {other:06x}, not menu content").into()
                        )
                    }
                }
            }
            Ok(())
        }
    }
}

/// One request of the development backup service and its answer, as `oxyos-backupd --client`.
fn backup_ask(socket: &Path, request: &serde_json::Value) -> Result<serde_json::Value> {
    let connection = oxyos_install::socket::Connection::connect(socket)?;
    connection.set_timeout(Duration::from_secs(120))?;
    connection.send(request)?;
    match connection.receive::<serde_json::Value>()? {
        Some(Some(answer)) => Ok(answer),
        _ => Err("the backup service did not answer".into()),
    }
}

/// Whether the backup service's state holds what a step waits for: an object's keys given, each
/// matching (null: absent or null); a list item by item, of the same length; anything else
/// equal.
pub fn backup_holds(wanted: &serde_json::Value, state: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (wanted, state) {
        (Value::Object(wanted), Value::Object(state)) => wanted.iter().all(|(key, value)| {
            state
                .get(key)
                .map_or(value.is_null(), |held| backup_holds(value, held))
        }),
        (Value::Array(wanted), Value::Array(state)) => {
            wanted.len() == state.len() && wanted.iter().zip(state).all(|(w, s)| backup_holds(w, s))
        }
        _ => wanted == state,
    }
}

fn main() -> Result<()> {
    // The search service (docs/adr/0051-search.mdx, decision 1): this program run again as its
    // child, its standard input the socket the compositor brokers questions over.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|flag| flag == "--search-service")
    {
        std::process::exit(match oxyos_search::service::main() {
            std::process::ExitCode::SUCCESS => 0,
            _ => 1,
        });
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => run(Path::new("target/oxyos-offscreen")),
        [flag, path] if flag == "--out" => run(Path::new(path)),
        [flag, path] if flag == "--app" => Ok(app(Path::new(path))?),
        [flag, path] if flag == "--other" => Ok(other(Path::new(path))?),
        // UPower, power-profiles-daemon and logind played on a run's own bus for the machine a
        // file describes (docs/adr/0057-battery-and-energy.mdx), a process of its own as a
        // session's are; `READY` is written once they are served. Never installed.
        [flag, address, machine, ready] if flag == "--play-power" => {
            let machine: oxyos_power::played::Machine =
                serde_json::from_str(&fs::read_to_string(machine)?)?;
            let address = address.to_str().ok_or("an address is text")?;
            let _played = oxyos_power::played::Played::serve(address, machine)?;
            fs::write(ready, "served\n")?;
            loop {
                std::thread::park();
            }
        }
        // The development backup service (docs/adr/0059-backups.mdx), as `oxyos-backupd
        // --development` runs it, on a run's own folders: its state, its played disks (each
        // folder of VOLUMES a disk connected), restic's cache and what it backs up; it serves
        // once SOCKET is there. A process of its own, as the system's service is. Never installed.
        [flag, state, volumes, socket, cache, sources @ ..]
            if flag == "--play-backup" && !sources.is_empty() =>
        {
            oxyos_backupd::run(oxyos_backupd::Arguments {
                development: true,
                state: state.into(),
                socket: Some(socket.into()),
                volumes: Some(volumes.into()),
                cache: Some(cache.into()),
                sources: sources.iter().map(PathBuf::from).collect(),
            }, None)?;
            Ok(())
        }
        _ if args.iter().any(|arg| arg == "--launch") => {
            let (out, launch) = launch::Launch::parse(&args)?;
            launch::run(&out, &launch)
        }
        _ => Err(
            "usage: oxyos-offscreen [--out DIR]\n       oxyos-offscreen --launch [--out DIR] [--press X,Y]... -- COMMAND [ARGS]..."
                .into(),
        ),
    }
}

/// The time the thread at `task` (`/proc/PID/task/TID`) ran, in nanoseconds (`schedstat`).
fn run_ns(task: &std::path::Path) -> u64 {
    std::fs::read_to_string(task.join("schedstat"))
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

/// The context switches of the thread at `task` (`/proc/PID/task/TID`): each time it slept and
/// was woken.
fn switches(task: &std::path::Path) -> u64 {
    std::fs::read_to_string(task.join("status"))
        .map(|status| {
            status
                .lines()
                .filter(|line| line.contains("ctxt_switches"))
                .filter_map(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod backup_tests {
    use super::backup_holds;
    use serde_json::json;

    #[test]
    fn a_wait_holds_when_every_key_given_matches() {
        let state = json!({
            "running": null, "held": false,
            "destinations": [{"id": "a", "backups": 1, "available": true}],
            "excluded": ["/home/nate/Pictures"]
        });
        assert!(backup_holds(&json!({"running": null}), &state));
        assert!(backup_holds(
            &json!({"destinations": [{"backups": 1}]}),
            &state
        ));
        assert!(!backup_holds(
            &json!({"destinations": [{"backups": 2}]}),
            &state
        ));
        // A list item by item, of the same length.
        assert!(!backup_holds(&json!({"destinations": []}), &state));
        assert!(!backup_holds(&json!({"excluded": []}), &state));
        // Null: absent or null.
        assert!(backup_holds(&json!({"lastBackup": null}), &state));
        assert!(!backup_holds(
            &json!({"running": {"phase": "backingUp"}}),
            &state
        ));
    }
}
