//! The person's backups through the files service (docs/adr/0059-backups.mdx, decisions 7 and 10),
//! against the backup service itself, run on the tests' own folders as `oxyos-backupd
//! --development` runs it (its user's alone, the home backed up to a played disk) over Debian's
//! restic, on a socket of its own: the state, a file's versions, what a folder held, a file and a
//! folder restored beside what is there (Keep Both), in its place (Replace), where it was, and
//! elsewhere, a folder's archive read strictly, a restore cancelled, the state followed, and the
//! refusals.
//!
//! restic must be installed (`apt install restic`). Without it these tests say so and pass, unless
//! `OXYOS_BACKUP_ENGINE=required` (the Debian job of CI), where its absence fails them.

use crate::backups::Backups;
use crate::restore::kept_name;
use crate::service::{BackupAccess, NetworkAccess, Reply, Service};
use crate::tests::{scope_in, Place};
use crate::uri;
use oxyos_backup::disks::Played;
use oxyos_backup::engine::Engine;
use oxyos_backup::policy::Sources;
use oxyos_backup::service::{self as backupd, Options};
use oxyos_service_socket::{Connection, Listener};
use serde_json::{json, Value};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// restic, if it is installed.
fn engine(cache: &Path) -> Option<Engine> {
    match Engine::from_path(cache) {
        Some(engine) => Some(engine),
        None if std::env::var("OXYOS_BACKUP_ENGINE").as_deref() == Ok("required") => {
            panic!("restic is not installed, and OXYOS_BACKUP_ENGINE=required")
        }
        None => {
            eprintln!(
                "restic is not installed: the backups' tests are skipped (apt install restic)"
            );
            None
        }
    }
}

/// The backup service on a socket of the test's own, backing up `home` to a played disk.
struct Backupd {
    service: backupd::Service,
    socket: PathBuf,
}

impl Backupd {
    fn start(place: &Place, home: &Path) -> Option<Backupd> {
        let engine = engine(&place.dir("cache"))?;
        let volumes = place.dir("volumes");
        let options = Options {
            state: place.path("state"),
            engine,
            disks: Arc::new(Played::open(&volumes).expect("volumes")),
            sources: Sources {
                paths: vec![home.to_path_buf()],
                excludes: vec![],
            },
            passwd: "/etc/passwd".into(),
            group: "/etc/group".into(),
            accounts: None,
            manifest: None,
            development: true,
            clock: Arc::new(backupd::now),
        };
        let service = backupd::Service::open(options, "test-computer").expect("service");
        let scheduler = service.clone();
        std::thread::spawn(move || scheduler.schedule());
        let socket = place.path("backupd.socket");
        let listener = Listener::bind(&socket, 0o600).expect("socket");
        let serving = service.clone();
        std::thread::spawn(move || {
            while let Ok(connection) = listener.accept() {
                let serving = serving.clone();
                std::thread::spawn(move || {
                    let _ = serving.serve(&connection);
                });
            }
        });
        // A disk connected, added, and backed up at once.
        std::fs::create_dir_all(volumes.join("1234-ABCD")).expect("disk");
        std::fs::write(volumes.join("1234-ABCD/.oxyos-label"), "Backup Disk").expect("label");
        let backupd = Backupd { service, socket };
        let added = loop {
            let answer =
                backupd.ask(json!({"addDisk": {"uuid": "1234-ABCD", "password": "correct horse"}}));
            if answer["refused"]["reason"] != "notConnected" {
                break answer;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(added.get("added").is_some(), "{added}");
        backupd.until_backups(1);
        Some(backupd)
    }

    fn ask(&self, request: Value) -> Value {
        let connection = Connection::connect(&self.socket).expect("connect");
        connection.send(&request).expect("send");
        connection
            .receive()
            .expect("receive")
            .expect("open")
            .expect("json")
    }

    fn until_backups(&self, count: u64) {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let status = self.service.status(true);
            if status.running.is_none()
                && status
                    .destinations
                    .first()
                    .is_some_and(|d| d.backups >= count)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "never {count} backups: {status:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn back_up(&self, count: u64) {
        assert_eq!(self.ask(json!({"backUpNow": {}})), json!("started"));
        self.until_backups(count);
    }
}

impl Drop for Backupd {
    fn drop(&mut self) {
        self.service.shutdown();
    }
}

/// The answer to request `id`, the events said meanwhile kept in `events`.
fn ask_with(service: &Service, id: u64, request: Value, events: &mut Vec<Value>) -> Value {
    service.request(id, &request.to_string(), Vec::new());
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(Reply {
            id: answered, json, ..
        }) = service.take()
        {
            let reply: Value = serde_json::from_str(&json).expect("json");
            if answered == id {
                return reply;
            }
            events.push(reply);
        }
        assert!(Instant::now() < deadline, "no answer to {request}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn ask(service: &Service, id: u64, request: Value) -> Value {
    ask_with(service, id, request, &mut Vec::new())
}

fn value(answer: Value) -> Value {
    assert_eq!(answer["ok"], true, "{answer}");
    answer["value"].clone()
}

fn mtime(path: &Path) -> i64 {
    std::fs::metadata(path).expect("there").mtime()
}

/// Nothing half written is left in `folder`.
fn nothing_staged(folder: &Path) {
    for item in std::fs::read_dir(folder).expect("folder").flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        assert!(
            !name.starts_with(".oxyos-restore"),
            "{name} left in {}",
            folder.display()
        );
    }
}

#[test]
fn a_restored_item_is_named_as_time_machine_names_it() {
    assert_eq!(kept_name("notes.txt", false, 1), "notes (original).txt");
    assert_eq!(kept_name("notes.txt", false, 2), "notes (original 2).txt");
    assert_eq!(kept_name("Taxes 2026", true, 1), "Taxes 2026 (original)");
    assert_eq!(kept_name("v1.2 notes", true, 1), "v1.2 notes (original)");
    assert_eq!(kept_name(".bashrc", false, 1), ".bashrc (original)");
    assert_eq!(
        kept_name("archive.tar.gz", false, 1),
        "archive.tar (original).gz"
    );
    // Never longer than a name can be, cut at a character's edge.
    let long = format!("{}.txt", "ñ".repeat(126));
    let kept = kept_name(&long, false, 3);
    assert!(kept.len() <= 255, "{}", kept.len());
    assert!(kept.ends_with(" (original 3).txt"), "{kept}");
}

#[test]
fn versions_are_browsed_and_restored_as_the_person_beside_in_place_and_elsewhere() {
    let place = Place::new("backups");
    let scope = scope_in(&place);
    let home = scope.home().to_path_buf();
    let documents = place.dir("home/ada/Documents");
    std::fs::create_dir_all(documents.join("Taxes 2026")).expect("folder");
    std::fs::write(documents.join("notes.txt"), "first\n").expect("notes");
    std::fs::write(documents.join("gone.txt"), "deleted later").expect("gone");
    std::fs::write(
        documents.join("Taxes 2026/return.pdf"),
        b"%PDF-1.7 \x00\x01",
    )
    .expect("pdf");
    std::fs::write(documents.join("private"), "mine").expect("private");
    std::fs::set_permissions(
        documents.join("private"),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("mode");
    symlink("notes.txt", documents.join("inside")).expect("link");
    symlink("/etc/hostname", documents.join("out")).expect("link");
    let mut large = Vec::with_capacity(24 << 20);
    let mut x: u32 = 7;
    for _ in 0..(24 << 20) {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        large.push((x >> 24) as u8);
    }
    std::fs::write(home.join("large.raw"), &large).expect("large");
    let Some(backupd) = Backupd::start(&place, &home) else {
        return;
    };
    // A second backup, the notes changed between.
    std::thread::sleep(Duration::from_millis(1100));
    std::fs::write(documents.join("notes.txt"), "second version\n").expect("notes");
    backupd.back_up(2);
    std::fs::write(documents.join("notes.txt"), "third, not backed up\n").expect("notes");
    std::fs::remove_file(documents.join("gone.txt")).expect("gone");

    let service = Service::with_backups(
        scope.clone(),
        NetworkAccess::Refused,
        BackupAccess::Granted(Backups::at(&backupd.socket)),
    )
    .expect("service");

    // The state: one destination, its last backup.
    let state = value(ask(&service, 1, json!({"op": "backups"})));
    assert_eq!(state["destinations"][0]["name"], "Backup Disk");
    assert_eq!(state["destinations"][0]["kind"], "disk");
    assert_eq!(state["destinations"][0]["available"], true);
    assert_eq!(state["backingUp"], false);
    let last = state["lastBackup"].as_i64().expect("last backup");
    assert!(last > 1_700_000_000_000, "milliseconds: {last}");

    // The notes' two versions, newest first, each a backup's.
    let notes = uri::of(&documents.join("notes.txt"), false);
    let versions = value(ask(&service, 2, json!({"op": "versions", "uri": notes})));
    let versions = versions.as_array().expect("versions").clone();
    assert_eq!(versions.len(), 2, "{versions:?}");
    assert_eq!(versions[0]["size"], 15);
    assert_eq!(versions[1]["size"], 6);
    assert_eq!(versions[0]["isDirectory"], false);
    assert!(versions[0]["backup"]["time"].as_i64() > versions[1]["backup"]["time"].as_i64());
    let older = versions[1]["backup"].clone();
    let newer = versions[0]["backup"].clone();

    // Keep Both: beside it, as Time Machine names it, the notes untouched, its time of change kept.
    let restored = value(ask(
        &service,
        3,
        json!({"op": "restoreVersion", "backup": older, "uri": notes}),
    ));
    let kept = documents.join("notes (original).txt");
    assert_eq!(restored["uri"], uri::of(&kept, false));
    assert_eq!(restored["skipped"], 0);
    assert_eq!(std::fs::read_to_string(&kept).expect("kept"), "first\n");
    assert_eq!(
        std::fs::read_to_string(documents.join("notes.txt")).expect("notes"),
        "third, not backed up\n"
    );
    assert_eq!(
        mtime(&kept) * 1000,
        versions[1]["lastModified"].as_i64().expect("modified")
    );
    let again = value(ask(
        &service,
        4,
        json!({"op": "restoreVersion", "backup": older, "uri": notes}),
    ));
    assert_eq!(
        again["uri"],
        uri::of(&documents.join("notes (original 2).txt"), false)
    );

    // Replace: in its place, in one step.
    let replaced = value(ask(
        &service,
        5,
        json!({"op": "restoreVersion", "backup": newer, "uri": notes, "replace": true}),
    ));
    assert_eq!(replaced["uri"], notes);
    assert_eq!(
        std::fs::read_to_string(documents.join("notes.txt")).expect("notes"),
        "second version\n"
    );

    // A file deleted since: back where it was, under its own name.
    let gone = uri::of(&documents.join("gone.txt"), false);
    let back = value(ask(
        &service,
        6,
        json!({"op": "restoreVersion", "backup": newer, "uri": gone}),
    ));
    assert_eq!(back["uri"], gone);
    assert_eq!(
        std::fs::read_to_string(documents.join("gone.txt")).expect("back"),
        "deleted later"
    );

    // An item of a folder deleted since: back where it was, its folder made again.
    let taxes = documents.join("Taxes 2026");
    std::fs::rename(&taxes, home.join("Taxes moved")).expect("moved away");
    let pdf = uri::of(&taxes.join("return.pdf"), false);
    let pdf_versions = value(ask(&service, 60, json!({"op": "versions", "uri": pdf})));
    let again = value(ask(
        &service,
        61,
        json!({"op": "restoreVersion", "backup": pdf_versions[0]["backup"], "uri": pdf}),
    ));
    assert_eq!(again["uri"], pdf);
    assert!(taxes.join("return.pdf").is_file());

    // What the folder held in the older backup, under the folder's URI as asked.
    let folder = uri::of(&documents, true);
    let contents = value(ask(
        &service,
        7,
        json!({"op": "versionContents", "backup": older, "uri": folder}),
    ));
    let mut names: Vec<(String, String, bool)> = contents
        .as_array()
        .expect("contents")
        .iter()
        .map(|item| {
            (
                item["name"].as_str().expect("name").to_owned(),
                item["uri"].as_str().expect("uri").to_owned(),
                item["isDirectory"].as_bool().expect("kind"),
            )
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            (
                "Taxes 2026".into(),
                uri::of(&documents.join("Taxes 2026"), true),
                true
            ),
            (
                "gone.txt".into(),
                uri::of(&documents.join("gone.txt"), false),
                false
            ),
            ("notes.txt".into(), notes.clone(), false),
            (
                "private".into(),
                uri::of(&documents.join("private"), false),
                false
            ),
        ]
    );

    // A folder has a version in every backup; one restored elsewhere is the folder as it was,
    // its link inside kept, its link leading out left out, its modes back.
    let folder_versions = value(ask(&service, 8, json!({"op": "versions", "uri": folder})));
    assert_eq!(folder_versions.as_array().expect("versions").len(), 2);
    assert_eq!(folder_versions[0]["isDirectory"], true);
    let restored_to = place.dir("home/ada/Restored");
    let restored = value(ask(
        &service,
        9,
        json!({"op": "restoreVersion", "backup": older, "uri": folder,
               "to": uri::of(&restored_to, true)}),
    ));
    let copy = restored_to.join("Documents");
    assert_eq!(restored["uri"], uri::of(&copy, true));
    assert_eq!(restored["skipped"], 1);
    assert_eq!(
        std::fs::read_to_string(copy.join("notes.txt")).expect("notes"),
        "first\n"
    );
    assert_eq!(
        std::fs::read(copy.join("Taxes 2026/return.pdf")).expect("pdf"),
        b"%PDF-1.7 \x00\x01"
    );
    assert_eq!(
        std::fs::read_link(copy.join("inside")).expect("link"),
        Path::new("notes.txt")
    );
    assert!(
        std::fs::symlink_metadata(copy.join("out")).is_err(),
        "a link leading out"
    );
    assert_eq!(
        std::fs::metadata(copy.join("private"))
            .expect("private")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    nothing_staged(&restored_to);
    nothing_staged(&documents);
    // The same folder restored where it is: beside it.
    let beside = value(ask(
        &service,
        10,
        json!({"op": "restoreVersion", "backup": older, "uri": folder}),
    ));
    assert_eq!(
        beside["uri"],
        uri::of(&home.join("Documents (original)"), true)
    );
    assert!(home
        .join("Documents (original)/Taxes 2026/return.pdf")
        .is_file());

    // Progress, as a transfer's; then a restore cancelled at its first progress leaves nothing.
    let large_uri = uri::of(&home.join("large.raw"), false);
    let large_versions = value(ask(
        &service,
        11,
        json!({"op": "versions", "uri": large_uri}),
    ));
    let backup = large_versions[0]["backup"].clone();
    let mut events = Vec::new();
    let to = place.dir("home/ada/Elsewhere");
    let whole = value(ask_with(
        &service,
        12,
        json!({"op": "restoreVersion", "backup": backup, "uri": large_uri, "to": uri::of(&to, true)}),
        &mut events,
    ));
    assert!(
        std::fs::read(to.join("large.raw")).expect("large") == large,
        "the large file differs"
    );
    assert_eq!(whole["uri"], uri::of(&to.join("large.raw"), false));
    let said: Vec<&Value> = events.iter().filter(|e| e["transfer"] == 12).collect();
    assert!(!said.is_empty(), "no progress: {events:?}");
    let end = &said[said.len() - 1]["progress"];
    assert_eq!(end["bytesWritten"], large.len());
    assert_eq!(end["totalBytes"], large.len());
    service.request(
        13,
        &json!({"op": "restoreVersion", "backup": backup, "uri": large_uri}).to_string(),
        Vec::new(),
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    let cancelled = loop {
        if let Some(Reply { id, json, .. }) = service.take() {
            let reply: Value = serde_json::from_str(&json).expect("json");
            if id == 0 && reply["transfer"] == 13 {
                service.call(
                    &json!({"op": "cancelTransfer", "transfer": 13}).to_string(),
                    &[],
                );
            } else if id == 13 {
                break reply;
            }
        }
        assert!(Instant::now() < deadline, "the restore never ended");
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(cancelled["error"], "cancelled", "{cancelled}");
    assert!(!home.join("large (original).raw").exists());
    nothing_staged(&home);
    drop(backupd);
}

#[test]
fn backups_are_asked_only_inside_the_scope_and_only_with_files_granted() {
    let place = Place::new("backups-scope");
    let scope = scope_in(&place);
    let home = scope.home().to_path_buf();
    std::fs::write(home.join("notes.txt"), "notes").expect("notes");
    std::fs::create_dir_all(home.join("Documents")).expect("documents");
    let Some(backupd) = Backupd::start(&place, &home) else {
        return;
    };
    let service = Service::with_backups(
        scope.clone(),
        NetworkAccess::Refused,
        BackupAccess::Granted(Backups::at(&backupd.socket)),
    )
    .expect("service");
    let notes = uri::of(&home.join("notes.txt"), false);
    let versions = value(ask(&service, 1, json!({"op": "versions", "uri": notes})));
    let backup = versions[0]["backup"].clone();

    // Outside what the application reaches: refused before the backup service is asked.
    let outside = ask(
        &service,
        2,
        json!({"op": "versions", "uri": "file:///etc/passwd"}),
    );
    assert_eq!(outside["error"], "refused", "{outside}");
    let elsewhere = ask(
        &service,
        3,
        json!({"op": "restoreVersion", "backup": backup, "uri": notes, "to": "file:///tmp/"}),
    );
    assert_eq!(elsewhere["error"], "refused", "{elsewhere}");
    // A volume's item: no backup holds it.
    let volume = uri::of(&place.path("media/ada/USB Stick/x.txt"), false);
    assert_eq!(
        value(ask(&service, 4, json!({"op": "versions", "uri": volume}))),
        json!([])
    );
    // A backup that is not one.
    let wrong = ask(
        &service,
        5,
        json!({"op": "restoreVersion", "backup": {"destination": "x", "id": "latest"}, "uri": notes}),
    );
    assert_eq!(wrong["error"], "not-found", "{wrong}");
    // The home's standard folders are never replaced; restored beside, they may be.
    let documents = uri::of(&home.join("Documents"), true);
    let standard = ask(
        &service,
        6,
        json!({"op": "restoreVersion", "backup": backup, "uri": documents, "replace": true}),
    );
    assert_eq!(standard["error"], "refused", "{standard}");
    let beside = value(ask(
        &service,
        7,
        json!({"op": "restoreVersion", "backup": backup, "uri": documents}),
    ));
    assert_eq!(
        beside["uri"],
        uri::of(&home.join("Documents (original)"), true)
    );
    nothing_staged(&home);

    // Without `files`: nothing is asked.
    let refused = Service::with_backups(scope, NetworkAccess::Refused, BackupAccess::Refused)
        .expect("service");
    for (id, request) in [
        (1, json!({"op": "backups"})),
        (2, json!({"op": "versions", "uri": notes})),
        (
            3,
            json!({"op": "versionContents", "backup": backup, "uri": uri::of(&home, true)}),
        ),
        (
            4,
            json!({"op": "restoreVersion", "backup": backup, "uri": notes}),
        ),
    ] {
        let answer = ask(&refused, id, request);
        assert_eq!(answer["error"], "not-permitted", "{answer}");
    }
    let watch = refused.call(&json!({"op": "watchBackups"}).to_string(), &[]);
    assert!(watch.json.contains("not-permitted"), "{}", watch.json);
    // `files` granted at first use: the backups come with the person's files.
    refused.grant_the_persons_files();
    let granted = ask(&refused, 5, json!({"op": "backups"}));
    assert_ne!(granted["error"], "not-permitted", "{granted}");
    drop(backupd);
}

#[test]
fn the_backups_state_is_followed_while_watched_and_when_the_service_comes_and_goes() {
    let place = Place::new("backups-watch");
    let scope = scope_in(&place);
    let home = scope.home().to_path_buf();
    std::fs::write(home.join("notes.txt"), "notes").expect("notes");
    // No service yet: none are kept, and the watch waits for it.
    let socket = place.path("run/backupd.socket");
    let service = Service::with_backups(
        scope,
        NetworkAccess::Refused,
        BackupAccess::Granted(Backups::at(&socket)),
    )
    .expect("service");
    let none = value(ask(&service, 1, json!({"op": "backups"})));
    assert_eq!(none["destinations"], json!([]));
    assert_eq!(none["lastBackup"], Value::Null);
    let watch = serde_json::from_str::<Value>(
        &service
            .call(&json!({"op": "watchBackups"}).to_string(), &[])
            .json,
    )
    .expect("json")["value"]
        .clone();
    let heard = |what: &str| {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(Reply { id: 0, json, .. }) = service.take() {
                if serde_json::from_str::<Value>(&json).expect("json") == json!({"watch": watch}) {
                    return;
                }
            }
            assert!(Instant::now() < deadline, "never heard {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    // The service starts, its folder made after the watch began, and a destination is added.
    let run = place.dir("run");
    let backupd = {
        let engine_place = Place::new("backups-watch-service");
        let Some(started) = Backupd::start(&engine_place, &home) else {
            return;
        };
        // Its socket, where the files service looks for it.
        std::os::unix::fs::symlink(&started.socket, run.join("backupd.socket.tmp")).expect("link");
        std::fs::rename(run.join("backupd.socket.tmp"), &socket).expect("socket");
        (started, engine_place)
    };
    heard("the service with a destination");
    let state = value(ask(&service, 2, json!({"op": "backups"})));
    assert_eq!(
        state["destinations"]
            .as_array()
            .expect("destinations")
            .len(),
        1
    );
    // A backup asked for is heard as it starts and ends.
    backupd.0.back_up(2);
    heard("the backup");
    // Unwatched: followed no more.
    service.call(&json!({"op": "unwatch", "watch": watch}).to_string(), &[]);
    drop(backupd);
}
