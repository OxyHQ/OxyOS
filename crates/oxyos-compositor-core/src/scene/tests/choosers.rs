//! File choosers (ADR 0029, decision 8): any application asks for its own window citing its
//! input; the chooser the program attaches makes the window's sheet, modal to that window alone;
//! only the chooser answers, and only the asker is told, once.

use super::*;

/// The chooser the program starts, a client of its own.
const CHOOSER: ClientId = 6;

fn ask(h: &mut Harness, client: ClientId, serial: u32, surface: u32, input: u64, text: &str) {
    let chunks = TextChunk::split(text);
    let count = chunks.len();
    for (index, chunk) in chunks.iter().enumerate() {
        h.request(
            client,
            Request::ChooseFiles {
                serial,
                surface,
                input,
                index: index as u16,
                last: index + 1 == count,
                text: PreeditText::new(chunk.as_str()).unwrap(),
            },
        )
        .unwrap();
    }
}

fn answer(h: &mut Harness, client: ClientId, input: u64, text: &str) -> Result<(), ErrorCode> {
    let chunks = TextChunk::split(text);
    let count = chunks.len();
    for (index, chunk) in chunks.iter().enumerate() {
        h.request(
            client,
            Request::AnswerFileChooser {
                input,
                index: index as u16,
                last: index + 1 == count,
                text: PreeditText::new(chunk.as_str()).unwrap(),
            },
        )?;
    }
    Ok(())
}

/// What `client` was told of its requests, each answer whole, in order.
fn chosen(h: &Harness, client: ClientId) -> Vec<(u32, String)> {
    let mut answers: Vec<(u32, String)> = Vec::new();
    let mut open = false;
    for event in h.events_for(client) {
        if let Event::FilesChosen {
            serial,
            index,
            last,
            text,
        } = event
        {
            if index == 0 || !open {
                answers.push((serial, String::new()));
            }
            answers.last_mut().unwrap().1.push_str(text.as_str());
            open = !last;
        }
    }
    answers
}

/// A press inside `surface`, released: an input of its client's.
fn press_in(h: &mut Harness, surface: SurfaceKey, x: i32, y: i32) -> u64 {
    let rect = h.rect(surface);
    let serial = h.press(rect.x + x, rect.y + y);
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    serial
}

/// A click inside `surface` that may not reach it.
fn click_in(h: &mut Harness, surface: SurfaceKey, x: i32, y: i32) {
    let rect = h.rect(surface);
    h.click(rect.x + x, rect.y + y);
}

/// The application's window of 800x600 asking, a press in it cited; the request's id.
fn asked(h: &mut Harness) -> (SurfaceKey, u32) {
    let window = h.toplevel(APP, 1, 800, 600);
    let input = press_in(h, window, 40, 40);
    ask(h, APP, 7, 1, input, r#"{"open":{"types":[".png"]}}"#);
    let requests = h.scene.take_file_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].client, APP);
    assert_eq!(requests[0].text, r#"{"open":{"types":[".png"]}}"#);
    (window, requests[0].id)
}

/// The chooser attached to request `id`, its window of `width`x`height` made and shown.
fn attach_sheet(h: &mut Harness, id: u32, width: u32, height: u32) -> SurfaceKey {
    h.scene.connect(CHOOSER, Grants::default()).unwrap();
    assert!(h.scene.attach_chooser(CHOOSER, id));
    h.toplevel(CHOOSER, 1, width, height)
}

#[test]
fn a_request_citing_the_asker_s_input_goes_to_the_program_and_any_other_is_refused_at_once() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 800, 600);
    // No input, or one the asker never received (another client's): refused, no panel.
    ask(&mut h, APP, 1, 1, 0, r#"{"open":{}}"#);
    let other = h.toplevel(OTHER, 1, 300, 200);
    let theirs = press_in(&mut h, other, 10, 10);
    ask(&mut h, APP, 2, 1, theirs, r#"{"open":{}}"#);
    assert!(h.scene.take_file_requests().is_empty());
    assert_eq!(
        chosen(&h, APP),
        vec![
            (1, r#"{"refused":{"reason":"needsInput"}}"#.to_string()),
            (2, r#"{"refused":{"reason":"needsInput"}}"#.to_string()),
        ]
    );
    // Its own press: handed on whole, in chunks reassembled.
    let input = press_in(&mut h, window, 40, 40);
    let long = format!(r#"{{"save":{{"name":"{}.txt"}}}}"#, "a".repeat(100));
    ask(&mut h, APP, 3, 1, input, &long);
    let requests = h.scene.take_file_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        (requests[0].client, requests[0].text.as_str()),
        (APP, long.as_str())
    );
    assert!(h.scene.chooser_open(requests[0].id));
    // A second panel for the same window is refused while the first is open.
    let again = press_in(&mut h, window, 50, 50);
    ask(&mut h, APP, 4, 1, again, r#"{"open":{}}"#);
    assert!(h.scene.take_file_requests().is_empty());
    assert_eq!(
        chosen(&h, APP).last().unwrap(),
        &(4, r#"{"refused":{"reason":"busy"}}"#.to_string())
    );
}

#[test]
fn a_request_names_a_toplevel_of_its_own_and_its_chunks_in_order() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 800, 600);
    let input = press_in(&mut h, window, 40, 40);
    let text = PreeditText::new("{}").unwrap();
    let choose = |surface, index| Request::ChooseFiles {
        serial: 1,
        surface,
        input,
        index,
        last: true,
        text,
    };
    assert_eq!(h.request(APP, choose(9, 0)), Err(ErrorCode::UnknownSurface));
    // OTHER's window is not APP's to name: its surface numbers are its own.
    h.toplevel(OTHER, 2, 300, 200);
    assert_eq!(h.request(APP, choose(2, 0)), Err(ErrorCode::UnknownSurface));
    let popup = h.popup(window, 2, menu(Rect::new(0, 0, 10, 10), 100, 100));
    h.present(popup);
    assert_eq!(h.request(APP, choose(2, 0)), Err(ErrorCode::InvalidRole));
    assert_eq!(h.request(APP, choose(1, 3)), Err(ErrorCode::InvalidOrder));
}

#[test]
fn the_chooser_s_window_is_a_sheet_at_the_top_of_the_asking_window_taking_its_keyboard() {
    let mut h = Harness::new();
    let (window, id) = asked(&mut h);
    let w = h.rect(window);
    let sheet = attach_sheet(&mut h, id, 600, 400);
    assert_eq!(h.scene.chooser_sheet(id), Some(sheet));
    assert_eq!(
        h.rect(sheet),
        Rect::new(w.x + 100, w.y + SHEET_TOP as i32, 600, 400)
    );
    // The sheet has the keyboard; the asking window stays the active one, and is drawn so.
    assert_eq!(h.scene.focus(), Some(sheet));
    assert_eq!(h.scene.active_window(), Some(window));
    assert!(h.scene.window_look(sheet).is_some_and(|look| look.active));
    // A press on the window beside the sheet does not reach it; one on the sheet reaches the
    // chooser; another window is as usable as ever.
    h.clear_log();
    click_in(&mut h, window, 20, 300);
    assert_eq!(pointer_presses(&h.events_for(APP)), 0);
    assert_eq!(h.scene.focus(), Some(sheet));
    press_in(&mut h, sheet, 20, 20);
    assert_eq!(pointer_presses(&h.events_for(CHOOSER)), 2);
    // Another window, brought forward (as the shell does): it takes presses and the keyboard.
    let other = h.toplevel(OTHER, 1, 200, 150);
    h.scene.activate(other);
    h.clear_log();
    press_in(&mut h, other, 10, 10);
    assert_eq!(pointer_presses(&h.events_for(OTHER)), 2);
    assert_eq!(h.scene.focus(), Some(other));
    // Back to the window: its sheet takes the keyboard again.
    click_in(&mut h, window, 20, 300);
    assert_eq!(h.scene.focus(), Some(sheet));
    assert_eq!(h.scene.active_window(), Some(window));
}

#[test]
fn a_sheet_fits_its_window_and_a_small_window_has_it_overhang() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 500, 360);
    let input = press_in(&mut h, window, 40, 40);
    ask(&mut h, APP, 1, 1, input, r#"{"open":{}}"#);
    let id = h.scene.take_file_requests()[0].id;
    let w = h.rect(window);
    let sheet = attach_sheet(&mut h, id, 760, 480);
    // 500 - 2 x 24 wide; 360 - 52 - 24 is under the least height, which it keeps.
    let r = h.rect(sheet);
    assert_eq!((r.width, r.height), (452, SHEET_MIN.height));
    assert_eq!((r.x, r.y), (w.x + 24, w.y + SHEET_TOP as i32));
}

#[test]
fn a_sheet_moves_with_its_window_and_its_drag_moves_the_window() {
    let mut h = Harness::new();
    let (window, id) = asked(&mut h);
    let sheet = attach_sheet(&mut h, id, 600, 400);
    let (w, s) = (h.rect(window), h.rect(sheet));
    // The chooser's window asks for what a window asks for: none of it is an error.
    for request in [
        Request::SetTitle {
            surface: 1,
            index: 0,
            last: true,
            text: PreeditText::new("Stash").unwrap(),
        },
        Request::SetSizeLimits {
            surface: 1,
            min_width: 400,
            min_height: 300,
            max_width: 0,
            max_height: 0,
        },
        Request::SetMaximized {
            surface: 1,
            enabled: true,
        },
        Request::MenusBegin {
            surface: 1,
            serial: 1,
            items: 0,
        },
        Request::MenusCommit { surface: 1 },
    ] {
        h.request(CHOOSER, request).unwrap();
    }
    assert_eq!(h.rect(sheet), s);
    // Its drag area pressed: the window moves, the sheet with it, in the same frame.
    let serial = h.press(s.x + 300, s.y + 10);
    h.request(CHOOSER, Request::BeginMove { surface: 1, serial })
        .unwrap();
    h.scene.pointer_motion(s.x + 300 + 120, s.y + 10 + 70);
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    let (moved, carried) = (h.rect(window), h.rect(sheet));
    assert_eq!((moved.x, moved.y), (w.x + 120, w.y + 70));
    assert_eq!((carried.x, carried.y), (s.x + 120, s.y + 70));
}

#[test]
fn only_the_chooser_answers_and_its_answer_reaches_the_asker_alone_once() {
    let mut h = Harness::new();
    let (window, id) = asked(&mut h);
    h.toplevel(OTHER, 1, 300, 200);
    let sheet = attach_sheet(&mut h, id, 600, 400);
    assert_eq!(
        answer(&mut h, APP, 0, r#"{"cancelled":{}}"#),
        Err(ErrorCode::Unauthorized)
    );
    assert_eq!(
        answer(&mut h, OTHER, 0, r#"{"cancelled":{}}"#),
        Err(ErrorCode::Unauthorized)
    );
    // Without an input of its own, a choice is handed on as one: the program refuses it.
    let choice = r#"{"chosen":["file:///home/ana/Pictures/a%20b.png"]}"#;
    answer(&mut h, CHOOSER, 0, choice).unwrap();
    let answers = h.scene.take_chooser_answers();
    assert_eq!(answers.len(), 1);
    assert!(!answers[0].with_input);
    let open = press_in(&mut h, sheet, 500, 380);
    answer(&mut h, CHOOSER, open, choice).unwrap();
    let answers = h.scene.take_chooser_answers();
    assert_eq!(
        answers,
        vec![ChooserAnswer {
            id,
            with_input: true,
            text: choice.to_string()
        }]
    );
    // The program's answer: to the asker alone, the sheet gone at once, the window's keyboard
    // its own again, and the chooser to be ended.
    h.clear_log();
    h.scene.answer_files(id, choice);
    h.pump();
    assert_eq!(chosen(&h, APP), vec![(7, choice.to_string())]);
    assert!(chosen(&h, OTHER).is_empty() && chosen(&h, CHOOSER).is_empty());
    assert!(!h.rendered(sheet));
    assert_eq!(h.dismissed(sheet), Some(DismissReason::Superseded));
    assert_eq!(h.scene.focus(), Some(window));
    assert_eq!(h.scene.take_ended_choosers(), vec![id]);
    assert!(!h.scene.chooser_open(id));
    // Once: neither the program nor the asker's withdrawal answers again, and the chooser no
    // longer answers anything.
    h.scene.answer_files(id, r#"{"cancelled":{}}"#);
    h.request(APP, Request::CancelChooseFiles { serial: 7 })
        .unwrap();
    assert_eq!(chosen(&h, APP).len(), 1);
    assert_eq!(
        answer(&mut h, CHOOSER, 0, r#"{"cancelled":{}}"#),
        Err(ErrorCode::Unauthorized)
    );
    // The window takes presses again.
    h.clear_log();
    press_in(&mut h, window, 20, 300);
    assert_eq!(pointer_presses(&h.events_for(APP)), 2);
}

#[test]
fn a_withdrawn_request_is_answered_cancelled_and_its_sheet_goes() {
    let mut h = Harness::new();
    let (window, id) = asked(&mut h);
    let sheet = attach_sheet(&mut h, id, 600, 400);
    h.request(APP, Request::CancelChooseFiles { serial: 7 })
        .unwrap();
    assert_eq!(
        chosen(&h, APP),
        vec![(7, r#"{"cancelled":{}}"#.to_string())]
    );
    assert!(!h.rendered(sheet));
    assert_eq!(h.scene.take_ended_choosers(), vec![id]);
    assert_eq!(h.scene.focus(), Some(window));
}

#[test]
fn a_request_ends_when_its_window_its_asker_or_its_chooser_goes() {
    // The window destroyed: cancelled to its asker; the sheet dismissed with its parent.
    let mut h = Harness::new();
    let (_, id) = asked(&mut h);
    let sheet = attach_sheet(&mut h, id, 600, 400);
    h.request(APP, Request::Destroy { surface: 1 }).unwrap();
    assert_eq!(
        chosen(&h, APP),
        vec![(7, r#"{"cancelled":{}}"#.to_string())]
    );
    assert_eq!(h.dismissed(sheet), Some(DismissReason::ParentGone));
    assert_eq!(h.scene.take_ended_choosers(), vec![id]);

    // The chooser gone without answering: cancelled.
    let mut h = Harness::new();
    let (window, id) = asked(&mut h);
    attach_sheet(&mut h, id, 600, 400);
    h.scene.disconnect(CHOOSER);
    h.pump();
    assert_eq!(
        chosen(&h, APP),
        vec![(7, r#"{"cancelled":{}}"#.to_string())]
    );
    assert_eq!(h.scene.take_ended_choosers(), vec![id]);
    assert_eq!(h.scene.focus(), Some(window));

    // The asker gone: ended, its chooser's sheet gone with its window, nothing told.
    let mut h = Harness::new();
    let (_, id) = asked(&mut h);
    let sheet = attach_sheet(&mut h, id, 600, 400);
    h.clear_log();
    h.scene.disconnect(APP);
    h.pump();
    assert!(!h.rendered(sheet));
    assert_eq!(h.scene.take_ended_choosers(), vec![id]);
    assert!(!h.scene.attach_chooser(CHOOSER, id));

    // Asked and gone before the program took it: nothing is handed on.
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 800, 600);
    let input = press_in(&mut h, window, 40, 40);
    ask(&mut h, APP, 7, 1, input, r#"{"open":{}}"#);
    h.scene.disconnect(APP);
    assert!(h.scene.take_file_requests().is_empty());
}

#[test]
fn a_chooser_serves_one_request_and_never_its_own() {
    let mut h = Harness::new();
    let (_, id) = asked(&mut h);
    assert!(!h.scene.attach_chooser(APP, id));
    assert!(!h.scene.attach_chooser(CHOOSER, id), "not connected");
    h.scene.connect(CHOOSER, Grants::default()).unwrap();
    assert!(h.scene.attach_chooser(CHOOSER, id));
    assert!(!h.scene.attach_chooser(OTHER, id), "it has a chooser");
    assert_eq!(h.scene.chooser_of(CHOOSER), Some(id));
    // Its second window is a window of its own, not another sheet.
    let first = h.toplevel(CHOOSER, 1, 600, 400);
    let second = h.toplevel(CHOOSER, 2, 300, 200);
    assert_eq!(h.scene.chooser_sheet(id), Some(first));
    assert!(h.scene.is_sheet(first) && !h.scene.is_sheet(second));
}

#[test]
fn nothing_is_asked_while_locked() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 800, 600);
    let input = press_in(&mut h, window, 40, 40);
    h.request(SHELL, Request::LockSession {}).unwrap();
    ask(&mut h, APP, 1, 1, input, r#"{"open":{}}"#);
    assert!(h.scene.take_file_requests().is_empty());
    assert_eq!(
        chosen(&h, APP),
        vec![(1, r#"{"refused":{"reason":"locked"}}"#.to_string())]
    );
}
