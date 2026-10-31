//! Accessibility snapshots (ADR 0003): applications publish one tree per surface; clients granted
//! `accessibility` (assistive technologies) receive them with global rectangles and send actions
//! back to the owning application. While the session is locked they hold only the lock
//! surfaces' trees, as the screen shows only those (ADR 0010). Applications' announcements reach
//! assistive technologies alone, and, while locked, only the lock screen's.

use super::{Layer, Scene, SurfaceKey};
use crate::scene::layer_of;
use oxyos_protocol::{
    AccessibleAction, AccessibleActions, AccessibleField, AccessibleRole, AccessibleState,
    ErrorCode, Event, Politeness, PreeditText, Rect, TextChunk, MAX_ACCESSIBLE_NODES,
    MAX_ACCESSIBLE_TEXT_BYTES, MAX_ANNOUNCEMENT_BYTES, NO_ACCESSIBLE_PARENT,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleNode {
    /// Index of the parent node, `NO_ACCESSIBLE_PARENT` for a root. Always before the node.
    pub parent: u32,
    pub role: AccessibleRole,
    pub state: AccessibleState,
    /// What it offers beyond activate and focus.
    pub actions: AccessibleActions,
    /// Surface-local, logical.
    pub rect: Rect,
    pub name: String,
    pub value: String,
    /// The labels of its custom actions, one per line.
    pub custom: String,
    /// A text field's selection, in bytes of its value.
    pub selection: Option<(u32, u32)>,
}

impl AccessibleNode {
    /// Its custom actions' labels, in order.
    pub fn custom_actions(&self) -> impl Iterator<Item = &str> {
        self.custom.lines().filter(|label| !label.is_empty())
    }
}

/// An assistive technology's action on a node, with what it names: a custom action's index,
/// a selection's range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Action {
    pub action: AccessibleAction,
    pub custom: u32,
    pub start: u32,
    pub end: u32,
}

/// Whether bytes `start..end` of `text` are a range between characters.
fn text_range(text: &str, start: u32, end: u32) -> bool {
    let (start, end) = (start as usize, end as usize);
    start <= end && end <= text.len() && text.is_char_boundary(start) && text.is_char_boundary(end)
}

/// A surface's accessibility: its committed snapshot, one being received, and the surface's
/// global rectangle when the snapshot was last sent to assistive technologies.
#[derive(Debug, Default)]
pub(super) struct SurfaceAccessibility {
    committed: Option<Vec<AccessibleNode>>,
    pending: Option<Vec<Option<AccessibleNode>>>,
    published: Option<Rect>,
    /// An announcement being received: its text so far, the next chunk's index, how it
    /// interrupts.
    announcing: Option<(String, u8, Politeness)>,
}

impl Scene {
    pub(super) fn accessibility_begin(
        &mut self,
        key: SurfaceKey,
        nodes: u32,
    ) -> Result<(), ErrorCode> {
        self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        if nodes > MAX_ACCESSIBLE_NODES {
            return Err(ErrorCode::QuotaExceeded);
        }
        self.accessibility.entry(key).or_default().pending = Some(vec![None; nodes as usize]);
        Ok(())
    }

    pub(super) fn accessibility_node(
        &mut self,
        key: SurfaceKey,
        index: u32,
        node: AccessibleNode,
    ) -> Result<(), ErrorCode> {
        let pending = self.pending_snapshot(key)?;
        // Parents come first: a tree, never a cycle.
        if node.parent != NO_ACCESSIBLE_PARENT && node.parent >= index {
            return Err(ErrorCode::InvalidMessage);
        }
        let slot = pending
            .get_mut(index as usize)
            .ok_or(ErrorCode::InvalidMessage)?;
        if slot.is_some() {
            return Err(ErrorCode::InvalidMessage);
        }
        *slot = Some(node);
        Ok(())
    }

    pub(super) fn accessibility_text(
        &mut self,
        key: SurfaceKey,
        index: u32,
        field: AccessibleField,
        text: TextChunk,
    ) -> Result<(), ErrorCode> {
        let pending = self.pending_snapshot(key)?;
        let node = pending
            .get_mut(index as usize)
            .and_then(Option::as_mut)
            .ok_or(ErrorCode::InvalidOrder)?;
        let target = match field {
            AccessibleField::Name => &mut node.name,
            AccessibleField::Value => &mut node.value,
            AccessibleField::Actions => &mut node.custom,
        };
        if target.len() + text.as_str().len() > MAX_ACCESSIBLE_TEXT_BYTES {
            return Err(ErrorCode::QuotaExceeded);
        }
        target.push_str(text.as_str());
        Ok(())
    }

    pub(super) fn accessibility_selection(
        &mut self,
        key: SurfaceKey,
        index: u32,
        start: u32,
        end: u32,
    ) -> Result<(), ErrorCode> {
        let pending = self.pending_snapshot(key)?;
        let node = pending
            .get_mut(index as usize)
            .and_then(Option::as_mut)
            .ok_or(ErrorCode::InvalidOrder)?;
        if start > end {
            return Err(ErrorCode::InvalidMessage);
        }
        node.selection = Some((start, end));
        Ok(())
    }

    pub(super) fn accessibility_commit(&mut self, key: SurfaceKey) -> Result<(), ErrorCode> {
        let entry = self
            .accessibility
            .get_mut(&key)
            .ok_or(ErrorCode::InvalidOrder)?;
        let pending = entry.pending.take().ok_or(ErrorCode::InvalidOrder)?;
        let nodes: Option<Vec<AccessibleNode>> = pending.into_iter().collect();
        let nodes = nodes.ok_or(ErrorCode::InvalidOrder)?;
        // A selection is a range of the value it belongs to, between characters.
        let selections = nodes.iter().all(|node| {
            node.selection
                .is_none_or(|(start, end)| text_range(&node.value, start, end))
        });
        if !selections {
            return Err(ErrorCode::InvalidMessage);
        }
        entry.committed = Some(nodes);
        self.publish_accessibility(key);
        Ok(())
    }

    fn pending_snapshot(
        &mut self,
        key: SurfaceKey,
    ) -> Result<&mut Vec<Option<AccessibleNode>>, ErrorCode> {
        self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        self.accessibility
            .get_mut(&key)
            .and_then(|entry| entry.pending.as_mut())
            .ok_or(ErrorCode::InvalidOrder)
    }

    /// An assistive technology's action on another client's node. A target that went away or
    /// a node it no longer has is answered with nothing (the snapshots may have crossed); while
    /// locked only lock surfaces can be acted on.
    /// Only what the node offers is sent: activate and focus on any node, the others where its
    /// snapshot says so, a custom action it names, a selection within its value.
    pub(super) fn accessibility_action(&mut self, target: SurfaceKey, index: u32, action: Action) {
        let Some(node) = self.live(target) else {
            return;
        };
        if self.locked && layer_of(&node.role) != Layer::Lock {
            return;
        }
        let Some(node) = self
            .accessibility
            .get(&target)
            .and_then(|entry| entry.committed.as_ref())
            .and_then(|nodes| nodes.get(index as usize))
        else {
            return;
        };
        let offers = |bit| node.actions.contains(bit);
        let offered = match action.action {
            AccessibleAction::Activate | AccessibleAction::Focus => true,
            AccessibleAction::Increment => offers(AccessibleActions::INCREMENT),
            AccessibleAction::Decrement => offers(AccessibleActions::DECREMENT),
            AccessibleAction::ScrollForward => offers(AccessibleActions::SCROLL_FORWARD),
            AccessibleAction::ScrollBackward => offers(AccessibleActions::SCROLL_BACKWARD),
            AccessibleAction::Dismiss => offers(AccessibleActions::DISMISS),
            AccessibleAction::Custom => (action.custom as usize) < node.custom_actions().count(),
            AccessibleAction::SetSelection => {
                offers(AccessibleActions::SET_SELECTION)
                    && text_range(&node.value, action.start, action.end)
            }
        };
        if !offered {
            return;
        }
        let serial = self.next_serial();
        // An action is the user's input, as a key press is: a popup it opens may grab with its
        // serial. Moving focus is navigation and authorizes nothing.
        if action.action != AccessibleAction::Focus {
            self.last_user_serial = serial;
            self.record_press(serial, target, false);
        }
        let custom = action.action == AccessibleAction::Custom;
        let selection = action.action == AccessibleAction::SetSelection;
        self.emit(
            target.client,
            Event::AccessibilityAction {
                surface: target.id,
                serial,
                index,
                action: action.action,
                custom: if custom { action.custom } else { 0 },
                start: if selection { action.start } else { 0 },
                end: if selection { action.end } else { 0 },
            },
        );
    }

    /// An application's announcement, chunk by chunk: once whole, to every assistive
    /// technology, as Android's announcements reach TalkBack alone; while locked, only the lock
    /// screen's.
    pub(super) fn accessibility_announce(
        &mut self,
        key: SurfaceKey,
        index: u8,
        last: bool,
        politeness: Politeness,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        let entry = self.accessibility.entry(key).or_default();
        if index == 0 {
            entry.announcing = Some((String::new(), 0, politeness));
        }
        let (received, next, _) = entry.announcing.as_mut().ok_or(ErrorCode::InvalidOrder)?;
        if index != *next {
            return Err(ErrorCode::InvalidOrder);
        }
        if received.len() + text.as_str().len() > MAX_ANNOUNCEMENT_BYTES {
            entry.announcing = None;
            return Err(ErrorCode::QuotaExceeded);
        }
        received.push_str(text.as_str());
        *next = next.saturating_add(1);
        if !last {
            return Ok(());
        }
        let (text, _, politeness) = entry.announcing.take().expect("receiving");
        if text.is_empty() || self.behind_lock(key) {
            return Ok(());
        }
        let chunks = TextChunk::split(&text);
        for target in self.assistive_clients() {
            for (index, chunk) in chunks.iter().enumerate() {
                self.emit(
                    target,
                    Event::AccessibilityAnnouncement {
                        client: key.client,
                        surface: key.id,
                        index: index as u8,
                        last: index + 1 == chunks.len(),
                        politeness,
                        text: PreeditText::new(chunk.as_str()).expect("a chunk"),
                    },
                );
            }
        }
        Ok(())
    }

    fn assistive_clients(&self) -> Vec<u32> {
        self.clients
            .iter()
            .filter(|(_, grants)| grants.accessibility)
            .map(|(client, _)| *client)
            .collect()
    }

    /// Sends a surface's committed snapshot to every assistive technology (or `only` one), with
    /// global rectangles.
    pub(super) fn publish_accessibility(&mut self, key: SurfaceKey) {
        self.publish_accessibility_to(key, None);
    }

    fn publish_accessibility_to(&mut self, key: SurfaceKey, only: Option<u32>) {
        // Behind the lock, what a surface shows is not read either.
        if self.behind_lock(key) {
            return;
        }
        let Some(origin) = self.global_rect_pending(key) else {
            return;
        };
        let Some(nodes) = self
            .accessibility
            .get(&key)
            .and_then(|e| e.committed.clone())
        else {
            return;
        };
        let parent = self
            .nodes
            .get(&key)
            .and_then(|node| node.parent)
            // A sheet of another client's window (a file chooser's) is a root of its own client.
            .filter(|parent| parent.client == key.client)
            .map_or(0, |parent| parent.id);
        let targets = match only {
            Some(client) => vec![client],
            None => self.assistive_clients(),
        };
        if only.is_none() {
            self.accessibility.get_mut(&key).expect("present").published = Some(origin);
        }
        for target in targets {
            self.emit(
                target,
                Event::AccessibilityTree {
                    client: key.client,
                    surface: key.id,
                    parent,
                    nodes: nodes.len() as u32,
                },
            );
            for (index, node) in nodes.iter().enumerate() {
                let index = index as u32;
                let rect = Rect::new(
                    origin.x.saturating_add(node.rect.x),
                    origin.y.saturating_add(node.rect.y),
                    node.rect.width,
                    node.rect.height,
                );
                self.emit(
                    target,
                    Event::AccessibleNode {
                        client: key.client,
                        surface: key.id,
                        index,
                        parent: node.parent,
                        role: node.role,
                        state: node.state,
                        actions: node.actions,
                        rect,
                    },
                );
                if let Some((start, end)) = node.selection {
                    self.emit(
                        target,
                        Event::AccessibleSelection {
                            client: key.client,
                            surface: key.id,
                            index,
                            start,
                            end,
                        },
                    );
                }
                for (field, text) in [
                    (AccessibleField::Name, &node.name),
                    (AccessibleField::Value, &node.value),
                    (AccessibleField::Actions, &node.custom),
                ] {
                    for chunk in TextChunk::split(text) {
                        self.emit(
                            target,
                            Event::AccessibleText {
                                client: key.client,
                                surface: key.id,
                                index,
                                field,
                                text: chunk,
                            },
                        );
                    }
                }
            }
            self.emit(
                target,
                Event::AccessibilityCommitted {
                    client: key.client,
                    surface: key.id,
                },
            );
        }
    }

    /// Whether the session is locked and `key` is not part of the lock screen.
    fn behind_lock(&self, key: SurfaceKey) -> bool {
        self.locked
            && self
                .nodes
                .get(&self.root_of(key))
                .is_some_and(|root| layer_of(&root.role) != Layer::Lock)
    }

    /// The session locked: assistive technologies drop every tree but the lock screen's.
    pub(super) fn hide_accessibility_behind_lock(&mut self) {
        let hidden: Vec<SurfaceKey> = self
            .accessibility
            .iter()
            .filter(|(key, entry)| entry.published.is_some() && self.behind_lock(**key))
            .map(|(key, _)| *key)
            .collect();
        let targets = self.assistive_clients();
        for key in hidden {
            self.accessibility.get_mut(&key).expect("present").published = None;
            for target in &targets {
                self.emit(
                    *target,
                    Event::AccessibilityRemoved {
                        client: key.client,
                        surface: key.id,
                    },
                );
            }
        }
    }

    /// The session unlocked: assistive technologies receive the trees the lock hid, as they are
    /// now.
    pub(super) fn show_accessibility_after_lock(&mut self) {
        let shown: Vec<SurfaceKey> = self
            .accessibility
            .iter()
            .filter(|(_, entry)| entry.committed.is_some() && entry.published.is_none())
            .map(|(key, _)| *key)
            .collect();
        for key in shown {
            self.publish_accessibility(key);
        }
    }

    /// A surface went away: assistive technologies drop its snapshot, if they hold it.
    pub(super) fn forget_accessibility(&mut self, key: SurfaceKey) {
        if let Some(entry) = self.accessibility.remove(&key) {
            if entry.committed.is_some() && entry.published.is_some() {
                for target in self.assistive_clients() {
                    self.emit(
                        target,
                        Event::AccessibilityRemoved {
                            client: key.client,
                            surface: key.id,
                        },
                    );
                }
            }
        }
    }

    /// A newly connected assistive technology receives every committed snapshot.
    pub(super) fn replay_accessibility(&mut self, client: u32) {
        let keys: Vec<SurfaceKey> = self
            .accessibility
            .iter()
            .filter(|(_, entry)| entry.committed.is_some())
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            self.publish_accessibility_to(key, Some(client));
        }
    }

    /// After geometry changed (a move, a resize, a re-placed popup): snapshots whose surface
    /// moved are sent again, so assistive technologies keep true global rectangles.
    pub(super) fn refresh_accessibility_geometry(&mut self) {
        if self.assistive_clients().is_empty() {
            return;
        }
        let moved: Vec<SurfaceKey> = self
            .accessibility
            .iter()
            .filter(|(_, entry)| entry.committed.is_some())
            .filter(|(key, entry)| entry.published != self.global_rect_pending(**key))
            .map(|(key, _)| *key)
            .collect();
        for key in moved {
            self.publish_accessibility(key);
        }
    }

    /// The committed snapshot of a surface, as an assistive technology receives it (with
    /// surface-local rectangles).
    pub fn accessibility_snapshot(&self, key: SurfaceKey) -> Option<&[AccessibleNode]> {
        self.accessibility
            .get(&key)
            .and_then(|entry| entry.committed.as_deref())
    }
}
