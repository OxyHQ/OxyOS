//! The index's questions and ranking, on records the tests make.

use crate::category::Category;
use crate::index::{Found, Index, Question, Record, Source, Usage, Viewer};
use crate::query;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

const NOW: i64 = 1_794_052_800_000;
const DAY: i64 = 86_400_000;

fn file(path: &str, category: Category, modified: i64) -> Record {
    let path = PathBuf::from(path);
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    Record {
        id: format!("file://{}", path.display()),
        source: Source::File,
        owner: None,
        category,
        titles: BTreeMap::from([(String::new(), name)]),
        keywords: Vec::new(),
        tags: Vec::new(),
        path: Some(path),
        url: None,
        application: None,
        icon: None,
        media: None,
        size: Some(10),
        allocated: Some(4096),
        modified: Some(modified),
        created: None,
        changed: Some(modified * 1_000_000),
    }
}

fn application(id: &str, name: &str, keywords: &[&str]) -> Record {
    Record {
        id: format!("application:{id}"),
        source: Source::Application,
        owner: None,
        category: Category::Application,
        titles: BTreeMap::from([(String::new(), name.to_string())]),
        keywords: keywords.iter().map(|k| k.to_string()).collect(),
        tags: Vec::new(),
        path: None,
        url: None,
        application: Some(id.to_string()),
        icon: None,
        media: None,
        size: None,
        allocated: None,
        modified: None,
        created: None,
        changed: None,
    }
}

fn shell() -> Viewer {
    Viewer {
        shell: true,
        ..Viewer::default()
    }
}

fn ask(index: &Index, text: &str, viewer: &Viewer) -> (Vec<String>, Option<String>) {
    ask_with(index, text, viewer, &HashMap::new(), Vec::new())
}

fn ask_with(
    index: &Index,
    text: &str,
    viewer: &Viewer,
    usage: &HashMap<String, Usage>,
    within: Vec<PathBuf>,
) -> (Vec<String>, Option<String>) {
    let question = Question {
        parsed: query::parse(text, &[], NOW),
        within,
        categories: Vec::new(),
        limit: 60,
    };
    let (found, top) = index
        .searcher_handle()
        .search(&question, viewer, usage, NOW)
        .unwrap();
    let ids: Vec<String> = found
        .iter()
        .map(|Found { record, .. }| record.id.clone())
        .collect();
    let top = top.map(|at| ids[at].clone());
    (ids, top)
}

fn home() -> Index {
    let mut index = Index::in_memory().unwrap();
    let report = file(
        "/home/ana/Documents/Quarterly Report.pdf",
        Category::Pdf,
        NOW - DAY,
    );
    index
        .put(
            &report,
            Some("Revenue grew in the third quarter, said the board."),
        )
        .unwrap();
    let mut camion = file(
        "/home/ana/Documents/Camión rojo.txt",
        Category::Text,
        NOW - 40 * DAY,
    );
    camion.tags = vec!["Work".into(), "Viajes largos".into()];
    index
        .put(&camion, Some("Un camión rojo en la carretera"))
        .unwrap();
    index
        .put(
            &file("/home/ana/Documents/Work", Category::Folder, NOW - 2 * DAY),
            None,
        )
        .unwrap();
    index
        .put(
            &file(
                "/home/ana/Documents/Work/settings notes.md",
                Category::Text,
                NOW - 3 * DAY,
            ),
            Some("notes about the router settings"),
        )
        .unwrap();
    index
        .put(
            &application("so.oxy.settings", "Settings", &["preferences"]),
            None,
        )
        .unwrap();
    index
        .put(
            &application("so.oxy.dropsea", "Dropsea", &["files", "finder"]),
            None,
        )
        .unwrap();
    let mut contact = application("so.oxy.allo", "Ana García", &["friend"]);
    contact.id = "so.oxy.allo:ana".into();
    contact.source = Source::Donated;
    contact.owner = Some("so.oxy.allo".into());
    contact.category = Category::Contact;
    index.put(&contact, Some("+34 600 000 000")).unwrap();
    index.commit(true).unwrap();
    index
}

#[test]
fn a_name_is_found_as_it_is_typed() {
    let index = home();
    for typed in ["q", "qua", "quarterly", "quarterly rep", "REPORT"] {
        let (ids, top) = ask(&index, typed, &shell());
        assert_eq!(
            ids.first().map(String::as_str),
            Some("file:///home/ana/Documents/Quarterly Report.pdf"),
            "{typed}: {ids:?}"
        );
        assert_eq!(top.as_deref(), ids.first().map(String::as_str), "{typed}");
    }
}

#[test]
fn accents_and_case_are_folded() {
    let index = home();
    let (ids, _) = ask(&index, "camion", &shell());
    assert_eq!(ids, ["file:///home/ana/Documents/Camión rojo.txt"]);
    let (ids, _) = ask(&index, "CAMIÓN", &shell());
    assert_eq!(ids, ["file:///home/ana/Documents/Camión rojo.txt"]);
}

#[test]
fn contents_words_and_phrases() {
    let index = home();
    let (ids, top) = ask(&index, "revenue", &shell());
    assert_eq!(ids, ["file:///home/ana/Documents/Quarterly Report.pdf"]);
    assert_eq!(top, None, "a match in the text alone is no top hit");
    // The last word, being typed, as a prefix.
    let (ids, _) = ask(&index, "the boa", &shell());
    assert_eq!(ids, ["file:///home/ana/Documents/Quarterly Report.pdf"]);
    let (ids, _) = ask(&index, r#""third quarter""#, &shell());
    assert_eq!(ids, ["file:///home/ana/Documents/Quarterly Report.pdf"]);
    let (ids, _) = ask(&index, r#""quarter third""#, &shell());
    assert!(ids.is_empty(), "{ids:?}");
    // Every word must match somewhere.
    let (ids, _) = ask(&index, "revenue camion", &shell());
    assert!(ids.is_empty(), "{ids:?}");
}

#[test]
fn filters_narrow() {
    let index = home();
    let (ids, _) = ask(&index, "kind:pdf", &shell());
    assert_eq!(ids, ["file:///home/ana/Documents/Quarterly Report.pdf"]);
    let (ids, _) = ask(&index, "tipo:carpeta", &shell());
    assert_eq!(ids, ["file:///home/ana/Documents/Work"]);
    let (ids, _) = ask(&index, "tag:work", &shell());
    assert_eq!(ids, ["file:///home/ana/Documents/Camión rojo.txt"]);
    let (ids, _) = ask(&index, r#"etiqueta:"viajes largos""#, &shell());
    assert_eq!(ids, ["file:///home/ana/Documents/Camión rojo.txt"]);
    // A tag's words are found as words too.
    let (ids, _) = ask(&index, "viajes", &shell());
    assert_eq!(ids, ["file:///home/ana/Documents/Camión rojo.txt"]);
    // The last week: neither the 40-day-old file nor the applications (no date).
    let (ids, _) = ask(&index, "date:week", &shell());
    assert_eq!(ids.len(), 3, "{ids:?}");
    assert!(!ids.iter().any(|id| id.contains("Camión")), "{ids:?}");
    let (ids, _) = ask(&index, "name:notes", &shell());
    assert_eq!(ids, ["file:///home/ana/Documents/Work/settings notes.md"]);
    let (ids, _) = ask(&index, "name:router", &shell());
    assert!(ids.is_empty(), "name: is the name alone: {ids:?}");
}

#[test]
fn within_a_folder() {
    let index = home();
    let usage = HashMap::new();
    let (ids, _) = ask_with(
        &index,
        "settings",
        &shell(),
        &usage,
        vec![PathBuf::from("/home/ana/Documents/Work")],
    );
    assert_eq!(ids, ["file:///home/ana/Documents/Work/settings notes.md"]);
}

#[test]
fn applications_come_first_and_used_items_rise() {
    let index = home();
    let (ids, top) = ask(&index, "sett", &shell());
    assert_eq!(ids[0], "application:so.oxy.settings", "{ids:?}");
    assert_eq!(top.as_deref(), Some("application:so.oxy.settings"));
    assert!(ids.contains(&"file:///home/ana/Documents/Work/settings notes.md".to_string()));
    // Opened from a search many times, lately: the notes come first.
    let usage = HashMap::from([(
        "file:///home/ana/Documents/Work/settings notes.md".to_string(),
        Usage {
            count: 12,
            last: NOW - DAY,
        },
    )]);
    let (ids, _) = ask_with(&index, "sett", &shell(), &usage, Vec::new());
    assert_eq!(
        ids[0], "file:///home/ana/Documents/Work/settings notes.md",
        "{ids:?}"
    );
}

#[test]
fn an_exact_name_beats_a_longer_one() {
    let mut index = home();
    index
        .put(
            &file("/home/ana/Work plan.txt", Category::Text, NOW - 100 * DAY),
            None,
        )
        .unwrap();
    index.commit(false).unwrap();
    let (ids, top) = ask(&index, "work", &shell());
    assert_eq!(ids[0], "file:///home/ana/Documents/Work", "{ids:?}");
    assert_eq!(top.as_deref(), Some("file:///home/ana/Documents/Work"));
    // A file's name without its extension is exact too.
    let (ids, _) = ask(&index, "work plan", &shell());
    assert_eq!(ids[0], "file:///home/ana/Work plan.txt", "{ids:?}");
}

#[test]
fn recent_files_rank_above_old_ones() {
    let mut index = Index::in_memory().unwrap();
    index
        .put(
            &file("/h/budget old.txt", Category::Text, NOW - 400 * DAY),
            None,
        )
        .unwrap();
    index
        .put(&file("/h/budget new.txt", Category::Text, NOW - DAY), None)
        .unwrap();
    index.commit(true).unwrap();
    let (ids, _) = ask(&index, "budget", &shell());
    assert_eq!(
        ids,
        ["file:///h/budget new.txt", "file:///h/budget old.txt"]
    );
}

#[test]
fn applications_see_files_and_their_own_items() {
    let index = home();
    let allo = Viewer {
        application: Some("so.oxy.allo".into()),
        ..Viewer::default()
    };
    let dropsea = Viewer {
        application: Some("so.oxy.dropsea".into()),
        ..Viewer::default()
    };
    assert_eq!(ask(&index, "ana", &shell()).0, ["so.oxy.allo:ana"]);
    assert_eq!(ask(&index, "ana", &allo).0, ["so.oxy.allo:ana"]);
    assert!(ask(&index, "ana", &dropsea).0.is_empty());
    // No application sees the applications or Settings' items: the shell's.
    assert!(ask(&index, "dropsea", &dropsea).0.is_empty());
    let (ids, _) = ask(&index, "sett", &dropsea);
    assert_eq!(ids, ["file:///home/ana/Documents/Work/settings notes.md"]);
    // The shell's switches hide categories.
    let hiding = Viewer {
        shell: true,
        hidden: vec![Category::Application],
        ..Viewer::default()
    };
    assert!(!ask(&index, "sett", &hiding)
        .0
        .iter()
        .any(|id| id.starts_with("application:")));
}

#[test]
fn removing_items_and_folders() {
    let mut index = home();
    index
        .remove_inside(&PathBuf::from("/home/ana/Documents/Work"))
        .unwrap();
    index
        .remove("file:///home/ana/Documents/Camión rojo.txt")
        .unwrap();
    index.commit(true).unwrap();
    assert!(ask(&index, "notes", &shell()).0.is_empty());
    assert!(ask(&index, "camion", &shell()).0.is_empty());
    // The folder itself stays.
    assert_eq!(
        ask(&index, "work", &shell()).0,
        ["file:///home/ana/Documents/Work"]
    );
    index
        .remove_source(&Source::Donated, Some("so.oxy.allo"))
        .unwrap();
    index.commit(true).unwrap();
    assert!(ask(&index, "ana", &shell()).0.is_empty());
    let files = index.files().unwrap();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(files.contains_key("file:///home/ana/Documents/Quarterly Report.pdf"));
}

#[test]
fn snippets_are_around_the_first_word() {
    let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(4)
        + "The camión was red. "
        + &"Sed do eiusmod tempor incididunt ut labore. ".repeat(6);
    let parsed = query::parse("camion", &[], NOW);
    let snippet = crate::index::Searcher::snippet(&text, &parsed).unwrap();
    assert!(
        snippet.starts_with('…') && snippet.ends_with('…'),
        "{snippet}"
    );
    assert!(snippet.contains("The camión was red."), "{snippet}");
    assert!(snippet.chars().count() <= 165, "{snippet}");
}

#[test]
fn what_the_files_occupy_by_kind_and_the_largest_first_come_from_the_columns() {
    let mut index = Index::in_memory().unwrap();
    let sized = |path: &str, category: Category, size: u64, allocated: u64| Record {
        size: Some(size),
        allocated: Some(allocated),
        ..file(path, category, NOW - DAY)
    };
    for (record, text) in [
        (
            sized(
                "/home/ada/Pictures/beach.jpg",
                Category::Image,
                3_000_000,
                3_002_368,
            ),
            None,
        ),
        (
            sized(
                "/home/ada/Pictures/trip.mov",
                Category::Movie,
                90_000_000,
                90_001_408,
            ),
            None,
        ),
        (
            sized(
                "/home/ada/Documents/report.pdf",
                Category::Pdf,
                500_000,
                503_808,
            ),
            None,
        ),
        (
            sized("/home/ada/Documents/notes.txt", Category::Text, 12, 4_096),
            Some("notes"),
        ),
        // A sparse disk image: large, holding little; ordered by its size, counted by its blocks.
        (
            sized("/home/ada/vm.img", Category::Other, 200_000_000, 8_192),
            None,
        ),
        // Elsewhere than the home: not the home's.
        (
            sized(
                "/media/ada/USB/big.iso",
                Category::Archive,
                700_000_000,
                700_000_256,
            ),
            None,
        ),
        (file("/home/ada/Documents", Category::Folder, NOW), None),
    ] {
        index.put(&record, text).unwrap();
    }
    index
        .put(&application("so.oxy.allo", "Allo", &[]), None)
        .unwrap();
    index.commit(true).unwrap();
    let searcher = index.searcher_handle();
    let home = std::path::Path::new("/home/ada");
    let occupied = searcher.occupied(home).unwrap();
    assert_eq!(
        occupied,
        BTreeMap::from([
            (Category::Pdf, (1, 503_808)),
            (Category::Text, (1, 4_096)),
            (Category::Image, (1, 3_002_368)),
            (Category::Movie, (1, 90_001_408)),
            (Category::Other, (1, 8_192)),
        ])
    );
    let largest: Vec<String> = searcher
        .largest(home, 3)
        .unwrap()
        .into_iter()
        .map(|record| record.title("").to_string())
        .collect();
    assert_eq!(largest, ["vm.img", "trip.mov", "beach.jpg"]);
    // A file removed leaves both.
    index.remove("file:///home/ada/Pictures/trip.mov").unwrap();
    index.commit(true).unwrap();
    let searcher = index.searcher_handle();
    assert!(!searcher
        .occupied(home)
        .unwrap()
        .contains_key(&Category::Movie));
    assert_eq!(searcher.largest(home, 10).unwrap().len(), 4);
}
