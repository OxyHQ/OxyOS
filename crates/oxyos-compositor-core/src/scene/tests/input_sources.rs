//! ADR 0041: the candidates an input method hands the system, drawn by the session's shell at
//! the caret; the shell's presses passed back; and the input-source shortcuts: Super+Alt+Space,
//! Super+Alt+Shift+Space, Control+Super+Space and the emoji key, with the launcher's search on
//! Super pressed alone and Spotlight on Super+Space (docs/adr/0051-search.mdx).

use super::*;
use oxyos_protocol::{CANDIDATES_NEXT, NO_CANDIDATE};

const SUPER: u32 = 125;
const CTRL: u32 = 29;
const ALT: u32 = 56;
const SHIFT: u32 = 42;
const SPACE: u32 = 57;
const EMOJI_PICKER: u32 = 0x249;

fn keys(h: &mut Harness, codes: &[u32]) {
    for code in codes {
        h.scene.keyboard(*code, true, None);
    }
    for code in codes.iter().rev() {
        h.scene.keyboard(*code, false, None);
    }
}

/// The shell's dock, so the shell shows output 1.
fn dock(h: &mut Harness) {
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
}

/// A served field with its caret at (10, 20), the shell showing output 1 with a candidates
/// surface of its own.
fn handed() -> (Harness, SurfaceKey, SurfaceKey) {
    let (mut h, window) = served("");
    dock(&mut h);
    move_caret(&mut h, window, Rect::new(10, 20, 2, 18), 2);
    h.request(
        SHELL,
        Request::CreateInputCandidates {
            surface: 7,
            size: Size::new(300, 40),
        },
    )
    .unwrap();
    let candidates = key(SHELL, 7);
    h.present(candidates);
    logged(&mut h);
    (h, window, candidates)
}

/// The input method hands the system `list`, highlighted and flagged.
fn hand(h: &mut Harness, list: &[&str], aux: &str, highlighted: u8, flags: u8) {
    for (index, text) in list.iter().enumerate() {
        for text in TextChunk::split(text) {
            h.request(
                IME,
                Request::InputMethodCandidate {
                    index: index as u8,
                    text,
                },
            )
            .unwrap();
        }
    }
    for text in TextChunk::split(aux) {
        h.request(IME, Request::InputMethodCandidatesAux { text })
            .unwrap();
    }
    h.request(
        IME,
        Request::InputMethodCandidates {
            count: list.len() as u8,
            highlighted,
            flags,
        },
    )
    .unwrap();
}

#[test]
fn the_shell_is_told_the_candidates_and_draws_them_under_the_caret() {
    let (mut h, window, candidates) = handed();
    // No list yet: its surface is hidden.
    assert!(!h.rendered(candidates));
    let long = "a candidate longer than one chunk of text";
    hand(&mut h, &["你好", long], "ni'hao", 0, CANDIDATES_NEXT);
    let events = logged(&mut h);
    let mut told = vec![Event::InputCandidate {
        index: 0,
        text: TextChunk::new("你好").unwrap(),
    }];
    told.extend(
        TextChunk::split(long)
            .into_iter()
            .map(|text| Event::InputCandidate { index: 1, text }),
    );
    told.push(Event::InputCandidatesAux {
        text: TextChunk::new("ni'hao").unwrap(),
    });
    told.push(Event::InputCandidates {
        count: 2,
        highlighted: 0,
        flags: CANDIDATES_NEXT,
    });
    let to_shell: Vec<Event> = to(&events, SHELL)
        .into_iter()
        .filter(|e| {
            matches!(
                e,
                Event::InputCandidate { .. }
                    | Event::InputCandidatesAux { .. }
                    | Event::InputCandidates { .. }
            )
        })
        .collect();
    assert_eq!(to_shell, told);
    // The application is told nothing of them.
    assert!(to(&events, APP).is_empty());
    // Below the caret's bottom-left corner, as the input method's own would be.
    let w = h.rect(window);
    assert_eq!(h.rect(candidates), Rect::new(w.x + 10, w.y + 38, 300, 40));
    // It follows the caret.
    move_caret(&mut h, window, Rect::new(60, 20, 2, 18), 3);
    assert_eq!(h.rect(candidates), Rect::new(w.x + 60, w.y + 38, 300, 40));
    // An empty list hides them, and the shell is told so.
    hand(&mut h, &[], "", NO_CANDIDATE, 0);
    assert!(
        to(&logged(&mut h), SHELL).contains(&Event::InputCandidates {
            count: 0,
            highlighted: NO_CANDIDATE,
            flags: 0,
        })
    );
    assert!(!h.rendered(candidates));
}

#[test]
fn the_shells_presses_reach_the_input_method_and_no_one_else_may_press() {
    let (mut h, _, _) = handed();
    hand(&mut h, &["你好", "拟好"], "", 0, CANDIDATES_NEXT);
    logged(&mut h);
    h.request(SHELL, Request::ChooseInputCandidate { index: 1 })
        .unwrap();
    h.request(SHELL, Request::TurnInputCandidatesPage { forward: true })
        .unwrap();
    assert_eq!(
        to(&logged(&mut h), IME),
        [
            Event::InputMethodCandidateChosen { index: 1 },
            Event::InputMethodCandidatesPage { forward: true }
        ]
    );
    // Past the list: refused.
    assert_eq!(
        h.request(SHELL, Request::ChooseInputCandidate { index: 2 }),
        Err(ErrorCode::InvalidMessage)
    );
    // An application never chooses for the person.
    assert_eq!(
        h.request(APP, Request::ChooseInputCandidate { index: 0 }),
        Err(ErrorCode::Unauthorized)
    );
    // Hidden since: a late press does nothing.
    hand(&mut h, &[], "", NO_CANDIDATE, 0);
    logged(&mut h);
    h.request(SHELL, Request::ChooseInputCandidate { index: 0 })
        .unwrap();
    assert!(to(&logged(&mut h), IME).is_empty());
}

#[test]
fn a_list_is_refused_when_malformed_and_dropped_when_focus_left() {
    let (mut h, _, candidates) = handed();
    // A count that is not the candidates', a highlight past them, a gap in the indices.
    for (count, highlighted) in [(2, 0), (1, 1)] {
        h.request(
            IME,
            Request::InputMethodCandidate {
                index: 0,
                text: TextChunk::new("你").unwrap(),
            },
        )
        .unwrap();
        assert_eq!(
            h.request(
                IME,
                Request::InputMethodCandidates {
                    count,
                    highlighted,
                    flags: 0
                }
            ),
            Err(ErrorCode::InvalidMessage)
        );
    }
    assert_eq!(
        h.request(
            IME,
            Request::InputMethodCandidate {
                index: 3,
                text: TextChunk::new("你").unwrap(),
            },
        ),
        Err(ErrorCode::InvalidOrder)
    );
    // Shown, then the field loses focus: hidden, and the shell told.
    let (mut h, window, candidates_2) = handed();
    let _ = candidates;
    hand(&mut h, &["你好"], "", 0, 0);
    assert!(h.rendered(candidates_2));
    logged(&mut h);
    h.request(APP, Request::TextInputDisable { surface: window.id })
        .unwrap();
    h.request(
        APP,
        Request::TextInputCommit {
            surface: window.id,
            serial: 3,
        },
    )
    .unwrap();
    let events = logged(&mut h);
    assert!(to(&events, IME).contains(&Event::InputMethodDeactivate {}));
    assert!(to(&events, SHELL).contains(&Event::InputCandidates {
        count: 0,
        highlighted: NO_CANDIDATE,
        flags: 0,
    }));
    assert!(!h.rendered(candidates_2));
}

#[test]
fn only_a_shell_or_the_input_method_makes_a_candidates_surface_one_each() {
    let (mut h, _, _) = handed();
    assert_eq!(
        h.request(
            SHELL,
            Request::CreateInputCandidates {
                surface: 8,
                size: Size::new(10, 10),
            }
        ),
        Err(ErrorCode::InvalidRole)
    );
    assert_eq!(
        h.request(
            OTHER,
            Request::CreateInputCandidates {
                surface: 8,
                size: Size::new(10, 10),
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
}

#[test]
fn super_alone_is_the_search_and_super_alt_space_the_input_sources() {
    let mut h = Harness::new();
    dock(&mut h);
    logged(&mut h);
    // Super pressed and released alone: the launcher's search.
    keys(&mut h, &[SUPER]);
    let searched = |events: &[(ClientId, Event)]| {
        to(events, SHELL).iter().any(|e| {
            matches!(
                e,
                Event::SystemShortcut {
                    shortcut: SystemShortcut::Search,
                    ..
                }
            )
        })
    };
    assert!(searched(&logged(&mut h)));
    // Super with another key, a button or a scroll is not.
    keys(&mut h, &[SUPER, SPACE]);
    assert!(!searched(&logged(&mut h)));
    h.scene.keyboard(SUPER, true, None);
    h.scene.pointer_button(272, true);
    h.scene.pointer_button(272, false);
    h.scene.keyboard(SUPER, false, None);
    assert!(!searched(&logged(&mut h)));
    // Super+Alt+Space is the first step back; held, each Space one further. Alt released and
    // pressed again with Super still held begins again from the one current then.
    h.scene.take_input_source_shortcuts();
    h.scene.keyboard(SUPER, true, None);
    h.scene.keyboard(ALT, true, None);
    for _ in 0..3 {
        h.scene.keyboard(SPACE, true, None);
        h.scene.keyboard(SPACE, false, None);
    }
    h.scene.keyboard(ALT, false, None);
    h.scene.keyboard(ALT, true, None);
    h.scene.keyboard(SPACE, true, None);
    h.scene.keyboard(SPACE, false, None);
    h.scene.keyboard(ALT, false, None);
    h.scene.keyboard(SUPER, false, None);
    keys(&mut h, &[SUPER, ALT, SPACE]);
    keys(&mut h, &[SUPER, ALT, SHIFT, SPACE]);
    keys(&mut h, &[SUPER, CTRL, SPACE]);
    keys(&mut h, &[EMOJI_PICKER]);
    // Super+Space is Spotlight's, Super+Shift+Space and Control+Super+Alt+Space nobody's.
    keys(&mut h, &[SUPER, SPACE]);
    keys(&mut h, &[SUPER, SHIFT, SPACE]);
    keys(&mut h, &[SUPER, CTRL, ALT, SPACE]);
    assert_eq!(
        h.scene.take_input_source_shortcuts(),
        [
            InputSourceShortcut::Previous { step: 1 },
            InputSourceShortcut::Previous { step: 2 },
            InputSourceShortcut::Previous { step: 3 },
            InputSourceShortcut::Previous { step: 1 },
            InputSourceShortcut::Previous { step: 1 },
            InputSourceShortcut::Next,
            InputSourceShortcut::Emoji,
            InputSourceShortcut::Emoji,
        ]
    );
    // None reaches the application, the search none either; Spotlight opened once.
    let events = logged(&mut h);
    assert!(!searched(&events));
    assert!(to(&events, APP).is_empty());
    let spotlights = to(&events, SHELL)
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::SystemShortcut {
                    shortcut: SystemShortcut::Spotlight,
                    ..
                }
            )
        })
        .count();
    assert_eq!(spotlights, 1);
}

#[test]
fn while_locked_no_input_source_shortcut_acts() {
    let mut h = Harness::new();
    dock(&mut h);
    h.scene.lock();
    logged(&mut h);
    keys(&mut h, &[SUPER, ALT, SPACE]);
    keys(&mut h, &[EMOJI_PICKER]);
    keys(&mut h, &[SUPER]);
    assert!(h.scene.take_input_source_shortcuts().is_empty());
    assert!(!to(&logged(&mut h), SHELL).iter().any(|e| matches!(
        e,
        Event::SystemShortcut {
            shortcut: SystemShortcut::Search,
            ..
        }
    )));
}

#[test]
fn a_super_chord_through_either_way_keys_come_is_never_the_search() {
    let mut h = Harness::new();
    dock(&mut h);
    logged(&mut h);
    const F2: u32 = 60;
    // Super+F2 as a client's key step sends it (`Scene::key`): the menu bar, not the search.
    h.scene.key(SUPER, true);
    h.scene.key(F2, true);
    h.scene.key(F2, false);
    h.scene.key(SUPER, false);
    let shortcuts: Vec<SystemShortcut> = to(&logged(&mut h), SHELL)
        .iter()
        .filter_map(|e| match e {
            Event::SystemShortcut { shortcut, .. } => Some(*shortcut),
            _ => None,
        })
        .collect();
    assert_eq!(shortcuts, [SystemShortcut::MenuBar]);
    // Super alone that way is.
    h.scene.key(SUPER, true);
    h.scene.key(SUPER, false);
    assert!(to(&logged(&mut h), SHELL).iter().any(|e| matches!(
        e,
        Event::SystemShortcut {
            shortcut: SystemShortcut::Search,
            ..
        }
    )));
}
