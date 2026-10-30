//! What the screen reader holds and says (docs/adr/0014-screen-reader.mdx): the trees the
//! compositor sends it (ADR 0003), its cursor over their elements, and what it says as they
//! change, as its commands move the cursor, and as applications announce.

use oxyos_protocol::{
    AccessibleAction, AccessibleField, AccessibleRole, AccessibleState, Event, Politeness,
    ReaderCommand, Request,
};
use oxyos_speech::{describe, Language};
use std::collections::BTreeMap;

/// A surface, by its client and its id, as the compositor names it.
pub type Surface = (u32, u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub role: AccessibleRole,
    pub state: AccessibleState,
    pub name: String,
    pub value: String,
}

impl Node {
    fn live(&self) -> Option<Politeness> {
        if self.state.contains(AccessibleState::LIVE_ASSERTIVE) {
            Some(Politeness::Assertive)
        } else if self.state.contains(AccessibleState::LIVE_POLITE) {
            Some(Politeness::Polite)
        } else {
            None
        }
    }

    /// What a live region says when it changes: its name, then its value.
    fn live_text(&self) -> String {
        [self.name.as_str(), self.value.as_str()]
            .iter()
            .filter(|text| !text.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Something to say, and whether it waits for what is being said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Utterance {
    pub text: String,
    pub politeness: Politeness,
}

#[derive(Debug)]
pub struct Reader {
    language: Language,
    /// Committed trees, by surface, in the order the compositor sent them first.
    trees: BTreeMap<Surface, Vec<Node>>,
    /// The order surfaces' trees arrived in: the cursor moves on through them.
    order: Vec<Surface>,
    receiving: BTreeMap<Surface, Vec<Option<Node>>>,
    announcing: BTreeMap<Surface, (String, Politeness)>,
    /// The element under the cursor.
    cursor: Option<(Surface, usize)>,
    /// The element last focused, as the trees said: a new one moves the cursor.
    focused: Option<(Surface, usize)>,
    said: Vec<Utterance>,
}

impl Reader {
    pub fn new(language: Language) -> Self {
        Self {
            language,
            trees: BTreeMap::new(),
            order: Vec::new(),
            receiving: BTreeMap::new(),
            announcing: BTreeMap::new(),
            cursor: None,
            focused: None,
            said: Vec::new(),
        }
    }

    /// What it has to say since the last call, in order.
    pub fn take_said(&mut self) -> Vec<Utterance> {
        std::mem::take(&mut self.said)
    }

    #[cfg(test)]
    pub fn cursor(&self) -> Option<(Surface, usize)> {
        self.cursor
    }

    fn say(&mut self, text: String, politeness: Politeness) {
        if !text.is_empty() {
            self.said.push(Utterance { text, politeness });
        }
    }

    fn describe(&self, (surface, index): (Surface, usize)) -> String {
        self.trees
            .get(&surface)
            .and_then(|nodes| nodes.get(index))
            .map_or_else(String::new, |node| {
                describe(
                    self.language,
                    node.role,
                    node.state,
                    &node.name,
                    &node.value,
                )
            })
    }

    /// Whether the cursor stops at an element: it has something to say, and it is not a
    /// container with no name of its own (a menu's items are what is read, not the menu).
    fn stops(&self, (surface, index): (Surface, usize)) -> bool {
        let Some(node) = self.trees.get(&surface).and_then(|nodes| nodes.get(index)) else {
            return false;
        };
        let container = matches!(
            node.role,
            AccessibleRole::Generic
                | AccessibleRole::Group
                | AccessibleRole::Menu
                | AccessibleRole::List
                | AccessibleRole::TabList
                | AccessibleRole::Toolbar
                | AccessibleRole::ScrollView
                | AccessibleRole::Grid
                | AccessibleRole::Dialog
        );
        let unnamed = node.name.trim().is_empty() && node.value.trim().is_empty();
        let says = !self.describe((surface, index)).is_empty();
        says && !(unnamed && container)
    }

    /// Takes in an event from the compositor.
    pub fn receive(&mut self, event: &Event) {
        match *event {
            Event::AccessibilityTree {
                client,
                surface,
                nodes,
                ..
            } => {
                self.receiving
                    .insert((client, surface), vec![None; nodes as usize]);
            }
            Event::AccessibleNode {
                client,
                surface,
                index,
                role,
                state,
                ..
            } => {
                if let Some(slot) = self.slot((client, surface), index) {
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
                if let Some(Some(node)) = self.slot((client, surface), index) {
                    match field {
                        AccessibleField::Name => node.name.push_str(text.as_str()),
                        AccessibleField::Value => node.value.push_str(text.as_str()),
                        AccessibleField::Actions => {}
                    }
                }
            }
            Event::AccessibilityCommitted { client, surface } => {
                let key = (client, surface);
                if let Some(nodes) = self.receiving.remove(&key) {
                    // A node never sent is kept as an empty one, so indices stay the
                    // application's.
                    let nodes: Vec<Node> = nodes
                        .into_iter()
                        .map(|node| {
                            node.unwrap_or(Node {
                                role: AccessibleRole::Generic,
                                state: AccessibleState::default(),
                                name: String::new(),
                                value: String::new(),
                            })
                        })
                        .collect();
                    self.commit(key, nodes);
                }
            }
            Event::AccessibilityRemoved { client, surface } => {
                let key = (client, surface);
                self.receiving.remove(&key);
                self.announcing.remove(&key);
                self.trees.remove(&key);
                self.order.retain(|surface| *surface != key);
                if self.cursor.is_some_and(|(surface, _)| surface == key) {
                    self.cursor = None;
                }
                if self.focused.is_some_and(|(surface, _)| surface == key) {
                    self.focused = None;
                }
            }
            Event::AccessibilityAnnouncement {
                client,
                surface,
                index,
                last,
                politeness,
                text,
            } => {
                let key = (client, surface);
                if index == 0 {
                    self.announcing.insert(key, (String::new(), politeness));
                }
                if let Some((said, _)) = self.announcing.get_mut(&key) {
                    said.push_str(text.as_str());
                }
                if last {
                    if let Some((text, politeness)) = self.announcing.remove(&key) {
                        self.say(text, politeness);
                    }
                }
            }
            _ => {}
        }
    }

    fn slot(&mut self, key: Surface, index: u32) -> Option<&mut Option<Node>> {
        self.receiving
            .get_mut(&key)
            .and_then(|nodes| nodes.get_mut(index as usize))
    }

    fn commit(&mut self, key: Surface, nodes: Vec<Node>) {
        let before = self.trees.insert(key, nodes);
        if !self.order.contains(&key) {
            self.order.push(key);
        }
        if let Some((surface, index)) = self.cursor {
            let count = self.trees[&key].len();
            if surface == key && index >= count {
                self.cursor = count.checked_sub(1).map(|last| (key, last));
            }
        }
        self.live_changes(key, before.as_deref());
        // The element the application focused: the cursor goes there and says it, unless it is
        // there already (the reader's own activation focused it).
        let focused = self.trees[&key]
            .iter()
            .position(|node| node.state.contains(AccessibleState::FOCUSED))
            .map(|index| (key, index));
        match focused {
            Some(at) if self.focused != Some(at) => {
                self.focused = Some(at);
                if self.cursor != Some(at) {
                    self.cursor = Some(at);
                    let text = self.describe(at);
                    self.say(text, Politeness::Assertive);
                }
            }
            None if self.focused.is_some_and(|(surface, _)| surface == key) => {
                self.focused = None;
            }
            _ => {}
        }
    }

    /// A live region whose text changed since the surface's previous tree is said, as TalkBack
    /// says Android's; a surface's first tree says nothing. Live regions are matched in order.
    fn live_changes(&mut self, key: Surface, before: Option<&[Node]>) {
        let Some(before) = before else {
            return;
        };
        let before: Vec<String> = before
            .iter()
            .filter(|node| node.live().is_some())
            .map(Node::live_text)
            .collect();
        let changed: Vec<(String, Politeness)> = self.trees[&key]
            .iter()
            .filter_map(|node| Some((node.live()?, node.live_text())))
            .enumerate()
            .filter(|(index, (_, text))| before.get(*index) != Some(text))
            .map(|(_, (politeness, text))| (text, politeness))
            .collect();
        for (text, politeness) in changed {
            self.say(text, politeness);
        }
    }

    /// The elements the cursor stops at, through every surface in order.
    fn stops_in_order(&self) -> Vec<(Surface, usize)> {
        self.order
            .iter()
            .flat_map(|surface| (0..self.trees[surface].len()).map(move |index| (*surface, index)))
            .filter(|at| self.stops(*at))
            .collect()
    }

    /// A command of the reader's keys: moving says the element moved to; activating asks its
    /// application to activate it (what that does is the application's to say).
    pub fn command(&mut self, command: ReaderCommand) -> Option<Request> {
        match command {
            ReaderCommand::Next | ReaderCommand::Previous => {
                let stops = self.stops_in_order();
                let current = self
                    .cursor
                    .and_then(|cursor| stops.iter().position(|at| *at == cursor));
                // Without a cursor, Next starts at the focused surface's first element, or the
                // first; Previous at the last.
                let next = match (command, current) {
                    (ReaderCommand::Next, Some(index)) => stops.get(index + 1),
                    (_, Some(index)) => index.checked_sub(1).and_then(|index| stops.get(index)),
                    (ReaderCommand::Next, None) => self
                        .focused
                        .and_then(|(surface, _)| stops.iter().find(|(s, _)| *s == surface))
                        .or(stops.first()),
                    (_, None) => stops.last(),
                };
                // At either end the cursor stays, saying nothing.
                let at = *next?;
                self.cursor = Some(at);
                let text = self.describe(at);
                self.say(text, Politeness::Assertive);
                None
            }
            ReaderCommand::Activate => {
                let ((client, surface), index) = self.cursor?;
                Some(Request::AccessibilityAction {
                    client,
                    surface,
                    index: index as u32,
                    action: AccessibleAction::Activate,
                    custom: 0,
                    start: 0,
                    end: 0,
                })
            }
        }
    }
}
