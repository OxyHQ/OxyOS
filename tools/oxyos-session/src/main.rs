//! The OxyOS session: the compositor (`oxyos-compositor`) on a real seat, running the
//! applications it launches. It runs as the user of a logind session on a seat, from a login on
//! a terminal, never as root (docs/adr/0006-seat.mdx):
//! - its display is the first card with a connected output, at that output's size;
//! - frames are composed on the GPU straight into the display's scanout buffers where Vulkan
//!   exports DMA-BUF, otherwise on the CPU, only what changed (docs/adr/0005-damage.mdx);
//! - its input is the seat's devices through libinput, with the keymap of `--layout`;
//! - Ctrl+Alt+Fn switches terminals, and logind's switches are followed.
//!
//! `oxyos-session [--layout LAYOUT] [--pointer FILE.cur[,SIZE]] [--shell PROGRAM,ARG...] -- COMMAND
//! [ARGS]...` runs COMMAND
//! with `OXYOS_COMPOSITOR_SOCKET` naming the compositor. With `--shell`, the session's shell
//! (ADR 0009) starts first: PROGRAM with its ARGs, then `--shell OUTPUT`, on a socket of its own
//! that grants `shell`; COMMAND starts once the shell's panels have set the work area its window is
//! placed in. The session ends when either exits, on Ctrl+Alt+Backspace, or on SIGINT or SIGTERM
//! (they are then asked to stop).
//!
//! The pointer is shown on the display's cursor plane, in the shape the surface under it wants:
//! the display's own arrow, or the images of a Windows cursor file or cursor scheme's folder
//! (`--pointer`, every shape its `Install.inf` names) closest to SIZE pixels (32 by default).

mod cursor_file;

use oxyos_compositor::{Compositor, Gpu, Result};
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{PointerShape, Rect};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const USAGE: &str = "usage: oxyos-session [--layout LAYOUT] [--pointer FILE.cur|SCHEME[,SIZE]] \
                     [--shell PROGRAM,ARG...] -- COMMAND [ARGS]...";
/// The shell's panels set the work area within this budget, or the session does not start.
const SHELL_DEADLINE: Duration = Duration::from_secs(60);
const OUTPUT_ID: u32 = 1;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mut layout = "us".to_string();
    let mut command: Vec<OsString> = Vec::new();
    let mut shell_command: Option<Vec<OsString>> = None;
    let mut pointer: Option<(PathBuf, u32)> = None;
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
            Some("--layout") => {
                layout = args
                    .next()
                    .and_then(|layout| layout.into_string().ok())
                    .ok_or(USAGE)?
            }
            Some("--") => {
                command = args.collect();
                break;
            }
            _ => return Err(USAGE.into()),
        }
    }
    if command.is_empty() {
        return Err(USAGE.into());
    }

    let seat = Rc::new(oxyos_seat::Seat::take()?);
    eprintln!("[session] logind session on terminal {:?}", seat.vt());
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
    compositor.reads_back = false;
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
    let mut application = Command::new(&command[0])
        .args(&command[1..])
        .env("OXYOS_COMPOSITOR_SOCKET", &socket)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .spawn()?;
    let ended = loop {
        if let Some(status) = application.try_wait()? {
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
            eprintln!("[session] the shell exited ({status})");
            break Ok(());
        }
        if compositor.ending || stop.load(Ordering::Relaxed) {
            eprintln!("[session] ending");
            break Ok(());
        }
        if let Err(error) = compositor.serve(&listener) {
            break Err(error);
        }
    };
    end(&mut application)?;
    if let Some(shell) = shell.as_mut() {
        end(shell)?;
    }
    compositor.shell_listener = None;
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
    let mut shell = Command::new(command.first().ok_or(USAGE)?)
        .args(&command[1..])
        .arg("--shell")
        .arg(OUTPUT_ID.to_string())
        .env("OXYOS_COMPOSITOR_SOCKET", socket)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .spawn()?;
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
