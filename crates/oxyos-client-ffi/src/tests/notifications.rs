//! Notifications' C ABI (ADR 0026): the constants and the structs' layouts against the header,
//! events field by field, and a notification posted and activated through a socket.

use super::*;
use oxyos_protocol::{NotificationField, NotificationOutcome};

#[test]
fn the_header_matches_the_notifications_abi() {
    for (name, value) in [
        (
            "OXYOS_EVENT_NOTIFICATION_RESULT",
            OXYOS_EVENT_NOTIFICATION_RESULT,
        ),
        (
            "OXYOS_EVENT_NOTIFICATION_TEXT",
            OXYOS_EVENT_NOTIFICATION_TEXT,
        ),
        ("OXYOS_EVENT_NOTIFICATION", OXYOS_EVENT_NOTIFICATION),
        (
            "OXYOS_EVENT_NOTIFICATION_RESPONSE",
            OXYOS_EVENT_NOTIFICATION_RESPONSE,
        ),
        (
            "OXYOS_EVENT_NOTIFICATION_REMOVED",
            OXYOS_EVENT_NOTIFICATION_REMOVED,
        ),
        (
            "OXYOS_EVENT_NOTIFICATION_POLICY",
            OXYOS_EVENT_NOTIFICATION_POLICY,
        ),
        ("OXYOS_NOTIFICATION_POSTED", OXYOS_NOTIFICATION_POSTED),
        ("OXYOS_NOTIFICATION_REFUSED", OXYOS_NOTIFICATION_REFUSED),
        ("OXYOS_NOTIFICATION_THROTTLED", OXYOS_NOTIFICATION_THROTTLED),
        (
            "OXYOS_NOTIFICATION_FIELD_IDENTIFIER",
            OXYOS_NOTIFICATION_FIELD_IDENTIFIER,
        ),
        (
            "OXYOS_NOTIFICATION_FIELD_TITLE",
            OXYOS_NOTIFICATION_FIELD_TITLE,
        ),
        (
            "OXYOS_NOTIFICATION_FIELD_SUBTITLE",
            OXYOS_NOTIFICATION_FIELD_SUBTITLE,
        ),
        (
            "OXYOS_NOTIFICATION_FIELD_BODY",
            OXYOS_NOTIFICATION_FIELD_BODY,
        ),
        (
            "OXYOS_NOTIFICATION_FIELD_DATA",
            OXYOS_NOTIFICATION_FIELD_DATA,
        ),
        (
            "OXYOS_NOTIFICATION_FIELD_ACTION_ID",
            OXYOS_NOTIFICATION_FIELD_ACTION_ID,
        ),
        (
            "OXYOS_NOTIFICATION_FIELD_ACTION_LABEL",
            OXYOS_NOTIFICATION_FIELD_ACTION_LABEL,
        ),
        (
            "OXYOS_NOTIFICATION_FIELD_ACTION_REPLY",
            OXYOS_NOTIFICATION_FIELD_ACTION_REPLY,
        ),
        (
            "OXYOS_NOTIFICATION_FIELD_APPLICATION",
            OXYOS_NOTIFICATION_FIELD_APPLICATION,
        ),
        (
            "OXYOS_NOTIFICATION_FIELD_REPLY",
            OXYOS_NOTIFICATION_FIELD_REPLY,
        ),
        (
            "OXYOS_NOTIFICATION_BANNER",
            OXYOS_NOTIFICATION_BANNER.into(),
        ),
        (
            "OXYOS_NOTIFICATION_WITHHELD",
            OXYOS_NOTIFICATION_WITHHELD.into(),
        ),
        (
            "OXYOS_NOTIFICATIONS_DO_NOT_DISTURB",
            OXYOS_NOTIFICATIONS_DO_NOT_DISTURB.into(),
        ),
        (
            "OXYOS_NOTIFICATIONS_PREVIEWS",
            OXYOS_NOTIFICATIONS_PREVIEWS.into(),
        ),
    ] {
        assert_eq!(define(name), i64::from(value), "{name}");
    }
    // The protocol's values, every one.
    for (field, constant) in [
        (
            NotificationField::Identifier,
            OXYOS_NOTIFICATION_FIELD_IDENTIFIER,
        ),
        (NotificationField::Title, OXYOS_NOTIFICATION_FIELD_TITLE),
        (
            NotificationField::Subtitle,
            OXYOS_NOTIFICATION_FIELD_SUBTITLE,
        ),
        (NotificationField::Body, OXYOS_NOTIFICATION_FIELD_BODY),
        (NotificationField::Data, OXYOS_NOTIFICATION_FIELD_DATA),
        (
            NotificationField::ActionId,
            OXYOS_NOTIFICATION_FIELD_ACTION_ID,
        ),
        (
            NotificationField::ActionLabel,
            OXYOS_NOTIFICATION_FIELD_ACTION_LABEL,
        ),
        (
            NotificationField::ActionReply,
            OXYOS_NOTIFICATION_FIELD_ACTION_REPLY,
        ),
        (
            NotificationField::Application,
            OXYOS_NOTIFICATION_FIELD_APPLICATION,
        ),
        (NotificationField::Reply, OXYOS_NOTIFICATION_FIELD_REPLY),
    ] {
        assert_eq!(field as u32, constant, "{field:?}");
    }
    for (outcome, constant) in [
        (NotificationOutcome::Posted, OXYOS_NOTIFICATION_POSTED),
        (NotificationOutcome::Refused, OXYOS_NOTIFICATION_REFUSED),
        (NotificationOutcome::Throttled, OXYOS_NOTIFICATION_THROTTLED),
    ] {
        assert_eq!(outcome as u32, constant, "{outcome:?}");
    }
    let action = fields("OxyOSNotificationAction");
    assert_eq!(names(&action), ["id", "label", "reply", "background"]);
    let rust = [
        offset_of!(OxyOSNotificationAction, id),
        offset_of!(OxyOSNotificationAction, label),
        offset_of!(OxyOSNotificationAction, reply),
        offset_of!(OxyOSNotificationAction, background),
    ];
    assert_eq!(
        c_offsets(&action),
        (rust.to_vec(), size_of::<OxyOSNotificationAction>())
    );
    let notification = fields("OxyOSNotification");
    assert_eq!(
        names(&notification),
        [
            "identifier",
            "title",
            "subtitle",
            "body",
            "data",
            "actions",
            "action_count",
            "banner"
        ]
    );
    let rust = [
        offset_of!(OxyOSNotification, identifier),
        offset_of!(OxyOSNotification, title),
        offset_of!(OxyOSNotification, subtitle),
        offset_of!(OxyOSNotification, body),
        offset_of!(OxyOSNotification, data),
        offset_of!(OxyOSNotification, actions),
        offset_of!(OxyOSNotification, action_count),
        offset_of!(OxyOSNotification, banner),
    ];
    assert_eq!(
        c_offsets(&notification),
        (rust.to_vec(), size_of::<OxyOSNotification>())
    );
    for name in [
        "oxyos_client_post_notification",
        "oxyos_client_withdraw_notification",
        "oxyos_client_activate_notification",
        "oxyos_client_set_do_not_disturb",
    ] {
        assert!(HEADER.contains(&format!(" {name}(")), "{name} is declared");
    }
}

#[test]
fn notification_events_convert_field_by_field() {
    let result = convert(&Event::NotificationResult {
        serial: 4,
        notification: 9,
        outcome: NotificationOutcome::Throttled,
    })
    .unwrap();
    assert_eq!(
        (result.kind, result.code, result.detail, result.width),
        (
            OXYOS_EVENT_NOTIFICATION_RESULT,
            9,
            OXYOS_NOTIFICATION_THROTTLED,
            4
        )
    );
    let text = convert(&Event::NotificationText {
        notification: 9,
        field: NotificationField::ActionLabel,
        action: 2,
        index: 1,
        last: true,
        text: PreeditText::new("Responder").unwrap(),
    })
    .unwrap();
    assert_eq!(
        (
            text.kind,
            text.code,
            text.height,
            text.width,
            text.detail,
            text.flag
        ),
        (
            OXYOS_EVENT_NOTIFICATION_TEXT,
            9,
            OXYOS_NOTIFICATION_FIELD_ACTION_LABEL,
            2,
            1,
            1
        )
    );
    assert_eq!(&text.text[..10], b"Responder\0");
    let shown = convert(&Event::Notification {
        notification: 9,
        banner: true,
        actions: 2,
        age_s: 61,
        withheld: true,
    })
    .unwrap();
    assert_eq!(
        (
            shown.kind,
            shown.code,
            shown.flag,
            shown.width,
            shown.detail
        ),
        (
            OXYOS_EVENT_NOTIFICATION,
            9,
            OXYOS_NOTIFICATION_BANNER | OXYOS_NOTIFICATION_WITHHELD,
            2,
            61
        )
    );
    let response = convert(&Event::NotificationResponse {
        notification: 9,
        action: 1,
        serial: 43,
    })
    .unwrap();
    assert_eq!(
        (
            response.kind,
            response.code,
            response.width,
            response.serial
        ),
        (OXYOS_EVENT_NOTIFICATION_RESPONSE, 9, 1, 43)
    );
    let removed = convert(&Event::NotificationRemoved { notification: 9 }).unwrap();
    assert_eq!(
        (removed.kind, removed.code),
        (OXYOS_EVENT_NOTIFICATION_REMOVED, 9)
    );
    let policy = convert(&Event::NotificationPolicy {
        do_not_disturb: false,
        previews: true,
    })
    .unwrap();
    assert_eq!(
        (policy.kind, policy.flag),
        (
            OXYOS_EVENT_NOTIFICATION_POLICY,
            OXYOS_NOTIFICATIONS_PREVIEWS
        )
    );
}

#[test]
fn a_notification_posted_and_activated_through_the_c_abi() {
    let directory = std::env::temp_dir().join(format!(
        "oxyos-client-ffi-notifications-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        let mut requests = Vec::new();
        loop {
            let request = server.recv_request().unwrap().0;
            let done = matches!(request, Request::ActivateNotification { .. });
            requests.push(request);
            if done {
                break;
            }
        }
        server
            .send_event(
                &Event::NotificationResponse {
                    notification: 9,
                    action: 1,
                    serial: 43,
                },
                None,
            )
            .unwrap();
        requests
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let strings: Vec<CString> = [
        "chat-7",
        "Ana",
        "¿Comemos?",
        r#"{"chat":7}"#,
        "reply",
        "Responder",
        "Mensaje",
        "read",
        "Leído",
    ]
    .iter()
    .map(|text| CString::new(*text).unwrap())
    .collect();
    let at = |index: usize| strings[index].as_ptr();
    let actions = [
        OxyOSNotificationAction {
            id: at(4),
            label: at(5),
            reply: at(6),
            background: true,
        },
        OxyOSNotificationAction {
            id: at(7),
            label: at(8),
            reply: std::ptr::null(),
            background: false,
        },
    ];
    let notification = OxyOSNotification {
        identifier: at(0),
        title: at(1),
        subtitle: std::ptr::null(),
        body: at(2),
        data: at(3),
        actions: actions.as_ptr(),
        action_count: actions.len(),
        banner: true,
    };
    let reply = "Voy";
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; the
    // notification, its actions and their strings outlive the calls.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        // An action without a label, or four actions, send nothing.
        let unlabelled = [OxyOSNotificationAction {
            label: std::ptr::null(),
            ..actions[1]
        }];
        let broken = OxyOSNotification {
            actions: unlabelled.as_ptr(),
            action_count: 1,
            ..notification
        };
        assert_eq!(
            oxyos_client_post_notification(client, 1, &broken),
            OXYOS_ERROR
        );
        let four = [actions[1]; 4];
        let many = OxyOSNotification {
            actions: four.as_ptr(),
            action_count: 4,
            ..notification
        };
        assert_eq!(
            oxyos_client_post_notification(client, 1, &many),
            OXYOS_ERROR
        );
        assert_eq!(oxyos_client_post_notification(client, 1, &notification), 0);
        assert_eq!(oxyos_client_withdraw_notification(client, 0), 0);
        assert_eq!(oxyos_client_set_do_not_disturb(client, 6, true), 0);
        assert_eq!(
            oxyos_client_activate_notification(
                client,
                9,
                1,
                42,
                reply.as_ptr().cast(),
                reply.len()
            ),
            0
        );
        let requests = compositor.join().unwrap();
        let mut event = OxyOSEvent::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while oxyos_client_poll(client, &mut event) != OXYOS_POLL_EVENT {
            assert!(std::time::Instant::now() < deadline, "the response arrives");
        }
        assert_eq!(
            (event.kind, event.code, event.width, event.serial),
            (OXYOS_EVENT_NOTIFICATION_RESPONSE, 9, 1, 43)
        );
        oxyos_client_close(client);
        // Each text, as sent: by field and action, whole.
        let mut texts: std::collections::BTreeMap<(u8, u8), String> = Default::default();
        for request in &requests {
            if let Request::NotificationText {
                serial: 1,
                field,
                action,
                text,
                ..
            } = request
            {
                texts
                    .entry((*field as u8, *action))
                    .or_default()
                    .push_str(text.as_str());
            }
        }
        let text = |field: NotificationField, action: u8| {
            texts.get(&(field as u8, action)).map(String::as_str)
        };
        assert_eq!(text(NotificationField::Identifier, 0), Some("chat-7"));
        assert_eq!(text(NotificationField::Title, 0), Some("Ana"));
        assert_eq!(text(NotificationField::Subtitle, 0), None);
        assert_eq!(text(NotificationField::Body, 0), Some("¿Comemos?"));
        assert_eq!(text(NotificationField::Data, 0), Some(r#"{"chat":7}"#));
        assert_eq!(text(NotificationField::ActionId, 1), Some("reply"));
        assert_eq!(text(NotificationField::ActionReply, 1), Some("Mensaje"));
        assert_eq!(text(NotificationField::ActionLabel, 2), Some("Leído"));
        assert_eq!(text(NotificationField::ActionReply, 2), None);
        let tail: Vec<Request> = requests
            .iter()
            .filter(|request| !matches!(request, Request::NotificationText { .. }))
            .copied()
            .collect();
        assert_eq!(
            tail,
            [
                Request::PostNotification {
                    serial: 1,
                    banner: true,
                    background: 0b01
                },
                Request::WithdrawNotification { notification: 0 },
                Request::SetDoNotDisturb {
                    serial: 6,
                    enabled: true
                },
                Request::NotificationReply {
                    notification: 9,
                    index: 0,
                    last: true,
                    text: PreeditText::new("Voy").unwrap()
                },
                Request::ActivateNotification {
                    notification: 9,
                    action: 1,
                    serial: 42
                },
            ]
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}
