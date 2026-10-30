//! The OxyOS session: the compositor (`oxyos-compositor`) on a real seat, running the
//! applications it launches. It runs as the user of a logind session on a seat, from a login on
//! a terminal, never as root (docs/adr/0006-seat.mdx):
//! - its display is the first card with a connected output, at that output's size;
//! - frames are composed on the GPU straight into the display's scanout buffers where Vulkan
//!   exports DMA-BUF, otherwise on the CPU, only what changed (docs/adr/0005-damage.mdx);
//! - its input is the seat's devices through libinput, with the system's keyboard
//!   (`/etc/default/keyboard`: the layout, variant and options passwords are typed with at a
//!   login, so the lock screen takes the same password), or `--layout`'s;
//! - Ctrl+Alt+Fn switches terminals, and logind's switches are followed;
//! - the session locks (docs/adr/0010-lock-screen.mdx) on Ctrl+Super+Q, at the shell's request,
//!   when logind asks and before the machine sleeps; only a password PAM accepts for the session's
//!   user, under the service `oxyos`, unlocks it.
//!
//! `oxyos-session [--layout LAYOUT] [--pointer FILE.cur[,SIZE]] [--shell PROGRAM,ARG...]
//! [--reader PROGRAM [--reader-on]] [-- COMMAND [ARGS]...]` runs COMMAND
//! with `OXYOS_COMPOSITOR_SOCKET` naming the compositor. With `--shell`, the session's shell
//! (ADR 0009) starts first: PROGRAM with its ARGs, then `--shell OUTPUT`, on a socket of its own
//! that grants `shell` and `lock` (the shell is the lock screen as well); COMMAND, if given,
//! starts once the shell's panels have set the work area its window is placed in. With a shell
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
//! `$XDG_CONFIG_HOME/oxyos/preferences` as the session starts: reduce motion takes the
//! transitions' time away, and the screen reader preference starts the reader as `--reader-on`
//! does.
//!
//! The pointer is shown on the display's cursor plane, in the shape the surface under it wants:
//! the display's own arrow, or the images of a Windows cursor file or cursor scheme's folder
//! (`--pointer`, every shape its `Install.inf` names) closest to SIZE pixels (32 by default).
//!
//! Started at boot by a unit of `Type=notify` (docs/adr/0017-boot-logo.mdx), the session sends
//! `READY=1` once its first frame, the splash, has flipped on the display; the unit then lets the
//! boot logo go, which showed the same pixels until then.

mod cursor_file;
mod notify;

use oxyos_compositor::{Authentication, Compositor, Gpu, Result};
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{AuthOutcome, PointerShape, Rect};
use rustix::fd::BorrowedFd;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const USAGE: &str = "usage: oxyos-session [--layout LAYOUT] [--pointer FILE.cur|SCHEME[,SIZE]] \
                     [--shell PROGRAM,ARG...] [--reader PROGRAM [--reader-on]] [-- COMMAND [ARGS]...] \
                     (COMMAND is required without a shell)";
/// The shell's panels set the work area within this budget, or the session does not start.
const SHELL_DEADLINE: Duration = Duration::from_secs(60);
const OUTPUT_ID: u32 = 1;
/// The PAM service the lock checks passwords under (`/etc/pam.d/oxyos`).
const PAM_SERVICE: &str = "oxyos";
/// A shell that exits while locked is started again, but no sooner than this after the last.
const SHELL_RESTART: Duration = Duration::from_secs(1);

fn main() -> Result<()> {
    // Before any thread or child: nothing the session starts inherits the service manager's
    // socket.
    let notify = notify::Notify::take();
    let mut args = std::env::args_os().skip(1);
    let mut layout: Option<oxyos_input::Layout> = None;
    let mut command: Vec<OsString> = Vec::new();
    let mut shell_command: Option<Vec<OsString>> = None;
    let mut pointer: Option<(PathBuf, u32)> = None;
    let mut reader_program: Option<OsString> = std::env::var_os("OXYOS_READER");
    let mut reader_on = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
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
    // The user's preferences; what the file does not say (or is not there) is the default.
    let preferences = oxyos_preferences::session_path()
        .map(|path| oxyos_preferences::Preferences::load(&path))
        .unwrap_or_default();
    // The screen reader the user keeps on starts with the session, when there is one to start.
    let reader_on = reader_on || preferences.screen_reader && reader_program.is_some();

    // The lock's authentication path first: without one the session never starts, as it could
    // lock and never unlock.
    let pam = oxyos_auth::Authenticator::for_session(PAM_SERVICE)?;
    eprintln!(
        "[session] the lock checks {}'s password with PAM ({PAM_SERVICE})",
        pam.user().to_string_lossy()
    );
    let pam = Pam(oxyos_auth::Checker::spawn(pam)?);
    let seat = Rc::new(oxyos_seat::Seat::take()?);
    eprintln!("[session] logind session on terminal {:?}", seat.vt());
    // Sleep waits for the lock to be on screen.
    if let Err(error) = seat.hold_sleep() {
        eprintln!("[session] holding sleep back: {error}; sleep will not wait for the lock");
    }
    let (display, card) = open_display(&seat)?;
    let mut display = display;
    let pointer_images = show_pointer(&mut display, pointer.as_ref());
    let (width, height) = display.size();
    eprintln!(
        "[session] showing on {} ({})",
        card.display(),
        display.mode_name()
    );
    let output = Rect::new(0, 0, width, height);
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: OUTPUT_ID,
            rect: output,
            scale_120: 120,
            refresh_mhz: display.refresh_hz() * 1000,
        })
        .map_err(|code| format!("output: {code:?}"))?;

    // The socket applications connect to, in the user's runtime directory.
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or("no XDG_RUNTIME_DIR: run oxyos-session in a login session")?;
    let socket = runtime.join(format!("oxyos-{}", std::process::id()));
    let listener = Listener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    let _socket = RemoveOnDrop(socket.clone());

    let own_uid = rustix::process::getuid().as_raw();
    let mut compositor = Compositor::new(scene, own_uid, output);
    compositor.pointer_images = pointer_images;
    // A session waits for its devices, clients and display rather than turning every 5 ms; at
    // most a quarter of a second, in which it notices its application or shell has exited.
    compositor.idle_wait = Duration::from_millis(250);
    compositor.records = false;
    compositor
        .transitions
        .set_reduce_motion(preferences.reduce_motion);
    compositor.reads_back = false;
    compositor.authentication = Some(Box::new(pam));
    // The Oxy mark on black while the session starts, as a Mac shows its own (ADR 0011).
    compositor.show_splash();
    // Its first frame on the display: the service manager that started the session lets the
    // boot logo go (ADR 0017). Failing to tell it never ends the session.
    if let Some(notify) = notify {
        compositor.on_first_flip = Some(Box::new(move || match notify.ready() {
            Ok(()) => eprintln!("[session] on screen: the service manager is told (READY=1)"),
            Err(error) => eprintln!("[session] telling the service manager it is ready: {error}"),
        }));
    }
    compositor.card_device = Some(oxyos_seat::Device::of(&card)?);
    compositor.seat = Some(seat.clone());
    // On the GPU straight into the display's buffers where the device exports them.
    match oxyos_gpu::VulkanCompositor::new() {
        Ok(mut gpu) if gpu.exports_dma_buf() => {
            use std::os::fd::AsFd;
            for target in gpu.scanout_targets(2, width, height)? {
                let framebuffer = display.card().import_dmabuf(
                    target.fd.as_fd(),
                    target.width,
                    target.height,
                    target.pitch,
                )?;
                compositor.scanout.push(framebuffer);
            }
            eprintln!(
                "[session] composing on {} ({:?}) into the display's buffers",
                gpu.device_name(),
                gpu.device_kind()
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
    compositor.display = Some(display);

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
    let input = oxyos_input::Input::open_with((width, height), &layout, Some((open, close)))?;
    eprintln!("[session] input from seat0, layout {layout}");
    compositor.input = Some(input);

    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(signal, stop.clone())?;
    }
    // The session's shell first, on its own socket, granted `shell`.
    let shell_socket = runtime.join(format!("oxyos-{}-shell", std::process::id()));
    let _shell_socket = RemoveOnDrop(shell_socket.clone());
    let mut shell = match &shell_command {
        Some(shell_command) => Some(start_shell(
            &mut compositor,
            &listener,
            &shell_socket,
            shell_command,
            output,
        )?),
        None => None,
    };
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
    // The applications the shell asks for (ADR 0013).
    let env_path = |name| std::env::var_os(name).map(PathBuf::from);
    let mut launcher = oxyos_apps::Launcher::new(
        oxyos_apps::Runtime {
            socket: socket.clone(),
            bloom_host: env_path("OXYOS_BLOOM_HOST"),
            bloom_bundle: env_path("OXYOS_BLOOM_BUNDLE"),
            window: (960, 600),
            env: Vec::new(),
        },
        oxyos_apps::directories(),
        oxyos_apps::languages(),
    );
    let mut shell_started = Instant::now();
    let ended = loop {
        for id in launcher.reap() {
            eprintln!("[session] {id} exited");
        }
        for request in compositor.scene.take_launches() {
            let launched = match launcher.launch(&request.id) {
                Ok(pid) => {
                    eprintln!("[session] started {} ({pid})", request.id);
                    true
                }
                Err(error) => {
                    eprintln!("[session] not started: {error}");
                    false
                }
            };
            compositor
                .scene
                .launched(request.client, request.serial, launched);
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
            match (reader.take(), &reader_program) {
                (Some(mut running), _) => {
                    eprintln!("[session] stopping the screen reader");
                    end(&mut running)?;
                }
                (None, Some(program)) => {
                    eprintln!("[session] starting the screen reader");
                    reader = Some(spawn_reader(&reader_socket, program)?);
                }
                (None, None) => eprintln!("[session] no screen reader (--reader)"),
            }
        }
        // The windows of what it started are their registry application's (ADR 0013).
        compositor.name_clients(&|pid| launcher.application_of(pid).map(String::from));
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
        if let Some(status) = shell
            .as_mut()
            .map(|shell| shell.try_wait())
            .transpose()?
            .flatten()
        {
            if !compositor.scene.is_locked() {
                eprintln!("[session] the shell exited ({status})");
                break Ok(());
            }
            // Failing closed: the session stays locked, under the covers, until the shell is
            // back and shows the lock screen.
            eprintln!("[session] the shell exited ({status}) while locked");
            shell = None;
        }
        if let (None, Some(shell_command)) = (&shell, &shell_command) {
            if shell_started.elapsed() >= SHELL_RESTART {
                eprintln!("[session] starting the shell again");
                shell_started = Instant::now();
                shell = Some(spawn_shell(&shell_socket, shell_command)?);
            }
        }
        if compositor.ending || stop.load(Ordering::Relaxed) {
            eprintln!("[session] ending");
            break Ok(());
        }
        if let Err(error) = compositor.serve(&listener) {
            break Err(error);
        }
    };
    if let Some(application) = application.as_mut() {
        end(application)?;
    }
    launcher.end(Duration::from_secs(2));
    if let Some(shell) = shell.as_mut() {
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
    compositor.display = None;
    compositor.seat = None;
    ended
}

/// Starts the session's shell on `socket`, granting `shell`, and serves the session until its
/// panels have set the work area (it differs from the output).
fn start_shell(
    compositor: &mut Compositor,
    listener: &Listener,
    socket: &std::path::Path,
    command: &[OsString],
    output: Rect,
) -> Result<std::process::Child> {
    let shell_listener = Listener::bind(socket)?;
    shell_listener.set_nonblocking(true)?;
    compositor.shell_listener = Some(shell_listener);
    let mut shell = spawn_shell(socket, command)?;
    let deadline = Instant::now() + SHELL_DEADLINE;
    loop {
        if let Some(status) = shell.try_wait()? {
            return Err(format!("the shell exited ({status}) before its panels were shown").into());
        }
        if Instant::now() > deadline {
            end(&mut shell)?;
            return Err("the shell's panels were not shown in time".into());
        }
        compositor.serve(listener)?;
        if compositor
            .scene
            .work_area(OUTPUT_ID)
            .is_some_and(|area| area != output)
        {
            eprintln!("[session] the shell's panels are shown");
            return Ok(shell);
        }
    }
}

/// The session's shell, on `socket`.
fn spawn_shell(socket: &std::path::Path, command: &[OsString]) -> Result<std::process::Child> {
    Ok(Command::new(command.first().ok_or(USAGE)?)
        .args(&command[1..])
        .arg("--shell")
        .arg(OUTPUT_ID.to_string())
        .env("OXYOS_COMPOSITOR_SOCKET", socket)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .spawn()?)
}

/// The session's screen reader, on `socket`, in the session's language.
fn spawn_reader(socket: &std::path::Path, program: &OsString) -> Result<std::process::Child> {
    Ok(Command::new(program)
        .env("OXYOS_COMPOSITOR_SOCKET", socket)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .spawn()?)
}

/// The session's authentication path (ADR 0010): PAM, off the compositor's thread, throttled.
struct Pam(oxyos_auth::Checker);

/// Whole seconds, rounded up: a wait of 0.2 s is still a wait.
fn seconds(wait: Duration) -> u32 {
    u32::try_from(wait.as_millis().div_ceil(1000)).unwrap_or(u32::MAX)
}

impl Authentication for Pam {
    fn attempt(
        &mut self,
        mut secret: oxyos_compositor_core::scene::Secret,
    ) -> Option<(AuthOutcome, u32)> {
        // The password's memory moves to PAM's side whole, and is overwritten there.
        let secret = oxyos_auth::Secret::new(std::mem::take(&mut *secret));
        match self.0.attempt(secret, Instant::now()) {
            oxyos_auth::Attempt::Checking => None,
            oxyos_auth::Attempt::Busy => Some((AuthOutcome::Busy, 0)),
            oxyos_auth::Attempt::Throttled(wait) => Some((AuthOutcome::Throttled, seconds(wait))),
        }
    }

    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }

    fn answer(&mut self) -> Option<(AuthOutcome, u32)> {
        let answer = self.0.answer(Instant::now())?;
        let outcome = match answer.outcome {
            oxyos_auth::Outcome::Accepted => AuthOutcome::Accepted,
            oxyos_auth::Outcome::Rejected => AuthOutcome::Rejected,
        };
        Some((outcome, seconds(answer.wait)))
    }
}

/// The pointer's images on the display's cursor plane, by shape: from a cursor file (the arrow)
/// or a cursor scheme's folder (every shape its `Install.inf` names), else the display's own
/// arrow. Without a cursor plane, or with files it cannot read, the session says so and goes on.
fn show_pointer(
    display: &mut oxyos_kms::Display,
    pointer: Option<&(PathBuf, u32)>,
) -> BTreeMap<PointerShape, Vec<(usize, Duration)>> {
    let mut images = BTreeMap::new();
    let Some(max) = display.pointer_size() else {
        eprintln!("[session] the display has no cursor plane: the pointer is not shown");
        return images;
    };
    let Some((path, size)) = pointer else {
        return images;
    };
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
fn open_display(seat: &oxyos_seat::Seat) -> Result<(oxyos_kms::Display, PathBuf)> {
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
            oxyos_kms::Display::open_from_seat(oxyos_kms::Card::from_fd(fd)?)
        });
        match opened {
            Ok(display) => return Ok((display, path)),
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

/// Removes the socket when the session ends.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
