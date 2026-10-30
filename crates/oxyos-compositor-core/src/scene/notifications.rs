//! Notifications (docs/adr/0026-notifications.mdx). An application posts one, its texts in chunks,
//! and the scene keeps it for the session: the application it is from is its client's launch
//! record (ADR 0013), never a claim, so only a process the session started posts at all. The
//! session's shell is told each, and shows it as a banner when the policy the program running the
//! compositor told allows (Do Not Disturb off, the session unlocked), and in its Notification
//! Center; while locked it is told what a notification says only if previews are allowed. The
//! shell activates one (or one of its actions, with a reply): the scene gives the application the
//! response once, brings its windows forward, and asks the program to start it first if none of
//! its clients runs. Nothing is kept beyond the session.

use super::launch::assemble;
use super::{ClientId, Scene, SurfaceKey};
use oxyos_protocol::{
    ErrorCode, Event, NotificationField, NotificationOutcome, PreeditText, TextChunk,
    MAX_NOTIFICATION_ACTIONS, MAX_NOTIFICATION_ACTION_BYTES, MAX_NOTIFICATION_BODY_BYTES,
    MAX_NOTIFICATION_DATA_BYTES, MAX_NOTIFICATION_IDENTIFIER_BYTES, MAX_NOTIFICATION_REPLY_BYTES,
    MAX_NOTIFICATION_TITLE_BYTES,
};
use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

/// Notifications kept for one application: past it, its oldest goes.
pub const MAX_NOTIFICATIONS_PER_APPLICATION: usize = 32;
/// Notifications kept in all: past it, the oldest goes.
pub const MAX_NOTIFICATIONS: usize = 256;
/// An application posts at most this many in any [`NOTIFICATION_WINDOW`]; more are throttled.
pub const NOTIFICATION_BURST: usize = 5;
pub const NOTIFICATION_WINDOW: Duration = Duration::from_secs(10);
/// Posts a client may have begun and not posted.
const MAX_RECEIVING_POSTS: usize = 4;
/// Responses kept for an application that is being started, until one of its clients connects.
const MAX_PENDING_RESPONSES: usize = 8;

/// How the program running the compositor wants notifications shown, from the user's preferences
/// (docs/adr/0019-settings.mdx): Do Not Disturb keeps every one in the Notification Center and
/// shows no banner; `previews` lets the lock screen show what they say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotificationPolicy {
    pub do_not_disturb: bool,
    pub previews: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Action {
    id: String,
    label: String,
    /// A reply action's placeholder: the shell offers a field for the reply.
    reply: Option<String>,
    /// It answers without bringing the application forward.
    background: bool,
}

/// What an application posted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Content {
    identifier: String,
    title: String,
    subtitle: String,
    body: String,
    data: String,
    actions: Vec<Action>,
}

#[derive(Clone, Debug)]
struct Kept {
    number: u32,
    application: String,
    content: Content,
    posted: Duration,
    /// The shells were told only its application (posted while locked, without previews).
    withheld: bool,
}

/// A response for an application none of whose clients runs yet: given to the first that
/// connects as it.
#[derive(Clone, Debug)]
struct Response {
    number: u32,
    content: Content,
    action: u8,
    reply: String,
}

#[derive(Debug, Default)]
pub(super) struct Notifications {
    /// Texts being received, by the client's post and the text: what came so far.
    receiving: BTreeMap<(ClientId, u32, NotificationField, u8), (String, u8)>,
    /// Posts begun, by the client's serial: the texts received whole.
    posts: BTreeMap<(ClientId, u32), Content>,
    /// Kept, oldest first.
    kept: VecDeque<Kept>,
    next: u32,
    /// When each application posted lately, oldest first.
    posted: BTreeMap<String, VecDeque<Duration>>,
    policy: NotificationPolicy,
    /// What the shells were last told of the policy.
    told_policy: BTreeMap<ClientId, NotificationPolicy>,
    /// Replies the shell is typing, by notification: what came so far.
    replies: BTreeMap<(ClientId, u32), (String, u8)>,
    replied: BTreeMap<u32, String>,
    /// Responses waiting for their application to start, by application.
    pending: BTreeMap<String, Vec<Response>>,
    /// Applications to start for a response, for the program running the compositor.
    launches: Vec<String>,
}

/// The most bytes of a field's text.
fn limit(field: NotificationField) -> usize {
    match field {
        NotificationField::Identifier => MAX_NOTIFICATION_IDENTIFIER_BYTES,
        NotificationField::Title | NotificationField::Subtitle => MAX_NOTIFICATION_TITLE_BYTES,
        NotificationField::Body => MAX_NOTIFICATION_BODY_BYTES,
        NotificationField::Data => MAX_NOTIFICATION_DATA_BYTES,
        NotificationField::ActionId
        | NotificationField::ActionLabel
        | NotificationField::ActionReply => MAX_NOTIFICATION_ACTION_BYTES,
        NotificationField::Application | NotificationField::Reply => 0,
    }
}

impl Content {
    fn set(&mut self, field: NotificationField, action: u8, text: String) {
        let actions = &mut self.actions;
        let mut of_action = |set: &mut dyn FnMut(&mut Action)| {
            let index = usize::from(action) - 1;
            if actions.len() <= index {
                actions.resize(index + 1, Action::default());
            }
            set(&mut actions[index]);
        };
        match field {
            NotificationField::Identifier => self.identifier = text,
            NotificationField::Title => self.title = text,
            NotificationField::Subtitle => self.subtitle = text,
            NotificationField::Body => self.body = text,
            NotificationField::Data => self.data = text,
            NotificationField::ActionId => of_action(&mut |a| a.id = text.clone()),
            NotificationField::ActionLabel => of_action(&mut |a| a.label = text.clone()),
            NotificationField::ActionReply => of_action(&mut |a| a.reply = Some(text.clone())),
            NotificationField::Application | NotificationField::Reply => {}
        }
    }
}

impl Scene {
    /// The time now, from the program running the compositor (its `CLOCK_MONOTONIC`, or a
    /// test's): what notifications' rate and age are measured on. The scene never reads a clock.
    pub fn set_clock(&mut self, now: Duration) {
        self.clock = self.clock.max(now);
    }

    /// The policy from the user's preferences: the shells are told it, and the banners and the
    /// lock screen follow it from the next notification.
    pub fn set_notification_policy(&mut self, policy: NotificationPolicy) {
        self.notifications.policy = policy;
        self.tell_notification_policy();
    }

    pub(super) fn notification_request(
        &mut self,
        client: ClientId,
        request: oxyos_protocol::Request,
    ) -> Result<(), ErrorCode> {
        use oxyos_protocol::Request;
        let shell = self.clients[&client].shell;
        match request {
            Request::NotificationText {
                serial,
                field,
                action,
                index,
                last,
                text,
            } => self.notification_text(client, serial, field, action, index, last, text),
            Request::PostNotification {
                serial,
                banner,
                background,
            } => {
                self.post_notification(client, serial, banner, background);
                Ok(())
            }
            Request::WithdrawNotification { notification } => {
                self.withdraw_notification(client, shell, notification);
                Ok(())
            }
            Request::NotificationReply {
                notification,
                index,
                last,
                text,
            } => {
                if !shell {
                    return Err(ErrorCode::Unauthorized);
                }
                let Some(reply) = assemble(
                    &mut self.notifications.replies,
                    (client, notification),
                    index,
                    last,
                    text,
                    MAX_NOTIFICATION_REPLY_BYTES,
                )?
                else {
                    return Ok(());
                };
                self.notifications.replied.insert(notification, reply);
                Ok(())
            }
            Request::ActivateNotification {
                notification,
                action,
                serial,
            } => {
                if !shell {
                    return Err(ErrorCode::Unauthorized);
                }
                self.activate_notification(client, notification, action, serial);
                Ok(())
            }
            _ => unreachable!("only notifications' requests are passed here"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn notification_text(
        &mut self,
        client: ClientId,
        serial: u32,
        field: NotificationField,
        action: u8,
        index: u8,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        let max = limit(field);
        let of_action = matches!(
            field,
            NotificationField::ActionId
                | NotificationField::ActionLabel
                | NotificationField::ActionReply
        );
        // What the session says (the application, the reply typed) is never the application's;
        // an action is numbered from 1, and nothing else has one.
        if max == 0
            || (of_action && !(1..=MAX_NOTIFICATION_ACTIONS).contains(&usize::from(action)))
            || (!of_action && action != 0)
        {
            return Err(ErrorCode::InvalidMessage);
        }
        let notifications = &mut self.notifications;
        if !notifications.posts.contains_key(&(client, serial))
            && notifications
                .posts
                .keys()
                .filter(|(poster, _)| *poster == client)
                .count()
                >= MAX_RECEIVING_POSTS
        {
            return Err(ErrorCode::QuotaExceeded);
        }
        let Some(whole) = assemble(
            &mut notifications.receiving,
            (client, serial, field, action),
            index,
            last,
            text,
            max,
        )?
        else {
            notifications.posts.entry((client, serial)).or_default();
            return Ok(());
        };
        notifications
            .posts
            .entry((client, serial))
            .or_default()
            .set(field, action, whole);
        Ok(())
    }

    fn post_notification(&mut self, client: ClientId, serial: u32, banner: bool, background: u8) {
        let notifications = &mut self.notifications;
        let mut content = notifications
            .posts
            .remove(&(client, serial))
            .unwrap_or_default();
        notifications
            .receiving
            .retain(|(poster, post, _, _), _| (*poster, *post) != (client, serial));
        let answer = |scene: &mut Scene, notification, outcome| {
            scene.emit(
                client,
                Event::NotificationResult {
                    serial,
                    notification,
                    outcome,
                },
            );
        };
        // The application is its launch record's: a process the session did not start posts
        // nothing.
        let Some(application) = self.application(client).map(str::to_string) else {
            answer(self, 0, NotificationOutcome::Refused);
            return;
        };
        // An action needs an id and a label; one without either is dropped with those after it.
        if let Some(broken) = content
            .actions
            .iter()
            .position(|action| action.id.is_empty() || action.label.is_empty())
        {
            content.actions.truncate(broken);
        }
        for (index, action) in content.actions.iter_mut().enumerate() {
            action.background = background & (1 << index) != 0;
        }
        let now = self.clock;
        let recent = self
            .notifications
            .posted
            .entry(application.clone())
            .or_default();
        while recent
            .front()
            .is_some_and(|at| now.saturating_sub(*at) >= NOTIFICATION_WINDOW)
        {
            recent.pop_front();
        }
        if recent.len() >= NOTIFICATION_BURST {
            answer(self, 0, NotificationOutcome::Throttled);
            return;
        }
        recent.push_back(now);
        // The same identifier again replaces the application's notification, as a Mac's does.
        if !content.identifier.is_empty() {
            let replaced: Vec<u32> = self
                .notifications
                .kept
                .iter()
                .filter(|kept| {
                    kept.application == application && kept.content.identifier == content.identifier
                })
                .map(|kept| kept.number)
                .collect();
            for number in replaced {
                self.remove_notification(number);
            }
        }
        let theirs = self
            .notifications
            .kept
            .iter()
            .filter(|kept| kept.application == application)
            .count();
        if theirs >= MAX_NOTIFICATIONS_PER_APPLICATION {
            let oldest = self
                .notifications
                .kept
                .iter()
                .find(|kept| kept.application == application)
                .map(|kept| kept.number);
            if let Some(number) = oldest {
                self.remove_notification(number);
            }
        }
        if self.notifications.kept.len() >= MAX_NOTIFICATIONS {
            if let Some(number) = self.notifications.kept.front().map(|kept| kept.number) {
                self.remove_notification(number);
            }
        }
        let notifications = &mut self.notifications;
        notifications.next += 1;
        let number = notifications.next;
        let withheld = self.locked && !notifications.policy.previews;
        notifications.kept.push_back(Kept {
            number,
            application,
            content,
            posted: now,
            withheld,
        });
        answer(self, number, NotificationOutcome::Posted);
        let show = banner && !self.notifications.policy.do_not_disturb && !self.locked;
        for shell in self.shell_clients() {
            self.tell_notification(shell, number, show);
        }
    }

    /// The clients of `application`, by their launch records.
    fn application_clients(&self, application: &str) -> Vec<ClientId> {
        self.clients
            .keys()
            .copied()
            .filter(|client| self.application(*client) == Some(application))
            .collect()
    }

    fn emit_text(
        &mut self,
        client: ClientId,
        notification: u32,
        field: NotificationField,
        action: u8,
        text: &str,
    ) {
        let chunks = TextChunk::split(text);
        let count = chunks.len().max(1);
        for index in 0..count {
            let chunk = chunks.get(index).map_or("", TextChunk::as_str);
            self.emit(
                client,
                Event::NotificationText {
                    notification,
                    field,
                    action,
                    index: index as u8,
                    last: index + 1 == count,
                    text: PreeditText::new(chunk).expect("a chunk fits"),
                },
            );
        }
    }

    /// Tells a shell one notification: its application, then (unless withheld) what it says and
    /// its actions, then that it is whole.
    fn tell_notification(&mut self, shell: ClientId, number: u32, banner: bool) {
        let Some(kept) = self
            .notifications
            .kept
            .iter()
            .find(|kept| kept.number == number)
            .cloned()
        else {
            return;
        };
        self.emit_text(
            shell,
            number,
            NotificationField::Application,
            0,
            &kept.application,
        );
        let withheld = kept.withheld;
        if !withheld {
            let content = &kept.content;
            for (field, text) in [
                (NotificationField::Title, &content.title),
                (NotificationField::Subtitle, &content.subtitle),
                (NotificationField::Body, &content.body),
            ] {
                if !text.is_empty() {
                    self.emit_text(shell, number, field, 0, text);
                }
            }
            for (index, action) in content.actions.iter().enumerate() {
                let n = index as u8 + 1;
                self.emit_text(
                    shell,
                    number,
                    NotificationField::ActionLabel,
                    n,
                    &action.label,
                );
                if let Some(placeholder) = &action.reply {
                    self.emit_text(
                        shell,
                        number,
                        NotificationField::ActionReply,
                        n,
                        placeholder,
                    );
                }
            }
        }
        let age_s = self.clock.saturating_sub(kept.posted).as_secs();
        self.emit(
            shell,
            Event::Notification {
                notification: number,
                banner,
                actions: if withheld {
                    0
                } else {
                    kept.content.actions.len() as u8
                },
                age_s: age_s.min(u64::from(u32::MAX)) as u32,
                withheld,
            },
        );
    }

    /// A shell connected (or was given the grant): every notification kept, none as a banner,
    /// and the policy.
    pub(super) fn tell_notifications(&mut self, shell: ClientId) {
        let numbers: Vec<u32> = self
            .notifications
            .kept
            .iter()
            .map(|kept| kept.number)
            .collect();
        self.tell_notification_policy();
        for number in numbers {
            self.tell_notification(shell, number, false);
        }
    }

    fn tell_notification_policy(&mut self) {
        let policy = self.notifications.policy;
        for shell in self.shell_clients() {
            if self.notifications.told_policy.get(&shell) != Some(&policy) {
                self.notifications.told_policy.insert(shell, policy);
                self.emit(
                    shell,
                    Event::NotificationPolicy {
                        do_not_disturb: policy.do_not_disturb,
                        previews: policy.previews,
                    },
                );
            }
        }
    }

    /// The session unlocked: what was withheld is told to the shells now, as no banner.
    pub(super) fn tell_withheld_notifications(&mut self) {
        let withheld: Vec<u32> = self
            .notifications
            .kept
            .iter_mut()
            .filter(|kept| kept.withheld)
            .map(|kept| {
                kept.withheld = false;
                kept.number
            })
            .collect();
        for shell in self.shell_clients() {
            for number in &withheld {
                self.tell_notification(shell, *number, false);
            }
        }
    }

    /// Forgets a notification and tells the shells and its application's clients.
    fn remove_notification(&mut self, number: u32) -> Option<Kept> {
        let index = self
            .notifications
            .kept
            .iter()
            .position(|kept| kept.number == number)?;
        let kept = self.notifications.kept.remove(index)?;
        self.notifications.replied.remove(&number);
        let mut told = self.shell_clients();
        told.extend(self.application_clients(&kept.application));
        told.sort_unstable();
        told.dedup();
        for client in told {
            self.emit(
                client,
                Event::NotificationRemoved {
                    notification: number,
                },
            );
        }
        Some(kept)
    }

    fn withdraw_notification(&mut self, client: ClientId, shell: bool, notification: u32) {
        let application = self.application(client).map(str::to_string);
        let numbers: Vec<u32> = self
            .notifications
            .kept
            .iter()
            .filter(|kept| {
                (notification == 0 || kept.number == notification)
                    && (shell || application.as_deref() == Some(kept.application.as_str()))
            })
            .map(|kept| kept.number)
            .collect();
        for number in numbers {
            self.remove_notification(number);
        }
    }

    /// The person pressed a notification in the shell (`action` 0) or one of its actions: it
    /// goes, its application is given the response once and, unless the action answers in the
    /// background, comes forward; one none of whose clients runs is started first. Nothing acts
    /// while locked, nor for an input the shell did not receive lately.
    fn activate_notification(&mut self, shell: ClientId, number: u32, action: u8, serial: u64) {
        let reply = self.notifications.replied.remove(&number);
        if self.locked || !self.recent_input(shell, serial) {
            return;
        }
        let Some(kept) = self
            .notifications
            .kept
            .iter()
            .find(|kept| kept.number == number)
        else {
            return;
        };
        if kept.withheld || usize::from(action) > kept.content.actions.len() {
            return;
        }
        let chosen = action
            .checked_sub(1)
            .map(|index| kept.content.actions[usize::from(index)].clone());
        // A reply goes only with a reply action.
        let reply = match &chosen {
            Some(Action { reply: Some(_), .. }) => reply.unwrap_or_default(),
            _ => String::new(),
        };
        let background = chosen.as_ref().is_some_and(|chosen| chosen.background);
        let kept = self.remove_notification(number).expect("kept");
        let response = Response {
            number,
            content: kept.content,
            action,
            reply,
        };
        // The window used last of the application's, and the client to answer: that window's,
        // else any of its clients.
        let clients = self.application_clients(&kept.application);
        let window = self
            .nodes
            .iter()
            .filter(|(key, node)| {
                clients.contains(&key.client)
                    && node.role == super::Role::Toplevel
                    && !node.dismissed
                    && node.mapped
                    && self.window_number(**key).is_some()
            })
            .map(|(key, _)| *key)
            .max_by_key(|key| (self.window_use(*key).0, key.client, key.id));
        let Some(client) = window.map(|key| key.client).or(clients.first().copied()) else {
            // None runs: the program starts it, and its first client is given the response.
            let pending = self
                .notifications
                .pending
                .entry(kept.application.clone())
                .or_default();
            if pending.len() >= MAX_PENDING_RESPONSES {
                pending.remove(0);
            }
            pending.push(response);
            if !self.notifications.launches.contains(&kept.application) {
                self.notifications.launches.push(kept.application);
            }
            return;
        };
        let input = self.respond(client, window, response);
        if let Some(window) = window.filter(|_| !background) {
            self.last_user_serial = input;
            self.show_window(window);
        }
    }

    /// Gives `client` a response: the notification's texts, the chosen action's id and the reply,
    /// then the response with a serial that is a user input of the client's, as a key press's is.
    fn respond(&mut self, client: ClientId, window: Option<SurfaceKey>, response: Response) -> u64 {
        let number = response.number;
        let content = &response.content;
        for (field, text) in [
            (NotificationField::Identifier, &content.identifier),
            (NotificationField::Title, &content.title),
            (NotificationField::Subtitle, &content.subtitle),
            (NotificationField::Body, &content.body),
            (NotificationField::Data, &content.data),
        ] {
            if !text.is_empty() {
                self.emit_text(client, number, field, 0, text);
            }
        }
        if let Some(index) = response.action.checked_sub(1) {
            let id = content.actions[usize::from(index)].id.clone();
            self.emit_text(
                client,
                number,
                NotificationField::ActionId,
                response.action,
                &id,
            );
        }
        if !response.reply.is_empty() {
            self.emit_text(client, number, NotificationField::Reply, 0, &response.reply);
        }
        let input = self.next_serial();
        match window {
            Some(window) => self.record_press(input, window, false),
            None => {
                let inputs = self.inputs.entry(client).or_default();
                inputs.push_back(input);
                if inputs.len() > super::RECENT_INPUTS {
                    inputs.pop_front();
                }
                self.activation.insert(client, input);
            }
        }
        self.emit(
            client,
            Event::NotificationResponse {
                notification: number,
                action: response.action,
                serial: input,
            },
        );
        input
    }

    /// The applications to start because a notification of theirs was activated while none of
    /// their clients ran, since the last call: the program running the compositor starts each
    /// (its registry's), and the first client its launch record names as it is given the
    /// responses. One that cannot be started is [`Scene::notification_launch_failed`].
    pub fn take_notification_launches(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notifications.launches)
    }

    /// The application could not be started: its responses go.
    pub fn notification_launch_failed(&mut self, application: &str) {
        self.notifications.pending.remove(application);
    }

    /// `client` is `application`'s (its launch record): the responses waiting for it are its.
    pub(super) fn deliver_pending_responses(&mut self, client: ClientId, application: &str) {
        for response in self
            .notifications
            .pending
            .remove(application)
            .unwrap_or_default()
        {
            self.respond(client, None, response);
        }
    }

    pub(super) fn forget_notification_client(&mut self, client: ClientId) {
        let notifications = &mut self.notifications;
        notifications
            .receiving
            .retain(|(poster, _, _, _), _| *poster != client);
        notifications
            .posts
            .retain(|(poster, _), _| *poster != client);
        notifications
            .replies
            .retain(|(shell, _), _| *shell != client);
        notifications.told_policy.remove(&client);
    }
}
