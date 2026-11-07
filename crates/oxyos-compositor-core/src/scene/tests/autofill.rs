//! AutoFill (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 6): a field that says what
//! it holds, focused in the key window of an application the session started, is offered the
//! provider's suggestions in the list a shell draws at its caret; ↑ ↓ Return and Escape are the
//! list's; the item picked, and only it, is filled into the field and its paired field as one
//! transaction. Nothing while locked or in another window.

use super::*;
use oxyos_protocol::TextContentType;

const UP: u32 = 103;
const DOWN: u32 = 108;
const RETURN: u32 = 28;
const APPLICATION: &str = "so.example.mail";

fn press(h: &mut Harness, code: u32) {
    h.scene.keyboard(code, true, None);
    h.scene.keyboard(code, false, None);
}

/// The shell showing output 1 (its dock), with its candidates surface.
fn shell_candidates(h: &mut Harness) -> SurfaceKey {
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Bottom,
            output: 1,
            size: Size::new(1, 48),
            exclusive: 48,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
    h.request(
        SHELL,
        Request::CreateInputCandidates {
            surface: 7,
            size: Size::new(280, 60),
        },
    )
    .unwrap();
    let candidates = key(SHELL, 7);
    h.present(candidates);
    candidates
}

/// Describes `window`'s field: `text`, holding `content`, paired or not, its caret at (10, 20).
fn describe(
    h: &mut Harness,
    window: SurfaceKey,
    new: bool,
    text: &str,
    content: TextContentType,
    paired: bool,
    serial: u32,
) {
    let surface = window.id;
    if new {
        h.request(window.client, Request::TextInputEnable { surface })
            .unwrap();
    }
    for text in TextChunk::split(text) {
        h.request(
            window.client,
            Request::TextInputSurrounding { surface, text },
        )
        .unwrap();
    }
    let end = text.len() as u32;
    for request in [
        Request::TextInputSelection {
            surface,
            anchor: end,
            cursor: end,
        },
        Request::TextInputCaret {
            surface,
            rect: Rect::new(10, 20, 2, 18),
        },
        Request::TextInputContentType {
            surface,
            content,
            paired,
        },
        Request::TextInputCommit { surface, serial },
    ] {
        h.request(window.client, request).unwrap();
    }
}

/// An application the session started, its key window's username field paired with a password
/// field, the shell's candidates surface: the field's query taken.
fn offered() -> (Harness, SurfaceKey, SurfaceKey, AutofillQuery) {
    let mut h = Harness::new();
    let candidates = shell_candidates(&mut h);
    h.scene.set_application(APP, APPLICATION);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    describe(&mut h, window, true, "", TextContentType::Username, true, 1);
    logged(&mut h);
    let queries = h.scene.take_autofill_queries();
    assert_eq!(queries.len(), 1);
    (h, window, candidates, queries[0].clone())
}

fn login(title: &str, username: &str) -> AutofillItem {
    AutofillItem {
        kind: AutofillKind::Login,
        title: title.into(),
        detail: username.into(),
    }
}

fn two_logins() -> Vec<AutofillItem> {
    vec![
        login("example.com", "bea@example.com"),
        login("example.com", "ana@example.com"),
    ]
}

/// The list the shell was told last, if any.
fn told(h: &mut Harness) -> Option<Option<AutofillList>> {
    h.scene
        .take_autofill_tells()
        .into_iter()
        .rev()
        .find(|(shell, _)| *shell == SHELL)
        .map(|(_, list)| list)
}

fn fills(events: &[Event]) -> Vec<(bool, String)> {
    let mut fills: Vec<(bool, String)> = Vec::new();
    for event in events {
        if let Event::TextInputFill { paired, text, .. } = event {
            match fills.last_mut() {
                Some((last, so_far)) if last == paired => so_far.push_str(text.as_str()),
                _ => fills.push((*paired, text.as_str().to_string())),
            }
        }
    }
    fills
}

#[test]
fn a_field_with_a_content_type_in_the_key_window_is_offered_for_its_application() {
    let (mut h, _, candidates, query) = offered();
    assert_eq!(
        query,
        AutofillQuery {
            token: query.token,
            client: APP,
            application: APPLICATION.into(),
            content: TextContentType::Username,
        }
    );
    // Nothing is shown until the provider answers, nor when it has nothing.
    assert!(!h.rendered(candidates));
    h.scene.answer_autofill(query.token, Vec::new());
    assert_eq!(told(&mut h), None);
    assert!(h.scene.autofill_shown().is_none());
}

#[test]
fn the_list_is_drawn_by_the_shell_under_the_caret_and_the_application_sees_nothing() {
    let (mut h, window, candidates, query) = offered();
    h.scene.answer_autofill(query.token, two_logins());
    assert_eq!(
        told(&mut h),
        Some(Some(AutofillList {
            items: two_logins(),
            highlighted: 0,
            content: TextContentType::Username,
        }))
    );
    h.pump();
    let w = h.rect(window);
    assert_eq!(h.rect(candidates), Rect::new(w.x + 10, w.y + 38, 280, 60));
    assert!(h.rendered(candidates));
    assert!(to(&logged(&mut h), APP).is_empty());
}

#[test]
fn a_field_without_a_content_type_or_of_a_process_the_session_did_not_start_is_not_offered() {
    let mut h = Harness::new();
    shell_candidates(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    // Started by no launch record: whatever its field says, nothing is asked.
    describe(&mut h, window, true, "", TextContentType::Password, true, 1);
    assert!(h.scene.take_autofill_queries().is_empty());
    // Its launch record found, its field is offered.
    h.scene.set_application(APP, APPLICATION);
    assert_eq!(h.scene.take_autofill_queries().len(), 1);
    // Another field, holding nothing for AutoFill: nothing.
    describe(&mut h, window, true, "", TextContentType::None, false, 2);
    assert!(h.scene.take_autofill_queries().is_empty());
}

#[test]
fn the_keys_move_pick_and_dismiss_and_never_reach_the_field() {
    let (mut h, _, _, query) = offered();
    h.scene.answer_autofill(query.token, two_logins());
    told(&mut h);
    logged(&mut h);
    press(&mut h, DOWN);
    assert_eq!(told(&mut h).flatten().map(|list| list.highlighted), Some(1));
    // Past the last, it stays.
    press(&mut h, DOWN);
    press(&mut h, UP);
    assert_eq!(told(&mut h).flatten().map(|list| list.highlighted), Some(0));
    press(&mut h, DOWN);
    press(&mut h, RETURN);
    assert_eq!(told(&mut h), Some(None));
    assert_eq!(
        h.scene.take_autofill_picks(),
        [AutofillPick {
            token: query.token,
            index: 1,
        }]
    );
    // None of those keys reached the application.
    assert!(!to(&logged(&mut h), APP)
        .iter()
        .any(|event| matches!(event, Event::Key { .. } | Event::Text { .. })));
    // The list gone, the keys are the field's again.
    press(&mut h, DOWN);
    assert!(to(&logged(&mut h), APP)
        .iter()
        .any(|event| matches!(event, Event::Key { key: DOWN, .. })));
}

#[test]
fn escape_dismisses_the_list_for_that_field() {
    let (mut h, window, _, query) = offered();
    h.scene.answer_autofill(query.token, two_logins());
    told(&mut h);
    press(&mut h, ESCAPE);
    assert_eq!(told(&mut h), Some(None));
    assert!(h.scene.take_autofill_picks().is_empty());
    // Typing in the same field does not bring it back.
    describe(
        &mut h,
        window,
        false,
        "b",
        TextContentType::Username,
        true,
        2,
    );
    assert_eq!(told(&mut h), None);
    assert!(h.scene.take_autofill_queries().is_empty());
}

#[test]
fn what_is_typed_narrows_the_logins_shown() {
    let (mut h, window, _, query) = offered();
    h.scene.answer_autofill(query.token, two_logins());
    told(&mut h);
    describe(
        &mut h,
        window,
        false,
        "A",
        TextContentType::Username,
        true,
        2,
    );
    let list = told(&mut h).flatten().expect("shown");
    assert_eq!(list.items, [login("example.com", "ana@example.com")]);
    // Return picks the one shown: the provider's second.
    press(&mut h, RETURN);
    assert_eq!(h.scene.take_autofill_picks()[0].index, 1);
    // Text no login begins hides the list.
    let (mut h, window, _, query) = offered();
    h.scene.answer_autofill(query.token, two_logins());
    told(&mut h);
    describe(
        &mut h,
        window,
        false,
        "z",
        TextContentType::Username,
        true,
        2,
    );
    assert_eq!(told(&mut h), Some(None));
}

#[test]
fn the_pick_fills_exactly_its_credential_into_the_field_and_its_pair_once() {
    let (mut h, window, _, query) = offered();
    h.scene.answer_autofill(query.token, two_logins());
    // Without a pick, nothing is filled.
    assert!(!h
        .scene
        .fill_autofill(query.token, Some("bea@example.com"), Some("fixture-pw")));
    press(&mut h, RETURN);
    logged(&mut h);
    let password = "fixture-password-longer-than-one-chunk-of-text";
    assert!(h
        .scene
        .fill_autofill(query.token, Some("bea@example.com"), Some(password)));
    let events = to(&logged(&mut h), APP);
    assert_eq!(
        fills(&events),
        [
            (false, "bea@example.com".to_string()),
            (true, password.to_string())
        ]
    );
    // One transaction, made for the field's commit, a user input as typing is.
    let done: Vec<&Event> = events
        .iter()
        .filter(|event| matches!(event, Event::TextInputDone { .. }))
        .collect();
    assert!(matches!(
        done[..],
        [Event::TextInputDone { surface, commit: 1, .. }] if *surface == window.id
    ));
    assert!(matches!(events.last(), Some(Event::TextInputDone { .. })));
    // Only once per pick.
    assert!(!h
        .scene
        .fill_autofill(query.token, Some("bea@example.com"), Some(password)));
}

#[test]
fn a_field_with_no_pair_is_filled_alone() {
    let mut h = Harness::new();
    shell_candidates(&mut h);
    h.scene.set_application(APP, APPLICATION);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    describe(
        &mut h,
        window,
        true,
        "",
        TextContentType::Username,
        false,
        1,
    );
    let token = h.scene.take_autofill_queries()[0].token;
    h.scene.answer_autofill(token, two_logins());
    press(&mut h, RETURN);
    logged(&mut h);
    assert!(h
        .scene
        .fill_autofill(token, Some("bea@example.com"), Some("fixture-pw")));
    assert_eq!(
        fills(&to(&logged(&mut h), APP)),
        [(false, "bea@example.com".to_string())]
    );
}

#[test]
fn the_shells_press_picks_only_with_an_input_and_only_its_own_list() {
    let (mut h, _, _, query) = offered();
    h.scene.answer_autofill(query.token, two_logins());
    assert!(!h.scene.choose_autofill(SHELL, 1, false));
    assert!(!h.scene.choose_autofill(APP, 1, true));
    assert!(!h.scene.choose_autofill(SHELL, 2, true));
    assert!(h.scene.take_autofill_picks().is_empty());
    assert!(h.scene.choose_autofill(SHELL, 1, true));
    assert_eq!(
        h.scene.take_autofill_picks(),
        [AutofillPick {
            token: query.token,
            index: 1,
        }]
    );
    assert!(h.scene.autofill_shown().is_none());
}

#[test]
fn another_window_made_key_hides_the_list_and_its_fill_is_refused() {
    let (mut h, window, _, query) = offered();
    h.scene.answer_autofill(query.token, two_logins());
    press(&mut h, RETURN);
    told(&mut h);
    let other = h.toplevel(OTHER, 1, 300, 200);
    h.activate(other);
    assert!(!h
        .scene
        .fill_autofill(query.token, Some("bea@example.com"), Some("fixture-pw")));
    assert!(fills(&to(&logged(&mut h), APP)).is_empty());
    // Key again, the field is offered anew: the old answer is dropped.
    h.activate(window);
    let again = h.scene.take_autofill_queries();
    assert_eq!(again.len(), 1);
    assert_ne!(again[0].token, query.token);
    h.scene.answer_autofill(query.token, two_logins());
    assert_eq!(told(&mut h), None);
}

#[test]
fn nothing_is_offered_or_filled_while_locked() {
    let (mut h, _, candidates, query) = offered();
    h.scene.answer_autofill(query.token, two_logins());
    press(&mut h, RETURN);
    told(&mut h);
    h.scene.lock();
    logged(&mut h);
    assert!(!h
        .scene
        .fill_autofill(query.token, Some("bea@example.com"), Some("fixture-pw")));
    assert!(fills(&to(&logged(&mut h), APP)).is_empty());
    assert!(h.scene.take_autofill_queries().is_empty());
    h.scene.answer_autofill(query.token, two_logins());
    assert_eq!(told(&mut h), None);
    assert!(!h.rendered(candidates));
}
