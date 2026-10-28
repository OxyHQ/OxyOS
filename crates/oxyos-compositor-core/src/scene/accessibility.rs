//! Accessibility snapshots (ADR 0003): applications publish one tree per surface; clients granted
//! `accessibility` (assistive technologies) receive them with global rectangles and send actions
//! back to the owning application.

use super::{Layer, Scene, SurfaceKey};
use crate::scene::layer_of;
use oxyos_protocol::{
    AccessibleAction, AccessibleField, AccessibleRole, AccessibleState, ErrorCode, Event, Rect,
    TextChunk, MAX_ACCESSIBLE_NODES, MAX_ACCESSIBLE_TEXT_BYTES, NO_ACCESSIBLE_PARENT,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleNode {
    /// Index of the parent node, `NO_ACCESSIBLE_PARENT` for a root. Always before the node.
    pub parent: u32,
    pub role: AccessibleRole,
    pub state: AccessibleState,
    /// Surface-local, logical.
    pub rect: Rect,
    pub name: String,
    pub value: String,
}

/// A surface's accessibility: its committed snapshot, one being received, and the surface's
/// global rectangle when the snapshot was last sent to assistive technologies.
#[derive(Debug, Default)]
pub(super) struct SurfaceAccessibility {
    committed: Option<Vec<AccessibleNode>>,
    pending: Option<Vec<Option<AccessibleNode>>>,
    published: Option<Rect>,
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
        };
        if target.len() + text.as_str().len() > MAX_ACCESSIBLE_TEXT_BYTES {
            return Err(ErrorCode::QuotaExceeded);
        }
        target.push_str(text.as_str());
        Ok(())
    }

    pub(super) fn accessibility_commit(&mut self, key: SurfaceKey) -> Result<(), ErrorCode> {
        let entry = self
            .accessibility
            .get_mut(&key)
            .ok_or(ErrorCode::InvalidOrder)?;
        let pending = entry.pending.take().ok_or(ErrorCode::InvalidOrder)?;
        let nodes: Option<Vec<AccessibleNode>> = pending.into_iter().collect();
        entry.committed = Some(nodes.ok_or(ErrorCode::InvalidOrder)?);
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
    pub(super) fn accessibility_action(
        &mut self,
        target: SurfaceKey,
        index: u32,
        action: AccessibleAction,
    ) {
        let Some(node) = self.live(target) else {
            return;
        };
        if self.locked && layer_of(&node.role) != Layer::Lock {
            return;
        }
        let known = self
            .accessibility
            .get(&target)
            .and_then(|entry| entry.committed.as_ref())
            .is_some_and(|nodes| (index as usize) < nodes.len());
        if known {
            let serial = self.next_serial();
            // Activation is the user's input, as a key press is: a popup it opens may grab with
            // its serial. Moving focus is navigation and authorizes nothing.
            if action == AccessibleAction::Activate {
                self.last_user_serial = serial;
                self.record_press(serial, target, false);
            }
            self.emit(
                target.client,
                Event::AccessibilityAction {
                    surface: target.id,
                    serial,
                    index,
                    action,
                },
            );
        }
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
                        rect,
                    },
                );
                for (field, text) in [
                    (AccessibleField::Name, &node.name),
                    (AccessibleField::Value, &node.value),
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

    /// A surface went away: assistive technologies drop its snapshot.
    pub(super) fn forget_accessibility(&mut self, key: SurfaceKey) {
        if let Some(entry) = self.accessibility.remove(&key) {
            if entry.committed.is_some() {
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
