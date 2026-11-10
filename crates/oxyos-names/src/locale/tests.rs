use super::*;
use std::collections::HashMap;

fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let pairs: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name| pairs.get(name).cloned()
}

/// The four copies' choice of variable before (`oxyos_preferences::environment_locale`,
/// `oxyos_intl::posix_locale`, `oxyos_speech::Language::from_env`, `oxyos_apps::languages`),
/// which were the same.
fn chosen_before(variable: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(variable)
        .find(|value| !value.is_empty())
}

#[test]
fn the_variable_is_chosen_in_posix_order_as_before() {
    let values = ["", "es_ES.UTF-8", "de_DE", "C"];
    for all in [None, Some(0), Some(1), Some(3)] {
        for messages in [None, Some(0), Some(2)] {
            for lang in [None, Some(0), Some(1), Some(3)] {
                let mut pairs = Vec::new();
                for (name, value) in [("LC_ALL", all), ("LC_MESSAGES", messages), ("LANG", lang)] {
                    if let Some(value) = value {
                        pairs.push((name, values[value]));
                    }
                }
                let variable = environment(&pairs);
                assert_eq!(
                    messages_locale(&variable),
                    chosen_before(&variable),
                    "{pairs:?}"
                );
            }
        }
    }
    assert_eq!(
        messages_locale(environment(&[("LC_ALL", ""), ("LANG", "pt_BR.UTF-8")])),
        Some("pt_BR.UTF-8".into())
    );
    assert_eq!(messages_locale(environment(&[("LC_CTYPE", "es_ES")])), None);
}

#[test]
fn a_locale_in_its_parts() {
    assert_eq!(
        PosixLocale::parse("sr_RS.UTF-8@latin"),
        Some(PosixLocale {
            name: "sr_RS",
            language: "sr",
            territory: Some("RS"),
            codeset: Some("UTF-8"),
            modifier: Some("latin"),
        })
    );
    assert_eq!(
        PosixLocale::parse("es"),
        Some(PosixLocale {
            name: "es",
            language: "es",
            territory: None,
            codeset: None,
            modifier: None,
        })
    );
    // The modifier is after the first `@`, whatever follows; the codeset before it.
    let odd = PosixLocale::parse("en_US@euro.UTF-8").unwrap();
    assert_eq!(
        (odd.name, odd.codeset, odd.modifier),
        ("en_US", None, Some("euro.UTF-8"))
    );
    for none in [
        "",
        "C",
        "POSIX",
        "C.UTF-8",
        "POSIX@euro",
        ".UTF-8",
        "@latin",
    ] {
        assert_eq!(PosixLocale::parse(none), None, "{none:?}");
    }
    // Only these spellings: a lowercase `c` is a (meaningless) language, as it always was.
    assert_eq!(PosixLocale::parse("c").map(|l| l.language), Some("c"));
}

/// The name every copy split off is the text before the first `.` or `@`.
#[test]
fn the_name_is_what_every_copy_took() {
    for &value in EXAMPLES {
        let before = value.split(['.', '@']).next().unwrap_or_default();
        let none = matches!(before, "" | "C" | "POSIX");
        assert_eq!(
            PosixLocale::parse(value).map(|l| l.name),
            (!none).then_some(before),
            "{value:?}"
        );
    }
}
