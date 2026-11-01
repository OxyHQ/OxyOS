//! What the screen reader holds and says (docs/adr/0014-screen-reader.mdx): the trees the
//! compositor sends it (ADR 0003), its cursor over their elements, and what it says as they
//! change, as its commands move the cursor, and as applications announce.

use crate::editing;
use oxyos_protocol::{
    AccessibleAction, AccessibleField, AccessibleRole, AccessibleState, Event, Politeness,
    ReaderCommand, Request,
};
use oxyos_speech::{describe_change, describe_item, describe_state_change, Language};
use std::collections::{BTreeMap, BTreeSet};

/// A surface, by its client and its id, as the compositor names it.
pub type Surface = (u32, u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub role: AccessibleRole,
    pub state: AccessibleState,
    pub name: String,
    pub value: String,
    /// A text field's selection in bytes of its value (never a secure field's).
    pub selection: Option<(u32, u32)>,
    /// What activating it does, when its name does not say (version 13).
    pub hint: String,
    /// Its place among its collection's items, 1-based, and their count (version 13).
    pub position: Option<(u32, u32)>,
}

impl Node {
    fn empty(role: AccessibleRole, state: AccessibleState) -> Self {
        Self {
            role,
            state,
            name: String::new(),
            value: String::new(),
            selection: None,
            hint: String::new(),
            position: None,
        }
    }

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
    /// The surfaces that are another's popups (their trees name a parent surface).
    popups: BTreeSet<Surface>,
    /// A popup whose title was said and nothing focused in it yet: its first focus follows the
    /// title rather than stopping it, in whichever of its trees it comes.
    titled: Option<Surface>,
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
            popups: BTreeSet::new(),
            titled: None,
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
                describe_item(
                    self.language,
                    node.role,
                    node.state,
                    &node.name,
                    &node.value,
                    node.position,
                )
            })
    }

    /// The element the cursor reached, said; then its hint, if it has one, after it (VoiceOver's
    /// hint, said after a pause and cut off by moving on), as TalkBack says a node's hint text.
    fn say_reached(&mut self, at: (Surface, usize), politeness: Politeness) {
        let text = self.describe(at);
        if text.is_empty() {
            return;
        }
        self.say(text, politeness);
        let hint = self
            .trees
            .get(&at.0)
            .and_then(|nodes| nodes.get(at.1))
            .map(|node| node.hint.trim().to_string())
            .unwrap_or_default();
        self.say(hint, Politeness::Polite);
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
                parent,
                nodes,
            } => {
                self.receiving
                    .insert((client, surface), vec![None; nodes as usize]);
                if parent != 0 {
                    self.popups.insert((client, surface));
                }
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
                    *slot = Some(Node::empty(role, state));
                }
            }
            Event::AccessibleSelection {
                client,
                surface,
                index,
                start,
                end,
            } => {
                if let Some(Some(node)) = self.slot((client, surface), index) {
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
                if let Some(Some(node)) = self.slot((client, surface), index) {
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
                if let Some(Some(node)) = self.slot((client, surface), index) {
                    match field {
                        AccessibleField::Name => node.name.push_str(text.as_str()),
                        AccessibleField::Value => node.value.push_str(text.as_str()),
                        AccessibleField::Actions => {}
                        AccessibleField::Hint => node.hint.push_str(text.as_str()),
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
                            node.unwrap_or_else(|| {
                                Node::empty(AccessibleRole::Generic, AccessibleState::default())
                            })
                        })
                        .collect();
                    self.commit(key, nodes);
                }
            }
            Event::AccessibilityRemoved { client, surface } => {
                let key = (client, surface);
                self.receiving.remove(&key);
                self.popups.remove(&key);
                if self.titled == Some(key) {
                    self.titled = None;
                }
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
        let first = before.is_none();
        self.live_changes(key, before.as_deref());
        self.text_changes(key, before.as_deref());
        self.state_changes(key, before.as_deref());
        // A popup that appears is said by its title, as TalkBack says the pane title of a window
        // whose state changed (Android's TYPE_WINDOW_STATE_CHANGED for a popup window): the menu
        // or dialog it holds, by its name and role ("Message actions, menu"). What is first focused
        // in it is said after that, not over it, in this tree or a later one.
        if first && self.popups.contains(&key) {
            if let Some(title) = self.trees[&key].iter().position(|node| {
                matches!(node.role, AccessibleRole::Menu | AccessibleRole::Dialog)
                    && !node.name.trim().is_empty()
            }) {
                let text = self.describe((key, title));
                self.say(text, Politeness::Assertive);
                self.titled = Some(key);
            }
        }
        // The element the application focused: the cursor goes there and says it, unless it is
        // there already (the reader's own activation focused it).
        let focused = self.trees[&key]
            .iter()
            .position(|node| node.state.contains(AccessibleState::FOCUSED))
            .map(|index| (key, index));
        match focused {
            Some(at) if self.focused != Some(at) => {
                self.focused = Some(at);
                let focus = if self.titled.take() == Some(key) {
                    Politeness::Polite
                } else {
                    Politeness::Assertive
                };
                if self.cursor != Some(at) {
                    self.cursor = Some(at);
                    self.say_reached(at, focus);
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

    /// The focused text field edited, or its caret or selection moved, since the surface's
    /// previous tree, as TalkBack and VoiceOver echo it: what was typed or deleted, what the caret
    /// passed over, what was selected or unselected, at once (each keystroke's echo cuts off the
    /// last). Only for the same field focused in both trees, with a selection in both: never an
    /// unfocused field, a field newly focused (its focus says it) or a secure one (it has none).
    fn text_changes(&mut self, key: Surface, before: Option<&[Node]>) {
        let Some((surface, index)) = self.focused else {
            return;
        };
        if surface != key {
            return;
        }
        let (Some(old), Some(new)) = (
            before.and_then(|nodes| nodes.get(index)),
            self.trees[&key].get(index),
        ) else {
            return;
        };
        let focused = |node: &Node| node.state.contains(AccessibleState::FOCUSED);
        if !focused(old) || !focused(new) || old.role != new.role {
            return;
        }
        let (Some(was), Some(is)) = (old.selection, new.selection) else {
            return;
        };
        if let Some(change) = editing::change((&old.value, was), (&new.value, is)) {
            let text = describe_change(self.language, &change);
            self.say(text, Politeness::Assertive);
        }
    }

    /// The focused element changed in place since the surface's previous tree, as VoiceOver says
    /// it: a switch turned on or off by the keyboard or a press ("activado"), a box checked, a
    /// disclosure opened, by its new state; a slider stepped by the arrows, by its new value. Only
    /// for the same element focused in both trees (its role and name the same): a newly focused
    /// one is said by its focus, and a text field's edits are its echo.
    fn state_changes(&mut self, key: Surface, before: Option<&[Node]>) {
        let Some((surface, index)) = self.focused else {
            return;
        };
        if surface != key {
            return;
        }
        let (Some(old), Some(new)) = (
            before.and_then(|nodes| nodes.get(index)),
            self.trees[&key].get(index),
        ) else {
            return;
        };
        let focused = |node: &Node| node.state.contains(AccessibleState::FOCUSED);
        if !focused(old) || !focused(new) || old.role != new.role || old.name != new.name {
            return;
        }
        let state = describe_state_change(self.language, new.role, old.state, new.state);
        let value = (old.selection.is_none()
            && new.selection.is_none()
            && old.value != new.value
            && !new.value.trim().is_empty())
        .then(|| new.value.trim().to_string());
        for text in state.into_iter().chain(value) {
            self.say(text, Politeness::Assertive);
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
                self.titled = None;
                self.cursor = Some(at);
                self.say_reached(at, Politeness::Assertive);
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
