//! The assistive technology the development compositor plays (a screen reader): a client of the
//! scene only, granted `accessibility`, holding the trees the compositor sends it as it receives
//! them (ADR 0003). What a step reports and acts on is what it holds, not the compositor's own
//! state: a tree the compositor keeps from it (behind the lock, ADR 0010) is not there.
//!
//! What it would say is kept as announcements: an application's (`AccessibilityAnnouncement`),
//! and a live region's text as it changes between two trees of a surface, as TalkBack announces
//! Android's live regions.

use oxyos_compositor::Compositor;
use oxyos_compositor_core::{ClientId, SurfaceKey};
use oxyos_protocol::{
    AccessibleActions, AccessibleField, AccessibleRole, AccessibleState, Event, Politeness,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Node {
    pub role: AccessibleRole,
    pub state: AccessibleState,
    pub actions: AccessibleActions,
    pub name: String,
    pub value: String,
    /// Its custom actions' labels.
    pub custom: Vec<String>,
    pub selection: Option<(u32, u32)>,
    /// What activating it does, when its name does not say (version 13).
    pub hint: String,
    /// Its place among its collection's items, 1-based, and their count (version 13).
    pub position: Option<(u32, u32)>,
    /// Its parent's index in the same tree, as the application nested it; None at the top.
    pub parent: Option<u32>,
    /// Where it is, in the global space's logical pixels (ADR 0003): x, y, width, height.
    pub rect: (i32, i32, u32, u32),
}

impl Node {
    /// How a live region announces what changes in it, if it is one.
    fn live(&self) -> Option<Politeness> {
        if self.state.contains(AccessibleState::LIVE_ASSERTIVE) {
            Some(Politeness::Assertive)
        } else if self.state.contains(AccessibleState::LIVE_POLITE) {
            Some(Politeness::Polite)
        } else {
            None
        }
    }

    /// What a live region says: its name, then its value.
    fn said(&self) -> String {
        match (self.name.is_empty(), self.value.is_empty()) {
            (_, true) => self.name.clone(),
            (true, false) => self.value.clone(),
            (false, false) => format!("{} {}", self.name, self.value),
        }
    }
}

/// What the reader would say, and why: an application announced it, or a live region changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    pub politeness: Politeness,
    pub text: String,
    pub live: bool,
}

#[derive(Debug, Default)]
pub struct Reader {
    client: ClientId,
    /// Committed trees, by surface.
    pub trees: BTreeMap<SurfaceKey, Vec<Node>>,
    /// Trees being received, with their custom actions' labels as received.
    receiving: BTreeMap<SurfaceKey, Vec<Option<(Node, String)>>>,
    /// Announcements being received: the text so far and how it interrupts.
    announcing: BTreeMap<SurfaceKey, (String, Politeness)>,
    /// What it would have said since the last `take_announcements`.
    announcements: Vec<Announcement>,
}

impl Reader {
    pub fn new(client: ClientId) -> Self {
        Self {
            client,
            ..Self::default()
        }
    }

    /// What it would have said since the last call, in order.
    pub fn take_announcements(&mut self) -> Vec<Announcement> {
        std::mem::take(&mut self.announcements)
    }

    /// Takes in what the compositor sent the reader since the last call.
    pub fn absorb(&mut self, compositor: &mut Compositor) {
        let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut compositor.local_events)
            .into_iter()
            .partition(|(client, _)| *client == self.client);
        compositor.local_events = others;
        for (_, event) in mine {
            self.receive(event);
        }
    }

    fn receive(&mut self, event: Event) {
        let key = |client, surface| SurfaceKey {
            client,
            id: surface,
        };
        match event {
            Event::AccessibilityTree {
                client,
                surface,
                nodes,
                ..
            } => {
                self.receiving
                    .insert(key(client, surface), vec![None; nodes as usize]);
            }
            Event::AccessibleNode {
                client,
                surface,
                index,
                parent,
                role,
                state,
                actions,
                rect,
            } => {
                if let Some(slot) = self.slot(key(client, surface), index) {
                    *slot = Some((
                        Node {
                            role,
                            state,
                            actions,
                            name: String::new(),
                            value: String::new(),
                            custom: Vec::new(),
                            selection: None,
                            hint: String::new(),
                            position: None,
                            parent: (parent != oxyos_protocol::NO_ACCESSIBLE_PARENT)
                                .then_some(parent),
                            rect: (rect.x, rect.y, rect.width, rect.height),
                        },
                        String::new(),
                    ));
                }
            }
            Event::AccessibleSelection {
                client,
                surface,
                index,
                start,
                end,
            } => {
                if let Some(Some((node, _))) = self.slot(key(client, surface), index) {
                    node.selection = Some((start, end));
                }
            }
            Event::AccessiblePosition {
                client,
                surface,
                index,
                position,
                count,
            } => {
                if let Some(Some((node, _))) = self.slot(key(client, surface), index) {
                    node.position = Some((position, count));
                }
            }
            Event::AccessibleText {
                client,
                surface,
                index,
                field,
                text,
            } => {
                if let Some(Some((node, labels))) = self.slot(key(client, surface), index) {
                    match field {
                        AccessibleField::Name => node.name.push_str(text.as_str()),
                        AccessibleField::Value => node.value.push_str(text.as_str()),
                        AccessibleField::Actions => labels.push_str(text.as_str()),
                        AccessibleField::Hint => node.hint.push_str(text.as_str()),
                    }
                }
            }
            Event::AccessibilityCommitted { client, surface } => {
                let key = key(client, surface);
                if let Some(nodes) = self.receiving.remove(&key) {
                    let nodes: Vec<Node> = nodes
                        .into_iter()
                        .flatten()
                        .map(|(mut node, labels)| {
                            node.custom = labels
                                .lines()
                                .filter(|label| !label.is_empty())
                                .map(str::to_string)
                                .collect();
                            node
                        })
                        .collect();
                    self.live_changes(key, &nodes);
                    self.trees.insert(key, nodes);
                }
            }
            Event::AccessibilityRemoved { client, surface } => {
                let key = key(client, surface);
                self.receiving.remove(&key);
                self.trees.remove(&key);
            }
            Event::AccessibilityAnnouncement {
                client,
                surface,
                index,
                last,
                politeness,
                text,
            } => {
                let key = key(client, surface);
                if index == 0 {
                    self.announcing.insert(key, (String::new(), politeness));
                }
                if let Some((said, _)) = self.announcing.get_mut(&key) {
                    said.push_str(text.as_str());
                }
                if last {
                    if let Some((text, politeness)) = self.announcing.remove(&key) {
                        self.announcements.push(Announcement {
                            politeness,
                            text,
                            live: false,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    fn slot(&mut self, key: SurfaceKey, index: u32) -> Option<&mut Option<(Node, String)>> {
        self.receiving
            .get_mut(&key)
            .and_then(|nodes| nodes.get_mut(index as usize))
    }

    /// A live region whose text changed since the surface's previous tree is announced, as
    /// TalkBack does; a surface's first tree announces nothing. Live regions are matched in
    /// order among the tree's live regions, so nodes coming and going around them move nothing.
    fn live_changes(&mut self, key: SurfaceKey, nodes: &[Node]) {
        let Some(before) = self.trees.get(&key) else {
            return;
        };
        let before: Vec<String> = before
            .iter()
            .filter(|node| node.live().is_some())
            .map(Node::said)
            .collect();
        let live = nodes.iter().filter_map(|node| Some((node.live()?, node)));
        for (index, (politeness, node)) in live.enumerate() {
            let said = node.said();
            let was = before.get(index);
            if !said.is_empty() && was != Some(&said) {
                self.announcements.push(Announcement {
                    politeness,
                    text: said,
                    live: true,
                });
            }
        }
    }
}
