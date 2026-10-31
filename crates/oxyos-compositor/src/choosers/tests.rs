//! The session's vocabulary for file choosers, read strictly, and a chooser's answer checked
//! against a home of the tests' own.

use super::*;
use std::path::PathBuf;

struct Home {
    root: PathBuf,
    scope: Scope,
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A home with Pictures/a.png, Pictures/b b.png, Documents/ and its Trash holding old.png; and a
/// file outside it.
fn home(name: &str) -> Home {
    let root = std::env::temp_dir().join(format!("oxyos-choosers-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    for folder in [
        "Pictures",
        "Documents",
        ".local/share/Trash/files",
        ".config",
    ] {
        std::fs::create_dir_all(home.join(folder)).unwrap();
    }
    for file in [
        "Pictures/a.png",
        "Pictures/b b.png",
        ".local/share/Trash/files/old.png",
    ] {
        std::fs::write(home.join(file), b"png").unwrap();
    }
    std::fs::write(root.join("outside.png"), b"png").unwrap();
    std::fs::write(root.join("mountinfo"), b"").unwrap();
    let scope = Scope::new(
        &home,
        &home.join(".local/share"),
        &home.join(".config"),
        Vec::new(),
        root.join("mountinfo"),
        std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&root).unwrap()),
    )
    .unwrap();
    Home { root, scope }
}

impl Home {
    fn uri(&self, relative: &str) -> String {
        uri::of(&self.root.join("home").join(relative), false)
    }
}

fn open(multiple: bool, directory: bool) -> Shown {
    Shown {
        save: false,
        multiple,
        directory,
        types: Vec::new(),
        title: None,
        suggested_name: None,
        folder: None,
        application: None,
    }
}

#[test]
fn a_request_is_read_strictly_and_shown_as_the_chooser_reads_it() {
    let home = home("read");
    let astro = Some(Asker {
        id: "so.oxy.astro".into(),
        name: "Astro".into(),
    });
    let shown = Ask::read(
        &format!(
            r#"{{"open":{{"multiple":true,"types":["image/png","image/*",".png",".tar.gz"],"title":"Subir fotos","folder":"{}"}}}}"#,
            home.uri("Pictures")
        ),
        astro.clone(),
        Some(&home.scope),
    )
    .unwrap();
    let json: Value = serde_json::from_str(&shown.to_json()).unwrap();
    assert_eq!(
        json,
        json!({
            "mode": "open",
            "multiple": true,
            "directory": false,
            "types": ["image/png", "image/*", ".png", ".tar.gz"],
            "title": "Subir fotos",
            "suggestedName": null,
            "folder": format!("{}/", home.uri("Pictures")),
            "application": {"id": "so.oxy.astro", "name": "Astro"},
        })
    );
    let save = Ask::read(r#"{"save":{"name":"Informe final.pdf"}}"#, None, None).unwrap();
    assert!(save.save && !save.multiple && !save.directory);
    assert_eq!(save.suggested_name.as_deref(), Some("Informe final.pdf"));
    assert_eq!(
        Ask::read(r#"{"open":{}}"#, None, None).unwrap(),
        open(false, false)
    );
    // A folder the person's files do not hold, or that is no folder, is left out.
    for folder in [
        uri::of(&home.root.join("outside.png"), false),
        home.uri("Pictures/a.png"),
        home.uri("Gone"),
        home.uri(".local/share/Trash/files"),
        "https://example.com/".into(),
    ] {
        let text = format!(r#"{{"open":{{"folder":"{folder}"}}}}"#);
        assert_eq!(
            Ask::read(&text, None, Some(&home.scope)).unwrap().folder,
            None,
            "{folder}"
        );
    }
    for refused in [
        r#"{"open":{"multiple":true,"path":"/etc"}}"#,
        r#"{"open":{},"save":{}}"#,
        r#"{"delete":{}}"#,
        r#"{"open":{"types":["PNG"]}}"#,
        r#"{"open":{"types":["image"]}}"#,
        r#"{"open":{"types":["."]}}"#,
        r#"{"open":{"types":[".p/ng"]}}"#,
        r#"{"save":{"name":"a/b"}}"#,
        r#"{"save":{"name":".."}}"#,
        r#"{"save":{"name":""}}"#,
        r#"{"save":{"directory":true}}"#,
        "{\"open\":{\"title\":\"a\\u0007b\"}}",
        r#""open""#,
        "",
    ] {
        assert_eq!(
            Ask::read(refused, None, None),
            Err("invalidRequest"),
            "{refused}"
        );
    }
    let many = format!(
        r#"{{"open":{{"types":[{}]}}}}"#,
        vec![r#"".png""#; MAX_TYPES + 1].join(",")
    );
    assert_eq!(Ask::read(&many, None, None), Err("invalidRequest"));
    let long = format!(
        r#"{{"save":{{"name":"{}"}}}}"#,
        "a".repeat(MAX_NAME_BYTES + 1)
    );
    assert_eq!(Ask::read(&long, None, None), Err("invalidRequest"));
}

#[test]
fn an_answer_chooses_only_what_the_request_allows_in_the_person_s_files() {
    let home = home("answer");
    let scope = Some(&home.scope);
    let (a, b) = (home.uri("Pictures/a.png"), home.uri("Pictures/b b.png"));
    let chosen = |shown: &Shown, uris: &[&str], with_input| {
        check(
            shown,
            &json!({ "chosen": uris }).to_string(),
            with_input,
            scope,
        )
    };
    // A cancellation, input or not.
    assert_eq!(
        check(&open(false, false), r#"{"cancelled":{}}"#, false, scope),
        Ok(r#"{"cancelled":{}}"#.into())
    );
    // A file, as its URI is spelled; several only when asked for, each once.
    assert_eq!(
        chosen(&open(false, false), &[&a], true),
        Ok(json!({ "chosen": [a] }).to_string())
    );
    assert!(b.contains("b%20b.png"));
    assert_eq!(
        chosen(&open(true, false), &[&a, &b, &a], true),
        Ok(json!({ "chosen": [a, b] }).to_string())
    );
    assert!(chosen(&open(false, false), &[&a, &b], true).is_err());
    // A folder for a folder, a file for a file.
    let pictures = format!("{}/", home.uri("Pictures"));
    assert_eq!(
        chosen(&open(false, true), &[&pictures], true),
        Ok(json!({ "chosen": [pictures] }).to_string())
    );
    assert!(chosen(&open(false, true), &[&a], true).is_err());
    assert!(chosen(&open(false, false), &[&pictures], true).is_err());
    // Without the chooser's input, nothing, nor anything outside the person's files, in a Trash,
    // not there, not a file:// URI, or no answer at all.
    for refused in [
        chosen(&open(false, false), &[&a], false),
        chosen(&open(false, false), &[], true),
        chosen(
            &open(false, false),
            &[&uri::of(&home.root.join("outside.png"), false)],
            true,
        ),
        chosen(
            &open(false, false),
            &[&home.uri(".local/share/Trash/files/old.png")],
            true,
        ),
        chosen(&open(false, false), &[&home.uri("Pictures/c.png")], true),
        chosen(&open(false, false), &["https://example.com/a.png"], true),
        chosen(&open(false, false), &["file:///etc/passwd"], true),
        check(&open(false, false), r#"{"chosen":"x"}"#, true, scope),
        check(&open(false, false), r#"{"opened":[]}"#, true, scope),
    ] {
        assert!(refused.is_err(), "{refused:?}");
    }
    // A save: a new file in a folder of the person's, or one there to replace; never a folder,
    // nor in a folder that is not there.
    let mut save = open(false, false);
    save.save = true;
    let new = home.uri("Documents/Informe final.pdf");
    assert_eq!(
        chosen(&save, &[&new], true),
        Ok(json!({ "chosen": [new] }).to_string())
    );
    assert_eq!(
        chosen(&save, &[&a], true),
        Ok(json!({ "chosen": [a] }).to_string())
    );
    assert!(chosen(&save, &[&home.uri("Documents")], true).is_err());
    assert!(chosen(&save, &[&home.uri("Gone/x.pdf")], true).is_err());
    assert!(chosen(&save, &[&new, &a], true).is_err());
    // No home: nothing can be chosen.
    assert!(check(
        &open(false, false),
        &json!({"chosen": [a]}).to_string(),
        true,
        None
    )
    .is_err());
}
