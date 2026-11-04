//! What the screen reader holds and says (docs/adr/0014-screen-reader.mdx): the trees the
//! compositor sends it (ADR 0003), its cursor over their elements, and what it says as they
//! change, as its commands move the cursor, and as applications announce.

use crate::editing;
use oxyos_protocol::{
    AccessibleAction, AccessibleField, AccessibleRole, AccessibleState, Event, Politeness,
    ReaderCommand, Request, NO_ACCESSIBLE_PARENT,
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
    /// The node that holds it, earlier in the tree; `NO_ACCESSIBLE_PARENT` for a root.
    pub parent: u32,
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
            parent: NO_ACCESSIBLE_PARENT,
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

    /// Its own words: its name, then its value.
    fn own_text(&self) -> String {
        [self.name.as_str(), self.value.as_str()]
            .iter()
            .filter(|text| !text.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Whether the node at `at` is within the one at `index`: its line of parents, each earlier in the
/// tree, reaches it. A node's descendants follow it, together.
fn within(nodes: &[Node], index: usize, mut at: usize) -> bool {
    while at > index {
        match nodes[at].parent {
            NO_ACCESSIBLE_PARENT => return false,
            parent => at = parent as usize,
        }
    }
    at == index
}

/// What the live region at `index` says when it changes: its own words, else, for a container
/// with none (a toast's row, Android's live ViewGroup), the text it holds, in order, as TalkBack
/// says a live region's content and an ARIA live region's text is read: its texts, headings and
/// images, not its controls' names (a toast's close button), nor a live region inside it, which
/// says itself.
fn live_text(nodes: &[Node], index: usize) -> String {
    let own = nodes[index].own_text();
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
        let text = node.own_text();
        if content && node.live().is_none() && !text.trim().is_empty() {
            parts.push(text);
        }
    }
    parts.join(", ")
}

/// Something to say, whether it waits for what is being said, and the language it is in (the
/// voice it is said with).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Utterance {
    pub text: String,
    pub politeness: Politeness,
    pub language: Language,
}

#[derive(Debug)]
pub struct Reader {
    /// The language it speaks: its environment's until the session's locale says (ADR 0040).
    language: Language,
    /// The session's locale as it arrives in chunks, and whether one has arrived whole.
    locale: String,
    told_locale: bool,
    /// Committed trees, by surface, in the order the compositor sent them first.
    trees: BTreeMap<Surface, Vec<Node>>,
    /// The order surfaces' trees arrived in: the cursor moves on through them.
    order: Vec<Surface>,
    receiving: BTreeMap<Surface, Vec<Option<Node>>>,
    /// The surfaces that are another's popups (their trees name a parent surface).
    popups: BTreeSet<Surface>,
    /// Each view's live regions as they last showed, by surface and the view the surface's tree
    /// is of (its first node's role and name: the window, or a modal shown over it, whose tree is
    /// the modal's alone): a live region compared with itself as it was, so one hidden under a
    /// modal and shown again unchanged says nothing.
    lives: BTreeMap<(Surface, AccessibleRole, String), Vec<String>>,
    /// Whether the session is locked, as the compositor last said (None before it said).
    locked: Option<bool>,
    /// The session's locking was just said: what is focused next (the lock screen's field) is
    /// said after it, not over it.
    after_lock: bool,
    /// A popup whose title was said and nothing focused in it yet: its first focus follows the
    /// title rather than stopping it, in whichever of its trees it comes.
    titled: Option<Surface>,
    /// Each surface that appeared as a named dialog (Launchpad, the Notification Center, a system
    /// sheet), by its title and description: its own, said as it appeared, never again as a modal
    /// over it goes.
    own_dialogs: BTreeMap<Surface, (String, String)>,
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
            locale: String::new(),
            told_locale: false,
            trees: BTreeMap::new(),
            order: Vec::new(),
            receiving: BTreeMap::new(),
            popups: BTreeSet::new(),
            lives: BTreeMap::new(),
            locked: None,
            after_lock: false,
            titled: None,
            own_dialogs: BTreeMap::new(),
            announcing: BTreeMap::new(),
            cursor: None,
            focused: None,
            said: Vec::new(),
        }
    }

    /// The language it speaks now.
    pub fn language(&self) -> Language {
        self.language
    }

    /// Whether the session has told its locale: what it speaks from then on.
    pub fn told_locale(&self) -> bool {
        self.told_locale
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
            self.said.push(Utterance {
                text,
                politeness,
                language: self.language,
            });
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
                parent,
                role,
                state,
                ..
            } => {
                if let Some(slot) = self.slot((client, surface), index) {
                    // A parent is earlier in the tree, else the node is a root.
                    let parent = if parent < index {
                        parent
                    } else {
                        NO_ACCESSIBLE_PARENT
                    };
                    *slot = Some(Node {
                        parent,
                        ..Node::empty(role, state)
                    });
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
            // The session locking is said as VoiceOver says "Screen locked": when it changes, never
            // as the reader connects to a session already locked (the login screen).
            Event::LockState { locked } => {
                if self.locked == Some(false) && locked {
                    let text = match self.language {
                        Language::Spanish => "Pantalla bloqueada",
                        Language::English => "Screen locked",
                    };
                    self.say(text.to_string(), Politeness::Assertive);
                    self.after_lock = true;
                }
                if !locked {
                    self.after_lock = false;
                }
                self.locked = Some(locked);
            }
            Event::AccessibilityRemoved { client, surface } => {
                let key = (client, surface);
                self.receiving.remove(&key);
                self.popups.remove(&key);
                if self.titled == Some(key) {
                    self.titled = None;
                }
                self.own_dialogs.remove(&key);
                self.announcing.remove(&key);
                self.trees.remove(&key);
                self.lives.retain(|(surface, ..), _| *surface != key);
                self.order.retain(|surface| *surface != key);
                if self.cursor.is_some_and(|(surface, _)| surface == key) {
                    self.cursor = None;
                }
                if self.focused.is_some_and(|(surface, _)| surface == key) {
                    self.focused = None;
                }
            }
            // The session's locale (ADR 0040): what is said from the next utterance is its language.
            Event::Locale { index, last, text } => {
                if index == 0 {
                    self.locale.clear();
                }
                self.locale.push_str(text.as_str());
                if last {
                    self.language = Language::of_locale(&self.locale);
                    self.told_locale = true;
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
        // The cursor follows the element it was on, as the focus does (below): one going or coming
        // before it moves it on, and its old place may hold the next element now, which a focus
        // there would otherwise take for the one the cursor was left on and never say (Settings'
        // Resolution choices, committed again a frame later with one node fewer before the rows;
        // its Keep Changes sheet, whose last button the cursor is on). Only an element gone
        // leaves the cursor where it was, on the last node if the tree is shorter now.
        if let Some(moved) = self.followed_cursor(key, before.as_deref()) {
            self.cursor = Some((key, moved));
        } else if let Some((surface, index)) = self.cursor {
            let count = self.trees[&key].len();
            if surface == key && index >= count {
                self.cursor = count.checked_sub(1).map(|last| (key, last));
            }
        }
        let first = before.is_none();
        // Where the element focused before is in this tree, if it is still the one focused: the
        // protocol's nodes have no identity but their place, and an element appearing before it
        // (a search field's clear button as the first letter is typed) moves it on.
        let kept = self.kept_focus(key, before.as_deref());
        if let Some(kept) = kept {
            self.text_changes(key, before.as_deref(), kept);
            self.state_changes(key, before.as_deref(), kept);
            // Followed where it went: the same element, not a new focus to say.
            if self.cursor == self.focused {
                self.cursor = Some((key, kept.1));
            }
            self.focused = Some((key, kept.1));
        }
        // After what the focused element's change says (the letter typed), which an alert it
        // brought (the name typed is taken) interrupts; an assertive one before a new focus, which
        // says itself, and a polite one after it, as it waits for what is being said (a search's
        // count after the field focused again).
        let polite = self.live_changes(key);
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
        // A dialog that appears is said by its title, then its description (its hint, as a sheet's
        // message is VoiceOver's after an alert's title), as a popup's is and as TalkBack says a
        // dialog's window title; what is focused in it is said after them. It appears:
        // - as a surface that is a dialog, its first tree's root (a shell's overlay: Launchpad, the
        //   Notification Center, a system sheet), as VoiceOver says a window coming to the front.
        //   That dialog is the surface's own: shown again as a modal over it goes (an open
        //   Launchpad folder) it is not said again, and another in its place (the next sheet,
        //   with what went wrong) is said as it comes;
        // - in a surface that was showing something else (a modal over a window, whose tree is
        //   then the modal's alone, ADR 0039 decision 2): Dropsea's Quick Look, which has no
        //   control to focus.
        if !self.popups.contains(&key) {
            let nodes = &self.trees[&key];
            let named =
                |node: &Node| node.role == AccessibleRole::Dialog && !node.name.trim().is_empty();
            let title = if first {
                nodes.first().filter(|node| named(node)).map(|_| 0)
            } else {
                nodes.iter().position(named)
            };
            if let Some(title) = title {
                let dialog = &nodes[title];
                let id = (dialog.name.clone(), dialog.hint.clone());
                // A surface's own dialog is told from the next by its message too (the sheet that
                // says what went wrong); a modal over a window by its title alone, so its message
                // changing in place (a countdown: "…in 14 seconds") is no new dialog to say.
                let own = title == 0 && (first || self.own_dialogs.contains_key(&key));
                let was_shown = before.as_deref().is_some_and(|nodes| {
                    nodes.iter().any(|node| {
                        node.role == AccessibleRole::Dialog
                            && node.name == id.0
                            && (!own || node.hint == id.1)
                    })
                }) || self.own_dialogs.get(&key) == Some(&id);
                if own {
                    self.own_dialogs.insert(key, id);
                }
                if !was_shown {
                    self.say_reached((key, title), Politeness::Assertive);
                    self.titled = Some(key);
                }
            }
        }
        // A dialog said by its title that went with nothing focused in it: what is focused next
        // is said at once, not after a title no longer shown.
        if self.titled == Some(key)
            && !self.popups.contains(&key)
            && !self.trees[&key]
                .iter()
                .any(|node| node.role == AccessibleRole::Dialog && !node.name.trim().is_empty())
        {
            self.titled = None;
        }
        // The element the application focused: the cursor goes there and says it, unless it is
        // there already (the reader's own activation focused it).
        let focused = self.trees[&key]
            .iter()
            .position(|node| node.state.contains(AccessibleState::FOCUSED))
            .map(|index| (key, index));
        match focused {
            Some(at) if kept.is_none() => {
                self.focused = Some(at);
                let titled = self.titled.take() == Some(key);
                let after_lock = std::mem::take(&mut self.after_lock);
                let focus = if titled || after_lock {
                    Politeness::Polite
                } else {
                    Politeness::Assertive
                };
                // The cursor on it already (its place holding the same element as before): the
                // reader's own activation focused it, and it was said as the cursor reached it.
                let new = &self.trees[&key][at.1];
                let under_cursor = self.cursor == Some(at)
                    && before
                        .as_deref()
                        .and_then(|nodes| nodes.get(at.1))
                        .is_some_and(|old| old.role == new.role && old.name == new.name);
                if !under_cursor {
                    self.cursor = Some(at);
                    self.say_reached(at, focus);
                }
            }
            None if self.focused.is_some_and(|(surface, _)| surface == key) => {
                self.focused = None;
            }
            _ => {}
        }
        for text in polite {
            self.say(text, Politeness::Polite);
        }
    }

    /// The element focused in the surface's previous tree, still focused in this one: its index
    /// there and here. At the same place it is the same element if its role is (its name may
    /// change in place: a network joining); at another, if its role and name are and what is at
    /// its old place is no longer an element like it (it moved, others coming or going before
    /// it). An element of another role focused at the same place is a new focus.
    fn kept_focus(&self, key: Surface, before: Option<&[Node]>) -> Option<(usize, usize)> {
        let (surface, was) = self.focused?;
        if surface != key {
            return None;
        }
        let old = before?.get(was)?;
        let nodes = &self.trees[&key];
        let is = nodes
            .iter()
            .position(|node| node.state.contains(AccessibleState::FOCUSED))?;
        let like = |node: &Node| node.role == old.role && node.name == old.name;
        let kept = if is == was {
            nodes[is].role == old.role
        } else {
            like(&nodes[is]) && !nodes.get(was).is_some_and(like)
        };
        kept.then_some((was, is))
    }

    /// Where the element under the cursor in the surface's previous tree is in this one, when it
    /// is no longer at its place: the nearest node of its role and name. None when it is still
    /// there, or gone (the cursor then stays where it was).
    fn followed_cursor(&self, key: Surface, before: Option<&[Node]>) -> Option<usize> {
        let (surface, at) = self.cursor?;
        if surface != key {
            return None;
        }
        let old = before?.get(at)?;
        let nodes = &self.trees[&key];
        let like = |node: &Node| node.role == old.role && node.name == old.name;
        if nodes.get(at).is_some_and(like) {
            return None;
        }
        (0..nodes.len())
            .filter(|&index| like(&nodes[index]))
            .min_by_key(|&index| index.abs_diff(at))
    }

    /// A live region whose text changed since its view last showed it is said, as TalkBack says
    /// Android's: compared with itself as the same view (the window, or the same modal) last
    /// showed it, so a live region hidden under a modal and shown again unchanged says nothing,
    /// as VoiceOver says a live region's changes, not its coming back. A surface's first tree says
    /// nothing; nor does a view's first showing. A live region appearing (a field's error, a
    /// toast) is a change: it says what it shows, once. What the view showed is matched by text,
    /// each once, not by place, so a region appearing before others (a toast above the last one)
    /// does not make them say themselves again.
    /// The assertive ones are said at once; the polite ones are returned, to be said after the rest
    /// of what the tree says.
    fn live_changes(&mut self, key: Surface) -> Vec<String> {
        let nodes = &self.trees[&key];
        // The view, by its first node that is not live content: a toast's row coming first in
        // the window is not another view.
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
        let now: Vec<(String, Politeness)> = nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| Some((live_text(nodes, index), node.live()?)))
            .collect();
        let texts: Vec<String> = now.iter().map(|(text, _)| text.clone()).collect();
        let shown = self.lives.insert((key, view.0, view.1), texts);
        let Some(mut shown) = shown else {
            return Vec::new();
        };
        let mut polite = Vec::new();
        for (text, politeness) in now {
            match shown.iter().position(|was| *was == text) {
                Some(at) => {
                    shown.swap_remove(at);
                }
                None if politeness == Politeness::Polite => polite.push(text),
                None => self.say(text, politeness),
            }
        }
        polite
    }

    /// The focused text field edited, or its caret or selection moved, since the surface's
    /// previous tree, as TalkBack and VoiceOver echo it: what was typed or deleted, what the caret
    /// passed over, what was selected or unselected, at once (each keystroke's echo cuts off the
    /// last). Only for the same field focused in both trees, with a selection in both: never an
    /// unfocused field, a field newly focused (its focus says it) or a secure one (it has none).
    fn text_changes(&mut self, key: Surface, before: Option<&[Node]>, (was, is): (usize, usize)) {
        let (Some(old), Some(new)) = (
            before.and_then(|nodes| nodes.get(was)),
            self.trees[&key].get(is),
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
    fn state_changes(&mut self, key: Surface, before: Option<&[Node]>, (was, is): (usize, usize)) {
        let (Some(old), Some(new)) = (
            before.and_then(|nodes| nodes.get(was)),
            self.trees[&key].get(is),
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
