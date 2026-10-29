use super::*;

/// Linux input codes (input-event-codes.h).
const SHIFT: u32 = 42;
const A: u32 = 30;
const O: u32 = 24;
const X: u32 = 45;
const ONE: u32 = 2;
const SPACE: u32 = 57;
const ENTER: u32 = 28;
const BACKSPACE: u32 = 14;
const SEMICOLON: u32 = 39;
/// The key right of Ñ on the Spanish layout: the dead acute (´).
const APOSTROPHE: u32 = 40;

/// The keymap of `layout` with the en_US.UTF-8 compose table (the image's, under `config/`, when
/// `XLOCALEDIR` names them, as CI does), which composes
/// Spanish dead keys as es_ES.UTF-8's does.
fn composing(layout: &str) -> Keyboard {
    let keyboard = Keyboard::with_locale(layout, OsStr::new("en_US.UTF-8")).unwrap();
    assert!(keyboard.composes(), "no compose table: set XLOCALEDIR to the image's (config/includes.chroot_after_packages/usr/share/X11/locale)");
    keyboard
}

/// The text a press and release of `code` types, through composition or not.
fn tap(keyboard: &mut Keyboard, code: u32) -> Option<String> {
    let typed = keyboard.key(code, true);
    let released = keyboard.key(code, false);
    match typed {
        Typed::Key(text) => {
            assert_eq!(released, Typed::Key(None), "a release types nothing");
            text
        }
        Typed::Compose { text, .. } => {
            assert_eq!(
                released,
                Typed::Nothing,
                "a taken key's release is nobody's"
            );
            text
        }
        Typed::SwitchTerminal(number) => panic!("a terminal switch to {number}, not text"),
        Typed::Terminate => panic!("the end of the session, not text"),
        Typed::Lock => panic!("the lock, not text"),
        Typed::Nothing => panic!("a press is always someone's"),
    }
}

fn press(keyboard: &mut Keyboard, code: u32) -> Typed {
    let typed = keyboard.key(code, true);
    keyboard.key(code, false);
    typed
}

/// Types what `key_for` returns for `character`, as a person would.
fn type_with(keyboard: &mut Keyboard, strokes: &[Stroke]) -> Option<String> {
    let mut text = None;
    for &(code, shifted) in strokes {
        if shifted {
            keyboard.key(SHIFT, true);
        }
        text = tap(keyboard, code);
        if shifted {
            keyboard.key(SHIFT, false);
        }
    }
    text
}

#[test]
fn keys_type_their_layouts_text() {
    let mut us = composing("us");
    assert_eq!(tap(&mut us, A).as_deref(), Some("a"));
    assert_eq!(tap(&mut us, SPACE).as_deref(), Some(" "));
    us.key(SHIFT, true);
    assert_eq!(tap(&mut us, A).as_deref(), Some("A"));
    assert_eq!(tap(&mut us, ONE).as_deref(), Some("!"));
    us.key(SHIFT, false);
    assert_eq!(tap(&mut us, ONE).as_deref(), Some("1"));
    // The same key on another layout.
    let mut es = composing("es");
    assert_eq!(tap(&mut es, SEMICOLON).as_deref(), Some("ñ"));
}

#[test]
fn modifiers_and_editing_keys_type_nothing() {
    let mut us = composing("us");
    assert_eq!(us.key(SHIFT, true), Typed::Key(None));
    assert_eq!(us.key(SHIFT, false), Typed::Key(None));
    // Enter and Backspace are keys, not text (control characters).
    assert_eq!(tap(&mut us, ENTER), None);
    assert_eq!(tap(&mut us, BACKSPACE), None);
}

#[test]
fn a_dead_key_composes_with_the_next_key() {
    let mut es = composing("es");
    // The dead key types nothing; the vowel types the composed character.
    assert_eq!(tap(&mut es, APOSTROPHE), None);
    assert_eq!(tap(&mut es, O).as_deref(), Some("ó"));
    // Composition is over: the vowel alone again.
    assert_eq!(tap(&mut es, O).as_deref(), Some("o"));
    // Shift belongs to the composed key.
    assert_eq!(tap(&mut es, APOSTROPHE), None);
    es.key(SHIFT, true);
    assert_eq!(tap(&mut es, O).as_deref(), Some("Ó"));
    es.key(SHIFT, false);
    // Two dead acutes compose the acute itself.
    assert_eq!(tap(&mut es, APOSTROPHE), None);
    assert_eq!(tap(&mut es, APOSTROPHE).as_deref(), Some("´"));
}

#[test]
fn a_sequence_that_composes_nothing_types_nothing() {
    let mut es = composing("es");
    assert_eq!(tap(&mut es, APOSTROPHE), None);
    // ´ and x compose nothing: the sequence is cancelled, and x is not typed with it.
    assert_eq!(tap(&mut es, X), None);
    // The keyboard is back to plain keys.
    assert_eq!(tap(&mut es, X).as_deref(), Some("x"));
}

#[test]
fn without_a_compose_table_dead_keys_type_nothing() {
    // A locale with no compose table anywhere.
    let mut es = Keyboard::with_locale("es", OsStr::new("xx_XX.UTF-8")).unwrap();
    assert!(!es.composes());
    assert_eq!(tap(&mut es, APOSTROPHE), None);
    assert_eq!(tap(&mut es, O).as_deref(), Some("o"));
    assert_eq!(es.key_for('ó'), None);
}

#[test]
fn characters_map_back_to_their_keys() {
    let es = composing("es");
    assert_eq!(es.key_for('a'), Some(vec![(A, false)]));
    assert_eq!(es.key_for('H'), Some(vec![(35, true)]));
    assert_eq!(es.key_for('!'), Some(vec![(ONE, true)]));
    assert_eq!(es.key_for('ñ'), Some(vec![(SEMICOLON, false)]));
    assert_eq!(es.key_for(' '), Some(vec![(SPACE, false)]));
    // A dead key and a vowel.
    assert_eq!(es.key_for('ó'), Some(vec![(APOSTROPHE, false), (O, false)]));
    assert_eq!(es.key_for('Ó'), Some(vec![(APOSTROPHE, false), (O, true)]));
    // Nothing on the layout types it.
    assert_eq!(es.key_for('ß'), None);
    // Typing what is found for each character types that character.
    let mut typing = composing("es");
    for character in "Hola OxyOS!¡Adiós Ñandú".chars() {
        let strokes = es.key_for(character).unwrap();
        assert_eq!(
            type_with(&mut typing, &strokes),
            Some(character.to_string()),
            "{character:?} from {strokes:?}"
        );
    }
}

#[test]
fn finding_keys_leaves_a_composition_under_way() {
    let mut es = composing("es");
    assert_eq!(tap(&mut es, APOSTROPHE), None);
    assert!(es.key_for('ó').is_some());
    assert_eq!(tap(&mut es, O).as_deref(), Some("ó"));
}

#[test]
fn a_composition_shows_its_sequence_and_its_keys_are_no_applications() {
    let mut es = composing("es");
    let compose = |composition: &str, text: Option<&str>| Typed::Compose {
        composition: composition.to_string(),
        text: text.map(str::to_string),
    };
    // The dead key shows as its accent; the vowel ends the composition with what it composed.
    assert_eq!(press(&mut es, APOSTROPHE), compose("´", None));
    assert_eq!(press(&mut es, O), compose("", Some("ó")));
    // A cancelled sequence ends the composition with nothing.
    assert_eq!(press(&mut es, APOSTROPHE), compose("´", None));
    assert_eq!(press(&mut es, X), compose("", None));
    // Shift held for a capital stays the application's key; the vowel is the table's.
    assert_eq!(press(&mut es, APOSTROPHE), compose("´", None));
    assert_eq!(es.key(SHIFT, true), Typed::Key(None));
    assert_eq!(es.key(O, true), compose("", Some("Ó")));
    assert_eq!(es.key(SHIFT, false), Typed::Key(None));
    assert_eq!(es.key(O, false), Typed::Nothing);
    // Plain keys are the application's again, press and release.
    assert_eq!(es.key(O, true), Typed::Key(Some("o".to_string())));
    assert_eq!(es.key(O, false), Typed::Key(None));
}

#[test]
fn ctrl_alt_and_a_function_key_switch_terminals_and_are_no_applications_key() {
    const CTRL: u32 = 29;
    const ALT: u32 = 56;
    const F2: u32 = 60;
    let mut us = composing("us");
    // The modifiers stay the application's keys; the function key is the switch.
    assert_eq!(us.key(CTRL, true), Typed::Key(None));
    assert_eq!(us.key(ALT, true), Typed::Key(None));
    assert_eq!(us.key(F2, true), Typed::SwitchTerminal(2));
    assert_eq!(us.key(F2, false), Typed::Nothing);
    assert_eq!(us.key(ALT, false), Typed::Key(None));
    assert_eq!(us.key(CTRL, false), Typed::Key(None));
    // Alone, the function key is a key.
    assert_eq!(us.key(F2, true), Typed::Key(None));
    assert_eq!(us.key(F2, false), Typed::Key(None));
}

#[test]
fn ctrl_alt_backspace_ends_the_session_and_is_no_applications_key() {
    const CTRL: u32 = 29;
    const ALT: u32 = 56;
    let mut us = composing("us");
    assert_eq!(us.key(CTRL, true), Typed::Key(None));
    assert_eq!(us.key(ALT, true), Typed::Key(None));
    assert_eq!(us.key(BACKSPACE, true), Typed::Terminate);
    assert_eq!(us.key(BACKSPACE, false), Typed::Nothing);
    assert_eq!(us.key(ALT, false), Typed::Key(None));
    assert_eq!(us.key(CTRL, false), Typed::Key(None));
    // Alone, Backspace is a key.
    assert_eq!(tap(&mut us, BACKSPACE), None);
}

#[test]
fn ctrl_super_q_locks_and_is_no_applications_key() {
    const CTRL: u32 = 29;
    const SUPER: u32 = 125;
    const Q: u32 = 16;
    let mut us = composing("us");
    assert_eq!(us.key(CTRL, true), Typed::Key(None));
    assert_eq!(us.key(SUPER, true), Typed::Key(None));
    assert_eq!(us.key(Q, true), Typed::Lock);
    assert_eq!(us.key(Q, false), Typed::Nothing);
    assert_eq!(us.key(SUPER, false), Typed::Key(None));
    // Ctrl+Q alone is the application's (quit), as is Super+Q.
    assert_eq!(us.key(Q, true), Typed::Key(None));
    assert_eq!(us.key(Q, false), Typed::Key(None));
    assert_eq!(us.key(CTRL, false), Typed::Key(None));
    assert_eq!(us.key(SUPER, true), Typed::Key(None));
    assert_eq!(us.key(Q, true), Typed::Key(Some("q".into())));
    assert_eq!(us.key(Q, false), Typed::Key(None));
    assert_eq!(us.key(SUPER, false), Typed::Key(None));
    // With Shift as well, it is not the lock.
    assert_eq!(us.key(CTRL, true), Typed::Key(None));
    assert_eq!(us.key(SUPER, true), Typed::Key(None));
    assert_eq!(us.key(SHIFT, true), Typed::Key(None));
    assert!(matches!(us.key(Q, true), Typed::Key(_)));
    // It is the key that types q: on AZERTY, US A's key, not US Q's (which types a).
    let mut fr = composing("fr");
    fr.key(CTRL, true);
    fr.key(SUPER, true);
    assert_eq!(fr.key(Q, true), Typed::Key(None));
    assert_eq!(fr.key(Q, false), Typed::Key(None));
    assert_eq!(fr.key(A, true), Typed::Lock);
}

#[test]
fn caps_lock_locks_on_its_press_and_unlocks_on_the_next() {
    const CAPS_LOCK: u32 = 58;
    let mut us = composing("us");
    assert!(!us.caps_locked());
    assert_eq!(us.key(CAPS_LOCK, true), Typed::Key(None));
    assert_eq!(us.key(CAPS_LOCK, false), Typed::Key(None));
    assert!(us.caps_locked());
    assert_eq!(tap(&mut us, A), Some("A".into()));
    us.key(CAPS_LOCK, true);
    us.key(CAPS_LOCK, false);
    assert!(!us.caps_locked());
    assert_eq!(tap(&mut us, A), Some("a".into()));
}
