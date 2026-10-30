//! The OxyOS session: the compositor (`oxyos-compositor`) on a real seat, running the
//! applications it launches. It runs as the user of a logind session on a seat, from a login on
//! a terminal, never as root (docs/adr/0006-seat.mdx):
//! - its displays are every connected output of the first card with one
//!   (docs/adr/0018-several-displays.mdx): side by side, left to right, top-aligned, the built-in
//!   panel first (the primary, which has the dock), each at its preferred mode, at a scale from
//!   its density, composed and flipped on its own, unless the user's preferences keep another
//!   mode, scale or arrangement for them (Settings' Displays page changes them, a mode on trial
//!   until kept); monitors plugged in and out are followed;
//! - frames are composed on the GPU straight into the display's scanout buffers where Vulkan
//!   exports DMA-BUF, otherwise on the CPU, only what changed (docs/adr/0005-damage.mdx);
//! - its input is the seat's devices through libinput, with the system's keyboard
//!   (`/etc/default/keyboard`: the layout, variant and options passwords are typed with at a
//!   login, so the lock screen takes the same password), or `--layout`'s;
//! - Ctrl+Alt+Fn switches terminals, and logind's switches are followed;
//! - the session locks (docs/adr/0010-lock-screen.mdx) on Ctrl+Super+Q, at the shell's request,
//!   when logind asks, before the machine sleeps, and after the user's `lock_after` minutes with
//!   no input (unless one of the user's processes holds logind's idle inhibitor, as a video
//!   player does); only a password PAM accepts for the session's user, under the service
//!   `oxyos`, unlocks it.
//!
//! `oxyos-session [--layout LAYOUT] [--pointer FILE.cur[,SIZE]] [--shell PROGRAM,ARG...]
//! [--reader PROGRAM [--reader-on]] [-- COMMAND [ARGS]...]` runs COMMAND
//! with `OXYOS_COMPOSITOR_SOCKET` naming the compositor. With `--shell`, the session's shell
//! (ADR 0009) starts first, one on each output: PROGRAM with its ARGs, then `--shell OUTPUT`, on a
//! socket of its own that grants `shell` and `lock` (the shell is the lock screen as well);
//! COMMAND, if given, starts once the shells' panels have set the work areas windows are placed
//! in. An output plugged in gets a shell of its own; one unplugged, its shell stopped. With a shell
//! COMMAND may be left out: the session is the desktop alone. Without one it is required. The
//! session ends when either exits, on Ctrl+Alt+Backspace, or on SIGINT or SIGTERM (they are then
//! asked to stop). A shell
//! that exits while the session is locked is started again, at most once a second: until it
//! shows the lock screen, the compositor's covers are all there is to see.
//!
//! The shell's dock and launcher start applications (docs/adr/0013-launching-applications.mdx):
//! the shell asks for an id, and the session starts it if its registry (`oxyos-apps`) lists it, a
//! Bloom application with the Bloom host `OXYOS_BLOOM_HOST` names and the bundle
//! `OXYOS_BLOOM_BUNDLE` names when its entry has none.
//!
//! The screen reader (docs/adr/0014-screen-reader.mdx) is `--reader PROGRAM` (or the program
//! `OXYOS_READER` names): off until Super+F5 starts it, which stops it again (`--reader-on` starts
//! it with the session). It runs on a socket of its own that grants `accessibility` and
//! `screen_reader`, so its keys (Ctrl+Alt with an arrow or Space) reach it alone.
//!
//! The user's preferences (docs/adr/0019-settings.mdx) are read from
//! `$XDG_CONFIG_HOME/oxyos/preferences` as the session starts, and the session is the file's one
//! writer: reduce motion takes the transitions' time away, the screen reader preference starts
//! the reader as `--reader-on` does, and every client is told the ones it draws with (an
//! automatic appearance following the sun where the system's time zone is). Settings, the
//! system application the image's policy (`/usr/share/oxyos/system-apps.json`) gives `settings`,
//! launched from the entry and running the host and bundle that policy names, changes them: each change is validated, written, applied (the
//! screen reader started or stopped) and told to every client. `lock_after` arms the idle lock
//! again from the change.
//!
//! The hardware keys (docs/adr/0025-hardware-keys-and-control-center.mdx) are the session's:
//! volume, mute, the microphone's mute, the display's and the keyboard's brightness act on
//! PipeWire's default devices (through WirePlumber's `wpctl`) and on the backlights (through
//! logind's `SetBrightness`) whatever has the keyboard, locked or not, and no application sees
//! them; the shell shows each level and its Control Center sets them, turns Wi-Fi on and off
//! (NetworkManager) and shows the battery (UPower) and Bluetooth (BlueZ), which the session looks
//! at every few seconds, off the compositor's turn.
//!
//! The pointer crosses between the outputs where they meet, and is shown on the cursor plane of
//! the display it is on, in the shape the surface under it wants: the display's own arrow, or the
//! images of a Windows cursor file or cursor scheme's folder (`--pointer`, every shape its
//! `Install.inf` names) closest to SIZE pixels at the display's scale (32 by default).
//!
//! Started at boot by a unit of `Type=notify` (docs/adr/0017-boot-logo.mdx), the session sends
//! `READY=1` once its first frame, the splash, has flipped on a display; the unit then lets the
//! boot logo go, which showed the same pixels until then.
//!
//! `oxyos-session --greeter --shell PROGRAM,ARG...` is the login screen
//! (docs/adr/0020-oxy-account-login.mdx), as the login program (`oxyos-login`) runs it in a logind
//! session of class `greeter`, as its own user: the same compositor on the seat, locked from the
//! start, the shell (the Bloom host's `OxyOSLogin`) its one client and the lock surfaces it makes
//! all there is to see. A password typed there is for the account the lock surface names, and
//! goes to the login program (`OXYOS_LOGIN_SOCKET`, `/run/oxyos/login/greeter`), never to PAM
//! from here: accepted, the splash is shown and the login screen ends, and the login program
//! starts the person's session. Nothing is launched, sleep is not held, and the first frame on a
//! display is told to the login program, which tells the service manager.

mod cursor_file;
mod installd;

use oxyos_compositor::displays::{Connector, Displays, Heads};
use oxyos_compositor::settings::{self, Settings};
use oxyos_compositor::{Authentication, Compositor, Gpu, LaunchRecord, Result};
use oxyos_compositor_core::{Grants, ModeInfo, OutputId, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{AuthOutcome, PointerShape};
use rustix::fd::BorrowedFd;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const USAGE: &str = "usage: oxyos-session [--greeter] [--layout LAYOUT] \
                     [--pointer FILE.cur|SCHEME[,SIZE]] [--shell PROGRAM,ARG...] \
                     [--reader PROGRAM [--reader-on]] [-- COMMAND [ARGS]...] \
                     (COMMAND is required without a shell; --greeter takes a shell and no COMMAND)";
/// Once someone signed in at the login screen, it shows the splash this long before it ends.
const SIGNED_IN_SPLASH: Duration = Duration::from_millis(250);
/// The shells' panels set the work areas within this budget, or the session does not start.
const SHELL_DEADLINE: Duration = Duration::from_secs(60);
/// The PAM service the lock checks passwords under (`/etc/pam.d/oxyos`).
const PAM_SERVICE: &str = "oxyos";
/// A shell that exits while locked is started again, but no sooner than this after the last.
const SHELL_RESTART: Duration = Duration::from_secs(1);
/// The longest a turn waits when nothing is due: the session wakes for its devices, its clients,
/// its displays and its children exiting, and for what is due on a clock (the sun an automatic
/// appearance follows, a shell started again); this only bounds a wait that something missed.
const IDLE_WAIT: Duration = Duration::from_secs(60);

fn main() -> Result<()> {
    // Before any thread or child: nothing the session starts inherits the service manager's
    // socket.
    let notify = oxyos_seat::notify::Notify::take();
    let mut args = std::env::args_os().skip(1);
    let mut layout: Option<oxyos_input::Layout> = None;
    let mut command: Vec<OsString> = Vec::new();
    let mut shell_command: Option<Vec<OsString>> = None;
    let mut pointer: Option<(PathBuf, u32)> = None;
    let mut reader_program: Option<OsString> = std::env::var_os("OXYOS_READER");
    let mut reader_on = false;
    let mut greeter = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--greeter") => greeter = true,
            Some("--pointer") => {
                let value = args
                    .next()
                    .and_then(|v| v.into_string().ok())
                    .ok_or(USAGE)?;
                let (path, size) = match value.rsplit_once(',') {
                    Some((path, size)) => (path.to_string(), size.parse().map_err(|_| USAGE)?),
                    None => (value, 32),
                };
                pointer = Some((PathBuf::from(path), size));
            }
            Some("--shell") => {
                let value = args
                    .next()
                    .and_then(|v| v.into_string().ok())
                    .ok_or(USAGE)?;
                shell_command = Some(value.split(',').map(OsString::from).collect());
            }
            Some("--reader") => reader_program = Some(args.next().ok_or(USAGE)?),
            Some("--reader-on") => reader_on = true,
            Some("--layout") => {
                layout = Some(oxyos_input::Layout::named(
                    &args
                        .next()
                        .and_then(|layout| layout.into_string().ok())
                        .ok_or(USAGE)?,
                ))
            }
            Some("--") => {
                command = args.collect();
                break;
            }
            _ => return Err(USAGE.into()),
        }
    }
    // A session shows something: its shell, an application, or both.
    if command.is_empty() && shell_command.is_none() {
        return Err(USAGE.into());
    }
    if reader_on && reader_program.is_none() {
        return Err(USAGE.into());
    }
    // The login screen is its shell alone.
    if greeter && (shell_command.is_none() || !command.is_empty()) {
        return Err(USAGE.into());
    }
    // The user's preferences; what the file does not say (or is not there) is the default.
    let mut settings = Settings::session();
    // The screen reader the user keeps on starts with the session, when there is one to start.
    let reader_on = reader_on || settings.preferences().screen_reader && reader_program.is_some();

    // The lock's authentication path first: without one the session never starts, as it could
    // lock and never unlock. The login screen's is the login program.
    let mut login_program = None;
    let authentication: Box<dyn Authentication> = if greeter {
        let socket = std::env::var_os("OXYOS_LOGIN_SOCKET")
            .map_or_else(|| PathBuf::from(oxyos_auth::greeter::SOCKET), PathBuf::from);
        let connection = oxyos_auth::greeter::Greeter::connect(&socket)
            .map_err(|error| format!("the login program at {}: {error}", socket.display()))?;
        eprintln!(
            "[session] the login screen: passwords go to the login program ({})",
            socket.display()
        );
        login_program = Some(connection.on_screen()?);
        Box::new(oxyos_compositor::login::LoginProgram(connection))
    } else {
        let pam = oxyos_auth::Authenticator::for_session(PAM_SERVICE)?;
        eprintln!(
            "[session] the lock checks {}'s password with PAM ({PAM_SERVICE})",
            pam.user().to_string_lossy()
        );
        let user = pam.user().to_owned();
        Box::new(Pam {
            checker: oxyos_auth::Checker::spawn(pam)?,
            user,
        })
    };
    let seat = Rc::new(oxyos_seat::Seat::take()?);
    eprintln!("[session] logind session on terminal {:?}", seat.vt());
    // Sleep waits for the lock to be on screen; the login screen has nothing to protect.
    if !greeter {
        if let Err(error) = seat.hold_sleep() {
            eprintln!("[session] holding sleep back: {error}; sleep will not wait for the lock");
        }
    }
    let (card, card_path) = open_card(&seat)?;
    let own_uid = rustix::process::getuid().as_raw();
    let mut compositor = Compositor::new(Scene::new(), own_uid);
    // A session waits for its devices, clients and displays rather than turning every 5 ms; at
    // most a quarter of a second while it starts. Its children exiting wake it: SIGCHLD writes to
    // a socket its turns wait on, which the loop reads before it looks at them (`children_exited`).
    compositor.idle_wait = Duration::from_millis(250);
    let children_exited = {
        let (read, write) = std::os::unix::net::UnixStream::pair()?;
        read.set_nonblocking(true)?;
        signal_hook::low_level::pipe::register(signal_hook::consts::SIGCHLD, write)?;
        compositor.wakers.push(read.try_clone()?.into());
        read
    };
    compositor.records = false;
    // What it does record: a turn that took longer than a refresh, and where the time went.
    compositor.slow_turns = true;
    // Applied before any client connects, so each is told them with its Welcome.
    settings.apply(&mut compositor, settings::now());
    // The machine's volumes, backlights, battery and network, on a thread of their own: every
    // call may wait on another process (ADR 0025).
    match oxyos_controls::Worker::spawn(Box::new(oxyos_controls::Session::new())) {
        Ok(worker) => {
            compositor.controls = Some(oxyos_compositor::controls::Controls::new(
                worker,
                Instant::now(),
            ))
        }
        Err(error) => eprintln!("[session] the hardware keys: {error}; they do nothing"),
    }
    compositor.reads_back = false;
    compositor.authentication = Some(authentication);
    if greeter {
        // Locked before any client connects: nothing but the login screen is ever shown.
        compositor.login = true;
        compositor.scene.lock();
    }
    compositor.card_device = Some(oxyos_seat::Device::of(&card_path)?);
    compositor.seat = Some(seat.clone());
    // On the GPU straight into the displays' buffers where the device exports them.
    match oxyos_gpu::VulkanCompositor::new() {
        Ok(gpu) if gpu.exports_dma_buf() => {
            eprintln!(
                "[session] composing on {} ({:?}) into the displays' buffers, {}",
                gpu.device_name(),
                gpu.device_kind(),
                if gpu.exports_sync_file() {
                    "each handed to the display with its completion (IN_FENCE_FD)"
                } else {
                    "each waited for before the display has it"
                }
            );
            compositor.gpu_only = true;
            compositor.gpu = Some(Gpu {
                compositor: gpu,
                frames: 0,
                mismatches: 0,
                device_ms: Vec::new(),
                readback_ms: Vec::new(),
                host_ms: Vec::new(),
                uploaded: Vec::new(),
                phases_ms: Vec::new(),
            });
        }
        Ok(gpu) => eprintln!(
            "[session] {} exports no DMA-BUF: composing on the CPU",
            gpu.device_name()
        ),
        Err(error) => eprintln!("[session] no Vulkan device ({error}): composing on the CPU"),
    }
    // Every connected output, shown and arranged as the user's preferences keep them (ADR 0018).
    let mut card = Card {
        card,
        path: card_path,
        pointer,
    };
    let mut displays = Displays::new(1);
    displays.follow(&mut card, &mut compositor, settings.preferences())?;
    if compositor.screens.is_empty() {
        return Err(format!("{}: no output could be shown", card.path.display()).into());
    }
    // The card's connectors changing: monitors plugged in and out.
    let name = card.path.file_name().and_then(|name| name.to_str());
    match name.map(oxyos_kms::Hotplug::open) {
        Some(Ok(hotplug)) => compositor.hotplug = Some(hotplug),
        Some(Err(error)) => {
            eprintln!("[session] hotplug events: {error}; monitors plugged in are not seen")
        }
        None => {}
    }
    // The Oxy mark on black while the session starts, as a Mac shows its own (ADR 0011).
    compositor.show_splash();
    // Its first frame on a display: the service manager that started the session lets the boot
    // logo go (ADR 0017). Failing to tell it never ends the session.
    if let Some(notify) = notify {
        compositor.on_first_flip = Some(Box::new(move || match notify.ready() {
            Ok(()) => eprintln!("[session] on screen: the service manager is told (READY=1)"),
            Err(error) => eprintln!("[session] telling the service manager it is ready: {error}"),
        }));
    } else if let Some(login_program) = login_program {
        // The login screen runs outside the login program's unit: the login program tells it.
        compositor.on_first_flip = Some(Box::new(move || {
            if let Err(error) = login_program.tell() {
                eprintln!("[session] telling the login program it is on screen: {error}");
            }
        }));
    }

    // The socket applications connect to, in the user's runtime directory.
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or("no XDG_RUNTIME_DIR: run oxyos-session in a login session")?;
    let socket = runtime.join(format!("oxyos-{}", std::process::id()));
    let listener = Listener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    let _socket = RemoveOnDrop(socket.clone());

    let open_seat = seat.clone();
    let close_seat = seat.clone();
    let open: oxyos_input::Open = Box::new(move |path| open_seat.open(path).map(|(fd, _)| fd));
    let close: oxyos_input::Close = Box::new(move |fd| {
        use std::os::fd::AsFd;
        if let Ok(device) = oxyos_seat::Device::of_fd(fd.as_fd()) {
            let _ = close_seat.close(device);
        }
    });
    let layout = layout
        .or_else(oxyos_input::Layout::system)
        .unwrap_or_else(|| oxyos_input::Layout::named("us"));
    // The pointer moves over every output: absolute devices span their bounding box.
    let bounds = compositor
        .screens
        .iter()
        .fold((0, 0), |(width, height), screen| {
            (
                width.max(screen.rect.right().max(0) as u32),
                height.max(screen.rect.bottom().max(0) as u32),
            )
        });
    let input = oxyos_input::Input::open_with(bounds, &layout, Some((open, close)))?;
    eprintln!("[session] input from seat0, layout {layout}");
    compositor.input = Some(input);
    compositor.confine_pointer();

    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(signal, stop.clone())?;
    }
    // The session's shells first, on their own socket, granted `shell`: one on each output.
    let shell_socket = runtime.join(format!("oxyos-{}-shell", std::process::id()));
    let _shell_socket = RemoveOnDrop(shell_socket.clone());
    let mut shells: BTreeMap<OutputId, std::process::Child> = BTreeMap::new();
    if let Some(shell_command) = &shell_command {
        shells = start_shells(&mut compositor, &listener, &shell_socket, shell_command)?;
    }
    let mut signed_in: Option<Instant> = None;
    // The screen reader's socket, only when the session has one (ADR 0014).
    let reader_socket = runtime.join(format!("oxyos-{}-reader", std::process::id()));
    let _reader_socket = RemoveOnDrop(reader_socket.clone());
    if reader_program.is_some() {
        let reader_listener = Listener::bind(&reader_socket)?;
        reader_listener.set_nonblocking(true)?;
        compositor.reader_listener = Some(reader_listener);
    }
    let mut reader = match (&reader_program, reader_on) {
        (Some(program), true) => Some(spawn_reader(&reader_socket, program)?),
        _ => None,
    };
    // The session is on screen: the mark fades out over it.
    compositor.end_splash();
    let mut application = match command.first() {
        Some(program) => Some(
            Command::new(program)
                .args(&command[1..])
                .env("OXYOS_COMPOSITOR_SOCKET", &socket)
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY")
                .spawn()?,
        ),
        None => None,
    };
    // The applications the shell asks for (ADR 0013). The image's policy of system applications
    // is read at each launch; one that cannot be used is said now, and grants nothing (ADR 0030).
    if let Err(error) = oxyos_apps::Policy::system() {
        eprintln!("oxyos-session: system applications: {error}");
    }
    let env_path = |name| std::env::var_os(name).map(PathBuf::from);
    // The applications the install service installed for this account are the registry's second
    // source, and each version launched is held, then noted shown and exited (ADR 0030, decisions
    // 7 and 10). The login screen launches nothing and holds nothing.
    let installed = oxyos_apps::session_user().map(|user| {
        oxyos_apps::installed_applications(std::path::Path::new(oxyos_apps::INSTALLED), &user)
    });
    let installd =
        (!greeter).then(|| installd::Installd::start(PathBuf::from(oxyos_install::ipc::SOCKET)));
    let mut versions = Versions::default();
    let mut launcher = oxyos_apps::Launcher::new(
        oxyos_apps::Runtime {
            socket: socket.clone(),
            bloom_host: env_path("OXYOS_BLOOM_HOST"),
            bloom_bundle: env_path("OXYOS_BLOOM_BUNDLE"),
            // The image's runtimes, never an environment variable's (ADR 0030, decision 2).
            runtimes: PathBuf::from(oxyos_apps::RUNTIMES),
            window: (960, 600),
            env: Vec::new(),
        },
        oxyos_apps::directories(),
        oxyos_apps::languages(),
    )
    .with_installed(installed);
    let mut shell_started = Instant::now();
    let ended = loop {
        // What woke the turn of children exiting: they are looked at below, every turn.
        drain(&children_exited);
        // Monitors plugged in and out (ADR 0018): an output added gets a shell of its own; one
        // removed, its shell stopped.
        if compositor.take_hotplug() {
            let (added, removed) =
                displays.follow(&mut card, &mut compositor, settings.preferences())?;
            for output in removed {
                if let Some(mut shell) = shells.remove(&output) {
                    end(&mut shell)?;
                }
            }
            if let Some(shell_command) = &shell_command {
                for output in added {
                    shells.insert(output, spawn_shell(&shell_socket, shell_command, output)?);
                }
            }
        }
        for (pid, launched) in launcher.reap() {
            eprintln!("[session] {} exited", launched.id);
            if let Some(note) = versions.exited(pid, &launched) {
                installd.as_ref().inspect(|i| i.note(note));
            }
        }
        // An installed version showing its first window (ADR 0030, decision 10).
        if versions.awaiting() {
            for note in versions.shown(&compositor.pids_showing_windows()) {
                installd.as_ref().inspect(|i| i.note(note));
            }
        }
        for request in compositor.scene.take_launches() {
            let launched = if greeter {
                // The login screen starts nothing.
                eprintln!("[session] not started: the login screen launches nothing");
                false
            } else {
                match launcher.launch(&request.id) {
                    Ok(pid) => {
                        eprintln!("[session] started {} ({pid})", request.id);
                        if let Some(note) = launcher
                            .application_of(pid)
                            .and_then(|l| versions.launched(pid, l))
                        {
                            installd.as_ref().inspect(|i| i.note(note));
                        }
                        true
                    }
                    Err(error) => {
                        eprintln!("[session] not started: {error}");
                        false
                    }
                }
            };
            compositor
                .scene
                .launched(request.client, request.serial, launched);
        }
        // A notification pressed while its application ran nowhere (ADR 0026): the session starts
        // it, and its first client is given the response.
        for id in compositor.scene.take_notification_launches() {
            let started = !greeter
                && match launcher.launch(&id) {
                    Ok(pid) => {
                        eprintln!("[session] started {id} ({pid}) for a notification");
                        if let Some(note) = launcher
                            .application_of(pid)
                            .and_then(|l| versions.launched(pid, l))
                        {
                            installd.as_ref().inspect(|i| i.note(note));
                        }
                        true
                    }
                    Err(error) => {
                        eprintln!("[session] not started for a notification: {error}");
                        false
                    }
                };
            if !started {
                compositor.scene.notification_launch_failed(&id);
            }
        }
        if let Some(status) = reader
            .as_mut()
            .map(|reader| reader.try_wait())
            .transpose()?
            .flatten()
        {
            eprintln!("[session] the screen reader exited ({status})");
            reader = None;
        }
        if compositor.take_reader_toggle() {
            let on = reader.is_none();
            switch_reader(&mut reader, reader_program.as_ref(), &reader_socket, on)?;
        }
        // The changes Settings asked for, and the sun an automatic appearance follows.
        for key in settings.serve(&mut compositor, settings::now()) {
            eprintln!("[session] {key} changed");
            if key == "screen_reader" {
                let on = settings.preferences().screen_reader;
                switch_reader(&mut reader, reader_program.as_ref(), &reader_socket, on)?;
            }
        }
        // The displays' changes Settings asked for, and a mode on trial going back once its time
        // is up (ADR 0018).
        displays.serve(&mut card, &mut compositor, &mut settings, Instant::now());
        // The windows of what it started are their registry application's (ADR 0013).
        // The policy's Settings is given `settings` (ADR 0019, ADR 0030).
        compositor.name_clients(&|pid| {
            launcher.application_of(pid).map(|launched| LaunchRecord {
                application: launched.id.clone(),
                grants: Grants {
                    settings: launched.grants.settings,
                    ..Grants::default()
                },
            })
        });
        if let Some(status) = application
            .as_mut()
            .map(|application| application.try_wait())
            .transpose()?
            .flatten()
        {
            eprintln!(
                "[session] {} exited ({status})",
                command[0].to_string_lossy()
            );
            break Ok(());
        }
        let mut exited = None;
        for (output, shell) in shells.iter_mut() {
            if let Some(status) = shell.try_wait()? {
                exited = Some((*output, status));
                break;
            }
        }
        if let Some((output, status)) = exited {
            if !compositor.scene.is_locked() {
                eprintln!("[session] the shell of output {output} exited ({status})");
                break Ok(());
            }
            // Failing closed: the session stays locked, under the covers, until the shell is
            // back and shows the lock screen.
            eprintln!("[session] the shell of output {output} exited ({status}) while locked");
            shells.remove(&output);
        }
        if let Some(shell_command) = &shell_command {
            let missing = compositor
                .screens
                .iter()
                .map(|screen| screen.id)
                .find(|output| !shells.contains_key(output));
            if let Some(output) = missing.filter(|_| shell_started.elapsed() >= SHELL_RESTART) {
                eprintln!("[session] starting the shell of output {output} again");
                shell_started = Instant::now();
                shells.insert(output, spawn_shell(&shell_socket, shell_command, output)?);
            }
        }
        if compositor.ending || stop.load(Ordering::Relaxed) {
            eprintln!("[session] ending");
            break Ok(());
        }
        // Signed in at the login screen: once the splash is shown, it ends.
        if compositor.signed_in {
            let since = *signed_in.get_or_insert_with(Instant::now);
            if since.elapsed() >= SIGNED_IN_SPLASH {
                eprintln!("[session] signed in: the login screen ends");
                break Ok(());
            }
        }
        // The turn waits until the next thing due on a clock, if anything is sooner than IDLE_WAIT:
        // the login screen's splash ending, a shell started again, the sun crossing the horizon,
        // a display's mode on trial going back.
        let mut wait = IDLE_WAIT;
        if let Some(deadline) = displays.trial_deadline() {
            wait = wait.min(deadline.saturating_duration_since(Instant::now()));
        }
        if let Some(since) = signed_in {
            wait = wait.min(SIGNED_IN_SPLASH.saturating_sub(since.elapsed()));
        }
        let shell_missing = shell_command.is_some()
            && compositor
                .screens
                .iter()
                .any(|screen| !shells.contains_key(&screen.id));
        if shell_missing {
            wait = wait.min(SHELL_RESTART.saturating_sub(shell_started.elapsed()));
        }
        if let Some(crossing) = settings.next_crossing() {
            let seconds = u64::try_from(crossing.saturating_sub(settings::now())).unwrap_or(0);
            wait = wait.min(Duration::from_secs(seconds));
        }
        compositor.idle_wait = wait;
        if let Err(error) = compositor.serve(&listener) {
            break Err(error);
        }
    };
    if let Some(application) = application.as_mut() {
        end(application)?;
    }
    launcher.end(Duration::from_secs(2));
    for shell in shells.values_mut() {
        end(shell)?;
    }
    if let Some(reader) = reader.as_mut() {
        end(reader)?;
    }
    compositor.shell_listener = None;
    compositor.reader_listener = None;
    // The last frame completes before the display and the devices are given back.
    compositor.complete_flip(true);
    compositor.input = None;
    for screen in &mut compositor.screens {
        screen.display = None;
    }
    compositor.seat = None;
    ended
}

/// Reads what woke a turn from a waker (`Compositor::wakers`) until nothing is left.
fn drain(mut waker: &std::os::unix::net::UnixStream) {
    let mut buffer = [0u8; 64];
    while matches!(waker.read(&mut buffer), Ok(read) if read > 0) {}
}

/// Starts the session's shells on `socket`, granting `shell`, one on each output, and serves the
/// session until each one's panels have set its output's work area (it differs from the output);
/// the login screen's, until each output shows its lock surface.
fn start_shells(
    compositor: &mut Compositor,
    listener: &Listener,
    socket: &std::path::Path,
    command: &[OsString],
) -> Result<BTreeMap<OutputId, std::process::Child>> {
    let shell_listener = Listener::bind(socket)?;
    shell_listener.set_nonblocking(true)?;
    compositor.shell_listener = Some(shell_listener);
    let mut shells = BTreeMap::new();
    for screen in &compositor.screens {
        shells.insert(screen.id, spawn_shell(socket, command, screen.id)?);
    }
    let deadline = Instant::now() + SHELL_DEADLINE;
    loop {
        for (output, shell) in shells.iter_mut() {
            if let Some(status) = shell.try_wait()? {
                return Err(format!(
                    "the shell of output {output} exited ({status}) before its panels were shown"
                )
                .into());
            }
        }
        if Instant::now() > deadline {
            for shell in shells.values_mut() {
                end(shell)?;
            }
            return Err("the shells' panels were not shown in time".into());
        }
        compositor.serve(listener)?;
        let shown = if compositor.login {
            lock_shown(compositor)
        } else {
            compositor.scene.outputs().iter().all(|output| {
                compositor
                    .scene
                    .work_area(output.id)
                    .is_some_and(|area| area != output.rect)
            })
        };
        if shown {
            eprintln!("[session] the shells' panels are shown");
            return Ok(shells);
        }
    }
}

/// Whether every output shows its lock surface (the login screen), none the compositor's cover.
fn lock_shown(compositor: &Compositor) -> bool {
    compositor.scene.is_locked()
        && !compositor
            .scene
            .render_list()
            .iter()
            .any(|item| matches!(item, oxyos_compositor_core::RenderItem::Cover { .. }))
}

/// The installed versions the session launched, by process, until each showed a window and
/// exited (docs/adr/0030-installing-applications.mdx, decision 10).
#[derive(Default)]
struct Versions {
    /// Each process of an installed version, and whether it showed a window.
    running: BTreeMap<u32, (String, u64, bool)>,
}

impl Versions {
    /// A process launched: held, if it is an installed version.
    fn launched(&mut self, pid: u32, launched: &oxyos_apps::Launched) -> Option<installd::Note> {
        let oxyos_apps::Origin::Installed { version_code } = launched.origin else {
            return None;
        };
        self.running
            .insert(pid, (launched.id.clone(), version_code, false));
        Some(installd::Note::Hold {
            id: launched.id.clone(),
            version_code,
        })
    }

    fn awaiting(&self) -> bool {
        self.running.values().any(|(_, _, shown)| !shown)
    }

    /// Each process showing its first window, of those `showing` names.
    fn shown(&mut self, showing: &std::collections::BTreeSet<u32>) -> Vec<installd::Note> {
        let mut notes = Vec::new();
        for (pid, (id, version_code, shown)) in self.running.iter_mut() {
            if !*shown && showing.contains(pid) {
                *shown = true;
                notes.push(installd::Note::Shown {
                    id: id.clone(),
                    version_code: *version_code,
                });
            }
        }
        notes
    }

    /// A process exited: whether it showed a window, for the service to keep or roll back.
    fn exited(&mut self, pid: u32, launched: &oxyos_apps::Launched) -> Option<installd::Note> {
        let (id, version_code, shown) = self.running.remove(&pid)?;
        debug_assert_eq!(id, launched.id);
        Some(installd::Note::Exited {
            id,
            version_code,
            shown,
        })
    }
}

/// The session's shell of `output`, on `socket`.
fn spawn_shell(
    socket: &std::path::Path,
    command: &[OsString],
    output: OutputId,
) -> Result<std::process::Child> {
    let mut shell = Command::new(command.first().ok_or(USAGE)?);
    shell
        .args(&command[1..])
        .arg("--shell")
        .arg(output.to_string())
        .env("OXYOS_COMPOSITOR_SOCKET", socket)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    // A Bloom host's bundle is known to its JavaScript by its source's URL, beside its assets: a
    // package's compiled bundle by the source its manifest names (ADR 0030).
    if let Some(bundle) = command
        .get(1)
        .map(std::path::Path::new)
        .filter(|b| b.is_file())
    {
        let url = oxyos_apps::script_url(bundle);
        shell.env("OXYOS_SCRIPT_URL", format!("file://{}", url.display()));
    }
    Ok(shell.spawn()?)
}

/// Starts the screen reader (`on`) or stops it, if it is not so already: the user's switch
/// (Super+F5) or the preference.
fn switch_reader(
    reader: &mut Option<std::process::Child>,
    program: Option<&OsString>,
    socket: &std::path::Path,
    on: bool,
) -> Result<()> {
    match (reader.take(), program, on) {
        (Some(mut running), _, false) => {
            eprintln!("[session] stopping the screen reader");
            end(&mut running)?;
        }
        (None, Some(program), true) => {
            eprintln!("[session] starting the screen reader");
            *reader = Some(spawn_reader(socket, program)?);
        }
        (None, None, true) => eprintln!("[session] no screen reader (--reader)"),
        (running, _, _) => *reader = running,
    }
    Ok(())
}

/// The session's screen reader, on `socket`, in the session's language.
fn spawn_reader(socket: &std::path::Path, program: &OsString) -> Result<std::process::Child> {
    Ok(Command::new(program)
        .env("OXYOS_COMPOSITOR_SOCKET", socket)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .spawn()?)
}

/// The session's authentication path (ADR 0010): PAM, off the compositor's thread, throttled,
/// for the session's own user.
struct Pam {
    checker: oxyos_auth::Checker,
    user: std::ffi::CString,
}

/// Whole seconds, rounded up: a wait of 0.2 s is still a wait.
fn seconds(wait: Duration) -> u32 {
    u32::try_from(wait.as_millis().div_ceil(1000)).unwrap_or(u32::MAX)
}

impl Authentication for Pam {
    fn attempt(
        &mut self,
        account: Option<&str>,
        mut secret: oxyos_compositor_core::scene::Secret,
    ) -> Option<(AuthOutcome, u32)> {
        // The lock checks the session's own user's password, never another account's.
        if account.is_some_and(|account| account.as_bytes() != self.user.to_bytes()) {
            return Some((AuthOutcome::Rejected, 0));
        }
        // The password's memory moves to PAM's side whole, and is overwritten there.
        let secret = oxyos_auth::Secret::new(std::mem::take(&mut *secret));
        match self.checker.attempt(secret, Instant::now()) {
            oxyos_auth::Attempt::Checking => None,
            oxyos_auth::Attempt::Busy => Some((AuthOutcome::Busy, 0)),
            oxyos_auth::Attempt::Throttled(wait) => Some((AuthOutcome::Throttled, seconds(wait))),
        }
    }

    fn as_fd(&self) -> BorrowedFd<'_> {
        self.checker.as_fd()
    }

    fn answer(&mut self) -> Option<(AuthOutcome, u32)> {
        let answer = self.checker.answer(Instant::now())?;
        let outcome = match answer.outcome {
            oxyos_auth::Outcome::Accepted => AuthOutcome::Accepted,
            oxyos_auth::Outcome::Rejected => AuthOutcome::Rejected,
        };
        Some((outcome, seconds(answer.wait)))
    }
}

/// The pointer's images on a display's cursor plane, by shape: from a cursor file (the arrow) or
/// a cursor scheme's folder (every shape its `Install.inf` names), each the image closest to its
/// size at the display's scale (in 120ths), else the display's own arrow. Without a cursor
/// plane, or with files it cannot read, the session says so and goes on.
fn show_pointer(
    display: &mut oxyos_kms::Display,
    pointer: Option<&(PathBuf, u32)>,
    scale_120: u32,
) -> BTreeMap<PointerShape, Vec<(usize, Duration)>> {
    let mut images = BTreeMap::new();
    let Some(max) = display.pointer_size() else {
        eprintln!("[session] the display has no cursor plane: the pointer is not shown on it");
        return images;
    };
    let Some((path, size)) = pointer else {
        return images;
    };
    let size = &oxyos_compositor::scaled(*size, scale_120);
    let files = if path.is_dir() {
        match cursor_file::scheme(path) {
            Ok(files) => files,
            Err(error) => {
                eprintln!("[session] the cursor scheme {}: {error}", path.display());
                return images;
            }
        }
    } else {
        vec![(PointerShape::Arrow, path.clone())]
    };
    for (shape, file) in files {
        // An animated cursor's every step is an image of the display's.
        let steps = std::fs::read(&file)
            .map_err(|error| error.to_string())
            .and_then(|bytes| cursor_file::read_any(&bytes, *size, max))
            .and_then(|steps| {
                steps
                    .into_iter()
                    .map(|(image, time)| {
                        display
                            .add_pointer_image(&image)
                            .map(|index| (index, time))
                            .map_err(|error| error.to_string())
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()
            });
        match steps {
            Ok(steps) => {
                images.insert(shape, steps);
            }
            Err(error) => eprintln!(
                "[session] the {shape:?} pointer from {}: {error}; the arrow is shown instead",
                file.display()
            ),
        }
    }
    eprintln!(
        "[session] {} pointer shapes from {}",
        images.len(),
        path.display()
    );
    images
}

/// Asks the application to stop (SIGTERM), and stops it if it has not within two seconds.
fn end(application: &mut std::process::Child) -> Result<()> {
    if application.try_wait()?.is_some() {
        return Ok(());
    }
    if let Some(pid) = rustix::process::Pid::from_raw(application.id() as i32) {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if application.try_wait()?.is_some() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    application.kill()?;
    application.wait()?;
    Ok(())
}

/// The first card with a connected output, opened through the seat, and its path.
fn open_card(seat: &oxyos_seat::Seat) -> Result<(Rc<oxyos_kms::Card>, PathBuf)> {
    let mut cards: Vec<PathBuf> = std::fs::read_dir("/dev/dri")?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("card"))
        })
        .collect();
    cards.sort();
    let mut last = None;
    for path in cards {
        let opened = seat.open(&path).and_then(|(fd, inactive)| {
            if inactive {
                return Err(std::io::Error::other(
                    "the session is not active: its terminal must be shown",
                ));
            }
            let card = oxyos_kms::Card::from_fd(fd)?;
            let connected = card
                .outputs()?
                .iter()
                .any(|output| output.connected && !output.modes.is_empty());
            if !connected {
                return Err(std::io::Error::other("no connected output"));
            }
            Ok(card)
        });
        match opened {
            Ok(card) => return Ok((Rc::new(card), path)),
            Err(error) => {
                if let Ok(device) = oxyos_seat::Device::of(&path) {
                    let _ = seat.close(device);
                }
                last = Some(format!("{}: {error}", path.display()));
            }
        }
    }
    Err(last.unwrap_or_else(|| "no DRM card".to_string()).into())
}

/// The session's card (docs/adr/0018-several-displays.mdx): each output it shows is a display on
/// a CRTC and planes of its own, with its own scanout buffers on the GPU and its pointer's images.
struct Card {
    card: Rc<oxyos_kms::Card>,
    path: PathBuf,
    /// `--pointer`: the cursor file or scheme each display's pointer images come from.
    pointer: Option<(PathBuf, u32)>,
}

/// A connector made ready to show an output: its display, scanout buffers and pointer images.
struct Head {
    display: oxyos_kms::Display,
    scanout: Vec<(usize, oxyos_kms::Framebuffer)>,
    pointer: BTreeMap<PointerShape, Vec<(usize, Duration)>>,
}

impl Heads for Card {
    type Head = Head;

    fn connectors(&mut self) -> Result<Vec<Connector>> {
        Ok(self.card.outputs()?.iter().map(Connector::of).collect())
    }

    /// A display on the connector at the mode, on a CRTC and planes no other display uses, which
    /// the card must take (an atomic test) before anything is shown on it.
    fn prepare(
        &mut self,
        compositor: &mut Compositor,
        connector: &Connector,
        mode: &ModeInfo,
        scale_120: u32,
    ) -> std::result::Result<Head, String> {
        let in_use: Vec<u32> = compositor
            .screens
            .iter()
            .filter_map(|screen| screen.display.as_ref())
            .flat_map(|display| display.objects())
            .collect();
        let wanted = oxyos_kms::Mode {
            width: u16::try_from(mode.width).map_err(|_| "a mode too wide")?,
            height: u16::try_from(mode.height).map_err(|_| "a mode too tall")?,
            refresh_hz: mode.refresh_mhz.div_ceil(1000),
            refresh_mhz: mode.refresh_mhz,
            preferred: mode.preferred,
        };
        let mut display = oxyos_kms::Display::on_connector(
            self.card.clone(),
            connector.id,
            Some(&wanted),
            &in_use,
        )
        .map_err(|error| error.to_string())?;
        display
            .test()
            .map_err(|error| format!("the card refuses {}: {error}", display.mode_name()))?;
        let pointer = show_pointer(&mut display, self.pointer.as_ref(), scale_120);
        // Its own scanout buffers on the GPU, taking turns.
        let mut scanout = Vec::new();
        if let (Some(gpu), true) = (&mut compositor.gpu, compositor.gpu_only) {
            use std::os::fd::AsFd;
            let (width, height) = display.size();
            let targets = gpu
                .compositor
                .add_scanout_targets(2, width, height)
                .map_err(|error| format!("its scanout buffers: {error}"))?;
            let indices: Vec<usize> = targets.iter().map(|(index, _)| *index).collect();
            let imported = targets
                .iter()
                .map(|(index, target)| {
                    display
                        .card()
                        .import_dmabuf(target.fd.as_fd(), target.width, target.height, target.pitch)
                        .map(|framebuffer| (*index, framebuffer))
                })
                .collect::<std::io::Result<Vec<_>>>();
            match imported {
                Ok(imported) => scanout = imported,
                Err(error) => {
                    gpu.compositor.remove_scanout_targets(&indices);
                    return Err(format!("its scanout buffers: {error}"));
                }
            }
        }
        Ok(Head {
            display,
            scanout,
            pointer,
        })
    }

    fn attach(&mut self, compositor: &mut Compositor, output: OutputId, head: Head) {
        if let Some(screen) = compositor.screen_mut(output) {
            screen.display = Some(head.display);
            screen.scanout = head.scanout;
            screen.pointer_images = head.pointer;
        }
    }

    /// The pointer's images chosen again for the display's new scale.
    fn rescale(&mut self, compositor: &mut Compositor, output: OutputId, scale_120: u32) {
        let pointer = self.pointer.clone();
        if let Some(screen) = compositor.screen_mut(output) {
            if let Some(display) = screen.display.as_mut() {
                display.clear_pointer_images();
                screen.pointer_images = show_pointer(display, pointer.as_ref(), scale_120);
            }
        }
    }
}

/// Removes the socket when the session ends.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
