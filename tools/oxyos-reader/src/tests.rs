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
            politeness: Politeness::Assertive
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
            politeness: Politeness::Polite
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
            politeness: Politeness::Assertive
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
                politeness: Politeness::Assertive
            },
            Utterance {
                text: "Reply, menu item".into(),
                politeness: Politeness::Polite
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
            politeness: Politeness::Assertive
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
                politeness: Politeness::Assertive
            },
            Utterance {
                text: "Reply, menu item".into(),
                politeness: Politeness::Polite
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
            politeness: Politeness::Assertive
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
            politeness: Politeness::Assertive
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
