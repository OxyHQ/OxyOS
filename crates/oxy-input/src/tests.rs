use super::*;

/// Linux input codes (input-event-codes.h).
const SHIFT: u32 = 42;
const A: u32 = 30;
const ONE: u32 = 2;
const SPACE: u32 = 57;
const ENTER: u32 = 28;
const BACKSPACE: u32 = 14;
const SEMICOLON: u32 = 39;

fn tap(keyboard: &mut Keyboard, code: u32) -> Option<String> {
    let text = keyboard.key(code, true);
    assert_eq!(keyboard.key(code, false), None, "a release types nothing");
    text
}

#[test]
fn keys_type_their_layouts_text() {
    let mut us = Keyboard::new("us").unwrap();
    assert_eq!(tap(&mut us, A).as_deref(), Some("a"));
    assert_eq!(tap(&mut us, SPACE).as_deref(), Some(" "));
    us.key(SHIFT, true);
    assert_eq!(tap(&mut us, A).as_deref(), Some("A"));
    assert_eq!(tap(&mut us, ONE).as_deref(), Some("!"));
    us.key(SHIFT, false);
    assert_eq!(tap(&mut us, ONE).as_deref(), Some("1"));
    // The same key on another layout.
    let mut es = Keyboard::new("es").unwrap();
    assert_eq!(tap(&mut es, SEMICOLON).as_deref(), Some("ñ"));
}

#[test]
fn modifiers_and_editing_keys_type_nothing() {
    let mut us = Keyboard::new("us").unwrap();
    assert_eq!(us.key(SHIFT, true), None);
    assert_eq!(us.key(SHIFT, false), None);
    // Enter and Backspace are keys, not text (control characters).
    assert_eq!(tap(&mut us, ENTER), None);
    assert_eq!(tap(&mut us, BACKSPACE), None);
}

#[test]
fn characters_map_back_to_their_keys() {
    let es = Keyboard::new("es").unwrap();
    assert_eq!(es.key_for('a'), Some((A, false)));
    assert_eq!(es.key_for('H'), Some((35, true)));
    assert_eq!(es.key_for('!'), Some((ONE, true)));
    assert_eq!(es.key_for('ñ'), Some((SEMICOLON, false)));
    assert_eq!(es.key_for(' '), Some((SPACE, false)));
    // A dead key and a vowel, not one key.
    assert_eq!(es.key_for('ó'), None);
    // Typing what a key is found for types that character.
    let mut typing = Keyboard::new("es").unwrap();
    for character in "Hola OxyOS!¡".chars() {
        let (code, shifted) = es.key_for(character).unwrap();
        if shifted {
            typing.key(SHIFT, true);
        }
        assert_eq!(tap(&mut typing, code), Some(character.to_string()));
        if shifted {
            typing.key(SHIFT, false);
        }
    }
}
