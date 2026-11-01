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

mod client;
mod displays;
mod ime;
mod launch;
mod lock;
mod pasteboard;
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
    /// The descriptor its turn is woken by, among the compositor's wakers, given back at the end.
    installs_waker: Option<std::os::fd::RawFd>,
    /// The Trash follower's waker among the compositor's, given back as the session finishes.
    pub trash_waker: Option<std::os::fd::RawFd>,
    /// `--controls FILE`: the machine the hardware keys and the Control Center act on, played
    /// (docs/adr/0025-hardware-keys-and-control-center.mdx); each step says what it is then.
    controls: Option<oxyos_controls::Fake>,
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
    /// The URLs the shell opened since a step last took them (docs/adr/0034-the-dock.mdx): each
    /// URL and the application started for it.
    pub url_opens: Vec<(String, String)>,
    /// The user's Trashes, followed as the session follows them (`--trash`, a scenario's own home).
    pub trash: Option<oxyos_compositor::trash::Trash>,
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
            app: None,
            settings: oxyos_compositor::settings::Settings::from_file(None),
            grant_settings: None,
            grants: Vec::new(),
            installs: None,
            installs_waker: None,
            trash_waker: None,
            controls: None,
            launched_clients: Default::default(),
            application_launches: Vec::new(),
            screenshot_opens: Vec::new(),
            url_opens: Vec::new(),
            trash: None,
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
            if let Some(launcher) = self.launcher.as_ref() {
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

    /// The store's, Settings' and applications' requests to the install service (ADR 0030), as
    /// the session serves them; what the store opens is launched from the registry.
    fn serve_installs(&mut self) {
        // The dock's Trash, as the session follows it (docs/adr/0034-the-dock.mdx).
        if let Some(trash) = self.trash.as_mut() {
            trash.serve(&mut self.compositor.scene);
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

    /// Starts the applications the shell asked for that the registry lists, and answers it;
    /// names the windows of what was started (ADR 0013), and grants them what their records give.
    fn serve_launches(&mut self) {
        self.serve_choosers();
        let (launcher, app) = (&self.launcher, &self.app);
        let grant_settings = self.grant_settings.as_deref();
        let fixtures = &self.grants;
        self.compositor.name_clients(&|pid| {
            let (id, granted) = match app {
                Some((started, id)) if *started == pid => {
                    (id.clone(), oxyos_apps::Grants::default())
                }
                _ => {
                    let launched = launcher.as_ref()?.application_of(pid)?;
                    (launched.id.clone(), launched.grants)
                }
            };
            let fixture = fixtures.iter().filter(|(fixed, _)| *fixed == id).fold(
                oxyos_apps::Grants::default(),
                |all, (_, g)| oxyos_apps::Grants {
                    settings: all.settings || g.settings,
                    store: all.store || g.store,
                    updates: all.updates || g.updates,
                    privacy: all.privacy || g.privacy,
                },
            );
            Some(oxyos_compositor::LaunchRecord {
                grants: Grants {
                    settings: granted.settings
                        || fixture.settings
                        || grant_settings == Some(id.as_str()),
                    store: granted.store || fixture.store,
                    updates: granted.updates || fixture.updates,
                    privacy: granted.privacy || fixture.privacy,
                    ..Grants::default()
                },
                application: id,
            })
        });
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
        for (pid, _) in launcher.reap() {
            self.choosers.exited(&mut self.compositor.scene, pid);
            self.compositor.forget_chooser(pid);
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
        // The URLs the shell opens and the force quits it asks for (docs/adr/0034-the-dock.mdx),
        // as the session serves them.
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
        for pid in self.compositor.take_force_quits() {
            if launcher.force_quit(pid) {
                eprintln!("[launch] force quit {pid}");
            }
        }
    }

    /// The changes Settings asked for, kept as the session keeps them. The screen reader is the
    /// scenario's (`--reader`): its preference is kept and told, not followed.
    fn serve_preferences(&mut self) {
        let now = oxyos_compositor::settings::now();
        for key in self.settings.serve(&mut self.compositor, now) {
            eprintln!("[preferences] {key} changed");
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

    /// Closes every connection, waits for the clients to exit, and checks that no scene
    /// resource or descriptor remains.
    fn finish(mut self, children: &mut Children) -> Result<()> {
        let clients: Vec<ClientId> = self.compositor.peers.keys().copied().collect();
        for client in clients {
            self.compositor.disconnect(client);
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
        // The install service's side, its threads waited for and its waker given back.
        self.installs = None;
        if let Some(waker) = self.installs_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        // The Trash's follower, its watcher's thread waited for and its waker given back.
        self.trash = None;
        if let Some(waker) = self.trash_waker.take() {
            use std::os::fd::AsRawFd;
            self.compositor.wakers.retain(|fd| fd.as_raw_fd() != waker);
        }
        // The render node the offer of GPU buffers holds goes with the session.
        self.compositor.withdraw_gpu_buffers();
        // The screenshots' writer, its thread waited for (it has written all it was given).
        self.compositor.screenshots = None;
        let fds = open_descriptors()?;
        if fds != self.baseline_fds {
            return Err(format!(
                "descriptors leaked: {} before, {fds} after",
                self.baseline_fds
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

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => run(Path::new("target/oxyos-offscreen")),
        [flag, path] if flag == "--out" => run(Path::new(path)),
        [flag, path] if flag == "--app" => Ok(app(Path::new(path))?),
        [flag, path] if flag == "--other" => Ok(other(Path::new(path))?),
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
