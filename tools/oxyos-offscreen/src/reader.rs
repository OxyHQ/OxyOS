//! The assistive technology the development compositor plays (a screen reader): a client of the
//! scene only, granted `accessibility`, holding the trees the compositor sends it as it receives
//! them (ADR 0003). What a step reports and acts on is what it holds, not the compositor's own
//! state: a tree the compositor keeps from it (behind the lock, ADR 0010) is not there.

use oxyos_compositor::Compositor;
use oxyos_compositor_core::{ClientId, SurfaceKey};
use oxyos_protocol::{AccessibleField, AccessibleRole, AccessibleState, Event};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Node {
    pub role: AccessibleRole,
    pub state: AccessibleState,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Default)]
pub struct Reader {
    client: ClientId,
    /// Committed trees, by surface.
    pub trees: BTreeMap<SurfaceKey, Vec<Node>>,
    /// Trees being received.
    receiving: BTreeMap<SurfaceKey, Vec<Option<Node>>>,
}

impl Reader {
    pub fn new(client: ClientId) -> Self {
        Self {
            client,
            ..Self::default()
        }
    }

    /// Takes in what the compositor sent the reader since the last call.
    pub fn absorb(&mut self, compositor: &mut Compositor) {
        let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut compositor.local_events)
            .into_iter()
            .partition(|(client, _)| *client == self.client);
        compositor.local_events = others;
        for (_, event) in mine {
            match event {
                Event::AccessibilityTree {
                    client,
                    surface,
                    nodes,
                    ..
                } => {
                    self.receiving.insert(
                        SurfaceKey {
                            client,
                            id: surface,
                        },
                        vec![None; nodes as usize],
                    );
                }
                Event::AccessibleNode {
                    client,
                    surface,
                    index,
                    role,
                    state,
                    ..
                } => {
                    let key = SurfaceKey {
                        client,
                        id: surface,
                    };
                    if let Some(slot) = self
                        .receiving
                        .get_mut(&key)
                        .and_then(|nodes| nodes.get_mut(index as usize))
                    {
                        *slot = Some(Node {
                            role,
                            state,
                            name: String::new(),
                            value: String::new(),
                        });
                    }
                }
                Event::AccessibleText {
                    client,
                    surface,
                    index,
                    field,
                    text,
                } => {
                    let key = SurfaceKey {
                        client,
                        id: surface,
                    };
                    if let Some(Some(node)) = self
                        .receiving
                        .get_mut(&key)
                        .and_then(|nodes| nodes.get_mut(index as usize))
                    {
                        match field {
                            AccessibleField::Name => node.name.push_str(text.as_str()),
                            AccessibleField::Value => node.value.push_str(text.as_str()),
                        }
                    }
                }
                Event::AccessibilityCommitted { client, surface } => {
                    let key = SurfaceKey {
                        client,
                        id: surface,
                    };
                    if let Some(nodes) = self.receiving.remove(&key) {
                        self.trees
                            .insert(key, nodes.into_iter().flatten().collect());
                    }
                }
                Event::AccessibilityRemoved { client, surface } => {
                    let key = SurfaceKey {
                        client,
                        id: surface,
                    };
                    self.receiving.remove(&key);
                    self.trees.remove(&key);
                }
                _ => {}
            }
        }
    }
}
