//! The Oxy display compositor (docs/native-foundation.mdx): it serves client connections over
//! the protocol, routes input through the scene, composes the frame (on the CPU, or on a Vulkan
//! device straight into the display's scanout buffers) with only the output's damage, and shows
//! it on a DRM/KMS display, its devices from libinput and its seat from logind or the terminal.
//!
//! Two programs drive it: the development compositor (`oxyos-offscreen`), which scripts input and
//! checks every frame, and the session (`oxyos-session`).

use oxyos_compositor_core::{ClientId, Grants, RenderItem, Scene, SurfaceKey};
use oxyos_ipc::{Connection, Credentials, Listener, SharedBuffer};
use oxyos_protocol::{
    AuthOutcome, Clock, ErrorCode, Event, GesturePhase, PointerKind, Rect, Request,
    ServerConnection, Size,
};
use rustix::event::{poll, PollFd, PollFlags};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::error::Error;
use std::time::{Duration, Instant};

pub mod splash;
pub mod transitions;

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

/// The desktop's colour, where no surface is.
pub const DESKTOP: u32 = 0x10_14_18;
/// Events a client may leave unread before it is disconnected as unresponsive.
const MAX_OUTBOX: usize = 1024;
/// Connections, including ones that have not completed Hello yet.
const MAX_PEERS: usize = oxyos_compositor_core::scene::MAX_CLIENTS;
/// Time a connection may take to send Hello.
const HELLO_DEADLINE: Duration = Duration::from_secs(2);

pub struct Peer {
    connection: Connection,
    protocol: ServerConnection,
    /// Events not sent yet, each with the descriptor it carries (a ClipboardSend's pipe).
    outbox: VecDeque<(Event, Option<OwnedFd>)>,
    /// Pipes of pastes this client must answer, in the order of their ClipboardSend events.
    pipes: VecDeque<OwnedFd>,
    buffers: HashMap<u64, SharedBuffer>,
    /// Presses routed to this client (the development compositor counts them).
    pub presses: usize,
    accepted: Instant,
    /// What it may do, decided when it connected.
    grants: Grants,
}

pub struct Compositor {
    /// The output's rectangle in the global space, in its pixels: what is composed.
    pub output: Rect,
    pub scene: Scene,
    pub peers: HashMap<ClientId, Peer>,
    /// The development compositor's shell socket: a same-user peer connecting there is granted
    /// `shell` (docs/adr/0009-session-shell.mdx) and `lock` (it is the lock screen too, ADR 0010), as a shipping session will grant it from its
    /// launch records (#35).
    pub shell_listener: Option<Listener>,
    pub next_client: ClientId,
    /// Peers of the shell socket are numbered down from here, so the clients the development
    /// compositor launches keep their numbers whatever order they connect in (the two above are
    /// the scene-only clients it plays: an assistive technology and an input method).
    next_shell_client: ClientId,
    pub own_uid: u32,
    pub framebuffer: Vec<u32>,
    pub composed: Vec<RenderItem>,
    pub frames: usize,
    /// Input events routed to clients, kept while `log_input` is set: (client, surface, kind).
    pub input_log: Vec<(ClientId, u32, &'static str)>,
    pub log_input: bool,
    /// Clients of the scene with no connection, played by the program running the compositor
    /// (a development compositor's assistive technology or input method): their events are kept
    /// in `local_events` for it, not dropped.
    pub local_clients: std::collections::BTreeSet<ClientId>,
    pub local_events: Vec<(ClientId, Event)>,
    /// With `--gpu`: every frame is also composed on this Vulkan device and must match the CPU
    /// reference byte for byte (docs/adr/0004-gpu-composition.mdx).
    pub gpu: Option<Gpu>,
    /// With `--gpu-only`: frames are composed on the Vulkan device alone, with no CPU reference;
    /// the frame the director checks is the one composed, read back after it is shown.
    pub gpu_only: bool,
    /// The frame just shown was kept on the device for `framebuffer_from_gpu`.
    pub shown_kept: bool,
    /// With `gpu_only`: whether every frame shown is read back into `framebuffer`, as the
    /// development compositor's checks need. A session never reads its frames back.
    pub reads_back: bool,
    /// Whether per-frame measurements are kept (flips, composition times, damage, the
    /// device's times), as the development compositor reports them. A session keeps none:
    /// they would grow with every frame.
    pub records: bool,
    /// The user ended the session (Ctrl+Alt+Backspace): the session's program stops.
    pub ending: bool,
    /// How long a turn may wait for something to happen when nothing has a deadline sooner: the
    /// development compositor's 5 ms by default, so its director's steps keep their pace; a
    /// session longer, waking for its devices, clients and display, the pointer's next step and a
    /// connection's deadline (`serve`).
    pub idle_wait: Duration,
    /// With `--dump-frames`: the latest composed frame, as `oxyos_gpu::frame_file` writes it.
    pub keep_frame: bool,
    pub last_frame: Vec<u8>,
    /// With `--kms`: every composed frame is shown on this display, and completes at its flip.
    pub display: Option<oxyos_kms::Display>,
    /// With `--input`: the seat's devices through libinput, and where the output sits on the
    /// display (its top-left corner, in the display's pixels).
    pub input: Option<oxyos_input::Input>,
    pub display_offset: (i32, i32),
    /// With `--kms` and `--gpu`: the display's framebuffers, imported from the Vulkan
    /// compositor's DMA-BUF scanout targets, and the one composed next.
    pub scanout: Vec<oxyos_kms::Framebuffer>,
    pub scanout_back: usize,
    /// For every frame shown: when it was handed to the display and when it flipped
    /// (CLOCK_MONOTONIC).
    pub flips: Vec<(std::time::Duration, std::time::Duration)>,
    /// The frame on its way to the display: when it was handed over. It completes at its flip,
    /// in a later turn; no frame is composed meanwhile (one on the display at a time).
    pub pending_flip: Option<std::time::Duration>,
    /// The pending flip only moves the pointer: the frame shown, committed again.
    pointer_flip: bool,
    /// Where the pointer is on the display (its pixels), once it has moved.
    pointer: Option<(i32, i32)>,
    /// The display's pointer images for each shape the session has any for (the arrow for any
    /// other), with how long each shows: one image, or an animated cursor's steps.
    pub pointer_images: BTreeMap<oxyos_protocol::PointerShape, Vec<(usize, std::time::Duration)>>,
    /// What the last flip showed (the scene's displayed content once it completed).
    pub flipped: Vec<RenderItem>,
    /// When the frame being composed started (CLOCK_MONOTONIC), and for every frame shown, how
    /// long from then to handing it to the display, in milliseconds.
    pub composing_since: std::time::Duration,
    pub compositions: Vec<f64>,
    /// For every frame composed: how long reading the clients' buffers took, in ms.
    pub reads: Vec<f64>,
    /// For every frame composed: the fraction of the output its damage covers (1 when whole).
    pub damaged: Vec<f64>,
    /// A number for every frame a client submitted, never reused, by surface and sequence:
    /// what identifies a surface's pixels to the GPU (`oxyos_gpu::Source`), since surface ids
    /// and sequences are the client's and come again when it creates a surface anew.
    pub versions: HashMap<(SurfaceKey, u64), u64>,
    pub next_version: u64,
    /// The version of each surface's pixels composed last, by `oxyos_gpu::Source` key: what a
    /// frame's damage is relative to.
    pub shown_versions: HashMap<u64, u64>,
    /// With `--kms`: the terminal the display is on (graphics mode, switches by request and,
    /// with `--input`, its keyboard off). Dropped after the display and the devices.
    pub terminal: Option<oxyos_vt::Terminal>,
    /// With `--seat`: the logind session the compositor runs in, instead of the terminal
    /// (logind then owns the terminal), and the card it opened through it.
    pub seat: Option<std::rc::Rc<oxyos_seat::Seat>>,
    pub card_device: Option<oxyos_seat::Device>,
    /// Another terminal has the display and the devices: frames complete without being shown.
    pub away: bool,
    /// What checks the lock's passwords; without one, none unlocks.
    pub authentication: Option<Box<dyn Authentication>>,
    /// The lock surface whose password is being checked.
    checking: Option<SurfaceKey>,
    /// Whether the frame being composed, and the one last shown, were of the locked session.
    composing_locked: bool,
    shown_locked: bool,
    /// The machine is about to sleep: it may once the lock is on screen.
    sleeping: bool,
    /// What logind was last told of the lock.
    hinted_locked: bool,
    /// Surfaces arriving and going, animated (docs/adr/0011-transitions.mdx), and the last frame
    /// composed of each surface shown, which one going is drawn from.
    pub transitions: transitions::Transitions,
    shown: HashMap<SurfaceKey, transitions::Snapshot>,
    /// Where the last frame drew surfaces in transition (their rectangle and where they were
    /// placed): what the next frame must compose again, with where they are drawn then.
    transition_drawn: Vec<oxyos_gpu::Rect>,
    /// The damage of the last frame composed (`None`: whole), for tests and records.
    pub last_damage: Option<Vec<oxyos_gpu::Rect>>,
    /// Frames composed while a transition ran, as the development compositor reports them.
    pub transition_frames: u64,
    /// The next frame is composed, and whole: the first, or after the display came back or the
    /// splash was shown.
    recompose: bool,
    /// The time transitions run on: `CLOCK_MONOTONIC`, or a test's.
    pub clock: Option<Duration>,
    /// The session's splash (splash.rs), over everything from `show_splash` to `end_splash`.
    splash: Option<transitions::Snapshot>,
}

/// The session's authentication path (ADR 0010): it checks a lock surface's password off the
/// compositor's thread, and only its `Accepted` unlocks. The session's is PAM; the development
/// compositor's a fixture, never a session's.
pub trait Authentication {
    /// Begins checking `secret`: `None` while it is checked, or an answer at once (busy,
    /// throttled) with the seconds until another may be tried.
    fn attempt(
        &mut self,
        secret: oxyos_compositor_core::scene::Secret,
    ) -> Option<(AuthOutcome, u32)>;
    /// Readable when the password being checked has its answer.
    fn as_fd(&self) -> BorrowedFd<'_>;
    /// The answer for the password being checked, once there is one.
    fn answer(&mut self) -> Option<(AuthOutcome, u32)>;
}

/// One layer's content as a frame composes it: a surface's pixels (`key`), a cover, or a
/// surface going; `rect` is where the surface is, `placed` where its transition draws it.
struct Content {
    key: Option<SurfaceKey>,
    rect: Rect,
    placed: Rect,
    extent: Size,
    format: Option<oxyos_protocol::Format>,
    pixels: Vec<u8>,
    source: Option<oxyos_gpu::Source>,
    changed: Option<(u64, Vec<oxyos_gpu::Rect>)>,
    opacity: u8,
}

impl Content {
    fn cover(rect: Rect) -> Self {
        Self {
            key: None,
            rect,
            placed: rect,
            extent: Size::new(1, 1),
            format: None,
            pixels: Vec::new(),
            source: None,
            changed: None,
            opacity: 255,
        }
    }
}

pub struct Gpu {
    pub compositor: oxyos_gpu::VulkanCompositor,
    pub frames: u64,
    pub mismatches: u64,
    /// The device's time for each frame composed for the display (GPU timestamps), in ms.
    pub device_ms: Vec<f64>,
    /// With `--gpu-only`: how long each readback of the shown frame took, in ms.
    pub readback_ms: Vec<f64>,
    /// For each frame composed for the display: the host's copy into the upload buffer, and
    /// submission to fence (`VulkanCompositor::last_host_ms`), in ms.
    pub host_ms: Vec<(f64, f64)>,
    /// The bytes each frame composed for the display uploaded.
    pub uploaded: Vec<u64>,
    /// The device's time before and after composing each frame for the display, in ms.
    pub phases_ms: Vec<(f64, f64)>,
}

impl Compositor {
    /// A compositor for `scene`'s output `output`, serving connections of the user `own_uid`.
    pub fn new(scene: Scene, own_uid: u32, output: Rect) -> Self {
        Self {
            output,
            scene,
            peers: HashMap::new(),
            shell_listener: None,
            next_client: 0,
            next_shell_client: ClientId::MAX - 2,
            own_uid,
            framebuffer: vec![DESKTOP; (output.width * output.height) as usize],
            composed: Vec::new(),
            frames: 0,
            input_log: Vec::new(),
            log_input: false,
            local_clients: Default::default(),
            local_events: Vec::new(),
            gpu: None,
            gpu_only: false,
            shown_kept: false,
            reads_back: true,
            records: true,
            ending: false,
            keep_frame: false,
            last_frame: Vec::new(),
            display: None,
            input: None,
            display_offset: (0, 0),
            scanout: Vec::new(),
            scanout_back: 0,
            flips: Vec::new(),
            pending_flip: None,
            pointer_flip: false,
            pointer: None,
            pointer_images: BTreeMap::new(),
            idle_wait: Duration::from_millis(5),
            flipped: Vec::new(),
            composing_since: std::time::Duration::ZERO,
            compositions: Vec::new(),
            reads: Vec::new(),
            damaged: Vec::new(),
            versions: HashMap::new(),
            next_version: 0,
            shown_versions: HashMap::new(),
            terminal: None,
            seat: None,
            card_device: None,
            away: false,
            authentication: None,
            checking: None,
            composing_locked: false,
            shown_locked: false,
            sleeping: false,
            hinted_locked: false,
            transitions: transitions::Transitions::default(),
            shown: HashMap::new(),
            transition_drawn: Vec::new(),
            last_damage: None,
            transition_frames: 0,
            recompose: true,
            clock: None,
            splash: None,
        }
    }

    /// Development session authority: same-user peers get ordinary application grants and
    /// nothing else. A shipping session derives grants from its launch records (#35).
    pub fn grants_for(&self, credentials: Credentials) -> Option<Grants> {
        (credentials.uid == self.own_uid).then(Grants::default)
    }

    pub fn accept(&mut self, listener: &Listener) -> Result<()> {
        self.accept_granting(listener, None)
    }

    /// Accepts a peer on `listener`, granting it `extra` beyond what its credentials give.
    fn accept_granting(&mut self, listener: &Listener, extra: Option<Grants>) -> Result<()> {
        let connection = match listener.accept() {
            Ok(connection) => connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let credentials = connection.peer_credentials()?;
        // Unauthorized peers and peers beyond the limit are closed at once; pending peers
        // count against the limit, so silent connections cannot exhaust descriptors.
        let Some(grants) = self.grants_for(credentials) else {
            return Ok(());
        };
        if self.peers.len() >= MAX_PEERS {
            return Ok(());
        }
        let grants = extra.unwrap_or(grants);
        let client = if extra.is_some() {
            self.next_shell_client -= 1;
            self.next_shell_client
        } else {
            self.next_client += 1;
            self.next_client
        };
        self.peers.insert(
            client,
            Peer {
                connection,
                protocol: ServerConnection::new(),
                outbox: VecDeque::new(),
                pipes: VecDeque::new(),
                buffers: HashMap::new(),
                presses: 0,
                accepted: Instant::now(),
                grants,
            },
        );
        Ok(())
    }

    /// Closes connections that have not sent Hello in time.
    pub fn expire_pending(&mut self) {
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

    pub fn fail(&mut self, client: ClientId, code: ErrorCode) {
        if let Some(peer) = self.peers.get(&client) {
            let _ = peer
                .connection
                .try_send_event(&Event::ProtocolError { code }, None);
        }
        self.disconnect(client);
    }

    pub fn disconnect(&mut self, client: ClientId) {
        self.versions.retain(|(key, _), _| key.client != client);
        self.scene.disconnect(client);
        self.peers.remove(&client);
        self.retire();
    }

    /// Drops buffers no consumer will read again, closing their descriptors.
    pub fn retire(&mut self) {
        for (client, buffer) in self.scene.drain_retired() {
            if let Some(peer) = self.peers.get_mut(&client) {
                peer.buffers.remove(&buffer);
            }
        }
    }

    pub fn receive(&mut self, client: ClientId) {
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
                peer.outbox.push_back((welcome, None));
                let grants = peer.grants;
                if self.scene.connect(client, grants).is_err() {
                    self.fail(client, ErrorCode::Unauthorized);
                }
                return;
            }
            Ok(None) => {}
            Err(code) => return self.fail(client, code),
        }
        let fd = match (request, fd) {
            (Request::ClipboardReceive { .. }, Some(pipe)) => {
                if let Err(code) = self.scene.handle(client, request) {
                    return self.fail(client, code);
                }
                // An allowed paste's pipe goes to the clipboard's owner with its ClipboardSend; a
                // refused one is closed here, and the receiver reads its end at once.
                if let Some(owner) = self.scene.take_transfer() {
                    if let Some(peer) = self.peers.get_mut(&owner) {
                        peer.pipes.push_back(pipe);
                    }
                }
                return;
            }
            (_, fd) => fd,
        };
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
        match request {
            Request::Submit {
                surface, sequence, ..
            } => {
                self.next_version += 1;
                self.versions.insert(
                    (
                        SurfaceKey {
                            client,
                            id: surface,
                        },
                        sequence,
                    ),
                    self.next_version,
                );
            }
            Request::Destroy { surface } => self.versions.retain(|(key, _), _| {
                *key != SurfaceKey {
                    client,
                    id: surface,
                }
            }),
            _ => {}
        }
        if goodbye {
            self.peers.remove(&client);
        }
        self.retire();
    }

    /// Routes scene events into bounded outboxes and sends what each socket accepts now.
    pub fn flush(&mut self) {
        for (client, event) in self.scene.drain_events() {
            if self.log_input {
                if let Some((surface, kind)) = input_kind(&event) {
                    self.input_log.push((client, surface, kind));
                }
            }
            if self.local_clients.contains(&client) {
                self.local_events.push((client, event));
                continue;
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
                let pipe = if event.carries_fd() {
                    match peer.pipes.pop_front() {
                        Some(pipe) => Some(pipe),
                        None => continue,
                    }
                } else {
                    None
                };
                peer.outbox.push_back((event, pipe));
            }
        }
        let mut unresponsive = Vec::new();
        for (client, peer) in &mut self.peers {
            while let Some((event, pipe)) = peer.outbox.front() {
                match peer
                    .connection
                    .try_send_event(event, pipe.as_ref().map(|pipe| pipe.as_fd()))
                {
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
    pub fn frame(&mut self) {
        if self.pending_flip.is_some() {
            return;
        }
        // The pointer is on the display's cursor plane, placed by every commit, in the shape the
        // surface under it wants.
        let image = self
            .pointer_images
            .get(&self.scene.pointer_shape())
            .map_or(0, |steps| pointer_step(steps, monotonic()));
        let pointer_moved = match (&mut self.display, self.away) {
            (Some(display), false) => display.set_pointer(self.pointer, image),
            _ => false,
        };
        let started = self.scene.begin_frame();
        let list = self.scene.frame_list();
        // Surfaces arriving start their transitions in this frame; those going are drawn from
        // their last frame composed (ADR 0011). Locked, nothing of the session is drawn going.
        let now = self.now();
        for appearance in self.scene.take_appearances() {
            if appearance.arrived {
                self.transitions
                    .arrive(appearance.key, appearance.kind, now);
            } else {
                let snapshot = self.shown.remove(&appearance.key);
                self.transitions
                    .go(appearance.key, appearance.kind, now, snapshot);
            }
        }
        if self.scene.is_locked() {
            self.transitions.lock();
        }
        let animating = self.transitions.advance(now);
        if started == 0 && list == self.composed && !animating && !self.recompose {
            self.scene.end_frame(0, Clock::Synthetic);
            if pointer_moved {
                self.present_pointer();
            }
            return;
        }
        self.composing_since = monotonic();
        self.composing_locked = self.scene.is_locked();
        if animating {
            self.transition_frames += 1;
        }
        self.transitions.record(now);
        // What changed on the output since the frame composed last (ADR 0005); everything when
        // there was none, or after the display came back.
        let scene = &self.scene;
        let drawn = |key: SurfaceKey, rect: Rect, extent: Size| match scene.presenting_damage(key) {
            Some(oxyos_compositor_core::SurfaceDamage::Rects(rects)) => Some(
                rects
                    .iter()
                    .map(|damage| on_output(damage, rect, extent))
                    .collect(),
            ),
            _ => None,
        };
        let changed_damage =
            (!self.recompose).then(|| output_damage(&self.composed, &list, &drawn));
        let output_rect = oxyos_gpu::Rect {
            x: self.output.x,
            y: self.output.y,
            width: self.output.width,
            height: self.output.height,
        };
        let area = |rects: &[oxyos_gpu::Rect]| {
            rects
                .iter()
                .filter_map(|rect| rect.intersect(&output_rect))
                .map(|rect| u64::from(rect.width) * u64::from(rect.height))
                .sum::<u64>()
        };
        // Each surface's pixels, read from its shared buffer, and the covers.
        // Where this frame draws surfaces in transition: their rectangle and where they are placed.
        let mut transition_drawn: Vec<oxyos_gpu::Rect> = Vec::new();
        let mut in_transition = |rect: Rect, placed: Rect| {
            for r in [rect, placed] {
                transition_drawn.push(oxyos_gpu::Rect {
                    x: r.x,
                    y: r.y,
                    width: r.width,
                    height: r.height,
                });
            }
        };
        let mut unreadable = Vec::new();
        let mut contents = Vec::new();
        for item in list.iter().copied() {
            match item {
                RenderItem::Surface { key, rect, frame } => match self.pixels(key, frame.buffer) {
                    Some((extent, format, pixels)) => {
                        let source = self.versions.get(&(key, frame.sequence)).map(|&version| {
                            oxyos_gpu::Source {
                                key: (u64::from(key.client) << 32) | u64::from(key.id),
                                version,
                            }
                        });
                        // What changed since the version composed last, when the frame says.
                        let changed = source.and_then(|source| {
                            let &from = self.shown_versions.get(&source.key)?;
                            match self.scene.presenting_damage(key)? {
                                oxyos_compositor_core::SurfaceDamage::Rects(rects)
                                    if from != source.version =>
                                {
                                    Some((
                                        from,
                                        rects
                                            .iter()
                                            .map(|r| oxyos_gpu::Rect {
                                                x: r.x,
                                                y: r.y,
                                                width: r.width,
                                                height: r.height,
                                            })
                                            .collect::<Vec<_>>(),
                                    ))
                                }
                                _ => None,
                            }
                        });
                        if let Some(source) = source {
                            self.shown_versions.insert(source.key, source.version);
                        }
                        // A lock surface fading in does so over the lock's cover.
                        let arriving = self.transitions.arriving(key, now);
                        if self.transitions.is_arriving(key) {
                            in_transition(rect, arriving.place(rect));
                        }
                        if arriving.opacity < 255 && self.transitions.lock_arriving(key) {
                            contents.push(Content::cover(rect));
                        }
                        contents.push(Content {
                            key: Some(key),
                            rect,
                            placed: arriving.place(rect),
                            extent,
                            format: Some(format),
                            pixels,
                            source,
                            changed,
                            opacity: arriving.opacity,
                        })
                    }
                    None => unreadable.push(key.client),
                },
                RenderItem::Cover { rect, .. } => contents.push(Content::cover(rect)),
            }
        }
        // The splash over everything while the session starts.
        if let Some(splash) = &self.splash {
            contents.push(Content {
                key: None,
                rect: splash.rect,
                placed: splash.rect,
                extent: splash.extent,
                format: Some(splash.format),
                pixels: splash.pixels.clone(),
                source: None,
                changed: None,
                opacity: 255,
            });
        }
        // Surfaces going, over what stays, from their last frames.
        for (snapshot, _, at) in self.transitions.going(now) {
            in_transition(snapshot.rect, at.place(snapshot.rect));
            contents.push(Content {
                key: None,
                rect: snapshot.rect,
                placed: at.place(snapshot.rect),
                extent: snapshot.extent,
                format: Some(snapshot.format),
                pixels: snapshot.pixels.clone(),
                source: None,
                changed: None,
                opacity: at.opacity,
            });
        }
        if self.records {
            self.reads
                .push((monotonic() - self.composing_since).as_secs_f64() * 1e3);
        }
        // What changed since the frame composed last (ADR 0005), and, while surfaces arrive or
        // go, where they were drawn then and are drawn now (ADR 0011): an opacity or a scale
        // changes no surface's frame.
        let damage = changed_damage.map(|mut rects| {
            rects.extend(self.transition_drawn.iter().copied());
            rects.extend(transition_drawn.iter().copied());
            rects
        });
        self.transition_drawn = transition_drawn;
        if self.records {
            self.damaged.push(match &damage {
                Some(rects) => (area(rects) as f64 / area(&[output_rect]) as f64).min(1.0),
                None => 1.0,
            });
        }
        self.last_damage = damage.clone();
        let layers: Vec<oxyos_gpu::Layer<'_>> = contents
            .iter()
            .map(|content| oxyos_gpu::Layer {
                x: content.placed.x,
                y: content.placed.y,
                width: content.placed.width,
                height: content.placed.height,
                extent: (content.extent.width, content.extent.height),
                paint: match content.format {
                    Some(oxyos_protocol::Format::Xrgb8888) => {
                        oxyos_gpu::Paint::Opaque(&content.pixels)
                    }
                    Some(oxyos_protocol::Format::Argb8888) => {
                        oxyos_gpu::Paint::Premultiplied(&content.pixels)
                    }
                    None => oxyos_gpu::Paint::Fill(0),
                },
                source: content.source,
                changed: content
                    .changed
                    .as_ref()
                    .map(|(from, rects)| (*from, rects.as_slice())),
                opacity: content.opacity,
            })
            .collect();
        // Frames older than those composed are never shown again.
        self.versions.retain(|(key, sequence), _| {
            list.iter().all(|item| match item {
                RenderItem::Surface {
                    key: shown, frame, ..
                } if shown == key => *sequence >= frame.sequence,
                _ => true,
            })
        });
        // The output is at the origin of the global space.
        if !self.gpu_only {
            self.framebuffer =
                oxyos_gpu::compose_cpu(self.output.width, self.output.height, DESKTOP, &layers);
        }
        if self.keep_frame {
            self.last_frame.clear();
            oxyos_gpu::frame_file::write(
                &mut self.last_frame,
                self.output.width,
                self.output.height,
                DESKTOP,
                &layers,
            )
            .expect("writing to memory");
        }
        if let (Some(gpu), false) = (&mut self.gpu, self.gpu_only) {
            gpu.frames += 1;
            let composed = match &damage {
                Some(damage) => gpu.compositor.compose_damaged(
                    self.output.width,
                    self.output.height,
                    DESKTOP,
                    &layers,
                    damage,
                ),
                None => {
                    gpu.compositor
                        .compose(self.output.width, self.output.height, DESKTOP, &layers)
                }
            };
            match composed {
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
        self.recompose = false;
        self.frames += 1;
        // On a display, the frame completes when it is on screen, at the kernel's flip time, in
        // a later turn (`complete_flip`).
        let presented = self.present(&layers, damage.as_deref());
        if !presented {
            self.scene.end_frame(0, Clock::Synthetic);
            self.retire();
            if self.gpu_only && self.reads_back {
                self.flush();
                self.framebuffer_from_gpu(&layers);
            }
        }
        // What each surface shown showed: what it is drawn from if it goes (ADR 0011).
        drop(layers);
        self.shown = contents
            .into_iter()
            .filter_map(|content| {
                let key = content.key?;
                Some((
                    key,
                    transitions::Snapshot {
                        rect: content.rect,
                        extent: content.extent,
                        format: content.format?,
                        pixels: content.pixels,
                    },
                ))
            })
            .collect();
    }

    /// Shows the session's splash, the Oxy mark on black, over everything from the next frame.
    pub fn show_splash(&mut self) {
        self.splash = splash::splash(Size::new(self.output.width, self.output.height));
        // The next frame is composed whole, with it.
        self.composed.clear();
        self.recompose = true;
    }

    pub fn showing_splash(&self) -> bool {
        self.splash.is_some()
    }

    /// The session is on screen: the splash fades out over it (400 ms).
    pub fn end_splash(&mut self) {
        if let Some(splash) = self.splash.take() {
            let now = self.now();
            self.transitions.fade(splash, now);
        }
    }

    /// The time transitions run on.
    fn now(&self) -> Duration {
        self.clock.unwrap_or_else(monotonic)
    }

    /// Only the pointer moved (or its animated image went on): the frame shown is committed again
    /// with it where it is now, no
    /// frame composed. Its flip completes no client's frame.
    fn present_pointer(&mut self) {
        let Some(display) = self.display.as_mut() else {
            return;
        };
        match display.present_again() {
            Ok(true) => {
                self.pending_flip = Some(monotonic());
                self.pointer_flip = true;
            }
            Ok(false) => {}
            Err(error) => {
                eprintln!("[kms] {error}; display output stops");
                self.display = None;
            }
        }
    }

    /// Completes the frame on its way to the display once it has flipped, at the kernel's flip
    /// time; with `wait`, waits for the flip (before giving the display away, or at the end).
    pub fn complete_flip(&mut self, wait: bool) {
        let Some(submitted) = self.pending_flip else {
            return;
        };
        let flip = match &self.display {
            Some(display) if wait => {
                // Bounded: a seat manager may have taken the display meanwhile.
                let deadline = Instant::now() + Duration::from_secs(1);
                loop {
                    match display.flip_done() {
                        Ok(None) if Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(1))
                        }
                        other => break other,
                    }
                }
            }
            Some(display) => display.flip_done(),
            None => Err(std::io::Error::other("the display went away")),
        };
        if self.pointer_flip {
            // The pointer moved on the frame shown: nothing of the scene completes.
            match flip {
                Ok(None) if !wait => return,
                Err(error) => {
                    eprintln!("[kms] {error}; display output stops");
                    self.display = None;
                }
                _ => {}
            }
            self.pointer_flip = false;
            self.pending_flip = None;
            return;
        }
        let (timestamp_ns, clock) = match flip {
            Ok(None) if !wait => return,
            // Never flipped: the frame completes unshown, and is composed again.
            Ok(None) => {
                eprintln!("[kms] a frame never flipped");
                self.composed.clear();
                self.recompose = true;
                (0, Clock::Synthetic)
            }
            Ok(Some(flip)) => {
                if self.records {
                    self.flips.push((submitted, flip));
                }
                (
                    u64::try_from(flip.as_nanos()).unwrap_or(u64::MAX),
                    Clock::Monotonic,
                )
            }
            Err(error) => {
                eprintln!("[kms] {error}; display output stops");
                self.display = None;
                // Composed again for the director's checks, without the display.
                self.composed.clear();
                self.recompose = true;
                (0, Clock::Synthetic)
            }
        };
        self.pending_flip = None;
        self.scene.end_frame(timestamp_ns, clock);
        self.retire();
        if clock == Clock::Monotonic {
            self.flipped = self.scene.render_list();
            self.shown_locked = self.composing_locked;
        }
        if self.gpu_only && self.reads_back && clock == Clock::Monotonic {
            // The clients hear the frame was shown before the director reads it back, so its
            // check delays no client's next frame.
            self.flush();
            self.framebuffer_from_gpu(&[]);
        }
    }

    /// With `--kms`: lets go of the display and the devices when the kernel switches to another
    /// terminal, and takes them back when it returns; the frame is then shown again.
    pub fn follow_terminal(&mut self) -> Result<()> {
        let Some(terminal) = self.terminal.as_mut() else {
            return Ok(());
        };
        for switch in terminal.switches()? {
            match switch {
                oxyos_vt::Switch::Release if !self.away => {
                    // The frame on its way is shown before the display is given away.
                    self.complete_flip(true);
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
                oxyos_vt::Switch::Acquire if self.away => {
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
                    self.recompose = true;
                }
                // A release while away, or an acquire while here: nothing to give or take.
                oxyos_vt::Switch::Release | oxyos_vt::Switch::Acquire => {}
            }
        }
        Ok(())
    }

    /// With `--seat`: lets go of the display and the devices when logind pauses the card
    /// (another session has the seat), acknowledging each pause, and takes them back when it
    /// resumes; the frame is then shown again with a modeset.
    pub fn follow_seat(&mut self) -> Result<()> {
        let Some(seat) = self.seat.clone() else {
            return Ok(());
        };
        for event in seat.events() {
            match event {
                oxyos_seat::Event::Paused(device, pause) => {
                    if Some(device) == self.card_device && !self.away {
                        self.complete_flip(true);
                        if let Some(display) = &mut self.display {
                            display.taken_back();
                        }
                        if let Some(input) = &mut self.input {
                            input.suspend();
                        }
                        // libinput released what was still down: applications hear it now.
                        self.read_input();
                        self.away = true;
                    }
                    if pause == oxyos_seat::Pause::Pause {
                        seat.paused(device)?;
                    }
                }
                // Input devices are opened anew when libinput resumes; the card keeps its own.
                oxyos_seat::Event::Lock => self.scene.lock(),
                // Sleep waits for the lock to be on screen (`follow_lock`); on waking, the next
                // sleep is held back again.
                oxyos_seat::Event::PrepareForSleep(true) => {
                    self.scene.lock();
                    self.sleeping = true;
                }
                oxyos_seat::Event::PrepareForSleep(false) => {
                    self.sleeping = false;
                    if let Err(error) = seat.hold_sleep() {
                        eprintln!("[seat] holding sleep back: {error}");
                    }
                }
                oxyos_seat::Event::Resumed(device, fd) => {
                    drop(fd);
                    if Some(device) == self.card_device && self.away {
                        if let Some(input) = &mut self.input {
                            input.resume()?;
                        }
                        self.away = false;
                        // Composed again, and shown with a modeset.
                        self.composed.clear();
                        self.recompose = true;
                    }
                }
            }
        }
        Ok(())
    }

    /// With `--input`: what the seat's devices did since the last turn, applied to the scene.
    /// Positions arrive in the display's pixels; the output is centred on the display.
    pub fn read_input(&mut self) {
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
        let trace = std::env::var_os("OXYOS_INPUT_TRACE").is_some();
        for action in actions {
            if trace {
                eprintln!("[input] {action:?} focus {:?}", self.scene.focus());
            }
            match action {
                oxyos_input::Action::Motion { x, y } => {
                    let (x, y) = (x.round() as i32, y.round() as i32);
                    self.pointer = Some((x, y));
                    self.scene.pointer_motion(x - dx, y - dy)
                }
                oxyos_input::Action::Button { code, pressed } => {
                    self.scene.pointer_button(code, pressed)
                }
                // libinput's units are logical pixels; the protocol's are 1/256 of one.
                oxyos_input::Action::Scroll { dx, dy } => self
                    .scene
                    .pointer_scroll((dx * 256.0).round() as i32, (dy * 256.0).round() as i32),
                // The protocol's spread is in 1/65536 and its turn in 1/256 degrees.
                oxyos_input::Action::Pinch {
                    phase,
                    scale,
                    rotation,
                } => self.scene.pointer_pinch(
                    match phase {
                        oxyos_input::GesturePhase::Begin => GesturePhase::Begin,
                        oxyos_input::GesturePhase::Update => GesturePhase::Update,
                        oxyos_input::GesturePhase::End => GesturePhase::End,
                        oxyos_input::GesturePhase::Cancel => GesturePhase::Cancel,
                    },
                    (scale.max(0.0) * 65536.0).round() as u32,
                    (rotation * 256.0).round() as i32,
                ),
                oxyos_input::Action::Key {
                    code,
                    pressed,
                    text,
                } => self.scene.keyboard(code, pressed, text.as_deref()),
                oxyos_input::Action::Terminate => self.ending = true,
                oxyos_input::Action::Lock => self.scene.lock(),
                oxyos_input::Action::CapsLock(on) => self.scene.set_caps_lock(on),
                oxyos_input::Action::SwitchTerminal(number) => {
                    if let Some(seat) = &self.seat {
                        if Some(u32::from(number)) != seat.vt() {
                            if let Err(error) = seat.switch_to(u32::from(number)) {
                                eprintln!("[seat] switching to {number}: {error}");
                            }
                        }
                    }
                    if let Some(terminal) = &self.terminal {
                        if number != terminal.number() {
                            if let Err(error) = terminal.activate(number) {
                                eprintln!("[vt] switching to {number}: {error}");
                            }
                        }
                    }
                }
                // What a sequence composes replaces its composition when committed.
                oxyos_input::Action::Compose { composition, text } => match text {
                    Some(text) => self.scene.text(&text),
                    None => {
                        self.scene.preedit(&composition, composition.len());
                    }
                },
            }
        }
    }

    /// With `--gpu-only`: the director's frame is the GPU's, the output's part of the frame shown
    /// (read back after the flip) or, with nothing shown, composed for it. An error counts as a
    /// mismatch, and the run fails.
    pub fn framebuffer_from_gpu(&mut self, layers: &[oxyos_gpu::Layer<'_>]) {
        let display = self.display.as_ref().map(|display| display.size());
        let Some(gpu) = &mut self.gpu else {
            return;
        };
        gpu.frames += 1;
        let started = monotonic();
        let frame = match display {
            Some((width, height)) if self.shown_kept => {
                self.shown_kept = false;
                let dx = (width.saturating_sub(self.output.width) / 2) as usize;
                let dy = (height.saturating_sub(self.output.height) / 2) as usize;
                gpu.compositor.read_kept().map(|shown| {
                    shown
                        .chunks_exact(width as usize)
                        .skip(dy)
                        .take(self.output.height as usize)
                        .flat_map(|row| row[dx..dx + self.output.width as usize].iter().copied())
                        .collect()
                })
            }
            _ => gpu
                .compositor
                .compose(self.output.width, self.output.height, DESKTOP, layers),
        };
        gpu.readback_ms
            .push((monotonic() - started).as_secs_f64() * 1e3);
        match frame {
            Ok(frame) => self.framebuffer = frame,
            Err(error) => {
                eprintln!("[gpu] {error}");
                gpu.mismatches += 1;
            }
        }
    }

    /// With `--kms`: hands the composed output, centred, to the display, which flips to it at
    /// its next refresh (`complete_flip`); true once handed over. With `--gpu` too, the frame
    /// is composed on the Vulkan device straight into a scanout buffer, with no copy through
    /// the CPU. A display error ends display output.
    pub fn present(
        &mut self,
        layers: &[oxyos_gpu::Layer<'_>],
        damage: Option<&[oxyos_gpu::Rect]>,
    ) -> bool {
        if self.away {
            return false;
        }
        let Some(display) = self.display.as_mut() else {
            return false;
        };
        let (width, height) = display.size();
        if let (Some(gpu), false) = (&mut self.gpu, self.scanout.is_empty()) {
            let dx = (width.saturating_sub(self.output.width) / 2) as i32;
            let dy = (height.saturating_sub(self.output.height) / 2) as i32;
            let shifted: Vec<oxyos_gpu::Layer<'_>> = layers
                .iter()
                .map(|layer| oxyos_gpu::Layer {
                    x: layer.x + dx,
                    y: layer.y + dy,
                    ..*layer
                })
                .collect();
            let damage: Option<Vec<oxyos_gpu::Rect>> = damage.map(|rects| {
                rects
                    .iter()
                    .map(|rect| oxyos_gpu::Rect {
                        x: rect.x + dx,
                        y: rect.y + dy,
                        ..*rect
                    })
                    .collect()
            });
            let back = self.scanout_back;
            let composed = if self.gpu_only && self.reads_back {
                self.shown_kept = true;
                gpu.compositor
                    .compose_to_scanout_kept(back, DESKTOP, &shifted, damage.as_deref())
            } else {
                gpu.compositor
                    .compose_to_scanout(back, DESKTOP, &shifted, damage.as_deref())
            }
            .map_err(|error| std::io::Error::other(error.to_string()));
            let submitted = monotonic();
            if self.records {
                gpu.device_ms.extend(gpu.compositor.last_gpu_ms());
                gpu.host_ms.push(gpu.compositor.last_host_ms());
                gpu.uploaded.push(gpu.compositor.last_uploaded_bytes());
                gpu.phases_ms.extend(gpu.compositor.last_gpu_phases());
                self.compositions
                    .push((submitted - self.composing_since).as_secs_f64() * 1e3);
            }
            let presented = composed.and_then(|()| display.present_framebuffer(self.scanout[back]));
            self.scanout_back = 1 - back;
            return self.handed_over(presented, submitted);
        }
        let mut pixels = vec![DESKTOP; width as usize * height as usize];
        let dx = (width.saturating_sub(self.output.width) / 2) as usize;
        let dy = (height.saturating_sub(self.output.height) / 2) as usize;
        let columns = (self.output.width as usize).min(width as usize);
        for (y, row) in self
            .framebuffer
            .chunks_exact(self.output.width as usize)
            .enumerate()
        {
            if y + dy >= height as usize {
                break;
            }
            let start = (y + dy) * width as usize + dx;
            pixels[start..start + columns].copy_from_slice(&row[..columns]);
        }
        let drawn = display.draw(&pixels);
        let submitted = monotonic();
        if self.records {
            self.compositions
                .push((submitted - self.composing_since).as_secs_f64() * 1e3);
        }
        let presented = drawn.and_then(|()| display.present());
        self.handed_over(presented, submitted)
    }

    pub fn handed_over(
        &mut self,
        presented: std::io::Result<()>,
        submitted: std::time::Duration,
    ) -> bool {
        match presented {
            Ok(()) => {
                self.pending_flip = Some(submitted);
                true
            }
            Err(error) => {
                eprintln!("[kms] {error}; display output stops");
                self.display = None;
                false
            }
        }
    }

    /// A client buffer's pixels (tightly packed rows), size and format; `None` if the
    /// client's buffer could not be read.
    pub fn pixels(
        &self,
        key: SurfaceKey,
        buffer: u64,
    ) -> Option<(Size, oxyos_protocol::Format, Vec<u8>)> {
        let shared = self.peers.get(&key.client)?.buffers.get(&buffer)?;
        let layout = shared.layout();
        let row = layout.extent.width as usize * 4;
        let mut pixels = vec![0; row * layout.extent.height as usize];
        for (y, out) in pixels.chunks_exact_mut(row).enumerate() {
            shared.read_row(y as u32, out).ok()?;
        }
        Some((layout.extent, layout.format, pixels))
    }

    /// How long a turn may wait: until the animated pointer's next step, the first connection
    /// still without its Hello reaches its deadline, or `idle_wait`, whichever is soonest.
    fn next_wake(&self) -> Duration {
        let mut wait = self.idle_wait;
        // A transition running composes a frame for every refresh (a display's flip wakes the
        // turn sooner).
        if !self.transitions.is_empty() {
            wait = wait.min(Duration::from_millis(16));
        }
        let steps = self.pointer_images.get(&self.scene.pointer_shape());
        if let Some(step) = steps.and_then(|steps| pointer_step_wait(steps, monotonic())) {
            wait = wait.min(step);
        }
        for peer in self.peers.values() {
            if peer.protocol.version().is_none() {
                wait = wait.min(HELLO_DEADLINE.saturating_sub(peer.accepted.elapsed()));
            }
        }
        wait
    }

    /// One turn of the compositor: accepts connections, handles every readable client, composes
    /// a frame and sends events. Waits for a client, the listener, the display's flip, the devices
    /// or the seat, at most until the pointer's next step, a connection's deadline or `idle_wait`.
    pub fn serve(&mut self, listener: &Listener) -> Result<()> {
        self.follow_terminal()?;
        self.follow_seat()?;
        self.read_input();
        let clients: Vec<ClientId> = self.peers.keys().copied().collect();
        // Poll the listener, every client and, with a frame on its way, the display (its flip);
        // never block on any one of them.
        let shell = usize::from(self.shell_listener.is_some());
        let readable: Vec<bool> = {
            let mut fds: Vec<PollFd<'_>> = Vec::with_capacity(clients.len() + 3);
            fds.push(PollFd::from_borrowed_fd(listener.as_fd(), PollFlags::IN));
            if let Some(shell_listener) = &self.shell_listener {
                fds.push(PollFd::from_borrowed_fd(
                    shell_listener.as_fd(),
                    PollFlags::IN,
                ));
            }
            for client in &clients {
                // A client with events waiting for room in its socket wakes the turn when it has
                // some.
                let peer = &self.peers[client];
                let flags = if peer.outbox.is_empty() {
                    PollFlags::IN
                } else {
                    PollFlags::IN | PollFlags::OUT
                };
                fds.push(PollFd::from_borrowed_fd(peer.connection.as_fd(), flags));
            }
            if let (Some(display), Some(_)) = (&self.display, self.pending_flip) {
                fds.push(PollFd::from_borrowed_fd(
                    display.card().as_fd(),
                    PollFlags::IN,
                ));
            }
            // libinput and the seat are read every turn; they only need to wake it.
            if let Some(input) = &self.input {
                fds.push(PollFd::from_borrowed_fd(input.as_fd(), PollFlags::IN));
            }
            if let Some(seat) = &self.seat {
                fds.push(PollFd::from_borrowed_fd(seat.as_fd(), PollFlags::IN));
            }
            if let (Some(authentication), Some(_)) = (&self.authentication, self.checking) {
                fds.push(PollFd::from_borrowed_fd(
                    authentication.as_fd(),
                    PollFlags::IN,
                ));
            }
            let wait = self.next_wake();
            let timeout = rustix::event::Timespec {
                tv_sec: wait.as_secs() as i64,
                tv_nsec: i64::from(wait.subsec_nanos()),
            };
            match poll(&mut fds, Some(&timeout)) {
                Ok(_) => fds.iter().map(|fd| !fd.revents().is_empty()).collect(),
                // A signal (the terminal's switch requests) interrupts poll, which is never
                // restarted: nothing was ready this turn.
                Err(rustix::io::Errno::INTR) => vec![false; fds.len()],
                Err(error) => return Err(error.into()),
            }
        };
        self.complete_flip(false);
        if readable[0] {
            self.accept(listener)?;
        }
        if shell == 1 && readable[1] {
            let shell_listener = self.shell_listener.take().expect("polled");
            let accepted = self.accept_granting(
                &shell_listener,
                // The shell is the lock screen as well (ADR 0010).
                Some(Grants {
                    shell: true,
                    lock: true,
                    ..Grants::default()
                }),
            );
            self.shell_listener = Some(shell_listener);
            accepted?;
        }
        for (index, client) in clients.iter().enumerate() {
            if readable[index + 1 + shell] {
                self.receive(*client);
            }
        }
        self.expire_pending();
        self.authenticate();
        self.frame();
        self.follow_lock();
        self.flush();
        Ok(())
    }

    /// Whether the lock is on screen: the session is locked and the frame shown is of it (or
    /// nothing is shown: there is no display, or another terminal has it).
    pub fn lock_shown(&self) -> bool {
        self.scene.is_locked()
            && (self.display.is_none()
                || self.away
                || (self.shown_locked && self.pending_flip.is_none()))
    }

    /// Tells logind of the lock as it changes, and lets the machine sleep once it is on screen.
    fn follow_lock(&mut self) {
        let Some(seat) = self.seat.clone() else {
            return;
        };
        let locked = self.scene.is_locked();
        if locked != self.hinted_locked {
            self.hinted_locked = locked;
            if let Err(error) = seat.set_locked_hint(locked) {
                eprintln!("[seat] the locked hint: {error}");
            }
        }
        if self.sleeping && self.lock_shown() {
            self.sleeping = false;
            seat.release_sleep();
        }
    }

    /// Answers the password being checked once it has its answer, unlocking on `Accepted`, and
    /// hands the next one a lock surface submitted to the authentication path.
    pub fn authenticate(&mut self) {
        if let Some(key) = self.checking {
            let answer = self
                .authentication
                .as_mut()
                .and_then(|authentication| authentication.answer());
            if let Some((outcome, wait_s)) = answer {
                self.checking = None;
                self.scene.authenticated(key, outcome, wait_s);
                if outcome == AuthOutcome::Accepted {
                    self.scene.unlock();
                }
            }
        }
        while let Some((key, secret)) = self.scene.take_authentication() {
            let Some(authentication) = self.authentication.as_mut() else {
                eprintln!("[lock] no authentication path: the session stays locked");
                self.scene.authenticated(key, AuthOutcome::Rejected, 0);
                continue;
            };
            match authentication.attempt(secret) {
                None => self.checking = Some(key),
                Some((outcome, wait_s)) => self.scene.authenticated(key, outcome, wait_s),
            }
        }
    }

    pub fn pixel(&self, x: i32, y: i32) -> u32 {
        self.framebuffer[(y as u32 * self.output.width + x as u32) as usize]
    }

    pub fn surface_rect(&self, client: ClientId, id: u32) -> Option<Rect> {
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

    pub fn surface_sequence(&self, client: ClientId, id: u32) -> Option<u64> {
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

/// A surface's damage (buffer pixels) on the output, where the surface shows at `rect`: scaled
/// as the surface is, rounded outward so every output pixel sampling a damaged buffer pixel is
/// in it.
pub fn on_output(damage: &Rect, rect: Rect, extent: Size) -> oxyos_gpu::Rect {
    let scale = |at: u32, length: u32, extent: u32, up: bool| {
        let product = u64::from(at) * u64::from(length);
        let extent = u64::from(extent.max(1));
        (if up {
            product.div_ceil(extent)
        } else {
            product / extent
        }) as i64
    };
    let (x0, y0) = (damage.x.max(0) as u32, damage.y.max(0) as u32);
    let left = scale(x0, rect.width, extent.width, false);
    let top = scale(y0, rect.height, extent.height, false);
    let right = scale(x0 + damage.width, rect.width, extent.width, true);
    let bottom = scale(y0 + damage.height, rect.height, extent.height, true);
    oxyos_gpu::Rect {
        x: rect.x + left as i32,
        y: rect.y + top as i32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    }
}

/// What changed on the output from one composed frame to the next (ADR 0005): a surface's
/// rectangle when it is new or gone, its damage (`drawn`, on the output) or else its rectangle
/// when it shows a new frame, its old and new rectangles when it moved, resized or changed
/// places in the stacking, and covers that came or went.
pub fn output_damage(
    before: &[RenderItem],
    after: &[RenderItem],
    drawn: &dyn Fn(SurfaceKey, Rect, Size) -> Option<Vec<oxyos_gpu::Rect>>,
) -> Vec<oxyos_gpu::Rect> {
    let rect = |rect: Rect| oxyos_gpu::Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    };
    let surfaces = |list: &[RenderItem]| -> Vec<(SurfaceKey, Rect, oxyos_compositor_core::Frame)> {
        list.iter()
            .filter_map(|item| match *item {
                RenderItem::Surface { key, rect, frame } => Some((key, rect, frame)),
                RenderItem::Cover { .. } => None,
            })
            .collect()
    };
    let (old, new) = (surfaces(before), surfaces(after));
    let mut damage = Vec::new();
    for &(key, now, frame) in &new {
        match old.iter().find(|(old_key, ..)| *old_key == key) {
            None => damage.push(rect(now)),
            Some(&(_, then, _)) if then != now => damage.extend([rect(then), rect(now)]),
            Some(&(.., then_frame)) if then_frame != frame => match drawn(key, now, frame.extent) {
                Some(drawn) => damage.extend(drawn),
                None => damage.push(rect(now)),
            },
            Some(_) => {}
        }
    }
    for &(key, then, _) in &old {
        if !new.iter().any(|(new_key, ..)| *new_key == key) {
            damage.push(rect(then));
        }
    }
    // Surfaces in both whose order among themselves changed show differently where they meet.
    let order = |list: &[(SurfaceKey, Rect, oxyos_compositor_core::Frame)],
                 other: &[(SurfaceKey, Rect, oxyos_compositor_core::Frame)]| {
        list.iter()
            .filter(|(key, ..)| other.iter().any(|(other_key, ..)| other_key == key))
            .map(|&(key, rect, _)| (key, rect))
            .collect::<Vec<_>>()
    };
    let (old_order, new_order) = (order(&old, &new), order(&new, &old));
    for (then, now) in old_order.iter().zip(&new_order) {
        if then.0 != now.0 {
            damage.extend([rect(then.1), rect(now.1)]);
        }
    }
    let covers = |list: &[RenderItem]| -> Vec<RenderItem> {
        list.iter()
            .filter(|item| matches!(item, RenderItem::Cover { .. }))
            .copied()
            .collect()
    };
    let (old_covers, new_covers) = (covers(before), covers(after));
    for (these, others) in [(&old_covers, &new_covers), (&new_covers, &old_covers)] {
        for cover in these.iter().filter(|cover| !others.contains(cover)) {
            if let RenderItem::Cover { rect: cover, .. } = cover {
                damage.push(rect(*cover));
            }
        }
    }
    damage
}

/// The image of an animated pointer's `steps` at time `now`: the animation loops from time zero,
/// so every animated cursor turns with the clock. A single image never changes.
fn pointer_step(steps: &[(usize, std::time::Duration)], now: std::time::Duration) -> usize {
    let cycle: std::time::Duration = steps.iter().map(|(_, time)| *time).sum();
    if steps.len() < 2 || cycle.is_zero() {
        return steps.first().map_or(0, |(image, _)| *image);
    }
    let mut at = std::time::Duration::from_nanos((now.as_nanos() % cycle.as_nanos()) as u64);
    for (image, time) in steps {
        if at < *time {
            return *image;
        }
        at -= *time;
    }
    steps[steps.len() - 1].0
}

/// How long until an animated pointer's `steps` go on to their next image at time `now`; none for
/// an image that never changes.
fn pointer_step_wait(
    steps: &[(usize, std::time::Duration)],
    now: std::time::Duration,
) -> Option<std::time::Duration> {
    let cycle: std::time::Duration = steps.iter().map(|(_, time)| *time).sum();
    if steps.len() < 2 || cycle.is_zero() {
        return None;
    }
    let mut at = std::time::Duration::from_nanos((now.as_nanos() % cycle.as_nanos()) as u64);
    for (_, time) in steps {
        if at < *time {
            return Some(*time - at);
        }
        at -= *time;
    }
    Some(std::time::Duration::ZERO)
}

/// Now, on CLOCK_MONOTONIC (the clock of the kernel's flip timestamps).
pub fn monotonic() -> std::time::Duration {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    std::time::Duration::new(now.tv_sec as u64, now.tv_nsec as u32)
}

/// The input events a step can cause, by surface and kind (motion is continuous and left out).
pub fn input_kind(event: &Event) -> Option<(u32, &'static str)> {
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
        // A transaction counts once, whatever parts it has.
        Event::TextInputDone { surface, .. } => (surface, "text_input"),
        Event::Scroll { surface, .. } => (surface, "scroll"),
        Event::PopupDone { surface, .. } => (surface, "popup.done"),
        Event::Focus { surface, focused } => {
            (surface, if focused { "focus.in" } else { "focus.out" })
        }
        _ => return None,
    })
}

/// Removes the private socket directory on every exit, including errors and deadlines.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_waits_until_an_animated_pointers_next_step() {
        let ms = std::time::Duration::from_millis;
        let steps = [(3, ms(30)), (4, ms(10)), (5, ms(20))];
        let wait = |t| pointer_step_wait(&steps, ms(t));
        assert_eq!(
            (wait(0), wait(29), wait(30), wait(45)),
            (Some(ms(30)), Some(ms(1)), Some(ms(10)), Some(ms(15)))
        );
        // It loops.
        assert_eq!(wait(60 + 35), Some(ms(5)));
        // One image never changes: nothing to wake for.
        assert_eq!(pointer_step_wait(&[(7, ms(30))], ms(5)), None);
        assert_eq!(pointer_step_wait(&[(7, ms(0)), (8, ms(0))], ms(5)), None);
    }

    #[test]
    fn an_animated_pointer_shows_the_step_the_clock_is_in() {
        let ms = std::time::Duration::from_millis;
        let steps = [(3, ms(30)), (4, ms(10)), (5, ms(20))];
        let at = |t| pointer_step(&steps, ms(t));
        assert_eq!(
            (at(0), at(29), at(30), at(39), at(40), at(59)),
            (3, 3, 4, 4, 5, 5)
        );
        // It loops.
        assert_eq!((at(60), at(95), at(6000 + 45)), (3, 4, 5));
        // One image, or steps that take no time, never change.
        assert_eq!(
            pointer_step(&[(7, std::time::Duration::ZERO)], ms(12345)),
            7
        );
        assert_eq!(pointer_step(&[(7, ms(0)), (8, ms(0))], ms(5)), 7);
        assert_eq!(pointer_step(&[], ms(5)), 0);
    }
    use oxyos_compositor_core::RenderItem;

    fn surface(id: u32, x: i32, sequence: u64) -> RenderItem {
        RenderItem::Surface {
            key: SurfaceKey { client: 1, id },
            rect: Rect {
                x,
                y: 0,
                width: 10,
                height: 10,
            },
            frame: oxyos_compositor_core::Frame {
                sequence,
                buffer: 1,
                configure: 1,
                extent: Size::new(10, 10),
            },
        }
    }

    fn at(x: i32) -> oxyos_gpu::Rect {
        oxyos_gpu::Rect {
            x,
            y: 0,
            width: 10,
            height: 10,
        }
    }

    fn output_damage(before: &[RenderItem], after: &[RenderItem]) -> Vec<oxyos_gpu::Rect> {
        super::output_damage(before, after, &|_, _, _| None)
    }

    #[test]
    fn a_surfaces_damage_is_on_the_output_where_it_shows() {
        let before = [surface(1, 0, 1)];
        let after = [surface(1, 0, 2)];
        // The surface's own damage, not its whole rectangle.
        let drawn = |_: SurfaceKey, rect: Rect, extent: Size| {
            Some(vec![on_output(&Rect::new(2, 3, 4, 1), rect, extent)])
        };
        assert_eq!(
            super::output_damage(&before, &after, &drawn),
            [oxyos_gpu::Rect {
                x: 2,
                y: 3,
                width: 4,
                height: 1
            }]
        );
        // Scaled outward: a 10x10 buffer at 15x15 (1.5x), pixels 3 to 5 cover output 4 to 9.
        let shown = Rect::new(100, 0, 15, 15);
        assert_eq!(
            on_output(&Rect::new(3, 3, 3, 3), shown, Size::new(10, 10)),
            oxyos_gpu::Rect {
                x: 104,
                y: 4,
                width: 5,
                height: 5
            }
        );
    }

    #[test]
    fn output_damage_is_what_changed() {
        let before = [surface(1, 0, 1), surface(2, 20, 1)];
        assert_eq!(output_damage(&before, &before), [], "nothing changed");
        // A new frame damages its surface; a move damages both places.
        assert_eq!(
            output_damage(&before, &[surface(1, 0, 2), surface(2, 20, 1)]),
            [at(0)]
        );
        assert_eq!(
            output_damage(&before, &[surface(1, 5, 1), surface(2, 20, 1)]),
            [at(0), at(5)]
        );
        // A surface going away damages where it was; a new one where it is.
        assert_eq!(output_damage(&before, &[surface(1, 0, 1)]), [at(20)]);
        assert_eq!(output_damage(&[surface(1, 0, 1)], &before), [at(20)]);
        // Swapped in the stacking: both, where they meet.
        let swapped = [surface(2, 20, 1), surface(1, 0, 1)];
        let damage = output_damage(&before, &swapped);
        assert!(
            damage.contains(&at(0)) && damage.contains(&at(20)),
            "{damage:?}"
        );
        // A cover that comes damages its rectangle.
        let cover = RenderItem::Cover {
            output: 1,
            rect: Rect {
                x: 0,
                y: 0,
                width: 30,
                height: 30,
            },
        };
        let covered = [surface(1, 0, 1), surface(2, 20, 1), cover];
        assert_eq!(
            output_damage(&before, &covered),
            [oxyos_gpu::Rect {
                x: 0,
                y: 0,
                width: 30,
                height: 30
            }]
        );
    }
}
