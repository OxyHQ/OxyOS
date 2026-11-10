use super::*;

/// The package format's and the install service's rule before this crate owned it
/// (`oxyos_package::manifest::valid_package_id`, `oxyos_install::install::valid_id`), as it was.
fn package_rule_before(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.split('.').count() >= 2
        && id.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
        })
}

/// The registry's and the preferences' rule before (`oxyos_apps::valid_id`,
/// `oxyos_preferences::application_id`), as it was: any case, one label, empty labels.
fn registry_rule_before(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// Why the registry's old rule took an id the documented one refuses, if it did.
fn tightened(id: &str) -> Option<&'static str> {
    if !registry_rule_before(id) || valid_app_id(id) {
        return None;
    }
    if id.bytes().any(|b| b.is_ascii_uppercase()) {
        Some("uppercase")
    } else if !id.contains('.') {
        Some("one label")
    } else if id.split('.').any(str::is_empty) {
        Some("an empty label")
    } else {
        None
    }
}

/// Every string of up to `length` bytes from `alphabet`.
fn strings(alphabet: &[u8], length: usize) -> Vec<String> {
    let mut all = vec![String::new()];
    let mut last = vec![String::new()];
    for _ in 0..length {
        last = last
            .iter()
            .flat_map(|s| {
                alphabet.iter().map(move |&b| {
                    let mut s = s.clone();
                    s.push(b as char);
                    s
                })
            })
            .collect();
        all.extend(last.iter().cloned());
    }
    all
}

#[test]
fn ids_by_the_documented_rule() {
    for (id, valid) in [
        ("so.oxy.allo", true),
        ("so.oxy.activitymonitor", true),
        ("so.oxy.probe.images", true),
        ("org.example.my-app_2", true),
        ("a.b", true),
        ("0.9", true),
        ("", false),
        ("allo", false),
        ("So.Oxy.Allo", false),
        ("org.example.FooViewer", false),
        (".hidden", false),
        ("so.oxy.", false),
        ("so..oxy", false),
        (".", false),
        ("..", false),
        ("so/oxy.allo", false),
        ("../so.oxy.allo", false),
        ("so.oxy.allo ", false),
        ("so oxy.allo", false),
        ("so.oxy.álló", false),
        ("so.oxy.allo\n", false),
    ] {
        assert_eq!(valid_app_id(id), valid, "{id:?}");
    }
    let longest = format!("so.{}", "a".repeat(MAX_APP_ID - 3));
    assert!(valid_app_id(&longest));
    assert!(!valid_app_id(&format!("{longest}a")));
}

/// The package format's and the install service's ids are exactly what they were: every string
/// of up to six bytes over the letters that matter, and the edges of the length.
#[test]
fn the_package_rule_is_unchanged() {
    for id in strings(b"aZ0.-_/ ", 6) {
        assert_eq!(valid_app_id(&id), package_rule_before(&id), "{id:?}");
    }
    for length in 120..=136 {
        for id in [
            format!("a.{}", "b".repeat(length)),
            format!("{}.b", "A".repeat(length)),
        ] {
            assert_eq!(valid_app_id(&id), package_rule_before(&id), "{id:?}");
        }
    }
}

/// The registry and the preferences refuse now only ids the package format always refused, and
/// only for one of three reasons: an application is installed under its package's id, so no
/// installed or system application had such an id (theirs are `so.oxy.*`).
#[test]
fn the_registry_only_refuses_ids_no_package_could_have() {
    let mut changed = 0;
    for id in strings(b"aZ0.-_/ ", 6) {
        if valid_app_id(&id) {
            assert!(registry_rule_before(&id), "{id:?}: the old rule refused it");
        } else if registry_rule_before(&id) {
            assert!(
                tightened(&id).is_some(),
                "{id:?}: refused for another reason"
            );
            assert!(!package_rule_before(&id), "{id:?}");
            changed += 1;
        }
    }
    assert!(changed > 0);
    for (id, reason) in [
        ("So.Oxy.Allo", "uppercase"),
        ("org.example.FooViewer", "uppercase"),
        ("allo", "one label"),
        ("so..oxy", "an empty label"),
        ("so.oxy.", "an empty label"),
    ] {
        assert_eq!(tightened(id), Some(reason), "{id:?}");
    }
}
