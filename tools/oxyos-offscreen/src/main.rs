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
mod ime;
mod launch;
mod lock;
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

const OUTPUT: Rect = Rect::new(0, 0, 1280, 720);
const OUTPUT_ID: OutputId = 1;
const DEADLINE: Duration = Duration::from_secs(10);
const BUTTON: u32 = 272;

/// The output's composed frame as a PNG in `directory`.
fn snapshot(compositor: &Compositor, directory: &Path, name: &str) -> Result<PathBuf> {
    let path = directory.join(name);
    fs::write(
        &path,
        png::encode(OUTPUT.width, OUTPUT.height, &compositor.framebuffer),
    )?;
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
    /// With `--launch`: the assistive technology the session plays (reader.rs).
    reader: reader::Reader,
}

impl Session {
    fn start() -> Result<Self> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.subsec_nanos();
        let directory =
            std::env::temp_dir().join(format!("oxyos-offscreen-{}-{nonce}", std::process::id()));
        DirBuilder::new().mode(0o700).create(&directory)?;
        let cleanup = TempDir(directory.clone());
        let socket = directory.join("s");
        let listener = Listener::bind(&socket)?;
        listener.set_nonblocking(true)?;
        let baseline_fds = open_descriptors()?;

        let mut scene = Scene::new();
        scene
            .add_output(Output {
                id: OUTPUT_ID,
                rect: OUTPUT,
                scale_120: 120,
                refresh_mhz: 60_000,
            })
            .map_err(|code| format!("output: {code:?}"))?;
        let own_uid = fs::metadata(&directory).map(|m| std::os::unix::fs::MetadataExt::uid(&m))?;
        let compositor = Compositor::new(scene, own_uid, OUTPUT);
        Ok(Self {
            compositor,
            listener,
            socket,
            baseline_fds,
            _cleanup: cleanup,
            ime: None,
            reader: reader::Reader::default(),
        })
    }

    /// Closes every connection, waits for the clients to exit, and checks that no scene
    /// resource or descriptor remains.
    fn finish(mut self, children: &mut Children) -> Result<()> {
        let clients: Vec<ClientId> = self.compositor.peers.keys().copied().collect();
        for client in clients {
            self.compositor.disconnect(client);
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
                if let Some(window) = compositor.surface_rect(app, 1) {
                    snapshots.push(snapshot(compositor, out, "1-windows.png")?);
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
                    let window = compositor.surface_rect(app, 1).expect("parent shown");
                    let below = (window.x + 10, window.bottom() as i32 + 10);
                    if compositor.pixel(below.0, below.1) != OTHER_WINDOW {
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
