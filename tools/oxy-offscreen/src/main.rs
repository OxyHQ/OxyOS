//! Offscreen development compositor: real client processes, real shared-memory buffers, the real
//! protocol transport and scene policy, composed on the CPU into PNG snapshots.
//!
//! This is the development path the architecture allows for headless machines. It is not
//! DRM/KMS, not Vulkan and not a session; input is injected by a scripted director, and feedback
//! timestamps are labeled synthetic because nothing is displayed.
//!
//! Without arguments it runs two first-party test clients through the popup scenario.
//! `--launch -- COMMAND` instead launches one application process (such as the native Bloom
//! host), passing the socket in `OXY_COMPOSITOR_SOCKET`, and presses the points `--press` names.

mod client;
mod launch;
mod png;
mod uinput;

use client::*;
use oxy_compositor_core::{ClientId, Grants, Output, OutputId, RenderItem, Scene, SurfaceKey};
use oxy_ipc::{Connection, Credentials, Listener, SharedBuffer};
use oxy_protocol::{Clock, ErrorCode, Event, PointerKind, Rect, Request, ServerConnection, Size};
use rustix::event::{poll, PollFd, PollFlags};
use rustix::fd::AsFd;
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

const OUTPUT: Rect = Rect::new(0, 0, 1280, 720);
const OUTPUT_ID: OutputId = 1;
const DEADLINE: Duration = Duration::from_secs(10);
/// Events a client may leave unread before it is disconnected as unresponsive.
const MAX_OUTBOX: usize = 1024;
/// Connections, including ones that have not completed Hello yet.
const MAX_PEERS: usize = oxy_compositor_core::scene::MAX_CLIENTS;
/// Time a connection may take to send Hello.
const HELLO_DEADLINE: Duration = Duration::from_secs(2);
const BUTTON: u32 = 272;

struct Peer {
    connection: Connection,
    protocol: ServerConnection,
    outbox: VecDeque<Event>,
    buffers: HashMap<u64, SharedBuffer>,
    presses: usize,
    accepted: Instant,
}

struct Compositor {
    scene: Scene,
    peers: HashMap<ClientId, Peer>,
    next_client: ClientId,
    own_uid: u32,
    framebuffer: Vec<u32>,
    composed: Vec<RenderItem>,
    frames: usize,
    /// Input events routed to clients, kept while `log_input` is set: (client, surface, kind).
    input_log: Vec<(ClientId, u32, &'static str)>,
    log_input: bool,
    /// With `--gpu`: every frame is also composed on this Vulkan device and must match the CPU
    /// reference byte for byte (docs/adr/0004-gpu-composition.mdx).
    gpu: Option<Gpu>,
    /// With `--dump-frames`: the latest composed frame, as `oxy_gpu::frame_file` writes it.
    keep_frame: bool,
    last_frame: Vec<u8>,
    /// With `--kms`: every composed frame is shown on this display, and completes at its flip.
    display: Option<oxy_kms::Display>,
    /// With `--input`: the seat's devices through libinput, and where the output sits on the
    /// display (its top-left corner, in the display's pixels).
    input: Option<oxy_input::Input>,
    display_offset: (i32, i32),
    /// With `--kms` and `--gpu`: the display's framebuffers, imported from the Vulkan
    /// compositor's DMA-BUF scanout targets, and the one composed next.
    scanout: Vec<oxy_kms::Framebuffer>,
    scanout_back: usize,
    /// For every frame shown: when it was handed to the display and when it flipped
    /// (CLOCK_MONOTONIC).
    flips: Vec<(std::time::Duration, std::time::Duration)>,
    /// With `--kms`: the terminal the display is on (graphics mode, switches by request and,
    /// with `--input`, its keyboard off). Dropped after the display and the devices.
    terminal: Option<oxy_vt::Terminal>,
    /// Another terminal has the display and the devices: frames complete without being shown.
    away: bool,
}

struct Gpu {
    compositor: oxy_gpu::VulkanCompositor,
    frames: u64,
    mismatches: u64,
}

impl Compositor {
    /// Development session authority: same-user peers get ordinary application grants and
    /// nothing else. A shipping session derives grants from its launch records (#35).
    fn grants_for(&self, credentials: Credentials) -> Option<Grants> {
        (credentials.uid == self.own_uid).then(Grants::default)
    }

    fn accept(&mut self, listener: &Listener) -> Result<()> {
        let connection = match listener.accept() {
            Ok(connection) => connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let credentials = connection.peer_credentials()?;
        // Unauthorized peers and peers beyond the limit are closed at once; pending peers
        // count against the limit, so silent connections cannot exhaust descriptors.
        if self.grants_for(credentials).is_none() || self.peers.len() >= MAX_PEERS {
            return Ok(());
        }
        self.next_client += 1;
        self.peers.insert(
            self.next_client,
            Peer {
                connection,
                protocol: ServerConnection::new(),
                outbox: VecDeque::new(),
                buffers: HashMap::new(),
                presses: 0,
                accepted: Instant::now(),
            },
        );
        Ok(())
    }

    /// Closes connections that have not sent Hello in time.
    fn expire_pending(&mut self) {
        let expired: Vec<ClientId> = self
            .peers
            .iter()
            .filter(|(_, peer)| {
                peer.protocol.version().is_none() && peer.accepted.elapsed() > HELLO_DEADLINE
            })
            .map(|(client, _)| *client)
            .collect();
        for client in expired {
            self.fail(client, ErrorCode::InvalidOrder);
        }
    }

    fn fail(&mut self, client: ClientId, code: ErrorCode) {
        if let Some(peer) = self.peers.get(&client) {
            let _ = peer
                .connection
                .try_send_event(&Event::ProtocolError { code });
        }
        self.disconnect(client);
    }

    fn disconnect(&mut self, client: ClientId) {
        self.scene.disconnect(client);
        self.peers.remove(&client);
        self.retire();
    }

    /// Drops buffers no consumer will read again, closing their descriptors.
    fn retire(&mut self) {
        for (client, buffer) in self.scene.drain_retired() {
            if let Some(peer) = self.peers.get_mut(&client) {
                peer.buffers.remove(&buffer);
            }
        }
    }

    fn receive(&mut self, client: ClientId) {
        let Some(peer) = self.peers.get_mut(&client) else {
            return;
        };
        let (request, fd) = match peer.connection.recv_request() {
            Ok(received) => received,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                return self.disconnect(client)
            }
            Err(_) => return self.fail(client, ErrorCode::InvalidMessage),
        };
        match peer.protocol.receive(&request) {
            Ok(Some(welcome)) => {
                let credentials = peer.connection.peer_credentials();
                peer.outbox.push_back(welcome);
                let grants = credentials.ok().and_then(|c| self.grants_for(c));
                match grants.map(|grants| self.scene.connect(client, grants)) {
                    Some(Ok(())) => {}
                    _ => self.fail(client, ErrorCode::Unauthorized),
                }
                return;
            }
            Ok(None) => {}
            Err(code) => return self.fail(client, code),
        }
        if let (Some(layout), Some(fd), Request::AttachBuffer { buffer, .. }) =
            (request.buffer_layout(), fd, request)
        {
            let Ok(shared) = SharedBuffer::import(fd, layout) else {
                return self.fail(client, ErrorCode::InvalidBuffer);
            };
            if let Err(code) = self.scene.handle(client, request) {
                return self.fail(client, code);
            }
            self.peers
                .get_mut(&client)
                .expect("present")
                .buffers
                .insert(buffer, shared);
            return;
        }
        let goodbye = matches!(request, Request::Goodbye {});
        if let Err(code) = self.scene.handle(client, request) {
            return self.fail(client, code);
        }
        if goodbye {
            self.peers.remove(&client);
        }
        self.retire();
    }

    /// Routes scene events into bounded outboxes and sends what each socket accepts now.
    fn flush(&mut self) {
        for (client, event) in self.scene.drain_events() {
            if self.log_input {
                if let Some((surface, kind)) = input_kind(&event) {
                    self.input_log.push((client, surface, kind));
                }
            }
            if let Some(peer) = self.peers.get_mut(&client) {
                if matches!(
                    event,
                    Event::Pointer {
                        kind: PointerKind::Press,
                        ..
                    }
                ) {
                    peer.presses += 1;
                }
                peer.outbox.push_back(event);
            }
        }
        let mut unresponsive = Vec::new();
        for (client, peer) in &mut self.peers {
            while let Some(event) = peer.outbox.front() {
                match peer.connection.try_send_event(event) {
                    Ok(true) => {
                        peer.outbox.pop_front();
                    }
                    Ok(false) => break,
                    Err(_) => {
                        unresponsive.push(*client);
                        break;
                    }
                }
            }
            if peer.outbox.len() > MAX_OUTBOX {
                unresponsive.push(*client);
            }
        }
        for client in unresponsive {
            self.disconnect(client);
        }
    }

    /// Composes the in-flight frame on the CPU, then completes it. Without new client frames
    /// the output is recomposed only if the scene changed (a surface went away, moved, ...).
    fn frame(&mut self) {
        let started = self.scene.begin_frame();
        let list = self.scene.frame_list();
        if started == 0 && list == self.composed {
            self.scene.end_frame(0, Clock::Synthetic);
            return;
        }
        // Each surface's pixels, read from its shared buffer, and the covers.
        let mut unreadable = Vec::new();
        let mut contents = Vec::new();
        for item in list.iter().copied() {
            match item {
                RenderItem::Surface { key, rect, frame } => match self.pixels(key, frame.buffer) {
                    Some((extent, format, pixels)) => {
                        contents.push((rect, extent, Some(format), pixels))
                    }
                    None => unreadable.push(key.client),
                },
                RenderItem::Cover { rect, .. } => {
                    contents.push((rect, Size::new(1, 1), None, Vec::new()))
                }
            }
        }
        let layers: Vec<oxy_gpu::Layer<'_>> = contents
            .iter()
            .map(|(rect, extent, format, pixels)| oxy_gpu::Layer {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                extent: (extent.width, extent.height),
                paint: match format {
                    Some(oxy_protocol::Format::Xrgb8888) => oxy_gpu::Paint::Opaque(pixels),
                    Some(oxy_protocol::Format::Argb8888) => oxy_gpu::Paint::Premultiplied(pixels),
                    None => oxy_gpu::Paint::Fill(0),
                },
            })
            .collect();
        // The output is at the origin of the global space.
        self.framebuffer = oxy_gpu::compose_cpu(OUTPUT.width, OUTPUT.height, DESKTOP, &layers);
        if self.keep_frame {
            self.last_frame.clear();
            oxy_gpu::frame_file::write(
                &mut self.last_frame,
                OUTPUT.width,
                OUTPUT.height,
                DESKTOP,
                &layers,
            )
            .expect("writing to memory");
        }
        if let Some(gpu) = &mut self.gpu {
            gpu.frames += 1;
            match gpu
                .compositor
                .compose(OUTPUT.width, OUTPUT.height, DESKTOP, &layers)
            {
                Ok(pixels) if pixels == self.framebuffer => {}
                Ok(_) => gpu.mismatches += 1,
                Err(error) => {
                    eprintln!("[gpu] {error}");
                    gpu.mismatches += 1;
                }
            }
        }
        // A buffer that could not be read is never reported as presented: its client is
        // disconnected before the frame completes.
        for client in unreadable {
            self.fail(client, ErrorCode::InvalidBuffer);
        }
        self.composed = self.scene.frame_list();
        self.frames += 1;
        // On a display, the frame completes when it is on screen, at the kernel's flip time.
        let (timestamp_ns, clock) = match self.show(&layers) {
            Some(flip) => (
                u64::try_from(flip.as_nanos()).unwrap_or(u64::MAX),
                Clock::Monotonic,
            ),
            None => (0, Clock::Synthetic),
        };
        self.scene.end_frame(timestamp_ns, clock);
        self.retire();
    }

    /// With `--kms`: lets go of the display and the devices when the kernel switches to another
    /// terminal, and takes them back when it returns; the frame is then shown again.
    fn follow_terminal(&mut self) -> Result<()> {
        let Some(terminal) = self.terminal.as_mut() else {
            return Ok(());
        };
        for switch in terminal.switches()? {
            match switch {
                oxy_vt::Switch::Release if !self.away => {
                    if let Some(display) = &mut self.display {
                        display.suspend()?;
                    }
                    if let Some(input) = &mut self.input {
                        input.suspend();
                    }
                    // libinput released what was still down: applications hear it now.
                    self.read_input();
                    self.away = true;
                    if let Some(terminal) = &self.terminal {
                        terminal.release()?;
                    }
                }
                oxy_vt::Switch::Acquire if self.away => {
                    if let Some(terminal) = &self.terminal {
                        terminal.acquired()?;
                    }
                    if let Some(display) = &mut self.display {
                        display.resume()?;
                    }
                    if let Some(input) = &mut self.input {
                        input.resume()?;
                    }
                    self.away = false;
                    // Composed again, and shown with a modeset.
                    self.composed.clear();
                }
                // A release while away, or an acquire while here: nothing to give or take.
                oxy_vt::Switch::Release | oxy_vt::Switch::Acquire => {}
            }
        }
        Ok(())
    }

    /// With `--input`: what the seat's devices did since the last turn, applied to the scene.
    /// Positions arrive in the display's pixels; the output is centred on the display.
    fn read_input(&mut self) {
        let Some(input) = self.input.as_mut() else {
            return;
        };
        let actions = match input.dispatch() {
            Ok(actions) => actions,
            Err(error) => {
                eprintln!("[input] {error}; input stops");
                self.input = None;
                return;
            }
        };
        let (dx, dy) = self.display_offset;
        let trace = std::env::var_os("OXY_INPUT_TRACE").is_some();
        for action in actions {
            if trace {
                eprintln!("[input] {action:?} focus {:?}", self.scene.focus());
            }
            match action {
                oxy_input::Action::Motion { x, y } => self
                    .scene
                    .pointer_motion(x.round() as i32 - dx, y.round() as i32 - dy),
                oxy_input::Action::Button { code, pressed } => {
                    self.scene.pointer_button(code, pressed)
                }
                // libinput's units are logical pixels; the protocol's are 1/256 of one.
                oxy_input::Action::Scroll { dx, dy } => self
                    .scene
                    .pointer_scroll((dx * 256.0).round() as i32, (dy * 256.0).round() as i32),
                oxy_input::Action::Key {
                    code,
                    pressed,
                    text,
                } => {
                    self.scene.key(code, pressed);
                    if let Some(text) = text {
                        self.scene.text(&text);
                    }
                }
                oxy_input::Action::SwitchTerminal(number) => {
                    if let Some(terminal) = &self.terminal {
                        if number != terminal.number() {
                            if let Err(error) = terminal.activate(number) {
                                eprintln!("[vt] switching to {number}: {error}");
                            }
                        }
                    }
                }
                // What a sequence composes replaces its composition when committed.
                oxy_input::Action::Compose { composition, text } => match text {
                    Some(text) => self.scene.text(&text),
                    None => {
                        self.scene.preedit(&composition, composition.len());
                    }
                },
            }
        }
    }

    /// With `--kms`: shows the composed output centred on the display and waits for its flip;
    /// returns the flip's CLOCK_MONOTONIC time. With `--gpu` too, the frame is composed on the
    /// Vulkan device straight into a scanout buffer, with no copy through the CPU. A display
    /// error ends display output.
    fn show(&mut self, layers: &[oxy_gpu::Layer<'_>]) -> Option<std::time::Duration> {
        if self.away {
            return None;
        }
        let display = self.display.as_mut()?;
        let (width, height) = display.size();
        if let (Some(gpu), false) = (&mut self.gpu, self.scanout.is_empty()) {
            let dx = (width.saturating_sub(OUTPUT.width) / 2) as i32;
            let dy = (height.saturating_sub(OUTPUT.height) / 2) as i32;
            let shifted: Vec<oxy_gpu::Layer<'_>> = layers
                .iter()
                .map(|layer| oxy_gpu::Layer {
                    x: layer.x + dx,
                    y: layer.y + dy,
                    ..*layer
                })
                .collect();
            let back = self.scanout_back;
            let composed = gpu
                .compositor
                .compose_to_scanout(back, DESKTOP, &shifted)
                .map_err(|error| std::io::Error::other(error.to_string()));
            let submitted = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
            let submitted =
                std::time::Duration::new(submitted.tv_sec as u64, submitted.tv_nsec as u32);
            let flip = composed
                .and_then(|()| display.present_framebuffer(self.scanout[back]))
                .and_then(|()| display.wait_flip());
            self.scanout_back = 1 - back;
            return match flip {
                Ok(flip) => {
                    self.flips.push((submitted, flip));
                    Some(flip)
                }
                Err(error) => {
                    eprintln!("[kms] {error}; display output stops");
                    self.display = None;
                    None
                }
            };
        }
        let mut pixels = vec![DESKTOP; width as usize * height as usize];
        let dx = (width.saturating_sub(OUTPUT.width) / 2) as usize;
        let dy = (height.saturating_sub(OUTPUT.height) / 2) as usize;
        let columns = (OUTPUT.width as usize).min(width as usize);
        for (y, row) in self
            .framebuffer
            .chunks_exact(OUTPUT.width as usize)
            .enumerate()
        {
            if y + dy >= height as usize {
                break;
            }
            let start = (y + dy) * width as usize + dx;
            pixels[start..start + columns].copy_from_slice(&row[..columns]);
        }
        let drawn = display.draw(&pixels);
        let submitted = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        let submitted = std::time::Duration::new(submitted.tv_sec as u64, submitted.tv_nsec as u32);
        let flip = drawn
            .and_then(|()| display.present())
            .and_then(|()| display.wait_flip());
        match flip {
            Ok(flip) => {
                self.flips.push((submitted, flip));
                Some(flip)
            }
            Err(error) => {
                eprintln!("[kms] {error}; display output stops");
                self.display = None;
                None
            }
        }
    }

    /// A client buffer's pixels (tightly packed rows), size and format; `None` if the
    /// client's buffer could not be read.
    fn pixels(
        &self,
        key: SurfaceKey,
        buffer: u64,
    ) -> Option<(Size, oxy_protocol::Format, Vec<u8>)> {
        let shared = self.peers.get(&key.client)?.buffers.get(&buffer)?;
        let layout = shared.layout();
        let row = layout.extent.width as usize * 4;
        let mut pixels = vec![0; row * layout.extent.height as usize];
        for (y, out) in pixels.chunks_exact_mut(row).enumerate() {
            shared.read_row(y as u32, out).ok()?;
        }
        Some((layout.extent, layout.format, pixels))
    }

    /// One turn of the compositor: accepts connections, handles every readable client, composes
    /// a frame and sends events. Waits at most 5 ms for input.
    fn serve(&mut self, listener: &Listener) -> Result<()> {
        self.follow_terminal()?;
        self.read_input();
        let clients: Vec<ClientId> = self.peers.keys().copied().collect();
        // Poll the listener and every client; never block on any one of them.
        let readable: Vec<bool> = {
            let mut fds: Vec<PollFd<'_>> = Vec::with_capacity(clients.len() + 1);
            fds.push(PollFd::from_borrowed_fd(listener.as_fd(), PollFlags::IN));
            for client in &clients {
                fds.push(PollFd::from_borrowed_fd(
                    self.peers[client].connection.as_fd(),
                    PollFlags::IN,
                ));
            }
            poll(
                &mut fds,
                Some(&rustix::event::Timespec {
                    tv_sec: 0,
                    tv_nsec: 5_000_000,
                }),
            )?;
            fds.iter().map(|fd| !fd.revents().is_empty()).collect()
        };
        if readable[0] {
            self.accept(listener)?;
        }
        for (index, client) in clients.iter().enumerate() {
            if readable[index + 1] {
                self.receive(*client);
            }
        }
        self.expire_pending();
        self.frame();
        self.flush();
        Ok(())
    }

    fn pixel(&self, x: i32, y: i32) -> u32 {
        self.framebuffer[(y as u32 * OUTPUT.width + x as u32) as usize]
    }

    fn snapshot(&self, directory: &Path, name: &str) -> Result<PathBuf> {
        let path = directory.join(name);
        fs::write(
            &path,
            png::encode(OUTPUT.width, OUTPUT.height, &self.framebuffer),
        )?;
        Ok(path)
    }

    fn surface_rect(&self, client: ClientId, id: u32) -> Option<Rect> {
        self.scene
            .render_list()
            .into_iter()
            .find_map(|item| match item {
                RenderItem::Surface { key, rect, .. } if key == (SurfaceKey { client, id }) => {
                    Some(rect)
                }
                _ => None,
            })
    }

    fn surface_sequence(&self, client: ClientId, id: u32) -> Option<u64> {
        self.scene
            .render_list()
            .into_iter()
            .find_map(|item| match item {
                RenderItem::Surface { key, frame, .. } if key == (SurfaceKey { client, id }) => {
                    Some(frame.sequence)
                }
                _ => None,
            })
    }
}

/// The input events a step can cause, by surface and kind (motion is continuous and left out).
fn input_kind(event: &Event) -> Option<(u32, &'static str)> {
    Some(match *event {
        Event::Pointer { surface, kind, .. } => (
            surface,
            match kind {
                PointerKind::Press => "press",
                PointerKind::Release => "release",
                PointerKind::Enter => "enter",
                PointerKind::Leave => "leave",
                PointerKind::Cancel => "cancel",
                PointerKind::Motion => return None,
            },
        ),
        Event::Key {
            surface, pressed, ..
        } => (surface, if pressed { "key.press" } else { "key.release" }),
        Event::Text { surface, .. } => (surface, "text"),
        Event::Preedit { surface, .. } => (surface, "preedit"),
        Event::Scroll { surface, .. } => (surface, "scroll"),
        Event::PopupDone { surface, .. } => (surface, "popup.done"),
        Event::Focus { surface, focused } => {
            (surface, if focused { "focus.in" } else { "focus.out" })
        }
        _ => return None,
    })
}

/// Removes the private socket directory on every exit, including errors and deadlines.
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
}

impl Session {
    fn start() -> Result<Self> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.subsec_nanos();
        let directory =
            std::env::temp_dir().join(format!("oxy-offscreen-{}-{nonce}", std::process::id()));
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
        let compositor = Compositor {
            scene,
            peers: HashMap::new(),
            next_client: 0,
            own_uid: fs::metadata(&directory).map(|m| std::os::unix::fs::MetadataExt::uid(&m))?,
            framebuffer: vec![DESKTOP; (OUTPUT.width * OUTPUT.height) as usize],
            composed: Vec::new(),
            frames: 0,
            input_log: Vec::new(),
            log_input: false,
            gpu: None,
            keep_frame: false,
            last_frame: Vec::new(),
            display: None,
            input: None,
            display_offset: (0, 0),
            scanout: Vec::new(),
            scanout_back: 0,
            flips: Vec::new(),
            terminal: None,
            away: false,
        };
        Ok(Self {
            compositor,
            listener,
            socket,
            baseline_fds,
            _cleanup: cleanup,
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
                    snapshots.push(compositor.snapshot(out, "1-windows.png")?);
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
                    snapshots.push(compositor.snapshot(out, "2-menu-open.png")?);
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
                    snapshots.push(compositor.snapshot(out, "3-menu-row-pressed.png")?);
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
                if compositor.surface_rect(app, 2).is_none() && compositor.scene.stats().popups == 0
                {
                    checks.outside_presses_delivered =
                        compositor.peers[&other].presses - checks.other_presses_before;
                    let window = compositor.surface_rect(app, 1).expect("parent shown");
                    let below = (window.x + 10, window.bottom() as i32 + 10);
                    if compositor.pixel(below.0, below.1) != OTHER_WINDOW {
                        return Err("menu pixels remain after dismissal".into());
                    }
                    snapshots.push(compositor.snapshot(out, "4-menu-dismissed.png")?);
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
        [] => run(Path::new("target/oxy-offscreen")),
        [flag, path] if flag == "--out" => run(Path::new(path)),
        [flag, path] if flag == "--app" => Ok(app(Path::new(path))?),
        [flag, path] if flag == "--other" => Ok(other(Path::new(path))?),
        _ if args.iter().any(|arg| arg == "--launch") => {
            let (out, launch) = launch::Launch::parse(&args)?;
            launch::run(&out, &launch)
        }
        _ => Err(
            "usage: oxy-offscreen [--out DIR]\n       oxy-offscreen --launch [--out DIR] [--press X,Y]... -- COMMAND [ARGS]..."
                .into(),
        ),
    }
}
