//! The service through its control socket, on homes the tests make: the index following files
//! made, written, renamed, moved and deleted; who sees what; donations, Settings' preferences, the
//! applications and their packages' declared items; and the service doing nothing while nothing
//! happens. The indexer is in the idle class: on a busy machine it waits for idle CPUs, so the
//! tests wait for it generously.

use crate::service::{serve, Paths};
use oxyos_files::{uri, Scope};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A service on a home of its own, through its control socket.
struct Running {
    home: PathBuf,
    _dir: tempfile::TempDir,
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    serial: u32,
    thread: Option<std::thread::JoinHandle<std::io::Result<()>>>,
    /// Answers read while waiting for another.
    unasked: Vec<Value>,
}

fn shell() -> Value {
    json!({"id": "so.oxy.shell", "launched": true, "shell": true, "settings": false})
}

fn application(id: &str) -> Value {
    json!({"id": id, "launched": true, "shell": false, "settings": false})
}

fn settings() -> Value {
    json!({"id": "so.oxy.settings", "launched": true, "shell": false, "settings": true})
}

impl Running {
    fn start(make: impl FnOnce(&Path)) -> Running {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("ana");
        std::fs::create_dir_all(&home).unwrap();
        make(&home);
        let mountinfo = dir.path().join("mountinfo");
        std::fs::write(&mountinfo, "").unwrap();
        let scope = Scope::new(
            &home,
            &home.join(".local/share"),
            &home.join(".config"),
            Vec::new(),
            mountinfo,
            rustix::process::getuid().as_raw(),
        )
        .unwrap();
        let home = scope.home().to_path_buf();
        let paths = Paths {
            cache: dir.path().join("cache"),
            data: dir.path().join("data"),
            config: dir.path().join("config/search.json"),
        };
        let (ours, theirs) = UnixStream::pair().unwrap();
        let thread = std::thread::Builder::new()
            .name("search-serve".into())
            .spawn(move || serve(theirs, scope, Some(paths)))
            .unwrap();
        ours.set_read_timeout(Some(Duration::from_secs(60)))
            .unwrap();
        Running {
            home,
            _dir: dir,
            writer: ours.try_clone().unwrap(),
            reader: BufReader::new(ours),
            serial: 0,
            thread: Some(thread),
            unasked: Vec::new(),
        }
    }

    fn say(&mut self, message: Value) {
        self.writer
            .write_all(format!("{message}\n").as_bytes())
            .unwrap();
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("an answer in time");
        serde_json::from_str(&line).unwrap()
    }

    /// Asks `ask` as `asker` (client 1) and waits for its answer.
    fn ask_as(&mut self, client: u32, asker: Value, ask: Value) -> Value {
        self.serial += 1;
        let serial = self.serial;
        self.say(json!({"ask": {"client": client, "serial": serial, "asker": asker, "text": ask.to_string()}}));
        loop {
            let message = self.read();
            let answer = &message["answer"];
            if answer["serial"] == serial && answer["client"] == client {
                return serde_json::from_str(answer["text"].as_str().unwrap()).unwrap();
            }
            self.unasked.push(message);
        }
    }

    fn ask(&mut self, ask: Value) -> Value {
        self.ask_as(1, shell(), ask)
    }

    fn query_as(&mut self, asker: Value, text: &str) -> Vec<String> {
        let answer = self.ask_as(1, asker, json!({"query": {"text": text}}));
        answer["results"]["items"]
            .as_array()
            .unwrap_or_else(|| panic!("{answer}"))
            .iter()
            .map(|item| item["id"].as_str().unwrap().to_string())
            .collect()
    }

    fn query(&mut self, text: &str) -> Vec<String> {
        self.query_as(shell(), text)
    }

    /// Waits until the index is idle (its first crawl over, nothing waiting).
    fn settled(&mut self) {
        let start = Instant::now();
        loop {
            let status = self.ask(json!({"status": {}}));
            if status["status"]["indexing"] == false {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "never settled: {status}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits until `text` finds what `wanted` says.
    fn until(&mut self, text: &str, wanted: impl Fn(&[String]) -> bool) -> Vec<String> {
        let start = Instant::now();
        loop {
            let found = self.query(text);
            if wanted(&found) {
                return found;
            }
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "{text:?} never found what was wanted: {found:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn id(&self, relative: &str) -> String {
        uri::of(&self.home.join(relative), false)
    }

    fn stop(mut self) {
        drop(self.writer);
        let _ = self.reader.get_ref().shutdown(std::net::Shutdown::Both);
        self.thread.take().unwrap().join().unwrap().unwrap();
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn the_index_follows_files_made_changed_renamed_and_deleted() {
    let mut service = Running::start(|home| {
        write(&home.join("Documents/plan.md"), "the harbour project");
        write(&home.join("Documents/.secret.txt"), "harbour");
        write(&home.join(".hidden/notes.txt"), "harbour");
        write(&home.join("code/node_modules/x/readme.md"), "harbour");
        write(
            &home.join("code/target/CACHEDIR.TAG"),
            "Signature: 8a477f597d28d172789f06886806bc55",
        );
        write(&home.join("code/target/debug.txt"), "harbour");
        write(&home.join("Old.noindex/a.txt"), "harbour");
    });
    service.settled();
    let plan = service.id("Documents/plan.md");
    assert_eq!(service.query("harbour"), [plan.clone()]);
    assert_eq!(service.query("plan"), [plan.clone()]);

    // Made.
    write(
        &service.home.join("Documents/lighthouse.txt"),
        "keeper of the harbour light",
    );
    let lighthouse = service.id("Documents/lighthouse.txt");
    service.until("keeper", |found| found == [lighthouse.clone()]);

    // Written again: its old words go, its new ones come.
    std::fs::write(
        service.home.join("Documents/lighthouse.txt"),
        "a beacon by the sea",
    )
    .unwrap();
    service.until("beacon", |found| found == [lighthouse.clone()]);
    assert!(service.query("keeper").is_empty());

    // Renamed: found by its new name, its text kept.
    std::fs::rename(
        service.home.join("Documents/lighthouse.txt"),
        service.home.join("Documents/faro.txt"),
    )
    .unwrap();
    let faro = service.id("Documents/faro.txt");
    service.until("faro", |found| found == [faro.clone()]);
    assert_eq!(service.query("beacon"), [faro.clone()]);
    assert!(service.query("lighthouse").is_empty());

    // A folder moved: everything in it where it is now.
    std::fs::rename(service.home.join("Documents"), service.home.join("Papers")).unwrap();
    let moved = service.id("Papers/plan.md");
    service.until("harbour", |found| found == [moved.clone()]);
    assert_eq!(service.query("beacon"), [service.id("Papers/faro.txt")]);

    // Deleted, and a folder deleted with what it holds.
    std::fs::remove_file(service.home.join("Papers/faro.txt")).unwrap();
    service.until("beacon", <[String]>::is_empty);
    std::fs::remove_dir_all(service.home.join("Papers")).unwrap();
    service.until("harbour", <[String]>::is_empty);
    assert!(service.query("papers").is_empty());

    // Moved in from outside the places, and out.
    let outside = service.home.parent().unwrap().join("outside");
    write(&outside.join("atlas.txt"), "a map of the islands");
    std::fs::rename(&outside, service.home.join("Maps")).unwrap();
    let atlas = service.id("Maps/atlas.txt");
    service.until("islands", |found| found == [atlas.clone()]);
    std::fs::rename(service.home.join("Maps"), &outside).unwrap();
    service.until("islands", <[String]>::is_empty);

    // Made hidden: left out.
    write(&service.home.join("Visible/x.txt"), "zebra");
    service.until("zebra", |found| found.len() == 1);
    std::fs::rename(service.home.join("Visible"), service.home.join(".Visible")).unwrap();
    service.until("zebra", <[String]>::is_empty);
    service.stop();
}

#[test]
fn files_unchanged_are_not_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("bea");
    write(&home.join("a.txt"), "alpha");
    write(&home.join("b.txt"), "bravo");
    write(&home.join("c.txt"), "charlie");
    let mountinfo = dir.path().join("mountinfo");
    std::fs::write(&mountinfo, "").unwrap();
    let paths = Paths {
        cache: dir.path().join("cache"),
        data: dir.path().join("data"),
        config: dir.path().join("config.json"),
    };
    let run = |changes: &dyn Fn(), checks: &dyn Fn(&mut Running)| {
        changes();
        let scope = Scope::new(&home, &home, &home, Vec::new(), mountinfo.clone(), 0).unwrap();
        let (ours, theirs) = UnixStream::pair().unwrap();
        let kept = paths.clone();
        let thread = std::thread::spawn(move || serve(theirs, scope, Some(kept)));
        ours.set_read_timeout(Some(Duration::from_secs(60)))
            .unwrap();
        let mut running = Running {
            home: std::fs::canonicalize(&home).unwrap(),
            _dir: tempfile::tempdir().unwrap(),
            writer: ours.try_clone().unwrap(),
            reader: BufReader::new(ours),
            serial: 0,
            thread: Some(thread),
            unasked: Vec::new(),
        };
        running.settled();
        checks(&mut running);
        running.stop();
    };
    run(&|| {}, &|service| {
        let status = service.ask(json!({"status": {}}));
        assert_eq!(status["status"]["read"], 3, "{status}");
    });
    // While it was not running: one changed, one deleted, one made.
    run(
        &|| {
            std::thread::sleep(Duration::from_millis(20));
            std::fs::write(home.join("b.txt"), "beta").unwrap();
            std::fs::remove_file(home.join("c.txt")).unwrap();
            write(&home.join("d.txt"), "delta");
        },
        &|service| {
            // The two changed read, the third not.
            let status = service.ask(json!({"status": {}}));
            assert_eq!(status["status"]["read"], 2, "{status}");
            assert_eq!(service.query("alpha"), [service.id("a.txt")]);
            assert_eq!(service.query("beta"), [service.id("b.txt")]);
            assert!(service.query("bravo").is_empty());
            assert!(service.query("charlie").is_empty());
            assert_eq!(service.query("delta"), [service.id("d.txt")]);
        },
    );
}

#[test]
fn applications_see_files_and_their_own_items_and_the_shell_sees_all() {
    let mut service = Running::start(|home| {
        write(&home.join("Ana's notes.txt"), "lunch with ana");
    });
    let package = service.home.parent().unwrap().join("settings-package");
    write(
        &package.join("searchable.json"),
        &json!([
            {"id": "settings:wifi", "url": "oxyos-settings://wifi",
             "titles": {"en": "Wi-Fi", "es": "Wi-Fi"},
             "keywords": {"en": ["wireless", "router"], "es": ["inalámbrica"]}},
            {"id": "settings:appearance", "url": "oxyos-settings://appearance",
             "titles": {"en": "Appearance", "es": "Apariencia"},
             "keywords": {"en": ["dark mode"], "es": ["modo oscuro"]}}
        ])
        .to_string(),
    );
    service.say(json!({"applications": [
        {"id": "so.oxy.settings", "name": "Settings", "keywords": ["preferences"], "icon": "/usr/lib/oxyos/apps/so.oxy.settings/icon.png", "package": package},
        {"id": "so.oxy.dropsea", "name": "Dropsea", "keywords": ["files", "finder"], "icon": null, "package": null}
    ]}));
    service.settled();
    let notes = service.id("Ana's notes.txt");
    service.until("router", |found| found == ["settings:wifi"]);
    assert_eq!(service.query("finder"), ["application:so.oxy.dropsea"]);
    assert_eq!(service.query("oscuro"), ["settings:appearance"]);
    // Titles in the asker's language.
    let answer = service.ask(json!({"query": {"text": "oscuro", "language": "es-ES"}}));
    assert_eq!(answer["results"]["items"][0]["title"], "Apariencia");
    assert_eq!(
        answer["results"]["items"][0]["url"],
        "oxyos-settings://appearance"
    );
    assert_eq!(answer["results"]["items"][0]["category"], "settings");
    let answer = service.ask(json!({"query": {"text": "sett"}}));
    assert_eq!(answer["results"]["topHit"], "application:so.oxy.settings");
    assert_eq!(
        answer["results"]["items"][0]["icon"],
        "file:///usr/lib/oxyos/apps/so.oxy.settings/icon.png"
    );

    // Donations: an application's own items, seen by it and by the shell alone.
    let done = service.ask_as(
        2,
        application("so.oxy.allo"),
        json!({"donate": {"items": [{"id": "ana", "title": "Ana García", "category": "contact", "text": "+34 600", "keywords": ["friend"], "url": "allo://contact/ana"}]}}),
    );
    assert_eq!(done, json!({"done": {}}));
    let both = |mut found: Vec<String>| {
        found.sort();
        found
    };
    let mut expected = vec![notes.clone(), "so.oxy.allo:ana".to_string()];
    expected.sort();
    assert_eq!(both(service.query("ana")), expected);
    assert_eq!(
        both(service.query_as(application("so.oxy.allo"), "ana")),
        expected
    );
    assert_eq!(
        service.query_as(application("so.oxy.dropsea"), "ana"),
        [notes.clone()]
    );
    assert!(service
        .query_as(application("so.oxy.dropsea"), "finder")
        .is_empty());
    assert!(service
        .query_as(application("so.oxy.dropsea"), "router")
        .is_empty());
    // A client the session did not launch donates nothing and sees files alone.
    let unlaunched =
        json!({"id": "so.oxy.allo", "launched": false, "shell": false, "settings": false});
    assert_eq!(
        service.ask_as(3, unlaunched.clone(), json!({"donate": {"items": []}})),
        json!({"refused": {"reason": "notPermitted"}})
    );
    assert_eq!(service.query_as(unlaunched, "ana"), [notes.clone()]);
    // Another application cannot remove them.
    assert_eq!(
        service.ask_as(
            4,
            application("so.oxy.dropsea"),
            json!({"remove": {"ids": ["ana"]}})
        ),
        json!({"done": {}})
    );
    assert_eq!(service.query("garcia"), ["so.oxy.allo:ana"]);
    assert_eq!(
        service.ask_as(2, application("so.oxy.allo"), json!({"removeAll": {}})),
        json!({"done": {}})
    );
    assert!(service.query("garcia").is_empty());

    // Opened from a search: it rises.
    write(&service.home.join("ana-budget.txt"), "x");
    let budget = service.id("ana-budget.txt");
    service.until("ana", |found| found.len() == 2);
    assert_eq!(
        service.ask(json!({"used": {"id": budget}})),
        json!({"noted": {}})
    );
    assert_eq!(
        service.ask(json!({"used": {"id": budget}})),
        json!({"noted": {}})
    );
    let answer = service.ask(json!({"query": {"text": "ana"}}));
    assert_eq!(answer["results"]["items"][0]["id"], budget);
    assert!(answer["results"]["items"][0]["lastUsed"].is_i64());
    service.stop();
}

#[test]
fn settings_alone_keeps_folders_and_categories_out() {
    let mut service = Running::start(|home| {
        write(&home.join("Private/diary.txt"), "dear diary");
        write(&home.join("Public/poster.txt"), "dear reader");
    });
    service.say(json!({"applications": [{"id": "so.oxy.dropsea", "name": "Dropsea"}]}));
    service.settled();
    service.until("dear", |found| found.len() == 2);
    let private = uri::of(&service.home.join("Private"), true);
    assert_eq!(
        service.ask(json!({"setPreferences": {"excluded": [private]}})),
        json!({"refused": {"reason": "notPermitted"}}),
        "the shell keeps nothing out"
    );
    let kept = service.ask_as(
        5,
        settings(),
        json!({"setPreferences": {"excluded": [private], "categories": {"application": false}}}),
    );
    assert_eq!(kept["preferences"]["excluded"], json!([private]));
    assert_eq!(kept["preferences"]["categories"]["application"], false);
    assert_eq!(kept["preferences"]["categories"]["pdf"], true);
    let poster = service.id("Public/poster.txt");
    service.until("dear", |found| found == [poster.clone()]);
    assert!(service.query("private").is_empty());
    // Applications hidden from the shell's Spotlight, not from the index nor from the launcher,
    // which asks for applications by name.
    assert!(service.query("dropsea").is_empty());
    let launcher =
        service.ask(json!({"query": {"text": "dropsea", "categories": ["application"]}}));
    assert_eq!(
        launcher["results"]["items"][0]["id"],
        "application:so.oxy.dropsea"
    );
    // A file made in the folder kept out stays out.
    write(&service.home.join("Private/more.txt"), "dear secret");
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(service.query("secret"), Vec::<String>::new());
    // Let back in: crawled.
    service.ask_as(
        5,
        settings(),
        json!({"setPreferences": {"excluded": [], "categories": {"application": true}}}),
    );
    service.until("dear", |found| found.len() == 3);
    assert_eq!(service.query("dropsea"), ["application:so.oxy.dropsea"]);
    let read = service.ask_as(5, settings(), json!({"preferences": {}}));
    assert_eq!(read["preferences"]["excluded"], json!([]));
    service.stop();
}

#[test]
fn watchers_are_told_of_changes() {
    let mut service = Running::start(|_| {});
    service.settled();
    assert_eq!(
        service.ask_as(7, shell(), json!({"watch": {}})),
        json!({"watching": {}})
    );
    write(&service.home.join("new.txt"), "fresh");
    let start = Instant::now();
    loop {
        if service
            .unasked
            .iter()
            .any(|message| message["answer"]["client"] == 7 && message["answer"]["serial"] == 0)
        {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "no change told");
        let message = service.read();
        service.unasked.push(message);
    }
    let told = service
        .unasked
        .iter()
        .find(|message| message["answer"]["serial"] == 0)
        .unwrap();
    assert_eq!(told["answer"]["text"], json!({"changed": {}}).to_string());
    service.say(json!({"gone": {"client": 7}}));
    service.stop();
}

#[test]
fn settings_alone_is_told_what_the_homes_files_occupy_and_its_largest_files() {
    let mut service = Running::start(|home| {
        std::fs::create_dir_all(home.join("Pictures")).unwrap();
        std::fs::write(home.join("Pictures/beach.jpg"), vec![7u8; 300_000]).unwrap();
        std::fs::write(home.join("Pictures/trip.mp4"), vec![8u8; 900_000]).unwrap();
        write(&home.join("Documents/notes.txt"), "a few words");
        // Hidden: no one's to count here (an application's data, a cache).
        std::fs::create_dir_all(home.join(".cache/big")).unwrap();
        std::fs::write(home.join(".cache/big/blob"), vec![9u8; 2_000_000]).unwrap();
    });
    service.settled();
    let blocks = |path: &Path| {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).unwrap().blocks() * 512
    };
    let answer = service.ask_as(1, settings(), json!({"storage": {}}));
    let storage = &answer["storage"];
    assert_eq!(storage["home"], uri::of(&service.home, true));
    assert_eq!(storage["indexing"], false);
    assert_eq!(
        storage["kinds"],
        json!({
            "image": {"files": 1, "bytes": blocks(&service.home.join("Pictures/beach.jpg"))},
            "movie": {"files": 1, "bytes": blocks(&service.home.join("Pictures/trip.mp4"))},
            "text": {"files": 1, "bytes": blocks(&service.home.join("Documents/notes.txt"))},
        })
    );
    let largest = service.ask_as(1, settings(), json!({"largest": {"limit": 2}}));
    let items = largest["results"]["items"].as_array().unwrap();
    let titles: Vec<&str> = items.iter().map(|i| i["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["trip.mp4", "beach.jpg"]);
    assert_eq!(items[0]["size"], 900_000);
    assert_eq!(items[0]["location"], "Pictures");
    assert_eq!(items[0]["uri"], service.id("Pictures/trip.mp4"));
    // A file deleted leaves both, once the index follows.
    std::fs::remove_file(service.home.join("Pictures/trip.mp4")).unwrap();
    service.until("trip", <[String]>::is_empty);
    let answer = service.ask_as(1, settings(), json!({"storage": {}}));
    assert!(
        answer["storage"]["kinds"].get("movie").is_none(),
        "{answer}"
    );
    // Nobody else is told: not an application, not the shell.
    for asker in [application("so.oxy.allo"), shell()] {
        for ask in [json!({"storage": {}}), json!({"largest": {}})] {
            assert_eq!(
                service.ask_as(2, asker.clone(), ask),
                json!({"refused": {"reason": "notPermitted"}})
            );
        }
    }
    service.stop();
}
