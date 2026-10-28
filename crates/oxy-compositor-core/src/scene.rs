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
use oxy_protocol::{
    Clock, DismissReason, ErrorCode, Event, Outcome, PanelEdge, PointerKind, Positioner, Rect,
    Request, ShellKind, Size, TouchKind, MAX_DIMENSION,
};
use std::collections::{BTreeMap, BTreeSet};

pub type ClientId = u32;
pub type OutputId = u32;

pub const MAX_CLIENTS: usize = 64;
pub const MAX_SURFACES_PER_CLIENT: usize = 64;
pub const MAX_POPUP_DEPTH: usize = 8;
pub const MAX_OUTPUTS: usize = 16;
pub const MAX_TOUCH_POINTS: usize = 10;
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Layer {
    Background,
    Normal,
    Panel,
    Overlay,
    Notification,
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
    presentation: Presentation,
    scale_120: u32,
    output: OutputId,
    mapped: bool,
    hidden: bool,
    dismissed: bool,
    fullscreen: bool,
    restore: Option<((i32, i32), Size)>,
    /// Local input region; `None` accepts input everywhere, an empty rectangle nowhere.
    input_region: Option<Rect>,
}

impl Node {
    fn is_popup(&self) -> bool {
        matches!(self.role, Role::Popup(_))
    }
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
    last_press: Option<Press>,
    moving: Option<Move>,
    locked: bool,
    shortcuts: BTreeSet<u32>,
    shortcut_target: Option<SurfaceKey>,
    events: Vec<(ClientId, Event)>,
    retired: Vec<(SurfaceKey, u64)>,
    frame_in_flight: bool,
    deferred: Vec<(SurfaceKey, u64)>,
}

fn physical(size: Size, scale_120: u32) -> Size {
    let scale = |logical: u32| {
        let value =
            (u64::from(logical) * u64::from(scale_120)).div_ceil(u64::from(SCALE_DENOMINATOR));
        value.clamp(1, u64::from(MAX_DIMENSION)) as u32
    };
    Size::new(scale(size.width), scale(size.height))
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
        Ok(())
    }

    pub fn outputs(&self) -> &[Output] {
        &self.outputs
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
                Role::Toplevel | Role::Dialog => {
                    if let Some(target) = self.outputs.first().copied() {
                        let size = self.nodes[&key].size;
                        if node.parent.is_none() {
                            let origin = self.centered(size, target.id);
                            self.nodes.get_mut(&key).expect("present").origin = origin;
                        }
                        self.nodes.get_mut(&key).expect("present").output = target.id;
                        self.reconfigure(key, None, size);
                        self.reconstrain_children(key);
                    }
                }
            }
        }
        self.refresh_pointer_focus();
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
            let (origin, size) = match self.nodes[&key].pending {
                Some((_, origin, size)) => (origin, size),
                None => (self.nodes[&key].origin, self.nodes[&key].size),
            };
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

    // ---- clients -------------------------------------------------------------------------

    pub fn connect(&mut self, client: ClientId, grants: Grants) -> Result<(), ErrorCode> {
        if self.clients.contains_key(&client) {
            return Err(ErrorCode::SurfaceExists);
        }
        if self.clients.len() >= MAX_CLIENTS {
            return Err(ErrorCode::QuotaExceeded);
        }
        self.clients.insert(client, grants);
        Ok(())
    }

    /// Removes every resource of the client. Its buffers are retired for the backend; no event
    /// is addressed to it any more.
    pub fn disconnect(&mut self, client: ClientId) {
        if self.clients.remove(&client).is_none() {
            return;
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
        self.events.retain(|(target, _)| *target != client);
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
        if revoked.shortcuts && self.shortcut_target.is_some_and(|key| key.client == client) {
            self.shortcut_target = None;
        }
    }

    /// Keys the compositor routes to the shell before any application sees them.
    pub fn set_shortcuts(&mut self, keys: impl IntoIterator<Item = u32>) {
        self.shortcuts = keys.into_iter().collect();
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
            Request::CreateToplevel { surface, size } => {
                self.check_new(key(surface))?;
                let output = self
                    .output_at(i64::from(self.pointer.0), i64::from(self.pointer.1))
                    .map_or(0, |o| o.id);
                let origin = self.centered(size, output);
                self.insert(key(surface), Role::Toplevel, None, origin, size, output);
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
                if grants.shortcuts && self.shortcut_target.is_none() {
                    self.shortcut_target = Some(key(surface));
                }
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
            Request::SetFullscreen { surface, enabled } => {
                let node = self
                    .nodes
                    .get(&key(surface))
                    .ok_or(ErrorCode::UnknownSurface)?;
                if node.role != Role::Toplevel {
                    return Err(ErrorCode::InvalidRole);
                }
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
                let node = self.nodes.get_mut(&key(surface)).expect("present");
                node.fullscreen = enabled;
                node.restore = if enabled { Some(current) } else { None };
                self.reconfigure(key(surface), Some(origin), size);
                Ok(())
            }
        }
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
                presentation,
                scale_120,
                output,
                mapped: false,
                hidden: false,
                dismissed: false,
                fullscreen: false,
                restore: None,
                input_region: None,
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
        let node = self.nodes.get_mut(&key).expect("present");
        node.scale_120 = scale_120;
        let discarded = node
            .presentation
            .configure(physical(size, scale_120))
            .expect("physical sizes are valid");
        let (serial, _) = node.presentation.configuration();
        node.pending = Some((serial, origin.unwrap_or(node.origin), size));
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
            Layer::Lock | Layer::Panel | Layer::Overlay | Layer::Notification | Layer::Background
        );
        let area = if privileged {
            output.rect
        } else {
            self.work_area(output.id)?
        };
        let placed = place(positioner, origin, area)?;
        let relative = (
            i32::try_from(i64::from(placed.x) - i64::from(origin.0)).ok()?,
            i32::try_from(i64::from(placed.y) - i64::from(origin.1)).ok()?,
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
                match self.placement(key, &positioner) {
                    Some((origin, size, _)) if (origin, size) == current => {}
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

    fn submit(&mut self, key: SurfaceKey, frame: Frame) -> Result<(), ErrorCode> {
        let node = self.nodes.get_mut(&key).ok_or(ErrorCode::UnknownSurface)?;
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
        if self.frame_in_flight {
            self.deferred.push((key, buffer));
            return;
        }
        self.retired.push((key, buffer));
        self.emit(
            key.client,
            Event::Released {
                surface: key.id,
                buffer,
            },
        );
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
        node.dismissed = true;
        let children = node.children.clone();
        for child in children.iter().rev() {
            self.dismiss(*child, reason);
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
        if self.shortcut_target == Some(key) {
            self.shortcut_target = None;
        }
    }

    fn dismiss_grab(&mut self, reason: DismissReason) {
        if let Some(first) = self.grab.first().copied() {
            self.dismiss(first, reason);
        }
    }

    /// Clears input state that refers to `key` without delivering events to it.
    fn detach_input(&mut self, key: SurfaceKey) {
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
        if self.focus == Some(key) {
            self.focus = None;
            self.focus_next();
        }
        if self.focus_before_grab == Some(key) {
            self.focus_before_grab = None;
        }
        if self.shortcut_target == Some(key) {
            self.shortcut_target = None;
        }
        if self.last_press.is_some_and(|press| press.surface == key) {
            self.last_press = None;
        }
    }

    // ---- visibility, stacking and focus ---------------------------------------------------

    fn visible(&self, key: SurfaceKey) -> bool {
        let mut current = key;
        loop {
            let Some(node) = self.nodes.get(&current) else {
                return false;
            };
            if node.dismissed || node.hidden || !node.mapped {
                return false;
            }
            match node.parent {
                Some(parent) => current = parent,
                None => break,
            }
        }
        let layer = layer_of(&self.nodes[&current].role);
        if self.locked {
            layer == Layer::Lock
        } else {
            layer != Layer::Lock
        }
    }

    fn global_origin(&self, key: SurfaceKey) -> Option<(i64, i64)> {
        let node = self.nodes.get(&key)?;
        let (x, y) = (i64::from(node.origin.0), i64::from(node.origin.1));
        match node.parent {
            Some(parent) => {
                let (px, py) = self.global_origin(parent)?;
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
        let (origin, size) = node
            .pending
            .map_or((node.origin, node.size), |(_, o, s)| (o, s));
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

    fn push_tree(&self, key: SurfaceKey, out: &mut Vec<RenderItem>) {
        if !self.visible(key) {
            return;
        }
        let node = &self.nodes[&key];
        if let (Some(rect), Some(frame)) = (self.global_rect(key), node.presentation.displayed()) {
            out.push(RenderItem::Surface { key, rect, frame });
        }
        for child in &node.children {
            self.push_tree(*child, out);
        }
    }

    /// Ready content bottom to top. Only presented frames appear: a surface without one is not
    /// drawn at all, never as an empty rectangle. While locked only lock surfaces and covers.
    pub fn render_list(&self) -> Vec<RenderItem> {
        let mut out = Vec::new();
        for root in &self.roots {
            self.push_tree(*root, &mut out);
        }
        if self.locked {
            for output in &self.outputs {
                let covered = out.iter().any(|item| match item {
                    RenderItem::Surface { key, .. } => {
                        self.nodes[key].role == Role::Lock { output: output.id }
                    }
                    RenderItem::Cover { .. } => false,
                });
                if !covered {
                    out.insert(
                        0,
                        RenderItem::Cover {
                            output: output.id,
                            rect: output.rect,
                        },
                    );
                }
            }
        }
        out
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
                RenderItem::Surface { rect, .. } | RenderItem::Cover { rect, .. } => {
                    rect.intersects(output.rect)
                }
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
                }
            )
    }

    fn set_focus(&mut self, key: Option<SurfaceKey>) {
        if self.focus == key {
            return;
        }
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
        }
        self.focus = key;
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

    pub fn minimize(&mut self, key: SurfaceKey) {
        let Some(node) = self.nodes.get(&key) else {
            return;
        };
        if node.role != Role::Toplevel || node.hidden {
            return;
        }
        let children = node.children.clone();
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
        self.refresh_pointer_focus();
    }

    pub fn restore(&mut self, key: SurfaceKey) {
        if let Some(node) = self.nodes.get_mut(&key) {
            if node.hidden {
                node.hidden = false;
                self.activate(key);
                self.refresh_pointer_focus();
            }
        }
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
    }

    /// Called only from the session authority's trusted authentication path; no protocol
    /// request reaches it.
    pub fn unlock(&mut self) {
        if !self.locked {
            return;
        }
        self.locked = false;
        self.pointer_cancel();
        self.touch_cancel();
        self.set_focus(None);
        self.focus_next();
        self.refresh_pointer_focus();
    }

    pub fn is_locked(&self) -> bool {
        self.locked
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
        if self.moving.is_some() {
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
        self.pointer = (x, y);
        if let Some(moving) = self.moving {
            self.move_window(moving, x, y);
            return;
        }
        let target = self.pointer_target();
        self.set_pointer_focus(target);
        if let Some(target) = target {
            let serial = self.next_serial();
            self.pointer_event(target, serial, PointerKind::Motion, 0);
        }
    }

    /// Compositor-owned move: no client round trip, so a stalled client still moves smoothly.
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
        if let Some((serial, pending, size)) = node.pending {
            node.pending = Some((
                serial,
                (
                    (i64::from(pending.0) + delta.0) as i32,
                    (i64::from(pending.1) + delta.1) as i32,
                ),
                size,
            ));
        }
        if let Some(output) = self.output_at(i64::from(origin.0), i64::from(origin.1)) {
            self.nodes.get_mut(&moving.surface).expect("present").output = output.id;
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
        let serial = self.next_serial();
        if pressed {
            self.buttons_down += 1;
            let hit = self.hit_test(self.pointer.0, self.pointer.1);
            if !self.grab.is_empty() && !hit.is_some_and(|h| self.in_grab(h)) {
                // Outside press: dismiss the chain and consume the press and its release.
                self.dismiss_grab(DismissReason::OutsidePress);
                self.swallowed_releases += 1;
                self.refresh_pointer_focus();
                return;
            }
            let Some(target) = self.capture.or(hit) else {
                self.swallowed_releases += 1;
                return;
            };
            if self.grab.is_empty() {
                let root = self.root_of(target);
                let blocked = target == root && self.focus_target(root) != root;
                self.activate(target);
                if blocked {
                    // Modal scope: the window behind a dialog does not receive the press.
                    self.swallowed_releases += 1;
                    return;
                }
                if self.focusable(target) && self.root_of(target) == root && target != root {
                    self.set_focus(Some(target));
                }
            }
            self.capture = Some(target);
            self.set_pointer_focus(Some(target));
            self.last_press = Some(Press {
                serial,
                client: target.client,
                surface: target,
                pointer: true,
            });
            self.pointer_event(target, serial, PointerKind::Press, button);
        } else {
            self.buttons_down = self.buttons_down.saturating_sub(1);
            if let Some(moving) = self.moving {
                if self.buttons_down == 0 {
                    self.moving = None;
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

    /// Pointer capture lost (for example a compositor gesture took over): the captured surface
    /// receives Cancel and no later release is delivered as a click.
    pub fn pointer_cancel(&mut self) {
        if let Some(target) = self.capture.take() {
            let serial = self.next_serial();
            self.pointer_event(target, serial, PointerKind::Cancel, 0);
        }
        self.swallowed_releases = self.buttons_down;
        self.moving = None;
    }

    pub fn key(&mut self, key: u32, pressed: bool) {
        let serial = self.next_serial();
        if !self.locked && self.shortcuts.contains(&key) {
            if let Some(target) = self.shortcut_target.filter(|t| self.live(*t).is_some()) {
                self.emit(
                    target.client,
                    Event::Key {
                        surface: target.id,
                        serial,
                        key,
                        pressed,
                    },
                );
                return;
            }
        }
        let Some(target) = self.focus.filter(|f| self.visible(*f)) else {
            return;
        };
        if pressed {
            self.last_press = Some(Press {
                serial,
                client: target.client,
                surface: target,
                pointer: false,
            });
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
        if self.touches.contains_key(&id) || self.touches.len() >= MAX_TOUCH_POINTS {
            return;
        }
        let serial = self.next_serial();
        let hit = self.hit_test(x, y);
        if !self.grab.is_empty() && !hit.is_some_and(|h| self.in_grab(h)) {
            self.dismiss_grab(DismissReason::OutsidePress);
            self.touches.insert(id, None);
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
        if let Some(target) = target {
            self.last_press = Some(Press {
                serial,
                client: target.client,
                surface: target,
                pointer: false,
            });
            self.touch_event(target, serial, TouchKind::Down, id, x, y);
        }
    }

    pub fn touch_motion(&mut self, id: u32, x: i32, y: i32) {
        if let Some(Some(target)) = self.touches.get(&id).copied() {
            let serial = self.next_serial();
            self.touch_event(target, serial, TouchKind::Motion, id, x, y);
        }
    }

    pub fn touch_up(&mut self, id: u32) {
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
        self.frame_in_flight = true;
        let mut started = 0;
        for node in self.nodes.values_mut() {
            if !node.presentation.is_presenting()
                && node.presentation.has_queued()
                && node.presentation.begin_present().ok().flatten().is_some()
            {
                started += 1;
            }
        }
        started
    }

    /// Completes the frame: in-flight frames become displayed, surfaces with their first ready
    /// content map, replaced buffers are released and feedback carries the timestamp's clock.
    pub fn end_frame(&mut self, timestamp_ns: u64, clock: Clock) {
        let keys: Vec<SurfaceKey> = self
            .nodes
            .iter()
            .filter(|(_, node)| node.presentation.is_presenting())
            .map(|(key, _)| *key)
            .collect();
        self.frame_in_flight = false;
        let mut newly_mapped = Vec::new();
        for key in keys {
            let node = self.nodes.get_mut(&key).expect("present");
            let previous = node.presentation.complete_present().expect("presenting");
            let shown = node.presentation.displayed().expect("just displayed");
            if let Some((serial, origin, size)) = node.pending {
                if shown.configure == serial {
                    node.origin = origin;
                    node.size = size;
                    node.pending = None;
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
        for (key, buffer) in std::mem::take(&mut self.deferred) {
            self.release(key, buffer);
        }
        for key in newly_mapped {
            let node = &self.nodes[&key];
            if node.parent.is_none() || node.role == Role::Dialog {
                if self.locked && layer_of(&node.role) == Layer::Lock && self.focus.is_none() {
                    self.set_focus(Some(key));
                } else if matches!(node.role, Role::Toplevel | Role::Dialog) {
                    self.activate(key);
                }
            }
        }
        self.refresh_pointer_focus();
    }

    // ---- outputs for the transport/backend -----------------------------------------------

    pub fn drain_events(&mut self) -> Vec<(ClientId, Event)> {
        std::mem::take(&mut self.events)
    }

    /// Buffers the backend may now drop: no compositor or output consumer refers to them.
    pub fn drain_retired(&mut self) -> Vec<(SurfaceKey, u64)> {
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
        }
    }
}

#[cfg(test)]
mod tests;
