//! Drag and drop between applications (docs/adr/0035-drag-and-drop.mdx). A source offers media
//! types and actions and starts a drag from a press it is still holding; from then the compositor
//! owns the pointer: the drag's icon follows it, the surface under it (and only that one) is the
//! drag's target, told the drag and its types, never its content, and the action is chosen from
//! both sides' and the modifiers held. Content moves only after a drop, to the target that
//! accepted it, through pipes the source writes (ReceiveDrop, DragSend), as a paste's do.

use super::*;
use crate::scene::launch::assemble;
use oxyos_protocol::{
    DragActions, DragCancelReason, MAX_DRAG_TYPES, MAX_TRASH_FILES_BYTES, MODIFIER_ALT,
    MODIFIER_SUPER,
};
use std::collections::VecDeque;

/// The first protocol version with drag and drop: a client of an older one is never sent a drag
/// event, and its surfaces are no target.
pub const DRAG_VERSION: u16 = 10;
/// Drops whose target has not finished that the compositor keeps open; an older one is pushed out.
pub const MAX_OPEN_DROPS: usize = 8;
/// The Linux input code of Escape, which cancels a drag.
const KEY_ESC: u32 = 1;
/// Requests to put files in the Trash not yet taken by the program.
const MAX_TRASH_REQUESTS: usize = 16;

#[derive(Debug, Default)]
pub(super) struct Drags {
    /// Types offered for a drag not started yet, by client: the drag's number and its types.
    pending: BTreeMap<ClientId, (u32, Vec<TextChunk>)>,
    /// The drag going on.
    current: Option<Drag>,
    /// Drops made whose target has not finished, oldest first.
    dropped: VecDeque<Dropped>,
    /// Names each drag for targets, never reused.
    offers: u32,
    /// Escape cancelled a drag: its release reaches nobody.
    escape_held: bool,
    /// The shell's TrashFiles in chunks, and those received whole.
    trash_receiving: BTreeMap<(ClientId, u32), (String, u16)>,
    trash_requests: Vec<TrashRequest>,
}

#[derive(Debug)]
struct Drag {
    source: ClientId,
    /// The source's number for it.
    drag: u32,
    /// The surface it began from.
    origin: SurfaceKey,
    icon: Option<SurfaceKey>,
    types: Vec<TextChunk>,
    actions: DragActions,
    /// The compositor's name for it, which targets know.
    offer: u32,
    target: Option<Target>,
    /// The action the source was last told.
    told: DragActions,
}

#[derive(Clone, Copy, Debug)]
struct Target {
    surface: SurfaceKey,
    /// The entering's serial, which AcceptDrag cites.
    serial: u64,
    mime: Option<TextChunk>,
    actions: DragActions,
    preferred: DragActions,
    /// The action the target was last told.
    told: DragActions,
}

#[derive(Debug)]
struct Dropped {
    source: ClientId,
    drag: u32,
    offer: u32,
    target: ClientId,
    types: Vec<TextChunk>,
    received: Vec<TextChunk>,
    action: DragActions,
}

/// The shell asked to put files in the Trash (TrashFiles): their URIs, and whether it cited an
/// input it received lately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashRequest {
    pub uris: Vec<String>,
    pub with_input: bool,
}

/// The action a drop would take (ADR 0035, decision 5): the one Option and Command force (a Mac's:
/// Option a copy, Command a move, both a link) if both sides allow it; else the target's preferred
/// one if both allow it; else the first both allow of copy, move and link; none while the target
/// takes no type.
pub fn negotiate_action(
    source: DragActions,
    target: DragActions,
    preferred: DragActions,
    accepts: bool,
    modifiers: u8,
) -> DragActions {
    if !accepts {
        return DragActions::NONE;
    }
    let both = source.intersection(target);
    let forced = match (
        modifiers & MODIFIER_ALT != 0,
        modifiers & MODIFIER_SUPER != 0,
    ) {
        (true, true) => Some(DragActions::LINK),
        (true, false) => Some(DragActions::COPY),
        (false, true) => Some(DragActions::MOVE),
        (false, false) => None,
    };
    if let Some(forced) = forced {
        return if both.contains(forced) {
            forced
        } else {
            DragActions::NONE
        };
    }
    if preferred.is_single() && both.contains(preferred) {
        return preferred;
    }
    [DragActions::COPY, DragActions::MOVE, DragActions::LINK]
        .into_iter()
        .find(|action| both.contains(*action))
        .unwrap_or(DragActions::NONE)
}

impl Scene {
    /// Whether a drag is going on: the pointer is the drag's.
    pub fn dragging(&self) -> bool {
        self.drags.current.is_some()
    }

    /// Whether `client` speaks drag and drop. A client whose version the program never said (a
    /// test's, the program's own) is taken to speak this one.
    fn speaks_drags(&self, client: ClientId) -> bool {
        self.version_of(client)
            .is_none_or(|version| version >= DRAG_VERSION)
    }

    pub(super) fn create_drag_icon(
        &mut self,
        key: SurfaceKey,
        size: Size,
        hotspot: (i32, i32),
    ) -> Result<(), ErrorCode> {
        self.check_new(key)?;
        let (x, y) = self.pointer;
        let output = self
            .output_at(i64::from(x), i64::from(y))
            .or_else(|| self.outputs.first().copied())
            .map_or(0, |output| output.id);
        let origin = icon_origin(self.pointer, hotspot);
        self.insert(key, Role::DragIcon { hotspot }, None, origin, size, output);
        let node = self.nodes.get_mut(&key).expect("inserted");
        // Shown only while the drag that names it goes on, and never hit: the target is what is
        // under the pointer, beneath it.
        node.hidden = true;
        node.input_region = Some(Rect::new(0, 0, 0, 0));
        Ok(())
    }

    pub(super) fn drag_type(&mut self, client: ClientId, drag: u32, mime: TextChunk) {
        let entry = self
            .drags
            .pending
            .entry(client)
            .or_insert_with(|| (drag, Vec::new()));
        if entry.0 != drag {
            *entry = (drag, Vec::new());
        }
        if entry.1.len() < MAX_DRAG_TYPES && !entry.1.contains(&mime) {
            entry.1.push(mime);
        }
    }

    pub(super) fn start_drag(
        &mut self,
        client: ClientId,
        drag: u32,
        surface: SurfaceKey,
        icon: u32,
        serial: u64,
        actions: DragActions,
    ) -> Result<(), ErrorCode> {
        if !self.nodes.contains_key(&surface) {
            return Err(ErrorCode::UnknownSurface);
        }
        let icon = if icon == 0 {
            None
        } else {
            let key = SurfaceKey { client, id: icon };
            let node = self.nodes.get(&key).ok_or(ErrorCode::UnknownSurface)?;
            if !matches!(node.role, Role::DragIcon { .. }) {
                return Err(ErrorCode::InvalidRole);
            }
            Some(key)
        };
        let types = self
            .drags
            .pending
            .remove(&client)
            .filter(|(number, types)| *number == drag && !types.is_empty())
            .map(|(_, types)| types);
        // Only from the latest press of this client on that window, its button still held, with
        // nothing else owning the pointer.
        let pressed = self.last_press.is_some_and(|press| {
            press.serial == serial
                && press.client == client
                && press.pointer
                && self.root_of(press.surface) == self.root_of(surface)
        });
        let valid = pressed
            && self.buttons_down > 0
            && self.visible(surface)
            && self.grab.is_empty()
            && self.drags.current.is_none()
            && self.moving.is_none()
            && self.resizing.is_none()
            && !self.locked
            && !actions.is_empty()
            && icon.is_none_or(|icon| self.drag_icon_free(icon));
        let Some(types) = types.filter(|_| valid) else {
            self.emit(
                client,
                Event::DragCancelled {
                    drag,
                    reason: DragCancelReason::Refused,
                },
            );
            return Ok(());
        };
        // The press is the drag's now: the surface holding it is told it was cancelled, so it
        // never becomes a click, and the pointer leaves it.
        if let Some(captured) = self.capture.take() {
            let serial = self.next_serial();
            self.pointer_event(captured, serial, PointerKind::Cancel, 0);
        }
        self.drags.offers += 1;
        self.drags.current = Some(Drag {
            source: client,
            drag,
            origin: surface,
            icon,
            types,
            actions,
            offer: self.drags.offers,
            target: None,
            told: DragActions::NONE,
        });
        self.set_pointer_focus(None);
        if let Some(icon) = icon {
            self.nodes.get_mut(&icon).expect("checked").hidden = false;
            self.place_drag_icon();
        }
        self.retarget_drag();
        Ok(())
    }

    /// An icon not shown by another drag.
    fn drag_icon_free(&self, icon: SurfaceKey) -> bool {
        self.drags
            .current
            .as_ref()
            .is_none_or(|drag| drag.icon != Some(icon))
    }

    /// The drag's icon at the pointer, on the output under it.
    fn place_drag_icon(&mut self) {
        let Some(icon) = self.drags.current.as_ref().and_then(|drag| drag.icon) else {
            return;
        };
        let Some(node) = self.nodes.get(&icon) else {
            return;
        };
        let Role::DragIcon { hotspot } = node.role else {
            return;
        };
        let origin = icon_origin(self.pointer, hotspot);
        let (x, y) = self.pointer;
        let output = self.output_at(i64::from(x), i64::from(y)).map(|o| o.id);
        let node = self.nodes.get_mut(&icon).expect("present");
        node.origin = origin;
        if let Some((serial, _, size)) = node.pending {
            node.pending = Some((serial, origin, size));
        }
        if let Some(output) = output.filter(|output| *output != node.output) {
            self.move_to_output(icon, output);
        }
    }

    /// The surface under the pointer becomes the drag's target, if it is not already: the one it
    /// left is told so, the one it entered told the drag and its types. True if it changed.
    fn retarget_drag(&mut self) -> bool {
        let Some(drag) = self.drags.current.as_ref() else {
            return false;
        };
        let hit = (!self.locked)
            .then(|| self.hit_test(self.pointer.0, self.pointer.1))
            .flatten()
            .filter(|hit| self.speaks_drags(hit.client));
        let old = drag.target.map(|target| target.surface);
        if hit == old {
            return false;
        }
        let (offer, actions, types) = (drag.offer, drag.actions, drag.types.clone());
        if let Some(old) = old.filter(|old| self.nodes.contains_key(old)) {
            self.emit(
                old.client,
                Event::DragLeave {
                    surface: old.id,
                    offer,
                },
            );
        }
        let target = hit.map(|surface| {
            let serial = self.next_serial();
            let (x, y) = self.local(surface, self.pointer.0, self.pointer.1);
            self.emit(
                surface.client,
                Event::DragEnter {
                    surface: surface.id,
                    offer,
                    serial,
                    x,
                    y,
                    actions,
                    types: types.len() as u8,
                    modifiers: self.modifiers(),
                },
            );
            for mime in &types {
                self.emit(surface.client, Event::DragOfferType { offer, mime: *mime });
            }
            Target {
                surface,
                serial,
                mime: None,
                actions: DragActions::NONE,
                preferred: DragActions::NONE,
                told: DragActions::NONE,
            }
        });
        self.drags.current.as_mut().expect("present").target = target;
        self.negotiate_drag();
        true
    }

    /// Chooses the action again and tells each side whose action changed.
    fn negotiate_drag(&mut self) {
        let modifiers = self.modifiers();
        let Some(drag) = self.drags.current.as_mut() else {
            return;
        };
        let action = drag.target.map_or(DragActions::NONE, |target| {
            negotiate_action(
                drag.actions,
                target.actions,
                target.preferred,
                target.mime.is_some(),
                modifiers,
            )
        });
        let mut events = Vec::new();
        if let Some(target) = drag.target.as_mut().filter(|target| target.told != action) {
            target.told = action;
            events.push((
                target.surface.client,
                Event::DragAction {
                    offer: drag.offer,
                    action,
                },
            ));
        }
        if drag.told != action {
            drag.told = action;
            events.push((
                drag.source,
                Event::DragSourceAction {
                    drag: drag.drag,
                    action,
                },
            ));
        }
        for (client, event) in events {
            self.emit(client, event);
        }
    }

    /// The pointer moved during a drag: the icon follows it, the target may change, and the target
    /// under it is told the motion.
    pub(super) fn drag_motion(&mut self) {
        self.place_drag_icon();
        if self.retarget_drag() {
            return;
        }
        let Some(drag) = self.drags.current.as_ref() else {
            return;
        };
        if let Some(target) = drag.target {
            let offer = drag.offer;
            let (x, y) = self.local(target.surface, self.pointer.0, self.pointer.1);
            self.emit(
                target.surface.client,
                Event::DragMotion {
                    surface: target.surface.id,
                    offer,
                    x,
                    y,
                    time_ns: self.input_time_ns,
                    modifiers: self.modifiers(),
                },
            );
        }
    }

    /// A button during a drag: another press is consumed; the release of the last drops.
    pub(super) fn drag_button(&mut self, serial: u64, pressed: bool) {
        if pressed {
            self.buttons_down += 1;
            return;
        }
        self.buttons_down = self.buttons_down.saturating_sub(1);
        if self.buttons_down == 0 {
            self.drop_drag(serial);
        }
    }

    /// The person released the drag: a drop on a target whose action is not none, else a cancel.
    fn drop_drag(&mut self, serial: u64) {
        let Some(drag) = self.drags.current.take() else {
            return;
        };
        self.hide_drag_icon(&drag);
        let accepted = drag
            .target
            .filter(|target| target.mime.is_some() && !target.told.is_empty());
        match accepted {
            Some(target) => {
                let action = target.told;
                let (x, y) = self.local(target.surface, self.pointer.0, self.pointer.1);
                // The drop is a user input of the target's, as a press is.
                let inputs = self.inputs.entry(target.surface.client).or_default();
                inputs.push_back(serial);
                if inputs.len() > RECENT_INPUTS {
                    inputs.pop_front();
                }
                self.last_user_serial = serial;
                self.emit(
                    target.surface.client,
                    Event::DragDrop {
                        surface: target.surface.id,
                        offer: drag.offer,
                        serial,
                        x,
                        y,
                        action,
                    },
                );
                self.emit(
                    drag.source,
                    Event::DragDropped {
                        drag: drag.drag,
                        action,
                    },
                );
                self.drags.dropped.push_back(Dropped {
                    source: drag.source,
                    drag: drag.drag,
                    offer: drag.offer,
                    target: target.surface.client,
                    types: drag.types,
                    received: Vec::new(),
                    action,
                });
                while self.drags.dropped.len() > MAX_OPEN_DROPS {
                    let old = self.drags.dropped.pop_front().expect("over the bound");
                    self.emit(
                        old.source,
                        Event::DragCancelled {
                            drag: old.drag,
                            reason: DragCancelReason::Abandoned,
                        },
                    );
                }
            }
            None => {
                self.leave_drag_target(&drag);
                self.emit(
                    drag.source,
                    Event::DragCancelled {
                        drag: drag.drag,
                        reason: DragCancelReason::NotAccepted,
                    },
                );
            }
        }
        self.capture = None;
        self.refresh_pointer_focus();
    }

    fn hide_drag_icon(&mut self, drag: &Drag) {
        if let Some(node) = drag.icon.and_then(|icon| self.nodes.get_mut(&icon)) {
            node.hidden = true;
        }
    }

    fn leave_drag_target(&mut self, drag: &Drag) {
        if let Some(target) = drag
            .target
            .filter(|target| self.nodes.contains_key(&target.surface))
        {
            self.emit(
                target.surface.client,
                Event::DragLeave {
                    surface: target.surface.id,
                    offer: drag.offer,
                },
            );
        }
    }

    /// Ends the drag going on without a drop: its target is told it left, its source why. The
    /// buttons still held release nothing.
    pub(super) fn cancel_drag(&mut self, reason: DragCancelReason) {
        let Some(drag) = self.drags.current.take() else {
            return;
        };
        self.hide_drag_icon(&drag);
        self.leave_drag_target(&drag);
        self.emit(
            drag.source,
            Event::DragCancelled {
                drag: drag.drag,
                reason,
            },
        );
        self.swallowed_releases = self.buttons_down;
        self.capture = None;
        if self.buttons_down == 0 {
            self.refresh_pointer_focus();
        }
    }

    /// A key during a drag: Escape cancels it, its press and release reaching nobody; a modifier
    /// may change the action. True if the key is taken.
    pub(super) fn drag_key(&mut self, key: u32, pressed: bool) -> bool {
        if key == KEY_ESC && !pressed && self.drags.escape_held {
            self.drags.escape_held = false;
            return true;
        }
        if self.drags.current.is_none() {
            return false;
        }
        if key == KEY_ESC && pressed {
            self.drags.escape_held = true;
            self.cancel_drag(DragCancelReason::Escape);
            return true;
        }
        self.negotiate_drag();
        false
    }

    pub(super) fn accept_drag(
        &mut self,
        client: ClientId,
        offer: u32,
        serial: u64,
        mime: PreeditText,
        actions: DragActions,
        preferred: DragActions,
    ) {
        let Some(drag) = self.drags.current.as_mut() else {
            return;
        };
        let offered = TextChunk::new(mime.as_str()).filter(|mime| drag.types.contains(mime));
        let Some(target) = drag.target.as_mut().filter(|target| {
            drag.offer == offer && target.surface.client == client && target.serial == serial
        }) else {
            return;
        };
        target.mime = offered;
        target.actions = actions;
        target.preferred = if preferred.is_single() {
            preferred
        } else {
            DragActions::NONE
        };
        self.negotiate_drag();
    }

    /// A target asks for one type of a drop: allowed once per type, for the client dropped on,
    /// before it finished. Its source is sent DragSend, and `take_transfer` names it for the pipe.
    pub(super) fn receive_drop(&mut self, client: ClientId, offer: u32, mime: TextChunk) {
        let Some(dropped) = self.drags.dropped.iter_mut().find(|dropped| {
            dropped.offer == offer && dropped.target == client && dropped.types.contains(&mime)
        }) else {
            return;
        };
        if dropped.received.contains(&mime) {
            return;
        }
        dropped.received.push(mime);
        let (source, drag) = (dropped.source, dropped.drag);
        self.transfer = Some(source);
        self.emit(source, Event::DragSend { drag, mime });
    }

    pub(super) fn finish_drop(&mut self, client: ClientId, offer: u32) {
        let Some(index) = self
            .drags
            .dropped
            .iter()
            .position(|dropped| dropped.offer == offer && dropped.target == client)
        else {
            return;
        };
        let dropped = self.drags.dropped.remove(index).expect("found");
        self.emit(
            dropped.source,
            Event::DragFinished {
                drag: dropped.drag,
                action: dropped.action,
            },
        );
    }

    pub(super) fn cancel_drag_request(&mut self, client: ClientId, drag: u32) {
        if self
            .drags
            .current
            .as_ref()
            .is_some_and(|current| current.source == client && current.drag == drag)
        {
            self.cancel_drag(DragCancelReason::Cancelled);
            return;
        }
        if let Some(index) = self
            .drags
            .dropped
            .iter()
            .position(|dropped| dropped.source == client && dropped.drag == drag)
        {
            self.drags.dropped.remove(index);
            self.emit(
                client,
                Event::DragCancelled {
                    drag,
                    reason: DragCancelReason::Cancelled,
                },
            );
        }
    }

    /// A surface went: the drag's origin cancels it, its icon is no longer shown, its target is
    /// left without a word.
    pub(super) fn forget_drag_surface(&mut self, key: SurfaceKey) {
        let Some(drag) = self.drags.current.as_mut() else {
            return;
        };
        if drag.icon == Some(key) {
            drag.icon = None;
        }
        if drag.target.is_some_and(|target| target.surface == key) {
            drag.target = None;
            self.negotiate_drag();
        }
        if self
            .drags
            .current
            .as_ref()
            .is_some_and(|drag| drag.origin == key)
        {
            self.cancel_drag(DragCancelReason::Cancelled);
        }
    }

    /// A client went: its drag, its drops and its offers end; drops on it are its source's to
    /// forget.
    pub(super) fn forget_drag_client(&mut self, client: ClientId) {
        self.drags.pending.remove(&client);
        self.drags
            .trash_receiving
            .retain(|(owner, _), _| *owner != client);
        if self
            .drags
            .current
            .as_ref()
            .is_some_and(|drag| drag.source == client)
        {
            self.cancel_drag(DragCancelReason::Cancelled);
        } else if let Some(drag) = self.drags.current.as_mut() {
            if drag
                .target
                .is_some_and(|target| target.surface.client == client)
            {
                drag.target = None;
                self.negotiate_drag();
            }
        }
        let gone: VecDeque<Dropped> = {
            let (gone, kept) = std::mem::take(&mut self.drags.dropped)
                .into_iter()
                .partition(|dropped| dropped.source == client || dropped.target == client);
            self.drags.dropped = kept;
            gone
        };
        for dropped in gone.into_iter().filter(|dropped| dropped.source != client) {
            self.emit(
                dropped.source,
                Event::DragCancelled {
                    drag: dropped.drag,
                    reason: DragCancelReason::Abandoned,
                },
            );
        }
    }

    /// The surfaces changed under a still pointer (one appeared, went, moved): the target is what
    /// is under it now.
    pub(super) fn refresh_drag_target(&mut self) {
        if self.drags.current.is_some() {
            self.retarget_drag();
        }
    }

    pub(super) fn trash_files_request(
        &mut self,
        client: ClientId,
        input: u64,
        index: u16,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        if !self.clients[&client].shell {
            return Err(ErrorCode::Unauthorized);
        }
        let Some(list) = assemble(
            &mut self.drags.trash_receiving,
            (client, 0),
            index,
            last,
            text,
            MAX_TRASH_FILES_BYTES,
        )?
        else {
            return Ok(());
        };
        if self.drags.trash_requests.len() >= MAX_TRASH_REQUESTS {
            return Err(ErrorCode::QuotaExceeded);
        }
        let with_input = input != 0 && self.recent_input(client, input);
        let uris = list
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_owned)
            .collect();
        self.drags
            .trash_requests
            .push(TrashRequest { uris, with_input });
        Ok(())
    }

    /// The shell's requests to put files in the Trash since the last call: the program trashes
    /// the files of one that cited an input, and ignores the rest.
    pub fn take_trash_files(&mut self) -> Vec<TrashRequest> {
        std::mem::take(&mut self.drags.trash_requests)
    }
}

/// Where an icon whose hotspot is `hotspot` is, the pointer at `pointer`.
fn icon_origin(pointer: (i32, i32), hotspot: (i32, i32)) -> (i32, i32) {
    (
        pointer.0.saturating_sub(hotspot.0),
        pointer.1.saturating_sub(hotspot.1),
    )
}
