//! Login items and background activity (docs/adr/0060-login-items-and-background-activity.mdx):
//! any client asks about its background items, each ask handed to the program, which alone
//! decides; its answer reaches the asker alone, and only one that speaks version 32; one list of
//! login items, the dock's and Settings', changed by either and told to both.

use super::*;
use crate::scene::{ApplicationList, BackgroundItemRequest};
use oxyos_protocol::{
    ApplicationListKind, BackgroundItemAction, BackgroundItemKind, BackgroundStatus,
};

fn states(events: &[Event]) -> Vec<(u32, BackgroundItemKind, BackgroundStatus)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::BackgroundItemState {
                serial,
                kind,
                status,
            } => Some((*serial, *kind, *status)),
            _ => None,
        })
        .collect()
}

#[test]
fn any_client_asks_the_program_decides_and_the_answer_reaches_it_alone() {
    let mut h = Harness::new();
    h.request(
        APP,
        Request::BackgroundItem {
            serial: 1,
            kind: BackgroundItemKind::LoginItem,
            action: BackgroundItemAction::Register,
        },
    )
    .unwrap();
    h.request(
        OTHER,
        Request::BackgroundItem {
            serial: 4,
            kind: BackgroundItemKind::Background,
            action: BackgroundItemAction::Status,
        },
    )
    .unwrap();
    // The scene answers nothing itself: every ask is the program's, in order.
    assert!(states(&to(&logged(&mut h), APP)).is_empty());
    assert_eq!(
        h.scene.take_background_item_requests(),
        [
            BackgroundItemRequest {
                client: APP,
                serial: 1,
                kind: BackgroundItemKind::LoginItem,
                action: BackgroundItemAction::Register,
            },
            BackgroundItemRequest {
                client: OTHER,
                serial: 4,
                kind: BackgroundItemKind::Background,
                action: BackgroundItemAction::Status,
            },
        ]
    );
    assert!(h.scene.take_background_item_requests().is_empty());
    // Its answer, and a change unasked, to the asker alone.
    h.scene.background_item_state(
        APP,
        1,
        BackgroundItemKind::LoginItem,
        BackgroundStatus::Enabled,
    );
    h.scene.background_item_state(
        APP,
        0,
        BackgroundItemKind::Background,
        BackgroundStatus::RequiresApproval,
    );
    let events = logged(&mut h);
    assert_eq!(
        states(&to(&events, APP)),
        [
            (1, BackgroundItemKind::LoginItem, BackgroundStatus::Enabled),
            (
                0,
                BackgroundItemKind::Background,
                BackgroundStatus::RequiresApproval
            ),
        ]
    );
    assert!(states(&to(&events, OTHER)).is_empty());
    assert!(states(&to(&events, SHELL)).is_empty());
}

#[test]
fn a_client_of_an_older_version_is_never_told_and_one_gone_is_forgotten() {
    let mut h = Harness::new();
    h.scene.set_version(APP, 31);
    h.scene.background_item_state(
        APP,
        0,
        BackgroundItemKind::LoginItem,
        BackgroundStatus::NotRegistered,
    );
    assert!(states(&to(&logged(&mut h), APP)).is_empty());
    // An ask of a client that went is not handed to the program, and nothing is told it.
    h.request(
        OTHER,
        Request::BackgroundItem {
            serial: 2,
            kind: BackgroundItemKind::Background,
            action: BackgroundItemAction::Register,
        },
    )
    .unwrap();
    h.scene.disconnect(OTHER);
    assert!(h.scene.take_background_item_requests().is_empty());
    h.scene.background_item_state(
        OTHER,
        2,
        BackgroundItemKind::Background,
        BackgroundStatus::Enabled,
    );
    assert!(to(&logged(&mut h), OTHER).is_empty());
    // A client asking past what the program has yet to take is refused.
    for serial in 0..32 {
        h.request(
            APP,
            Request::BackgroundItem {
                serial,
                kind: BackgroundItemKind::LoginItem,
                action: BackgroundItemAction::Status,
            },
        )
        .unwrap();
    }
    assert_eq!(
        h.request(
            APP,
            Request::BackgroundItem {
                serial: 32,
                kind: BackgroundItemKind::LoginItem,
                action: BackgroundItemAction::Status,
            },
        ),
        Err(ErrorCode::QuotaExceeded)
    );
    assert_eq!(h.scene.take_background_item_requests().len(), 32);
}

fn set_list(
    h: &mut Harness,
    client: ClientId,
    list: ApplicationListKind,
    text: &str,
) -> Result<(), ErrorCode> {
    h.request(
        client,
        Request::SetApplicationList {
            serial: 7,
            list,
            index: 0,
            last: true,
            text: PreeditText::new(text).unwrap(),
        },
    )
}

fn lists_told(events: &[Event]) -> Vec<(ApplicationListKind, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::ApplicationList { list, text, .. } => Some((*list, text.as_str().to_string())),
            _ => None,
        })
        .collect()
}

#[test]
fn open_at_login_is_one_list_the_dock_and_settings_change_and_both_are_told() {
    let mut h = Harness::new();
    h.scene
        .connect(
            SETTINGS,
            Grants {
                settings: true,
                ..Grants::default()
            },
        )
        .unwrap();
    // Settings changes Open at Login, as the shell does; never the dock, nor does an application.
    set_list(
        &mut h,
        SETTINGS,
        ApplicationListKind::OpenAtLogin,
        "so.oxy.allo",
    )
    .unwrap();
    set_list(
        &mut h,
        SHELL,
        ApplicationListKind::OpenAtLogin,
        "so.oxy.inbox",
    )
    .unwrap();
    let changes = h.scene.take_application_list_changes();
    assert_eq!(
        changes
            .iter()
            .map(|change| (change.client, change.list, change.text.as_str()))
            .collect::<Vec<_>>(),
        [
            (SETTINGS, ApplicationListKind::OpenAtLogin, "so.oxy.allo"),
            (SHELL, ApplicationListKind::OpenAtLogin, "so.oxy.inbox"),
        ]
    );
    assert_eq!(
        set_list(&mut h, SETTINGS, ApplicationListKind::Dock, "so.oxy.allo"),
        Err(ErrorCode::Unauthorized)
    );
    assert_eq!(
        set_list(&mut h, APP, ApplicationListKind::OpenAtLogin, "so.oxy.allo"),
        Err(ErrorCode::Unauthorized)
    );
    // What the program keeps is told to the shell and to Settings, once each; the dock to the
    // shell alone; no application is told either.
    logged(&mut h);
    h.scene.set_application_list(
        ApplicationListKind::OpenAtLogin,
        ApplicationList {
            chosen: true,
            ids: vec!["so.oxy.inbox".into()],
        },
    );
    h.scene.set_application_list(
        ApplicationListKind::Dock,
        ApplicationList {
            chosen: true,
            ids: vec!["so.oxy.dropsea".into()],
        },
    );
    let events = logged(&mut h);
    let open_at_login = (ApplicationListKind::OpenAtLogin, "so.oxy.inbox".to_string());
    let dock = (ApplicationListKind::Dock, "so.oxy.dropsea".to_string());
    assert_eq!(
        lists_told(&to(&events, SHELL)),
        [open_at_login.clone(), dock]
    );
    assert_eq!(lists_told(&to(&events, SETTINGS)), [open_at_login.clone()]);
    assert!(lists_told(&to(&events, APP)).is_empty());
    // A Settings that connects later is told it.
    h.scene
        .connect(
            10,
            Grants {
                settings: true,
                ..Grants::default()
            },
        )
        .unwrap();
    assert_eq!(lists_told(&to(&logged(&mut h), 10)), [open_at_login]);
}
