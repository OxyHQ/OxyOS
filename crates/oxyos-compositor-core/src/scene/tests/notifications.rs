//! Notifications (ADR 0026): who posts (the launch record, never a claim), what the session keeps
//! and tells the shell, the rate, Do Not Disturb, the lock screen's privacy, and how a press on
//! one reaches its application, once.

use super::*;
use oxyos_protocol::{NotificationField, NotificationOutcome, MAX_NOTIFICATION_ACTIONS};
use std::time::Duration;

const ALLO: &str = "so.oxy.allo";
const INBOX: &str = "so.oxy.inbox";

/// A notification as an application posts it.
#[derive(Default)]
struct Post<'a> {
    identifier: &'a str,
    title: &'a str,
    body: &'a str,
    data: &'a str,
    /// (id, label, reply placeholder)
    actions: &'a [(&'a str, &'a str, Option<&'a str>)],
    banner: bool,
    background: u8,
}

fn text(
    h: &mut Harness,
    client: ClientId,
    serial: u32,
    field: NotificationField,
    action: u8,
    value: &str,
) {
    let chunks = TextChunk::split(value);
    let count = chunks.len().max(1);
    for index in 0..count {
        let chunk = chunks.get(index).map_or("", TextChunk::as_str);
        h.request(
            client,
            Request::NotificationText {
                serial,
                field,
                action,
                index: index as u8,
                last: index + 1 == count,
                text: PreeditText::new(chunk).unwrap(),
            },
        )
        .unwrap();
    }
}

/// Posts and returns what the session answered: its number for it and the outcome.
fn post(h: &mut Harness, client: ClientId, serial: u32, post: &Post) -> (u32, NotificationOutcome) {
    for (field, value) in [
        (NotificationField::Identifier, post.identifier),
        (NotificationField::Title, post.title),
        (NotificationField::Body, post.body),
        (NotificationField::Data, post.data),
    ] {
        if !value.is_empty() {
            text(h, client, serial, field, 0, value);
        }
    }
    for (index, (id, label, reply)) in post.actions.iter().enumerate() {
        let n = index as u8 + 1;
        text(h, client, serial, NotificationField::ActionId, n, id);
        text(h, client, serial, NotificationField::ActionLabel, n, label);
        if let Some(placeholder) = reply {
            text(
                h,
                client,
                serial,
                NotificationField::ActionReply,
                n,
                placeholder,
            );
        }
    }
    h.clear_log();
    h.request(
        client,
        Request::PostNotification {
            serial,
            banner: post.banner,
            background: post.background,
        },
    )
    .unwrap();
    to(&h.log, client)
        .into_iter()
        .find_map(|event| match event {
            Event::NotificationResult {
                serial: answered,
                notification,
                outcome,
            } if answered == serial => Some((notification, outcome)),
            _ => None,
        })
        .expect("answered")
}

fn hello(h: &mut Harness, client: ClientId, serial: u32) -> (u32, NotificationOutcome) {
    post(
        h,
        client,
        serial,
        &Post {
            title: "Ana",
            body: "¿Comemos?",
            banner: true,
            ..Post::default()
        },
    )
}

/// What a shell (or an application) was told of notifications: each whole, by number, its texts
/// by field and action, and its `Notification` event if one came.
#[derive(Debug, Default, Clone, PartialEq)]
struct Told {
    texts: BTreeMap<(u8, u8), String>,
    shown: Option<Event>,
    response: Option<Event>,
    removed: bool,
}

fn told(events: &[Event]) -> BTreeMap<u32, Told> {
    let mut all: BTreeMap<u32, Told> = BTreeMap::new();
    for event in events {
        match *event {
            Event::NotificationText {
                notification,
                field,
                action,
                index,
                text,
                ..
            } => {
                let entry = all.entry(notification).or_default();
                let value = entry.texts.entry((field as u8, action)).or_default();
                if index == 0 {
                    value.clear();
                }
                value.push_str(text.as_str());
            }
            Event::Notification { notification, .. } => {
                all.entry(notification).or_default().shown = Some(*event);
            }
            Event::NotificationResponse { notification, .. } => {
                all.entry(notification).or_default().response = Some(*event);
            }
            Event::NotificationRemoved { notification } => {
                all.entry(notification).or_default().removed = true;
            }
            _ => {}
        }
    }
    all
}

fn field(told: &Told, field: NotificationField, action: u8) -> Option<&str> {
    told.texts.get(&(field as u8, action)).map(String::as_str)
}

fn banner(told: &Told) -> Option<bool> {
    match told.shown {
        Some(Event::Notification { banner, .. }) => Some(banner),
        _ => None,
    }
}

fn responses(events: &[Event]) -> Vec<(u32, u8)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::NotificationResponse {
                notification,
                action,
                ..
            } => Some((*notification, *action)),
            _ => None,
        })
        .collect()
}

/// A press on the shell's dock: an input the shell received, which its activation cites.
fn shell_press(h: &mut Harness) -> u64 {
    if !h.rendered(key(SHELL, 50)) {
        h.request(
            SHELL,
            Request::CreateShellSurface {
                surface: 50,
                kind: ShellKind::Panel,
                edge: PanelEdge::Bottom,
                output: 1,
                size: Size::new(1, 48),
                exclusive: 48,
            },
        )
        .unwrap();
        h.present(key(SHELL, 50));
    }
    let serial = h.press(960, 1060);
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    serial
}

fn activate(h: &mut Harness, notification: u32, action: u8, serial: u64) {
    h.clear_log();
    h.request(
        SHELL,
        Request::ActivateNotification {
            notification,
            action,
            serial,
        },
    )
    .unwrap();
}

fn reply(h: &mut Harness, notification: u32, value: &str) {
    h.request(
        SHELL,
        Request::NotificationReply {
            notification,
            index: 0,
            last: true,
            text: PreeditText::new(value).unwrap(),
        },
    )
    .unwrap();
}

#[test]
fn the_posting_application_is_its_launch_record_never_its_claim() {
    let mut h = Harness::new();
    // A client the session did not start claims to be Allo: it posts nothing, and the shell is
    // told nothing.
    let window = h.toplevel(APP, 1, 400, 300);
    h.request(
        APP,
        Request::SetAppId {
            surface: window.id,
            index: 0,
            last: true,
            text: PreeditText::new(ALLO).unwrap(),
        },
    )
    .unwrap();
    assert_eq!(hello(&mut h, APP, 1), (0, NotificationOutcome::Refused));
    assert!(told(&to(&h.log, SHELL)).is_empty());
    // Started by the session as Inbox, whatever it claims: its notification is Inbox's.
    h.scene.set_application(APP, INBOX);
    let (number, outcome) = hello(&mut h, APP, 2);
    assert_eq!(outcome, NotificationOutcome::Posted);
    assert_ne!(number, 0);
    let shell = told(&to(&h.log, SHELL));
    let notification = &shell[&number];
    assert_eq!(
        field(notification, NotificationField::Application, 0),
        Some(INBOX)
    );
    assert_eq!(
        field(notification, NotificationField::Title, 0),
        Some("Ana")
    );
    assert_eq!(
        field(notification, NotificationField::Body, 0),
        Some("¿Comemos?")
    );
    assert_eq!(banner(notification), Some(true));
    // Applications are never told another's notifications, nor the application's own texts.
    assert!(told(&to(&h.log, OTHER)).is_empty());
    assert!(told(&to(&h.log, APP)).is_empty());
    // What the session says is never the application's to send.
    assert_eq!(
        h.request(
            APP,
            Request::NotificationText {
                serial: 3,
                field: NotificationField::Application,
                action: 0,
                index: 0,
                last: true,
                text: PreeditText::new(ALLO).unwrap(),
            },
        ),
        Err(ErrorCode::InvalidMessage)
    );
}

#[test]
fn a_shell_that_connects_is_told_every_notification_kept_and_none_as_a_banner() {
    let mut h = Harness::new();
    h.scene.set_application(APP, ALLO);
    let (first, _) = hello(&mut h, APP, 1);
    let (second, _) = hello(&mut h, APP, 2);
    h.scene.disconnect(SHELL);
    h.clear_log();
    h.scene
        .connect(
            SHELL,
            Grants {
                shell: true,
                ..Grants::default()
            },
        )
        .unwrap();
    h.pump();
    let shell = told(&to(&h.log, SHELL));
    assert_eq!(shell.keys().copied().collect::<Vec<_>>(), [first, second]);
    assert!(shell.values().all(|told| banner(told) == Some(false)));
    assert!(to(&h.log, SHELL).contains(&Event::NotificationPolicy {
        do_not_disturb: false,
        previews: false
    }));
    // Kept across the application's going: they are its application's, not its process's.
    h.scene.disconnect(APP);
    h.scene.disconnect(SHELL);
    h.clear_log();
    h.scene
        .connect(
            SHELL,
            Grants {
                shell: true,
                ..Grants::default()
            },
        )
        .unwrap();
    h.pump();
    assert_eq!(told(&to(&h.log, SHELL)).len(), 2);
}

#[test]
fn an_application_posts_five_in_ten_seconds_and_is_throttled_beyond() {
    let mut h = Harness::new();
    h.scene.set_application(APP, ALLO);
    h.scene.set_application(OTHER, INBOX);
    h.scene.set_clock(Duration::from_secs(100));
    assert_eq!(hello(&mut h, APP, 1).1, NotificationOutcome::Posted);
    h.scene.set_clock(Duration::from_secs(105));
    for serial in 2..=5 {
        assert_eq!(hello(&mut h, APP, serial).1, NotificationOutcome::Posted);
    }
    assert_eq!(hello(&mut h, APP, 6), (0, NotificationOutcome::Throttled));
    assert!(
        told(&to(&h.log, SHELL)).is_empty(),
        "the shell is told nothing of it"
    );
    // Another application is its own.
    assert_eq!(hello(&mut h, OTHER, 1).1, NotificationOutcome::Posted);
    // Within the ten seconds, still throttled; once the first is ten seconds old, one more.
    h.scene.set_clock(Duration::from_millis(109_999));
    assert_eq!(hello(&mut h, APP, 7).1, NotificationOutcome::Throttled);
    h.scene.set_clock(Duration::from_secs(110));
    assert_eq!(hello(&mut h, APP, 8).1, NotificationOutcome::Posted);
    assert_eq!(hello(&mut h, APP, 9).1, NotificationOutcome::Throttled);
    // A clock told backwards changes nothing.
    h.scene.set_clock(Duration::from_secs(1));
    assert_eq!(hello(&mut h, APP, 10).1, NotificationOutcome::Throttled);
}

#[test]
fn the_session_keeps_at_most_thirty_two_an_application_and_replaces_by_identifier() {
    let mut h = Harness::new();
    h.scene.set_application(APP, ALLO);
    let mut numbers = Vec::new();
    for serial in 0..33 {
        // Ten seconds apart, so none is throttled.
        h.scene
            .set_clock(Duration::from_secs(10 * u64::from(serial)));
        numbers.push(hello(&mut h, APP, serial).0);
    }
    // The 33rd took the first's place: the shell and the application are told it went.
    assert!(told(&to(&h.log, SHELL))[&numbers[0]].removed);
    assert!(told(&to(&h.log, APP))[&numbers[0]].removed);
    // The same identifier again replaces it.
    let chat = Post {
        identifier: "chat-1",
        title: "Ana",
        body: "uno",
        banner: true,
        ..Post::default()
    };
    h.scene.set_clock(Duration::from_secs(1000));
    let (one, _) = post(&mut h, APP, 100, &chat);
    h.scene.set_clock(Duration::from_secs(1010));
    let (two, _) = post(
        &mut h,
        APP,
        101,
        &Post {
            body: "dos",
            ..chat
        },
    );
    let shell = told(&to(&h.log, SHELL));
    assert!(shell[&one].removed);
    assert_eq!(field(&shell[&two], NotificationField::Body, 0), Some("dos"));
    // Texts and actions are bounded: a title past its limit closes nothing but is refused.
    let long = "a".repeat(oxyos_protocol::MAX_NOTIFICATION_TITLE_BYTES + 1);
    let chunks = TextChunk::split(&long);
    let mut result = Ok(());
    for (index, chunk) in chunks.iter().enumerate() {
        result = h.request(
            APP,
            Request::NotificationText {
                serial: 200,
                field: NotificationField::Title,
                action: 0,
                index: index as u8,
                last: index + 1 == chunks.len(),
                text: PreeditText::new(chunk.as_str()).unwrap(),
            },
        );
        if result.is_err() {
            break;
        }
    }
    assert_eq!(result, Err(ErrorCode::QuotaExceeded));
    assert_eq!(
        h.request(
            APP,
            Request::NotificationText {
                serial: 201,
                field: NotificationField::ActionLabel,
                action: MAX_NOTIFICATION_ACTIONS as u8 + 1,
                index: 0,
                last: true,
                text: PreeditText::new("Más").unwrap(),
            },
        ),
        Err(ErrorCode::InvalidMessage)
    );
}

#[test]
fn an_application_withdraws_its_own_and_the_shell_any() {
    let mut h = Harness::new();
    h.scene.set_application(APP, ALLO);
    h.scene.set_application(OTHER, INBOX);
    let (allo, _) = hello(&mut h, APP, 1);
    let (inbox, _) = hello(&mut h, OTHER, 1);
    h.clear_log();
    // Inbox cannot withdraw Allo's, even all at once.
    h.request(OTHER, Request::WithdrawNotification { notification: allo })
        .unwrap();
    h.request(OTHER, Request::WithdrawNotification { notification: 0 })
        .unwrap();
    let shell = told(&to(&h.log, SHELL));
    assert_eq!(shell.keys().copied().collect::<Vec<_>>(), [inbox]);
    assert!(shell[&inbox].removed);
    h.clear_log();
    h.request(SHELL, Request::WithdrawNotification { notification: 0 })
        .unwrap();
    assert!(told(&to(&h.log, SHELL))[&allo].removed);
    assert!(told(&to(&h.log, APP))[&allo].removed);
}

#[test]
fn do_not_disturb_keeps_every_notification_and_shows_no_banner() {
    let mut h = Harness::new();
    h.scene.set_application(APP, ALLO);
    h.scene.set_notification_policy(NotificationPolicy {
        do_not_disturb: true,
        previews: false,
    });
    h.pump();
    assert!(to(&h.log, SHELL).contains(&Event::NotificationPolicy {
        do_not_disturb: true,
        previews: false
    }));
    let (number, outcome) = hello(&mut h, APP, 1);
    assert_eq!(outcome, NotificationOutcome::Posted);
    let shell = told(&to(&h.log, SHELL));
    assert_eq!(banner(&shell[&number]), Some(false));
    assert_eq!(
        field(&shell[&number], NotificationField::Title, 0),
        Some("Ana")
    );
    // Off again: the next is a banner.
    h.scene
        .set_notification_policy(NotificationPolicy::default());
    let (next, _) = hello(&mut h, APP, 2);
    assert_eq!(banner(&told(&to(&h.log, SHELL))[&next]), Some(true));
    // The same policy again tells nothing.
    h.clear_log();
    h.scene
        .set_notification_policy(NotificationPolicy::default());
    h.pump();
    assert!(to(&h.log, SHELL).is_empty());
}

#[test]
fn the_locked_shell_is_told_only_whose_notifications_arrive_unless_previews_are_allowed() {
    let mut h = Harness::new();
    h.scene.set_application(APP, ALLO);
    // A press on the shell's dock before the lock: the shell received it.
    let serial = shell_press(&mut h);
    h.scene.lock();
    let (number, _) = post(
        &mut h,
        APP,
        1,
        &Post {
            title: "Ana",
            body: "la clave es 1234",
            actions: &[("reply", "Responder", Some("Mensaje"))],
            banner: true,
            ..Post::default()
        },
    );
    let shell = told(&to(&h.log, SHELL));
    let locked = &shell[&number];
    assert_eq!(locked.texts.len(), 1, "its application alone: {locked:?}");
    assert_eq!(field(locked, NotificationField::Application, 0), Some(ALLO));
    assert_eq!(
        locked.shown,
        Some(Event::Notification {
            notification: number,
            banner: false,
            actions: 0,
            age_s: 0,
            withheld: true
        })
    );
    // Nothing of it can be acted on while locked, from the lock screen or anywhere.
    activate(&mut h, number, 0, serial);
    assert!(responses(&to(&h.log, APP)).is_empty());
    // Unlocked, the shell is told what it says, as no banner.
    h.clear_log();
    h.scene.unlock();
    h.pump();
    let unlocked = &told(&to(&h.log, SHELL))[&number];
    assert_eq!(
        field(unlocked, NotificationField::Body, 0),
        Some("la clave es 1234")
    );
    assert_eq!(
        field(unlocked, NotificationField::ActionReply, 1),
        Some("Mensaje")
    );
    assert_eq!(banner(unlocked), Some(false));
    // With previews allowed, the locked shell is told what it says, still as no banner.
    h.scene.set_notification_policy(NotificationPolicy {
        do_not_disturb: false,
        previews: true,
    });
    h.scene.lock();
    let (shown, _) = hello(&mut h, APP, 2);
    let previewed = &told(&to(&h.log, SHELL))[&shown];
    assert_eq!(field(previewed, NotificationField::Title, 0), Some("Ana"));
    assert_eq!(banner(previewed), Some(false));
    assert!(matches!(
        previewed.shown,
        Some(Event::Notification {
            withheld: false,
            ..
        })
    ));
}

#[test]
fn a_press_on_a_notification_brings_its_application_forward_and_answers_it_once() {
    let mut h = Harness::new();
    h.scene.set_application(APP, ALLO);
    let allo = h.toplevel(APP, 1, 400, 300);
    let other = h.toplevel(OTHER, 1, 400, 300);
    h.activate(allo);
    h.scene.minimize(allo);
    h.activate(other);
    let (number, _) = post(
        &mut h,
        APP,
        1,
        &Post {
            identifier: "chat-7",
            title: "Ana",
            body: "¿Comemos?",
            data: r#"{"chat":7}"#,
            actions: &[
                ("reply", "Responder", Some("Mensaje")),
                ("read", "Leído", None),
            ],
            banner: true,
            ..Post::default()
        },
    );
    // An input the shell never received acts on nothing; nor does the application's own.
    let at = h.rect(other);
    let theirs = h.press(at.x + 200, at.y + 150);
    h.scene.pointer_button(BUTTON, false);
    activate(&mut h, number, 0, theirs);
    activate(&mut h, number, 0, 12345);
    assert!(responses(&to(&h.log, APP)).is_empty());
    // An application cannot activate one.
    assert_eq!(
        h.request(
            APP,
            Request::ActivateNotification {
                notification: number,
                action: 0,
                serial: theirs
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
    // The shell's press: Allo's window is restored and active, and Allo is answered once, with
    // what it posted.
    let serial = shell_press(&mut h);
    reply(&mut h, number, "ignored without a reply action");
    activate(&mut h, number, 0, serial);
    let app = to(&h.log, APP);
    assert_eq!(responses(&app), [(number, 0)]);
    let answered = &told(&app)[&number];
    assert_eq!(
        field(answered, NotificationField::Identifier, 0),
        Some("chat-7")
    );
    assert_eq!(
        field(answered, NotificationField::Data, 0),
        Some(r#"{"chat":7}"#)
    );
    assert_eq!(field(answered, NotificationField::Title, 0), Some("Ana"));
    assert_eq!(field(answered, NotificationField::Reply, 0), None);
    assert!(answered.removed);
    assert_eq!(h.scene.active_window(), Some(allo));
    assert!(h.rendered(allo));
    // The response's serial is the application's input, as a key press is.
    let Some(Event::NotificationResponse { serial: input, .. }) = answered.response else {
        panic!("a response");
    };
    assert!(h.scene.recent_input(APP, input));
    assert!(told(&to(&h.log, SHELL))[&number].removed);
    // Gone: pressed again, nothing.
    let again = shell_press(&mut h);
    activate(&mut h, number, 0, again);
    assert!(responses(&to(&h.log, APP)).is_empty());
}

#[test]
fn a_reply_goes_with_its_action_and_a_background_action_leaves_focus_where_it_is() {
    let mut h = Harness::new();
    h.scene.set_application(APP, ALLO);
    let allo = h.toplevel(APP, 1, 400, 300);
    let other = h.toplevel(OTHER, 1, 400, 300);
    h.activate(other);
    let actions: &[(&str, &str, Option<&str>)] = &[
        ("reply", "Responder", Some("Mensaje")),
        ("read", "Leído", None),
    ];
    let (first, _) = post(
        &mut h,
        APP,
        1,
        &Post {
            title: "Ana",
            actions,
            background: 0b01,
            ..Post::default()
        },
    );
    let serial = shell_press(&mut h);
    reply(&mut h, first, "Voy");
    activate(&mut h, first, 1, serial);
    let app = to(&h.log, APP);
    assert_eq!(responses(&app), [(first, 1)]);
    let answered = &told(&app)[&first];
    assert_eq!(
        field(answered, NotificationField::ActionId, 1),
        Some("reply")
    );
    assert_eq!(field(answered, NotificationField::Reply, 0), Some("Voy"));
    // A background action: answered, and the other window keeps the focus.
    assert_eq!(h.scene.active_window(), Some(other));
    // An action that is no reply takes no reply, and brings the application forward.
    let (second, _) = post(
        &mut h,
        APP,
        2,
        &Post {
            title: "Ana",
            actions,
            background: 0b01,
            ..Post::default()
        },
    );
    let serial = shell_press(&mut h);
    reply(&mut h, second, "nada");
    activate(&mut h, second, 2, serial);
    let app = to(&h.log, APP);
    assert_eq!(responses(&app), [(second, 2)]);
    assert_eq!(
        field(&told(&app)[&second], NotificationField::ActionId, 2),
        Some("read")
    );
    assert_eq!(
        field(&told(&app)[&second], NotificationField::Reply, 0),
        None
    );
    assert_eq!(h.scene.active_window(), Some(allo));
    // An action it does not have is nothing.
    let (third, _) = hello(&mut h, APP, 3);
    let serial = shell_press(&mut h);
    activate(&mut h, third, 1, serial);
    assert!(responses(&to(&h.log, APP)).is_empty());
}

#[test]
fn an_application_none_of_whose_clients_runs_is_started_and_its_first_client_answered_once() {
    let mut h = Harness::new();
    h.scene.set_application(APP, ALLO);
    let (number, _) = hello(&mut h, APP, 1);
    h.scene.disconnect(APP);
    let serial = shell_press(&mut h);
    activate(&mut h, number, 0, serial);
    assert_eq!(h.scene.take_notification_launches(), [ALLO]);
    assert!(
        h.scene.take_notification_launches().is_empty(),
        "asked once"
    );
    // Another application's client is not it.
    h.scene.set_application(OTHER, INBOX);
    h.pump();
    assert!(responses(&to(&h.log, OTHER)).is_empty());
    // Allo's process connects, and its launch record names it: it is answered, once.
    h.scene.connect(APP, Grants::default()).unwrap();
    h.scene.set_application(APP, ALLO);
    h.pump();
    let app = to(&h.log, APP);
    assert_eq!(responses(&app), [(number, 0)]);
    assert_eq!(
        field(&told(&app)[&number], NotificationField::Title, 0),
        Some("Ana")
    );
    h.clear_log();
    h.scene.connect(9, Grants::default()).unwrap();
    h.scene.set_application(9, ALLO);
    h.pump();
    assert!(responses(&to(&h.log, 9)).is_empty());
    // One that cannot be started leaves nothing waiting.
    let (next, _) = hello(&mut h, APP, 2);
    h.scene.disconnect(APP);
    h.scene.disconnect(9);
    let serial = shell_press(&mut h);
    activate(&mut h, next, 0, serial);
    assert_eq!(h.scene.take_notification_launches(), [ALLO]);
    h.scene.notification_launch_failed(ALLO);
    h.scene.connect(APP, Grants::default()).unwrap();
    h.scene.set_application(APP, ALLO);
    h.pump();
    assert!(responses(&to(&h.log, APP)).is_empty());
}

#[test]
fn a_banner_surface_is_at_its_outputs_top_right_corner_above_every_window_and_panel() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    shell_press(&mut h);
    for edge in [PanelEdge::None, PanelEdge::Right, PanelEdge::Top] {
        h.request(
            SHELL,
            Request::CreateShellSurface {
                surface: 60,
                kind: ShellKind::Notification,
                edge,
                output: 1,
                size: Size::new(380, 120),
                exclusive: 48,
            },
        )
        .unwrap();
        h.present(key(SHELL, 60));
        assert_eq!(
            h.rect(key(SHELL, 60)),
            Rect::new(1920 - 380, 0, 380, 120),
            "{edge:?}"
        );
        let order: Vec<SurfaceKey> = h
            .scene
            .render_list()
            .into_iter()
            .filter_map(|item| match item {
                RenderItem::Surface { key, .. } => Some(key),
                _ => None,
            })
            .collect();
        assert_eq!(order, [window, key(SHELL, 50), key(SHELL, 60)]);
        // It keeps nothing out of where windows go.
        assert_eq!(
            h.scene.work_area(1).map(|area| area.height),
            Some(1080 - 48)
        );
        h.request(SHELL, Request::Destroy { surface: 60 }).unwrap();
    }
}

#[test]
fn do_not_disturb_from_the_control_center_goes_the_preferences_way() {
    let mut h = Harness::new();
    h.request(
        SHELL,
        Request::SetDoNotDisturb {
            serial: 7,
            enabled: true,
        },
    )
    .unwrap();
    // Only the session's shell: an application has no Control Center.
    assert_eq!(
        h.request(
            APP,
            Request::SetDoNotDisturb {
                serial: 1,
                enabled: true
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
    assert_eq!(
        h.scene.take_preference_changes(),
        [PreferenceChange {
            client: SHELL,
            serial: 7,
            key: "do_not_disturb".to_string(),
            value: "true".to_string(),
        }]
    );
    // Locked, it is refused at once and never handed over.
    h.scene.lock();
    logged(&mut h);
    h.request(
        SHELL,
        Request::SetDoNotDisturb {
            serial: 8,
            enabled: false,
        },
    )
    .unwrap();
    assert!(h.scene.take_preference_changes().is_empty());
    assert!(
        to(&logged(&mut h), SHELL).contains(&Event::PreferenceResult {
            serial: 8,
            accepted: false
        })
    );
}
