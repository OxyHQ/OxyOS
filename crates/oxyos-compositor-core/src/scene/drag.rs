//! Drag and drop between applications (docs/adr/0035-drag-and-drop.mdx). A source offers media
//! types and actions and starts a drag from a press it is still holding; from then the compositor
//! owns the pointer: the drag's icon follows it, the surface under it (and only that one) is the
//! drag's target, told the drag and its types, never its content, and the action is chosen from
//! both sides' and the modifiers held. Content moves only after a drop, to the target that
//! accepted it, through pipes the source writes (ReceiveDrop, DragSend), as a paste's do.

use super::dock::PING_TIMEOUT;
use super::*;
use crate::scene::launch::assemble;
use oxyos_protocol::{
    DragActions, DragCancelReason, MAX_DRAG_TYPES, MAX_FILE_TYPE_BYTES, MAX_FILE_VOLUME_BYTES,
    MAX_OPEN_FILES_BYTES, MAX_TRASH_FILES_BYTES, MODIFIER_ALT, MODIFIER_SUPER,
};
use std::collections::VecDeque;
use std::time::Duration;

/// The first protocol version with drag and drop: a client of an older one is never sent a drag
/// event, and its surfaces are no target.
pub const DRAG_VERSION: u16 = 10;
/// Drops whose target has not finished that the compositor keeps open; an older one is pushed out.
pub const MAX_OPEN_DROPS: usize = 8;
/// The Linux input code of Escape, which cancels a drag.
const KEY_ESC: u32 = 1;
/// The badge a drag's pointer shows for a copy or a link (`RenderItem::DragBadge`): its side and
/// where its top left is from the pointer's tip, in logical pixels, below and right of the arrow
/// as a Mac's drag cursors have it.
pub const DRAG_BADGE_SIDE: u32 = 16;
pub const DRAG_BADGE_OFFSET: (i32, i32) = (9, 12);
/// Requests to put files in the Trash not yet taken by the program.
const MAX_TRASH_REQUESTS: usize = 16;
/// Requests to open files with an application not yet taken by the program.
const MAX_OPEN_REQUESTS: usize = 16;
/// The first protocol version whose drags may vanish and say their files' types (ADR 0035).
pub const VANISH_VERSION: u16 = 26;
/// The first protocol version whose drags say the volumes their files are on
/// (docs/adr/0047-finder-drags-and-the-desktop.mdx): a target of an older one is never told them.
pub const VOLUMES_VERSION: u16 = 28;

#[derive(Debug, Default)]
pub(super) struct Drags {
    /// What a client said of a drag not started yet: its number, its types and the rest.
    pending: BTreeMap<ClientId, Pending>,
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
    /// The shell's OpenFiles in chunks, and those received whole.
    open_receiving: BTreeMap<(ClientId, u32), (String, u16)>,
    open_requests: Vec<OpenFilesRequest>,
    /// A dragged file's type in chunks (DragFileType), by client.
    file_type_receiving: BTreeMap<(ClientId, u32), (String, u16)>,
    /// A dragged file's volume in chunks (DragFileVolume), by client.
    file_volume_receiving: BTreeMap<(ClientId, u32), (String, u16)>,
    /// The drag released over a target that has not yet answered where it was released.
    releasing: Option<Releasing>,
}

/// A drag released over a target, decided once the target has answered every event it was sent
/// before the release (a Ping after them, answered): its answer to the last motion, which it may
/// not have read when the person let go, is the one a drop on it is decided by, as a Mac's drag
/// asks its destination where it is dropped. A target that does not answer in a Ping's time is
/// decided by what it said last.
#[derive(Debug)]
struct Releasing {
    /// The release's serial, the drop's.
    serial: u64,
    target: ClientId,
    ping: u32,
    sent: Duration,
}

/// What a source said of a drag before starting it: the types it offers, its files' types
/// (DragFileType) and volumes (DragFileVolume), and whether it vanishes dropped on nothing
/// (DragVanish).
#[derive(Debug, Default)]
struct Pending {
    drag: u32,
    types: Vec<TextChunk>,
    file_types: Vec<String>,
    file_volumes: Vec<String>,
    vanish: bool,
}

#[derive(Debug)]
struct Drag {
    source: ClientId,
    /// The source's number for it.
    drag: u32,
    /// The surface it began from.
    origin: SurfaceKey,
    icon: Option<SurfaceKey>,
    /// Where its icon was as the press began (global, logical): the dragged item's own place,
    /// the grip under the pointer. A drop refused slides the image back there.
    home: (i32, i32),
    types: Vec<TextChunk>,
    /// The media types of the files it carries, as its source declares them.
    file_types: Vec<String>,
    /// The volumes the files it carries are on, as its source declares them.
    file_volumes: Vec<String>,
    /// Dropped on nothing that takes it, its image vanishes in a poof where it is.
    vanish: bool,
    actions: DragActions,
    /// The compositor's name for it, which targets know.
    offer: u32,
    target: Option<Target>,
    /// The action the source was last told.
    told: DragActions,
    /// Where the person let go of it (global, logical): it stays there until it is decided,
    /// whatever the pointer does meanwhile, and its target and the drop's point are there.
    let_go: Option<(i32, i32)>,
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

/// The shell asked to open files with an application (OpenFiles): the application's id, the files'
/// URIs, and whether it cited an input it received lately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenFilesRequest {
    pub application: String,
    pub uris: Vec<String>,
    pub with_input: bool,
}

/// How a drag's image leaves the screen as the drag ends: taken by a target (at once), back to
/// where it came from (a Mac's slide), or vanished in a poof where it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ending {
    Taken,
    Returns,
    Vanishes,
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

    /// Every drag icon surface there is, of any client, with where it is shown (global, logical;
    /// none while hidden): a drag shows one, and its source destroys it once the drag ended.
    pub fn drag_icons(&self) -> Vec<(SurfaceKey, Option<Rect>)> {
        self.nodes
            .iter()
            .filter(|(_, node)| matches!(node.role, Role::DragIcon { .. }))
            .map(|(key, node)| {
                let shown = node.mapped && !node.hidden && !node.dismissed;
                (*key, shown.then(|| self.global_rect(*key)).flatten())
            })
            .collect()
    }

    /// The badge the drag's pointer shows, a Mac's: a copy's plus, a link's arrow, from the action
    /// negotiated with the target under it (Option copies, Option and Command link); none for a
    /// move or a drop that would be refused.
    pub(super) fn drag_badge(&self) -> Option<RenderItem> {
        let drag = self.drags.current.as_ref()?;
        if self.locked || !(drag.told == DragActions::COPY || drag.told == DragActions::LINK) {
            return None;
        }
        let (x, y) = self.drag_point();
        Some(RenderItem::DragBadge {
            rect: Rect::new(
                x.saturating_add(DRAG_BADGE_OFFSET.0),
                y.saturating_add(DRAG_BADGE_OFFSET.1),
                DRAG_BADGE_SIDE,
                DRAG_BADGE_SIDE,
            ),
            action: drag.told,
        })
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

    /// What `client` says of its drag `drag` before starting it; a number other than the one
    /// it spoke of begins another.
    fn pending(&mut self, client: ClientId, drag: u32) -> &mut Pending {
        let entry = self.drags.pending.entry(client).or_insert_with(|| Pending {
            drag,
            ..Pending::default()
        });
        if entry.drag != drag {
            *entry = Pending {
                drag,
                ..Pending::default()
            };
        }
        entry
    }

    pub(super) fn drag_type(&mut self, client: ClientId, drag: u32, mime: TextChunk) {
        let entry = self.pending(client, drag);
        if entry.types.len() < MAX_DRAG_TYPES && !entry.types.contains(&mime) {
            entry.types.push(mime);
        }
    }

    /// The media type of a file the drag carries (DragFileType), in chunks; kept distinct, at
    /// most MAX_DRAG_TYPES.
    pub(super) fn drag_file_type(
        &mut self,
        client: ClientId,
        drag: u32,
        index: u16,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        let Some(mime) = assemble(
            &mut self.drags.file_type_receiving,
            (client, drag),
            index,
            last,
            text,
            MAX_FILE_TYPE_BYTES,
        )?
        else {
            return Ok(());
        };
        let entry = self.pending(client, drag);
        if !mime.is_empty()
            && entry.file_types.len() < MAX_DRAG_TYPES
            && !entry.file_types.contains(&mime)
        {
            entry.file_types.push(mime);
        }
        Ok(())
    }

    /// A volume a file the drag carries is on (DragFileVolume), in chunks, as its files' types
    /// are said; kept distinct, at most MAX_DRAG_TYPES.
    pub(super) fn drag_file_volume(
        &mut self,
        client: ClientId,
        drag: u32,
        index: u16,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        let Some(volume) = assemble(
            &mut self.drags.file_volume_receiving,
            (client, drag),
            index,
            last,
            text,
            MAX_FILE_VOLUME_BYTES,
        )?
        else {
            return Ok(());
        };
        let entry = self.pending(client, drag);
        if !volume.is_empty()
            && entry.file_volumes.len() < MAX_DRAG_TYPES
            && !entry.file_volumes.contains(&volume)
        {
            entry.file_volumes.push(volume);
        }
        Ok(())
    }

    /// The drag, dropped on nothing that takes it, vanishes where it is (DragVanish).
    pub(super) fn drag_vanish(&mut self, client: ClientId, drag: u32) {
        self.pending(client, drag).vanish = true;
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
        let hotspot = icon.and_then(|icon| match self.nodes[&icon].role {
            Role::DragIcon { hotspot } => Some(hotspot),
            _ => None,
        });
        let said = self
            .drags
            .pending
            .remove(&client)
            .filter(|said| said.drag == drag && !said.types.is_empty());
        // Only from the latest press of this client on that window, with nothing else owning the
        // pointer: its button still held, or released since and nothing pressed after it. A
        // client reads a press, its motions and its release in one turn when the machine is
        // loaded, and asks for the drag only then: as a Mac's drag session takes the mouse's
        // events queued after the one it began from, the drag is dropped where the press was
        // released (ADR 0035, decision 2).
        let press = self.last_press.filter(|press| {
            press.serial == serial
                && press.client == client
                && press.pointer
                && self.root_of(press.surface) == self.root_of(surface)
        });
        let let_go = press
            .and_then(|press| press.released)
            .filter(|_| self.buttons_down == 0);
        let valid = press.is_some()
            && (self.buttons_down > 0 || let_go.is_some())
            && self.visible(surface)
            && self.grab.is_empty()
            && self.drags.current.is_none()
            && self.moving.is_none()
            && self.resizing.is_none()
            && !self.locked
            && !actions.is_empty()
            && icon.is_none_or(|icon| self.drag_icon_free(icon));
        let pressed_at = self.last_press.map_or(self.pointer, |press| press.at);
        let Some(said) = said.filter(|_| valid) else {
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
            home: icon_origin(pressed_at, hotspot.unwrap_or_default()),
            types: said.types,
            file_types: said.file_types,
            file_volumes: said.file_volumes,
            vanish: said.vanish,
            actions,
            offer: self.drags.offers,
            target: None,
            told: DragActions::NONE,
            let_go: let_go.map(|(at, _)| at),
        });
        self.set_pointer_focus(None);
        if let Some(icon) = icon {
            self.nodes.get_mut(&icon).expect("checked").hidden = false;
            self.place_drag_icon();
        }
        self.retarget_drag();
        // Released already: it enters the surface where it was let go and is dropped there, once
        // that surface has answered (`Releasing`).
        if let Some((_, release)) = let_go {
            self.drop_drag(release);
        }
        Ok(())
    }

    /// The client the latest pointer press was delivered to, while its button is held (a drag it
    /// may start from it, a scenario's hand waiting for it to take the motions).
    pub fn holding_press(&self) -> Option<ClientId> {
        self.last_press
            .filter(|press| press.pointer && press.released.is_none() && self.buttons_down > 0)
            .map(|press| press.client)
    }

    /// The client of the surface the drag going on is over (its target), if any.
    pub fn drag_target(&self) -> Option<ClientId> {
        self.drags
            .current
            .as_ref()
            .and_then(|drag| drag.target)
            .map(|target| target.surface.client)
    }

    /// Where the drag is (global, logical): at the pointer, or where the person let go of it.
    fn drag_point(&self) -> (i32, i32) {
        self.drags
            .current
            .as_ref()
            .and_then(|drag| drag.let_go)
            .unwrap_or(self.pointer)
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
        let at = self.drag_point();
        let origin = icon_origin(at, hotspot);
        let (x, y) = at;
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

    /// The surface under the drag (`drag_point`) becomes its target, if it is not already: the
    /// one it left is told so, the one it entered told the drag and its types. True if it changed.
    fn retarget_drag(&mut self) -> bool {
        let at = self.drag_point();
        let Some(drag) = self.drags.current.as_ref() else {
            return false;
        };
        let hit = (!self.locked)
            .then(|| self.hit_test(at.0, at.1))
            .flatten()
            .filter(|hit| self.speaks_drags(hit.client));
        let old = drag.target.map(|target| target.surface);
        if hit == old {
            return false;
        }
        let (offer, actions, types, file_types, file_volumes) = (
            drag.offer,
            drag.actions,
            drag.types.clone(),
            drag.file_types.clone(),
            drag.file_volumes.clone(),
        );
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
            // Its files' volumes only to a target that knows them (version 28): an older one is
            // told none, and its count of them is left out of its DragEnter anyway.
            let volumes: &[String] = if self
                .version_of(surface.client)
                .is_none_or(|version| version >= VOLUMES_VERSION)
            {
                &file_volumes
            } else {
                &[]
            };
            let serial = self.next_serial();
            let (x, y) = self.local(surface, at.0, at.1);
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
                    file_types: file_types.len() as u8,
                    file_volumes: volumes.len() as u8,
                },
            );
            for mime in &types {
                self.emit(surface.client, Event::DragOfferType { offer, mime: *mime });
            }
            // Its files' types after (version 26: an older target is never told them, and reads
            // the count as none).
            for mime in &file_types {
                let chunks = TextChunk::split(mime);
                let count = chunks.len();
                for (index, chunk) in chunks.iter().enumerate() {
                    self.emit(
                        surface.client,
                        Event::DragOfferFileType {
                            offer,
                            index: index as u16,
                            last: index + 1 == count,
                            text: PreeditText::new(chunk.as_str()).expect("a chunk fits"),
                        },
                    );
                }
            }
            // And their volumes after those (version 28).
            for volume in volumes {
                let chunks = TextChunk::split(volume);
                let count = chunks.len();
                for (index, chunk) in chunks.iter().enumerate() {
                    self.emit(
                        surface.client,
                        Event::DragOfferFileVolume {
                            offer,
                            index: index as u16,
                            last: index + 1 == count,
                            text: PreeditText::new(chunk.as_str()).expect("a chunk fits"),
                        },
                    );
                }
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
    /// under it is told the motion. Released, the drag stays where it was let go.
    pub(super) fn drag_motion(&mut self) {
        if self.released() {
            return;
        }
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

    /// The person released the drag: decided once its target has answered where it was released
    /// (`Releasing`), at once without one.
    fn drop_drag(&mut self, serial: u64) {
        if self.drags.releasing.is_some() {
            return;
        }
        let pointer = self.pointer;
        if let Some(drag) = self.drags.current.as_mut() {
            drag.let_go.get_or_insert(pointer);
        }
        let target = self
            .drags
            .current
            .as_ref()
            .and_then(|drag| drag.target)
            .map(|target| target.surface.client);
        if let Some(target) = target {
            if let Some(ping) = self.ping_after_events(target) {
                self.drags.releasing = Some(Releasing {
                    serial,
                    target,
                    ping,
                    sent: self.clock,
                });
                return;
            }
        }
        self.decide_drop(serial);
    }

    /// The released drag's target answered the Ping sent after the release: it is decided now.
    pub(super) fn drop_answered(&mut self, client: ClientId, ping: u32) {
        if self
            .drags
            .releasing
            .as_ref()
            .is_some_and(|releasing| releasing.target == client && releasing.ping == ping)
        {
            let releasing = self.drags.releasing.take().expect("checked");
            self.decide_drop(releasing.serial);
        }
    }

    /// The released drag's target did not answer in a Ping's time: it is decided by what it said
    /// last.
    pub(super) fn drop_unanswered(&mut self) {
        if self
            .drags
            .releasing
            .as_ref()
            .is_some_and(|releasing| self.clock.saturating_sub(releasing.sent) >= PING_TIMEOUT)
        {
            let releasing = self.drags.releasing.take().expect("checked");
            self.decide_drop(releasing.serial);
        }
    }

    /// When the released drag's target is late, after the clock.
    pub(super) fn drop_deadline(&self) -> Option<Duration> {
        self.drags
            .releasing
            .as_ref()
            .map(|releasing| (releasing.sent + PING_TIMEOUT).saturating_sub(self.clock))
    }

    /// Whether the drag was released and waits for its target's answer: the pointer no longer
    /// moves it.
    fn released(&self) -> bool {
        self.drags.releasing.is_some()
    }

    /// The released drag: a drop on a target whose action is not none, else a cancel.
    fn decide_drop(&mut self, serial: u64) {
        self.drags.releasing = None;
        let let_go = self.drag_point();
        let Some(drag) = self.drags.current.take() else {
            return;
        };
        let accepted = drag
            .target
            .filter(|target| target.mime.is_some() && !target.told.is_empty());
        // Taken, the image is gone at once; refused, it slides back to where it came from, or
        // vanishes in a poof where it is if its source said so.
        let ending = match (accepted.is_some(), drag.vanish) {
            (true, _) => Ending::Taken,
            (false, false) => Ending::Returns,
            (false, true) => Ending::Vanishes,
        };
        self.hide_drag_icon(&drag, ending);
        match accepted {
            Some(target) => {
                let action = target.told;
                let (x, y) = self.local(target.surface, let_go.0, let_go.1);
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

    /// The drag's icon is no longer shown. A drag that dropped nothing goes back to where it came
    /// from as a Mac's does: the compositor slides it from its last frame to its home, fading
    /// (`Change::Returned`); one its source said vanishes, dropped on nothing, puffs away where it
    /// is (`Change::Vanished`, a Mac's poof).
    fn hide_drag_icon(&mut self, drag: &Drag, ending: Ending) {
        let Some(icon) = drag.icon else {
            return;
        };
        let Some(node) = self.nodes.get_mut(&icon) else {
            return;
        };
        let shown = node.mapped && !node.hidden && !node.dismissed;
        node.hidden = true;
        let (size, origin) = (node.size, node.origin);
        let centre = |at: (i32, i32)| {
            (
                at.0 + (size.width / 2) as i32,
                at.1 + (size.height / 2) as i32,
            )
        };
        if !shown {
            return;
        }
        match ending {
            Ending::Taken => {}
            Ending::Returns => {
                let (x, y) = centre(drag.home);
                self.appeared(icon, appearances::Change::Returned { x, y });
            }
            Ending::Vanishes => {
                let (x, y) = centre(origin);
                self.appeared(icon, appearances::Change::Vanished { x, y });
            }
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
        self.drags.releasing = None;
        let Some(drag) = self.drags.current.take() else {
            return;
        };
        // Cancelled by the person or the source, the image goes back; the lock ends it at once.
        let ending = if reason == DragCancelReason::Locked {
            Ending::Taken
        } else {
            Ending::Returns
        };
        self.hide_drag_icon(&drag, ending);
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
        self.drags
            .open_receiving
            .retain(|(owner, _), _| *owner != client);
        self.drags
            .file_type_receiving
            .retain(|(owner, _), _| *owner != client);
        self.drags
            .file_volume_receiving
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
            // Released over it, the drag has no target to wait for now.
            if let Some(releasing) = self
                .drags
                .releasing
                .take_if(|releasing| releasing.target == client)
            {
                self.decide_drop(releasing.serial);
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
        if self.drags.current.is_some() && !self.released() {
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

    /// OpenFiles (the shell's, ADR 0034): the application's id on the first line, then the files'
    /// URIs. Received whole, it is kept for the program with whether it cited an input of the
    /// shell's.
    pub(super) fn open_files_request(
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
            &mut self.drags.open_receiving,
            (client, 0),
            index,
            last,
            text,
            MAX_OPEN_FILES_BYTES,
        )?
        else {
            return Ok(());
        };
        if self.drags.open_requests.len() >= MAX_OPEN_REQUESTS {
            return Err(ErrorCode::QuotaExceeded);
        }
        let with_input = input != 0 && self.recent_input(client, input);
        let mut lines = list.lines().map(str::trim);
        let application = lines.next().unwrap_or_default().to_owned();
        let uris = lines
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_owned)
            .collect();
        self.drags.open_requests.push(OpenFilesRequest {
            application,
            uris,
            with_input,
        });
        Ok(())
    }

    /// The shell's requests to open files with an application since the last call: the program
    /// opens those of one that cited an input, and ignores the rest.
    pub fn take_open_files(&mut self) -> Vec<OpenFilesRequest> {
        std::mem::take(&mut self.drags.open_requests)
    }
}

/// Where an icon whose hotspot is `hotspot` is, the pointer at `pointer`.
fn icon_origin(pointer: (i32, i32), hotspot: (i32, i32)) -> (i32, i32) {
    (
        pointer.0.saturating_sub(hotspot.0),
        pointer.1.saturating_sub(hotspot.1),
    )
}
