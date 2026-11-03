use crate::reader::{Reader, Utterance};
use oxyos_protocol::{
    AccessibleAction, AccessibleField, AccessibleRole as R, AccessibleState, Event, Politeness,
    PreeditText, ReaderCommand, Rect, Request, TextChunk, NO_ACCESSIBLE_PARENT,
};
use oxyos_speech::Language;

const APP: u32 = 1;
const WINDOW: u32 = 1;
const FOCUSED: u16 = AccessibleState::FOCUSED;

/// The events of a committed tree of `surface`: each node's role, state, name and value.
fn tree(surface: u32, nodes: &[(R, u16, &str, &str)]) -> Vec<Event> {
    tree_of(surface, 0, nodes)
}

/// The same for a popup of the surface `parent` (0: none), as the compositor names it.
fn tree_of(surface: u32, parent: u32, nodes: &[(R, u16, &str, &str)]) -> Vec<Event> {
    let mut events = vec![Event::AccessibilityTree {
        client: APP,
        surface,
        parent,
        nodes: nodes.len() as u32,
    }];
    for (index, (role, bits, name, value)) in nodes.iter().enumerate() {
        let index = index as u32;
        events.push(Event::AccessibleNode {
            client: APP,
            surface,
            index,
            parent: if index == 0 { NO_ACCESSIBLE_PARENT } else { 0 },
            role: *role,
            state: AccessibleState::new(*bits).unwrap(),
            actions: Default::default(),
            rect: Rect::new(0, 0, 10, 10),
        });
        for (field, text) in [
            (AccessibleField::Name, name),
            (AccessibleField::Value, value),
        ] {
            for text in TextChunk::split(text) {
                events.push(Event::AccessibleText {
                    client: APP,
                    surface,
                    index,
                    field,
                    text,
                });
            }
        }
    }
    events.push(Event::AccessibilityCommitted {
        client: APP,
        surface,
    });
    events
}

fn feed(reader: &mut Reader, events: &[Event]) -> Vec<String> {
    for event in events {
        reader.receive(event);
    }
    texts(reader)
}

fn texts(reader: &mut Reader) -> Vec<String> {
    reader.take_said().into_iter().map(|u| u.text).collect()
}

fn command(reader: &mut Reader, command: ReaderCommand) -> Vec<String> {
    assert_eq!(reader.command(command), None);
    texts(reader)
}

/// sample/A11yProbe.tsx's window, roughly, with `focused` the focused node's index.
fn probe(focused: Option<usize>, level: &str) -> Vec<Event> {
    let mut nodes = vec![
        (R::Generic, 0, "", ""),
        (R::Slider, 0, "Volumen", ""),
        (R::Text, AccessibleState::LIVE_POLITE, level, ""),
        (R::Button, 0, "Guardar", ""),
        (R::Group, 0, "", ""),
        (R::TextInput, 0, "Nota", "hola ñandú"),
        (R::Button, AccessibleState::DISABLED, "Borrar", ""),
    ];
    if let Some(index) = focused {
        nodes[index].1 |= FOCUSED;
    }
    tree(WINDOW, &nodes)
}

#[test]
fn the_cursor_moves_through_what_has_something_to_say() {
    let mut reader = Reader::new(Language::Spanish);
    assert!(feed(&mut reader, &probe(None, "Nivel 5")).is_empty());
    assert_eq!(
        command(&mut reader, ReaderCommand::Next),
        ["Volumen, regulador"]
    );
    assert_eq!(command(&mut reader, ReaderCommand::Next), ["Nivel 5"]);
    assert_eq!(
        command(&mut reader, ReaderCommand::Next),
        ["Guardar, botón"]
    );
    // The empty group is passed over.
    assert_eq!(
        command(&mut reader, ReaderCommand::Next),
        ["Nota, hola ñandú, campo de texto"]
    );
    assert_eq!(
        command(&mut reader, ReaderCommand::Next),
        ["Borrar, atenuado, botón"]
    );
    // At the end it stays, saying nothing.
    assert!(command(&mut reader, ReaderCommand::Next).is_empty());
    assert_eq!(reader.cursor(), Some(((APP, WINDOW), 6)));
    assert_eq!(
        command(&mut reader, ReaderCommand::Previous),
        ["Nota, hola ñandú, campo de texto"]
    );
    // In English when the session is.
    let mut english = Reader::new(Language::English);
    feed(&mut english, &probe(None, "Level 5"));
    assert_eq!(
        command(&mut english, ReaderCommand::Previous),
        ["Borrar, dimmed, button"]
    );
}

/// The session's locale, in chunks as the compositor tells it.
fn locale(tag: &str) -> Vec<Event> {
    let chunks = TextChunk::split(tag);
    let count = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| Event::Locale {
            index: index as u8,
            last: index + 1 == count,
            text: PreeditText::new(chunk.as_str()).unwrap(),
        })
        .collect()
}

#[test]
fn the_reader_speaks_the_sessions_locale_from_the_next_utterance() {
    // Its environment's until told.
    let mut reader = Reader::new(Language::English);
    assert!(!reader.told_locale());
    feed(&mut reader, &probe(None, "Nivel 5"));
    assert!(feed(&mut reader, &locale("es-ES")).is_empty());
    assert!(reader.told_locale());
    assert_eq!(reader.language(), Language::Spanish);
    let said = |reader: &mut Reader, command| {
        assert_eq!(reader.command(command), None);
        reader
            .take_said()
            .into_iter()
            .map(|u| (u.text, u.language))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        said(&mut reader, ReaderCommand::Next),
        [("Volumen, regulador".to_string(), Language::Spanish)]
    );
    // A change: English, with its voice, from the next thing said; long enough for two chunks.
    feed(&mut reader, &locale("en-US-u-hc-h23-ca-gregory-nu-latn"));
    assert_eq!(reader.language(), Language::English);
    assert_eq!(
        said(&mut reader, ReaderCommand::Next),
        [("Nivel 5".to_string(), Language::English)]
    );
    assert_eq!(
        said(&mut reader, ReaderCommand::Next),
        [("Guardar, button".to_string(), Language::English)]
    );
    feed(&mut reader, &locale("es-MX-u-hc-h12"));
    assert_eq!(
        said(&mut reader, ReaderCommand::Next),
        [(
            "Nota, hola ñandú, campo de texto".to_string(),
            Language::Spanish
        )]
    );
}

#[test]
fn the_focused_element_is_said_once_and_takes_the_cursor() {
    let mut reader = Reader::new(Language::Spanish);
    feed(&mut reader, &probe(None, "Nivel 5"));
    // The application focuses Nota: it is said, at once.
    for event in probe(Some(5), "Nivel 5") {
        reader.receive(&event);
    }
    assert_eq!(
        reader.take_said(),
        [Utterance {
            text: "Nota, hola ñandú, campo de texto".into(),
            politeness: Politeness::Assertive,
            language: Language::Spanish,
        }]
    );
    // The same focus in the next tree says nothing again.
    assert!(feed(&mut reader, &probe(Some(5), "Nivel 5")).is_empty());
    assert_eq!(
        command(&mut reader, ReaderCommand::Next),
        ["Borrar, atenuado, botón"]
    );
    // Activating asks the application to activate the element under the cursor.
    assert_eq!(
        reader.command(ReaderCommand::Activate),
        Some(Request::AccessibilityAction {
            client: APP,
            surface: WINDOW,
            index: 6,
            action: AccessibleAction::Activate,
            custom: 0,
            start: 0,
            end: 0,
        })
    );
    assert!(texts(&mut reader).is_empty());
    // An element the reader's cursor is on that the application then focuses is not said twice.
    assert_eq!(
        command(&mut reader, ReaderCommand::Previous),
        ["Nota, hola ñandú, campo de texto"]
    );
    assert_eq!(
        command(&mut reader, ReaderCommand::Previous),
        ["Guardar, botón"]
    );
    assert!(feed(&mut reader, &probe(Some(3), "Nivel 5")).is_empty());
}

#[test]
fn announcements_and_live_regions_are_said_with_their_politeness() {
    let mut reader = Reader::new(Language::Spanish);
    feed(&mut reader, &probe(None, "Nivel 5"));
    for event in probe(None, "Nivel 6") {
        reader.receive(&event);
    }
    assert_eq!(
        reader.take_said(),
        [Utterance {
            text: "Nivel 6".into(),
            politeness: Politeness::Polite,
            language: Language::Spanish,
        }]
    );
    // An announcement in chunks, said once it is whole.
    let long = "Guardado: todo lo que escribiste está a salvo";
    let chunks: Vec<TextChunk> = TextChunk::split(long);
    for (index, chunk) in chunks.iter().enumerate() {
        reader.receive(&Event::AccessibilityAnnouncement {
            client: APP,
            surface: WINDOW,
            index: index as u8,
            last: index + 1 == chunks.len(),
            politeness: Politeness::Assertive,
            text: PreeditText::new(chunk.as_str()).unwrap(),
        });
        if index + 1 < chunks.len() {
            assert!(reader.take_said().is_empty());
        }
    }
    assert_eq!(
        reader.take_said(),
        [Utterance {
            text: long.into(),
            politeness: Politeness::Assertive,
            language: Language::Spanish,
        }]
    );
}

#[test]
fn a_surface_gone_takes_the_cursor_with_it_and_the_next_surface_follows() {
    let mut reader = Reader::new(Language::English);
    feed(&mut reader, &probe(None, "Level 5"));
    let menu = 2;
    feed(
        &mut reader,
        &tree(menu, &[(R::Menu, 0, "", ""), (R::MenuItem, 0, "Copy", "")]),
    );
    assert_eq!(
        command(&mut reader, ReaderCommand::Previous),
        ["Copy, menu item"]
    );
    assert_eq!(
        command(&mut reader, ReaderCommand::Previous),
        ["Borrar, dimmed, button"]
    );
    assert_eq!(
        command(&mut reader, ReaderCommand::Next),
        ["Copy, menu item"]
    );
    reader.receive(&Event::AccessibilityRemoved {
        client: APP,
        surface: menu,
    });
    assert_eq!(reader.cursor(), None);
    // Nothing to activate without a cursor.
    assert_eq!(reader.command(ReaderCommand::Activate), None);
    assert_eq!(
        command(&mut reader, ReaderCommand::Next),
        ["Volumen, slider"]
    );
}

/// sample/App.tsx's menu in its popup: the menu, named by its label, and its rows, `focused` the
/// focused row's index.
fn menu(focused: Option<usize>) -> Vec<Event> {
    let mut nodes = vec![
        (R::Generic, 0, "", ""),
        (R::Menu, 0, "Message actions", ""),
        (R::MenuItem, 0, "Reply", ""),
        (R::MenuItem, 0, "Forward", ""),
        (R::MenuItem, 0, "Archive", ""),
    ];
    if let Some(index) = focused {
        nodes[index].1 |= FOCUSED;
    }
    tree_of(MENU, WINDOW, &nodes)
}

const MENU: u32 = 2;

/// The window behind the menu: its trigger, `focused` or not.
fn trigger(focused: bool, expanded: bool) -> Vec<Event> {
    let mut state = if expanded {
        AccessibleState::EXPANDED
    } else {
        AccessibleState::COLLAPSED
    };
    if focused {
        state |= FOCUSED;
    }
    tree(
        WINDOW,
        &[(R::Generic, 0, "", ""), (R::Button, state, "Open menu", "")],
    )
}

#[test]
fn a_menu_that_opens_is_said_by_its_title_then_its_focused_row() {
    let mut reader = Reader::new(Language::English);
    feed(&mut reader, &trigger(false, false));
    feed(&mut reader, &trigger(false, true));
    // Its title at once, as TalkBack says a popup window's pane title; its focused row after it.
    for event in menu(Some(2)) {
        reader.receive(&event);
    }
    assert_eq!(
        reader.take_said(),
        [
            Utterance {
                text: "Message actions, menu".into(),
                politeness: Politeness::Assertive,
                language: Language::English,
            },
            Utterance {
                text: "Reply, menu item".into(),
                politeness: Politeness::Polite,
                language: Language::English,
            },
        ]
    );
    // The menu's next tree says nothing of it again; a row focused in it is said, at once.
    assert!(feed(&mut reader, &menu(Some(2))).is_empty());
    for event in menu(Some(4)) {
        reader.receive(&event);
    }
    assert_eq!(
        reader.take_said(),
        [Utterance {
            text: "Archive, menu item".into(),
            politeness: Politeness::Assertive,
            language: Language::English,
        }]
    );
    // It closes, focus back on its trigger: the trigger is said, and nothing of the menu.
    reader.receive(&Event::AccessibilityRemoved {
        client: APP,
        surface: MENU,
    });
    assert_eq!(
        feed(&mut reader, &trigger(true, false)),
        ["Open menu, collapsed, button"]
    );
    // Opened again, it is said again.
    assert_eq!(
        feed(&mut reader, &menu(Some(2))),
        ["Message actions, menu", "Reply, menu item"]
    );
}

#[test]
fn only_a_popup_is_said_by_its_title() {
    // A window's first tree, with a menu in it (a menu bar's, drawn in the window), says
    // nothing of the menu: it is no popup appearing.
    let mut reader = Reader::new(Language::English);
    let nodes = [
        (R::Generic, 0, "", ""),
        (R::Menu, 0, "Message actions", ""),
        (R::MenuItem, 0, "Reply", ""),
    ];
    assert!(feed(&mut reader, &tree(WINDOW, &nodes)).is_empty());
    // A popup with no named menu or dialog (a tooltip) is said by nothing but its focus.
    let tip = [(R::Generic, 0, "", ""), (R::Text, 0, "Saved", "")];
    assert!(feed(&mut reader, &tree_of(MENU, WINDOW, &tip)).is_empty());
}

#[test]
fn a_menu_focusing_its_row_after_it_appears_is_said_whole() {
    // Bloom mounts the menu, then focuses its first row: the row follows the title, in the
    // menu's next tree, rather than cutting it off.
    let mut reader = Reader::new(Language::English);
    feed(&mut reader, &trigger(false, true));
    for event in menu(None).into_iter().chain(menu(Some(2))) {
        reader.receive(&event);
    }
    assert_eq!(
        reader.take_said(),
        [
            Utterance {
                text: "Message actions, menu".into(),
                politeness: Politeness::Assertive,
                language: Language::English,
            },
            Utterance {
                text: "Reply, menu item".into(),
                politeness: Politeness::Polite,
                language: Language::English,
            },
        ]
    );
    // The cursor moved by the reader's keys before any focus: a later focus is said at once.
    reader.receive(&Event::AccessibilityRemoved {
        client: APP,
        surface: MENU,
    });
    assert_eq!(feed(&mut reader, &menu(None)), ["Message actions, menu"]);
    assert_eq!(
        command(&mut reader, ReaderCommand::Next),
        ["Open menu, expanded, button"]
    );
    assert_eq!(
        command(&mut reader, ReaderCommand::Next),
        ["Message actions, menu"]
    );
    assert_eq!(
        command(&mut reader, ReaderCommand::Next),
        ["Reply, menu item"]
    );
    for event in menu(Some(3)) {
        reader.receive(&event);
    }
    assert_eq!(
        reader.take_said(),
        [Utterance {
            text: "Forward, menu item".into(),
            politeness: Politeness::Assertive,
            language: Language::English,
        }]
    );
}

/// A message window: a send button and a text field, `focused` the focused one's index (1 or
/// 2), the field's value and its selection in bytes (none: a secure field's).
fn message(focused: usize, value: &str, selection: Option<(u32, u32)>) -> Vec<Event> {
    let mut nodes = vec![
        (R::Generic, 0, "", ""),
        (R::Button, 0, "Enviar", ""),
        (R::TextInput, 0, "Mensaje", value),
    ];
    nodes[focused].1 |= FOCUSED;
    let mut events = tree(WINDOW, &nodes);
    if let Some((start, end)) = selection {
        // As the compositor sends it: after the field's node, before its text.
        let at = events
            .iter()
            .position(|event| matches!(event, Event::AccessibleNode { index: 2, .. }))
            .unwrap();
        events.insert(
            at + 1,
            Event::AccessibleSelection {
                client: APP,
                surface: WINDOW,
                index: 2,
                start,
                end,
            },
        );
    }
    events
}

/// The message field focused, with `value` and its selection.
fn typing(value: &str, start: u32, end: u32) -> Vec<Event> {
    message(2, value, Some((start, end)))
}

/// A reader whose message field is focused with `value`, the caret at `caret`.
fn focused_on(language: Language, value: &str, caret: u32) -> Reader {
    let mut reader = Reader::new(language);
    feed(&mut reader, &message(1, value, Some((caret, caret))));
    feed(&mut reader, &typing(value, caret, caret));
    reader
}

#[test]
fn what_is_typed_is_said_as_it_is_typed() {
    let mut reader = Reader::new(Language::Spanish);
    feed(&mut reader, &message(1, "", Some((0, 0))));
    // Focused, it is said as any element; nothing is echoed.
    assert_eq!(
        feed(&mut reader, &typing("", 0, 0)),
        ["Mensaje, campo de texto"]
    );
    for event in typing("h", 1, 1) {
        reader.receive(&event);
    }
    // At once: each keystroke's echo cuts off the last, as TalkBack's.
    assert_eq!(
        reader.take_said(),
        [Utterance {
            text: "h".into(),
            politeness: Politeness::Assertive,
            language: Language::Spanish,
        }]
    );
    assert_eq!(feed(&mut reader, &typing("ho", 2, 2)), ["o"]);
    assert_eq!(feed(&mut reader, &typing("ho ", 3, 3)), ["espacio"]);
    assert_eq!(feed(&mut reader, &typing("ho ñ", 5, 5)), ["ñ"]);
    // Typed before the end, beside the same letter: the letter typed, where the caret was.
    assert_eq!(feed(&mut reader, &typing("ho ñ", 2, 2)), ["ñ"]);
    assert_eq!(feed(&mut reader, &typing("hoo ñ", 3, 3)), ["o"]);
    // A tree with nothing changed says nothing.
    assert!(feed(&mut reader, &typing("hoo ñ", 3, 3)).is_empty());
    let mut english = focused_on(Language::English, "a", 1);
    assert_eq!(feed(&mut english, &typing("a ", 2, 2)), ["space"]);
    assert_eq!(feed(&mut english, &typing("a \n", 3, 3)), ["new line"]);
}

#[test]
fn what_is_deleted_is_said_deleted() {
    let mut reader = focused_on(Language::Spanish, "hola mundo", 10);
    // Backspace.
    assert_eq!(
        feed(&mut reader, &typing("hola mund", 9, 9)),
        ["o, borrado"]
    );
    // A word of it (Ctrl+Backspace).
    assert_eq!(feed(&mut reader, &typing("hola ", 5, 5)), ["mund, borrado"]);
    assert_eq!(
        feed(&mut reader, &typing("hola", 4, 4)),
        ["espacio, borrado"]
    );
    // Delete, after the caret.
    feed(&mut reader, &typing("hola", 0, 0));
    assert_eq!(feed(&mut reader, &typing("ola", 0, 0)), ["h, borrado"]);
    // A character of more than one byte.
    let mut english = focused_on(Language::English, "año", 3);
    assert_eq!(feed(&mut english, &typing("ao", 1, 1)), ["ñ, deleted"]);
}

#[test]
fn the_caret_says_what_it_passes_over() {
    let mut reader = focused_on(Language::Spanish, "hola mundo", 10);
    // By character, either way.
    assert_eq!(feed(&mut reader, &typing("hola mundo", 9, 9)), ["o"]);
    assert_eq!(feed(&mut reader, &typing("hola mundo", 10, 10)), ["o"]);
    // By word: the word, without the space passed.
    assert_eq!(feed(&mut reader, &typing("hola mundo", 5, 5)), ["mundo"]);
    assert_eq!(feed(&mut reader, &typing("hola mundo", 4, 4)), ["espacio"]);
    assert_eq!(feed(&mut reader, &typing("hola mundo", 0, 0)), ["hola"]);
    assert_eq!(feed(&mut reader, &typing("hola mundo", 5, 5)), ["hola"]);
    // Across lines: the line the caret moved to; over a line break alone, the break.
    let lines = "uno\ndos\n\ntres";
    let mut reader = focused_on(Language::Spanish, lines, 2);
    assert_eq!(feed(&mut reader, &typing(lines, 6, 6)), ["dos"]);
    assert_eq!(feed(&mut reader, &typing(lines, 7, 7)), ["s"]);
    assert_eq!(feed(&mut reader, &typing(lines, 8, 8)), ["nueva línea"]);
    assert_eq!(feed(&mut reader, &typing(lines, 12, 12)), ["tres"]);
    // An empty line.
    assert_eq!(feed(&mut reader, &typing(lines, 8, 8)), ["en blanco"]);
    // An offset inside a character is taken to its start.
    let mut reader = focused_on(Language::English, "ñu", 0);
    assert_eq!(feed(&mut reader, &typing("ñu", 3, 3)), ["ñu"]);
    assert_eq!(feed(&mut reader, &typing("ñu", 1, 1)), ["ñu"]);
}

#[test]
fn a_selection_says_what_it_takes_and_gives_back() {
    let mut reader = focused_on(Language::Spanish, "hola mundo", 5);
    // Shift+Right twice, then back.
    assert_eq!(
        feed(&mut reader, &typing("hola mundo", 5, 6)),
        ["m, seleccionado"]
    );
    assert_eq!(
        feed(&mut reader, &typing("hola mundo", 5, 7)),
        ["u, seleccionado"]
    );
    assert_eq!(
        feed(&mut reader, &typing("hola mundo", 5, 6)),
        ["u, deseleccionado"]
    );
    // An arrow collapses it: what it held is unselected.
    assert_eq!(
        feed(&mut reader, &typing("hola mundo", 6, 6)),
        ["m, deseleccionado"]
    );
    // Shift+Left from the caret, then Shift+Right past where it began.
    assert_eq!(
        feed(&mut reader, &typing("hola mundo", 5, 6)),
        ["m, seleccionado"]
    );
    assert_eq!(
        feed(&mut reader, &typing("hola mundo", 4, 6)),
        ["espacio, seleccionado"]
    );
    assert_eq!(
        feed(&mut reader, &typing("hola mundo", 5, 6)),
        ["espacio, deseleccionado"]
    );
    // A word selected around the caret (a double click): the word.
    feed(&mut reader, &typing("hola mundo", 7, 7));
    assert_eq!(
        feed(&mut reader, &typing("hola mundo", 5, 10)),
        ["mundo, seleccionado"]
    );
    // Select All: the whole text, from a selection or the caret.
    assert_eq!(
        feed(&mut reader, &typing("hola mundo", 0, 10)),
        ["hola mundo, seleccionado"]
    );
    let mut english = focused_on(Language::English, "hello world", 3);
    assert_eq!(
        feed(&mut english, &typing("hello world", 0, 11)),
        ["hello world, selected"]
    );
    assert_eq!(
        feed(&mut english, &typing("hello world", 0, 5)),
        ["world, unselected"]
    );
}

#[test]
fn a_paste_or_what_replaces_the_selection_is_said_whole() {
    let mut reader = focused_on(Language::Spanish, "hola ", 5);
    assert_eq!(
        feed(&mut reader, &typing("hola mundo cruel", 16, 16)),
        ["mundo cruel"]
    );
    // Typed over a selection: what was typed.
    feed(&mut reader, &typing("hola mundo cruel", 5, 16));
    assert_eq!(feed(&mut reader, &typing("hola g", 6, 6)), ["g"]);
    // The selection deleted.
    feed(&mut reader, &typing("hola g", 0, 6));
    assert_eq!(feed(&mut reader, &typing("", 0, 0)), ["hola g, borrado"]);
}

#[test]
fn a_value_the_application_replaces_is_not_echoed() {
    // Unfocused: nothing, whatever changes.
    let mut reader = Reader::new(Language::Spanish);
    feed(&mut reader, &message(1, "hola", Some((4, 4))));
    feed(&mut reader, &message(1, "hola", Some((4, 4))));
    assert!(feed(&mut reader, &message(1, "hola!", Some((5, 5)))).is_empty());
    assert!(feed(&mut reader, &message(1, "hola!", Some((0, 0)))).is_empty());
    assert!(feed(&mut reader, &message(1, "hola!", Some((0, 5)))).is_empty());
    // Focused, a value replaced elsewhere than at the caret: nothing.
    let mut reader = focused_on(Language::Spanish, "hola", 4);
    assert!(feed(&mut reader, &typing("adiós", 6, 6)).is_empty());
    assert!(feed(&mut reader, &typing("adiós amigo", 0, 0)).is_empty());
}

#[test]
fn a_secure_field_says_nothing_of_its_text() {
    // Its tree has no value and no selection: it is said focused, and nothing after.
    let mut reader = Reader::new(Language::English);
    feed(&mut reader, &message(1, "", None));
    assert_eq!(
        feed(&mut reader, &message(2, "", None)),
        ["Mensaje, text field"]
    );
    assert!(feed(&mut reader, &message(2, "", None)).is_empty());
    assert!(feed(&mut reader, &message(2, "", None)).is_empty());
}

#[test]
fn a_field_newly_focused_is_said_and_not_echoed() {
    let mut reader = focused_on(Language::Spanish, "hola", 4);
    // Another element focused while the field changes: that element, and nothing of the field.
    assert_eq!(
        feed(&mut reader, &message(1, "holas", Some((5, 5)))),
        ["Enviar, botón"]
    );
    // The field focused again, changed meanwhile: it is said, not echoed.
    assert_eq!(
        feed(&mut reader, &typing("holas!", 6, 6)),
        ["Mensaje, holas!, campo de texto"]
    );
    assert_eq!(feed(&mut reader, &typing("holas!?", 7, 7)), ["?"]);
}

#[test]
fn the_focused_element_changed_in_place_says_its_new_state_or_value_once() {
    let mut reader = Reader::new(Language::Spanish);
    let checked = AccessibleState::CHECKED;
    let window = |motion: u16, level: &str| {
        tree(
            WINDOW,
            &[
                (R::Generic, 0, "", ""),
                (R::Switch, FOCUSED | motion, "Reducir movimiento", ""),
                (R::Slider, 0, "Volumen", level),
            ],
        )
    };
    assert_eq!(
        feed(&mut reader, &window(0, "5")),
        ["Reducir movimiento, desactivado, interruptor"]
    );
    // Space turns it on: its new state, once, not its whole description again.
    assert_eq!(feed(&mut reader, &window(checked, "5")), ["activado"]);
    assert_eq!(
        feed(&mut reader, &window(checked, "5")),
        Vec::<String>::new()
    );
    // Another element's value changing is not the focused one's: nothing.
    assert_eq!(
        feed(&mut reader, &window(checked, "6")),
        Vec::<String>::new()
    );
    // A slider stepped by the arrows while focused says its new value.
    let slider = |level: &str| {
        tree(
            WINDOW,
            &[
                (R::Generic, 0, "", ""),
                (R::Switch, checked, "Reducir movimiento", ""),
                (R::Slider, FOCUSED, "Volumen", level),
            ],
        )
    };
    assert_eq!(feed(&mut reader, &slider("6")), ["Volumen, 6, regulador"]);
    assert_eq!(feed(&mut reader, &slider("7")), ["7"]);
}

/// Launchpad's search over its icons: the field (`value`, its caret at its end), its clear button
/// before it while it holds text, and Atlas; `focus` names the focused one.
fn launchpad(value: &str, focus: &str) -> Vec<Event> {
    let mut nodes = vec![(R::Generic, 0, "", ""), (R::Button, 0, "Cerrar", "")];
    if !value.is_empty() {
        nodes.push((R::Button, 0, "Borrar la búsqueda", ""));
    }
    let field = nodes.len() as u32;
    nodes.push((R::TextInput, 0, "Buscar", value));
    nodes.push((R::Button, 0, "Atlas", ""));
    for node in &mut nodes {
        if node.2 == focus {
            node.1 |= FOCUSED;
        }
    }
    let mut events = tree(WINDOW, &nodes);
    let at = events
        .iter()
        .position(|event| matches!(event, Event::AccessibleNode { index, .. } if *index == field))
        .unwrap();
    let caret = value.len() as u32;
    events.insert(
        at + 1,
        Event::AccessibleSelection {
            client: APP,
            surface: WINDOW,
            index: field,
            start: caret,
            end: caret,
        },
    );
    events
}

#[test]
fn focus_is_followed_by_the_element_not_by_its_place() {
    let mut reader = Reader::new(Language::Spanish);
    feed(&mut reader, &launchpad("", "Buscar"));
    assert_eq!(feed(&mut reader, &launchpad("", "Atlas")), ["Atlas, botón"]);
    // The field focused again as the letters typed bring its clear button before it: the field
    // now where Atlas was is another element, and is said.
    assert_eq!(
        feed(&mut reader, &launchpad("inb", "Buscar")),
        ["Buscar, inb, campo de texto"]
    );
    // The same field, typed on: its echo.
    assert_eq!(feed(&mut reader, &launchpad("inbo", "Buscar")), ["o"]);
    // Its clear button gone, the field moves back a place: the same element, not said again.
    let said = feed(&mut reader, &launchpad("", "Buscar"));
    assert!(
        said.iter().all(|text| !text.contains("campo de texto")),
        "{said:?}"
    );
}

/// A list of three files with the second focused, each row's place in it and the list's hint
/// added to the tree's events as a version 13 compositor sends them.
fn files(focused: usize) -> Vec<Event> {
    let mut nodes = vec![
        (R::List, 0, "Documentos", ""),
        (R::ListItem, 0, "notas.txt", ""),
        (R::ListItem, 0, "foto.png", ""),
        (R::ListItem, 0, "informe.pdf", ""),
    ];
    nodes[focused].1 |= FOCUSED | AccessibleState::SELECTED;
    let mut events = tree(WINDOW, &nodes);
    let commit = events.pop().unwrap();
    for index in 1..4 {
        events.push(Event::AccessiblePosition {
            client: APP,
            surface: WINDOW,
            index,
            position: index,
            count: 3,
        });
    }
    events.push(Event::AccessibleText {
        client: APP,
        surface: WINDOW,
        index: 2,
        field: AccessibleField::Hint,
        text: TextChunk::new("Abre el archivo").unwrap(),
    });
    events.push(commit);
    events
}

#[test]
fn an_item_says_its_place_and_then_its_hint() {
    let mut reader = Reader::new(Language::Spanish);
    // The focused row: its description with its place, at once; its hint after it.
    assert_eq!(
        feed(&mut reader, &files(1)),
        ["notas.txt, seleccionado, 1 de 3"]
    );
    reader.receive(&files(2)[0]);
    let mut said = Vec::new();
    for event in &files(2)[1..] {
        reader.receive(event);
    }
    said.extend(reader.take_said());
    assert_eq!(
        said,
        [
            Utterance {
                text: "foto.png, seleccionado, 2 de 3".into(),
                politeness: Politeness::Assertive,
                language: Language::Spanish,
            },
            Utterance {
                text: "Abre el archivo".into(),
                politeness: Politeness::Polite,
                language: Language::Spanish,
            },
        ]
    );
    // The reader's own keys say the same, the hint after the place.
    assert_eq!(
        command(&mut reader, ReaderCommand::Next),
        ["informe.pdf, 3 de 3"]
    );
    assert_eq!(
        command(&mut reader, ReaderCommand::Previous),
        ["foto.png, seleccionado, 2 de 3", "Abre el archivo"]
    );
}

/// A window with Dropsea's status bar, a live region, and its list.
fn status(text: &str) -> Vec<Event> {
    tree(
        WINDOW,
        &[
            (R::Generic, 0, "", ""),
            (R::List, 0, "Documents", ""),
            (R::Text, AccessibleState::LIVE_POLITE, text, ""),
        ],
    )
}

/// The same surface while a modal is shown: its tree is the modal's alone.
fn quick_look(focused: bool) -> Vec<Event> {
    tree(
        WINDOW,
        &[
            (R::Dialog, 0, "Quick Look", ""),
            (R::Text, 0, "notes.txt", ""),
            (R::Button, if focused { FOCUSED } else { 0 }, "Close", ""),
        ],
    )
}

#[test]
fn a_live_region_shown_again_unchanged_says_nothing() {
    let mut reader = Reader::new(Language::English);
    assert!(feed(&mut reader, &status("1 of 3 selected")).is_empty());
    assert_eq!(
        feed(&mut reader, &status("2 of 3 selected")),
        ["2 of 3 selected"]
    );
    // A modal over the window: the window's live region is not in the tree meanwhile.
    feed(&mut reader, &quick_look(false));
    // Back unchanged: nothing said; changed while hidden: its new text.
    assert!(feed(&mut reader, &status("2 of 3 selected")).is_empty());
    feed(&mut reader, &quick_look(false));
    assert_eq!(
        feed(&mut reader, &status("3 of 3 selected")),
        ["3 of 3 selected"]
    );
}

#[test]
fn a_dialog_shown_over_a_window_is_said_by_its_title_as_it_appears() {
    let mut reader = Reader::new(Language::English);
    feed(&mut reader, &status("1 of 3 selected"));
    // Nothing focused in it: its title alone, once.
    assert_eq!(
        feed(&mut reader, &quick_look(false)),
        ["Quick Look, dialog"]
    );
    assert!(feed(&mut reader, &quick_look(false)).is_empty());
    feed(&mut reader, &status("1 of 3 selected"));
    // A control focused in it: the title, then the control after it, politely.
    for event in &quick_look(true) {
        reader.receive(event);
    }
    assert_eq!(
        reader.take_said(),
        [
            Utterance {
                text: "Quick Look, dialog".into(),
                politeness: Politeness::Assertive,
                language: Language::English,
            },
            Utterance {
                text: "Close, button".into(),
                politeness: Politeness::Polite,
                language: Language::English,
            },
        ]
    );
}

/// Settings' trial of a display mode over its window: an alert with its message as its hint, and
/// its default button focused.
fn keep_displays(seconds: u32) -> Vec<Event> {
    let mut events = tree(
        WINDOW,
        &[
            (
                R::Dialog,
                0,
                "¿Mantener esta configuración de pantalla?",
                "",
            ),
            (R::Button, 0, "Mantener", ""),
            (R::Button, FOCUSED, "Revertir", ""),
        ],
    );
    let commit = events.pop().unwrap();
    let message = format!("La pantalla vuelve a su configuración anterior en {seconds} segundos.");
    for text in TextChunk::split(&message) {
        events.push(Event::AccessibleText {
            client: APP,
            surface: WINDOW,
            index: 0,
            field: AccessibleField::Hint,
            text,
        });
    }
    events.push(commit);
    events
}

#[test]
fn an_alert_says_its_title_then_its_message_then_its_default_button() {
    let mut reader = Reader::new(Language::Spanish);
    feed(&mut reader, &status("1 of 3 selected"));
    for event in &keep_displays(15) {
        reader.receive(event);
    }
    let polite = |text: &str| Utterance {
        text: text.into(),
        politeness: Politeness::Polite,
        language: Language::Spanish,
    };
    assert_eq!(
        reader.take_said(),
        [
            Utterance {
                text: "¿Mantener esta configuración de pantalla?, diálogo".into(),
                politeness: Politeness::Assertive,
                language: Language::Spanish
            },
            polite("La pantalla vuelve a su configuración anterior en 15 segundos."),
            polite("Revertir, botón"),
        ]
    );
    // Its message counting down in place says nothing: the dialog was said, and its hint is no
    // live region.
    assert!(feed(&mut reader, &keep_displays(14)).is_empty());
}

#[test]
fn a_surface_that_appears_as_a_dialog_is_said_by_its_title_then_its_focus() {
    const LAUNCHPAD: u32 = 9;
    let mut reader = Reader::new(Language::Spanish);
    feed(&mut reader, &tree(WINDOW, &[(R::TextInput, 0, "Line", "")]));
    let launchpad = |focused: u16, search: &str| {
        tree(
            LAUNCHPAD,
            &[
                (R::Dialog, 0, "Launchpad", ""),
                (R::Search, focused, "Buscar aplicaciones", search),
                (R::Button, 0, "Allo", ""),
            ],
        )
    };
    // It appears with its search focused: its title, then the search after it, politely.
    for event in &launchpad(FOCUSED, "") {
        reader.receive(event);
    }
    assert_eq!(
        reader.take_said(),
        [
            Utterance {
                text: "Launchpad, diálogo".into(),
                politeness: Politeness::Assertive,
                language: Language::Spanish
            },
            Utterance {
                text: "Buscar aplicaciones, campo de búsqueda".into(),
                politeness: Politeness::Polite,
                language: Language::Spanish
            },
        ]
    );
    // A folder opened over it, modal (its tree the folder's alone), then closed: Launchpad is
    // shown again, not appearing, and says nothing of its title.
    feed(
        &mut reader,
        &tree(
            LAUNCHPAD,
            &[
                (R::Heading, 0, "Utilidades", ""),
                (R::Button, FOCUSED, "Notas", ""),
            ],
        ),
    );
    assert!(!feed(&mut reader, &launchpad(0, ""))
        .iter()
        .any(|said| said.starts_with("Launchpad")));
    // Gone and opened again: said again.
    reader.receive(&Event::AccessibilityRemoved {
        client: APP,
        surface: LAUNCHPAD,
    });
    assert_eq!(
        feed(&mut reader, &launchpad(FOCUSED, "")),
        [
            "Launchpad, diálogo",
            "Buscar aplicaciones, campo de búsqueda"
        ]
    );
    // A system sheet: its title, then its message (the dialog's hint) politely; the next sheet in
    // its place, saying what went wrong, is said as it comes.
    const SHEET: u32 = 12;
    let sheet = |message: &str| {
        let mut events = tree(
            SHEET,
            &[
                (R::Dialog, 0, "Unirse a Vecinos", ""),
                (R::Button, 0, "Cancelar", ""),
            ],
        );
        let commit = events.pop().unwrap();
        for text in TextChunk::split(message) {
            events.push(Event::AccessibleText {
                client: APP,
                surface: SHEET,
                index: 0,
                field: AccessibleField::Hint,
                text,
            });
        }
        events.push(commit);
        events
    };
    for event in &sheet("Escribe la contraseña.") {
        reader.receive(event);
    }
    assert_eq!(
        reader.take_said(),
        [
            Utterance {
                text: "Unirse a Vecinos, diálogo".into(),
                politeness: Politeness::Assertive,
                language: Language::Spanish
            },
            Utterance {
                text: "Escribe la contraseña.".into(),
                politeness: Politeness::Polite,
                language: Language::Spanish
            },
        ]
    );
    assert!(feed(&mut reader, &sheet("Escribe la contraseña.")).is_empty());
    assert_eq!(
        feed(&mut reader, &sheet("Contraseña incorrecta.")),
        ["Unirse a Vecinos, diálogo", "Contraseña incorrecta."]
    );
    // A window's first tree whose root is no dialog says no title.
    assert!(feed(
        &mut reader,
        &tree(3, &[(R::Generic, 0, "", ""), (R::Dialog, 0, "Ajustes", "")])
    )
    .is_empty());
}

#[test]
fn a_dialog_closed_with_nothing_focused_leaves_the_next_focus_said_at_once() {
    let mut reader = Reader::new(Language::English);
    feed(&mut reader, &status("1 of 3 selected"));
    assert_eq!(
        feed(&mut reader, &quick_look(false)),
        ["Quick Look, dialog"]
    );
    // Closed: the list focused again is said at once, not as if after the dialog's title.
    let mut back = tree(
        WINDOW,
        &[
            (R::Generic, 0, "", ""),
            (R::List, FOCUSED, "Documents", ""),
            (R::Text, AccessibleState::LIVE_POLITE, "1 of 3 selected", ""),
        ],
    );
    for event in back.drain(..) {
        reader.receive(&event);
    }
    assert_eq!(
        reader.take_said(),
        [Utterance {
            text: "Documents, list".into(),
            politeness: Politeness::Assertive,
            language: Language::English,
        }]
    );
}

#[test]
fn the_session_locking_is_said_then_the_lock_screens_field() {
    let mut reader = Reader::new(Language::Spanish);
    // Connected to an unlocked session: nothing said of it.
    reader.receive(&Event::LockState { locked: false });
    assert!(reader.take_said().is_empty());
    reader.receive(&Event::LockState { locked: true });
    for event in &tree_of(9, 0, &[(R::TextInput, FOCUSED, "Contraseña", "")]) {
        reader.receive(event);
    }
    assert_eq!(
        reader.take_said(),
        [
            Utterance {
                text: "Pantalla bloqueada".into(),
                politeness: Politeness::Assertive,
                language: Language::Spanish,
            },
            Utterance {
                text: "Contraseña, campo de texto".into(),
                politeness: Politeness::Polite,
                language: Language::Spanish,
            },
        ]
    );
    // Connected to a session already locked (the login screen): nothing said of it.
    let mut login = Reader::new(Language::Spanish);
    login.receive(&Event::LockState { locked: true });
    assert!(login.take_said().is_empty());
}

/// `tree`'s events with each node's parent as given (`NO_ACCESSIBLE_PARENT` for the root).
fn with_parents(mut events: Vec<Event>, parents: &[u32]) -> Vec<Event> {
    for event in &mut events {
        if let Event::AccessibleNode { index, parent, .. } = event {
            *parent = parents[*index as usize];
        }
    }
    events
}

/// A form: its field, focused, and under it the error, while there is one (a Bloom `Field`'s
/// error is an `alert`: an assertive live region).
fn form(error: Option<&str>) -> Vec<Event> {
    let mut nodes = vec![
        (R::Generic, 0, "", ""),
        (R::TextInput, FOCUSED, "Nombre de usuario", "ada"),
    ];
    if let Some(error) = error {
        nodes.push((R::Alert, AccessibleState::LIVE_ASSERTIVE, error, ""));
    }
    nodes.push((R::Button, 0, "Comprobar", ""));
    tree(WINDOW, &nodes)
}

#[test]
fn an_alert_appearing_is_said_at_once_and_once() {
    let mut reader = Reader::new(Language::Spanish);
    // The field focused, as the window shows.
    assert_eq!(
        feed(&mut reader, &form(None)),
        ["Nombre de usuario, ada, campo de texto"]
    );
    for event in &form(Some("Ese nombre ya está en uso")) {
        reader.receive(event);
    }
    assert_eq!(
        reader.take_said(),
        [Utterance {
            text: "Ese nombre ya está en uso".into(),
            politeness: Politeness::Assertive,
            language: Language::Spanish
        }]
    );
    assert!(feed(&mut reader, &form(Some("Ese nombre ya está en uso"))).is_empty());
    assert!(feed(&mut reader, &form(None)).is_empty());
    // Shown again: said again, as it is an answer again.
    assert_eq!(
        feed(&mut reader, &form(Some("Ese nombre ya está en uso"))),
        ["Ese nombre ya está en uso"]
    );
}

/// A window with a toast's row while `toast` holds its title and its text: a live container (no
/// words of its own) holding them and its close button.
fn toasts(toast: &[(&str, &str)]) -> Vec<Event> {
    let mut nodes = vec![(R::Generic, 0, "", ""), (R::Button, 0, "Abrir", "")];
    let mut parents = vec![NO_ACCESSIBLE_PARENT, 0];
    for (title, text) in toast {
        let row = nodes.len() as u32;
        nodes.push((R::Generic, AccessibleState::LIVE_POLITE, "", ""));
        nodes.push((R::Text, 0, title, ""));
        nodes.push((R::Text, 0, text, ""));
        nodes.push((R::Button, 0, "Cerrar", ""));
        parents.extend([0, row, row, row]);
    }
    with_parents(tree(WINDOW, &nodes), &parents)
}

#[test]
fn a_live_container_says_the_text_it_holds_not_its_controls() {
    let mut reader = Reader::new(Language::Spanish);
    assert!(feed(&mut reader, &toasts(&[])).is_empty());
    for event in &toasts(&[("Guardado", "Los cambios se han guardado")]) {
        reader.receive(event);
    }
    assert_eq!(
        reader.take_said(),
        [Utterance {
            text: "Guardado, Los cambios se han guardado".into(),
            politeness: Politeness::Polite,
            language: Language::Spanish
        }]
    );
    // Another above it: that one alone, not the one already said.
    assert_eq!(
        feed(
            &mut reader,
            &toasts(&[
                ("Copiado", "3 archivos"),
                ("Guardado", "Los cambios se han guardado")
            ])
        ),
        ["Copiado, 3 archivos"]
    );
    assert!(feed(&mut reader, &toasts(&[("Copiado", "3 archivos")])).is_empty());
}

/// A window with no root of its own, as the Bloom host's: a toast's row first, while there is
/// one, then a label, a status line and a button, each a root.
fn rootless(toast: bool, saved: &str) -> Vec<Event> {
    let mut nodes = Vec::new();
    let mut parents = Vec::new();
    if toast {
        nodes.extend([
            (R::Generic, AccessibleState::LIVE_POLITE, "", ""),
            (R::Text, 0, "Copiado", ""),
            (R::Button, 0, "Cerrar", ""),
        ]);
        parents.extend([NO_ACCESSIBLE_PARENT, 0, 0]);
    }
    nodes.extend([
        (R::Text, 0, "Nombre de usuario", ""),
        (R::Generic, AccessibleState::LIVE_POLITE, saved, ""),
        (R::Button, 0, "Guardar", ""),
    ]);
    parents.extend([NO_ACCESSIBLE_PARENT; 3]);
    with_parents(tree(WINDOW, &nodes), &parents)
}

#[test]
fn a_toast_first_in_the_window_is_said_alone() {
    let mut reader = Reader::new(Language::Spanish);
    assert!(feed(&mut reader, &rootless(false, "0 cambios guardados")).is_empty());
    assert_eq!(
        feed(&mut reader, &rootless(true, "0 cambios guardados")),
        ["Copiado"]
    );
    assert_eq!(
        feed(&mut reader, &rootless(false, "1 cambio guardado")),
        ["1 cambio guardado"]
    );
}

#[test]
fn a_polite_live_region_is_said_after_a_new_focus_and_an_assertive_one_before() {
    let mut reader = Reader::new(Language::Spanish);
    let window = |search: u16, allo: u16, count: &str, alert: &str| {
        tree(
            WINDOW,
            &[
                (R::Search, search, "Buscar aplicaciones", "l"),
                (R::Button, allo, "Allo", ""),
                (R::Text, AccessibleState::LIVE_POLITE, count, ""),
                (R::Text, AccessibleState::LIVE_ASSERTIVE, alert, ""),
            ],
        )
    };
    feed(&mut reader, &window(0, FOCUSED, "2 resultados", ""));
    // The search focused again as its count changes: the search said, then the count after it,
    // politely (said before, the focus would cut it off).
    for event in &window(FOCUSED, 0, "1 resultado", "") {
        reader.receive(event);
    }
    assert_eq!(
        reader.take_said(),
        [
            Utterance {
                text: "Buscar aplicaciones, l, campo de búsqueda".into(),
                politeness: Politeness::Assertive,
                language: Language::Spanish
            },
            Utterance {
                text: "1 resultado".into(),
                politeness: Politeness::Polite,
                language: Language::Spanish
            },
        ]
    );
    // An alert with a new focus: the alert first, at once, then the focus.
    assert_eq!(
        feed(
            &mut reader,
            &window(0, FOCUSED, "1 resultado", "Sin conexión")
        ),
        ["Sin conexión", "Allo, botón"]
    );
}
