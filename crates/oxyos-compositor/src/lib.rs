//! The Oxy display compositor (docs/native-foundation.mdx): it serves client connections over
//! the protocol, routes input through the scene, composes the frame (on the CPU, or on a Vulkan
//! device straight into the display's scanout buffers) with only the output's damage, and shows
//! it on a DRM/KMS display, its devices from libinput and its seat from logind or the terminal.
//!
//! Two programs drive it: the development compositor (`oxyos-offscreen`), which scripts input and
//! checks every frame, and the session (`oxyos-session`).
//!
//! Each of the scene's outputs is a [`Screen`] (docs/adr/0018-several-displays.mdx): composed on
//! its own from the scene's frame list, clipped to it and scaled to its pixels, with its own
//! damage, display, flips and pointer.

use oxyos_compositor_core::{
    physical, ClientId, Grants, Output, OutputId, RenderItem, Scene, SurfaceKey, WindowLook,
};
use oxyos_ipc::{Connection, Credentials, Listener, SharedBuffer};
use oxyos_protocol::{
    AuthOutcome, Clock, ErrorCode, Event, GesturePhase, PointerKind, Rect, Request, ScrollSource,
    ServerConnection, Size, TextChunk,
};
use rustix::event::{poll, PollFd, PollFlags};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::error::Error;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub mod arrangement;
pub mod controls;
mod gpu_buffers;
pub mod scanout;
pub use gpu_buffers::GpuFrame;
pub mod displays;
pub mod idle;
pub mod installs;
pub mod login;
pub mod motion;
pub mod settings;
pub mod splash;
pub mod transitions;
pub mod window;

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

/// How each window of a frame is drawn (docs/adr/0016-windows.mdx), by surface.
pub type Looks = HashMap<SurfaceKey, WindowLook>;

/// The desktop's colour, where no surface is.
pub const DESKTOP: u32 = 0x10_14_18;
/// Events a client may leave unread before it is disconnected as unresponsive.
const MAX_OUTBOX: usize = 1024;
/// The most requests read from one client in a turn: a burst (a window's menus, an accessibility
/// snapshot, hundreds of packets) is read in a few turns, not one turn a packet, so a frame sent
/// after it is not left waiting behind it.
const REQUESTS_PER_TURN: usize = 256;
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
    /// What it may do, decided when it connected (its launch record adds to it in the scene).
    grants: Grants,
    /// Its process, as the kernel gave it when it connected.
    pub pid: i32,
    /// Its launch record was looked for (ADR 0013).
    named: bool,
}

impl Peer {
    /// Whether another request is waiting to be read, without waiting for one.
    fn has_request(&self) -> bool {
        let mut fds = [PollFd::from_borrowed_fd(
            self.connection.as_fd(),
            PollFlags::IN,
        )];
        let now = rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        matches!(poll(&mut fds, Some(&now)), Ok(ready) if ready > 0)
            && fds[0].revents().contains(PollFlags::IN)
    }
}

/// What the program running the compositor knows of a client's process from its launch records
/// (docs/adr/0013-launching-applications.mdx): the application it started it as, and what that
/// application may do beyond an application's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchRecord {
    pub application: String,
    pub grants: Grants,
}

/// One of the scene's outputs as the compositor composes and shows it
/// (docs/adr/0018-several-displays.mdx): its frame, its damage, its display and its flips, and
/// the pointer on its cursor plane.
pub struct Screen {
    /// The scene's output it shows.
    pub id: OutputId,
    /// Its rectangle in the global space, in logical pixels, and its scale (in 120ths).
    pub rect: Rect,
    pub scale_120: u32,
    /// Its display's refresh, in mHz (its output's, `Output::refresh_mhz`): what its frames are
    /// paced by, a display's flips or, `paced`, its own vblanks.
    pub refresh_mhz: u32,
    /// Without a display, whether it is paced as one at `refresh_mhz` (the development
    /// compositor's outputs given a refresh): a frame composed completes at its next vblank, in a
    /// later turn, as one on a display completes at its flip; otherwise at once.
    pub paced: bool,
    /// When its first vblank was (CLOCK_MONOTONIC), with `paced`: the phase of the others.
    vblank_origin: std::time::Duration,
    /// What it composes, in its own pixels: its display's mode, or its rectangle at its scale.
    pub width: u32,
    pub height: u32,
    /// The frame composed last, in its pixels (with `gpu_only`, the one read back, if any).
    pub framebuffer: Vec<u32>,
    /// The scene's frame list it composed last, and how its windows were drawn: what its next
    /// frame's damage is relative to.
    pub composed: Vec<RenderItem>,
    looks: Looks,
    /// Its next frame is composed, and whole: the first, or after its display came back or the
    /// splash was shown.
    recompose: bool,
    /// Where its last frame drew surfaces in transition (global, logical): what its next frame
    /// composes again.
    transition_drawn: Vec<oxyos_gpu::Rect>,
    /// The damage of its last frame composed, in its pixels (`None`: whole), for tests and
    /// records.
    pub last_damage: Option<Vec<oxyos_gpu::Rect>>,
    /// With `--kms`, or in a session: the display it is shown on, and completes its frames at
    /// its flips.
    pub display: Option<oxyos_kms::Display>,
    /// With `--gpu` too: the display's framebuffers, each imported from a Vulkan scanout target
    /// (its index in the Vulkan compositor), and the one composed next.
    pub scanout: Vec<(usize, oxyos_kms::Framebuffer)>,
    scanout_back: usize,
    /// For every frame shown: when it was handed to the display and when it flipped
    /// (CLOCK_MONOTONIC).
    pub flips: Vec<(std::time::Duration, std::time::Duration)>,
    /// The frame on its way to the display: when it was handed over. It completes at its flip,
    /// in a later turn; this output composes nothing meanwhile (one on its display at a time).
    pub pending_flip: Option<std::time::Duration>,
    /// The pending flip only moves the pointer: the frame shown, committed again.
    pointer_flip: bool,
    /// The display's pointer images for each shape the session has any for (the arrow for any
    /// other), with how long each shows: one image, or an animated cursor's steps.
    pub pointer_images: BTreeMap<oxyos_protocol::PointerShape, Vec<(usize, std::time::Duration)>>,
    /// The client's pointer image the display keeps (its surface, the image's number and its
    /// number on the display), shown at the display's scale.
    pub client_pointer: Option<(SurfaceKey, u64, usize)>,
    /// What its last flip showed (the scene's displayed content once it completed).
    pub flipped: Vec<RenderItem>,
    /// Whether the frame being composed, and the one last shown, were of the locked session.
    composing_locked: bool,
    shown_locked: bool,
    /// The frame just shown was kept on the device for `framebuffer_from_gpu`.
    pub shown_kept: bool,
    /// A full-screen window's buffer on its display's primary plane, and the surface told it
    /// could be (docs/adr/0031-gpu-buffers.mdx, decision 8).
    pub(crate) direct: scanout::Direct,
}

impl Screen {
    /// The display's number for a client's pointer image, put on its cursor plane at the
    /// display's scale the first time it is shown; None when the display cannot show it (no
    /// cursor plane, or larger than its plane takes), and the arrow is shown.
    pub fn show_client_pointer(
        &mut self,
        key: SurfaceKey,
        pointer: &ClientPointer,
    ) -> Option<usize> {
        let display = self.display.as_mut()?;
        if let Some((shown_key, id, index)) = self.client_pointer {
            if shown_key == key && id == pointer.id {
                return Some(index);
            }
        }
        let image = pointer_image_at(&pointer.image, pointer.scale_120, self.scale_120);
        let index = match self.client_pointer {
            Some((_, _, index)) => display.replace_pointer_image(index, &image).map(|()| index),
            None => display.add_pointer_image(&image),
        };
        match index {
            Ok(index) => {
                self.client_pointer = Some((key, pointer.id, index));
                Some(index)
            }
            Err(error) => {
                eprintln!("[compositor] a client's pointer image not shown: {error}");
                None
            }
        }
    }

    /// A screen for `output`, composing `pixels` (its display's mode); none: its rectangle at
    /// its scale.
    pub fn new(output: &Output, pixels: Option<(u32, u32)>) -> Self {
        let (width, height) = pixels.unwrap_or((
            scaled(output.rect.width, output.scale_120),
            scaled(output.rect.height, output.scale_120),
        ));
        Self {
            id: output.id,
            rect: output.rect,
            scale_120: output.scale_120,
            refresh_mhz: output.refresh_mhz,
            paced: false,
            vblank_origin: monotonic(),
            width,
            height,
            framebuffer: vec![DESKTOP; width as usize * height as usize],
            composed: Vec::new(),
            looks: Looks::new(),
            recompose: true,
            transition_drawn: Vec::new(),
            last_damage: None,
            display: None,
            scanout: Vec::new(),
            scanout_back: 0,
            flips: Vec::new(),
            pending_flip: None,
            pointer_flip: false,
            pointer_images: BTreeMap::new(),
            client_pointer: None,
            flipped: Vec::new(),
            composing_locked: false,
            shown_locked: false,
            shown_kept: false,
            direct: scanout::Direct::default(),
        }
    }

    /// The time between its display's frames: 8.33 ms at 120 Hz.
    pub fn refresh_period(&self) -> std::time::Duration {
        refresh_period(self.refresh_mhz)
    }

    /// With `paced`: its first vblank after `at` (CLOCK_MONOTONIC), when a frame composed then is
    /// shown.
    pub fn next_vblank(&self, at: std::time::Duration) -> std::time::Duration {
        let period = self.refresh_period().as_nanos().max(1);
        let since = at.saturating_sub(self.vblank_origin).as_nanos();
        let count = since / period + 1;
        self.vblank_origin
            + std::time::Duration::from_nanos(u64::try_from(count * period).unwrap_or(u64::MAX))
    }

    /// A rectangle of the global space (logical) in this screen's pixels, its edges rounded to
    /// the nearest pixel, so that what meets in the global space meets on the screen.
    fn place(&self, x: i32, y: i32, width: u32, height: u32) -> (i32, i32, i32, i32) {
        let at = |v: i64, origin: i32| {
            let offset = v - i64::from(origin);
            (offset * i64::from(self.scale_120) * 2 + 120).div_euclid(240)
        };
        let (x0, y0) = (at(i64::from(x), self.rect.x), at(i64::from(y), self.rect.y));
        let x1 = at(i64::from(x) + i64::from(width), self.rect.x);
        let y1 = at(i64::from(y) + i64::from(height), self.rect.y);
        let clamp = |v: i64| v.clamp(-i64::from(i32::MAX), i64::from(i32::MAX)) as i32;
        (clamp(x0), clamp(y0), clamp(x1), clamp(y1))
    }

    /// Where pixels of `extent` showing at a rectangle of the global space are on this screen
    /// (`place`). Pixels drawn for this screen's scale (`extent` the protocol's size for the
    /// rectangle here, docs/adr/0001) are drawn 1:1 from the placed corner: rounding both edges
    /// may make the rectangle a pixel wider or narrower than they are (961 at 1.25 is 1202
    /// pixels, its edges rounded 1201 apart), which would draw them stretched.
    fn place_pixels(
        &self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        extent: (u32, u32),
    ) -> (i32, i32, i32, i32) {
        let (x0, y0, x1, y1) = self.place(x, y, width, height);
        let own = physical(Size::new(width.max(1), height.max(1)), self.scale_120);
        if (own.width, own.height) == extent {
            let end = |start: i32, length: u32| start.saturating_add(length as i32);
            (x0, y0, end(x0, extent.0), end(y0, extent.1))
        } else {
            (x0, y0, x1, y1)
        }
    }

    /// A layer of the global space (logical) on this screen, in its pixels: its rectangle placed
    /// (`place_pixels`), its corners' radius and a shadow's geometry at the screen's scale; none
    /// if it is off the screen.
    fn layer<'a>(&self, layer: &oxyos_gpu::Layer<'a>) -> Option<oxyos_gpu::Layer<'a>> {
        let (x0, y0, x1, y1) =
            self.place_pixels(layer.x, layer.y, layer.width, layer.height, layer.extent);
        let on = x1 > x0
            && y1 > y0
            && x1 > 0
            && y1 > 0
            && x0 < self.width as i32
            && y0 < self.height as i32;
        if !on {
            return None;
        }
        let length = |length: u32| scaled(length, self.scale_120);
        // A shadow's rectangles are relative to its layer: placed as the global space's, then
        // made relative to the layer as placed, so a window's edge and its shadow's clip meet.
        let within = |rect: oxyos_gpu::Rect| {
            let (rx0, ry0, rx1, ry1) =
                self.place(layer.x + rect.x, layer.y + rect.y, rect.width, rect.height);
            oxyos_gpu::Rect {
                x: rx0 - x0,
                y: ry0 - y0,
                width: (rx1 - rx0).max(0) as u32,
                height: (ry1 - ry0).max(0) as u32,
            }
        };
        let paint = match layer.paint {
            oxyos_gpu::Paint::Shadow(profile, shadow) => oxyos_gpu::Paint::Shadow(
                profile,
                oxyos_gpu::Shadow {
                    shape: within(shadow.shape),
                    radius: length(shadow.radius),
                    clip: within(shadow.clip),
                    clip_radius: length(shadow.clip_radius),
                    scale: scaled(shadow.scale, self.scale_120)
                        .clamp(1, oxyos_gpu::MAX_SHADOW_SCALE),
                },
            ),
            paint => paint,
        };
        Some(oxyos_gpu::Layer {
            x: x0,
            y: y0,
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
            paint,
            radius: length(layer.radius),
            // A clip is of the global space too.
            clip: layer.clip.map(|clip| {
                let (cx0, cy0, cx1, cy1) = self.place(clip.x, clip.y, clip.width, clip.height);
                oxyos_gpu::Rect {
                    x: cx0,
                    y: cy0,
                    width: (cx1 - cx0).max(0) as u32,
                    height: (cy1 - cy0).max(0) as u32,
                }
            }),
            ..*layer
        })
    }

    /// A surface's damage (buffer pixels) where it shows at `rect` (the global space's): on this
    /// screen, every pixel whose filtered sample reads a damaged buffer pixel (`on_output`, at the
    /// size the surface is drawn here, ADR 0011), as a rectangle of the global space rounded
    /// outward, so that `covering` holds them.
    fn damage_on(&self, damage: &Rect, rect: Rect, extent: Size) -> oxyos_gpu::Rect {
        let (x0, y0, x1, y1) = self.place_pixels(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            (extent.width, extent.height),
        );
        let placed = Rect::new(x0, y0, (x1 - x0).max(0) as u32, (y1 - y0).max(0) as u32);
        let on = on_output(damage, placed, extent);
        let scale = i64::from(self.scale_120);
        let back = |pixels: i32, origin: i32, up: bool| {
            let at = i64::from(pixels) * 120;
            let v = i64::from(origin)
                + if up {
                    (at + scale - 1).div_euclid(scale)
                } else {
                    at.div_euclid(scale)
                };
            v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
        };
        let left = back(on.x, self.rect.x, false);
        let top = back(on.y, self.rect.y, false);
        let right = back(on.x.saturating_add(on.width as i32), self.rect.x, true);
        let bottom = back(on.y.saturating_add(on.height as i32), self.rect.y, true);
        oxyos_gpu::Rect {
            x: left,
            y: top,
            width: (right - left).max(0) as u32,
            height: (bottom - top).max(0) as u32,
        }
    }

    /// A rectangle of the global space (logical) in this screen's pixels, rounded outward, so
    /// that it covers every pixel showing any of it; none if it is off the screen.
    fn covering(&self, rect: &oxyos_gpu::Rect) -> Option<oxyos_gpu::Rect> {
        let on = rect.intersect(&oxyos_gpu::Rect {
            x: self.rect.x,
            y: self.rect.y,
            width: self.rect.width,
            height: self.rect.height,
        })?;
        let scale = i64::from(self.scale_120);
        let down = |v: i64| (v * scale).div_euclid(120);
        let up = |v: i64| (v * scale + 119).div_euclid(120);
        let (left, top) = (
            i64::from(on.x) - i64::from(self.rect.x),
            i64::from(on.y) - i64::from(self.rect.y),
        );
        let x0 = down(left).min(i64::from(self.width));
        let y0 = down(top).min(i64::from(self.height));
        let x1 = up(left + i64::from(on.width)).min(i64::from(self.width));
        let y1 = up(top + i64::from(on.height)).min(i64::from(self.height));
        (x1 > x0 && y1 > y0).then(|| oxyos_gpu::Rect {
            x: x0 as i32,
            y: y0 as i32,
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
        })
    }

    /// Where a point of the global space is on its display (pixels), if it is on this screen;
    /// the screen is centred on a larger display.
    pub fn on_display(&self, (x, y): (i32, i32)) -> Option<(i32, i32)> {
        if !self.rect.contains_point(x, y) {
            return None;
        }
        let (px, py, _, _) = self.place(x, y, 0, 0);
        let (dx, dy) = self.display_centring();
        Some((px + dx, py + dy))
    }

    /// Where the screen's pixels start on its display: centred when the display is larger.
    fn display_centring(&self) -> (i32, i32) {
        self.display.as_ref().map_or((0, 0), |display| {
            let (width, height) = display.size();
            (
                (width.saturating_sub(self.width) / 2) as i32,
                (height.saturating_sub(self.height) / 2) as i32,
            )
        })
    }

    /// The pixel at a point of the screen's own pixels.
    pub fn pixel(&self, x: u32, y: u32) -> u32 {
        self.framebuffer[(y * self.width + x) as usize]
    }
}

/// The time between a display's frames at `refresh_mhz` (16.67 ms at 60 Hz, 6.94 at 144).
pub fn refresh_period(refresh_mhz: u32) -> std::time::Duration {
    std::time::Duration::from_nanos(1_000_000_000_000 / u64::from(refresh_mhz.max(1)))
}

/// A logical length at a scale (in 120ths), in pixels, rounded to the nearest.
pub fn scaled(length: u32, scale_120: u32) -> u32 {
    ((u64::from(length) * u64::from(scale_120) * 2 + 120) / 240) as u32
}

pub struct Compositor {
    /// Each of the scene's outputs, in the scene's order (the first is the primary).
    pub screens: Vec<Screen>,
    pub scene: Scene,
    pub peers: HashMap<ClientId, Peer>,
    /// The development compositor's shell socket: a same-user peer connecting there is granted
    /// `shell` (docs/adr/0009-session-shell.mdx) and `lock` (it is the lock screen too, ADR 0010), as a shipping session will grant it from its
    /// launch records (#35).
    pub shell_listener: Option<Listener>,
    /// The screen reader's socket (docs/adr/0014-screen-reader.mdx): a same-user peer connecting
    /// there is granted `accessibility` and `screen_reader`. The session listens there only for
    /// the reader it starts.
    pub reader_listener: Option<Listener>,
    /// The user pressed the screen reader's switch (Super+F5) since the program running the
    /// compositor last took it (`take_reader_toggle`).
    reader_toggled: bool,
    pub next_client: ClientId,
    /// Peers of the shell socket are numbered down from here, so the clients the development
    /// compositor launches keep their numbers whatever order they connect in (the two above are
    /// the scene-only clients it plays: an assistive technology and an input method).
    next_shell_client: ClientId,
    pub own_uid: u32,
    /// The windows' shadows (window.rs).
    shadows: window::Shadows,
    pub frames: usize,
    /// Input events routed to clients, kept while `log_input` is set: (client, surface, kind).
    pub input_log: Vec<(ClientId, u32, &'static str)>,
    pub log_input: bool,
    /// Clients of the scene with no connection, played by the program running the compositor
    /// (a development compositor's assistive technology or input method): their events are kept
    /// in `local_events` for it, not dropped.
    pub local_clients: std::collections::BTreeSet<ClientId>,
    pub local_events: Vec<(ClientId, Event)>,
    /// Pipes of pastes a local client must answer, in the order of its ClipboardSend events.
    pub local_pipes: VecDeque<(ClientId, OwnedFd)>,
    /// The pointer images clients set (ADR 0029), by surface: the scene's number for it, the
    /// image as read from the client's buffer, and the scale it was drawn for.
    pub client_pointers: HashMap<SurfaceKey, ClientPointer>,
    /// With `--gpu`: every frame is also composed on this Vulkan device and must match the CPU
    /// reference byte for byte (docs/adr/0004-gpu-composition.mdx).
    pub gpu: Option<Gpu>,
    /// With `--gpu-only`: frames are composed on the Vulkan device alone, with no CPU reference;
    /// the frame the director checks is the one composed, read back after it is shown.
    pub gpu_only: bool,
    /// Frames shown by putting a full-screen window's buffer on its display's primary plane
    /// (docs/adr/0031-gpu-buffers.mdx, decision 8).
    pub direct_frames: u64,
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
    /// session as long as nothing of its own is due, waking for its devices, clients, display and
    /// `wakers`, the pointer's next step and a connection's deadline (`serve`).
    pub idle_wait: Duration,
    /// The hardware keys and the Control Center (docs/adr/0025-hardware-keys-and-control-center.mdx):
    /// what acts on the system's volumes, backlights and Wi-Fi and finds its battery and network,
    /// given by the program running the compositor. Without them the keys do nothing and the
    /// shell is told nothing of the system.
    pub controls: Option<controls::Controls>,
    /// Whether a turn whose work (not its wait) took longer than the fastest display's refresh
    /// (it delays that display's frame and the input after it) is logged, with where the time
    /// went, at most once a second: a session's, whose frames are otherwise not measured.
    pub slow_turns: bool,
    slow_turn_logged: Option<Instant>,
    /// With `slow_turns`: when the GPU's composing times were last summed up (`serve`).
    gpu_times_logged: Option<Instant>,
    /// Descriptors that only wake a turn, readable when the program running the compositor has
    /// something of its own to do (a child of its exited); it reads them itself.
    pub wakers: Vec<std::os::fd::OwnedFd>,
    /// With `--dump-frames`: the latest composed frame, as `oxyos_gpu::frame_file` writes it.
    pub keep_frame: bool,
    pub last_frame: Vec<u8>,
    /// With `--input`: the seat's devices through libinput, and where the global space's origin
    /// is in their coordinates (the development compositor's output, centred on a larger
    /// display, starts there).
    pub input: Option<oxyos_input::Input>,
    pub display_offset: (i32, i32),
    /// With a display: the card's connectors changed (`hotplug`, ADR 0018) since the program
    /// running the compositor last took it (`take_hotplug`), which then reads them again.
    pub hotplug: Option<oxyos_kms::Hotplug>,
    hotplugged: bool,
    /// Where the pointer is in the global space (logical pixels), once it has moved.
    pointer: Option<(i32, i32)>,
    /// When the frame being composed started (CLOCK_MONOTONIC), and for every frame shown, how
    /// long from then to handing it to the display, in milliseconds.
    pub composing_since: std::time::Duration,
    pub compositions: Vec<f64>,
    /// For every frame composed: how long reading the clients' buffers took, in ms.
    pub reads: Vec<f64>,
    /// For every frame composed: the fraction of the output its damage covers (1 when whole).
    pub damaged: Vec<f64>,
    /// The frames read from the clients' buffers, by surface: the frame (sequence and buffer)
    /// and its pixels. A frame shown is never written by its client until it is released, so a
    /// frame composed again (a window moved, a transition, the pointer over it) is not read
    /// again: only a new one is (`read_frame`).
    frames_read: HashMap<SurfaceKey, FrameRead>,
    /// Clients' GPU buffers and timelines (docs/adr/0031-gpu-buffers.mdx).
    gpu_buffers: gpu_buffers::GpuBuffers,
    /// The bytes of pixels read so far from each client's buffers (`read_frame`): none while it
    /// shows nothing new.
    pub bytes_read: HashMap<ClientId, u64>,
    /// For every turn that composed a frame: when it started (CLOCK_MONOTONIC), how long
    /// composing every screen's frame took, from gathering the scene to handing it over, and the
    /// part of that reading the clients' buffers, in ms.
    pub frame_times: Vec<(std::time::Duration, f64, f64)>,
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
    /// A login screen (docs/adr/0020-oxy-account-login.mdx): the scene is locked from the start
    /// and an accepted password signs a person in rather than unlocking. The splash is shown and
    /// [`Compositor::signed_in`] says so; the login program then ends the compositor and starts
    /// the person's session. Nothing behind the lock is ever shown.
    pub login: bool,
    /// At a login screen, a password was accepted.
    pub signed_in: bool,
    /// The lock surface whose password is being checked.
    checking: Option<SurfaceKey>,
    /// The machine is about to sleep: it may once the lock is on screen.
    sleeping: bool,
    /// What logind was last told of the lock.
    hinted_locked: bool,
    /// logind refused the session's idle hint (it takes one only from a graphical session
    /// type): it is not told again.
    idle_hint_refused: bool,
    /// Surfaces arriving and going, animated (docs/adr/0011-transitions.mdx), and the last frame
    /// composed of each surface shown, which one going is drawn from.
    pub transitions: transitions::Transitions,
    shown: HashMap<SurfaceKey, transitions::Snapshot>,
    /// The windows each shell surface showed live in the frame composed last (ADR 0022).
    shown_views: HashMap<SurfaceKey, Vec<ShownView>>,
    /// Frames composed while a transition ran, as the development compositor reports them.
    pub transition_frames: u64,
    /// The time transitions and the idle lock run on: `CLOCK_MONOTONIC`, or a test's.
    pub clock: Option<Duration>,
    /// The idle lock (idle.rs, ADR 0010): the session locks after the user's `lock_after` with no
    /// input. Never, until something sets it (`set_lock_after`).
    pub idle: idle::Idle,
    /// The session's splash (splash.rs), over everything on every screen from `show_splash` to
    /// `end_splash`: one for each.
    splash: Vec<transitions::Snapshot>,
    /// With `--gpu`: the screen whose frame the Vulkan compositor's own output holds, which the
    /// next frame composed there for the same screen is relative to.
    gpu_holds: Option<OutputId>,
    /// Called once, when the first frame composed has flipped on a display: the session tells
    /// its service manager it is on screen, and the boot logo is let go
    /// (docs/adr/0017-boot-logo.mdx).
    pub on_first_flip: Option<Box<dyn FnOnce()>>,
}

/// The session's authentication path (ADR 0010): it checks a lock surface's password off the
/// compositor's thread, and only its `Accepted` unlocks. The session's is PAM; a login screen's
/// the login program's (ADR 0020); the development compositor's a fixture, never a session's.
pub trait Authentication {
    /// Begins checking `secret`, for `account` when the lock surface named one (a login
    /// screen's), else for the session's own user: `None` while it is checked, or an answer at
    /// once (busy, throttled, an account it may not check) with the seconds until another may be
    /// tried.
    fn attempt(
        &mut self,
        account: Option<&str>,
        secret: oxyos_compositor_core::scene::Secret,
    ) -> Option<(AuthOutcome, u32)>;
    /// Readable when the password being checked has its answer.
    fn as_fd(&self) -> BorrowedFd<'_>;
    /// The answer for the password being checked, once there is one.
    fn answer(&mut self) -> Option<(AuthOutcome, u32)>;
}

/// What a turn gathered for the screens it composes (`Compositor::frame`): the frame list and
/// how its windows are drawn, what each surface shows completed, the layers in the global space,
/// where surfaces in transition are drawn, and for the screen, how many frames it presents,
/// whether the session is locked and whether the pointer moved on it.
struct ScreenFrame<'a> {
    list: &'a [RenderItem],
    looks: &'a Looks,
    displayed: &'a HashMap<SurfaceKey, oxyos_compositor_core::Frame>,
    layers: &'a [oxyos_gpu::Layer<'a>],
    transition_drawn: &'a [oxyos_gpu::Rect],
    started: usize,
    locked: bool,
    pointer_moved: bool,
}

/// One layer's content as a frame composes it: a surface's pixels (`key`), a cover, a surface
/// going, or a window's shadow; `rect` is where the surface is, `placed` where its transition
/// draws it, with its corners' radius.
struct Content {
    key: Option<SurfaceKey>,
    rect: Rect,
    placed: Rect,
    extent: Size,
    format: Option<oxyos_protocol::Format>,
    pixels: Rc<Pixels>,
    source: Option<oxyos_gpu::Source>,
    changed: Option<(u64, Vec<oxyos_gpu::Rect>)>,
    opacity: u8,
    radius: u32,
    shadow: Option<(std::rc::Rc<oxyos_gpu::ShadowProfile>, oxyos_gpu::Shadow)>,
    /// A window's look, kept with its last frame for its transition going.
    look: Option<WindowLook>,
    /// Where on the output it may draw, if not everywhere.
    clip: Option<oxyos_gpu::Rect>,
    /// Its pixels are those of the content at this index (the same window's frame, read once).
    shares: Option<usize>,
}

impl Content {
    fn cover(rect: Rect) -> Self {
        Self {
            key: None,
            rect,
            placed: rect,
            extent: Size::new(1, 1),
            format: None,
            pixels: Rc::default(),
            source: None,
            changed: None,
            opacity: 255,
            radius: 0,
            shadow: None,
            look: None,
            clip: None,
            shares: None,
        }
    }

    /// The shadow of the window at `rect`, placed at `placed`, drawn with `look`, at `opacity`;
    /// and its corners' radius.
    fn shadow(
        shadows: &window::Shadows,
        look: WindowLook,
        corner: u32,
        rect: Rect,
        placed: Rect,
        opacity: u8,
    ) -> (Self, u32) {
        let (profile, source) = shadows.of(look);
        let drawn = window::drawn(look, corner, rect, placed, profile);
        let shadow = Self {
            key: None,
            rect: drawn.shadow_rect,
            placed: drawn.shadow_rect,
            extent: Size::new(profile.entries(), 1),
            format: None,
            pixels: Rc::default(),
            source: Some(*source),
            changed: None,
            opacity,
            radius: 0,
            shadow: Some((profile.clone(), drawn.shadow)),
            look: None,
            clip: None,
            shares: None,
        };
        (shadow, drawn.radius)
    }
}

/// A window a shell surface showed live in the frame composed last, as the scene placed it
/// (docs/adr/0022-window-views.mdx): what goes with the surface if it goes.
#[derive(Clone, Copy, Debug)]
struct ShownView {
    window: SurfaceKey,
    sequence: u64,
    buffer: u64,
    rect: Rect,
    clip: Rect,
    radius: u32,
    opacity: u8,
}

/// A frame's pixels as the compositor read them, and the client's GPU buffer they are in when it
/// is one (docs/adr/0031-gpu-buffers.mdx): a composition on the GPU samples that in place, one on
/// the CPU draws `bytes` (empty when only the GPU composes).
#[derive(Debug, Default)]
pub struct Pixels {
    pub bytes: Vec<u8>,
    pub gpu: Option<GpuFrame>,
}

impl std::ops::Deref for Pixels {
    type Target = Vec<u8>;
    fn deref(&self) -> &Vec<u8> {
        &self.bytes
    }
}

impl From<Vec<u8>> for Pixels {
    fn from(bytes: Vec<u8>) -> Self {
        Self { bytes, gpu: None }
    }
}

/// Where a frame's pixels are read from: clients' shared memory, their GPU buffers, and whether
/// the CPU needs their pixels (it composes, or checks the GPU's composition).
type Readers<'a> = (
    &'a HashMap<ClientId, Peer>,
    &'a gpu_buffers::GpuBuffers,
    bool,
);

/// The pixels of a surface's frame in `buffer`, read from its client's shared memory or GPU
/// buffer.
fn read_pixels(
    (peers, gpu, needed): Readers<'_>,
    key: SurfaceKey,
    buffer: u64,
) -> Option<(Size, oxyos_protocol::Format, Pixels)> {
    if gpu.has(key.client, buffer) {
        let (extent, format, bytes, gpu) = gpu.read(key.client, buffer, needed)?;
        return Some((extent, format, Pixels { bytes, gpu }));
    }
    let shared = peers.get(&key.client)?.buffers.get(&buffer)?;
    let layout = shared.layout();
    let row = layout.extent.width as usize * 4;
    let mut pixels = vec![0; row * layout.extent.height as usize];
    for (y, out) in pixels.chunks_exact_mut(row).enumerate() {
        shared.read_row(y as u32, out).ok()?;
    }
    Some((layout.extent, layout.format, pixels.into()))
}

/// What a surface read last showed: its frame (sequence and buffer), its size and format, and its
/// pixels (`Compositor::frames_read`).
type FrameRead = ((u64, u64), Size, oxyos_protocol::Format, Rc<Pixels>);

/// A surface's frame (`sequence`, in `buffer`) as `read_pixels` reads it, read only if it is not
/// the one read last for the surface: a frame shown is never written by its client until it is
/// released, so the same frame composed again (a window moved, a transition, the pointer over
/// it) has the same pixels.
fn read_frame(
    (frames_read, bytes_read): (
        &mut HashMap<SurfaceKey, FrameRead>,
        &mut HashMap<ClientId, u64>,
    ),
    readers: Readers<'_>,
    key: SurfaceKey,
    sequence: u64,
    buffer: u64,
) -> Option<(Size, oxyos_protocol::Format, Rc<Pixels>)> {
    if let Some((frame, extent, format, pixels)) = frames_read.get(&key) {
        if *frame == (sequence, buffer) {
            return Some((*extent, *format, pixels.clone()));
        }
    }
    let (extent, format, pixels) = read_pixels(readers, key, buffer)?;
    *bytes_read.entry(key.client).or_default() += pixels.len() as u64;
    let pixels = Rc::new(pixels);
    frames_read.insert(key, ((sequence, buffer), extent, format, pixels.clone()));
    Some((extent, format, pixels))
}

/// `inner`, a part of a surface at `rect`, where it is drawn when the surface is drawn at
/// `placed` (its transition scaling and moving it): scaled and moved with it.
pub fn placed_within(rect: Rect, placed: Rect, inner: Rect) -> Rect {
    if rect == placed {
        return inner;
    }
    let along = |at: i64, from: i32, to: i32, length: u32, drawn: u32| {
        i64::from(to) + (at - i64::from(from)) * i64::from(drawn) / i64::from(length.max(1))
    };
    let x0 = along(
        i64::from(inner.x),
        rect.x,
        placed.x,
        rect.width,
        placed.width,
    );
    let y0 = along(
        i64::from(inner.y),
        rect.y,
        placed.y,
        rect.height,
        placed.height,
    );
    let x1 = along(
        i64::from(inner.x) + i64::from(inner.width),
        rect.x,
        placed.x,
        rect.width,
        placed.width,
    );
    let y1 = along(
        i64::from(inner.y) + i64::from(inner.height),
        rect.y,
        placed.y,
        rect.height,
        placed.height,
    );
    Rect::new(
        x0 as i32,
        y0 as i32,
        (x1 - x0).max(0) as u32,
        (y1 - y0).max(0) as u32,
    )
}

pub struct Gpu {
    pub compositor: oxyos_gpu::VulkanCompositor,
    pub frames: u64,
    pub mismatches: u64,
    /// The device's time for each frame composed for the display (GPU timestamps), in ms.
    pub device_ms: Vec<f64>,
    /// With `--gpu-only`: how long each readback of the shown frame took, in ms.
    pub readback_ms: Vec<f64>,
    /// For each frame composed for the display: the host's copy into the upload buffer, and its
    /// wait for the device (`VulkanCompositor::last_host_ms`), in ms.
    pub host_ms: Vec<(f64, f64)>,
    /// The bytes each frame composed for the display uploaded.
    pub uploaded: Vec<u64>,
    /// The device's time before and after composing each frame for the display, in ms.
    pub phases_ms: Vec<(f64, f64)>,
}

impl Compositor {
    /// A compositor for `scene`, a screen for each of its outputs (at its rectangle and scale,
    /// with no display), serving connections of the user `own_uid`.
    pub fn new(scene: Scene, own_uid: u32) -> Self {
        Self {
            screens: scene
                .outputs()
                .iter()
                .map(|output| Screen::new(output, None))
                .collect(),
            scene,
            peers: HashMap::new(),
            shell_listener: None,
            reader_listener: None,
            reader_toggled: false,
            next_client: 0,
            next_shell_client: ClientId::MAX - 2,
            own_uid,
            // The shadows' profiles are the first two versions numbered.
            shadows: window::Shadows::new(0),
            frames: 0,
            input_log: Vec::new(),
            log_input: false,
            local_clients: Default::default(),
            local_events: Vec::new(),
            local_pipes: VecDeque::new(),
            client_pointers: HashMap::new(),
            gpu: None,
            gpu_only: false,
            direct_frames: 0,
            reads_back: true,
            records: true,
            ending: false,
            keep_frame: false,
            last_frame: Vec::new(),
            input: None,
            display_offset: (0, 0),
            hotplug: None,
            hotplugged: false,
            pointer: None,
            idle_wait: Duration::from_millis(5),
            controls: None,
            wakers: Vec::new(),
            slow_turns: false,
            slow_turn_logged: None,
            gpu_times_logged: None,
            composing_since: std::time::Duration::ZERO,
            compositions: Vec::new(),
            reads: Vec::new(),
            damaged: Vec::new(),
            frames_read: HashMap::new(),
            gpu_buffers: Default::default(),
            bytes_read: HashMap::new(),
            frame_times: Vec::new(),
            versions: HashMap::new(),
            next_version: 2,
            shown_versions: HashMap::new(),
            terminal: None,
            seat: None,
            card_device: None,
            away: false,
            authentication: None,
            login: false,
            signed_in: false,
            checking: None,
            sleeping: false,
            hinted_locked: false,
            idle_hint_refused: false,
            transitions: transitions::Transitions::default(),
            shown: HashMap::new(),
            shown_views: HashMap::new(),
            transition_frames: 0,
            clock: None,
            idle: idle::Idle::default(),
            splash: Vec::new(),
            gpu_holds: None,
            on_first_flip: None,
        }
    }

    /// Development session authority: same-user peers get ordinary application grants and
    /// nothing else. A shipping session derives grants from its launch records (#35).
    pub fn grants_for(&self, credentials: Credentials) -> Option<Grants> {
        (credentials.uid == self.own_uid).then(Grants::default)
    }

    /// Whether the user pressed the screen reader's switch since the last call (ADR 0014): the
    /// program running the compositor starts its reader, or stops it.
    pub fn take_reader_toggle(&mut self) -> bool {
        std::mem::take(&mut self.reader_toggled)
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
                pid: credentials.pid,
                named: false,
            },
        );
        Ok(())
    }

    /// Applies launch records (docs/adr/0013-launching-applications.mdx): each client connected
    /// since the last call whose process `record_of` names (one the session started) is that
    /// application, its windows the registry's, whatever they claim, and has the grants its
    /// record gives (ADR 0019: `settings`, for the image's policy's Settings). A client's record is
    /// looked for once, when it has said Hello.
    /// The processes (by pid, their connections' credentials) showing a window: a toplevel of
    /// theirs mapped. The session tells the install service an installed version showed one
    /// (docs/adr/0030-installing-applications.mdx, decision 10).
    pub fn pids_showing_windows(&self) -> std::collections::BTreeSet<u32> {
        self.scene
            .clients_showing_windows()
            .into_iter()
            .filter_map(|client| u32::try_from(self.peers.get(&client)?.pid).ok())
            .collect()
    }

    pub fn name_clients(&mut self, record_of: &dyn Fn(u32) -> Option<LaunchRecord>) {
        for (client, peer) in self.peers.iter_mut() {
            if peer.named || peer.protocol.version().is_none() {
                continue;
            }
            peer.named = true;
            let Ok(pid) = u32::try_from(peer.pid) else {
                continue;
            };
            let Some(record) = record_of(pid) else {
                continue;
            };
            self.scene.set_application(*client, &record.application);
            if let Err(code) = self.scene.grant(*client, record.grants) {
                eprintln!(
                    "[compositor] {} is not given {:?}: {code:?}",
                    record.application, record.grants
                );
            }
        }
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

    /// A request the scene refused: said in the log by its name, never its fields (a password may
    /// be one), and the client disconnected.
    fn refuse(&mut self, client: ClientId, opcode: u8, code: ErrorCode) {
        let name = Request::SCHEMA
            .iter()
            .find(|(known, _, _)| *known == opcode)
            .map_or("an unknown request", |(_, name, _)| *name);
        eprintln!("[compositor] client {client:?}: {name} refused: {code:?}");
        self.fail(client, code);
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
        self.disconnect_gpu(client);
        self.client_pointers.retain(|key, _| key.client != client);
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
            self.retire_gpu_buffer(client, buffer);
        }
    }

    pub fn receive(&mut self, client: ClientId) {
        // What notifications' rate is measured on (ADR 0026).
        let now = self.now();
        self.scene.set_clock(now);
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
                // What the compositor takes of clients' GPU buffers (ADR 0031, decision 2).
                for event in self.gpu_buffers.announcement() {
                    peer.outbox.push_back((event, None));
                }
                let grants = peer.grants;
                if self.scene.connect(client, grants).is_err() {
                    self.fail(client, ErrorCode::Unauthorized);
                }
                return;
            }
            Ok(None) => {}
            Err(code) => return self.fail(client, code),
        }
        // A request of a newer version than the connection's is not one it has (ADR 0033).
        if peer
            .protocol
            .version()
            .is_some_and(|version| oxyos_protocol::request_since(&request) > version)
        {
            return self.fail(client, ErrorCode::InvalidMessage);
        }
        let opcode = request.opcode();
        let fd = match (request, fd) {
            (Request::ClipboardReceive { .. }, Some(pipe)) => {
                if let Err(code) = self.scene.handle(client, request) {
                    return self.refuse(client, opcode, code);
                }
                // An allowed paste's pipe goes to the clipboard's owner with its ClipboardSend; a
                // refused one is closed here, and the receiver reads its end at once.
                if let Some(owner) = self.scene.take_transfer() {
                    self.pass_pipe(owner, pipe);
                }
                return;
            }
            (_, fd) => fd,
        };
        // Clients' GPU buffers, their timelines and points, and frames held for an acquire
        // point (ADR 0031); a frame ready now goes on to the scene.
        let fd = if matches!(
            request,
            Request::AttachDmabufPlane { .. }
                | Request::AttachDmabuf { .. }
                | Request::ImportTimeline { .. }
                | Request::DestroyTimeline { .. }
                | Request::SetSync { .. }
                | Request::Submit { .. }
                | Request::DestroyBuffer { .. }
                | Request::Destroy { .. }
        ) {
            match self.receive_gpu(client, request, fd) {
                Some(Ok(())) => return self.retire(),
                Some(Err(code)) => return self.fail(client, code),
                None => None,
            }
        } else {
            fd
        };
        if let (Some(layout), Some(fd), Request::AttachBuffer { buffer, .. }) =
            (request.buffer_layout(), fd, request)
        {
            let Ok(shared) = SharedBuffer::import(fd, layout) else {
                return self.fail(client, ErrorCode::InvalidBuffer);
            };
            if let Err(code) = self.scene.handle(client, request) {
                return self.refuse(client, opcode, code);
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
            return self.refuse(client, opcode, code);
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
            Request::Destroy { surface } => {
                let destroyed = SurfaceKey {
                    client,
                    id: surface,
                };
                self.versions.retain(|(key, _), _| *key != destroyed);
                self.client_pointers.remove(&destroyed);
            }
            Request::SetPointerImage {
                surface,
                buffer,
                hotspot_x,
                hotspot_y,
                scale_120,
            } => {
                let key = SurfaceKey {
                    client,
                    id: surface,
                };
                let image = self
                    .peers
                    .get(&client)
                    .and_then(|peer| peer.buffers.get(&buffer))
                    .and_then(|shared| read_pointer_image(shared, (hotspot_x, hotspot_y)));
                match (image, self.scene.pointer_image_of(key)) {
                    (Some(image), Some(id)) => {
                        self.client_pointers.insert(
                            key,
                            ClientPointer {
                                id,
                                image,
                                scale_120,
                            },
                        );
                    }
                    _ => {
                        self.client_pointers.remove(&key);
                    }
                }
            }
            _ => {}
        }
        if goodbye {
            self.peers.remove(&client);
        }
        self.retire();
    }

    /// A frame held for its acquire point (docs/adr/0031-gpu-buffers.mdx), now ready: the scene's,
    /// as a `Submit` received now would be.
    pub(crate) fn submit_to_scene(&mut self, client: ClientId, request: Request) {
        if let Err(code) = self.scene.handle(client, request) {
            return self.fail(client, code);
        }
        if let Request::Submit {
            surface, sequence, ..
        } = request
        {
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
        self.retire();
    }

    /// A paste's pipe, to the clipboard's owner that answers it: a peer's goes with its
    /// ClipboardSend, a local client's is kept for the program playing it.
    fn pass_pipe(&mut self, owner: ClientId, pipe: OwnedFd) {
        if let Some(peer) = self.peers.get_mut(&owner) {
            peer.pipes.push_back(pipe);
        } else if self.local_clients.contains(&owner) {
            self.local_pipes.push_back((owner, pipe));
        }
    }

    /// The session reads `mime` of what the clipboard holds (Scene::paste_for_session): the read
    /// end of the pipe its owner writes it into, or None when the clipboard has no such type.
    pub fn read_clipboard(&mut self, mime: TextChunk) -> std::io::Result<Option<OwnedFd>> {
        if !self.scene.paste_for_session(mime) {
            return Ok(None);
        }
        let (read, write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?;
        if let Some(owner) = self.scene.take_transfer() {
            self.pass_pipe(owner, write);
        }
        Ok(Some(read))
    }

    /// Routes scene events into bounded outboxes and sends what each socket accepts now.
    pub fn flush(&mut self) {
        for (client, event) in self.scene.drain_events() {
            // A GPU buffer's release waits for the GPU and any transition drawing it (ADR 0031).
            if self.hold_gpu_event(client, &event) {
                continue;
            }
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
            // Each client's events as its version defines them (ADR 0033).
            let version = peer.protocol.version().unwrap_or(oxyos_protocol::VERSION);
            while let Some((event, pipe)) = peer.outbox.front() {
                match peer.connection.try_send_event_for(
                    event,
                    pipe.as_ref().map(|pipe| pipe.as_fd()),
                    version,
                ) {
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

    /// Adds `output` to the scene and a screen for it, composing `pixels` (its display's mode;
    /// none: its rectangle at its scale). Its display, scanout buffers and pointer images are
    /// the caller's to give it (`screen_mut`).
    pub fn add_output(&mut self, output: Output, pixels: Option<(u32, u32)>) -> Result<()> {
        self.scene
            .add_output(output)
            .map_err(|code| format!("output {}: {code:?}", output.id))?;
        let mut screen = Screen::new(&output, pixels);
        screen.recompose = true;
        self.screens.push(screen);
        self.confine_pointer();
        Ok(())
    }

    /// Removes an output and its screen (its monitor was unplugged): its windows go to the
    /// primary, its shell's surfaces are dismissed (the scene's `remove_output`), its frame on
    /// its way completes unshown, its display is turned off and its scanout buffers freed.
    pub fn remove_output(&mut self, id: OutputId) {
        let Some(index) = self.screens.iter().position(|screen| screen.id == id) else {
            return;
        };
        self.detach_display(id);
        self.screens.remove(index);
        if self.gpu_holds == Some(id) {
            self.gpu_holds = None;
        }
        self.scene.remove_output(id);
        self.retire();
        // What moved to the screens left shows there.
        for screen in &mut self.screens {
            screen.recompose = true;
        }
        self.confine_pointer();
    }

    /// Turns a screen's display off and lets go of it and its scanout buffers (its monitor was
    /// unplugged): its frame on its way completes unshown, and it composes on, undisplayed.
    pub fn detach_display(&mut self, id: OutputId) {
        let Some(screen) = self.screens.iter_mut().find(|screen| screen.id == id) else {
            return;
        };
        if screen.pending_flip.take().is_some() && !std::mem::take(&mut screen.pointer_flip) {
            self.scene.end_frame_on(id, 0, Clock::Synthetic);
        }
        if let Some(mut display) = screen.display.take() {
            if let Err(error) = display.turn_off() {
                eprintln!("[kms] turning output {id} off: {error}");
            }
        }
        let targets: Vec<usize> = screen.scanout.drain(..).map(|(index, _)| index).collect();
        screen.scanout_back = 0;
        if let Some(gpu) = &mut self.gpu {
            gpu.compositor.remove_scanout_targets(&targets);
        }
        self.retire();
    }

    /// Moves an output, and what is on it, to `origin` in the global space (the arrangement
    /// closing the gap one removed left).
    pub fn move_output(&mut self, id: OutputId, origin: (i32, i32)) -> Result<()> {
        self.scene
            .set_output_origin(id, origin)
            .map_err(|code| format!("output {id}: {code:?}"))?;
        if let Some(screen) = self.screen_mut(id) {
            screen.rect.x = origin.0;
            screen.rect.y = origin.1;
            screen.recompose = true;
        }
        self.confine_pointer();
        Ok(())
    }

    /// Gives an output another rectangle and scale, and its screen the pixels it composes now
    /// (`pixels`, its display's mode; none: its rectangle at its scale): a display at another
    /// mode or scale, or moved (docs/adr/0018-several-displays.mdx). What is on it keeps its place
    /// on it (the scene's `set_output_geometry`); its next frame is composed whole.
    pub fn set_output_geometry(
        &mut self,
        id: OutputId,
        rect: Rect,
        scale_120: u32,
        pixels: Option<(u32, u32)>,
    ) -> Result<()> {
        self.scene
            .set_output_geometry(id, rect, scale_120)
            .map_err(|code| format!("output {id}: {code:?}"))?;
        if let Some(screen) = self.screen_mut(id) {
            let (width, height) = pixels.unwrap_or((
                scaled(rect.width, scale_120),
                scaled(rect.height, scale_120),
            ));
            screen.rect = rect;
            screen.scale_120 = scale_120;
            if (width, height) != (screen.width, screen.height) {
                screen.width = width;
                screen.height = height;
                screen.framebuffer = vec![DESKTOP; width as usize * height as usize];
            }
            screen.recompose = true;
        }
        self.confine_pointer();
        Ok(())
    }

    /// Gives an output another refresh, in mHz: its display driven at another mode of the same
    /// size (docs/adr/0018-several-displays.mdx). Its frames are paced by it, and every surface on
    /// it is told (`Refresh`), as the shells are the output.
    pub fn set_output_refresh(&mut self, id: OutputId, refresh_mhz: u32) -> Result<()> {
        self.scene
            .set_output_refresh(id, refresh_mhz)
            .map_err(|code| format!("output {id}: {code:?}"))?;
        if let Some(screen) = self.screen_mut(id) {
            screen.refresh_mhz = refresh_mhz;
        }
        Ok(())
    }

    /// The time between frames of the fastest display: what a frame's work must fit in for none
    /// to miss its refresh. None without a screen.
    pub fn fastest_refresh(&self) -> Option<Duration> {
        self.screens.iter().map(Screen::refresh_period).min()
    }

    /// Makes an output the primary, the first of the arrangement (it has the dock and takes the
    /// windows of an output removed), its screen first as the scene's output is.
    pub fn set_primary_output(&mut self, id: OutputId) -> Result<()> {
        self.scene
            .set_primary_output(id)
            .map_err(|code| format!("output {id}: {code:?}"))?;
        if let Some(index) = self.screens.iter().position(|screen| screen.id == id) {
            let screen = self.screens.remove(index);
            self.screens.insert(0, screen);
        }
        Ok(())
    }

    pub fn screen(&self, id: OutputId) -> Option<&Screen> {
        self.screens.iter().find(|screen| screen.id == id)
    }

    pub fn screen_mut(&mut self, id: OutputId) -> Option<&mut Screen> {
        self.screens.iter_mut().find(|screen| screen.id == id)
    }

    /// Whether the card's connectors may have changed since the last call (`hotplug`, or the
    /// display coming back from another session, which may have changed them meanwhile): the
    /// program running the compositor reads them again and adds or removes outputs.
    pub fn take_hotplug(&mut self) -> bool {
        std::mem::take(&mut self.hotplugged)
    }

    /// The pointer moves within the outputs, crossing where they meet (ADR 0018): told to the
    /// devices whenever the outputs change, and by the program running the compositor once it
    /// gives it its devices (`input`).
    pub fn confine_pointer(&mut self) {
        let Some(input) = self.input.as_mut() else {
            return;
        };
        let (dx, dy) = self.display_offset;
        input.set_area(
            self.screens
                .iter()
                .map(|screen| {
                    (
                        f64::from(screen.rect.x + dx),
                        f64::from(screen.rect.y + dy),
                        f64::from(screen.rect.width),
                        f64::from(screen.rect.height),
                    )
                })
                .collect(),
        );
    }

    /// Composes the frame of every screen that has none on its way to its display and shows
    /// something new (ADR 0018), on the CPU or the GPU, and hands it to its display; a screen
    /// without one completes its frame at once. Each screen's frame presents the surfaces on it
    /// (`Scene::begin_frame_on`) and completes at its own flip.
    pub fn frame(&mut self) {
        self.follow_direct();
        // The pointer is on the cursor plane of the display it is over, placed by every commit,
        // in the shape the surface under it wants; the other displays hide it.
        let shape = self.scene.pointer_shape();
        let client_pointer = self.scene.pointer_image().and_then(|(key, id)| {
            self.client_pointers
                .get(&key)
                .filter(|pointer| pointer.id == id)
                .map(|pointer| (key, pointer))
        });
        let clock = monotonic();
        let mut pointer_moved = vec![false; self.screens.len()];
        for (moved, screen) in pointer_moved.iter_mut().zip(&mut self.screens) {
            if screen.pending_flip.is_some() || self.away {
                continue;
            }
            let custom =
                client_pointer.and_then(|(key, pointer)| screen.show_client_pointer(key, pointer));
            let image = custom.unwrap_or_else(|| {
                screen
                    .pointer_images
                    .get(&shape)
                    .map_or(0, |steps| pointer_step(steps, clock))
            });
            let at = self.pointer.and_then(|at| screen.on_display(at));
            if let Some(display) = &mut screen.display {
                *moved = display.set_pointer(at, image);
            }
        }
        // The screens that compose now: those with no frame on its way to their display.
        let ready: Vec<usize> = (0..self.screens.len())
            .filter(|&index| self.screens[index].pending_flip.is_none())
            .collect();
        if ready.is_empty() {
            return;
        }
        let started: Vec<usize> = ready
            .iter()
            .map(|&index| self.scene.begin_frame_on(self.screens[index].id))
            .collect();
        let list = self.scene.frame_list();
        // Toplevels and dialogs are drawn with corners and a shadow (ADR 0016).
        let looks: Looks = list
            .iter()
            .filter_map(|item| match *item {
                RenderItem::Surface { key, .. } => Some((key, self.scene.window_look(key)?)),
                RenderItem::Cover { .. } | RenderItem::View { .. } => None,
            })
            .collect();
        // Surfaces arriving start their transitions in this frame; those going are drawn from
        // their last frame composed (ADR 0011). Locked, nothing of the session is drawn going.
        let now = self.now();
        // A window minimized goes into the dock, the shell's bottom panel, and comes back out
        // of it; a workspace's windows slide by the width of the output they are on.
        let dock = self.scene.dock().map(|dock| {
            (
                dock.x + dock.width as i32 / 2,
                dock.y + dock.height as i32 / 2,
            )
        });
        let drawn_at = |list: &[RenderItem], key: SurfaceKey| {
            list.iter().find_map(|item| match *item {
                RenderItem::Surface {
                    key: shown, rect, ..
                } if shown == key => Some(rect),
                _ => None,
            })
        };
        for appearance in self.scene.take_appearances() {
            // A window zoomed (ADR 0016): its frame goes from where it was drawn last (where its
            // zoom has it, if zooming) to where it is configured.
            if appearance.change == oxyos_compositor_core::AppearanceChange::Zoomed {
                let key = appearance.key;
                if let (Some(at), Some(to)) = (drawn_at(&list, key), self.scene.upcoming_rect(key))
                {
                    // Its corners square as it fills the work area and round as it leaves it.
                    let corner = looks.get(&key).map_or(0, |&look| window::corner(look));
                    let (from, radius) = self.transitions.zooming(key, now).unwrap_or_else(|| {
                        let from = self
                            .screens
                            .iter()
                            .find_map(|screen| drawn_at(&screen.composed, key))
                            .unwrap_or(at);
                        (from, corner)
                    });
                    let upcoming = if self.scene.is_maximized(key) {
                        0
                    } else {
                        window::WINDOW_RADIUS
                    };
                    self.transitions.zoom(
                        key,
                        appearance.kind,
                        from,
                        at,
                        to,
                        (radius, upcoming),
                        now,
                    );
                }
                continue;
            }
            let snapshot = if appearance.arrived() {
                None
            } else {
                self.shown.remove(&appearance.key).map(|mut snapshot| {
                    // The windows a shell surface showed live go with it, from their frames now.
                    let views = self.shown_views.remove(&appearance.key).unwrap_or_default();
                    snapshot.views = views
                        .into_iter()
                        .filter_map(|view| {
                            let (extent, format, pixels) = read_frame(
                                (&mut self.frames_read, &mut self.bytes_read),
                                (
                                    &self.peers,
                                    &self.gpu_buffers,
                                    self.gpu.is_none() || !self.gpu_only,
                                ),
                                view.window,
                                view.sequence,
                                view.buffer,
                            )?;
                            Some(transitions::ViewSnapshot {
                                rect: view.rect,
                                clip: view.clip,
                                radius: view.radius,
                                opacity: view.opacity,
                                extent,
                                format,
                                pixels,
                            })
                        })
                        .collect();
                    snapshot
                })
            };
            let rect = snapshot.as_ref().map(|snapshot| snapshot.rect).or_else(|| {
                list.iter().find_map(|item| match *item {
                    RenderItem::Surface { key, rect, .. } if key == appearance.key => Some(rect),
                    _ => None,
                })
            });
            let output = rect
                .and_then(|rect| {
                    self.scene.output_at_point(
                        rect.x + rect.width as i32 / 2,
                        rect.y + rect.height as i32 / 2,
                    )
                })
                .or_else(|| self.scene.outputs().first().copied())
                .map_or(Rect::new(0, 0, 1, 1), |output| output.rect);
            let place = transitions::Place { output, dock };
            self.transitions.appear(appearance, now, snapshot, &place);
        }
        if self.scene.is_locked() {
            self.transitions.lock();
        }
        for item in &list {
            if let RenderItem::Surface { key, rect, .. } = *item {
                self.transitions.zoom_shown(key, rect);
            }
        }
        let animating = self.transitions.advance(now);
        let changed = ready.iter().zip(&started).any(|(&index, &started)| {
            let screen = &self.screens[index];
            started > 0 || screen.recompose || list != screen.composed || looks != screen.looks
        });
        if !changed && !animating {
            for (&index, moved) in ready.iter().zip(&pointer_moved) {
                let id = self.screens[index].id;
                self.scene.end_frame_on(id, 0, Clock::Synthetic);
                if *moved {
                    self.present_pointer(index);
                }
            }
            return;
        }
        self.composing_since = monotonic();
        let locked = self.scene.is_locked();
        if animating {
            self.transition_frames += 1;
        }
        self.transitions.record(now);
        // What each surface shows now, completed: what a frame presenting a new one changed is
        // relative to (`presenting_damage`).
        let displayed: HashMap<SurfaceKey, oxyos_compositor_core::Frame> = self
            .scene
            .render_list()
            .into_iter()
            .filter_map(|item| match item {
                RenderItem::Surface { key, frame, .. } => Some((key, frame)),
                RenderItem::Cover { .. } | RenderItem::View { .. } => None,
            })
            .collect();
        // Each surface's pixels, read from its shared buffer, and the covers.
        // Where this frame draws surfaces in transition: their rectangle and where they are placed,
        // a window's with its shadow.
        let mut transition_drawn: Vec<oxyos_gpu::Rect> = Vec::new();
        let shadows = &self.shadows;
        // A window zooming is drawn in its rectangle as a window of that size (`zoomed`): its
        // corners and shadow are not scaled.
        let mut in_transition = |rect: Rect,
                                 placed: Rect,
                                 look: Option<WindowLook>,
                                 zoomed: bool| {
            for at in [rect, placed] {
                transition_drawn.push(match look {
                    Some(look) => window::extent(look, if zoomed { at } else { rect }, at, shadows),
                    None => oxyos_gpu::Rect {
                        x: at.x,
                        y: at.y,
                        width: at.width,
                        height: at.height,
                    },
                });
            }
        };
        let mut unreadable = Vec::new();
        let mut contents = Vec::new();
        // Where each surface is drawn: its content's index, its rectangle, where its transition
        // places it and at what opacity; and the windows each shell surface showed live.
        let mut surfaces_at: HashMap<SurfaceKey, (usize, Rect, Rect, u8)> = HashMap::new();
        let mut views_shown: HashMap<SurfaceKey, Vec<ShownView>> = HashMap::new();
        for item in list.iter().copied() {
            match item {
                RenderItem::Surface { key, rect, frame } => match read_frame(
                    (&mut self.frames_read, &mut self.bytes_read),
                    (
                        &self.peers,
                        &self.gpu_buffers,
                        self.gpu.is_none() || !self.gpu_only,
                    ),
                    key,
                    frame.sequence,
                    frame.buffer,
                ) {
                    Some((extent, format, pixels)) => {
                        let source = self.versions.get(&(key, frame.sequence)).map(|&version| {
                            oxyos_gpu::Source {
                                key: (u64::from(key.client) << 32) | u64::from(key.id),
                                version,
                            }
                        });
                        // What changed since the version composed last, when the frame says:
                        // only for a frame presenting now over the one composed last.
                        let shown = displayed
                            .get(&key)
                            .and_then(|shown| self.versions.get(&(key, shown.sequence)));
                        let changed = source.and_then(|source| {
                            let &from = self.shown_versions.get(&source.key)?;
                            match self.scene.presenting_damage(key)? {
                                oxyos_compositor_core::SurfaceDamage::Rects(rects)
                                    if from != source.version && shown == Some(&from) =>
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
                        let look = looks.get(&key).copied();
                        let zooming = self.transitions.zooming(key, now);
                        let zoomed = zooming.map(|(at, _)| at);
                        let placed = zoomed.unwrap_or_else(|| arriving.place(rect));
                        if self.transitions.is_arriving(key) || zoomed.is_some() {
                            in_transition(rect, placed, look, zoomed.is_some());
                        }
                        if arriving.opacity < 255 && self.transitions.lock_arriving(key) {
                            contents.push(Content::cover(rect));
                        }
                        // A window's shadow beneath it, and its corners: a window zooming has
                        // them as a window of the size it is drawn at, its corners on their way
                        // from one radius to the other.
                        let radius = look.map_or(0, |look| {
                            let cast = if zoomed.is_some() { placed } else { rect };
                            let corner = zooming.map_or_else(|| window::corner(look), |(_, r)| r);
                            let (shadow, radius) = Content::shadow(
                                shadows,
                                look,
                                corner,
                                cast,
                                placed,
                                arriving.opacity,
                            );
                            contents.push(shadow);
                            radius
                        });
                        contents.push(Content {
                            key: Some(key),
                            rect,
                            placed,
                            extent,
                            format: Some(format),
                            pixels,
                            source,
                            changed,
                            opacity: arriving.opacity,
                            radius,
                            shadow: None,
                            look,
                            clip: None,
                            shares: None,
                        });
                        surfaces_at
                            .insert(key, (contents.len() - 1, rect, placed, arriving.opacity));
                    }
                    None => unreadable.push(key.client),
                },
                RenderItem::Cover { rect, .. } => contents.push(Content::cover(rect)),
                // A window the shell's surface shows live (ADR 0022): over it, where the shell
                // placed it, as the surface is drawn (in its transition), box-filtered.
                RenderItem::View {
                    surface,
                    window,
                    frame,
                    rect,
                    clip,
                    radius,
                    opacity,
                } => {
                    let Some(&(_, surface_rect, surface_placed, surface_opacity)) =
                        surfaces_at.get(&surface)
                    else {
                        continue;
                    };
                    // The window's frame, read once: shared with its own layer when it is drawn.
                    let drawn = list.iter().any(|item| {
                        matches!(item, RenderItem::Surface { key, frame: drawn, .. }
                            if *key == window && drawn.buffer == frame.buffer)
                    });
                    let read = match surfaces_at.get(&window) {
                        Some(&(index, ..)) if drawn => {
                            let shared = &contents[index];
                            shared
                                .format
                                .map(|format| (shared.extent, format, Rc::default(), Some(index)))
                        }
                        _ => read_frame(
                            (&mut self.frames_read, &mut self.bytes_read),
                            (
                                &self.peers,
                                &self.gpu_buffers,
                                self.gpu.is_none() || !self.gpu_only,
                            ),
                            window,
                            frame.sequence,
                            frame.buffer,
                        )
                        .map(|(extent, format, pixels)| (extent, format, pixels, None)),
                    };
                    // A frame that cannot be read is not drawn: its window's own layer says so.
                    let Some((extent, format, pixels, shares)) = read else {
                        continue;
                    };
                    let source = self
                        .versions
                        .get(&(window, frame.sequence))
                        .map(|&version| oxyos_gpu::Source {
                            key: (u64::from(window.client) << 32) | u64::from(window.id),
                            version,
                        });
                    views_shown.entry(surface).or_default().push(ShownView {
                        window,
                        sequence: frame.sequence,
                        buffer: frame.buffer,
                        rect,
                        clip,
                        radius,
                        opacity,
                    });
                    let placed = placed_within(surface_rect, surface_placed, rect);
                    let clip = placed_within(surface_rect, surface_placed, clip);
                    contents.push(Content {
                        key: None,
                        rect: placed,
                        placed,
                        extent,
                        format: Some(format),
                        pixels,
                        source,
                        changed: None,
                        opacity: (u32::from(opacity) * u32::from(surface_opacity) / 255) as u8,
                        radius,
                        shadow: None,
                        look: None,
                        clip: Some(oxyos_gpu::Rect {
                            x: clip.x,
                            y: clip.y,
                            width: clip.width,
                            height: clip.height,
                        }),
                        shares,
                    });
                }
            }
        }
        // Frames read of surfaces no longer drawn are let go: a surface drawn again is read again.
        self.frames_read.retain(|key, _| {
            list.iter().any(|item| match *item {
                RenderItem::Surface { key: drawn, .. } | RenderItem::View { window: drawn, .. } => {
                    drawn == *key
                }
                RenderItem::Cover { .. } => false,
            })
        });
        // The splash over everything while the session starts, on every screen.
        for splash in &self.splash {
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
                radius: 0,
                shadow: None,
                look: None,
                clip: None,
                shares: None,
            });
        }
        // Surfaces going, over what stays, from their last frames.
        for (snapshot, _, at) in self.transitions.going(now) {
            let placed = at.place(snapshot.rect);
            in_transition(snapshot.rect, placed, snapshot.look, false);
            let radius = snapshot.look.map_or(0, |look| {
                let (shadow, radius) = Content::shadow(
                    shadows,
                    look,
                    window::corner(look),
                    snapshot.rect,
                    placed,
                    at.opacity,
                );
                contents.push(shadow);
                radius
            });
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
                radius,
                shadow: None,
                look: None,
                clip: None,
                shares: None,
            });
            // The windows it showed live go with it (ADR 0022).
            for view in &snapshot.views {
                let clip = placed_within(snapshot.rect, placed, view.clip);
                contents.push(Content {
                    key: None,
                    rect: view.rect,
                    placed: placed_within(snapshot.rect, placed, view.rect),
                    extent: view.extent,
                    format: Some(view.format),
                    pixels: view.pixels.clone(),
                    source: None,
                    changed: None,
                    opacity: (u32::from(view.opacity) * u32::from(at.opacity) / 255) as u8,
                    radius: view.radius,
                    shadow: None,
                    look: None,
                    clip: Some(oxyos_gpu::Rect {
                        x: clip.x,
                        y: clip.y,
                        width: clip.width,
                        height: clip.height,
                    }),
                    shares: None,
                });
            }
        }
        if self.records {
            self.reads
                .push((monotonic() - self.composing_since).as_secs_f64() * 1e3);
        }
        // The layers in the global space (logical pixels); each screen places them in its own.
        let layers: Vec<oxyos_gpu::Layer<'_>> = contents
            .iter()
            .map(|content| {
                // A window shown live reads its own layer's pixels.
                (
                    content,
                    content.shares.map_or(content.pixels.as_slice(), |index| {
                        contents[index].pixels.as_slice()
                    }),
                )
            })
            .map(|(content, pixels)| {
                // A client's GPU buffer, sampled in place where the Vulkan compositor holds it.
                let gpu = content
                    .shares
                    .map_or(&content.pixels, |index| &contents[index].pixels)
                    .gpu
                    .as_ref();
                (content, pixels, gpu)
            })
            .map(|(content, pixels, gpu)| oxyos_gpu::Layer {
                x: content.placed.x,
                y: content.placed.y,
                width: content.placed.width,
                height: content.placed.height,
                extent: (content.extent.width, content.extent.height),
                paint: match (&content.shadow, content.format) {
                    (Some((profile, shadow)), _) => {
                        oxyos_gpu::Paint::Shadow(profile.pixels(), *shadow)
                    }
                    (None, Some(_)) if gpu.is_some() => {
                        let gpu = gpu.expect("checked");
                        oxyos_gpu::Paint::Imported {
                            image: gpu.image,
                            opaque: gpu.opaque,
                            pixels,
                        }
                    }
                    (None, Some(oxyos_protocol::Format::Xrgb8888)) => {
                        oxyos_gpu::Paint::Opaque(pixels)
                    }
                    (None, Some(oxyos_protocol::Format::Argb8888)) => {
                        oxyos_gpu::Paint::Premultiplied(pixels)
                    }
                    (None, None) => oxyos_gpu::Paint::Fill(0),
                },
                source: content.source,
                changed: content
                    .changed
                    .as_ref()
                    .map(|(from, rects)| (*from, rects.as_slice())),
                opacity: content.opacity,
                radius: content.radius,
                // Drawn at another size (a transition, a window's view in the overview, an
                // output's scale), filtered (ADR 0011, step 5; ADR 0022).
                filter: oxyos_gpu::Filter::Box,
                clip: content.clip,
            })
            .collect();
        // Frames older than those composed are never shown again.
        self.versions.retain(|(key, sequence), _| {
            list.iter().all(|item| match item {
                RenderItem::Surface {
                    key: shown, frame, ..
                }
                | RenderItem::View {
                    window: shown,
                    frame,
                    ..
                } if shown == key => *sequence >= frame.sequence,
                _ => true,
            })
        });
        // A buffer that could not be read is never reported as presented: its client is
        // disconnected before the frame completes, and its surfaces are not of what is composed.
        for client in unreadable {
            self.fail(client, ErrorCode::InvalidBuffer);
        }
        let composed_list = self.scene.frame_list();
        self.frames += 1;
        for (&index, (&started, &moved)) in ready.iter().zip(started.iter().zip(&pointer_moved)) {
            self.compose_screen(
                index,
                ScreenFrame {
                    list: &composed_list,
                    looks: &looks,
                    displayed: &displayed,
                    layers: &layers,
                    transition_drawn: &transition_drawn,
                    started,
                    locked,
                    pointer_moved: moved,
                },
            );
        }
        if self.records {
            self.frame_times.push((
                self.composing_since,
                (monotonic() - self.composing_since).as_secs_f64() * 1e3,
                self.reads.last().copied().unwrap_or(0.0),
            ));
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
                        look: content.look,
                        views: Vec::new(),
                    },
                ))
            })
            .collect();
        self.shown_views = views_shown;
    }

    /// Composes one screen's frame from what the turn gathered (`frame`), if anything on it
    /// changed or its frame presents something, and hands it to its display; else its frame
    /// completes at once.
    fn compose_screen(&mut self, index: usize, frame: ScreenFrame<'_>) {
        let screen = &self.screens[index];
        let id = screen.id;
        // What changed on the screen since it composed last (ADR 0005); everything when it never
        // has, or after its display came back. A surface's own damage is its frame presenting
        // over the one it showed, which is the one this screen composed last only if it did.
        let scene = &self.scene;
        let composed = &screen.composed;
        let drawn = |key: SurfaceKey, rect: Rect, extent: Size| {
            let shown = frame.displayed.get(&key)?;
            let composed_here = composed.iter().any(|item| {
                matches!(item, RenderItem::Surface { key: k, frame: f, .. } if *k == key && f == shown)
            });
            match scene.presenting_damage(key) {
                Some(oxyos_compositor_core::SurfaceDamage::Rects(rects)) if composed_here => Some(
                    rects
                        .iter()
                        .map(|damage| screen.damage_on(damage, rect, extent))
                        .collect(),
                ),
                _ => None,
            }
        };
        // Global, logical; then in the screen's pixels.
        let damage = (!screen.recompose).then(|| {
            let mut rects = output_damage(
                (&screen.composed, &screen.looks),
                (frame.list, frame.looks),
                &self.shadows,
                &drawn,
            );
            rects.extend(screen.transition_drawn.iter().copied());
            rects.extend(frame.transition_drawn.iter().copied());
            rects
                .iter()
                .filter_map(|rect| screen.covering(rect))
                .collect::<Vec<_>>()
        });
        let screen_rect = oxyos_gpu::Rect {
            x: 0,
            y: 0,
            width: screen.width,
            height: screen.height,
        };
        if frame.started == 0 && damage.as_ref().is_some_and(|damage| damage.is_empty()) {
            // Nothing on it changed: nothing composed, its frame completes now.
            self.scene.end_frame_on(id, 0, Clock::Synthetic);
            let screen = &mut self.screens[index];
            screen.composed = frame.list.to_vec();
            screen.looks = frame.looks.clone();
            screen.transition_drawn = frame.transition_drawn.to_vec();
            if frame.pointer_moved {
                self.present_pointer(index);
            }
            return;
        }
        // The layers on the screen, in its pixels.
        let layers: Vec<oxyos_gpu::Layer<'_>> = frame
            .layers
            .iter()
            .filter_map(|layer| screen.layer(layer))
            .collect();
        if self.records {
            let area = |rects: &[oxyos_gpu::Rect]| {
                let within: Vec<oxyos_gpu::Rect> = rects
                    .iter()
                    .filter_map(|rect| rect.intersect(&screen_rect))
                    .collect();
                union_area(&within)
            };
            self.damaged.push(match &damage {
                Some(rects) => (area(rects) as f64 / area(&[screen_rect]) as f64).min(1.0),
                None => 1.0,
            });
        }
        let (width, height) = (screen.width, screen.height);
        if !self.gpu_only {
            // Only what changed since the screen's frame before, onto it (ADR 0005); the whole
            // output when that is unknown.
            let framebuffer = &mut self.screens[index].framebuffer;
            match &damage {
                Some(damage) if framebuffer.len() == width as usize * height as usize => {
                    oxyos_gpu::compose_cpu_damaged(
                        framebuffer,
                        width,
                        height,
                        DESKTOP,
                        &layers,
                        damage,
                    )
                }
                _ => *framebuffer = oxyos_gpu::compose_cpu(width, height, DESKTOP, &layers),
            }
        }
        if self.keep_frame && index == 0 {
            self.last_frame.clear();
            oxyos_gpu::frame_file::write(&mut self.last_frame, width, height, DESKTOP, &layers)
                .expect("writing to memory");
        }
        if let (Some(gpu), false) = (&mut self.gpu, self.gpu_only) {
            gpu.frames += 1;
            // The device's own output holds another screen's frame: this one is composed whole.
            let composed = match &damage {
                Some(damage) if self.gpu_holds == Some(id) => gpu
                    .compositor
                    .compose_damaged(width, height, DESKTOP, &layers, damage),
                _ => gpu.compositor.compose(width, height, DESKTOP, &layers),
            };
            self.gpu_holds = Some(id);
            match composed {
                Ok(pixels) if pixels == self.screens[index].framebuffer => {}
                Ok(_) => gpu.mismatches += 1,
                Err(error) => {
                    eprintln!("[gpu] {error}");
                    gpu.mismatches += 1;
                }
            }
        }
        {
            let screen = &mut self.screens[index];
            screen.composed = frame.list.to_vec();
            screen.looks = frame.looks.clone();
            screen.transition_drawn = frame.transition_drawn.to_vec();
            screen.recompose = false;
            screen.composing_locked = frame.locked;
            screen.last_damage = damage.clone();
        }
        // On a display, the frame completes when it is on screen, at the kernel's flip time, in
        // a later turn (`complete_flip`).
        let presented = self.present(index, &layers, damage.as_deref());
        if !presented {
            // Paced as a display (no display, not away): shown at its next vblank, in a later turn.
            let paced = self.screens[index].paced && self.screens[index].display.is_none();
            if paced && !self.away {
                self.screens[index].pending_flip = Some(monotonic());
            } else {
                self.scene.end_frame_on(id, 0, Clock::Synthetic);
                self.retire();
            }
            if self.gpu_only && self.reads_back {
                self.flush();
                self.framebuffer_from_gpu(index, &layers);
            }
        }
    }

    /// Shows the session's splash, the Oxy mark on black, over everything on every screen from
    /// the next frame.
    pub fn show_splash(&mut self) {
        self.splash = self
            .screens
            .iter()
            .filter_map(|screen| {
                let mut splash = splash::splash(Size::new(screen.width, screen.height))?;
                splash.rect = screen.rect;
                Some(splash)
            })
            .collect();
        // The next frames are composed whole, with it.
        for screen in &mut self.screens {
            screen.composed.clear();
            screen.recompose = true;
        }
    }

    pub fn showing_splash(&self) -> bool {
        !self.splash.is_empty()
    }

    /// The session is on screen: the splash fades out over it (400 ms).
    pub fn end_splash(&mut self) {
        let now = self.now();
        for (index, splash) in std::mem::take(&mut self.splash).into_iter().enumerate() {
            self.transitions.fade(index as u32, splash, now);
        }
    }

    /// The time transitions run on.
    fn now(&self) -> Duration {
        self.clock.unwrap_or_else(monotonic)
    }

    /// Only the pointer moved (or its animated image went on): the frame a screen shows is
    /// committed again with it where it is now, no frame composed. Its flip completes no
    /// client's frame.
    fn present_pointer(&mut self, index: usize) {
        let screen = &mut self.screens[index];
        let Some(display) = screen.display.as_mut() else {
            return;
        };
        match display.present_again() {
            Ok(true) => {
                screen.pending_flip = Some(monotonic());
                screen.pointer_flip = true;
            }
            Ok(false) => {}
            Err(error) => {
                eprintln!("[kms] {error}; output {} stops", screen.id);
                screen.display = None;
            }
        }
    }

    /// Completes the frames on their way to the displays once they have flipped, each at its
    /// kernel's flip time; with `wait`, waits for their flips (before giving the displays away,
    /// or at the end).
    pub fn complete_flip(&mut self, wait: bool) {
        for index in 0..self.screens.len() {
            self.complete_screen_flip(index, wait);
        }
    }

    fn complete_screen_flip(&mut self, index: usize, wait: bool) {
        let screen = &mut self.screens[index];
        let Some(submitted) = screen.pending_flip else {
            return;
        };
        // Paced without a display: its frame is shown at its next vblank (waited for with `wait`),
        // and read back, if it is, when composed.
        let vblank = (screen.paced && screen.display.is_none()).then(|| {
            let due = screen.next_vblank(submitted);
            let now = monotonic();
            if now < due && wait {
                std::thread::sleep(due - now);
            }
            (wait || monotonic() >= due).then_some(due)
        });
        let flip = match &screen.display {
            _ if vblank.is_some() => Ok(vblank.flatten()),
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
        if screen.pointer_flip {
            // The pointer moved on the frame shown: nothing of the scene completes.
            match flip {
                Ok(None) if !wait => return,
                Err(error) => {
                    eprintln!("[kms] {error}; output {} stops", screen.id);
                    screen.display = None;
                }
                _ => {}
            }
            screen.pointer_flip = false;
            screen.pending_flip = None;
            return;
        }
        let (timestamp_ns, clock) = match flip {
            Ok(None) if !wait => return,
            // Never flipped: the frame completes unshown, and is composed again.
            Ok(None) => {
                eprintln!("[kms] a frame never flipped");
                screen.composed.clear();
                screen.recompose = true;
                (0, Clock::Synthetic)
            }
            Ok(Some(flip)) => {
                // What it shows now holds its client's buffer; what it showed is let go.
                screen.direct.shown = screen.direct.next.take();
                if self.records {
                    screen.flips.push((submitted, flip));
                }
                if let Some(first_flip) = self.on_first_flip.take() {
                    first_flip();
                }
                (
                    u64::try_from(flip.as_nanos()).unwrap_or(u64::MAX),
                    Clock::Monotonic,
                )
            }
            Err(error) => {
                eprintln!("[kms] {error}; output {} stops", screen.id);
                screen.display = None;
                // Composed again for the director's checks, without the display.
                screen.composed.clear();
                screen.recompose = true;
                (0, Clock::Synthetic)
            }
        };
        screen.pending_flip = None;
        // A frame that never flipped leaves the one shown on the display.
        screen.direct.next = None;
        let shown_direct = screen.direct.shown_direct;
        let id = screen.id;
        self.scene.end_frame_on(id, timestamp_ns, clock);
        self.retire();
        if clock == Clock::Monotonic {
            let shown = self.scene.render_list();
            let screen = &mut self.screens[index];
            screen.flipped = shown;
            screen.shown_locked = screen.composing_locked;
        }
        // A client's buffer shown directly was copied for the director when it was handed over.
        if self.gpu_only
            && self.reads_back
            && clock == Clock::Monotonic
            && vblank.is_none()
            && !shown_direct
        {
            // The clients hear the frame was shown before the director reads it back, so its
            // check delays no client's next frame.
            self.flush();
            self.framebuffer_from_gpu(index, &[]);
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
                    // The frame on its way is shown before the display is given away. The terminal's
                    // path drives one display (the development compositor's): master is the card's.
                    self.complete_flip(true);
                    if let Some(display) = self.screens.iter_mut().find_map(|s| s.display.as_mut())
                    {
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
                    if let Some(display) = self.screens.iter_mut().find_map(|s| s.display.as_mut())
                    {
                        display.resume()?;
                    }
                    if let Some(input) = &mut self.input {
                        input.resume()?;
                    }
                    self.away = false;
                    self.shown_again();
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
                        for display in self.screens.iter_mut().filter_map(|s| s.display.as_mut()) {
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
                // The user's processes hold idle back, or no longer (a video playing).
                oxyos_seat::Event::IdleInhibited(held) => {
                    eprintln!(
                        "[idle] {}",
                        if held {
                            "held back: the session does not lock while it is"
                        } else {
                            "no longer held back"
                        }
                    );
                    let now = self.now();
                    self.idle.set_held(held, now);
                }
                oxyos_seat::Event::Resumed(device, fd) => {
                    drop(fd);
                    if Some(device) == self.card_device && self.away {
                        if let Some(input) = &mut self.input {
                            input.resume()?;
                        }
                        self.away = false;
                        self.shown_again();
                    }
                }
            }
        }
        Ok(())
    }

    /// The displays are back from another session: every screen is composed again and shown
    /// with a modeset, and the connectors are read again, as monitors may have come or gone
    /// meanwhile.
    fn shown_again(&mut self) {
        for screen in &mut self.screens {
            screen.composed.clear();
            screen.recompose = true;
        }
        self.hotplugged = self.hotplug.is_some();
    }

    /// With `--input`: what the seat's devices did since the last turn, applied to the scene.
    /// Positions arrive in the global space, offset by `display_offset` (the development
    /// compositor's output is centred on its display).
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
        for oxyos_input::Timed { time_ns, action } in actions {
            if trace {
                eprintln!(
                    "[input] {action:?} at {time_ns} focus {:?}",
                    self.scene.focus()
                );
            }
            // What it makes reaches the clients with when its device registered it.
            self.scene.set_input_time(time_ns);
            match action {
                oxyos_input::Action::Motion { x, y } => {
                    let (x, y) = (x.round() as i32 - dx, y.round() as i32 - dy);
                    self.pointer = Some((x, y));
                    self.scene.pointer_motion(x, y)
                }
                oxyos_input::Action::Button { code, pressed } => {
                    self.scene.pointer_button(code, pressed)
                }
                // libinput's units are logical pixels; the protocol's are 1/256 of one.
                oxyos_input::Action::Scroll {
                    dx,
                    dy,
                    source,
                    stop,
                } => self.scene.pointer_scroll(
                    (dx * 256.0).round() as i32,
                    (dy * 256.0).round() as i32,
                    match source {
                        oxyos_input::ScrollSource::Wheel => ScrollSource::Wheel,
                        oxyos_input::ScrollSource::Finger => ScrollSource::Finger,
                        oxyos_input::ScrollSource::Continuous => ScrollSource::Continuous,
                    },
                    stop,
                ),
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
                oxyos_input::Action::Hold { phase, fingers } => self.scene.pointer_hold(
                    match phase {
                        oxyos_input::GesturePhase::Begin => GesturePhase::Begin,
                        oxyos_input::GesturePhase::Update => GesturePhase::Update,
                        oxyos_input::GesturePhase::End => GesturePhase::End,
                        oxyos_input::GesturePhase::Cancel => GesturePhase::Cancel,
                    },
                    u8::try_from(fingers).unwrap_or(u8::MAX),
                ),
                oxyos_input::Action::Key {
                    code,
                    pressed,
                    text,
                } => self.scene.keyboard(code, pressed, text.as_deref()),
                oxyos_input::Action::Terminate => self.ending = true,
                oxyos_input::Action::Lock => self.scene.lock(),
                oxyos_input::Action::ToggleReader => self.reader_toggled = true,
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

    /// With `--gpu-only`: the director's frame of a screen is the GPU's, the screen's part of
    /// the frame shown (read back after the flip) or, with nothing shown, composed for it. An
    /// error counts as a mismatch, and the run fails.
    pub fn framebuffer_from_gpu(&mut self, index: usize, layers: &[oxyos_gpu::Layer<'_>]) {
        let Some(gpu) = &mut self.gpu else {
            return;
        };
        let screen = &mut self.screens[index];
        let display = screen.display.as_ref().map(|display| display.size());
        gpu.frames += 1;
        let started = monotonic();
        let (width, height) = (screen.width, screen.height);
        let frame = match display {
            Some((display_width, display_height)) if screen.shown_kept => {
                screen.shown_kept = false;
                let dx = (display_width.saturating_sub(width) / 2) as usize;
                let dy = (display_height.saturating_sub(height) / 2) as usize;
                gpu.compositor.read_kept().map(|shown| {
                    shown
                        .chunks_exact(display_width as usize)
                        .skip(dy)
                        .take(height as usize)
                        .flat_map(|row| row[dx..dx + width as usize].iter().copied())
                        .collect()
                })
            }
            _ => {
                self.gpu_holds = Some(screen.id);
                gpu.compositor.compose(width, height, DESKTOP, layers)
            }
        };
        gpu.readback_ms
            .push((monotonic() - started).as_secs_f64() * 1e3);
        match frame {
            Ok(frame) => screen.framebuffer = frame,
            Err(error) => {
                eprintln!("[gpu] {error}");
                gpu.mismatches += 1;
            }
        }
    }

    /// Hands a screen's composed frame, centred, to its display, which flips to it at its next
    /// refresh (`complete_flip`); true once handed over. With `--gpu` too, the frame is composed
    /// on the Vulkan device straight into one of the screen's scanout buffers, with no copy
    /// through the CPU. A display error ends that screen's display output.
    pub fn present(
        &mut self,
        index: usize,
        layers: &[oxyos_gpu::Layer<'_>],
        damage: Option<&[oxyos_gpu::Rect]>,
    ) -> bool {
        if self.away {
            return false;
        }
        let screen = &mut self.screens[index];
        if screen.display.is_none() {
            return false;
        }
        // A full-screen window's buffer straight to the display, where it can be (ADR 0031,
        // decision 8); its surface told whether it could be first.
        self.tell_scanout(index);
        if let Some(shown) = self.present_direct(index, layers) {
            return shown;
        }
        self.composing_again(index);
        let screen = &mut self.screens[index];
        let Some(display) = screen.display.as_mut() else {
            return false;
        };
        let (width, height) = display.size();
        if let (Some(gpu), false) = (&mut self.gpu, screen.scanout.is_empty()) {
            let dx = (width.saturating_sub(screen.width) / 2) as i32;
            let dy = (height.saturating_sub(screen.height) / 2) as i32;
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
            let (target, framebuffer) = screen.scanout[screen.scanout_back];
            // A frame kept for the director's check is waited for; a session's goes to the display
            // with its completion, if the device gives one, and the turn goes on at once.
            let composed = if self.gpu_only && self.reads_back {
                screen.shown_kept = true;
                gpu.compositor
                    .compose_to_scanout_kept(target, DESKTOP, &shifted, damage.as_deref())
                    .map(|()| None)
            } else {
                gpu.compositor
                    .compose_to_scanout(target, DESKTOP, &shifted, damage.as_deref())
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
            let presented = composed
                .and_then(|completion| display.present_framebuffer_after(framebuffer, completion));
            screen.scanout_back = (screen.scanout_back + 1) % screen.scanout.len();
            return self.handed_over(index, presented, submitted);
        }
        let mut pixels = vec![DESKTOP; width as usize * height as usize];
        let dx = (width.saturating_sub(screen.width) / 2) as usize;
        let dy = (height.saturating_sub(screen.height) / 2) as usize;
        let columns = (screen.width as usize).min(width as usize);
        for (y, row) in screen
            .framebuffer
            .chunks_exact(screen.width as usize)
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
        self.handed_over(index, presented, submitted)
    }

    pub fn handed_over(
        &mut self,
        index: usize,
        presented: std::io::Result<()>,
        submitted: std::time::Duration,
    ) -> bool {
        let screen = &mut self.screens[index];
        match presented {
            Ok(()) => {
                screen.pending_flip = Some(submitted);
                true
            }
            Err(error) => {
                eprintln!("[kms] {error}; output {} stops", screen.id);
                screen.display = None;
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
        read_pixels((&self.peers, &self.gpu_buffers, true), key, buffer)
            .map(|(extent, format, pixels)| (extent, format, pixels.bytes))
    }

    /// The session locks after `minutes` with no input (0: never); a change arms the idle
    /// lock's deadline again from now. The user's `lock_after` (settings.rs).
    pub fn set_lock_after(&mut self, minutes: u32) {
        let now = self.now();
        self.idle.set_lock_after(minutes, now);
    }

    /// One turn of the idle lock: the person's input since the last arms it again, and once its
    /// deadline passes the session locks (unless it is already) and logind is told it is idle.
    pub fn follow_idle(&mut self) {
        let now = self.now();
        let turn = self
            .idle
            .turn(self.scene.activity(), self.scene.is_locked(), now);
        if turn.lock {
            eprintln!(
                "[idle] no input for {:?}: the session locks",
                self.idle.lock_after().unwrap_or_default()
            );
            self.scene.lock();
        }
        if let (Some(idle), Some(seat), false) = (turn.idle, &self.seat, self.idle_hint_refused) {
            if let Err(error) = seat.set_idle_hint(idle) {
                // systemd 257 takes it only from an x11, wayland or mir session, never a
                // terminal's: once refused, it is not asked again.
                eprintln!("[seat] the idle hint: {error}; logind is not told again");
                self.idle_hint_refused = true;
            }
        }
    }

    /// How long a turn may wait: until the animated pointer's next step, the first connection
    /// still without its Hello reaches its deadline, the idle lock's deadline, a transition's next
    /// frame, a paced screen's next vblank, or `idle_wait`, whichever is soonest.
    fn next_wake(&self) -> Duration {
        let mut wait = self.idle_wait;
        // A client's GPU buffer released once a frame the GPU composes is done, with no sync file
        // to wake the turn: looked at again soon.
        if self.gpu_sync_pending()
            && self
                .gpu
                .as_ref()
                .is_some_and(|gpu| gpu.compositor.completion_fd().is_none())
        {
            wait = wait.min(Duration::from_millis(1));
        }
        if let Some(deadline) = self.idle.deadline() {
            wait = wait.min(deadline.saturating_sub(self.now()));
        }
        if let Some(controls) = &self.controls {
            wait = wait.min(controls.next_wake(Instant::now()));
        }
        // A transition running composes a frame for every refresh of the fastest display (a
        // display's flip wakes the turn sooner).
        if let (false, Some(period)) = (self.transitions.is_empty(), self.fastest_refresh()) {
            wait = wait.min(period);
        }
        // A frame paced without a display is shown at its screen's next vblank.
        let clock = monotonic();
        for screen in &self.screens {
            if let (Some(submitted), None, true) =
                (screen.pending_flip, &screen.display, screen.paced)
            {
                wait = wait.min(screen.next_vblank(submitted).saturating_sub(clock));
            }
        }
        let shape = self.scene.pointer_shape();
        for screen in &self.screens {
            let steps = screen.pointer_images.get(&shape);
            if let Some(step) = steps.and_then(|steps| pointer_step_wait(steps, monotonic())) {
                wait = wait.min(step);
            }
        }
        for peer in self.peers.values() {
            if peer.protocol.version().is_none() {
                wait = wait.min(HELLO_DEADLINE.saturating_sub(peer.accepted.elapsed()));
            }
        }
        wait
    }

    /// One turn of the compositor: accepts connections, handles every readable client, reads the
    /// devices, composes a frame and sends events. Waits for a client, the listener, the display's
    /// flip, the devices or the seat, at most until the pointer's next step, a connection's
    /// deadline, the idle lock's or `idle_wait`.
    pub fn serve(&mut self, listener: &Listener) -> Result<()> {
        let started = Instant::now();
        self.follow_terminal()?;
        self.follow_seat()?;
        self.follow_idle();
        let clients: Vec<ClientId> = self.peers.keys().copied().collect();
        // Poll the listener, every client and, with a frame on its way, the display (its flip);
        // never block on any one of them.
        let shell = usize::from(self.shell_listener.is_some());
        let reader = usize::from(self.reader_listener.is_some());
        let waiting: Instant;
        let readable: Vec<bool> = {
            let mut fds: Vec<PollFd<'_>> = Vec::with_capacity(clients.len() + 4);
            fds.push(PollFd::from_borrowed_fd(listener.as_fd(), PollFlags::IN));
            for extra in [&self.shell_listener, &self.reader_listener]
                .into_iter()
                .flatten()
            {
                fds.push(PollFd::from_borrowed_fd(extra.as_fd(), PollFlags::IN));
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
            // The card of each display with a frame on its way (one card: its flips are read
            // once, for every display, `Display::flip_done`).
            for screen in &self.screens {
                if let (Some(display), Some(_)) = (&screen.display, screen.pending_flip) {
                    fds.push(PollFd::from_borrowed_fd(
                        display.card().as_fd(),
                        PollFlags::IN,
                    ));
                }
            }
            // libinput, the seat and the card's hotplugs are read every turn; they only need to
            // wake it.
            if let Some(hotplug) = &self.hotplug {
                fds.push(PollFd::from_borrowed_fd(hotplug.as_fd(), PollFlags::IN));
            }
            if let Some(input) = &self.input {
                fds.push(PollFd::from_borrowed_fd(input.as_fd(), PollFlags::IN));
            }
            if let Some(seat) = &self.seat {
                fds.push(PollFd::from_borrowed_fd(seat.as_fd(), PollFlags::IN));
            }
            // The controls' worker answering, and the program's own: only to wake the turn.
            if let Some(wake) = self
                .controls
                .as_ref()
                .and_then(|controls| controls.wake_fd())
            {
                fds.push(PollFd::from_borrowed_fd(wake, PollFlags::IN));
            }
            for waker in &self.wakers {
                fds.push(PollFd::from_borrowed_fd(waker.as_fd(), PollFlags::IN));
            }
            // Acquire points a frame waits for, and the GPU finishing a frame that read a
            // client's buffer whose release waits for it (docs/adr/0031-gpu-buffers.mdx).
            for fd in self.gpu_buffers.wake_fds() {
                fds.push(PollFd::from_borrowed_fd(fd, PollFlags::IN));
            }
            if self.gpu_sync_pending() {
                if let Some(fd) = self
                    .gpu
                    .as_ref()
                    .and_then(|gpu| gpu.compositor.completion_fd())
                {
                    fds.push(PollFd::from_borrowed_fd(fd, PollFlags::IN));
                }
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
            waiting = Instant::now();
            match poll(&mut fds, Some(&timeout)) {
                Ok(_) => fds.iter().map(|fd| !fd.revents().is_empty()).collect(),
                // A signal (the terminal's switch requests) interrupts poll, which is never
                // restarted: nothing was ready this turn.
                Err(rustix::io::Errno::INTR) => vec![false; fds.len()],
                Err(error) => return Err(error.into()),
            }
        };
        let woke = Instant::now();
        self.complete_flip(false);
        // The input that woke the turn, or came while it waited, read now: the frame this turn
        // composes shows it. Read before the wait, it would be composed a turn later, behind this
        // turn's frame, and the devices' events would queue meanwhile (libinput: "event
        // processing lagging behind").
        self.read_input();
        let read = woke.elapsed();
        if let Some(hotplug) = &self.hotplug {
            match hotplug.changed() {
                Ok(changed) => self.hotplugged |= changed,
                Err(error) => {
                    eprintln!("[kms] hotplug events: {error}; monitors plugged in are not seen");
                    self.hotplug = None;
                }
            }
        }
        if readable[0] {
            self.accept(listener)?;
        }
        if shell == 1 && readable[1] {
            let shell_listener = self.shell_listener.take().expect("polled");
            let accepted = self.accept_granting(
                &shell_listener,
                // The shell is the lock screen as well (ADR 0010), and shows the system shortcuts
                // that need a picture: the application switcher and the search (ADR 0024).
                Some(Grants {
                    shell: true,
                    lock: true,
                    shortcuts: true,
                    ..Grants::default()
                }),
            );
            self.shell_listener = Some(shell_listener);
            accepted?;
        }
        if reader == 1 && readable[1 + shell] {
            let reader_listener = self.reader_listener.take().expect("polled");
            let accepted = self.accept_granting(
                &reader_listener,
                Some(Grants {
                    accessibility: true,
                    screen_reader: true,
                    ..Grants::default()
                }),
            );
            self.reader_listener = Some(reader_listener);
            accepted?;
        }
        for (index, client) in clients.iter().enumerate() {
            if readable[index + 1 + shell + reader] {
                self.receive(*client);
                for _ in 1..REQUESTS_PER_TURN {
                    if !self.peers.get(client).is_some_and(Peer::has_request) {
                        break;
                    }
                    self.receive(*client);
                }
            }
        }
        self.follow_gpu_sync();
        self.expire_pending();
        // A turn that waited until the deadline locks now, and composes the lock.
        self.follow_idle();
        self.authenticate();
        if let Some(controls) = &mut self.controls {
            controls.serve(&mut self.scene, Instant::now());
        }
        let framing = Instant::now();
        let frames = self.frames;
        self.frame();
        let framed = framing.elapsed();
        self.follow_lock();
        self.flush();
        if self.slow_turns {
            // The turn's work, not its wait.
            let work = waiting.duration_since(started) + woke.elapsed();
            let ms = |d: Duration| d.as_secs_f64() * 1e3;
            if self.fastest_refresh().is_some_and(|refresh| work > refresh)
                && self
                    .slow_turn_logged
                    .is_none_or(|logged| logged.elapsed() >= Duration::from_secs(1))
            {
                self.slow_turn_logged = Some(Instant::now());
                let gpu = match &self.gpu {
                    Some(gpu) if self.frames != frames => {
                        let (copy, wait) = gpu.compositor.last_host_ms();
                        format!(" (uploading {copy:.1}, waiting for the GPU {wait:.1})")
                    }
                    _ => String::new(),
                };
                eprintln!(
                    "[compositor] a turn worked {:.1} ms: before its wait {:.1}, the devices {:.1}, \
                     the clients {:.1}, the frame {:.1}{gpu}, events {:.1}",
                    ms(work),
                    ms(waiting.duration_since(started)),
                    ms(read),
                    ms(framing.duration_since(woke) - read),
                    ms(framed),
                    ms(woke.elapsed() - framing.duration_since(woke) - framed),
                );
            }
            // Every ten seconds that composed frames on the GPU: how long it took to compose them.
            if self
                .gpu_times_logged
                .is_none_or(|logged| logged.elapsed() >= Duration::from_secs(10))
            {
                self.gpu_times_logged = Some(Instant::now());
                if let Some(gpu) = &mut self.gpu {
                    let mut times = gpu.compositor.take_completed_ms();
                    if !times.is_empty() {
                        times.sort_by(f64::total_cmp);
                        eprintln!(
                            "[compositor] the GPU composed {} frames in the last 10 s: {:.2} ms median, \
                             {:.2} at the 95th percentile, {:.2} at most",
                            times.len(),
                            times[times.len() / 2],
                            times[(times.len() * 95 / 100).min(times.len() - 1)],
                            times[times.len() - 1],
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether the lock is on screen: the session is locked and the frame every display shows
    /// is of it (or nothing is shown: there is no display, or another terminal has them).
    pub fn lock_shown(&self) -> bool {
        self.scene.is_locked()
            && (self.away
                || self.screens.iter().all(|screen| {
                    screen.display.is_none()
                        || (screen.shown_locked && screen.pending_flip.is_none())
                }))
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

    /// Answers the password being checked once it has its answer, unlocking on `Accepted` (at a
    /// login screen, showing the splash: someone signed in), and hands the next one a lock
    /// surface submitted to the authentication path.
    pub fn authenticate(&mut self) {
        if let Some(key) = self.checking {
            let answer = self
                .authentication
                .as_mut()
                .and_then(|authentication| authentication.answer());
            if let Some((outcome, wait_s)) = answer {
                self.checking = None;
                self.scene.authenticated(key, outcome, wait_s);
                match (outcome, self.login) {
                    (AuthOutcome::Accepted, false) => self.scene.unlock(),
                    (AuthOutcome::Accepted, true) => {
                        if !self.signed_in {
                            self.signed_in = true;
                            self.show_splash();
                        }
                    }
                    _ => {}
                }
            }
        }
        while let Some(submitted) = self.scene.take_authentication() {
            let key = submitted.surface;
            let Some(authentication) = self.authentication.as_mut() else {
                eprintln!("[lock] no authentication path: the session stays locked");
                self.scene.authenticated(key, AuthOutcome::Rejected, 0);
                continue;
            };
            match authentication.attempt(submitted.account.as_deref(), submitted.secret) {
                None => self.checking = Some(key),
                Some((outcome, wait_s)) => self.scene.authenticated(key, outcome, wait_s),
            }
        }
    }

    /// The pixel composed last at a point of the global space: on the screen it is on, in the
    /// screen's pixels (`DESKTOP` off every screen).
    pub fn pixel(&self, x: i32, y: i32) -> u32 {
        self.screens
            .iter()
            .find(|screen| screen.rect.contains_point(x, y))
            .map_or(DESKTOP, |screen| {
                let (px, py, _, _) = screen.place(x, y, 0, 0);
                screen.pixel(
                    (px.max(0) as u32).min(screen.width - 1),
                    (py.max(0) as u32).min(screen.height - 1),
                )
            })
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

/// A surface's damage (buffer pixels) on the output, where the surface shows at `rect`: every
/// output pixel whose filtered sample reads a damaged buffer pixel (`oxyos_gpu::filter::span`,
/// ADR 0011), which drawn larger reaches up to a buffer pixel beyond them; at 1:1 the damage
/// itself.
pub fn on_output(damage: &Rect, rect: Rect, extent: Size) -> oxyos_gpu::Rect {
    let (x0, y0) = (damage.x.max(0) as u32, damage.y.max(0) as u32);
    let (left, right) = oxyos_gpu::filter::span(
        x0,
        x0.saturating_add(damage.width),
        extent.width,
        rect.width,
    );
    let (top, bottom) = oxyos_gpu::filter::span(
        y0,
        y0.saturating_add(damage.height),
        extent.height,
        rect.height,
    );
    oxyos_gpu::Rect {
        x: rect.x + left as i32,
        y: rect.y + top as i32,
        width: right - left,
        height: bottom - top,
    }
}

/// What changed on the output from one composed frame to the next (ADR 0005), from each frame's
/// list and how its windows were drawn: a surface where it is drawn (a window with its shadow,
/// ADR 0016) when it is new or gone, its damage (`drawn`, on the output) or else its rectangle
/// when it shows a new frame, where it was and is drawn when it moved, resized, changed places in
/// the stacking or changed its look (became active, full-screen), and covers that came or went.
pub fn output_damage(
    before: (&[RenderItem], &Looks),
    after: (&[RenderItem], &Looks),
    shadows: &window::Shadows,
    drawn: &dyn Fn(SurfaceKey, Rect, Size) -> Option<Vec<oxyos_gpu::Rect>>,
) -> Vec<oxyos_gpu::Rect> {
    let rect = |rect: Rect| oxyos_gpu::Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    };
    type Shown = (
        SurfaceKey,
        Rect,
        oxyos_compositor_core::Frame,
        Option<WindowLook>,
    );
    let surfaces = |(list, looks): (&[RenderItem], &Looks)| -> Vec<Shown> {
        list.iter()
            .filter_map(|item| match *item {
                RenderItem::Surface { key, rect, frame } => {
                    Some((key, rect, frame, looks.get(&key).copied()))
                }
                RenderItem::Cover { .. } | RenderItem::View { .. } => None,
            })
            .collect()
    };
    // Where a surface is drawn: a window with its shadow.
    let extent = |at: Rect, look: Option<WindowLook>| match look {
        Some(look) => window::extent(look, at, at, shadows),
        None => rect(at),
    };
    let (old, new) = (surfaces(before), surfaces(after));
    let mut damage = Vec::new();
    for &(key, now, frame, look) in &new {
        match old.iter().find(|(old_key, ..)| *old_key == key) {
            None => damage.push(extent(now, look)),
            Some(&(_, then, _, then_look)) if then != now || then_look != look => {
                damage.extend([extent(then, then_look), extent(now, look)])
            }
            Some(&(_, _, then_frame, _)) if then_frame != frame => {
                match drawn(key, now, frame.extent) {
                    Some(drawn) => damage.extend(drawn),
                    None => damage.push(rect(now)),
                }
            }
            Some(_) => {}
        }
    }
    for &(key, then, _, look) in &old {
        if !new.iter().any(|(new_key, ..)| *new_key == key) {
            damage.push(extent(then, look));
        }
    }
    // Surfaces in both whose order among themselves changed show differently where they meet.
    let order = |list: &[Shown], other: &[Shown]| {
        list.iter()
            .filter(|(key, ..)| other.iter().any(|(other_key, ..)| other_key == key))
            .map(|&(key, rect, _, look)| (key, extent(rect, look)))
            .collect::<Vec<_>>()
    };
    let (old_order, new_order) = (order(&old, &new), order(&new, &old));
    for (then, now) in old_order.iter().zip(&new_order) {
        if then.0 != now.0 {
            damage.extend([then.1, now.1]);
        }
    }
    // A window shown live (ADR 0022) where it is cut to, when it came, went, moved or shows a
    // new frame.
    let views = |list: &[RenderItem]| -> Vec<RenderItem> {
        list.iter()
            .filter(|item| matches!(item, RenderItem::View { .. }))
            .copied()
            .collect()
    };
    let (old_views, new_views) = (views(before.0), views(after.0));
    for (these, others) in [(&old_views, &new_views), (&new_views, &old_views)] {
        for view in these.iter().filter(|view| !others.contains(view)) {
            if let RenderItem::View { clip, .. } = view {
                damage.push(rect(*clip));
            }
        }
    }
    let covers = |list: &[RenderItem]| -> Vec<RenderItem> {
        list.iter()
            .filter(|item| matches!(item, RenderItem::Cover { .. }))
            .copied()
            .collect()
    };
    let (old_covers, new_covers) = (covers(before.0), covers(after.0));
    for (these, others) in [(&old_covers, &new_covers), (&new_covers, &old_covers)] {
        for cover in these.iter().filter(|cover| !others.contains(cover)) {
            if let RenderItem::Cover { rect: cover, .. } = cover {
                damage.push(rect(*cover));
            }
        }
    }
    damage
}

/// How many pixels `rects` cover together, each counted once however many hold it: over the
/// grid their edges make, the cells some rectangle holds.
fn union_area(rects: &[oxyos_gpu::Rect]) -> u64 {
    let edges = |start: fn(&oxyos_gpu::Rect) -> i64, length: fn(&oxyos_gpu::Rect) -> i64| {
        let mut edges: Vec<i64> = rects
            .iter()
            .flat_map(|rect| [start(rect), start(rect) + length(rect)])
            .collect();
        edges.sort_unstable();
        edges.dedup();
        edges
    };
    let xs = edges(|r| i64::from(r.x), |r| i64::from(r.width));
    let ys = edges(|r| i64::from(r.y), |r| i64::from(r.height));
    let mut area = 0;
    for x in xs.windows(2) {
        for y in ys.windows(2) {
            let held = rects.iter().any(|r| {
                i64::from(r.x) <= x[0]
                    && x[1] <= i64::from(r.x) + i64::from(r.width)
                    && i64::from(r.y) <= y[0]
                    && y[1] <= i64::from(r.y) + i64::from(r.height)
            });
            if held {
                area += ((x[1] - x[0]) * (y[1] - y[0])) as u64;
            }
        }
    }
    area
}

/// A client's pointer image (ADR 0029), read from its buffer when it set it.
#[derive(Clone, Debug)]
pub struct ClientPointer {
    /// The scene's number for it (Scene::pointer_image).
    pub id: u64,
    pub image: oxyos_kms::PointerImage,
    /// The scale it was drawn for, in 120ths.
    pub scale_120: u32,
}

/// The pixels of a pointer image from a client's buffer (premultiplied ARGB8888), with its
/// hotspot; None if the buffer cannot be read.
fn read_pointer_image(
    shared: &SharedBuffer,
    hotspot: (u32, u32),
) -> Option<oxyos_kms::PointerImage> {
    let extent = shared.layout().extent;
    let mut row = vec![0u8; extent.width as usize * 4];
    let mut pixels = Vec::with_capacity((extent.width * extent.height) as usize);
    for y in 0..extent.height {
        shared.read_row(y, &mut row).ok()?;
        pixels.extend(
            row.chunks_exact(4)
                .map(|pixel| u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]])),
        );
    }
    Some(oxyos_kms::PointerImage {
        width: extent.width,
        height: extent.height,
        pixels,
        hotspot,
    })
}

/// `image`, drawn for `from_120`, as a display at `to_120` shows it: each of its pixels the
/// nearest of the image's, the hotspot moved with them. The same image at its own scale.
pub fn pointer_image_at(
    image: &oxyos_kms::PointerImage,
    from_120: u32,
    to_120: u32,
) -> oxyos_kms::PointerImage {
    if from_120 == to_120 || from_120 == 0 {
        return image.clone();
    }
    let scaled = |length: u32| {
        ((u64::from(length) * u64::from(to_120) + u64::from(from_120) / 2) / u64::from(from_120))
            .max(1) as u32
    };
    let (width, height) = (scaled(image.width), scaled(image.height));
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let source_y = (u64::from(y) * u64::from(image.height) / u64::from(height)) as u32;
        for x in 0..width {
            let source_x = (u64::from(x) * u64::from(image.width) / u64::from(width)) as u32;
            pixels.push(image.pixels[(source_y * image.width + source_x) as usize]);
        }
    }
    let hotspot = (
        (u64::from(image.hotspot.0) * u64::from(width) / u64::from(image.width)) as u32,
        (u64::from(image.hotspot.1) * u64::from(height) / u64::from(image.height)) as u32,
    );
    oxyos_kms::PointerImage {
        width,
        height,
        pixels,
        hotspot: (hotspot.0.min(width - 1), hotspot.1.min(height - 1)),
    }
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
        }
        | Event::KeyText {
            surface, pressed, ..
        } => (surface, if pressed { "key.press" } else { "key.release" }),
        Event::Text { surface, .. } => (surface, "text"),
        Event::Preedit { surface, .. } => (surface, "preedit"),
        // A transaction counts once, whatever parts it has.
        Event::TextInputDone { surface, .. } => (surface, "text_input"),
        Event::Scroll { surface, .. } => (surface, "scroll"),
        Event::Hold { surface, phase, .. } => (
            surface,
            match phase {
                GesturePhase::Begin => "hold.begin",
                GesturePhase::End => "hold.end",
                GesturePhase::Cancel => "hold.cancel",
                GesturePhase::Update => return None,
            },
        ),
        Event::PopupDone { surface, .. } => (surface, "popup.done"),
        Event::Focus { surface, focused } => {
            (surface, if focused { "focus.in" } else { "focus.out" })
        }
        // A command of its menus chosen in the shell's bar (ADR 0021).
        Event::MenuItemActivated { surface, .. } => (surface, "menu.activated"),
        // Asked to close a window: the shell's, the close bubble's or Super+W's (ADR 0024).
        Event::CloseRequested { surface } => (surface, "close.requested"),
        // The person pressed one of its notifications, or an action of one (ADR 0026): of no
        // surface.
        Event::NotificationResponse { .. } => (0, "notification.response"),
        _ => return None,
    })
}

/// Removes the private socket directory on every exit, including errors and deadlines.
#[cfg(test)]
mod tests {
    use super::*;

    fn screen(rect: Rect, scale_120: u32) -> Screen {
        Screen::new(
            &Output {
                id: 2,
                rect,
                scale_120,
                refresh_mhz: 60_000,
            },
            None,
        )
    }

    #[test]
    fn a_paced_screen_shows_a_frame_at_its_next_vblank_and_the_fastest_screen_sets_the_pace() {
        let mut screen = screen(Rect::new(0, 0, 64, 64), 120);
        screen.refresh_mhz = 120_000;
        screen.vblank_origin = Duration::from_secs(5);
        let ms = |ms: f64| Duration::from_secs_f64(5.0 + ms / 1e3);
        assert_eq!(screen.refresh_period(), Duration::from_nanos(8_333_333));
        // A frame composed just after a vblank waits for the next; one composed on it, too.
        assert_eq!(
            screen.next_vblank(ms(0.1)),
            Duration::from_nanos(5_008_333_333)
        );
        assert_eq!(
            screen.next_vblank(ms(0.0)),
            Duration::from_nanos(5_008_333_333)
        );
        assert_eq!(
            screen.next_vblank(ms(9.0)),
            Duration::from_nanos(5_016_666_666)
        );
        // The fastest of the screens is what transitions and a slow turn are measured by.
        let mut compositor = Compositor::new(Scene::new(), 0);
        assert_eq!(compositor.fastest_refresh(), None);
        compositor
            .screens
            .push(self::screen(Rect::new(0, 0, 64, 64), 120));
        compositor.screens.push(screen);
        assert_eq!(
            compositor.fastest_refresh(),
            Some(Duration::from_nanos(8_333_333))
        );
        assert_eq!(refresh_period(143_998), Duration::from_nanos(6_944_540));
    }

    #[test]
    fn pixels_drawn_for_the_screen_s_scale_are_drawn_one_to_one() {
        // 961 logical at 1.25 is 1202 pixels (ADR 0001's ceil); its edges, rounded, are 1201
        // apart. Drawn from them it would be squeezed a pixel; it is drawn 1202 wide.
        let panel = screen(Rect::new(0, 0, 1280, 800), 150);
        assert_eq!(panel.place(1, 0, 961, 601), (1, 0, 1203, 751));
        assert_eq!(
            panel.place_pixels(1, 0, 961, 601, (1202, 752)),
            (1, 0, 1203, 752)
        );
        assert_eq!(panel.place(2, 0, 961, 600), (3, 0, 1204, 750));
        assert_eq!(
            panel.place_pixels(2, 0, 961, 600, (1202, 750)),
            (3, 0, 1205, 750)
        );
        // Pixels of another size (a transition drawing the window smaller) fill the rectangle.
        assert_eq!(
            panel.place_pixels(2, 0, 961, 600, (900, 600)),
            (3, 0, 1204, 750)
        );
    }

    #[test]
    fn a_screen_places_the_global_space_at_its_scale() {
        // 400x300 logical at 1.5, right of a 640-wide output: 600x450 pixels.
        let right = screen(Rect::new(640, 0, 400, 300), 180);
        assert_eq!((right.width, right.height), (600, 450));
        // Edges round to the nearest pixel, so rectangles that meet still meet.
        assert_eq!(right.place(650, 10, 20, 20), (15, 15, 45, 45));
        assert_eq!(right.place(651, 1, 1, 1), (17, 2, 18, 3));
        assert_eq!(right.place(652, 1, 1, 1), (18, 2, 20, 3));
        // Damage covers every pixel showing any of it, and nothing off the screen.
        let rect = |x, y, width, height| oxyos_gpu::Rect {
            x,
            y,
            width,
            height,
        };
        assert_eq!(right.covering(&rect(641, 1, 1, 1)), Some(rect(1, 1, 2, 2)));
        assert_eq!(
            right.covering(&rect(600, -10, 60, 20)),
            Some(rect(0, 0, 30, 15))
        );
        assert_eq!(right.covering(&rect(0, 0, 640, 480)), None);
        assert_eq!(
            right.covering(&rect(1000, 290, 100, 100)),
            Some(rect(540, 435, 60, 15))
        );
        // A surface's damage reaches every pixel whose filtered sample reads it, at the size the
        // surface is drawn on the screen: a 20x20 buffer at 20x20 logical is 30x30 pixels here,
        // and its pixel 10 is read by pixels 14 to 16 of them (29 to 31 of the screen; pixel 14's
        // centre falls between buffer pixels 9's and 10's), which the logical damage, 660 to 661
        // (pixels 30 and 31), would miss.
        let surface = Rect::new(650, 10, 20, 20);
        let damage = right.damage_on(&Rect::new(10, 10, 1, 1), surface, Size::new(20, 20));
        assert_eq!(damage, rect(659, 19, 3, 3));
        assert_eq!(right.covering(&damage), Some(rect(28, 28, 5, 5)));
        // A point is on the screen it is in, in its pixels.
        assert_eq!(right.on_display((640, 0)), Some((0, 0)));
        assert_eq!(right.on_display((1039, 299)), Some((599, 449)));
        assert_eq!(right.on_display((639, 0)), None);
        // At scale 1 the global space is the screen's pixels.
        let left = screen(Rect::new(0, 0, 640, 480), 120);
        assert_eq!(left.place(-3, 7, 10, 10), (-3, 7, 7, 17));
        assert_eq!(
            left.covering(&rect(630, 470, 50, 50)),
            Some(rect(630, 470, 10, 10))
        );
    }

    #[test]
    fn a_layer_is_placed_with_its_corners_and_shadow_at_the_screens_scale() {
        let right = screen(Rect::new(640, 0, 400, 300), 180);
        let rect = |x, y, width, height| oxyos_gpu::Rect {
            x,
            y,
            width,
            height,
        };
        let profile = [0u8; 4];
        let shadow = oxyos_gpu::Shadow {
            shape: rect(2, 4, 10, 10),
            radius: 4,
            clip: rect(5, 5, 10, 10),
            clip_radius: 3,
            scale: 256,
        };
        let layer = oxyos_gpu::Layer {
            x: 650,
            y: 10,
            width: 20,
            height: 20,
            extent: (1, 1),
            paint: oxyos_gpu::Paint::Shadow(&profile, shadow),
            source: None,
            changed: None,
            opacity: 200,
            radius: 6,
            filter: oxyos_gpu::Filter::Box,
            clip: None,
        };
        let placed = right.layer(&layer).expect("on the screen");
        // A clip is placed as the layer is: one that is the layer's rectangle is the placed one.
        let clipped = oxyos_gpu::Layer {
            clip: Some(rect(650, 10, 20, 20)),
            ..layer
        };
        let clipped = right.layer(&clipped).expect("on the screen");
        assert_eq!(
            clipped.clip,
            Some(rect(clipped.x, clipped.y, clipped.width, clipped.height))
        );
        assert_eq!(
            (
                placed.x,
                placed.y,
                placed.width,
                placed.height,
                placed.radius
            ),
            (15, 15, 30, 30, 9)
        );
        let oxyos_gpu::Paint::Shadow(_, at) = placed.paint else {
            panic!("a shadow");
        };
        // Its rectangles as the global space's are placed, relative to the layer as placed; its
        // radii and its size at the scale.
        assert_eq!(
            at,
            oxyos_gpu::Shadow {
                shape: rect(3, 6, 15, 15),
                radius: 6,
                clip: rect(8, 8, 15, 15),
                clip_radius: 5,
                scale: 384,
            }
        );
        assert_eq!(placed.opacity, 200);
        // At scale 1 a layer is as it was; one off the screen is none.
        let left = screen(Rect::new(0, 0, 640, 480), 120);
        let same = left.layer(&oxyos_gpu::Layer { x: 600, ..layer }).unwrap();
        assert_eq!((same.x, same.y, same.radius), (600, 10, 6));
        assert!(matches!(same.paint, oxyos_gpu::Paint::Shadow(_, at) if at == shadow));
        assert!(left.layer(&oxyos_gpu::Layer { x: 640, ..layer }).is_none());
    }

    #[test]
    fn a_turn_waits_for_the_idle_locks_deadline_and_never_wakes_for_it_otherwise() {
        let (minute, second) = (Duration::from_secs(60), Duration::from_secs(1));
        let output = Rect::new(0, 0, 64, 64);
        let mut scene = Scene::new();
        scene
            .add_output(oxyos_compositor_core::Output {
                id: 1,
                rect: output,
                scale_120: 120,
                refresh_mhz: 60_000,
            })
            .unwrap();
        let mut compositor = Compositor::new(scene, 0);
        // A turn with nothing else to wake for waits an hour.
        let hour = 60 * minute;
        compositor.idle_wait = hour;
        compositor.clock = Some(10 * minute);
        // Nothing set it (the development compositor's own run) or 0: no deadline.
        assert_eq!(compositor.next_wake(), hour);
        compositor.set_lock_after(0);
        assert_eq!(compositor.next_wake(), hour);
        compositor.set_lock_after(1);
        assert_eq!(compositor.next_wake(), minute);
        compositor.clock = Some(10 * minute + 45 * second);
        compositor.follow_idle();
        assert_eq!(compositor.next_wake(), 15 * second);
        // Input arms it again: a minute from it.
        compositor.scene.pointer_motion(3, 3);
        compositor.follow_idle();
        assert_eq!(compositor.next_wake(), minute);
        assert!(!compositor.scene.is_locked());
        // At the deadline the session locks; locked and idle, nothing is left to wake for.
        compositor.clock = Some(11 * minute + 45 * second);
        compositor.follow_idle();
        assert!(compositor.scene.is_locked());
        assert_eq!(compositor.next_wake(), hour);
        // Idle held back (a video playing): no deadline while it is.
        compositor.scene.key(30, true);
        compositor.follow_idle();
        assert_eq!(compositor.next_wake(), minute);
        compositor.idle.set_held(true, 11 * minute + 45 * second);
        assert_eq!(compositor.next_wake(), hour);
    }

    #[test]
    fn a_waker_wakes_a_turn_that_would_wait_an_hour() {
        use std::io::Write;
        let mut scene = Scene::new();
        scene
            .add_output(oxyos_compositor_core::Output {
                id: 1,
                rect: Rect::new(0, 0, 64, 64),
                scale_120: 120,
                refresh_mhz: 60_000,
            })
            .unwrap();
        let mut compositor = Compositor::new(scene, 0);
        let path = std::env::temp_dir().join(format!("oxyos-waker-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = Listener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let (read, mut write) = std::os::unix::net::UnixStream::pair().unwrap();
        compositor.wakers.push(read.into());
        // With nothing to wake it, a turn waits as long as it may.
        compositor.serve(&listener).unwrap();
        compositor.idle_wait = Duration::from_millis(200);
        let started = Instant::now();
        compositor.serve(&listener).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(150));
        // A waker readable (a child of the session's exited) ends the wait at once.
        compositor.idle_wait = Duration::from_secs(3600);
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            write.write_all(&[1]).unwrap();
            write
        });
        let started = Instant::now();
        compositor.serve(&listener).unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        drop(writer.join().unwrap());
        std::fs::remove_file(&path).unwrap();
    }

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
        let none = Looks::new();
        super::output_damage(
            (before, &none),
            (after, &none),
            &window::Shadows::new(0),
            &|_, _, _| None,
        )
    }

    #[test]
    fn a_surfaces_damage_is_on_the_output_where_it_shows() {
        let before = [surface(1, 0, 1)];
        let after = [surface(1, 0, 2)];
        // The surface's own damage, not its whole rectangle.
        let drawn = |_: SurfaceKey, rect: Rect, extent: Size| {
            Some(vec![on_output(&Rect::new(2, 3, 4, 1), rect, extent)])
        };
        let none = Looks::new();
        assert_eq!(
            super::output_damage(
                (&before, &none),
                (&after, &none),
                &window::Shadows::new(0),
                &drawn
            ),
            [oxyos_gpu::Rect {
                x: 2,
                y: 3,
                width: 4,
                height: 1
            }]
        );
        // Where it is sampled: a 10x10 buffer at 15x15 (1.5x), pixels 3 to 5 are read by output
        // pixels 4 to 9 (pixel 9's centre is 6⅓ buffer pixels in, between pixels 5 and 6's
        // centres, so it reads pixel 5 too).
        let shown = Rect::new(100, 0, 15, 15);
        assert_eq!(
            on_output(&Rect::new(3, 3, 3, 3), shown, Size::new(10, 10)),
            oxyos_gpu::Rect {
                x: 104,
                y: 4,
                width: 6,
                height: 6
            }
        );
        // Drawn smaller, the output pixels covering them: a 20x20 buffer at 10x10, pixels 3 to 5
        // are within output pixels 1 and 2.
        assert_eq!(
            on_output(
                &Rect::new(3, 3, 3, 3),
                Rect::new(0, 0, 10, 10),
                Size::new(20, 20)
            ),
            oxyos_gpu::Rect {
                x: 1,
                y: 1,
                width: 2,
                height: 2
            }
        );
    }

    #[test]
    fn a_window_is_damaged_with_its_shadow_and_when_its_look_changes() {
        let shadows = window::Shadows::new(0);
        let list = [surface(1, 100, 1)];
        let window = SurfaceKey { client: 1, id: 1 };
        let rect = Rect::new(100, 0, 10, 10);
        let looks = |active| {
            Looks::from([(
                window,
                WindowLook {
                    active,
                    maximized: false,
                },
            )])
        };
        let damage = |before: (&[RenderItem], &Looks), after: (&[RenderItem], &Looks)| {
            super::output_damage(before, after, &shadows, &|_, _, _| None)
        };
        let (active, inactive) = (looks(true), looks(false));
        let with_shadow = |look: bool| {
            let look = WindowLook {
                active: look,
                maximized: false,
            };
            window::extent(look, rect, rect, &shadows)
        };
        // Nothing changed, nothing damaged.
        assert_eq!(damage((&list, &active), (&list, &active)), []);
        // Only its look: where both shadows are.
        assert_eq!(
            damage((&list, &active), (&list, &inactive)),
            [with_shadow(true), with_shadow(false)]
        );
        // Gone and new: with its shadow; a window full-screen (no look) or back, both.
        assert_eq!(
            damage((&list, &active), (&[], &Looks::new())),
            [with_shadow(true)]
        );
        assert_eq!(
            damage((&[], &Looks::new()), (&list, &inactive)),
            [with_shadow(false)]
        );
        assert_eq!(
            damage((&list, &Looks::new()), (&list, &active)),
            [at(100), with_shadow(true)]
        );
        // A new frame of it: its own rectangle, not its shadow.
        assert_eq!(
            damage((&list, &active), (&[surface(1, 100, 2)], &active)),
            [at(100)]
        );
    }

    #[test]
    fn overlapping_damage_is_counted_once() {
        let rect = |x, y, width, height| oxyos_gpu::Rect {
            x,
            y,
            width,
            height,
        };
        assert_eq!(union_area(&[]), 0);
        assert_eq!(union_area(&[rect(0, 0, 10, 10), rect(0, 0, 10, 10)]), 100);
        assert_eq!(union_area(&[rect(0, 0, 10, 10), rect(5, 5, 10, 10)]), 175);
        assert_eq!(union_area(&[rect(0, 0, 4, 4), rect(10, 10, 2, 3)]), 22);
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
