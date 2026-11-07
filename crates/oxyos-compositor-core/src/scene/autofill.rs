//! AutoFill (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 6): a field that says what
//! it holds (a username, an email, a password, a new password, a verification code), focused in
//! the key window of an application the session started, is offered the platform's suggestions in
//! a list of its own at the field's caret; the person's pick is filled into the field, and into
//! the paired field of the account's pair, as one transaction (ADR 0008).
//!
//! The scene decides when and for which field, and owns the list's keys; it knows no keychain.
//! The program running the compositor asks its provider for each field the scene names
//! (`take_autofill_queries`), gives the answer back (`answer_autofill`), words the list the scene
//! tells a shell to draw (`take_autofill_tells`; the shell's candidates surface, placed at the
//! caret as an input method's candidates are, ADR 0041), and fills the item the person picked
//! (`take_autofill_picks`, `fill_autofill`). The application never sees the list: it is told only
//! the text of the item chosen, as if typed. Nothing is offered while locked, in a window that is
//! not the key window, or to a process the session did not start; nothing is filled without the
//! person's pick.

use super::{ClientId, Scene, SurfaceKey};
use oxyos_protocol::{Event, TextChunk, TextContentType, MAX_SURROUNDING_BYTES};
use std::collections::BTreeSet;

const KEY_ESC: u32 = 1;
const KEY_ENTER: u32 = 28;
const KEY_KPENTER: u32 = 96;
const KEY_UP: u32 = 103;
const KEY_DOWN: u32 = 108;

/// Items one list may hold.
pub const MAX_AUTOFILL_ITEMS: usize = 32;
/// Bytes of an item's title or detail, and of a fill's text.
pub const MAX_AUTOFILL_TEXT_BYTES: usize = 512;

/// What an item fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutofillKind {
    /// A saved login: its username and its password.
    Login,
    /// A strong password made for a new-password field, shown as it will be filled.
    StrongPassword,
    /// A login's current verification code.
    Code,
}

/// One suggestion as the list shows it: a login's title and username, a strong password and the
/// password itself, a code's login. Never a saved password. `Debug` shows neither text.
#[derive(Clone, PartialEq, Eq)]
pub struct AutofillItem {
    pub kind: AutofillKind,
    pub title: String,
    pub detail: String,
}

impl std::fmt::Debug for AutofillItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutofillItem")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// A field offered AutoFill: the program asks its provider for `application`'s suggestions for
/// `content`, and answers with `token`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutofillQuery {
    pub token: u64,
    pub client: ClientId,
    /// The application the session started the field's process as (its launch record).
    pub application: String,
    pub content: TextContentType,
}

/// The person picked `index` of the items answered for `token`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutofillPick {
    pub token: u64,
    pub index: usize,
}

/// What a shell is to draw: the items shown, the highlighted one among them. None hides it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutofillList {
    pub items: Vec<AutofillItem>,
    pub highlighted: usize,
    pub content: TextContentType,
}

#[derive(Debug)]
struct Offered {
    token: u64,
    surface: SurfaceKey,
    field: u32,
    content: TextContentType,
    /// As answered, in the provider's order.
    items: Vec<AutofillItem>,
    /// Those shown now (indices into `items`) and the highlighted one among them.
    shown: Vec<usize>,
    highlighted: usize,
    /// Escape, or a pick: not shown again for this field.
    closed: bool,
    /// The item picked, until its fill arrives.
    picked: Option<usize>,
}

#[derive(Debug, Default)]
pub(super) struct Autofill {
    next_token: u64,
    offered: Option<Offered>,
    queries: Vec<AutofillQuery>,
    picks: Vec<AutofillPick>,
    /// The shell told to draw the list, and what it was told.
    pub(super) told: Option<(ClientId, AutofillList)>,
    tells: Vec<(ClientId, Option<AutofillList>)>,
    /// Keys whose press the list took: their release is the list's too, and they do not repeat.
    pub(super) held: BTreeSet<u32>,
}

/// Whether a field's text is a prefix of an item, as typed: a username's or an email's list keeps
/// the logins it may still be.
fn matches(item: &AutofillItem, typed: &str) -> bool {
    item.kind != AutofillKind::Login
        || item
            .detail
            .to_lowercase()
            .starts_with(&typed.to_lowercase())
}

impl Scene {
    /// The field AutoFill may serve now: the focused surface's, with a content type, in the key
    /// window of an application the session started, while unlocked.
    fn autofill_target(&self) -> Option<(SurfaceKey, u32, TextContentType, String)> {
        if self.locked {
            return None;
        }
        let focus = self.focus.filter(|f| self.visible(*f))?;
        if self.active != Some(self.root_of(focus)) {
            return None;
        }
        let application = self.application(focus.client)?;
        let state = self.text_input(focus)?;
        (state.content != TextContentType::None)
            .then(|| (focus, state.field, state.content, application.to_string()))
    }

    /// Brings AutoFill up to date with the focused field: another field is offered anew (its
    /// suggestions asked for), the same keeps its list (narrowed by what is typed), none ends it.
    pub(super) fn refresh_autofill(&mut self) {
        let target = self.autofill_target();
        let same = match (&self.autofill.offered, &target) {
            (Some(offered), Some((surface, field, content, _))) => {
                offered.surface == *surface
                    && offered.field == *field
                    && offered.content == *content
            }
            _ => false,
        };
        if !same {
            self.autofill.offered = None;
            if let Some((surface, field, content, application)) = target {
                self.autofill.next_token += 1;
                let token = self.autofill.next_token;
                self.autofill.offered = Some(Offered {
                    token,
                    surface,
                    field,
                    content,
                    items: Vec::new(),
                    shown: Vec::new(),
                    highlighted: 0,
                    closed: false,
                    picked: None,
                });
                // Only the latest field's answer is wanted: an older query not taken yet goes.
                self.autofill.queries.clear();
                self.autofill.queries.push(AutofillQuery {
                    token,
                    client: surface.client,
                    application,
                    content,
                });
            }
        }
        self.narrow_autofill();
        self.tell_autofill();
    }

    /// What is shown of the answer: every item, a login only while the field's text begins it.
    fn narrow_autofill(&mut self) {
        let Some(offered) = self.autofill.offered.as_ref() else {
            return;
        };
        let typed = match offered.content {
            TextContentType::Username | TextContentType::Email => self
                .text_input(offered.surface)
                .map(|state| state.surrounding.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        let offered = self.autofill.offered.as_mut().expect("present");
        let highlighted = offered.shown.get(offered.highlighted).copied();
        offered.shown = (0..offered.items.len())
            .filter(|&index| matches(&offered.items[index], &typed))
            .collect();
        offered.highlighted = highlighted
            .and_then(|item| offered.shown.iter().position(|&shown| shown == item))
            .unwrap_or(0);
    }

    /// The list to draw now, and the shell of the caret's output that draws it.
    fn autofill_list(&self) -> Option<(ClientId, AutofillList)> {
        let offered = self.autofill.offered.as_ref()?;
        if offered.closed || offered.shown.is_empty() {
            return None;
        }
        let shell = self.caret_shell(offered.surface)?;
        Some((
            shell,
            AutofillList {
                items: offered
                    .shown
                    .iter()
                    .map(|&index| offered.items[index].clone())
                    .collect(),
                highlighted: offered.highlighted,
                content: offered.content,
            },
        ))
    }

    /// Tells the shells what changed of the list: the one drawing it now, what it draws; one that
    /// drew it before and does not now, nothing.
    pub(super) fn tell_autofill(&mut self) {
        let now = self.autofill_list();
        if now != self.autofill.told {
            if let Some((before, _)) = &self.autofill.told {
                if now.as_ref().is_none_or(|(shell, _)| shell != before) {
                    self.autofill.tells.push((*before, None));
                }
            }
            if let Some((shell, list)) = &now {
                self.autofill.tells.push((*shell, Some(list.clone())));
            }
            self.autofill.told = now;
        }
        self.place_candidates();
    }

    /// The suggestions the provider answered for `token` (none: nothing to offer), at most
    /// `MAX_AUTOFILL_ITEMS`, each text bounded. An answer for a field no longer offered is
    /// dropped.
    pub fn answer_autofill(&mut self, token: u64, items: Vec<AutofillItem>) {
        let Some(offered) = self
            .autofill
            .offered
            .as_mut()
            .filter(|offered| offered.token == token)
        else {
            return;
        };
        offered.items = items
            .into_iter()
            .filter(|item| {
                item.title.len() <= MAX_AUTOFILL_TEXT_BYTES
                    && item.detail.len() <= MAX_AUTOFILL_TEXT_BYTES
            })
            .take(MAX_AUTOFILL_ITEMS)
            .collect();
        offered.shown.clear();
        offered.highlighted = 0;
        self.narrow_autofill();
        self.tell_autofill();
    }

    /// The fields offered since the last call, in order.
    pub fn take_autofill_queries(&mut self) -> Vec<AutofillQuery> {
        std::mem::take(&mut self.autofill.queries)
    }

    /// The items the person picked since the last call, in order.
    pub fn take_autofill_picks(&mut self) -> Vec<AutofillPick> {
        std::mem::take(&mut self.autofill.picks)
    }

    /// What each shell is to be told of the list since the last call, in order: the list to
    /// draw, or none.
    pub fn take_autofill_tells(&mut self) -> Vec<(ClientId, Option<AutofillList>)> {
        std::mem::take(&mut self.autofill.tells)
    }

    /// The list shown now, if any, and the shell drawing it.
    pub fn autofill_shown(&self) -> Option<&(ClientId, AutofillList)> {
        self.autofill.told.as_ref()
    }

    /// A key while the list is shown: ↑ and ↓ move the highlight, Return picks it, Escape closes
    /// the list. Each is the list's, its release too; any other key goes on to the field.
    pub(super) fn autofill_key(&mut self, key: u32, pressed: bool) -> bool {
        if !pressed {
            return self.autofill.held.remove(&key);
        }
        if self.autofill.told.is_none() {
            return false;
        }
        let offered = self
            .autofill
            .offered
            .as_mut()
            .expect("a list told is offered");
        match key {
            KEY_UP => offered.highlighted = offered.highlighted.saturating_sub(1),
            KEY_DOWN => {
                offered.highlighted = (offered.highlighted + 1).min(offered.shown.len() - 1);
            }
            KEY_ENTER | KEY_KPENTER => {
                let index = offered.highlighted;
                self.pick_autofill(index);
            }
            KEY_ESC => offered.closed = true,
            _ => return false,
        }
        self.autofill.held.insert(key);
        self.tell_autofill();
        true
    }

    /// The shell's press on an item of the list it draws (`index` among those shown), citing an
    /// input it received: picked. A press of another shell, without an input, or for a list since
    /// hidden picks nothing.
    pub fn choose_autofill(&mut self, client: ClientId, index: usize, with_input: bool) -> bool {
        let drawing = self
            .autofill
            .told
            .as_ref()
            .is_some_and(|(shell, list)| *shell == client && index < list.items.len());
        let is_shell = self.clients.get(&client).is_some_and(|grants| grants.shell);
        if !(drawing && is_shell && with_input) {
            return false;
        }
        self.pick_autofill(index);
        self.tell_autofill();
        true
    }

    fn pick_autofill(&mut self, shown: usize) {
        let Some(offered) = self.autofill.offered.as_mut() else {
            return;
        };
        let Some(&index) = offered.shown.get(shown) else {
            return;
        };
        offered.closed = true;
        offered.picked = Some(index);
        self.autofill.picks.push(AutofillPick {
            token: offered.token,
            index,
        });
    }

    /// Fills the item picked for `token` (`take_autofill_picks`) into its field: `own` the field's
    /// whole text, `paired` its paired field's when it has one, as one transaction (ADR 0008),
    /// a user input as typing is. Only once per pick, only while the field is still the one
    /// offered (focused in the key window, unlocked); false, filling nothing, otherwise.
    pub fn fill_autofill(&mut self, token: u64, own: Option<&str>, paired: Option<&str>) -> bool {
        let bounded =
            |text: Option<&str>| text.is_none_or(|text| text.len() <= MAX_SURROUNDING_BYTES);
        if !bounded(own) || !bounded(paired) {
            return false;
        }
        let Some(offered) = self
            .autofill
            .offered
            .as_mut()
            .filter(|offered| offered.token == token)
        else {
            return false;
        };
        if offered.picked.take().is_none() {
            return false;
        }
        let (surface, field) = (offered.surface, offered.field);
        let still = self
            .autofill_target()
            .is_some_and(|(key, number, _, _)| key == surface && number == field);
        let Some(state) = self.text_input(surface).filter(|_| still) else {
            return false;
        };
        let (commit, has_pair) = (state.serial, state.paired);
        let paired = paired.filter(|_| has_pair);
        if own.is_none() && paired.is_none() {
            return false;
        }
        let serial = self.next_serial();
        self.last_user_serial = serial;
        self.record_press(serial, surface, false);
        for (is_paired, text) in [(false, own), (true, paired)] {
            let Some(text) = text else { continue };
            // An empty text is one empty chunk: the field is emptied.
            let chunks = TextChunk::split(text);
            let chunks = if chunks.is_empty() {
                vec![TextChunk::new("").expect("empty chunk")]
            } else {
                chunks
            };
            for text in chunks {
                self.emit(
                    surface.client,
                    Event::TextInputFill {
                        surface: surface.id,
                        paired: is_paired,
                        text,
                    },
                );
            }
        }
        self.emit(
            surface.client,
            Event::TextInputDone {
                surface: surface.id,
                serial,
                commit,
            },
        );
        true
    }

    /// The surface of the field the list is for.
    pub(super) fn autofill_surface(&self) -> Option<SurfaceKey> {
        self.autofill
            .offered
            .as_ref()
            .map(|offered| offered.surface)
    }

    /// A shell is gone: another draws the list.
    pub(super) fn forget_autofill_shell(&mut self, client: ClientId) {
        if self
            .autofill
            .told
            .as_ref()
            .is_some_and(|(shell, _)| *shell == client)
        {
            self.autofill.told = None;
            self.tell_autofill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_items_text_is_never_debugged() {
        let item = AutofillItem {
            kind: AutofillKind::Login,
            title: "example.com".into(),
            detail: "bea@example.com".into(),
        };
        let shown = format!("{item:?}");
        assert!(!shown.contains("example"));
    }

    #[test]
    fn a_login_matches_what_is_typed_from_its_start_whatever_the_case() {
        let item = AutofillItem {
            kind: AutofillKind::Login,
            title: "example.com".into(),
            detail: "Bea@example.com".into(),
        };
        assert!(matches(&item, ""));
        assert!(matches(&item, "bEA"));
        assert!(!matches(&item, "ea"));
    }
}
