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

/// Whether the node at `at` is within the one at `index`: its line of parents, each earlier in the
/// tree, reaches it. A node's descendants follow it, together.
fn within(nodes: &[Node], index: usize, mut at: usize) -> bool {
    while at > index {
        match nodes[at].parent {
            Some(parent) if (parent as usize) < at => at = parent as usize,
            _ => return false,
        }
    }
    at == index
}

/// What the live region at `index` says, as the screen reader (tools/oxyos-reader) says it: its
/// own words, else, for a container with none (a toast's row), the text it holds in order (its
/// texts, headings and images, not its controls' names, nor a live region inside it).
fn live_said(nodes: &[Node], index: usize) -> String {
    let own = nodes[index].said();
    if !own.is_empty() {
        return own;
    }
    let mut parts = Vec::new();
    for (at, node) in nodes.iter().enumerate().skip(index + 1) {
        if !within(nodes, index, at) {
            break;
        }
        let content = matches!(
            node.role,
            AccessibleRole::Text
                | AccessibleRole::Heading
                | AccessibleRole::Image
                | AccessibleRole::Generic
        );
        let text = node.said();
        if content && node.live().is_none() && !text.trim().is_empty() {
            parts.push(text);
        }
    }
    parts.join(", ")
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
    /// Each view's live regions as they last showed, by surface and the view its tree is of (its
    /// first node's role and name: the window, or a modal shown over it): see `live_changes`.
    lives: BTreeMap<(SurfaceKey, AccessibleRole, String), Vec<String>>,
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

    /// Takes in what the compositor sent the reader since the last call: everything the scene
    /// emitted up to now, routed first. The development compositor acts on its scene itself (a
    /// window moved to its place, as `--companion-at` puts the companion), and what the scene
    /// sent the reader for it (the tree again, at the window's new place) waited in the scene's
    /// queue until the next turn routed it: a step resolved its node by name in between found the
    /// tree at the window's old place, and pressed there (settings-caret-blink-scaled's press on
    /// the companion's field landed on Settings, where the companion had first been placed).
    pub fn absorb(&mut self, compositor: &mut Compositor) {
        compositor.flush();
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
                self.lives.retain(|(surface, ..), _| *surface != key);
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

    /// A live region whose text changed since its view last showed it is announced, as TalkBack
    /// does, compared with itself as the same view (the window, or the same modal) last showed it:
    /// one hidden under a modal and shown again unchanged announces nothing. A view's first
    /// showing announces nothing; a live region appearing in a view shown before (a field's
    /// error, a toast) announces what it shows. What the view showed is matched by text, each
    /// once, not by place, as the screen reader does; the view is its first node that is not live
    /// content (a toast's row first in the window is not another view).
    fn live_changes(&mut self, key: SurfaceKey, nodes: &[Node]) {
        let mut content_end = 0;
        let view = nodes
            .iter()
            .enumerate()
            .find(|&(index, node)| {
                if index < content_end {
                    return false;
                }
                if node.live().is_none() {
                    return true;
                }
                content_end = (index + 1..nodes.len())
                    .find(|&at| !within(nodes, index, at))
                    .unwrap_or(nodes.len());
                false
            })
            .map_or((AccessibleRole::Generic, String::new()), |(_, root)| {
                (root.role, root.name.clone())
            });
        let now: Vec<(Politeness, String)> = nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| Some((node.live()?, live_said(nodes, index))))
            .collect();
        let texts = now.iter().map(|(_, said)| said.clone()).collect();
        let Some(mut shown) = self.lives.insert((key, view.0, view.1), texts) else {
            return;
        };
        for (politeness, said) in now {
            match shown.iter().position(|was| *was == said) {
                Some(at) => {
                    shown.swap_remove(at);
                }
                None if !said.is_empty() => self.announcements.push(Announcement {
                    politeness,
                    text: said,
                    live: true,
                }),
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_compositor_core::{Grants, Output, Scene};
    use oxyos_protocol::{
        AccessibleRole, AccessibleState, Clock, Format, Rect, Request, Size, NO_ACCESSIBLE_PARENT,
    };

    const APP: ClientId = 1;
    const READER: ClientId = ClientId::MAX;

    /// A window of `APP`'s, shown, with one node at (10, 20) in it; the reader local.
    fn session() -> (Compositor, SurfaceKey) {
        let mut scene = Scene::new();
        scene
            .add_output(Output {
                id: 1,
                rect: Rect::new(0, 0, 1280, 800),
                scale_120: 120,
                refresh_mhz: 60_000,
            })
            .unwrap();
        scene.connect(APP, Grants::default()).unwrap();
        scene
            .connect(
                READER,
                Grants {
                    accessibility: true,
                    ..Grants::default()
                },
            )
            .unwrap();
        let mut compositor = Compositor::new(scene, 1000);
        compositor.local_clients.insert(READER);
        let scene = &mut compositor.scene;
        scene
            .handle(
                APP,
                Request::CreateToplevel {
                    surface: 1,
                    size: Size::new(400, 300),
                },
            )
            .unwrap();
        let (serial, extent) = scene
            .drain_events()
            .into_iter()
            .find_map(|(client, event)| match event {
                oxyos_protocol::Event::Configure { serial, rect, .. } if client == APP => {
                    Some((serial, Size::new(rect.width, rect.height)))
                }
                _ => None,
            })
            .unwrap();
        for request in [
            Request::AttachBuffer {
                buffer: 1,
                format: Format::Xrgb8888,
                extent,
                stride: extent.width * 4,
                offset: 0,
            },
            Request::Submit {
                surface: 1,
                sequence: 1,
                buffer: 1,
                configure: serial,
                extent,
            },
        ] {
            scene.handle(APP, request).unwrap();
        }
        scene.begin_frame();
        scene.end_frame(1, Clock::Synthetic);
        for request in [
            Request::AccessibilityBegin {
                surface: 1,
                nodes: 1,
            },
            Request::AccessibleNode {
                surface: 1,
                index: 0,
                parent: NO_ACCESSIBLE_PARENT,
                role: AccessibleRole::Button,
                state: AccessibleState::new(0).unwrap(),
                actions: Default::default(),
                rect: Rect::new(10, 20, 100, 30),
            },
            Request::AccessibilityCommit { surface: 1 },
        ] {
            scene.handle(APP, request).unwrap();
        }
        (compositor, SurfaceKey { client: APP, id: 1 })
    }

    #[test]
    fn a_window_the_session_moves_is_held_at_its_new_place_at_once() {
        let (mut compositor, window) = session();
        let mut reader = Reader::new(READER);
        reader.absorb(&mut compositor);
        let at = compositor.scene.global_rect(window).unwrap();
        let node = |reader: &Reader| reader.trees[&window][0].rect;
        assert_eq!(node(&reader), (at.x + 10, at.y + 20, 100, 30));
        // Moved by the session itself, with no turn of the compositor in between: what the reader
        // holds is the window where it is now.
        compositor
            .scene
            .move_window_to(window, (at.x + 300, at.y + 40));
        reader.absorb(&mut compositor);
        assert_eq!(node(&reader), (at.x + 310, at.y + 60, 100, 30));
    }
}
