//! The OxyOS session: the compositor (`oxy-compositor`) on a real seat, running the
//! applications it launches. It runs as the user of a logind session on a seat, from a login on
//! a terminal, never as root (docs/adr/0006-seat.mdx):
//! - its display is the first card with a connected output, at that output's size;
//! - frames are composed on the GPU straight into the display's scanout buffers where Vulkan
//!   exports DMA-BUF, otherwise on the CPU, only what changed (docs/adr/0005-damage.mdx);
//! - its input is the seat's devices through libinput, with the keymap of `--layout`;
//! - Ctrl+Alt+Fn switches terminals, and logind's switches are followed.
//!
//! `oxy-session [--layout LAYOUT] -- COMMAND [ARGS]...` runs COMMAND with
//! `OXY_COMPOSITOR_SOCKET` naming the compositor. The session ends when it exits, on
//! Ctrl+Alt+Backspace, or on SIGINT or SIGTERM (the application is then asked to stop).

use oxy_compositor::{Compositor, Gpu, Result};
use oxy_compositor_core::{Output, Scene};
use oxy_ipc::Listener;
use oxy_protocol::Rect;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const USAGE: &str = "usage: oxy-session [--layout LAYOUT] -- COMMAND [ARGS]...";
const OUTPUT_ID: u32 = 1;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mut layout = "us".to_string();
    let mut command: Vec<OsString> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.to_str() {
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

    let seat = Rc::new(oxy_seat::Seat::take()?);
    eprintln!("[session] logind session on terminal {:?}", seat.vt());
    let (display, card) = open_display(&seat)?;
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
        .ok_or("no XDG_RUNTIME_DIR: run oxy-session in a login session")?;
    let socket = runtime.join(format!("oxy-{}", std::process::id()));
    let listener = Listener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    let _socket = RemoveOnDrop(socket.clone());

    let own_uid = rustix::process::getuid().as_raw();
    let mut compositor = Compositor::new(scene, own_uid, output);
    compositor.records = false;
    compositor.reads_back = false;
    compositor.card_device = Some(oxy_seat::Device::of(&card)?);
    compositor.seat = Some(seat.clone());
    // On the GPU straight into the display's buffers where the device exports them.
    match oxy_gpu::VulkanCompositor::new() {
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
    let open: oxy_input::Open = Box::new(move |path| open_seat.open(path).map(|(fd, _)| fd));
    let close: oxy_input::Close = Box::new(move |fd| {
        use std::os::fd::AsFd;
        if let Ok(device) = oxy_seat::Device::of_fd(fd.as_fd()) {
            let _ = close_seat.close(device);
        }
    });
    let input = oxy_input::Input::open_with((width, height), &layout, Some((open, close)))?;
    eprintln!("[session] input from seat0, layout {layout}");
    compositor.input = Some(input);

    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(signal, stop.clone())?;
    }
    let mut application = Command::new(&command[0])
        .args(&command[1..])
        .env("OXY_COMPOSITOR_SOCKET", &socket)
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
        if compositor.ending || stop.load(Ordering::Relaxed) {
            eprintln!("[session] ending");
            break Ok(());
        }
        if let Err(error) = compositor.serve(&listener) {
            break Err(error);
        }
    };
    end(&mut application)?;
    // The last frame completes before the display and the devices are given back.
    compositor.complete_flip(true);
    compositor.input = None;
    compositor.display = None;
    compositor.seat = None;
    ended
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
fn open_display(seat: &oxy_seat::Seat) -> Result<(oxy_kms::Display, PathBuf)> {
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
            oxy_kms::Display::open_from_seat(oxy_kms::Card::from_fd(fd)?)
        });
        match opened {
            Ok(display) => return Ok((display, path)),
            Err(error) => {
                if let Ok(device) = oxy_seat::Device::of(&path) {
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
