//! Login items and background activity for the session
//! (docs/adr/0060-login-items-and-background-activity.mdx): the program running the compositor's
//! side of an application's `BackgroundItem` asks, as a Mac's `SMAppService` is the system's.
//!
//! An application has two kinds of background item. Its **login item** is itself, opened at login:
//! a place in the person's Open at Login list (`open_at_login`, ADR 0034), the one list the dock's
//! menu and Settings' Login Items & Extensions change too; registering adds it there and
//! unregistering takes it out. Its **background activity** is its process kept running once its
//! window closes, its JavaScript going on without a window (a chat's connection, a sync), until it
//! ends itself, unregisters, or the person turns it off: one process of an application at a time.
//!
//! Who may have them is the session's to decide, never the client's claim: a process the session
//! launched (its launch record, ADR 0013), with the capability that guards each (`login-item`,
//! `background`, ADR 0030, decision 8). A system application of the image's policy has them; a
//! development entry, unverified, has none; an installed application has what it declared, as its
//! account decided: granted when it first registers (the person is told by a notification, as a
//! Mac tells them, and turns it off in Login Items & Extensions), refused once the person turned it
//! off (`RequiresApproval`) until they turn it on again.
//!
//! Turned off, it applies at once: the application's process running in the background is ended,
//! and its processes with windows lose the right, so each ends as its window closes, as any
//! application's does.

use crate::installs::{Decision, Grant, Installs};
use crate::settings::Settings;
use crate::Compositor;
use oxyos_apps::{Launched, Origin};
use oxyos_compositor_core::{BackgroundItemRequest, ClientId, Scene};
use oxyos_protocol::{BackgroundItemAction, BackgroundItemKind, BackgroundStatus};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Who the session's own notifications are posted as: Settings, where they are managed.
pub const NOTIFIER: &str = "so.oxy.settings";
/// What pressing one opens: Settings' Login Items & Extensions.
pub const LOGIN_ITEMS_URL: &str = "oxyos-settings://login-items";

/// The capability that guards a kind of item (ADR 0030, decision 8).
pub fn capability_of(kind: BackgroundItemKind) -> &'static str {
    match kind {
        BackgroundItemKind::LoginItem => "login-item",
        BackgroundItemKind::Background => "background",
    }
}

fn kind_of(capability: &str) -> Option<BackgroundItemKind> {
    match capability {
        "login-item" => Some(BackgroundItemKind::LoginItem),
        "background" => Some(BackgroundItemKind::Background),
        _ => None,
    }
}

/// Whether an application may have a kind of item, as its launch record and its account's
/// decisions say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Allowance {
    /// A system application, or an installed one whose account granted it.
    Allowed,
    /// An installed application that declared it and was never decided: granted as it registers.
    Undecided,
    /// The person turned it off.
    Denied,
    /// Not a process the session launched, a development entry, or an installed application that
    /// did not declare it.
    NotPermitted,
}

/// The notification's words in the session's language.
pub fn added_notification(spanish: bool, name: &str) -> (&'static str, String) {
    if spanish {
        (
            "Ítems en segundo plano añadidos",
            format!(
                "«{name}» ha añadido un ítem que puede ejecutarse en segundo plano. Puedes gestionarlo en Ítems de inicio y extensiones."
            ),
        )
    } else {
        (
            "Background Items Added",
            format!(
                "“{name}” added an item that can run in the background. You can manage this in Login Items & Extensions."
            ),
        )
    }
}

/// A process of an application that registered its background activity.
#[derive(Clone, Debug)]
struct Registered {
    id: String,
    /// The person quit its application: once its windows close, it ends rather than running on.
    quit: bool,
    /// It has shown a window: once it shows none, it is in the background.
    shown: bool,
}

/// A connected client: its process, and what the session launched it as (none: not a process
/// the session launched).
type Client = (ClientId, u32, Option<Launched>);

/// What a turn brings: the applications' asks, the registrations the install service granted (each
/// with whether it kept the decision) and the decisions it kept.
#[derive(Default)]
struct Turn {
    requests: Vec<BackgroundItemRequest>,
    grants: Vec<(Grant, bool)>,
    decisions: Vec<Decision>,
    /// The clients whose application the person quit.
    quits: Vec<ClientId>,
}

impl Turn {
    fn is_empty(&self) -> bool {
        self.requests.is_empty() && self.grants.is_empty() && self.decisions.is_empty()
    }
}

/// The session's login items and background activity.
pub struct BackgroundActivity {
    /// The install service's entries for the session's account
    /// (`oxyos_apps::installed_applications`): an installed application's decisions are beside
    /// them, read fresh at each ask.
    installed: Option<PathBuf>,
    /// Each process that registered its background activity, by pid.
    registered: BTreeMap<u32, Registered>,
    /// The application's process running in the background now, by id.
    in_background: BTreeMap<String, u32>,
    /// Open at Login as last seen: an application whose place in it the person changed is told.
    login_items: Option<Vec<String>>,
    /// The session's language, for the notification.
    spanish: bool,
}

impl BackgroundActivity {
    pub fn new(installed: Option<PathBuf>) -> Self {
        Self {
            installed,
            registered: BTreeMap::new(),
            in_background: BTreeMap::new(),
            login_items: None,
            spanish: false,
        }
    }

    /// The applications with a process running in the background now.
    pub fn in_background(&self) -> BTreeSet<String> {
        self.in_background.keys().cloned().collect()
    }

    /// Each application's process running in the background now, by id: its pid.
    pub fn background_processes(&self) -> &BTreeMap<String, u32> {
        &self.in_background
    }

    /// Whether `launched` may have a kind of item.
    pub fn allowance(&self, launched: Option<&Launched>, kind: BackgroundItemKind) -> Allowance {
        let Some(launched) = launched else {
            return Allowance::NotPermitted;
        };
        match launched.origin {
            Origin::System => Allowance::Allowed,
            Origin::Development => Allowance::NotPermitted,
            Origin::Installed { .. } => {
                let decisions = self
                    .installed
                    .as_deref()
                    .map(|installed| oxyos_apps::capability_decisions(installed, &launched.id))
                    .unwrap_or_default();
                match decisions.get(capability_of(kind)).map(String::as_str) {
                    Some("granted") => Allowance::Allowed,
                    Some("undecided") => Allowance::Undecided,
                    Some("denied") => Allowance::Denied,
                    _ => Allowance::NotPermitted,
                }
            }
        }
    }

    /// How a process's item stands.
    fn status(
        &self,
        pid: u32,
        launched: Option<&Launched>,
        kind: BackgroundItemKind,
        login_items: &[String],
    ) -> BackgroundStatus {
        let allowance = self.allowance(launched, kind);
        let Some(launched) = launched else {
            return BackgroundStatus::NotPermitted;
        };
        match kind {
            // Its place in the person's list, whoever put it there.
            BackgroundItemKind::LoginItem if login_items.contains(&launched.id) => {
                BackgroundStatus::Enabled
            }
            BackgroundItemKind::Background
                if allowance == Allowance::Allowed && self.registered.contains_key(&pid) =>
            {
                BackgroundStatus::Enabled
            }
            _ => match allowance {
                Allowance::NotPermitted => BackgroundStatus::NotPermitted,
                Allowance::Denied => BackgroundStatus::RequiresApproval,
                Allowance::Allowed | Allowance::Undecided => BackgroundStatus::NotRegistered,
            },
        }
    }

    /// Whether a process that registered its background activity runs: its windows are watched
    /// each turn.
    pub fn watching(&self) -> bool {
        !self.registered.is_empty()
    }

    /// A turn: the applications' asks, the registrations the install service granted, the
    /// decisions it kept, the person's Open at Login, and the windows of the processes that
    /// registered their background activity. `launched` says what the session launched a process
    /// as. Returns the processes to end: one in the background without the right, or a second of
    /// its application. Nothing is looked at while nothing of it changed.
    pub fn serve(
        &mut self,
        compositor: &mut Compositor,
        settings: &mut Settings,
        launched: &dyn Fn(u32) -> Option<Launched>,
        mut installs: Option<&mut Installs>,
        now: i64,
    ) -> Vec<u32> {
        let turn = Turn {
            requests: compositor.scene.take_background_item_requests(),
            quits: compositor.scene.take_quits(),
            grants: installs
                .as_deref_mut()
                .map(Installs::take_grants)
                .unwrap_or_default(),
            decisions: installs
                .as_deref_mut()
                .map(Installs::take_decisions)
                .unwrap_or_default(),
        };
        let list_changed = self.login_items.as_ref() != Some(&settings.preferences().open_at_login);
        if turn.is_empty() && !list_changed && !self.watching() {
            return Vec::new();
        }
        // Who each connected client is, by its process.
        let clients: Vec<Client> = compositor
            .peers
            .iter()
            .filter_map(|(client, peer)| {
                let pid = u32::try_from(peer.pid).ok()?;
                Some((*client, pid, launched(pid)))
            })
            .collect();
        let showing = if self.watching() {
            compositor.pids_showing_windows()
        } else {
            BTreeSet::new()
        };
        self.serve_clients(
            compositor, settings, turn, &clients, &showing, installs, now,
        )
    }

    /// [`BackgroundActivity::serve`] with what the turn holds, the clients connected (each with its
    /// process and what the session launched it as) and the processes showing a window.
    #[allow(clippy::too_many_arguments)]
    fn serve_clients(
        &mut self,
        compositor: &mut Compositor,
        settings: &mut Settings,
        turn: Turn,
        clients: &[Client],
        showing: &BTreeSet<u32>,
        installs: Option<&mut Installs>,
        now: i64,
    ) -> Vec<u32> {
        let mut installs = installs;
        let Turn {
            requests,
            grants,
            decisions,
            quits,
        } = turn;
        let mut end = Vec::new();
        let login_items = settings.preferences().open_at_login.clone();
        let list_changed = self.login_items.as_ref() != Some(&login_items);
        self.spanish = oxyos_preferences::session_locale(
            settings.preferences(),
            oxyos_preferences::environment_locale().as_deref(),
        )
        .starts_with("es");
        let of = |client: ClientId| clients.iter().find(|(c, _, _)| *c == client);
        // What Settings is told anew: what runs in the background, or a decision it did not make.
        let mut background_changed = false;
        let mut decided_here = false;

        // The person changed Open at Login (the dock's menu, Settings): each application whose
        // place in it changed is told, unasked.
        if list_changed {
            if let Some(before) = self.login_items.replace(login_items.clone()) {
                let changed: BTreeSet<&String> = before
                    .iter()
                    .filter(|id| !login_items.contains(id))
                    .chain(login_items.iter().filter(|id| !before.contains(id)))
                    .collect();
                for (client, pid, record) in clients {
                    if let Some(record) = record.as_ref().filter(|r| changed.contains(&r.id)) {
                        let status = self.status(
                            *pid,
                            Some(record),
                            BackgroundItemKind::LoginItem,
                            &login_items,
                        );
                        compositor.scene.background_item_state(
                            *client,
                            0,
                            BackgroundItemKind::LoginItem,
                            status,
                        );
                    }
                }
            }
        }

        for BackgroundItemRequest {
            client,
            serial,
            kind,
            action,
        } in requests
        {
            let Some((_, pid, record)) = of(client) else {
                continue;
            };
            let (pid, record) = (*pid, record.clone());
            let answer = |scene: &mut Scene, status| {
                scene.background_item_state(client, serial, kind, status);
            };
            let login_items = settings.preferences().open_at_login.clone();
            let allowance = self.allowance(record.as_ref(), kind);
            match (action, record) {
                (BackgroundItemAction::Status, record) => {
                    let status = self.status(pid, record.as_ref(), kind, &login_items);
                    answer(&mut compositor.scene, status);
                }
                (_, None) => answer(&mut compositor.scene, BackgroundStatus::NotPermitted),
                (BackgroundItemAction::Register, Some(record)) => match allowance {
                    Allowance::NotPermitted => {
                        answer(&mut compositor.scene, BackgroundStatus::NotPermitted)
                    }
                    Allowance::Denied => {
                        answer(&mut compositor.scene, BackgroundStatus::RequiresApproval)
                    }
                    Allowance::Undecided => match installs.as_deref_mut() {
                        // Granted as it registers: the account's decision first, then the item.
                        Some(installs) => installs.grant(Grant {
                            client,
                            serial,
                            id: record.id.clone(),
                            capability: capability_of(kind).to_string(),
                        }),
                        // No install service to keep the decision: nothing is registered.
                        None => answer(&mut compositor.scene, BackgroundStatus::NotRegistered),
                    },
                    Allowance::Allowed => {
                        // Told the person only when it added something: a login item it put in
                        // the list. A system application's background activity is the system's.
                        let status =
                            self.register(compositor, settings, pid, &record, kind, false, now);
                        answer(&mut compositor.scene, status);
                    }
                },
                (BackgroundItemAction::Unregister, Some(record)) => {
                    match kind {
                        BackgroundItemKind::LoginItem => {
                            let rest: Vec<String> = login_items
                                .iter()
                                .filter(|id| **id != record.id)
                                .cloned()
                                .collect();
                            if rest.len() != login_items.len() {
                                self.keep_login_items(compositor, settings, rest, now);
                            }
                        }
                        BackgroundItemKind::Background => {
                            // Without the right, a process in the background ends: it asked to.
                            if self.registered.remove(&pid).is_some()
                                && self.in_background.get(&record.id) == Some(&pid)
                            {
                                self.in_background.remove(&record.id);
                                background_changed = true;
                                end.push(pid);
                            }
                        }
                    }
                    let login_items = settings.preferences().open_at_login.clone();
                    let status = self.status(pid, Some(&record), kind, &login_items);
                    answer(&mut compositor.scene, status);
                }
            }
        }

        // The registrations the install service granted: the item, and the person told.
        for (grant, kept) in grants {
            let Some(kind) = kind_of(&grant.capability) else {
                continue;
            };
            decided_here |= kept;
            let found = of(grant.client).and_then(|(_, pid, record)| Some((*pid, record.clone()?)));
            match found {
                Some((pid, record)) if kept => {
                    let status = self.register(compositor, settings, pid, &record, kind, true, now);
                    compositor.scene.background_item_state(
                        grant.client,
                        grant.serial,
                        kind,
                        status,
                    );
                }
                Some(_) => compositor.scene.background_item_state(
                    grant.client,
                    grant.serial,
                    kind,
                    BackgroundStatus::NotRegistered,
                ),
                None => {}
            }
        }

        // The person's decisions, applied at once: the process in the background of an
        // application turned off ends, and each of its processes is told how its item stands.
        for Decision {
            id,
            capability,
            granted,
        } in decisions
        {
            let Some(kind) = kind_of(&capability) else {
                continue;
            };
            if kind == BackgroundItemKind::Background && !granted {
                if let Some(pid) = self.in_background.remove(&id) {
                    self.registered.remove(&pid);
                    background_changed = true;
                    end.push(pid);
                }
            }
            let login_items = settings.preferences().open_at_login.clone();
            for (client, pid, record) in clients {
                let Some(record) = record.as_ref().filter(|r| r.id == id) else {
                    continue;
                };
                // A background item is the processes' that registered it; a login item, every
                // process's of the application.
                if kind == BackgroundItemKind::Background && !self.registered.contains_key(pid) {
                    continue;
                }
                let status = self.status(*pid, Some(record), kind, &login_items);
                compositor
                    .scene
                    .background_item_state(*client, 0, kind, status);
            }
        }

        // Quit (the dock's, the switcher's, Super+Q) ends an application: a process of it that
        // registered its background activity ends as its windows close.
        for client in quits {
            if let Some((_, pid, _)) = of(client) {
                if let Some(registered) = self.registered.get_mut(pid) {
                    registered.quit = true;
                }
            }
        }

        // The windows of the processes that registered: one that showed a window and shows none is
        // in the background, if it may be and its application has none there yet; else it ends.
        if self.watching() {
            let running: BTreeSet<u32> = clients.iter().map(|(_, pid, _)| *pid).collect();
            // Those gone are forgotten.
            let gone: Vec<u32> = self
                .registered
                .keys()
                .filter(|pid| !running.contains(pid))
                .copied()
                .collect();
            for pid in gone {
                if let Some(registered) = self.registered.remove(&pid) {
                    if self.in_background.get(&registered.id) == Some(&pid) {
                        self.in_background.remove(&registered.id);
                        background_changed = true;
                    }
                }
            }
            let pids: Vec<u32> = self.registered.keys().copied().collect();
            for pid in pids {
                if showing.contains(&pid) {
                    self.registered.get_mut(&pid).expect("listed").shown = true;
                    continue;
                }
                let registered = self.registered[&pid].clone();
                if !registered.shown || self.in_background.get(&registered.id) == Some(&pid) {
                    continue;
                }
                let record = clients
                    .iter()
                    .find(|(_, p, _)| *p == pid)
                    .and_then(|(_, _, record)| record.clone());
                let allowed = self.allowance(record.as_ref(), BackgroundItemKind::Background)
                    == Allowance::Allowed;
                if allowed && !registered.quit && !self.in_background.contains_key(&registered.id) {
                    eprintln!(
                        "[background] {} runs in the background ({pid})",
                        registered.id
                    );
                    self.in_background.insert(registered.id, pid);
                    background_changed = true;
                } else {
                    eprintln!(
                        "[background] {} ended: {}",
                        registered.id,
                        if registered.quit {
                            "the person quit it"
                        } else if allowed {
                            "its application already runs in the background"
                        } else {
                            "it may not run in the background"
                        }
                    );
                    self.registered.remove(&pid);
                    end.push(pid);
                }
            }
        }

        if background_changed || decided_here {
            if let Some(installs) = installs {
                installs.set_in_background(self.in_background());
                installs.tell_privacy(&compositor.scene);
            }
        }
        end
    }

    /// Registers a kind of item an application may have; how it then stands. `granted_now`: its
    /// capability was granted as it registered, so it is new to the person, who is told.
    #[allow(clippy::too_many_arguments)]
    fn register(
        &mut self,
        compositor: &mut Compositor,
        settings: &mut Settings,
        pid: u32,
        record: &Launched,
        kind: BackgroundItemKind,
        granted_now: bool,
        now: i64,
    ) -> BackgroundStatus {
        let added = match kind {
            BackgroundItemKind::LoginItem => {
                let mut login_items = settings.preferences().open_at_login.clone();
                if login_items.contains(&record.id) {
                    false
                } else {
                    login_items.push(record.id.clone());
                    if !self.keep_login_items(compositor, settings, login_items, now) {
                        return BackgroundStatus::NotRegistered;
                    }
                    true
                }
            }
            BackgroundItemKind::Background => {
                self.registered.entry(pid).or_insert(Registered {
                    id: record.id.clone(),
                    quit: false,
                    shown: false,
                });
                granted_now
            }
        };
        if added {
            let (title, body) = added_notification(self.spanish, &record.name);
            let identifier = format!("background-items:{}", record.id);
            if let Err(outcome) = compositor.scene.post_system_notification_opening(
                NOTIFIER,
                &identifier,
                title,
                &body,
                false,
                LOGIN_ITEMS_URL,
            ) {
                eprintln!(
                    "[background] the person was not told {}: {outcome:?}",
                    record.id
                );
            }
        }
        BackgroundStatus::Enabled
    }

    /// Keeps Open at Login as `ids`, as the person's own change is kept, and remembers it so the
    /// application that changed it is not told again: whether it was kept.
    fn keep_login_items(
        &mut self,
        compositor: &mut Compositor,
        settings: &mut Settings,
        ids: Vec<String>,
        now: i64,
    ) -> bool {
        match settings.set_open_at_login(compositor, &ids, now) {
            Ok(_) => {
                self.login_items = Some(settings.preferences().open_at_login.clone());
                true
            }
            Err(error) => {
                eprintln!("[background] Open at Login {ids:?} not kept: {error}");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests;
