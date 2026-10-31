use super::*;

fn state(bits: u16) -> AccessibleState {
    AccessibleState::new(bits).unwrap()
}

#[test]
fn the_language_is_the_sessions_spanish_or_english() {
    for locale in ["es_ES.UTF-8", "es", "es-MX", "ES_ar", "es_ES@euro"] {
        assert_eq!(Language::of_locale(locale), Language::Spanish, "{locale}");
    }
    for locale in ["en_US.UTF-8", "C", "POSIX", "", "fr_FR.UTF-8", "est_EE"] {
        assert_eq!(Language::of_locale(locale), Language::English, "{locale}");
    }
    assert_eq!(Language::Spanish.voice(), "es");
    assert_eq!(Language::English.voice(), "en");
}

#[test]
fn an_element_is_said_by_its_name_value_state_and_role() {
    use AccessibleRole as R;
    let cases = [
        (
            R::Button,
            0,
            "Guardar",
            "",
            "Guardar, botón",
            "Guardar, button",
        ),
        (
            R::TextInput,
            0,
            "Nota",
            "hola ñandú",
            "Nota, hola ñandú, campo de texto",
            "Nota, hola ñandú, text field",
        ),
        (
            R::CheckBox,
            0,
            "Recordar",
            "",
            "Recordar, no marcada, casilla",
            "Recordar, unchecked, checkbox",
        ),
        (
            R::CheckBox,
            AccessibleState::CHECKED,
            "Recordar",
            "",
            "Recordar, marcada, casilla",
            "Recordar, checked, checkbox",
        ),
        (
            R::Switch,
            AccessibleState::CHECKED,
            "Wi-Fi",
            "",
            "Wi-Fi, activado, interruptor",
            "Wi-Fi, on, switch",
        ),
        (
            R::Button,
            AccessibleState::DISABLED,
            "Borrar",
            "",
            "Borrar, atenuado, botón",
            "Borrar, dimmed, button",
        ),
        (
            R::MenuItem,
            AccessibleState::COLLAPSED | AccessibleState::FOCUSED,
            "Abrir",
            "",
            "Abrir, contraído, elemento de menú",
            "Abrir, collapsed, menu item",
        ),
        (
            R::Tab,
            AccessibleState::SELECTED,
            "Inicio",
            "",
            "Inicio, seleccionado, pestaña",
            "Inicio, selected, tab",
        ),
        (
            R::Slider,
            0,
            "Volumen",
            "5",
            "Volumen, 5, regulador",
            "Volumen, 5, slider",
        ),
        // Plain text is its text; a value equal to the name is said once.
        (
            R::Text,
            AccessibleState::LIVE_POLITE,
            "Nivel 5",
            "Nivel 5",
            "Nivel 5",
            "Nivel 5",
        ),
        (
            R::Heading,
            0,
            " Ajustes ",
            "",
            "Ajustes, encabezado",
            "Ajustes, heading",
        ),
        // An image with no name is still an image; a container with nothing says nothing.
        (R::Image, 0, "", "", "imagen", "image"),
        (R::Generic, 0, "", "", "", ""),
        (R::Group, AccessibleState::DISABLED, "", "", "", ""),
    ];
    for (role, bits, name, value, spanish, english) in cases {
        let said = |language| describe(language, role, state(bits), name, value);
        assert_eq!(said(Language::Spanish), spanish);
        assert_eq!(said(Language::English), english);
    }
}

#[test]
fn a_text_change_is_said_with_what_happened_to_it() {
    use TextChange as C;
    let s = |text: &str| text.to_string();
    let cases = [
        (C::Inserted(s("a")), "a", "a"),
        (C::Inserted(s(" ")), "espacio", "space"),
        (C::Inserted(s("\n")), "nueva línea", "new line"),
        (C::Inserted(s("hola mundo")), "hola mundo", "hola mundo"),
        (C::Deleted(s("ñ")), "ñ, borrado", "ñ, deleted"),
        (C::Deleted(s(" ")), "espacio, borrado", "space, deleted"),
        (C::Traversed(s("\t")), "tabulador", "tab"),
        (C::Traversed(s("mundo")), "mundo", "mundo"),
        (C::Line(s("segunda")), "segunda", "segunda"),
        (C::Line(s("")), "en blanco", "blank"),
        (C::Selected(s("ab")), "ab, seleccionado", "ab, selected"),
        (C::Unselected(s("b")), "b, deseleccionado", "b, unselected"),
        (C::Inserted(s("")), "", ""),
        (C::Selected(s("")), "", ""),
    ];
    for (change, spanish, english) in cases {
        assert_eq!(describe_change(Language::Spanish, &change), spanish);
        assert_eq!(describe_change(Language::English, &change), english);
    }
}

#[test]
fn polite_waits_its_turn_and_assertive_interrupts() {
    let mut speaker = Speaker::new(Memory::default());
    speaker.say("uno", Politeness::Polite).unwrap();
    speaker.say("dos", Politeness::Polite).unwrap();
    speaker.say("tres", Politeness::Polite).unwrap();
    // The first is being said; the others wait.
    assert_eq!(speaker.backend().said, [("uno".to_string(), false)]);
    assert!(speaker.busy());
    speaker.backend_mut().busy = false;
    speaker.pump().unwrap();
    assert_eq!(speaker.backend().said.len(), 2);
    assert_eq!(speaker.backend().said[1], ("dos".to_string(), false));
    // An assertive one stops "dos" and drops "tres".
    speaker.say("¡Ya!", Politeness::Assertive).unwrap();
    assert_eq!(speaker.backend().stopped, 1);
    assert_eq!(speaker.backend().said[2], ("¡Ya!".to_string(), true));
    speaker.backend_mut().busy = false;
    speaker.pump().unwrap();
    assert_eq!(speaker.backend().said.len(), 3);
    assert!(!speaker.busy());
    // Nothing is said of nothing.
    speaker.say("", Politeness::Assertive).unwrap();
    assert_eq!(speaker.backend().said.len(), 3);
    assert_eq!(speaker.backend().stopped, 1);
}

#[test]
fn a_record_says_each_utterance_as_a_json_line() {
    let dir = std::env::temp_dir().join(format!("oxyos-speech-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("said.jsonl");
    let _ = std::fs::remove_file(&path);
    let mut speaker = Speaker::new(Record::create(&path).unwrap());
    speaker.say("Nota, \"hola\"", Politeness::Polite).unwrap();
    speaker.say("dos\\tres\n", Politeness::Polite).unwrap();
    speaker.say("ya", Politeness::Assertive).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].ends_with(r#","text":"Nota, \"hola\"","interrupting":false}"#));
    assert!(lines[1].ends_with(r#","text":"dos\\tres\u000a","interrupting":false}"#));
    assert!(lines[2].ends_with(r#","text":"ya","interrupting":true}"#));
    let times: Vec<i64> = lines
        .iter()
        .map(|line| line[5..line.find(',').unwrap()].parse().unwrap())
        .collect();
    assert!(times.windows(2).all(|t| t[0] <= t[1]));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// eSpeak NG itself, when it is installed: it says something and is stopped at once.
#[test]
fn espeak_ng_speaks_and_stops_when_installed() {
    let installed = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join("espeak-ng").is_file())
    });
    if !installed {
        eprintln!("espeak-ng is not installed: skipped");
        return;
    }
    let mut voice = Espeak::new("espeak-ng", Language::Spanish);
    voice
        .say("Hola, esto es OxyOS hablando un buen rato.", false)
        .unwrap();
    assert!(voice.speaking());
    voice.stop();
    assert!(!voice.speaking());
}

#[test]
fn a_missing_voice_is_an_error_not_silence() {
    let mut voice = Espeak::new("/nonexistent/espeak-ng", Language::English);
    assert!(voice.say("hello", false).is_err());
    assert!(!voice.speaking());
}
