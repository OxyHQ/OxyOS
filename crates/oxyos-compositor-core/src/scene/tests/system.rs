//! Installing applications (ADR 0030): who may ask the session, an input cited and noted, sheets
//! for the shell alone, and only the shell's input confirming one.

use super::*;

/// A press on the shell's panel: an input the shell received.
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

/// A press in the application's window: an input the application received.
fn app_press(h: &mut Harness) -> u64 {
    if !h.rendered(key(APP, 1)) {
        h.toplevel(APP, 1, 400, 300);
    }
    let rect = h.rect(key(APP, 1));
    let serial = h.press(rect.x + 20, rect.y + 20);
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    serial
}

fn ask(
    h: &mut Harness,
    client: ClientId,
    serial: u32,
    input: u64,
    text: &str,
) -> Result<(), ErrorCode> {
    let chunks = TextChunk::split(text);
    let count = chunks.len();
    for (index, chunk) in chunks.iter().enumerate() {
        h.request(
            client,
            Request::SystemRequest {
                serial,
                input,
                index: index as u16,
                last: index + 1 == count,
                text: PreeditText::new(chunk.as_str()).unwrap(),
            },
        )?;
    }
    Ok(())
}

fn store() -> Grants {
    Grants {
        store: true,
        ..Grants::default()
    }
}

fn texts(events: &[Event]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            Event::SystemText { text, .. } | Event::SheetText { text, .. } => {
                Some(text.as_str().to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn only_the_store_updates_and_privacy_grants_ask_and_an_input_is_noted() {
    let mut h = Harness::new();
    let long = format!(r#"{{"install":{{"id":"so.oxy.{}"}}}}"#, "x".repeat(100));
    // Without a grant that asks: refused and said, nothing handed on.
    h.clear_log();
    ask(&mut h, APP, 1, 0, &long).unwrap();
    h.pump();
    assert!(h.scene.take_system_requests().is_empty());
    assert_eq!(
        texts(&h.events_for(APP)),
        r#"{"refused":{"reason":"notPermitted"}}"#
    );
    h.scene.grant(APP, store()).unwrap();
    let input = app_press(&mut h);
    ask(&mut h, APP, 2, input, &long).unwrap();
    ask(&mut h, APP, 3, 0, r#"{"state":{}}"#).unwrap();
    // The shell's input is not the application's to cite.
    let shells = shell_press(&mut h);
    ask(&mut h, APP, 4, shells, r#"{"check":{}}"#).unwrap();
    let asked = h.scene.take_system_requests();
    assert_eq!(
        asked
            .iter()
            .map(|r| (r.serial, r.with_input, r.text.as_str()))
            .collect::<Vec<_>>(),
        [
            (2, true, long.as_str()),
            (3, false, r#"{"state":{}}"#),
            (4, false, r#"{"check":{}}"#),
        ]
    );
    // An answer reaches its asker, whole.
    h.clear_log();
    let answer = format!(r#"{{"status":"{}"}}"#, "y".repeat(200));
    h.scene.system_text(APP, 2, &answer);
    h.pump();
    assert_eq!(texts(&h.events_for(APP)), answer);
    // Revoked, it asks no more and is told nothing.
    h.scene.revoke(APP, store());
    ask(&mut h, APP, 5, 0, "{}").unwrap();
    assert!(h.scene.take_system_requests().is_empty());
    h.clear_log();
    h.scene.system_text(APP, 2, &answer);
    h.pump();
    assert!(h.events_for(APP).is_empty());
}

#[test]
fn a_sheet_is_the_shells_alone_and_only_its_input_accepts_it() {
    let mut h = Harness::new();
    h.scene.grant(APP, store()).unwrap();
    h.clear_log();
    let sheet = h.scene.show_sheet(None, r#"{"kind":"install"}"#).unwrap();
    h.pump();
    assert_eq!(texts(&h.events_for(SHELL)), r#"{"kind":"install"}"#);
    assert!(
        h.events_for(APP).is_empty(),
        "the application never sees the sheet"
    );
    // Not the shell: refused.
    let input = app_press(&mut h);
    assert_eq!(
        h.request(
            APP,
            Request::SheetAnswer {
                sheet,
                accepted: true,
                serial: input
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
    // The shell citing an input it did not receive: ignored, the sheet still asked.
    h.request(
        SHELL,
        Request::SheetAnswer {
            sheet,
            accepted: true,
            serial: input,
        },
    )
    .unwrap();
    assert!(h.scene.take_sheet_answers().is_empty());
    let press = shell_press(&mut h);
    h.request(
        SHELL,
        Request::SheetAnswer {
            sheet,
            accepted: true,
            serial: press,
        },
    )
    .unwrap();
    assert_eq!(
        h.scene.take_sheet_answers(),
        [SheetAnswered {
            sheet,
            accepted: true
        }]
    );
    // Answered once: a second answer is nothing.
    h.request(
        SHELL,
        Request::SheetAnswer {
            sheet,
            accepted: false,
            serial: 0,
        },
    )
    .unwrap();
    assert!(h.scene.take_sheet_answers().is_empty());
    // A refusal stands without an input.
    let other = h.scene.show_sheet(None, r#"{"kind":"uninstall"}"#).unwrap();
    assert_ne!(other, sheet);
    h.request(
        SHELL,
        Request::SheetAnswer {
            sheet: other,
            accepted: false,
            serial: 0,
        },
    )
    .unwrap();
    assert_eq!(
        h.scene.take_sheet_answers(),
        [SheetAnswered {
            sheet: other,
            accepted: false
        }]
    );
    // Withdrawn, the shell is told.
    let third = h.scene.show_sheet(None, "{}").unwrap();
    h.clear_log();
    h.scene.dismiss_sheet(third);
    h.pump();
    assert!(h
        .events_for(SHELL)
        .contains(&Event::SheetDismissed { sheet: third }));
}

#[test]
fn no_sheet_is_accepted_while_locked() {
    let mut h = Harness::new();
    let sheet = h.scene.show_sheet(None, "{}").unwrap();
    let press = shell_press(&mut h);
    h.scene.lock();
    h.request(
        SHELL,
        Request::SheetAnswer {
            sheet,
            accepted: true,
            serial: press,
        },
    )
    .unwrap();
    assert!(h.scene.take_sheet_answers().is_empty());
}

#[test]
fn a_shell_that_connects_later_is_sent_each_sheet_still_asked() {
    let mut h = Harness::new();
    let asked = h.scene.show_sheet(None, r#"{"kind":"install"}"#).unwrap();
    let answered = h.scene.show_sheet(None, r#"{"kind":"uninstall"}"#).unwrap();
    h.request(
        SHELL,
        Request::SheetAnswer {
            sheet: answered,
            accepted: false,
            serial: 0,
        },
    )
    .unwrap();
    // The shell started again: a new connection, granted.
    const LATE: ClientId = 20;
    h.scene
        .connect(
            LATE,
            Grants {
                shell: true,
                ..Grants::default()
            },
        )
        .unwrap();
    h.pump();
    let sheets: Vec<u32> = h
        .events_for(LATE)
        .iter()
        .filter_map(|event| match event {
            Event::SheetText {
                sheet, last: true, ..
            } => Some(*sheet),
            _ => None,
        })
        .collect();
    assert_eq!(
        sheets,
        [asked],
        "the one still asked, once; not the one answered"
    );
    assert_eq!(texts(&h.events_for(LATE)), r#"{"kind":"install"}"#);
}

#[test]
fn a_sheet_goes_with_the_client_it_was_asked_for() {
    let mut h = Harness::new();
    let mine = h.scene.show_sheet(Some(APP), "{}").unwrap();
    let other = h.scene.show_sheet(Some(OTHER), "{}").unwrap();
    h.clear_log();
    h.scene.disconnect(APP);
    h.pump();
    assert!(h
        .events_for(SHELL)
        .contains(&Event::SheetDismissed { sheet: mine }));
    assert_eq!(
        h.scene.take_sheet_answers(),
        [SheetAnswered {
            sheet: mine,
            accepted: false
        }],
        "answered no, so what waited for it ends"
    );
    // The other's still stands, and the shell may still answer it.
    let press = shell_press(&mut h);
    h.request(
        SHELL,
        Request::SheetAnswer {
            sheet: other,
            accepted: true,
            serial: press,
        },
    )
    .unwrap();
    assert_eq!(
        h.scene.take_sheet_answers(),
        [SheetAnswered {
            sheet: other,
            accepted: true
        }]
    );
}

#[test]
fn an_application_asks_for_a_capability_and_is_answered() {
    let mut h = Harness::new();
    let input = app_press(&mut h);
    h.request(
        APP,
        Request::RequestCapability {
            serial: 7,
            input,
            capability: 1,
        },
    )
    .unwrap();
    assert_eq!(
        h.request(
            APP,
            Request::RequestCapability {
                serial: 8,
                input,
                capability: 9
            }
        ),
        Err(ErrorCode::InvalidMessage)
    );
    assert_eq!(
        h.scene.take_capability_requests(),
        [CapabilityRequest {
            client: APP,
            serial: 7,
            capability: 1,
            with_input: true
        }]
    );
    h.clear_log();
    h.scene.capability_result(APP, 7, false);
    h.pump();
    assert!(h.events_for(APP).contains(&Event::CapabilityResult {
        serial: 7,
        granted: false
    }));
}
