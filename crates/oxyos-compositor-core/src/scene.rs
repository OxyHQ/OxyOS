//! Global surface policy: roles, ownership, stacking, placement, input routing, popup grabs,
//! focus, the secure lock and the frame lifecycle. The scene never runs client code and never
//! waits on a client: every operation completes from compositor state alone.
//!
//! Policies (see `docs/popup-lifecycle.mdx`):
//! - Surface identifiers are connection-local, so a client can only name its own surfaces as
//!   parents; cross-client parenting is unrepresentable.
//! - A press or touch outside an active popup grab dismisses the whole chain and is consumed,
//!   together with its matching release; it never reaches the surface underneath.
//! - Removing a parent dismisses its children (`PopupDone`) and leaves them inert until the client
//!   destroys them, so identifiers never change meaning behind the client's back.
//! - Buffers are released only when no displayed, queued or in-flight frame still refers to them;
//!   releases caused while a backend frame is in flight wait for that frame to end.

use crate::placement::{anchor_point, place};
use crate::presentation::{Error as PresentError, Frame, Presentation};
use oxyos_protocol::{
    BufferLayout, Clock, DismissReason, ErrorCode, Event, GesturePhase, Outcome, PanelEdge,
    PointerKind, PointerShape, Positioner, PreeditStyle, Rect, Request, ShellKind, Size, TextChunk,
    TouchKind, MAX_COORDINATE, MAX_DIMENSION,
};
use std::collections::{BTreeMap, BTreeSet};

pub type ClientId = u32;
pub type OutputId = u32;
/// A frame of the scene: of one output (each composes and flips on its own), or of all
/// (`None`, `begin_frame`).
type FrameOf = Option<OutputId>;

pub const MAX_CLIENTS: usize = 64;
pub const MAX_SURFACES_PER_CLIENT: usize = 64;
pub const MAX_POPUP_DEPTH: usize = 8;
pub const MAX_OUTPUTS: usize = 16;
pub const MAX_TOUCH_POINTS: usize = 10;
pub const MAX_BUFFERS_PER_CLIENT: usize = 128;
/// Media types an offer may name (ADR 0007).
pub const MAX_CLIPBOARD_TYPES: usize = 16;
/// Offers a client may have begun and not committed.
pub const MAX_CLIPBOARD_OFFERS: usize = 4;
/// The latest user inputs per client whose serials authorize clipboard requests.
pub const RECENT_INPUTS: usize = 16;
/// Output scale is expressed in 120ths: 120 = 1x, 180 = 1.5x.
pub const SCALE_DENOMINATOR: u32 = 120;
const MIN_VISIBLE: i64 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceKey {
    pub client: ClientId,
    pub id: u32,
}

/// Capabilities issued by the session authority from authenticated peer credentials. Protocol
/// requests can never add to them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Grants {
    pub shell: bool,
    pub lock: bool,
    pub shortcuts: bool,
    /// An assistive technology: reads every surface's accessibility snapshot and acts on
    /// nodes (ADR 0003).
    pub accessibility: bool,
    /// The input method (ADR 0008): serves the focused text field, receiving its state and the
    /// keys first. At most one connected client holds it.
    pub input_method: bool,
    /// The screen reader (ADR 0014): its keys (Ctrl+Alt with an arrow or Space) are taken from
    /// the keyboard and sent to it alone. At most one connected client holds it.
    pub screen_reader: bool,
    /// Settings (ADR 0019): changes the user's preferences (`SetPreference`) and is told every
    /// one as it is kept. Given from launch records, by [`Scene::grant`].
    pub settings: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Output {
    pub id: OutputId,
    /// Logical geometry in the global coordinate space.
    pub rect: Rect,
    pub scale_120: u32,
    pub refresh_mhz: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Toplevel,
    Dialog,
    Popup(Positioner),
    Shell {
        kind: ShellKind,
        edge: PanelEdge,
        exclusive: u32,
        output: OutputId,
    },
    Lock {
        output: OutputId,
    },
    /// The input method's candidates, next to the caret of the field it serves (ADR 0008).
    Candidates,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Layer {
    Background,
    Normal,
    Panel,
    Overlay,
    Notification,
    Candidates,
    Lock,
}

#[derive(Debug)]
struct Node {
    role: Role,
    parent: Option<SurfaceKey>,
    children: Vec<SurfaceKey>,
    /// Displayed origin: global for roots, relative to the parent otherwise.
    origin: (i32, i32),
    /// Displayed logical size.
    size: Size,
    /// Geometry of the latest configure, applied when a frame for it is displayed.
    pending: Option<(u64, (i32, i32), Size)>,
    /// The geometry an edge resize wants while a configure is unanswered (ADR 0016): one
    /// configure is in flight, and this one, the latest the pointer asked for, is sent once a
    /// frame for that one is displayed. Any other configure replaces it.
    wanted: Option<((i32, i32), Size)>,
    /// The geometry (and full-screen state) of the configure a frame being presented answers,
    /// kept when a newer configure replaces `pending` before that frame is on screen: the frame
    /// is drawn, and then shown, at the size it was drawn for, never scaled to another.
    answering: Option<(u64, (i32, i32), Size, bool)>,
    presentation: Presentation,
    scale_120: u32,
    output: OutputId,
    mapped: bool,
    hidden: bool,
    dismissed: bool,
    fullscreen: bool,
    /// Whether the latest configure was full-screen, and whether the frame displayed answers a
    /// full-screen one: what is drawn full-screen, as `pending` and `size` are what is drawn.
    pending_fullscreen: bool,
    shown_fullscreen: bool,
    restore: Option<((i32, i32), Size)>,
    /// A toplevel zoomed to fill the work area (ADR 0016), and the frame it had before.
    maximized: bool,
    unzoomed: Option<((i32, i32), Size)>,
    /// The sizes a toplevel can take.
    limits: window::Limits,
    /// Local input region; `None` accepts input everywhere, an empty rectangle nowhere.
    input_region: Option<Rect>,
    /// Damage requested for the next Submit, in buffer pixels.
    requested: Vec<Rect>,
    /// What changed in the frames submitted since one last began presenting, and the extent of
    /// the last one submitted.
    unshown: Option<SurfaceDamage>,
    last_extent: Option<Size>,
    /// What changed in the frame presenting (or last presented), relative to the one displayed
    /// before it.
    presenting_damage: SurfaceDamage,
    /// The frame its in-flight frame is presented in (`begin_frame_on`).
    presenting_in: FrameOf,
    /// A window's workspace (ADR 0009): the one active when it was made. Only a toplevel's
    /// counts; what is under it follows it.
    workspace: u32,
    /// The pointer's shape while it is over the surface.
    pointer_shape: PointerShape,
}

/// What changed in a surface's frame since the frame displayed before it, in buffer pixels
/// (docs/adr/0005-damage.mdx).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceDamage {
    Whole,
    Rects(Vec<Rect>),
}

impl SurfaceDamage {
    /// At most this many rectangles; more become the one holding them all.
    pub const MAX_RECTS: usize = 16;

    /// What changed in either.
    fn and(self, other: SurfaceDamage) -> SurfaceDamage {
        match (self, other) {
            (SurfaceDamage::Rects(mut these), SurfaceDamage::Rects(those)) => {
                these.extend(those);
                SurfaceDamage::Rects(bounded(these))
            }
            _ => SurfaceDamage::Whole,
        }
    }
}

/// `rects`, or past `SurfaceDamage::MAX_RECTS` the one rectangle holding them all.
fn bounded(rects: Vec<Rect>) -> Vec<Rect> {
    if rects.len() <= SurfaceDamage::MAX_RECTS {
        return rects;
    }
    let left = rects.iter().map(|r| i64::from(r.x)).min().unwrap_or(0);
    let top = rects.iter().map(|r| i64::from(r.y)).min().unwrap_or(0);
    let right = rects
        .iter()
        .map(|r| i64::from(r.x) + i64::from(r.width))
        .max()
        .unwrap_or(0);
    let bottom = rects
        .iter()
        .map(|r| i64::from(r.y) + i64::from(r.height))
        .max()
        .unwrap_or(0);
    vec![Rect::new(
        left as i32,
        top as i32,
        (right - left) as u32,
        (bottom - top) as u32,
    )]
}

impl Node {
    fn is_popup(&self) -> bool {
        matches!(self.role, Role::Popup(_))
    }

    /// The geometry the surface is going to have: what an edge resize wants next, else its
    /// latest configure's, else what is displayed.
    fn upcoming(&self) -> ((i32, i32), Size) {
        self.wanted.unwrap_or_else(|| {
            self.pending
                .map_or((self.origin, self.size), |(_, origin, size)| (origin, size))
        })
    }

    /// Content and geometry as displayed, or (`in_flight`) as they will be once the frame
    /// between `begin_frame` and `end_frame` completes.
    fn view(&self, in_flight: bool) -> Option<(Frame, (i32, i32), Size)> {
        if in_flight {
            if let Some(frame) = self.presentation.presenting() {
                let (origin, size) = match (self.pending, self.answering) {
                    (Some((serial, origin, size)), _) if serial == frame.configure => {
                        (origin, size)
                    }
                    (_, Some((serial, origin, size, _))) if serial == frame.configure => {
                        (origin, size)
                    }
                    _ => (self.origin, self.size),
                };
                return Some((frame, origin, size));
            }
        }
        self.presentation
            .displayed()
            .map(|frame| (frame, self.origin, self.size))
    }
}

/// A client buffer the transport has validated (file type, seals, size) and registered.
#[derive(Clone, Copy, Debug)]
struct BufferEntry {
    layout: BufferLayout,
    /// The client destroyed it while still held; it is retired once released.
    destroyed: bool,
}

/// What the clipboard holds: an offer its owner committed, and the name receivers know it by.
#[derive(Clone, Debug)]
struct Clipboard {
    owner: ClientId,
    offer: u32,
    types: Vec<TextChunk>,
    selection: u32,
}

#[derive(Clone, Copy, Debug)]
struct Press {
    serial: u64,
    client: ClientId,
    surface: SurfaceKey,
    pointer: bool,
}

#[derive(Clone, Copy, Debug)]
struct Move {
    surface: SurfaceKey,
    pointer: (i32, i32),
    origin: (i32, i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderItem {
    Surface {
        key: SurfaceKey,
        rect: Rect,
        frame: Frame,
    },
    /// Compositor-drawn opaque cover for an output without a ready lock surface.
    Cover { output: OutputId, rect: Rect },
    /// A window the shell's surface `surface` shows live (docs/adr/0022-window-views.mdx), drawn
    /// above that surface: the window's frame scaled into `rect` (its proportions kept), only
    /// within `clip` (within `rect` and the surface), its corners rounded by `radius`, at
    /// `opacity`. It takes no input: the surface under it does.
    View {
        surface: SurfaceKey,
        window: SurfaceKey,
        frame: Frame,
        rect: Rect,
        clip: Rect,
        radius: u32,
        opacity: u8,
    },
}

/// How a window is drawn (`Scene::window_look`): whether it is the active window
/// (`Scene::active_window`, the one told `Activated`) or one of its dialogs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowLook {
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub clients: usize,
    pub surfaces: usize,
    pub popups: usize,
    pub held_buffers: usize,
    pub deferred_releases: usize,
    pub grab_depth: usize,
    pub touch_points: usize,
    pub pending_events: usize,
    pub retired_buffers: usize,
    pub registered_buffers: usize,
}

#[derive(Debug, Default)]
pub struct Scene {
    clients: BTreeMap<ClientId, Grants>,
    nodes: BTreeMap<SurfaceKey, Node>,
    /// Root surfaces, bottom to top; kept sorted by layer.
    roots: Vec<SurfaceKey>,
    outputs: Vec<Output>,
    serial: u64,
    grab: Vec<SurfaceKey>,
    focus: Option<SurfaceKey>,
    focus_before_grab: Option<SurfaceKey>,
    pointer: (i32, i32),
    pointer_focus: Option<SurfaceKey>,
    capture: Option<SurfaceKey>,
    buttons_down: u32,
    swallowed_releases: u32,
    touches: BTreeMap<u32, Option<SurfaceKey>>,
    /// The surface a touchpad pinch going on goes to (the one under the pointer when it began),
    /// or none if it began over nothing or its surface went.
    pinch: Option<SurfaceKey>,
    last_press: Option<Press>,
    /// Serial of the latest user press, key or touch-down, delivered or consumed.
    last_user_serial: u64,
    /// Counts the person's input, of every kind and wherever it goes (`Scene::activity`).
    activity: u64,
    /// Per client: the serial at connect or of the latest user input delivered to it. A window
    /// mapped after the user interacted elsewhere does not take focus.
    activation: BTreeMap<ClientId, u64>,
    /// Per client: the serials of its latest user inputs (RECENT_INPUTS), oldest first.
    inputs: BTreeMap<ClientId, std::collections::VecDeque<u64>>,
    /// The clipboard (ADR 0007): an offer its owner committed; empty if none.
    clipboard: Option<Clipboard>,
    /// Offers begun and not committed, by client and the client's offer number: their types.
    offers: BTreeMap<(ClientId, u32), Vec<TextChunk>>,
    /// Names each clipboard content for receivers, never reused.
    selections: u32,
    /// The client last told what the clipboard holds, and the content it was told about.
    told: Option<(ClientId, Option<u32>)>,
    /// The owner that must answer the paste the request handled last (Scene::take_transfer).
    transfer: Option<ClientId>,
    moving: Option<Move>,
    /// A window the compositor is resizing by its edges, and the edges the pointer is beside
    /// when it is in a window's resize band (ADR 0016).
    resizing: Option<window::Resize>,
    resize_hover: Option<window::Edges>,
    /// The active window (ADR 0016).
    active: Option<SurfaceKey>,
    locked: bool,
    /// The system shortcuts' keys held and taken, and the application switcher (ADR 0024).
    system_keys: shortcuts::SystemKeys,
    /// How recently each window was active, and the windows hidden with their application.
    applications: shortcuts::Applications,
    events: Vec<(ClientId, Event)>,
    buffers: BTreeMap<(ClientId, u64), BufferEntry>,
    /// Buffers no consumer will read again and the client cannot reuse: the backend may drop
    /// their memory and file descriptors.
    retired: Vec<(ClientId, u64)>,
    /// The frames between `begin_frame` and `end_frame`: the scene's (`None`) or an output's
    /// (docs/adr/0018-several-displays.mdx), with the surfaces whose buffers each may read (those
    /// shown on its output; every one for the scene's).
    frames_in_flight: BTreeMap<FrameOf, Option<BTreeSet<SurfaceKey>>>,
    /// Releases waiting for the frames in flight when they were asked for that may still read
    /// the buffer.
    deferred: Vec<(SurfaceKey, u64, BTreeSet<FrameOf>)>,
    accessibility: BTreeMap<SurfaceKey, accessibility::SurfaceAccessibility>,
    /// What each surface with a text field told about it (ADR 0008).
    text_inputs: BTreeMap<SurfaceKey, text_input::SurfaceTextInput>,
    /// The input method, if one is connected.
    input_method: Option<input_method::InputMethod>,
    /// The workspaces and the windows the shell is told about (ADR 0009).
    workspaces: workspaces::Workspaces,
    shell_windows: shell::ShellWindows,
    /// The lock's password, given and checked (ADR 0010).
    authentication: lock::Authentication,
    /// Whether the keyboard's Caps Lock is on.
    caps_lock: bool,
    /// Applications the shell asked for (ADR 0013).
    launches: launch::Launches,
    /// The user's preferences as the program running the compositor told them, and the changes
    /// Settings asked for (ADR 0019).
    preferences: preferences::Preferences,
    /// The screen reader's keys held and taken (ADR 0014).
    reader_keys: reader::ReaderKeys,
    /// The hardware keys pressed and the Control Center's requests for the program, and the
    /// levels, battery, network and Bluetooth it said (ADR 0025).
    controls: controls::Controls,
    /// Surfaces arrived and gone since the compositor last asked (ADR 0011).
    appearances: Vec<appearances::Appearance>,
    /// Windows' menus and what the shell was told of the active one's (ADR 0021).
    menu_bars: menus::MenuBars,
    /// The windows each shell surface shows live (ADR 0022).
    window_views: BTreeMap<SurfaceKey, window_views::SurfaceViews>,
}

/// Largest logical size whose buffer at `scale_120` stays within `MAX_DIMENSION`. The
/// compositor configures this size (with the scale) instead of a buffer the client cannot submit.
fn fit(size: Size, scale_120: u32) -> Size {
    let limit = (u64::from(MAX_DIMENSION) * u64::from(SCALE_DENOMINATOR) / u64::from(scale_120))
        .max(1) as u32;
    Size::new(size.width.min(limit), size.height.min(limit))
}

/// Buffer size for a logical size that `fit` already made representable.
fn physical(size: Size, scale_120: u32) -> Size {
    let scale = |logical: u32| {
        (u64::from(logical) * u64::from(scale_120)).div_ceil(u64::from(SCALE_DENOMINATOR)) as u32
    };
    let extent = Size::new(scale(size.width), scale(size.height));
    debug_assert!(
        extent.is_valid(),
        "{size:?} at {scale_120} is not representable"
    );
    extent
}

fn layer_of(role: &Role) -> Layer {
    match role {
        Role::Shell { kind, .. } => match kind {
            ShellKind::Background => Layer::Background,
            ShellKind::Panel => Layer::Panel,
            ShellKind::Overlay => Layer::Overlay,
            ShellKind::Notification => Layer::Notification,
        },
        Role::Lock { .. } => Layer::Lock,
        Role::Candidates => Layer::Candidates,
        _ => Layer::Normal,
    }
}

fn to_fixed(value: i64) -> i32 {
    (value * 256).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- outputs -------------------------------------------------------------------------

    pub fn add_output(&mut self, output: Output) -> Result<(), ErrorCode> {
        if self.outputs.len() >= MAX_OUTPUTS {
            return Err(ErrorCode::QuotaExceeded);
        }
        if output.rect.is_empty()
            || output.rect.width > MAX_DIMENSION
            || output.rect.height > MAX_DIMENSION
            || !(SCALE_DENOMINATOR..=4 * SCALE_DENOMINATOR).contains(&output.scale_120)
            || self.outputs.iter().any(|o| o.id == output.id)
        {
            return Err(ErrorCode::InvalidGeometry);
        }
        self.outputs.push(output);
        // Windows left on no output (the last one went) come to this one, as to the one left
        // when theirs is removed.
        let orphans: Vec<SurfaceKey> = self
            .roots
            .iter()
            .copied()
            .filter(|key| {
                let node = &self.nodes[key];
                !node.dismissed
                    && matches!(node.role, Role::Toplevel | Role::Dialog)
                    && !self.outputs.iter().any(|o| o.id == node.output)
            })
            .collect();
        for key in orphans {
            self.rehome(key, output);
        }
        if !self.outputs.is_empty() {
            self.place_candidates();
            self.refresh_pointer_focus();
        }
        Ok(())
    }

    /// The outputs in their arrangement's order: the first is the primary, which has the dock
    /// and takes the windows of an output removed (docs/adr/0018-several-displays.mdx).
    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    /// The output a point of the global space is on, else the primary.
    pub fn output_at_point(&self, x: i32, y: i32) -> Option<Output> {
        self.output_at(i64::from(x), i64::from(y))
    }

    /// Moves an output to `origin` in the global space (the arrangement closing the gap one
    /// removed left): what is on it moves with it, as it is shown. Its windows keep their place
    /// on it, its shell and lock surfaces their edges.
    pub fn set_output_origin(&mut self, id: OutputId, origin: (i32, i32)) -> Result<(), ErrorCode> {
        let output = self
            .outputs
            .iter_mut()
            .find(|o| o.id == id)
            .ok_or(ErrorCode::InvalidGeometry)?;
        let delta = (
            i64::from(origin.0) - i64::from(output.rect.x),
            i64::from(origin.1) - i64::from(output.rect.y),
        );
        if delta == (0, 0) {
            return Ok(());
        }
        if i64::from(origin.0).abs() > i64::from(MAX_COORDINATE)
            || i64::from(origin.1).abs() > i64::from(MAX_COORDINATE)
        {
            return Err(ErrorCode::InvalidGeometry);
        }
        output.rect = Rect::new(origin.0, origin.1, output.rect.width, output.rect.height);
        let shift = |(x, y): (i32, i32)| {
            (
                (i64::from(x) + delta.0) as i32,
                (i64::from(y) + delta.1) as i32,
            )
        };
        // Roots alone: children are placed relative to them.
        let roots: Vec<SurfaceKey> = self
            .roots
            .iter()
            .copied()
            .filter(|key| self.nodes[key].output == id)
            .collect();
        for key in roots {
            let node = self.nodes.get_mut(&key).expect("a root");
            node.origin = shift(node.origin);
            node.pending = node
                .pending
                .map(|(serial, origin, size)| (serial, shift(origin), size));
            node.restore = node.restore.map(|(origin, size)| (shift(origin), size));
            self.reconstrain_children(key);
        }
        self.refresh_accessibility_geometry();
        self.place_candidates();
        self.refresh_pointer_focus();
        Ok(())
    }

    /// Popups and output-bound shell/lock surfaces on the removed output are dismissed; windows
    /// move to the first remaining output.
    pub fn remove_output(&mut self, id: OutputId) {
        let Some(index) = self.outputs.iter().position(|o| o.id == id) else {
            return;
        };
        self.outputs.remove(index);
        let affected: Vec<SurfaceKey> = self
            .nodes
            .iter()
            .filter(|(_, node)| node.output == id && !node.dismissed)
            .map(|(key, _)| *key)
            .collect();
        for key in affected {
            let Some(node) = self.nodes.get(&key) else {
                continue;
            };
            if node.dismissed {
                continue;
            }
            match node.role {
                Role::Popup(_) | Role::Shell { .. } | Role::Lock { .. } => {
                    self.dismiss(key, DismissReason::OutputRemoved);
                }
                // Placed again next to the caret, wherever it is now (below).
                Role::Candidates => {}
                Role::Toplevel | Role::Dialog => {
                    // To the primary; with none left, to the next output added.
                    if let Some(target) = self.outputs.first().copied() {
                        self.rehome(key, target);
                    }
                }
            }
        }
        self.place_candidates();
        self.refresh_pointer_focus();
    }

    /// Moves a window of an output gone to `target`. Windows on a removed output are not visible
    /// anywhere; moving them is a compositor-owned move, applied at once like a drag, so their
    /// last content shows on `target` while new buffers are prepared.
    fn rehome(&mut self, key: SurfaceKey, target: Output) {
        let Some(node) = self.nodes.get(&key) else {
            return;
        };
        let fullscreen = node.fullscreen;
        let mut size = node.upcoming().1;
        if node.parent.is_none() {
            let origin = if fullscreen {
                size = Size::new(target.rect.width, target.rect.height);
                (target.rect.x, target.rect.y)
            } else {
                self.centered(size, target.id)
            };
            let restore = node
                .restore
                .map(|(_, restored)| (self.centered(restored, target.id), restored));
            let node = self.nodes.get_mut(&key).expect("present");
            node.origin = origin;
            node.restore = restore;
            node.pending = node.pending.map(|(serial, _, s)| (serial, origin, s));
            node.answering = node
                .answering
                .map(|(serial, _, s, fullscreen)| (serial, origin, s, fullscreen));
        }
        self.nodes.get_mut(&key).expect("present").output = target.id;
        self.reconfigure(key, None, size);
        self.reconstrain_children(key);
    }

    /// Logical geometry is unchanged; affected surfaces are asked for buffers at the new scale
    /// and keep showing their current content until those are ready.
    pub fn set_output_scale(&mut self, id: OutputId, scale_120: u32) -> Result<(), ErrorCode> {
        if !(SCALE_DENOMINATOR..=4 * SCALE_DENOMINATOR).contains(&scale_120) {
            return Err(ErrorCode::InvalidGeometry);
        }
        let output = self
            .outputs
            .iter_mut()
            .find(|o| o.id == id)
            .ok_or(ErrorCode::InvalidGeometry)?;
        output.scale_120 = scale_120;
        let affected: Vec<SurfaceKey> = self
            .nodes
            .iter()
            .filter(|(_, node)| node.output == id && !node.dismissed)
            .map(|(key, _)| *key)
            .collect();
        for key in affected {
            let (origin, size) = self.nodes[&key].upcoming();
            self.reconfigure(key, Some(origin), size);
        }
        Ok(())
    }

    fn output(&self, id: OutputId) -> Option<Output> {
        self.outputs.iter().copied().find(|o| o.id == id)
    }

    fn output_at(&self, x: i64, y: i64) -> Option<Output> {
        self.outputs
            .iter()
            .copied()
            .find(|o| {
                x >= i64::from(o.rect.x)
                    && x < o.rect.right()
                    && y >= i64::from(o.rect.y)
                    && y < o.rect.bottom()
            })
            .or_else(|| self.outputs.first().copied())
    }

    /// Output area minus the exclusive zones of shell panels on it.
    pub fn work_area(&self, id: OutputId) -> Option<Rect> {
        let output = self.output(id)?;
        let mut area = output.rect;
        for node in self.nodes.values() {
            let Role::Shell {
                kind: ShellKind::Panel,
                edge,
                exclusive,
                output,
            } = node.role
            else {
                continue;
            };
            if output != id || node.dismissed || exclusive == 0 {
                continue;
            }
            let exclusive = exclusive.min(area.height / 2).min(area.width / 2);
            match edge {
                PanelEdge::Top => {
                    area.y += exclusive as i32;
                    area.height -= exclusive;
                }
                PanelEdge::Bottom => area.height -= exclusive,
                PanelEdge::Left => {
                    area.x += exclusive as i32;
                    area.width -= exclusive;
                }
                PanelEdge::Right => area.width -= exclusive,
                PanelEdge::None => {}
            }
        }
        Some(area)
    }

    fn centered(&self, size: Size, output: OutputId) -> (i32, i32) {
        let Some(area) = self.work_area(output) else {
            return (0, 0);
        };
        let x = i64::from(area.x) + (i64::from(area.width) - i64::from(size.width)) / 2;
        let y = i64::from(area.y) + (i64::from(area.height) - i64::from(size.height)) / 2;
        (
            x.max(i64::from(area.x)) as i32,
            y.max(i64::from(area.y)) as i32,
        )
    }

    /// Where a new window opens: centred in its output's work area or, when a window shown on
    /// the active workspace already sits exactly there, a step down and to the right of it, as a
    /// Mac cascades new windows, so one never opens exactly over another and hides it. A step
    /// that would leave the work area starts again from its top-left corner.
    fn placed(&self, size: Size, output: OutputId) -> (i32, i32) {
        const STEP: i32 = 24;
        let mut origin = self.centered(size, output);
        let Some(area) = self.work_area(output) else {
            return origin;
        };
        let taken = |at: (i32, i32)| {
            self.roots.iter().any(|root| {
                let node = &self.nodes[root];
                node.role == Role::Toplevel
                    && !node.dismissed
                    && !node.hidden
                    && node.output == output
                    && node.workspace == self.workspaces.active
                    && node.pending.map_or(node.origin, |(_, origin, _)| origin) == at
            })
        };
        let fits = |at: (i32, i32)| {
            i64::from(at.0) + i64::from(size.width) <= area.right()
                && i64::from(at.1) + i64::from(size.height) <= area.bottom()
        };
        // Bounded: a work area holds only so many steps.
        for _ in 0..64 {
            if !taken(origin) {
                break;
            }
            let next = (origin.0 + STEP, origin.1 + STEP);
            origin = if fits(next) { next } else { (area.x, area.y) };
        }
        origin
    }

    // ---- clients -------------------------------------------------------------------------

    pub fn connect(&mut self, client: ClientId, grants: Grants) -> Result<(), ErrorCode> {
        if self.clients.contains_key(&client) {
            return Err(ErrorCode::SurfaceExists);
        }
        if self.clients.len() >= MAX_CLIENTS {
            return Err(ErrorCode::QuotaExceeded);
        }
        self.check_grants(grants)?;
        self.clients.insert(client, Grants::default());
        self.activation.insert(client, self.serial);
        // Every client draws as the user prefers from its first frame.
        self.tell_drawing(client);
        self.grant(client, grants)
    }

    /// Whether `grants` can be given now: the grants one client at a time holds are not held.
    fn check_grants(&self, grants: Grants) -> Result<(), ErrorCode> {
        // One input method at a time: the session chooses it.
        if grants.input_method && self.input_method.is_some() {
            return Err(ErrorCode::Unauthorized);
        }
        // One screen reader at a time: the session starts it.
        if grants.screen_reader && self.screen_reader().is_some() {
            return Err(ErrorCode::Unauthorized);
        }
        Ok(())
    }

    /// Adds capabilities to a connected client: the counterpart of [`Scene::revoke`]. Only the
    /// session authority, the program running the compositor, calls it, from what it knows of
    /// the client's process (its launch record, ADR 0013); no request can. A grant one client at
    /// a time holds is refused while another holds it (`Unauthorized`), and nothing is given.
    pub fn grant(&mut self, client: ClientId, granted: Grants) -> Result<(), ErrorCode> {
        let Some(&held) = self.clients.get(&client) else {
            return Ok(());
        };
        let new = Grants {
            shell: granted.shell && !held.shell,
            lock: granted.lock && !held.lock,
            shortcuts: granted.shortcuts && !held.shortcuts,
            accessibility: granted.accessibility && !held.accessibility,
            input_method: granted.input_method && !held.input_method,
            screen_reader: granted.screen_reader && !held.screen_reader,
            settings: granted.settings && !held.settings,
        };
        self.check_grants(new)?;
        let grants = self.clients.get_mut(&client).expect("connected");
        grants.shell |= new.shell;
        grants.lock |= new.lock;
        grants.shortcuts |= new.shortcuts;
        grants.accessibility |= new.accessibility;
        grants.input_method |= new.input_method;
        grants.screen_reader |= new.screen_reader;
        grants.settings |= new.settings;
        if new.accessibility {
            self.replay_accessibility(client);
        }
        if new.input_method {
            self.input_method = Some(input_method::InputMethod::new(client));
            self.refresh_input_method();
        }
        // A lock screen that connects while locked (the shell started again) makes its surfaces.
        if new.lock && self.locked {
            self.emit(client, Event::LockState { locked: true });
        }
        if new.settings {
            self.tell_preferences(client);
        }
        if new.shell {
            self.tell_controls(client);
        }
        Ok(())
    }

    /// Removes every resource of the client. Its buffers are retired for the backend; no event
    /// is addressed to it any more.
    pub fn disconnect(&mut self, client: ClientId) {
        if self.clients.remove(&client).is_none() {
            return;
        }
        // The keys an input method held go on to the field before anything else changes.
        self.forget_input_method(client);
        self.forget_shell_client(client);
        self.forget_switcher(client);
        self.forget_menu_bar_client(client);
        self.forget_launches(client);
        self.forget_preference_changes(client);
        // The clipboard is its owner's offer: it empties when the owner goes, before focus moves
        // on, so the client focus reaches is told once.
        self.inputs.remove(&client);
        self.offers.retain(|(owner, _), _| *owner != client);
        if self
            .clipboard
            .as_ref()
            .is_some_and(|clipboard| clipboard.owner == client)
        {
            self.clipboard = None;
            self.tell_clipboard();
        }
        if self.told.is_some_and(|(told, _)| told == client) {
            self.told = None;
        }
        let keys: Vec<SurfaceKey> = self
            .nodes
            .keys()
            .copied()
            .filter(|key| key.client == client)
            .collect();
        for key in keys.iter().rev() {
            self.remove_node(*key);
        }
        let owned: Vec<u64> = self
            .buffers
            .range((client, 0)..=(client, u64::MAX))
            .map(|(&(_, buffer), _)| buffer)
            .collect();
        for buffer in owned {
            self.buffers.remove(&(client, buffer));
            // Buffers still read by an in-flight frame are retired when it ends.
            if !self
                .deferred
                .iter()
                .any(|(k, b, _)| k.client == client && *b == buffer)
            {
                self.retired.push((client, buffer));
            }
        }
        self.events.retain(|(target, _)| *target != client);
        self.activation.remove(&client);
        if self.last_press.is_some_and(|press| press.client == client) {
            self.last_press = None;
        }
        self.refresh_pointer_focus();
    }

    /// Withdraws capabilities; surfaces whose role needed them are dismissed.
    pub fn revoke(&mut self, client: ClientId, revoked: Grants) {
        let Some(grants) = self.clients.get_mut(&client) else {
            return;
        };
        grants.shell &= !revoked.shell;
        grants.lock &= !revoked.lock;
        grants.shortcuts &= !revoked.shortcuts;
        grants.accessibility &= !revoked.accessibility;
        grants.input_method &= !revoked.input_method;
        grants.screen_reader &= !revoked.screen_reader;
        grants.settings &= !revoked.settings;
        if revoked.shell || revoked.shortcuts {
            self.forget_switcher(client);
        }
        if revoked.input_method {
            self.forget_input_method(client);
        }
        if revoked.settings {
            self.forget_preference_changes(client);
        }
        let keys: Vec<SurfaceKey> = self
            .nodes
            .iter()
            .filter(|(key, node)| {
                key.client == client
                    && !node.dismissed
                    && match node.role {
                        Role::Shell { .. } => revoked.shell,
                        Role::Lock { .. } => revoked.lock,
                        _ => false,
                    }
            })
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            self.dismiss(key, DismissReason::AuthorizationRevoked);
        }
    }

    // ---- requests ------------------------------------------------------------------------

    /// Applies one request. An error is a protocol violation: the caller sends `ProtocolError`
    /// and disconnects the client. Valid but unsatisfiable requests (a denied grab, a stale
    /// frame) are answered with events instead.
    pub fn handle(&mut self, client: ClientId, request: Request) -> Result<(), ErrorCode> {
        let grants = *self.clients.get(&client).ok_or(ErrorCode::InvalidOrder)?;
        let key = |id| SurfaceKey { client, id };
        match request {
            Request::Hello { .. } => Err(ErrorCode::InvalidOrder),
            Request::Goodbye {} => {
                self.disconnect(client);
                Ok(())
            }
            Request::CreateInputCandidates { surface, size } => {
                self.create_candidates(client, key(surface), size)
            }
            Request::CreateToplevel { surface, size } => {
                self.check_new(key(surface))?;
                let output = self
                    .output_at(i64::from(self.pointer.0), i64::from(self.pointer.1))
                    .map_or(0, |o| o.id);
                let origin = self.placed(size, output);
                self.insert(key(surface), Role::Toplevel, None, origin, size, output);
                self.number_window(key(surface));
                Ok(())
            }
            Request::CreateDialog {
                surface,
                parent,
                size,
            } => {
                self.check_new(key(surface))?;
                let parent_node = self.live(key(parent)).ok_or(ErrorCode::InvalidParent)?;
                if parent_node.role != Role::Toplevel {
                    return Err(ErrorCode::InvalidParent);
                }
                let origin = (
                    ((i64::from(parent_node.size.width) - i64::from(size.width)) / 2) as i32,
                    ((i64::from(parent_node.size.height) - i64::from(size.height)) / 2) as i32,
                );
                let output = parent_node.output;
                self.insert(
                    key(surface),
                    Role::Dialog,
                    Some(key(parent)),
                    origin,
                    size,
                    output,
                );
                Ok(())
            }
            Request::CreatePopup {
                surface,
                parent,
                positioner,
            } => self.create_popup(key(surface), key(parent), positioner),
            Request::CreateShellSurface {
                surface,
                kind,
                edge,
                output,
                size,
                exclusive,
            } => {
                if !grants.shell {
                    return Err(ErrorCode::Unauthorized);
                }
                self.check_new(key(surface))?;
                let target = self.output(output).ok_or(ErrorCode::InvalidGeometry)?;
                let (origin, size) = Self::shell_geometry(kind, edge, size, target.rect);
                let role = Role::Shell {
                    kind,
                    edge,
                    exclusive: if kind == ShellKind::Panel {
                        exclusive
                    } else {
                        0
                    },
                    output,
                };
                self.insert(key(surface), role, None, origin, size, output);
                Ok(())
            }
            Request::CreateLockSurface { surface, output } => {
                if !grants.lock {
                    return Err(ErrorCode::Unauthorized);
                }
                self.check_new(key(surface))?;
                let target = self.output(output).ok_or(ErrorCode::InvalidGeometry)?;
                if self
                    .nodes
                    .values()
                    .any(|node| !node.dismissed && node.role == Role::Lock { output })
                {
                    return Err(ErrorCode::InvalidRole);
                }
                let size = Size::new(target.rect.width, target.rect.height);
                self.insert(
                    key(surface),
                    Role::Lock { output },
                    None,
                    (target.rect.x, target.rect.y),
                    size,
                    output,
                );
                Ok(())
            }
            Request::GrabPopup { surface, serial } => self.grab_popup(key(surface), serial),
            Request::Reposition {
                surface,
                positioner,
            } => {
                let node = self
                    .nodes
                    .get(&key(surface))
                    .ok_or(ErrorCode::UnknownSurface)?;
                if !node.is_popup() {
                    return Err(ErrorCode::InvalidRole);
                }
                if node.dismissed {
                    return Ok(());
                }
                self.validate_anchor(node.parent.expect("popups have parents"), &positioner)?;
                self.nodes.get_mut(&key(surface)).expect("present").role = Role::Popup(positioner);
                self.reposition_popup(key(surface));
                Ok(())
            }
            Request::ClipboardOffer { offer, serial } => {
                // Only with a user input of its own: a background client cannot offer.
                if !self.recent_input(client, serial) {
                    return Ok(());
                }
                let begun = self.offers.range((client, 0)..=(client, u32::MAX)).count();
                if begun >= MAX_CLIPBOARD_OFFERS && !self.offers.contains_key(&(client, offer)) {
                    return Err(ErrorCode::QuotaExceeded);
                }
                self.offers.insert((client, offer), Vec::new());
                Ok(())
            }
            Request::ClipboardType { offer, mime } => {
                // An offer that was refused takes no types.
                let Some(types) = self.offers.get_mut(&(client, offer)) else {
                    return Ok(());
                };
                if types.len() >= MAX_CLIPBOARD_TYPES {
                    return Err(ErrorCode::QuotaExceeded);
                }
                if !types.contains(&mime) {
                    types.push(mime);
                }
                Ok(())
            }
            Request::ClipboardCommit { offer } => {
                let types = self.offers.remove(&(client, offer));
                // The clipboard changes only for the client the user is typing into.
                if let Some(types) = types.filter(|types| !types.is_empty()) {
                    if self.focused_client() == Some(client) {
                        self.selections += 1;
                        self.clipboard = Some(Clipboard {
                            owner: client,
                            offer,
                            types,
                            selection: self.selections,
                        });
                        self.tell_clipboard();
                    }
                }
                Ok(())
            }
            Request::ClipboardClear { serial } => {
                let owner = self.clipboard.as_ref().map(|clipboard| clipboard.owner);
                if owner == Some(client) && self.recent_input(client, serial) {
                    self.clipboard = None;
                    self.tell_clipboard();
                }
                Ok(())
            }
            Request::ClipboardReceive {
                selection,
                mime,
                serial,
            } => {
                // A paste: by the client the user is typing into, with an input of its own, of
                // the content it was told about, in one of its types. Anything else is refused,
                // and the receiver's pipe is closed unanswered.
                let allowed =
                    self.focused_client() == Some(client) && self.recent_input(client, serial);
                let send = self.clipboard.as_ref().filter(|clipboard| {
                    allowed && clipboard.selection == selection && clipboard.types.contains(&mime)
                });
                if let Some(clipboard) = send {
                    let (owner, offer) = (clipboard.owner, clipboard.offer);
                    self.transfer = Some(owner);
                    self.emit(owner, Event::ClipboardSend { offer, mime });
                }
                Ok(())
            }
            Request::TextInputEnable { surface } => self.text_input_enable(key(surface), true),
            Request::TextInputDisable { surface } => self.text_input_enable(key(surface), false),
            Request::TextInputPurpose {
                surface,
                purpose,
                hints,
            } => self.text_input_purpose(key(surface), purpose, hints),
            Request::TextInputSurrounding { surface, text } => {
                self.text_input_surrounding(key(surface), text)
            }
            Request::TextInputSelection {
                surface,
                anchor,
                cursor,
            } => self.text_input_selection(key(surface), anchor, cursor),
            Request::TextInputCaret { surface, rect } => self.text_input_caret(key(surface), rect),
            Request::TextInputCommit { surface, serial } => {
                self.text_input_commit(key(surface), serial)?;
                self.refresh_input_method();
                Ok(())
            }
            Request::SetTitle {
                surface,
                index,
                last,
                text,
            } => self.name_window(key(surface), false, index, last, text),
            Request::SetAppId {
                surface,
                index,
                last,
                text,
            } => self.name_window(key(surface), true, index, last, text),
            Request::SetPointerShape { surface, shape } => {
                let node = self
                    .nodes
                    .get_mut(&key(surface))
                    .filter(|node| !node.dismissed)
                    .ok_or(ErrorCode::UnknownSurface)?;
                node.pointer_shape = shape;
                Ok(())
            }
            Request::SetSizeLimits {
                surface,
                min_width,
                min_height,
                max_width,
                max_height,
            } => self.set_size_limits(
                key(surface),
                Size::new(min_width, min_height),
                Size::new(max_width, max_height),
            ),
            Request::SetMaximized { surface, enabled } => self.set_maximized(key(surface), enabled),
            Request::Minimize { surface } => self.minimize_request(key(surface)),
            Request::MenusBegin {
                surface,
                serial,
                items,
            } => self.menus_begin(key(surface), serial, items),
            Request::MenuItem {
                surface,
                index,
                parent,
                kind,
                flags,
            } => self.menu_item(key(surface), index, parent, kind, flags),
            Request::MenuText {
                surface,
                index,
                field,
                text,
            } => self.menu_text(key(surface), index, field, text),
            Request::MenusCommit { surface } => self.menus_commit(key(surface)),
            Request::ActivateMenuItem {
                window,
                serial,
                index,
            } => self.activate_menu_item(client, window, serial, index),
            Request::FocusWindow { .. }
            | Request::CloseWindow { .. }
            | Request::MinimizeWindow { .. }
            | Request::MoveWindowToWorkspace { .. }
            | Request::ActivateWorkspace { .. }
            | Request::AddWorkspace {}
            | Request::RemoveWorkspace { .. }
            | Request::HideApplication { .. }
            | Request::QuitApplication { .. }
            | Request::FocusShellSurface { .. } => self.shell_request(client, request),
            Request::SetControlLevel { .. }
            | Request::SetControlMuted { .. }
            | Request::SetWifiEnabled { .. }
            | Request::SetAppearance { .. }
            | Request::RefreshControls {} => self.control_request(client, request),
            Request::ClearWindowViews { .. } | Request::PlaceWindowView { .. } => {
                self.window_view_request(client, request)
            }
            Request::LaunchApplication {
                serial,
                index,
                last,
                text,
            } => self.launch_request(client, serial, index, last, text),
            Request::SetPreference {
                serial,
                index,
                last,
                text,
            } => self.preference_request(client, serial, index, last, text),
            Request::LockSession {}
            | Request::AuthenticateSecret { .. }
            | Request::Authenticate { .. }
            | Request::AuthenticateAccount { .. } => self.lock_request(client, request),
            Request::InputMethodKeyHandled { .. }
            | Request::InputMethodDelete { .. }
            | Request::InputMethodCommitText { .. }
            | Request::InputMethodPreeditText { .. }
            | Request::InputMethodPreeditCursor { .. }
            | Request::InputMethodPreeditSegment { .. }
            | Request::InputMethodCommit { .. } => self.input_method_request(client, request),
            Request::Damage { surface, rect } => {
                let node = self
                    .nodes
                    .get_mut(&key(surface))
                    .ok_or(ErrorCode::UnknownSurface)?;
                if rect.width == 0 || rect.height == 0 {
                    return Err(ErrorCode::InvalidGeometry);
                }
                let requested = std::mem::take(&mut node.requested);
                node.requested = bounded([requested, vec![rect]].concat());
                Ok(())
            }
            Request::SetInputRegion { surface, region } => {
                let node = self
                    .nodes
                    .get_mut(&key(surface))
                    .ok_or(ErrorCode::UnknownSurface)?;
                node.input_region = Some(region);
                self.refresh_pointer_focus();
                Ok(())
            }
            Request::Submit {
                surface,
                sequence,
                buffer,
                configure,
                extent,
            } => self.submit(
                key(surface),
                Frame {
                    sequence,
                    buffer,
                    configure,
                    extent,
                },
            ),
            Request::AttachBuffer { buffer, .. } => {
                let layout = request.buffer_layout().expect("attach carries a layout");
                if buffer == 0 || layout.required_bytes().is_none() {
                    return Err(ErrorCode::InvalidBuffer);
                }
                if self.buffers.contains_key(&(client, buffer)) {
                    return Err(ErrorCode::InvalidBuffer);
                }
                let owned = self.buffers.range((client, 0)..=(client, u64::MAX)).count();
                if owned >= MAX_BUFFERS_PER_CLIENT {
                    return Err(ErrorCode::QuotaExceeded);
                }
                self.buffers.insert(
                    (client, buffer),
                    BufferEntry {
                        layout,
                        destroyed: false,
                    },
                );
                Ok(())
            }
            Request::DestroyBuffer { buffer } => {
                match self.buffers.get_mut(&(client, buffer)) {
                    Some(entry) if !entry.destroyed => entry.destroyed = true,
                    _ => return Err(ErrorCode::InvalidBuffer),
                }
                if !self.buffer_held(client, buffer) {
                    self.buffers.remove(&(client, buffer));
                    self.retired.push((client, buffer));
                }
                Ok(())
            }
            Request::Destroy { surface } => {
                if !self.nodes.contains_key(&key(surface)) {
                    return Err(ErrorCode::UnknownSurface);
                }
                self.remove_node(key(surface));
                self.refresh_pointer_focus();
                Ok(())
            }
            Request::BeginMove { surface, serial } => {
                let node = self
                    .nodes
                    .get(&key(surface))
                    .ok_or(ErrorCode::UnknownSurface)?;
                if node.role != Role::Toplevel {
                    return Err(ErrorCode::InvalidRole);
                }
                let valid = self.last_press.is_some_and(|press| {
                    press.serial == serial
                        && press.client == client
                        && press.pointer
                        && self.root_of(press.surface) == key(surface)
                }) && self.buttons_down > 0
                    && node.mapped
                    && !node.fullscreen
                    && !self.locked;
                if valid {
                    self.moving = Some(Move {
                        surface: key(surface),
                        pointer: self.pointer,
                        origin: node.origin,
                    });
                    self.set_pointer_focus(None);
                }
                Ok(())
            }
            Request::AccessibilityBegin { surface, nodes } => {
                self.accessibility_begin(key(surface), nodes)
            }
            Request::AccessibleNode {
                surface,
                index,
                parent,
                role,
                state,
                actions,
                rect,
            } => self.accessibility_node(
                key(surface),
                index,
                AccessibleNode {
                    parent,
                    role,
                    state,
                    actions,
                    rect,
                    name: String::new(),
                    value: String::new(),
                    custom: String::new(),
                    selection: None,
                },
            ),
            Request::AccessibleSelection {
                surface,
                index,
                start,
                end,
            } => self.accessibility_selection(key(surface), index, start, end),
            Request::AccessibilityAnnounce {
                surface,
                index,
                last,
                politeness,
                text,
            } => self.accessibility_announce(key(surface), index, last, politeness, text),
            Request::AccessibleText {
                surface,
                index,
                field,
                text,
            } => self.accessibility_text(key(surface), index, field, text),
            Request::AccessibilityCommit { surface } => self.accessibility_commit(key(surface)),
            Request::AccessibilityAction {
                client: target,
                surface,
                index,
                action,
                custom,
                start,
                end,
            } => {
                if !grants.accessibility {
                    return Err(ErrorCode::Unauthorized);
                }
                self.accessibility_action(
                    SurfaceKey {
                        client: target,
                        id: surface,
                    },
                    index,
                    accessibility::Action {
                        action,
                        custom,
                        start,
                        end,
                    },
                );
                Ok(())
            }
            Request::SetFullscreen { surface, enabled } => {
                let node = self
                    .nodes
                    .get(&key(surface))
                    .ok_or(ErrorCode::UnknownSurface)?;
                if node.role != Role::Toplevel {
                    return Err(ErrorCode::InvalidRole);
                }
                self.set_fullscreen(key(surface), enabled)
            }
        }
    }

    /// A toplevel full-screen on its output or back to its frame, as it asked (`SetFullscreen`)
    /// or as Super+Ctrl+F asks for it (ADR 0024).
    fn set_fullscreen(&mut self, key: SurfaceKey, enabled: bool) -> Result<(), ErrorCode> {
        let node = self.nodes.get(&key).ok_or(ErrorCode::UnknownSurface)?;
        if node.fullscreen == enabled || node.dismissed {
            return Ok(());
        }
        let output = self.output(node.output).ok_or(ErrorCode::InvalidGeometry)?;
        let (origin, size) = if enabled {
            (
                (output.rect.x, output.rect.y),
                Size::new(output.rect.width, output.rect.height),
            )
        } else {
            node.restore.unwrap_or((node.origin, node.size))
        };
        let current = (node.origin, node.size);
        let node = self.nodes.get_mut(&key).expect("present");
        node.fullscreen = enabled;
        node.restore = if enabled { Some(current) } else { None };
        self.reconfigure(key, Some(origin), size);
        Ok(())
    }

    fn check_new(&self, key: SurfaceKey) -> Result<(), ErrorCode> {
        if key.id == 0 {
            return Err(ErrorCode::InvalidMessage);
        }
        if self.nodes.contains_key(&key) {
            return Err(ErrorCode::SurfaceExists);
        }
        let owned = self.nodes.keys().filter(|k| k.client == key.client).count();
        if owned >= MAX_SURFACES_PER_CLIENT {
            return Err(ErrorCode::QuotaExceeded);
        }
        Ok(())
    }

    fn live(&self, key: SurfaceKey) -> Option<&Node> {
        self.nodes.get(&key).filter(|node| !node.dismissed)
    }

    fn shell_geometry(
        kind: ShellKind,
        edge: PanelEdge,
        size: Size,
        output: Rect,
    ) -> ((i32, i32), Size) {
        if kind == ShellKind::Background {
            return ((output.x, output.y), Size::new(output.width, output.height));
        }
        let thickness_v = size.height.min(output.height);
        let thickness_h = size.width.min(output.width);
        match edge {
            PanelEdge::Top => ((output.x, output.y), Size::new(output.width, thickness_v)),
            PanelEdge::Bottom => (
                (output.x, (output.bottom() - i64::from(thickness_v)) as i32),
                Size::new(output.width, thickness_v),
            ),
            PanelEdge::Left => ((output.x, output.y), Size::new(thickness_h, output.height)),
            PanelEdge::Right => (
                ((output.right() - i64::from(thickness_h)) as i32, output.y),
                Size::new(thickness_h, output.height),
            ),
            PanelEdge::None => {
                let size = Size::new(thickness_h, thickness_v);
                let x = i64::from(output.x) + (i64::from(output.width) - i64::from(size.width)) / 2;
                let y =
                    i64::from(output.y) + (i64::from(output.height) - i64::from(size.height)) / 2;
                ((x as i32, y as i32), size)
            }
        }
    }

    fn insert(
        &mut self,
        key: SurfaceKey,
        role: Role,
        parent: Option<SurfaceKey>,
        origin: (i32, i32),
        size: Size,
        output: OutputId,
    ) {
        let scale_120 = self
            .output(output)
            .map_or(SCALE_DENOMINATOR, |o| o.scale_120);
        let size = fit(size, scale_120);
        let presentation =
            Presentation::new(physical(size, scale_120)).expect("physical sizes are valid");
        let (serial, _) = presentation.configuration();
        let layer = layer_of(&role);
        self.nodes.insert(
            key,
            Node {
                role,
                parent,
                children: Vec::new(),
                origin,
                size,
                pending: Some((serial, origin, size)),
                wanted: None,
                answering: None,
                presentation,
                scale_120,
                output,
                mapped: false,
                hidden: false,
                dismissed: false,
                fullscreen: false,
                pending_fullscreen: false,
                shown_fullscreen: false,
                restore: None,
                maximized: false,
                unzoomed: None,
                limits: window::Limits::NONE,
                input_region: None,
                requested: Vec::new(),
                unshown: None,
                last_extent: None,
                presenting_damage: SurfaceDamage::Whole,
                presenting_in: None,
                workspace: self.workspaces.active,
                pointer_shape: PointerShape::Arrow,
            },
        );
        match parent {
            Some(parent) => self
                .nodes
                .get_mut(&parent)
                .expect("validated parent")
                .children
                .push(key),
            None => {
                let index = self
                    .roots
                    .iter()
                    .position(|root| layer_of(&self.nodes[root].role) > layer)
                    .unwrap_or(self.roots.len());
                self.roots.insert(index, key);
            }
        }
        self.send_configure(key);
    }

    fn send_configure(&mut self, key: SurfaceKey) {
        let node = &self.nodes[&key];
        let (serial, _) = node.presentation.configuration();
        let (origin, size) = match node.pending {
            Some((_, origin, size)) => (origin, size),
            None => (node.origin, node.size),
        };
        let rect = if node.is_popup() {
            Rect::new(origin.0, origin.1, size.width, size.height)
        } else {
            Rect::new(0, 0, size.width, size.height)
        };
        let event = Event::Configure {
            surface: key.id,
            serial,
            rect,
            scale_120: node.scale_120,
            fullscreen: node.fullscreen,
            maximized: node.maximized,
        };
        self.emit(key.client, event);
    }

    /// Requests a new buffer for `size` (and, for popups, a new relative origin). The displayed
    /// content stays until a frame for this configure is presented.
    fn reconfigure(&mut self, key: SurfaceKey, origin: Option<(i32, i32)>, size: Size) {
        let scale_120 = {
            let node = &self.nodes[&key];
            self.output(node.output)
                .map_or(node.scale_120, |o| o.scale_120)
        };
        let size = fit(size, scale_120);
        let node = self.nodes.get_mut(&key).expect("present");
        // A frame answering the configure replaced is on its way to the screen: its geometry stays
        // with it.
        if let (Some(frame), Some((serial, origin, size))) =
            (node.presentation.presenting(), node.pending)
        {
            if frame.configure == serial {
                node.answering = Some((serial, origin, size, node.pending_fullscreen));
            }
        }
        node.scale_120 = scale_120;
        let discarded = node
            .presentation
            .configure(physical(size, scale_120))
            .expect("physical sizes are valid");
        let (serial, _) = node.presentation.configuration();
        // Without an explicit origin the latest requested one is kept, not the displayed one.
        let keep = node.pending.map_or(node.origin, |(_, origin, _)| origin);
        node.pending = Some((serial, origin.unwrap_or(keep), size));
        node.pending_fullscreen = node.fullscreen;
        node.wanted = None;
        if let Some(frame) = discarded {
            self.discard(key, frame);
        }
        self.send_configure(key);
    }

    fn validate_anchor(
        &self,
        parent: SurfaceKey,
        positioner: &Positioner,
    ) -> Result<(), ErrorCode> {
        let parent = self.nodes.get(&parent).ok_or(ErrorCode::InvalidParent)?;
        let bounds = Rect::new(0, 0, parent.size.width, parent.size.height);
        if bounds.contains(positioner.anchor_rect) {
            Ok(())
        } else {
            Err(ErrorCode::InvalidPositioner)
        }
    }

    fn popup_depth(&self, mut key: SurfaceKey) -> usize {
        let mut depth = 0;
        while let Some(node) = self.nodes.get(&key) {
            if !node.is_popup() {
                break;
            }
            depth += 1;
            match node.parent {
                Some(parent) => key = parent,
                None => break,
            }
        }
        depth
    }

    fn create_popup(
        &mut self,
        key: SurfaceKey,
        parent: SurfaceKey,
        positioner: Positioner,
    ) -> Result<(), ErrorCode> {
        self.check_new(key)?;
        let parent_node = self.live(parent).ok_or(ErrorCode::InvalidParent)?;
        if !parent_node.mapped {
            return Err(ErrorCode::InvalidParent);
        }
        if self.popup_depth(parent) + 1 > MAX_POPUP_DEPTH {
            return Err(ErrorCode::QuotaExceeded);
        }
        self.validate_anchor(parent, &positioner)?;
        let locked_out =
            self.locked && layer_of(&self.nodes[&self.root_of(parent)].role) != Layer::Lock;
        let placed = self.placement(parent, &positioner);
        let unplaceable = placed.is_none();
        let (origin, size, output) =
            placed.unwrap_or(((0, 0), positioner.size, parent_node.output));
        self.insert(
            key,
            Role::Popup(positioner),
            Some(parent),
            origin,
            size,
            output,
        );
        if locked_out {
            self.dismiss(key, DismissReason::Locked);
        } else if unplaceable {
            self.dismiss(key, DismissReason::Unplaceable);
        }
        Ok(())
    }

    /// Relative origin, size and constraining output for a popup of `parent`.
    fn placement(
        &self,
        parent: SurfaceKey,
        positioner: &Positioner,
    ) -> Option<((i32, i32), Size, OutputId)> {
        let parent_rect = self.global_rect_pending(parent)?;
        let origin = (parent_rect.x, parent_rect.y);
        let (x, y) = anchor_point(positioner, origin);
        let output = self.output_at(x, y)?;
        let privileged = matches!(
            layer_of(&self.nodes[&self.root_of(parent)].role),
            Layer::Lock
                | Layer::Panel
                | Layer::Overlay
                | Layer::Notification
                | Layer::Background
                | Layer::Candidates
        );
        let area = if privileged {
            output.rect
        } else {
            self.work_area(output.id)?
        };
        let placed = place(positioner, origin, area)?;
        // The relative origin travels in a Configure; anything the codec cannot carry is
        // unplaceable rather than an event the compositor could not send.
        let encodable = |value: i64| -> Option<i32> {
            let limit = i64::from(MAX_COORDINATE);
            (-limit..=limit).contains(&value).then_some(value as i32)
        };
        let relative = (
            encodable(i64::from(placed.x) - i64::from(origin.0))?,
            encodable(i64::from(placed.y) - i64::from(origin.1))?,
        );
        Some((relative, Size::new(placed.width, placed.height), output.id))
    }

    fn reposition_popup(&mut self, key: SurfaceKey) {
        let node = &self.nodes[&key];
        let (Role::Popup(positioner), Some(parent)) = (node.role, node.parent) else {
            return;
        };
        match self.placement(parent, &positioner) {
            Some((origin, size, output)) => {
                self.nodes.get_mut(&key).expect("present").output = output;
                self.reconfigure(key, Some(origin), size);
            }
            None => self.dismiss(key, DismissReason::Unplaceable),
        }
    }

    /// After a parent moved or resized: re-place popups whose constrained geometry changed.
    /// Until the client presents the new configure, popups keep their parent-relative position
    /// and therefore move with the parent in the same frame.
    fn reconstrain_children(&mut self, key: SurfaceKey) {
        let children = self
            .nodes
            .get(&key)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child in children {
            let Some(node) = self.live(child) else {
                continue;
            };
            if let Role::Popup(positioner) = node.role {
                let current = node
                    .pending
                    .map_or((node.origin, node.size), |(_, o, s)| (o, s));
                let scale = node.scale_120;
                match self.placement(key, &positioner) {
                    Some((origin, size, output))
                        if (origin, size) == current
                            && self.output(output).map(|o| o.scale_120) == Some(scale) =>
                    {
                        self.nodes.get_mut(&child).expect("present").output = output;
                    }
                    Some(_) => self.reposition_popup(child),
                    None => self.dismiss(child, DismissReason::Unplaceable),
                }
            }
            self.reconstrain_children(child);
        }
    }

    fn grab_popup(&mut self, key: SurfaceKey, serial: u64) -> Result<(), ErrorCode> {
        let node = self.nodes.get(&key).ok_or(ErrorCode::UnknownSurface)?;
        if !node.is_popup() {
            return Err(ErrorCode::InvalidRole);
        }
        if node.dismissed {
            return Ok(());
        }
        let parent = node.parent.expect("popups have parents");
        let chain_ok = match self.grab.last() {
            Some(top) => *top == parent,
            None => !self.nodes[&parent].is_popup(),
        };
        let press_ok = self.last_press.is_some_and(|press| {
            press.serial == serial
                && press.client == key.client
                && (self.is_ancestor_or_self(press.surface, parent)
                    || self.grab.contains(&press.surface))
        });
        if self.locked {
            self.dismiss(key, DismissReason::Locked);
        } else if !chain_ok || !press_ok {
            self.dismiss(key, DismissReason::GrabDenied);
        } else {
            if self.grab.is_empty() {
                self.focus_before_grab = self.focus;
            }
            self.grab.push(key);
            self.set_focus(Some(key));
            // The implicit button capture moves to the grab so press-drag-release selection works.
            self.capture = None;
            self.refresh_pointer_focus();
        }
        Ok(())
    }

    fn buffer_held(&self, client: ClientId, buffer: u64) -> bool {
        self.nodes
            .iter()
            .filter(|(key, _)| key.client == client)
            .any(|(_, node)| node.presentation.held_buffers().any(|f| f.buffer == buffer))
            || self
                .deferred
                .iter()
                .any(|(key, b, _)| key.client == client && *b == buffer)
    }

    /// The registered layout of a client buffer, for the backend that reads it.
    pub fn buffer_layout(&self, client: ClientId, buffer: u64) -> Option<BufferLayout> {
        self.buffers
            .get(&(client, buffer))
            .map(|entry| entry.layout)
    }

    fn submit(&mut self, key: SurfaceKey, frame: Frame) -> Result<(), ErrorCode> {
        if !self.nodes.contains_key(&key) {
            return Err(ErrorCode::UnknownSurface);
        }
        match self.buffers.get(&(key.client, frame.buffer)) {
            Some(entry) if !entry.destroyed && entry.layout.extent == frame.extent => {}
            _ => return Err(ErrorCode::InvalidBuffer),
        }
        // A buffer whose release is waiting for the in-flight frame is still being read, on
        // whichever surface it was.
        if self
            .deferred
            .iter()
            .any(|(held, buffer, _)| held.client == key.client && *buffer == frame.buffer)
        {
            return Err(ErrorCode::InvalidBuffer);
        }
        // A buffer may be attached to only one surface at a time.
        let elsewhere = self.nodes.iter().any(|(other, node)| {
            other.client == key.client
                && *other != key
                && node
                    .presentation
                    .held_buffers()
                    .any(|f| f.buffer == frame.buffer)
        });
        if elsewhere {
            return Err(ErrorCode::InvalidBuffer);
        }
        self.send_window_views(key, frame.sequence);
        let node = self.nodes.get_mut(&key).expect("checked");
        // What the client drew into this frame since its previous one: the damage it asked for,
        // clipped to the buffer, or the whole surface without any or at a new size. It adds to
        // what the frames not yet presenting changed, shown or discarded.
        let requested = std::mem::take(&mut node.requested);
        let buffer = Rect::new(0, 0, frame.extent.width, frame.extent.height);
        let drawn = if requested.is_empty() || node.last_extent != Some(frame.extent) {
            SurfaceDamage::Whole
        } else {
            SurfaceDamage::Rects(
                requested
                    .iter()
                    .filter_map(|rect| intersect(rect, &buffer))
                    .collect(),
            )
        };
        node.last_extent = Some(frame.extent);
        node.unshown = Some(match node.unshown.take() {
            Some(unshown) => unshown.and(drawn),
            None => drawn,
        });
        if node.dismissed {
            self.discard(key, frame);
            return Ok(());
        }
        match node.presentation.submit(frame) {
            Ok(superseded) => {
                if let Some(old) = superseded {
                    self.discard(key, old);
                }
                Ok(())
            }
            Err(PresentError::StaleConfigure) => {
                self.discard(key, frame);
                Ok(())
            }
            Err(PresentError::OutOfOrder) => Err(ErrorCode::InvalidOrder),
            Err(_) => Err(ErrorCode::InvalidBuffer),
        }
    }

    // ---- lifecycle helpers ---------------------------------------------------------------

    // ---- clipboard (ADR 0007) -------------------------------------------------------------

    /// Whether `serial` is one of the latest user inputs delivered to `client`.
    fn recent_input(&self, client: ClientId, serial: u64) -> bool {
        self.inputs
            .get(&client)
            .is_some_and(|inputs| inputs.contains(&serial))
    }

    /// The client whose surface has keyboard focus.
    fn focused_client(&self) -> Option<ClientId> {
        self.focus.map(|key| key.client)
    }

    /// Tells the client with keyboard focus what the clipboard holds, if it was not told yet:
    /// its types under the name it has for receivers, or that it is empty.
    fn tell_clipboard(&mut self) {
        let Some(client) = self.focused_client() else {
            self.told = None;
            return;
        };
        let selection = self.clipboard.as_ref().map(|clipboard| clipboard.selection);
        if self.told == Some((client, selection)) {
            return;
        }
        self.told = Some((client, selection));
        match self.clipboard.clone() {
            Some(clipboard) => {
                self.emit(
                    client,
                    Event::ClipboardTypes {
                        selection: clipboard.selection,
                        count: clipboard.types.len() as u8,
                    },
                );
                for mime in clipboard.types {
                    self.emit(
                        client,
                        Event::ClipboardType {
                            selection: clipboard.selection,
                            mime,
                        },
                    );
                }
            }
            None => self.emit(client, Event::ClipboardEmpty {}),
        }
    }

    /// The owner that must answer the paste the request handled last, if it was allowed: the
    /// compositor passes it the receiver's pipe with the ClipboardSend event queued for it.
    pub fn take_transfer(&mut self) -> Option<ClientId> {
        self.transfer.take()
    }

    fn emit(&mut self, client: ClientId, event: Event) {
        if self.clients.contains_key(&client) {
            self.events.push((client, event));
        }
    }

    fn discard(&mut self, key: SurfaceKey, frame: Frame) {
        self.emit(
            key.client,
            Event::Feedback {
                surface: key.id,
                sequence: frame.sequence,
                outcome: Outcome::Discarded,
                timestamp_ns: 0,
                clock: Clock::Synthetic,
            },
        );
        self.release(key, frame.buffer);
    }

    fn release(&mut self, key: SurfaceKey, buffer: u64) {
        let reading: BTreeSet<FrameOf> = self
            .frames_in_flight
            .iter()
            .filter(|(_, reads)| reads.as_ref().is_none_or(|reads| reads.contains(&key)))
            .map(|(frame, _)| *frame)
            .collect();
        if reading.is_empty() {
            self.release_now(key, buffer);
        } else {
            self.deferred.push((key, buffer, reading));
        }
    }

    fn release_now(&mut self, key: SurfaceKey, buffer: u64) {
        self.emit(
            key.client,
            Event::Released {
                surface: key.id,
                buffer,
            },
        );
        match self.buffers.get(&(key.client, buffer)) {
            Some(entry) if entry.destroyed && !self.buffer_held(key.client, buffer) => {
                self.buffers.remove(&(key.client, buffer));
                self.retired.push((key.client, buffer));
            }
            // The client is gone and its registry entries with it.
            None => self.retired.push((key.client, buffer)),
            Some(_) => {}
        }
    }

    fn release_all(&mut self, key: SurfaceKey) {
        let Some(node) = self.nodes.get_mut(&key) else {
            return;
        };
        let held: Vec<Frame> = node.presentation.take_all();
        node.mapped = false;
        for frame in held {
            self.release(key, frame.buffer);
        }
    }

    fn root_of(&self, mut key: SurfaceKey) -> SurfaceKey {
        while let Some(parent) = self.nodes.get(&key).and_then(|node| node.parent) {
            key = parent;
        }
        key
    }

    fn is_ancestor_or_self(&self, ancestor: SurfaceKey, mut key: SurfaceKey) -> bool {
        loop {
            if key == ancestor {
                return true;
            }
            match self.nodes.get(&key).and_then(|node| node.parent) {
                Some(parent) => key = parent,
                None => return false,
            }
        }
    }

    fn in_grab(&self, key: SurfaceKey) -> bool {
        self.grab
            .iter()
            .any(|popup| self.is_ancestor_or_self(*popup, key))
    }

    /// Sends PopupDone to `key` (if it is a child or privileged surface) and every descendant,
    /// releases their content and makes them inert until the client destroys them.
    fn dismiss(&mut self, key: SurfaceKey, reason: DismissReason) {
        let Some(node) = self.nodes.get_mut(&key) else {
            return;
        };
        if node.dismissed {
            return;
        }
        let shown = node.mapped && !node.hidden;
        node.dismissed = true;
        let children = node.children.clone();
        for child in children.iter().rev() {
            self.dismiss(*child, reason);
        }
        if shown {
            self.appeared(key, appearances::Change::Gone);
        }
        self.emit(
            key.client,
            Event::PopupDone {
                surface: key.id,
                reason,
            },
        );
        self.release_all(key);
        self.detach_input(key);
        if let Some(index) = self.grab.iter().position(|popup| *popup == key) {
            self.grab.truncate(index);
            if self.grab.is_empty() {
                let restore = self.focus_before_grab.take();
                self.set_focus(restore.filter(|k| self.focusable(*k)));
                if self.focus.is_none() {
                    self.focus_next();
                }
            } else {
                let top = self.grab.last().copied();
                self.set_focus(top);
            }
        }
        if self.focus == Some(key) {
            self.set_focus(None);
            self.focus_next();
        }
    }

    fn dismiss_grab(&mut self, reason: DismissReason) {
        if let Some(first) = self.grab.first().copied() {
            self.dismiss(first, reason);
        }
    }

    /// Clears input state that refers to `key` without delivering events to it.
    fn detach_input(&mut self, key: SurfaceKey) {
        if self.pinch == Some(key) {
            self.pinch = None;
        }
        if self.capture == Some(key) {
            self.capture = None;
            self.swallowed_releases = self.buttons_down;
        }
        if self.pointer_focus == Some(key) {
            self.pointer_focus = None;
        }
        for target in self.touches.values_mut() {
            if *target == Some(key) {
                *target = None;
            }
        }
        if self.moving.is_some_and(|m| m.surface == key) {
            self.moving = None;
        }
    }

    fn remove_node(&mut self, key: SurfaceKey) {
        let Some(node) = self.nodes.get(&key) else {
            return;
        };
        let children = node.children.clone();
        let parent = node.parent;
        for child in children {
            if self.nodes.get(&child).is_some_and(|n| !n.dismissed) {
                self.dismiss(child, DismissReason::ParentGone);
            }
            if let Some(orphan) = self.nodes.get_mut(&child) {
                orphan.parent = None;
            }
        }
        if !self.nodes[&key].dismissed {
            if self.nodes[&key].mapped && !self.nodes[&key].hidden {
                self.appeared(key, appearances::Change::Gone);
            }
            // Without a PopupDone: the client asked for this.
            let node = self.nodes.get_mut(&key).expect("present");
            node.dismissed = true;
            self.release_all(key);
            self.detach_input(key);
            if let Some(index) = self.grab.iter().position(|popup| *popup == key) {
                self.grab.truncate(index);
                if self.grab.is_empty() {
                    let restore = self.focus_before_grab.take();
                    self.set_focus(restore.filter(|k| *k != key && self.focusable(*k)));
                } else {
                    let top = self.grab.last().copied();
                    self.set_focus(top);
                }
            }
        }
        if let Some(parent) = parent.and_then(|p| self.nodes.get_mut(&p)) {
            parent.children.retain(|child| *child != key);
        }
        self.roots.retain(|root| *root != key);
        self.nodes.remove(&key);
        self.forget_accessibility(key);
        self.forget_text_input(key);
        self.forget_secrets(Some(key));
        self.forget_window(key);
        self.forget_application_window(key);
        self.forget_menus(key);
        self.forget_window_views(key);
        if self.active == Some(key) {
            self.active = None;
        }
        if self.resizing.is_some_and(|resize| resize.surface == key) {
            self.resizing = None;
        }
        if self.focus == Some(key) {
            self.focus = None;
            self.focus_next();
        }
        if self.focus_before_grab == Some(key) {
            self.focus_before_grab = None;
        }
        if self.last_press.is_some_and(|press| press.surface == key) {
            self.last_press = None;
        }
    }

    // ---- visibility, stacking and focus ---------------------------------------------------

    fn visible(&self, key: SurfaceKey) -> bool {
        self.visible_in(key, false)
    }

    fn visible_in(&self, key: SurfaceKey, in_flight: bool) -> bool {
        let mut current = key;
        loop {
            let Some(node) = self.nodes.get(&current) else {
                return false;
            };
            let mapped = node.mapped || in_flight && node.presentation.is_presenting();
            if node.dismissed || node.hidden || !mapped {
                return false;
            }
            match node.parent {
                Some(parent) => current = parent,
                None => break,
            }
        }
        let root = &self.nodes[&current];
        // A window on another workspace is not shown, nor is anything under it.
        if root.role == Role::Toplevel && root.workspace != self.workspaces.active {
            return false;
        }
        let layer = layer_of(&root.role);
        if self.locked {
            layer == Layer::Lock
        } else {
            layer != Layer::Lock
        }
    }

    fn global_origin(&self, key: SurfaceKey) -> Option<(i64, i64)> {
        self.origin_in(key, false)
    }

    fn origin_in(&self, key: SurfaceKey, in_flight: bool) -> Option<(i64, i64)> {
        let node = self.nodes.get(&key)?;
        let origin = node
            .view(in_flight)
            .map_or(node.origin, |(_, origin, _)| origin);
        let (x, y) = (i64::from(origin.0), i64::from(origin.1));
        match node.parent {
            Some(parent) => {
                let (px, py) = self.origin_in(parent, in_flight)?;
                Some((px + x, py + y))
            }
            None => Some((x, y)),
        }
    }

    /// Displayed global geometry.
    pub fn global_rect(&self, key: SurfaceKey) -> Option<Rect> {
        let node = self.nodes.get(&key)?;
        let (x, y) = self.global_origin(key)?;
        Some(Rect::new(
            i32::try_from(x).ok()?,
            i32::try_from(y).ok()?,
            node.size.width,
            node.size.height,
        ))
    }

    /// Geometry once every pending configure is presented; used to place new children.
    fn global_rect_pending(&self, key: SurfaceKey) -> Option<Rect> {
        let node = self.nodes.get(&key)?;
        let (origin, size) = node.upcoming();
        let (x, y) = match node.parent {
            Some(parent) => {
                let p = self.global_rect_pending(parent)?;
                (
                    i64::from(p.x) + i64::from(origin.0),
                    i64::from(p.y) + i64::from(origin.1),
                )
            }
            None => (i64::from(origin.0), i64::from(origin.1)),
        };
        Some(Rect::new(
            i32::try_from(x).ok()?,
            i32::try_from(y).ok()?,
            size.width,
            size.height,
        ))
    }

    fn push_tree(&self, key: SurfaceKey, out: &mut Vec<RenderItem>, in_flight: bool) {
        if !self.visible_in(key, in_flight) {
            return;
        }
        let node = &self.nodes[&key];
        if let (Some((x, y)), Some((frame, _, size))) =
            (self.origin_in(key, in_flight), node.view(in_flight))
        {
            if let (Ok(x), Ok(y)) = (i32::try_from(x), i32::try_from(y)) {
                let rect = Rect::new(x, y, size.width, size.height);
                out.push(RenderItem::Surface { key, rect, frame });
                // Over a shell surface, the windows it shows live; below its popups.
                if matches!(node.role, Role::Shell { .. }) {
                    self.push_window_views(key, rect, out, in_flight);
                }
            }
        }
        for child in &node.children {
            self.push_tree(*child, out, in_flight);
        }
    }

    /// Ready content bottom to top. Only presented frames appear: a surface without one is not
    /// drawn at all, never as an empty rectangle. While locked only lock surfaces and covers.
    pub fn render_list(&self) -> Vec<RenderItem> {
        self.list(false)
    }

    /// What the backend composes between `begin_frame` and `end_frame`: in-flight frames with
    /// the geometry they were rendered for, displayed content elsewhere.
    pub fn frame_list(&self) -> Vec<RenderItem> {
        self.list(true)
    }

    fn list(&self, in_flight: bool) -> Vec<RenderItem> {
        let mut out = Vec::new();
        for root in &self.roots {
            self.push_tree(*root, &mut out, in_flight);
        }
        if self.locked {
            // Covers go above everything: an output without its own ready lock surface shows
            // nothing else, not even another output's lock surface where outputs overlap.
            let mut covers = Vec::new();
            for output in &self.outputs {
                let covered = out.iter().any(|item| match item {
                    RenderItem::Surface { key, .. } => {
                        self.nodes[key].role == Role::Lock { output: output.id }
                    }
                    RenderItem::Cover { .. } | RenderItem::View { .. } => false,
                });
                if !covered {
                    covers.push(RenderItem::Cover {
                        output: output.id,
                        rect: output.rect,
                    });
                }
            }
            out.extend(covers);
        }
        out
    }

    /// How `key` is drawn as a window (docs/adr/0016-windows.mdx), if it is one the compositor
    /// rounds and shadows: a toplevel or a dialog, not drawn full-screen. Between `begin_frame`
    /// and `end_frame`, as the frame being composed draws it (as `frame_list`); otherwise as
    /// displayed: a window going full-screen is drawn so from its first full-screen frame.
    pub fn window_look(&self, key: SurfaceKey) -> Option<WindowLook> {
        let node = self.live(key)?;
        let fullscreen = match (node.presentation.presenting(), node.pending, node.answering) {
            (Some(frame), Some((serial, ..)), _) if serial == frame.configure => {
                node.pending_fullscreen
            }
            (Some(frame), _, Some((serial, _, _, fullscreen))) if serial == frame.configure => {
                fullscreen
            }
            _ => node.shown_fullscreen,
        };
        if !matches!(node.role, Role::Toplevel | Role::Dialog) || fullscreen {
            return None;
        }
        Some(WindowLook {
            active: self.active == Some(self.root_of(key)),
        })
    }

    /// A surface whose buffer may be scanned out directly: a fullscreen window covering the
    /// output with nothing (menu, panel, notification) above it on that output.
    pub fn scanout_candidate(&self, output: OutputId) -> Option<SurfaceKey> {
        let output = self.output(output)?;
        let top = self
            .render_list()
            .into_iter()
            .rev()
            .find(|item| match item {
                RenderItem::Surface { rect, .. }
                | RenderItem::Cover { rect, .. }
                | RenderItem::View { clip: rect, .. } => rect.intersects(output.rect),
            })?;
        match top {
            RenderItem::Surface { key, rect, .. }
                if rect == output.rect && self.nodes[&key].fullscreen =>
            {
                Some(key)
            }
            _ => None,
        }
    }

    fn hit_test(&self, x: i32, y: i32) -> Option<SurfaceKey> {
        self.render_list()
            .into_iter()
            .rev()
            .find_map(|item| match item {
                RenderItem::Surface { key, rect, .. } if rect.contains_point(x, y) => {
                    let local_x = i64::from(x) - i64::from(rect.x);
                    let local_y = i64::from(y) - i64::from(rect.y);
                    let accepts = match self.nodes[&key].input_region {
                        None => true,
                        Some(region) => region.contains_point(local_x as i32, local_y as i32),
                    };
                    accepts.then_some(key)
                }
                _ => None,
            })
    }

    fn focusable(&self, key: SurfaceKey) -> bool {
        self.visible(key)
            && !matches!(
                self.nodes[&key].role,
                Role::Shell {
                    kind: ShellKind::Background,
                    ..
                } | Role::Candidates
            )
    }

    fn set_focus(&mut self, key: Option<SurfaceKey>) {
        if self.focus == key {
            return;
        }
        // The input method stops serving the field before focus leaves it.
        self.deactivate_input_method();
        if let Some(old) = self.focus.take() {
            self.emit(
                old.client,
                Event::Focus {
                    surface: old.id,
                    focused: false,
                },
            );
        }
        if let Some(new) = key {
            self.emit(
                new.client,
                Event::Focus {
                    surface: new.id,
                    focused: true,
                },
            );
            if self.caps_lock {
                self.emit(
                    new.client,
                    Event::CapsLock {
                        surface: new.id,
                        on: true,
                    },
                );
            }
        }
        self.focus = key;
        self.update_active();
        self.tell_clipboard();
        self.refresh_input_method();
    }

    /// Focus target inside a window group: its topmost live modal dialog, else the window.
    fn focus_target(&self, root: SurfaceKey) -> SurfaceKey {
        self.nodes[&root]
            .children
            .iter()
            .rev()
            .copied()
            .find(|child| self.nodes[child].role == Role::Dialog && self.visible(*child))
            .unwrap_or(root)
    }

    fn focus_next(&mut self) {
        if !self.grab.is_empty() {
            return;
        }
        let candidate = self
            .roots
            .iter()
            .rev()
            .copied()
            .filter(|root| {
                let layer = layer_of(&self.nodes[root].role);
                if self.locked {
                    layer == Layer::Lock
                } else {
                    layer == Layer::Normal
                }
            })
            .find(|root| self.visible(*root))
            .map(|root| self.focus_target(root));
        self.set_focus(candidate);
    }

    /// Raises a window group within its layer and focuses it.
    pub fn activate(&mut self, key: SurfaceKey) {
        let root = self.root_of(key);
        if !self.nodes.contains_key(&root)
            || self.locked && layer_of(&self.nodes[&root].role) != Layer::Lock
        {
            return;
        }
        let layer = layer_of(&self.nodes[&root].role);
        self.roots.retain(|r| *r != root);
        let index = self
            .roots
            .iter()
            .position(|r| layer_of(&self.nodes[r].role) > layer)
            .unwrap_or(self.roots.len());
        self.roots.insert(index, root);
        if self.grab.is_empty() && self.focusable(root) {
            let target = self.focus_target(root);
            self.set_focus(Some(target));
        }
    }

    fn lower_below_focus(&mut self, key: SurfaceKey) {
        let root = self.root_of(key);
        let Some(focused) = self.focus.map(|f| self.root_of(f)) else {
            return;
        };
        if focused == root
            || layer_of(&self.nodes[&focused].role) != layer_of(&self.nodes[&root].role)
        {
            return;
        }
        self.roots.retain(|r| *r != root);
        let index = self
            .roots
            .iter()
            .position(|r| *r == focused)
            .unwrap_or(self.roots.len());
        self.roots.insert(index, root);
    }

    pub fn minimize(&mut self, key: SurfaceKey) {
        self.conceal(key, appearances::Change::Minimized);
    }

    /// Takes a window off the screen until it is restored: minimized into the dock, or hidden
    /// with its application (ADR 0024).
    fn conceal(&mut self, key: SurfaceKey, change: appearances::Change) {
        let Some(node) = self.nodes.get(&key) else {
            return;
        };
        if node.role != Role::Toplevel || node.hidden {
            return;
        }
        let children = node.children.clone();
        if node.mapped {
            self.appeared(key, change);
        }
        self.nodes.get_mut(&key).expect("present").hidden = true;
        for child in children {
            if self.nodes[&child].is_popup() {
                self.dismiss(child, DismissReason::ParentHidden);
            }
        }
        self.detach_input(key);
        if self.focus.is_some_and(|f| self.root_of(f) == key) {
            self.set_focus(None);
            self.focus_next();
        }
        // Minimized while the shell's top bar had focus: no longer the active window.
        self.update_active();
        self.refresh_pointer_focus();
    }

    /// Shows a minimized window again and activates it; a window hidden with its application
    /// brings back every window hidden with it (ADR 0024).
    pub fn restore(&mut self, key: SurfaceKey) {
        if self.applications_hidden(key) {
            self.reveal_application(key);
            self.activate(key);
            self.refresh_pointer_focus();
            return;
        }
        if let Some(node) = self.nodes.get_mut(&key) {
            if node.hidden {
                node.hidden = false;
                if node.mapped {
                    self.appeared(key, appearances::Change::Restored);
                }
                self.activate(key);
                self.refresh_pointer_focus();
            }
        }
    }

    // ---- window management -----------------------------------------------------------------

    /// Moves a window to `origin` (global logical), as the shell does for a keyboard move or
    /// tiling: at once and without the client, like an interactive move, and kept on screen the
    /// same way. Popups keep their parent-relative position, so they move with it in the same
    /// frame (an open menu stays open), and are then re-placed if their constraints changed.
    pub fn move_window_to(&mut self, key: SurfaceKey, origin: (i32, i32)) {
        let Some(node) = self.live(key) else {
            return;
        };
        if node.role != Role::Toplevel || node.fullscreen || node.hidden || self.locked {
            return;
        }
        let current = node.origin;
        self.move_window(
            Move {
                surface: key,
                pointer: current,
                origin: current,
            },
            origin.0,
            origin.1,
        );
        self.reconstrain_children(key);
        self.refresh_pointer_focus();
    }

    /// Resizes a window, as the shell does to maximize or tile it: the client is configured
    /// with `size` (fitted to its output's scale). Its content stays until it presents a frame
    /// for the new size; then the window takes that size and its popups are re-placed.
    pub fn resize_window(&mut self, key: SurfaceKey, size: Size) {
        let Some(node) = self.live(key) else {
            return;
        };
        if node.role != Role::Toplevel
            || node.fullscreen
            || node.hidden
            || self.locked
            || size.width == 0
            || size.height == 0
        {
            return;
        }
        let size = node.limits.clamp(size);
        self.reconfigure(key, None, size);
        // Maximized or tiled, its frame goes there animated (ADR 0016).
        self.appeared(key, appearances::Change::Zoomed);
    }

    // ---- secure lock ---------------------------------------------------------------------

    /// Protects the session immediately: popups and grabs are dismissed, input to ordinary
    /// surfaces stops and only lock surfaces or compositor covers are rendered from now on.
    pub fn lock(&mut self) {
        if self.locked {
            return;
        }
        self.locked = true;
        self.dismiss_grab(DismissReason::Locked);
        self.cancel_switcher();
        let popups: Vec<SurfaceKey> = self
            .nodes
            .iter()
            .filter(|(_, node)| node.is_popup() && !node.dismissed)
            .map(|(key, _)| *key)
            .collect();
        for popup in popups {
            self.dismiss(popup, DismissReason::Locked);
        }
        self.pointer_cancel();
        self.touch_cancel();
        self.set_pointer_focus(None);
        self.set_focus(None);
        self.focus_next();
        self.hide_accessibility_behind_lock();
        self.tell_lock_state();
    }

    /// Called only from the session authority's trusted authentication path; no protocol
    /// request reaches it.
    pub fn unlock(&mut self) {
        if !self.locked {
            return;
        }
        self.locked = false;
        // The lock screen goes, over the session shown again.
        let lock_surfaces: Vec<SurfaceKey> = self
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.role, Role::Lock { .. }) && node.mapped)
            .map(|(key, _)| *key)
            .collect();
        for key in lock_surfaces {
            self.appeared(key, appearances::Change::Gone);
        }
        self.forget_secrets(None);
        self.pointer_cancel();
        self.touch_cancel();
        self.set_focus(None);
        self.focus_next();
        self.refresh_pointer_focus();
        self.show_accessibility_after_lock();
        self.tell_lock_state();
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    pub fn caps_lock(&self) -> bool {
        self.caps_lock
    }

    /// Counts the person's input: every key, pointer motion, button, scroll, pinch and touch,
    /// and the text the keyboard types, wherever it goes (an application, the shell, the lock
    /// screen, nothing). The session's idle lock is armed again whenever it changes (ADR 0010);
    /// what clients draw and the time passing never change it.
    pub fn activity(&self) -> u64 {
        self.activity
    }

    fn user_input(&mut self) {
        self.activity = self.activity.wrapping_add(1);
    }

    /// The keyboard's Caps Lock came on or went off: the surface with keyboard focus is told.
    pub fn set_caps_lock(&mut self, on: bool) {
        if self.caps_lock == on {
            return;
        }
        self.caps_lock = on;
        if let Some(focus) = self.focus {
            self.emit(
                focus.client,
                Event::CapsLock {
                    surface: focus.id,
                    on,
                },
            );
        }
    }

    // ---- input ---------------------------------------------------------------------------

    fn next_serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }

    fn local(&self, key: SurfaceKey, x: i32, y: i32) -> (i32, i32) {
        let (ox, oy) = self.global_origin(key).unwrap_or((0, 0));
        (to_fixed(i64::from(x) - ox), to_fixed(i64::from(y) - oy))
    }

    fn pointer_event(&mut self, key: SurfaceKey, serial: u64, kind: PointerKind, button: u32) {
        let (x, y) = self.local(key, self.pointer.0, self.pointer.1);
        self.emit(
            key.client,
            Event::Pointer {
                surface: key.id,
                serial,
                kind,
                x,
                y,
                button,
            },
        );
    }

    fn pointer_target(&self) -> Option<SurfaceKey> {
        if self.moving.is_some() || self.resizing.is_some() || self.resize_hover.is_some() {
            return None;
        }
        if let Some(capture) = self.capture {
            return Some(capture);
        }
        let hit = self.hit_test(self.pointer.0, self.pointer.1)?;
        if !self.grab.is_empty() && !self.in_grab(hit) {
            return None;
        }
        Some(hit)
    }

    fn set_pointer_focus(&mut self, target: Option<SurfaceKey>) {
        if self.pointer_focus == target {
            return;
        }
        if let Some(old) = self.pointer_focus.take() {
            if self.nodes.contains_key(&old) {
                let serial = self.next_serial();
                self.pointer_event(old, serial, PointerKind::Leave, 0);
            }
        }
        if let Some(new) = target {
            let serial = self.next_serial();
            self.pointer_event(new, serial, PointerKind::Enter, 0);
        }
        self.pointer_focus = target;
    }

    fn refresh_pointer_focus(&mut self) {
        let target = self.pointer_target();
        self.set_pointer_focus(target);
    }

    pub fn pointer_motion(&mut self, x: i32, y: i32) {
        self.user_input();
        self.pointer = (x, y);
        if let Some(moving) = self.moving {
            self.move_window(moving, x, y);
            return;
        }
        if let Some(resize) = self.resizing {
            self.resize_to_pointer(resize, x, y);
            return;
        }
        self.resize_hover = if self.capture.is_none() {
            self.resize_edges_at(x, y).map(|(_, edges)| edges)
        } else {
            None
        };
        let target = self.pointer_target();
        self.set_pointer_focus(target);
        if let Some(target) = target {
            let serial = self.next_serial();
            self.pointer_event(target, serial, PointerKind::Motion, 0);
        }
    }

    /// Compositor-owned move: no client round trip, so a stalled client still moves smoothly.
    /// The pointer's shape now: the one the surface under it wants, the arrow over none, and the
    /// move shape while the compositor moves a window.
    pub fn pointer_shape(&self) -> PointerShape {
        if self.moving.is_some() {
            return PointerShape::Move;
        }
        if let Some(edges) = self.resizing.map(|r| r.edges).or(self.resize_hover) {
            return edges.pointer_shape();
        }
        self.pointer_focus
            .and_then(|key| self.live(key))
            .map_or(PointerShape::Arrow, |node| node.pointer_shape)
    }

    fn move_window(&mut self, moving: Move, x: i32, y: i32) {
        let Some(node) = self.nodes.get(&moving.surface) else {
            self.moving = None;
            return;
        };
        let size = node.size;
        let dx = i64::from(x) - i64::from(moving.pointer.0);
        let dy = i64::from(y) - i64::from(moving.pointer.1);
        let mut nx = i64::from(moving.origin.0) + dx;
        let mut ny = i64::from(moving.origin.1) + dy;
        if let Some(bounds) = self.output_bounds() {
            nx = nx.clamp(
                i64::from(bounds.x) - i64::from(size.width) + MIN_VISIBLE,
                bounds.right() - MIN_VISIBLE,
            );
            ny = ny.clamp(i64::from(bounds.y), bounds.bottom() - MIN_VISIBLE);
        }
        let origin = (nx as i32, ny as i32);
        let node = self.nodes.get_mut(&moving.surface).expect("present");
        let delta = (
            i64::from(origin.0) - i64::from(node.origin.0),
            i64::from(origin.1) - i64::from(node.origin.1),
        );
        node.origin = origin;
        let moved = |at: (i32, i32)| {
            (
                (i64::from(at.0) + delta.0) as i32,
                (i64::from(at.1) + delta.1) as i32,
            )
        };
        if let Some((serial, pending, size)) = node.pending {
            node.pending = Some((serial, moved(pending), size));
        }
        if let Some((wanted, size)) = node.wanted {
            node.wanted = Some((moved(wanted), size));
        }
        if let Some((serial, answering, size, fullscreen)) = node.answering {
            node.answering = Some((serial, moved(answering), size, fullscreen));
        }
        if let Some(output) = self.output_at(i64::from(origin.0), i64::from(origin.1)) {
            self.move_to_output(moving.surface, output.id);
        }
        self.refresh_accessibility_geometry();
        self.place_candidates();
    }

    /// Assigns `output` to a window and its dialogs, asking for buffers at the new scale when it
    /// differs; displayed content stays until they arrive. Popups follow their anchor output in
    /// `reconstrain_children`.
    fn move_to_output(&mut self, key: SurfaceKey, output: OutputId) {
        let Some(scale) = self.output(output).map(|o| o.scale_120) else {
            return;
        };
        let Some(node) = self.nodes.get_mut(&key) else {
            return;
        };
        node.output = output;
        let rescale = node.scale_120 != scale && !node.dismissed;
        let size = node.upcoming().1;
        let children = node.children.clone();
        if rescale {
            self.reconfigure(key, None, size);
        }
        for child in children {
            if self
                .nodes
                .get(&child)
                .is_some_and(|n| n.role == Role::Dialog)
            {
                self.move_to_output(child, output);
            }
        }
    }

    fn output_bounds(&self) -> Option<Rect> {
        let first = self.outputs.first()?.rect;
        let (mut x0, mut y0, mut x1, mut y1) = (
            i64::from(first.x),
            i64::from(first.y),
            first.right(),
            first.bottom(),
        );
        for output in &self.outputs {
            x0 = x0.min(i64::from(output.rect.x));
            y0 = y0.min(i64::from(output.rect.y));
            x1 = x1.max(output.rect.right());
            y1 = y1.max(output.rect.bottom());
        }
        Some(Rect::new(
            x0 as i32,
            y0 as i32,
            (x1 - x0) as u32,
            (y1 - y0) as u32,
        ))
    }

    pub fn pointer_button(&mut self, button: u32, pressed: bool) {
        self.user_input();
        let serial = self.next_serial();
        if pressed {
            self.buttons_down += 1;
            let hit = self.hit_test(self.pointer.0, self.pointer.1);
            self.last_user_serial = serial;
            if self.resizing.is_some() {
                return;
            }
            if self.buttons_down == 1 && self.capture.is_none() {
                if let Some((window, edges)) = self.resize_edges_at(self.pointer.0, self.pointer.1)
                {
                    self.begin_resize(window, edges);
                    return;
                }
            }
            if !self.grab.is_empty() && !hit.is_some_and(|h| self.in_grab(h)) {
                // Outside press: dismiss the chain and consume the press and its release. Every
                // held button's release is consumed, and the press invalidates older serials.
                self.dismiss_grab(DismissReason::OutsidePress);
                self.consume_press();
                self.refresh_pointer_focus();
                return;
            }
            let Some(target) = self.capture.or(hit) else {
                self.consume_press();
                return;
            };
            if self.grab.is_empty() {
                let root = self.root_of(target);
                let blocked = target == root && self.focus_target(root) != root;
                self.activate(target);
                if blocked {
                    // Modal scope: the window behind a dialog does not receive the press.
                    self.consume_press();
                    return;
                }
                if self.focusable(target) && self.root_of(target) == root && target != root {
                    self.set_focus(Some(target));
                }
            }
            self.capture = Some(target);
            self.set_pointer_focus(Some(target));
            self.record_press(serial, target, true);
            self.pointer_event(target, serial, PointerKind::Press, button);
        } else {
            self.buttons_down = self.buttons_down.saturating_sub(1);
            if self.resizing.is_some() {
                if self.buttons_down == 0 {
                    self.end_resize();
                }
                return;
            }
            if let Some(moving) = self.moving {
                if self.buttons_down == 0 {
                    // The move owned the pointer; the window that started it gets no capture.
                    self.moving = None;
                    self.capture = None;
                    self.reconstrain_children(moving.surface);
                    self.refresh_pointer_focus();
                }
                return;
            }
            if self.swallowed_releases > 0 {
                self.swallowed_releases -= 1;
                if self.buttons_down == 0 {
                    self.capture = None;
                    self.refresh_pointer_focus();
                }
                return;
            }
            if let Some(target) = self.pointer_target() {
                self.pointer_event(target, serial, PointerKind::Release, button);
            }
            if self.buttons_down == 0 {
                self.capture = None;
                self.refresh_pointer_focus();
            }
        }
    }

    /// A press that reaches no client: its release is consumed and it invalidates every earlier
    /// press serial, so no client can authorize a grab with an older one.
    fn consume_press(&mut self) {
        self.swallowed_releases = self.buttons_down;
        self.last_press = None;
    }

    fn record_press(&mut self, serial: u64, target: SurfaceKey, pointer: bool) {
        let inputs = self.inputs.entry(target.client).or_default();
        inputs.push_back(serial);
        if inputs.len() > RECENT_INPUTS {
            inputs.pop_front();
        }
        self.last_press = Some(Press {
            serial,
            client: target.client,
            surface: target,
            pointer,
        });
        self.activation.insert(target.client, serial);
    }

    /// Pointer capture lost (for example a compositor gesture took over): the captured surface
    /// receives Cancel and no later release is delivered as a click.
    pub fn pointer_cancel(&mut self) {
        if let Some(target) = self.capture.take() {
            let serial = self.next_serial();
            self.pointer_event(target, serial, PointerKind::Cancel, 0);
        }
        self.swallowed_releases = self.buttons_down;
        self.moving = None;
        self.resizing = None;
    }

    /// A wheel or touchpad scroll, in 1/256 logical pixels, for the surface under the pointer
    /// (or holding its capture). It is not a press: it authorizes no grab and activates nothing.
    pub fn pointer_scroll(&mut self, dx: i32, dy: i32) {
        self.user_input();
        let Some(target) = self.pointer_target() else {
            return;
        };
        let serial = self.next_serial();
        self.emit(
            target.client,
            Event::Scroll {
                surface: target.id,
                serial,
                dx,
                dy,
            },
        );
    }

    /// A touchpad pinch: it goes to the surface under the pointer when it began (as a scroll
    /// does) until it ends, at the pointer, which a pinch does not move. `scale_65536` and
    /// `rotation_256` are as the Pinch event carries them. A pinch that began over no surface,
    /// or whose surface went, reaches nothing.
    pub fn pointer_pinch(&mut self, phase: GesturePhase, scale_65536: u32, rotation_256: i32) {
        self.user_input();
        if phase == GesturePhase::Begin {
            self.pinch = self.pointer_target();
        }
        let Some(target) = self.pinch else {
            return;
        };
        if matches!(phase, GesturePhase::End | GesturePhase::Cancel) {
            self.pinch = None;
        }
        let serial = self.next_serial();
        let (x, y) = self.local(target, self.pointer.0, self.pointer.1);
        self.emit(
            target.client,
            Event::Pinch {
                surface: target.id,
                serial,
                phase,
                x,
                y,
                scale_65536,
                rotation_256,
            },
        );
    }

    /// The keyboard's composition (preedit) for the focused surface: `text` replaces the
    /// previous composition, with the composing cursor at byte `cursor`; empty text ends it.
    /// Returns false, delivering nothing, for a cursor outside it or inside a character.
    pub fn preedit(&mut self, text: &str, cursor: usize) -> bool {
        self.user_input();
        let Ok(cursor) = u32::try_from(cursor) else {
            return false;
        };
        let whole = [(0, text.len() as u32, PreeditStyle::Composing)];
        self.text_transaction(&TextTransaction {
            preedit: text,
            cursor: (cursor, cursor),
            segments: if text.is_empty() { &[] } else { &whole },
            ..TextTransaction::default()
        })
    }

    /// Text committed by the keyboard layout, for the focused surface. It counts as user input,
    /// like a key press, and ends any composition.
    pub fn text(&mut self, text: &str) {
        self.user_input();
        self.text_transaction(&TextTransaction {
            commit: text,
            ..TextTransaction::default()
        });
    }

    /// A key from the keyboard: the screen reader's first (ADR 0014), then the hardware keys
    /// (ADR 0025), then the system shortcuts (ADR 0024), then the focused surface.
    pub fn key(&mut self, key: u32, pressed: bool) {
        self.user_input();
        if !self.reader_key(key, pressed)
            && !self.hardware_key(key, pressed)
            && !self.system_key(key, pressed)
        {
            self.surface_key(key, pressed);
        }
    }

    fn surface_key(&mut self, key: u32, pressed: bool) {
        let serial = self.next_serial();
        if pressed {
            self.last_user_serial = serial;
        }
        let Some(target) = self.focus.filter(|f| self.visible(*f)) else {
            return;
        };
        if pressed {
            self.record_press(serial, target, false);
        }
        self.emit(
            target.client,
            Event::Key {
                surface: target.id,
                serial,
                key,
                pressed,
            },
        );
    }

    fn touch_event(
        &mut self,
        key: SurfaceKey,
        serial: u64,
        kind: TouchKind,
        id: u32,
        x: i32,
        y: i32,
    ) {
        let (lx, ly) = self.local(key, x, y);
        self.emit(
            key.client,
            Event::Touch {
                surface: key.id,
                serial,
                kind,
                id,
                x: lx,
                y: ly,
            },
        );
    }

    pub fn touch_down(&mut self, id: u32, x: i32, y: i32) {
        self.user_input();
        if self.touches.contains_key(&id) || self.touches.len() >= MAX_TOUCH_POINTS {
            return;
        }
        let serial = self.next_serial();
        self.last_user_serial = serial;
        let hit = self.hit_test(x, y);
        if !self.grab.is_empty() && !hit.is_some_and(|h| self.in_grab(h)) {
            self.dismiss_grab(DismissReason::OutsidePress);
            self.touches.insert(id, None);
            self.last_press = None;
            return;
        }
        let target = hit.filter(|h| {
            let root = self.root_of(*h);
            !(self.grab.is_empty() && *h == root && self.focus_target(root) != root)
        });
        if let Some(h) = hit {
            if self.grab.is_empty() {
                self.activate(h);
            }
        }
        self.touches.insert(id, target);
        if target.is_none() {
            self.last_press = None;
        }
        if let Some(target) = target {
            self.record_press(serial, target, false);
            self.touch_event(target, serial, TouchKind::Down, id, x, y);
        }
    }

    pub fn touch_motion(&mut self, id: u32, x: i32, y: i32) {
        self.user_input();
        if let Some(Some(target)) = self.touches.get(&id).copied() {
            let serial = self.next_serial();
            self.touch_event(target, serial, TouchKind::Motion, id, x, y);
        }
    }

    pub fn touch_up(&mut self, id: u32) {
        self.user_input();
        if let Some(Some(target)) = self.touches.remove(&id) {
            let serial = self.next_serial();
            self.touch_event(target, serial, TouchKind::Up, id, 0, 0);
        }
    }

    pub fn touch_cancel(&mut self) {
        let touches = std::mem::take(&mut self.touches);
        for (id, target) in touches {
            if let Some(target) = target {
                let serial = self.next_serial();
                self.touch_event(target, serial, TouchKind::Cancel, id, 0, 0);
            }
        }
    }

    // ---- frames --------------------------------------------------------------------------

    /// Moves every queued frame to the backend. Releases requested until `end_frame` wait,
    /// because the composition may still read displayed buffers.
    pub fn begin_frame(&mut self) -> usize {
        self.begin(None)
    }

    /// Moves the queued frames of the surfaces on `output` to the backend: what that output's
    /// frame presents, completed by `end_frame_on` at its own flip, whatever the other outputs'
    /// frames do (docs/adr/0018-several-displays.mdx). A surface is on the output it was placed
    /// on; one shown across two is composed on both, and presented with its own.
    pub fn begin_frame_on(&mut self, output: OutputId) -> usize {
        self.begin(Some(output))
    }

    fn begin(&mut self, frame: FrameOf) -> usize {
        let mut started = 0;
        let mut arriving = Vec::new();
        let mut presenting = Vec::new();
        for (key, node) in self.nodes.iter_mut() {
            if frame.is_some_and(|output| node.output != output) {
                continue;
            }
            if !node.presentation.is_presenting()
                && node.presentation.has_queued()
                && node.presentation.begin_present().ok().flatten().is_some()
            {
                node.presenting_in = frame;
                if let Some(frame) = node.presentation.presenting() {
                    presenting.push((*key, frame.sequence));
                }
                // Relative to the frame displayed now: everything the client drew since.
                node.presenting_damage = node.unshown.take().unwrap_or(SurfaceDamage::Whole);
                started += 1;
                // Its first frame: it arrives in the frame being composed.
                if !node.mapped && !node.dismissed {
                    arriving.push(*key);
                }
            }
        }
        for key in arriving {
            self.appeared(key, appearances::Change::Arrived);
        }
        // What the frame may read: an output's, the surfaces shown on it.
        let reads = frame.map(|output| {
            let rect = self.output(output).map(|output| output.rect);
            self.frame_list()
                .into_iter()
                .filter_map(|item| match item {
                    RenderItem::Surface { key, rect: at, .. }
                    | RenderItem::View {
                        window: key,
                        clip: at,
                        ..
                    } if rect.is_some_and(|rect| at.intersects(rect)) => Some(key),
                    _ => None,
                })
                .collect()
        });
        self.frames_in_flight.insert(frame, reads);
        // A shell surface's windows shown live are shown with the frame they were sent with.
        for (key, sequence) in presenting {
            self.present_window_views(key, sequence);
        }
        started
    }

    /// What changed in a surface's frame presenting (between `begin_frame` and `end_frame`),
    /// relative to its frame displayed before, in buffer pixels; `None` for no such surface.
    pub fn presenting_damage(&self, key: SurfaceKey) -> Option<&SurfaceDamage> {
        self.nodes.get(&key).map(|node| &node.presenting_damage)
    }

    /// Completes the frame: in-flight frames become displayed, surfaces with their first ready
    /// content map, replaced buffers are released and feedback carries the timestamp's clock.
    pub fn end_frame(&mut self, timestamp_ns: u64, clock: Clock) {
        self.end(None, timestamp_ns, clock);
    }

    /// Completes `output`'s frame (`begin_frame_on`), as `end_frame` does the scene's.
    pub fn end_frame_on(&mut self, output: OutputId, timestamp_ns: u64, clock: Clock) {
        self.end(Some(output), timestamp_ns, clock);
    }

    fn end(&mut self, frame: FrameOf, timestamp_ns: u64, clock: Clock) {
        let keys: Vec<SurfaceKey> = self
            .nodes
            .iter()
            .filter(|(_, node)| {
                node.presentation.is_presenting()
                    && (frame.is_none() || node.presenting_in == frame)
            })
            .map(|(key, _)| *key)
            .collect();
        // The scene's frame ends every one; an output's, its own.
        match frame {
            None => self.frames_in_flight.clear(),
            Some(_) => {
                self.frames_in_flight.remove(&frame);
            }
        }
        let mut newly_mapped = Vec::new();
        let mut resized = Vec::new();
        let mut answered = Vec::new();
        for key in keys {
            let node = self.nodes.get_mut(&key).expect("present");
            let previous = node.presentation.complete_present().expect("presenting");
            let shown = node.presentation.displayed().expect("just displayed");
            // Answering a configure replaced while it was presented: shown where that one put it.
            if let Some((serial, origin, size, fullscreen)) = node.answering.take() {
                if shown.configure == serial {
                    if (node.size != size || node.origin != origin) && !node.children.is_empty() {
                        resized.push(key);
                    }
                    node.origin = origin;
                    node.size = size;
                    node.shown_fullscreen = fullscreen;
                }
            }
            if let Some((serial, origin, size)) = node.pending {
                if shown.configure == serial {
                    if (node.size != size || node.origin != origin) && !node.children.is_empty() {
                        resized.push(key);
                    }
                    node.origin = origin;
                    node.size = size;
                    node.pending = None;
                    node.shown_fullscreen = node.pending_fullscreen;
                    // The configure in flight is answered: the next an edge resize wants goes.
                    if let Some(wanted) = node.wanted.take() {
                        if wanted != (origin, size) {
                            answered.push((key, wanted));
                        }
                    }
                }
            }
            if !node.mapped {
                node.mapped = true;
                newly_mapped.push(key);
            }
            self.emit(
                key.client,
                Event::Feedback {
                    surface: key.id,
                    sequence: shown.sequence,
                    outcome: Outcome::Presented,
                    timestamp_ns,
                    clock,
                },
            );
            if let Some(previous) = previous {
                self.release(key, previous.buffer);
            }
        }
        // Releases waiting for no frame still in flight go now.
        let mut waiting = Vec::new();
        for (key, buffer, mut frames) in std::mem::take(&mut self.deferred) {
            frames.retain(|of| self.frames_in_flight.contains_key(of));
            if frames.is_empty() {
                self.release_now(key, buffer);
            } else {
                waiting.push((key, buffer, frames));
            }
        }
        self.deferred.extend(waiting);
        for (key, (origin, size)) in answered {
            self.reconfigure(key, Some(origin), size);
        }
        // A parent's new geometry is on screen: re-place popups whose constrained geometry changed.
        for key in resized {
            self.reconstrain_children(key);
        }
        self.refresh_accessibility_geometry();
        self.place_candidates();
        for key in newly_mapped {
            let node = &self.nodes[&key];
            if node.parent.is_none() || node.role == Role::Dialog {
                if self.locked && layer_of(&node.role) == Layer::Lock && self.focus.is_none() {
                    self.set_focus(Some(key));
                } else if matches!(node.role, Role::Toplevel | Role::Dialog) {
                    // Focus-stealing prevention: a window mapped after the user interacted with
                    // another client stays behind the focused window and gets no keyboard focus.
                    let activated = self
                        .activation
                        .get(&key.client)
                        .is_some_and(|serial| self.last_user_serial <= *serial);
                    if activated {
                        self.activate(key);
                    } else {
                        self.lower_below_focus(key);
                    }
                }
            }
        }
        self.refresh_pointer_focus();
    }

    // ---- outputs for the transport/backend -----------------------------------------------

    pub fn drain_events(&mut self) -> Vec<(ClientId, Event)> {
        // The shell is told what changed of windows and workspaces since it was last told.
        self.tell_shell();
        // Then the active window's menus, once it knows the window (ADR 0021).
        self.tell_menu_bar();
        std::mem::take(&mut self.events)
    }

    /// Buffers the backend may now drop: no compositor or output consumer refers to them.
    pub fn drain_retired(&mut self) -> Vec<(ClientId, u64)> {
        std::mem::take(&mut self.retired)
    }

    pub fn focus(&self) -> Option<SurfaceKey> {
        self.focus
    }

    pub fn grab_chain(&self) -> &[SurfaceKey] {
        &self.grab
    }

    pub fn work_areas(&self) -> Vec<(OutputId, Rect)> {
        self.outputs
            .iter()
            .filter_map(|o| Some((o.id, self.work_area(o.id)?)))
            .collect()
    }

    pub fn stats(&self) -> Stats {
        Stats {
            clients: self.clients.len(),
            surfaces: self.nodes.len(),
            popups: self.nodes.values().filter(|n| n.is_popup()).count(),
            held_buffers: self
                .nodes
                .values()
                .map(|n| n.presentation.held_buffers().count())
                .sum(),
            deferred_releases: self.deferred.len(),
            grab_depth: self.grab.len(),
            touch_points: self.touches.len(),
            pending_events: self.events.len(),
            retired_buffers: self.retired.len(),
            registered_buffers: self.buffers.len(),
        }
    }
}

mod accessibility;
pub use accessibility::AccessibleNode;
mod text_input;
pub use text_input::{TextInputState, TextTransaction, MAX_PREEDIT_BYTES, MAX_PREEDIT_SEGMENTS};
mod input_method;
pub use input_method::MAX_PENDING_KEYS;
mod workspaces;
pub use workspaces::MAX_WORKSPACES;
mod shell;
pub use shell::{MAX_APP_ID_BYTES, MAX_TITLE_BYTES};
mod lock;
pub use lock::{Secret, Submitted};
mod launch;
pub use launch::LaunchRequest;
mod preferences;
pub use preferences::{DrawingPreferences, PreferenceChange};
mod appearances;
mod menus;
mod window_views;
pub use window_views::fitted;
mod controls;
/// The screen reader's keys (docs/adr/0014-screen-reader.mdx).
mod reader;
/// The system shortcuts and applications' windows (ADR 0024).
mod shortcuts;
pub use controls::{
    BluetoothStatus, ControlLevel, ControlRequest, HardwareKey, NetworkStatus, PowerStatus,
};
pub mod window;
pub use appearances::{Appearance, Change as AppearanceChange, Kind as AppearanceKind};
pub use menus::{MenuEntry, Menus};

#[cfg(test)]
mod tests;

/// The part of both rectangles, if any.
fn intersect(a: &Rect, b: &Rect) -> Option<Rect> {
    let left = i64::from(a.x).max(i64::from(b.x));
    let top = i64::from(a.y).max(i64::from(b.y));
    let right = (i64::from(a.x) + i64::from(a.width)).min(i64::from(b.x) + i64::from(b.width));
    let bottom = (i64::from(a.y) + i64::from(a.height)).min(i64::from(b.y) + i64::from(b.height));
    (right > left && bottom > top).then(|| {
        Rect::new(
            left as i32,
            top as i32,
            (right - left) as u32,
            (bottom - top) as u32,
        )
    })
}
