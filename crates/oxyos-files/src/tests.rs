//! Tests on directories the tests make: a home, a media parent with a volume in it, and a mount
//! table of their own naming the volume.

use crate::entries;
use crate::places::{self, Favourite, Volume};
use crate::service::{self, Service};
use crate::trash;
use crate::watch::Watcher;
use crate::{Error, Scope};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// A directory of the test's own, removed when it ends.
struct Place {
    root: PathBuf,
}

impl Place {
    fn new(name: &str) -> Place {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "oxyos-files-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test directory");
        Place {
            root: std::fs::canonicalize(&root).expect("canonical"),
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn dir(&self, relative: &str) -> PathBuf {
        let path = self.path(relative);
        std::fs::create_dir_all(&path).expect("folder");
        path
    }

    fn file(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("folder");
        std::fs::write(&path, text).expect("file");
        path
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        // A read-only folder a test made is made writable so it can go.
        let _ = std::process::Command::new("chmod")
            .args(["-R", "u+rwx"])
            .arg(&self.root)
            .status();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A home with a volume `USB Stick` mounted for the user, as a session's scope sees them.
fn scope_in(place: &Place) -> Scope {
    let home = place.dir("home/ada");
    let media = place.dir("media/ada");
    place.dir("media/ada/USB Stick");
    let mountinfo = place.file(
        "mountinfo",
        &format!(
            "22 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw\n\
             90 22 8:17 / {}/USB\\040Stick rw,nosuid,nodev,relatime shared:50 - vfat /dev/sdb1 rw\n\
             91 22 0:5 / /mnt/elsewhere rw - tmpfs tmpfs rw\n",
            media.display()
        ),
    );
    Scope::new(
        &home,
        &home.join(".local/share"),
        &home.join(".config"),
        vec![media],
        mountinfo,
        1000,
    )
    .expect("scope")
}

fn text(path: &Path) -> &str {
    path.to_str().expect("UTF-8")
}

#[test]
fn a_path_is_taken_only_inside_the_home_and_the_volumes() {
    let place = Place::new("scope");
    let scope = scope_in(&place);
    let home = scope.home().to_path_buf();
    let documents = place.dir("home/ada/Documents");
    let outside = place.dir("outside");
    std::os::unix::fs::symlink(&outside, home.join("away")).expect("link");
    std::os::unix::fs::symlink(&documents, home.join("docs")).expect("link");

    assert_eq!(scope.folder(text(&home)).expect("home"), home);
    assert_eq!(scope.folder(text(&documents)).expect("in"), documents);
    // A link inside leading inside is where it leads.
    assert_eq!(
        scope.folder(text(&home.join("docs"))).expect("link"),
        documents
    );
    let volume = place.path("media/ada/USB Stick");
    assert_eq!(scope.folder(text(&volume)).expect("volume"), volume);

    for refused in [
        "relative/path".to_string(),
        format!("{}/../outside", documents.display()),
        format!("{}/./Documents", home.display()),
        format!("{}\0", home.display()),
        "/etc".to_string(),
        text(&outside).to_string(),
        // A link inside leading out.
        text(&home.join("away")).to_string(),
        // A media parent is not a volume, and a mount elsewhere is not the user's.
        text(&place.path("media/ada")).to_string(),
    ] {
        assert!(
            matches!(scope.folder(&refused), Err(Error::Refused(_))),
            "{refused:?} was taken"
        );
    }
    assert!(matches!(
        scope.folder(text(&home.join("missing"))),
        Err(Error::NotFound)
    ));
    let file = place.file("home/ada/notes.txt", "hi");
    assert!(matches!(scope.folder(text(&file)), Err(Error::NotAFolder)));
}

#[test]
fn an_item_is_never_the_home_a_volume_or_inside_a_trash() {
    let place = Place::new("items");
    let scope = scope_in(&place);
    let home = scope.home().to_path_buf();
    let outside = place.dir("outside");
    std::os::unix::fs::symlink(&outside, home.join("away")).expect("link");
    // The link itself is an item of the home: it can be renamed or put in the Trash.
    assert_eq!(
        scope.item(text(&home.join("away"))).expect("link"),
        home.join("away")
    );
    for refused in [
        home.clone(),
        place.path("media/ada/USB Stick"),
        place.dir("home/ada/.local/share/Trash/files"),
        place.dir("media/ada/USB Stick/.Trash-1000/files"),
        outside.join("x"),
        PathBuf::from("/"),
    ] {
        assert!(
            matches!(scope.item(text(&refused)), Err(Error::Refused(_))),
            "{} was taken",
            refused.display()
        );
    }
    let trashed = place.file("home/ada/.local/share/Trash/files/old.txt", "old");
    assert!(matches!(scope.item(text(&trashed)), Err(Error::Refused(_))));
}

#[test]
fn the_mount_table_gives_the_volumes_mounted_for_the_user() {
    let media = vec![PathBuf::from("/media/ada"), PathBuf::from("/run/media/ada")];
    let table = "22 1 8:1 / / rw - ext4 /dev/sda1 rw\n\
                 90 22 8:17 / /media/ada/USB\\040Stick rw - vfat /dev/sdb1 rw\n\
                 91 22 8:33 / /run/media/ada/Photos rw - exfat /dev/sdc1 rw\n\
                 92 22 8:33 / /run/media/ada/Photos rw - exfat /dev/sdc1 rw\n\
                 93 22 8:49 / /media/ada/deep/inside rw - ext4 /dev/sdd1 rw\n\
                 94 22 8:49 / /media/bob/Theirs rw - ext4 /dev/sde1 rw\n\
                 broken line\n";
    assert_eq!(
        places::volumes(&media, table),
        vec![
            Volume {
                name: "USB Stick".into(),
                path: "/media/ada/USB Stick".into()
            },
            Volume {
                name: "Photos".into(),
                path: "/run/media/ada/Photos".into()
            },
        ]
    );
}

#[test]
fn favourites_are_the_user_directories_that_exist_in_the_home() {
    let place = Place::new("favourites");
    let scope = scope_in(&place);
    let home = scope.home().to_path_buf();
    place.dir("home/ada/Desktop");
    place.dir("home/ada/Papers");
    place.dir("home/ada/Music");
    place.dir("outside");
    place.file(
        "home/ada/.config/user-dirs.dirs",
        &format!(
            "# written by xdg-user-dirs-update\n\
             XDG_DESKTOP_DIR=\"$HOME/Desktop\"\n\
             XDG_DOCUMENTS_DIR=\"$HOME/Papers\"\n\
             XDG_DOWNLOAD_DIR=\"$HOME/\"\n\
             XDG_PICTURES_DIR=\"{}\"\n\
             XDG_VIDEOS_DIR=\"$HOME/Videos\"\n",
            place.path("outside").display()
        ),
    );
    // Documents where the file says; Downloads disabled (the home itself); Pictures outside the
    // home left out; Music at its default; Videos not there.
    assert_eq!(
        places::favourites(&scope),
        vec![
            (Favourite::Desktop, home.join("Desktop")),
            (Favourite::Documents, home.join("Papers")),
            (Favourite::Music, home.join("Music")),
        ]
    );
}

#[test]
fn items_are_listed_created_renamed_and_moved_never_replacing_one() {
    let place = Place::new("entries");
    let scope = scope_in(&place);
    let home = scope.home().to_path_buf();
    place.file("home/ada/notes.txt", "twelve bytes");
    place.file("home/ada/.hidden", "");
    place.dir("home/ada/Projects");
    std::os::unix::fs::symlink(home.join("Projects"), home.join("work")).expect("link");
    std::os::unix::fs::symlink(home.join("gone"), home.join("broken")).expect("link");

    let mut listed = entries::list(&home).expect("list");
    listed.sort_by(|a, b| a.name.cmp(&b.name));
    let names: Vec<_> = listed.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [".hidden", "Projects", "broken", "notes.txt", "work"]
    );
    let notes = listed
        .iter()
        .find(|e| e.name == "notes.txt")
        .expect("notes");
    assert_eq!(
        (notes.size, notes.folder, notes.link, notes.hidden),
        (12, false, false, false)
    );
    assert!(notes.modified_ms > 1_600_000_000_000);
    let work = listed.iter().find(|e| e.name == "work").expect("work");
    assert!(work.folder && work.link && work.size == 0);
    let broken = listed.iter().find(|e| e.name == "broken").expect("broken");
    assert!(broken.link && !broken.folder);
    assert!(
        listed
            .iter()
            .find(|e| e.name == ".hidden")
            .expect("hidden")
            .hidden
    );

    let made = entries::create_folder(&home, "New Folder").expect("folder");
    assert!(made.folder && made.path == home.join("New Folder"));
    assert!(matches!(
        entries::create_folder(&home, "New Folder"),
        Err(Error::Exists)
    ));
    for name in ["", "  ", ".", "..", "a/b", "nul\0", &"x".repeat(256)] {
        assert!(
            matches!(entries::create_folder(&home, name), Err(Error::InvalidName)),
            "{name:?}"
        );
    }

    let renamed = entries::rename(&home.join("notes.txt"), "Notes.md").expect("rename");
    assert_eq!(renamed.path, home.join("Notes.md"));
    assert!(matches!(
        entries::rename(&home.join("Notes.md"), "Projects"),
        Err(Error::Exists)
    ));
    assert!(home.join("Notes.md").exists() && home.join("Projects").is_dir());

    let to = entries::move_into(&home.join("Notes.md"), &home.join("Projects")).expect("move");
    assert_eq!(to, home.join("Projects/Notes.md"));
    place.file("home/ada/Notes.md", "another");
    assert!(matches!(
        entries::move_into(&home.join("Notes.md"), &home.join("Projects")),
        Err(Error::Exists)
    ));
    assert_eq!(
        std::fs::read_to_string(home.join("Projects/Notes.md")).expect("kept"),
        "twelve bytes"
    );
    assert!(matches!(
        entries::move_into(&home.join("Projects"), &home.join("Projects")),
        Err(Error::Refused(_))
    ));
    place.dir("home/ada/Projects/Inner");
    assert!(matches!(
        entries::move_into(&home.join("Projects"), &home.join("Projects/Inner")),
        Err(Error::Refused(_))
    ));
}

#[test]
fn trash_paths_are_percent_encoded_as_the_specification_stores_them() {
    let path = "/home/ada/Año nuevo/50% #1.txt";
    let encoded = trash::encode(path);
    assert_eq!(encoded, "/home/ada/A%C3%B1o%20nuevo/50%25%20%231.txt");
    assert_eq!(trash::decode(&encoded), path);
    assert_eq!(trash::decode("100%"), "100%");
    assert_eq!(trash::decode("%zz%4"), "%zz%4");
}

#[test]
fn an_item_goes_to_the_homes_trash_and_back() {
    let place = Place::new("trash");
    let scope = scope_in(&place);
    let home = scope.home().to_path_buf();
    let report = place.file("home/ada/Documents/Año report.txt", "first");
    let id = trash::trash(&scope, &report).expect("trash");
    let trash_dir = home.join(".local/share/Trash");
    assert_eq!(id, trash_dir.join("files/Año report.txt"));
    assert!(!report.exists());
    let info =
        std::fs::read_to_string(trash_dir.join("info/Año report.txt.trashinfo")).expect("info");
    let lines: Vec<_> = info.lines().collect();
    assert_eq!(lines[0], "[Trash Info]");
    assert_eq!(lines[1], format!("Path={}", trash::encode(text(&report))));
    let date = lines[2].strip_prefix("DeletionDate=").expect("date");
    assert_eq!(date.len(), 19, "{date}");
    assert!(date.parse::<jiff::civil::DateTime>().is_ok(), "{date}");
    let mode = std::fs::metadata(&trash_dir)
        .expect("trash")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);

    // Another item of the same name takes a number.
    let again = place.file("home/ada/Año report.txt", "second");
    let second = trash::trash(&scope, &again).expect("trash");
    assert_eq!(second, trash_dir.join("files/Año report 2.txt"));
    assert_eq!(trash::count(&scope), 2);

    let listed = trash::list(&scope);
    assert_eq!(listed.len(), 2);
    let first = listed.iter().find(|item| item.id == id).expect("first");
    let numbered = listed
        .iter()
        .find(|item| item.id == second)
        .expect("second");
    assert_eq!(first.original.as_deref(), Some(report.as_path()));
    assert_eq!(
        (first.name.as_str(), first.entry.name.as_str()),
        ("Año report.txt", "Año report.txt")
    );
    assert_eq!(numbered.name, "Año report.txt");
    assert_eq!(numbered.entry.name, "Año report 2.txt");
    assert_eq!(numbered.original.as_deref(), Some(again.as_path()));
    let deleted = first.deleted_ms.expect("deleted");
    let now = jiff::Timestamp::now().as_millisecond();
    assert!((now - deleted).abs() < 5_000, "{deleted} vs {now}");

    // Back where it was, its info gone; not over an item there now.
    assert_eq!(trash::restore(&scope, text(&id)).expect("restore"), report);
    assert_eq!(std::fs::read_to_string(&report).expect("back"), "first");
    assert!(!trash_dir.join("info/Año report.txt.trashinfo").exists());
    place.file("home/ada/Año report.txt", "third");
    assert!(matches!(
        trash::restore(&scope, text(&second)),
        Err(Error::Exists)
    ));
    assert_eq!(
        std::fs::read_to_string(home.join("Año report.txt")).expect("kept"),
        "third"
    );

    // Removed for good, only from the Trash; ids outside it are refused.
    trash::remove(&scope, text(&second)).expect("remove");
    assert_eq!(trash::count(&scope), 0);
    for refused in [
        text(&home.join("Año report.txt")).to_string(),
        format!("{}/files/../info/x", trash_dir.display()),
        "/etc/passwd".to_string(),
    ] {
        assert!(
            matches!(trash::remove(&scope, &refused), Err(Error::Refused(_))),
            "{refused}"
        );
    }
    assert!(home.join("Año report.txt").exists());
}

#[test]
fn emptying_the_trash_deletes_what_it_holds_even_read_only_folders() {
    let place = Place::new("empty");
    let scope = scope_in(&place);
    let folder = place.dir("home/ada/Old");
    place.file("home/ada/Old/inner/a.txt", "a");
    std::fs::set_permissions(folder.join("inner"), std::fs::Permissions::from_mode(0o500))
        .expect("mode");
    trash::trash(&scope, &folder).expect("trash");
    trash::trash(&scope, &place.file("home/ada/b.txt", "b")).expect("trash");
    // An info left without its item goes too.
    place.file(
        "home/ada/.local/share/Trash/info/orphan.trashinfo",
        "[Trash Info]\n",
    );
    assert_eq!(trash::empty(&scope).expect("empty"), 2);
    assert_eq!(trash::count(&scope), 0);
    let infos = std::fs::read_dir(place.path("home/ada/.local/share/Trash/info")).expect("info");
    assert_eq!(infos.count(), 0);
}

#[test]
fn an_item_on_a_volume_goes_to_that_volumes_trash() {
    let place = Place::new("volume-trash");
    let scope = scope_in(&place);
    let volume = place.path("media/ada/USB Stick");
    let photo = place.file("media/ada/USB Stick/DCIM/one.jpg", "jpeg");
    // The second method: .Trash-UID at the volume's root, paths relative to it.
    let id = trash::trash(&scope, &photo).expect("trash");
    assert_eq!(id, volume.join(".Trash-1000/files/one.jpg"));
    let info =
        std::fs::read_to_string(volume.join(".Trash-1000/info/one.jpg.trashinfo")).expect("info");
    assert!(info.contains("\nPath=DCIM/one.jpg\n"), "{info}");
    let listed = trash::list(&scope);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].original.as_deref(), Some(photo.as_path()));
    assert_eq!(trash::restore(&scope, text(&id)).expect("restore"), photo);

    // The first method, when the volume has a sticky .Trash: .Trash/UID.
    let shared = place.dir("media/ada/USB Stick/.Trash");
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).expect("sticky");
    let id = trash::trash(&scope, &photo).expect("trash");
    assert_eq!(id, shared.join("1000/files/one.jpg"));
    assert_eq!(trash::count(&scope), 1);
    // A .Trash that is not sticky is not used.
    trash::empty(&scope).expect("empty");
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).expect("not sticky");
    let other = place.file("media/ada/USB Stick/two.jpg", "jpeg");
    assert_eq!(
        trash::trash(&scope, &other).expect("trash"),
        volume.join(".Trash-1000/files/two.jpg")
    );
}

fn changes(watcher_events: &mpsc::Receiver<Vec<u32>>) -> Vec<u32> {
    let mut all = Vec::new();
    while let Ok(ids) = watcher_events.recv_timeout(Duration::from_millis(300)) {
        all.extend(ids);
    }
    all.sort_unstable();
    all.dedup();
    all
}

#[test]
fn a_watched_folder_says_when_its_items_change() {
    let place = Place::new("watch");
    let scope = scope_in(&place);
    let documents = place.dir("home/ada/Documents");
    let music = place.dir("home/ada/Music");
    let (told, heard) = mpsc::channel();
    let watcher = Watcher::new(scope.mountinfo(), move |ids| {
        let _ = told.send(ids);
    })
    .expect("watcher");
    let docs = watcher.watch(&[documents.clone()], false).expect("watch");
    let tunes = watcher.watch(&[music.clone()], false).expect("watch");
    // The same folder twice: two watches, one inotify descriptor.
    let again = watcher.watch(&[documents.clone()], false).expect("watch");

    for _ in 0..20 {
        std::fs::write(documents.join("a.txt"), "x").expect("write");
    }
    assert_eq!(changes(&heard), vec![docs, again]);
    std::fs::rename(documents.join("a.txt"), music.join("a.txt")).expect("move");
    assert_eq!(changes(&heard), vec![docs, tunes, again]);
    watcher.unwatch(docs);
    std::fs::remove_file(music.join("a.txt")).expect("remove");
    std::fs::create_dir(documents.join("New")).expect("create");
    assert_eq!(changes(&heard), vec![tunes, again]);
    watcher.unwatch(again);
    std::fs::remove_dir(documents.join("New")).expect("remove");
    assert_eq!(changes(&heard), Vec::<u32>::new());
    assert_eq!(watcher.len(), 1);
    // A folder that goes says so.
    std::fs::remove_dir(&music).expect("remove");
    assert_eq!(changes(&heard), vec![tunes]);
}

/// Requests through the service, each answer read from its queue as the host reads it.
fn ask(service: &Service, request: serde_json::Value) -> serde_json::Value {
    service.request(&request.to_string());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(answer) = service.take() {
            let answer: serde_json::Value = serde_json::from_str(&answer).expect("json");
            if answer.get("id") == request.get("id") {
                return answer;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no answer to {request}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_service_answers_requests_in_order_with_their_ids() {
    let place = Place::new("service");
    let scope = scope_in(&place);
    let home = scope.home().to_path_buf();
    place.dir("home/ada/Documents");
    place.file("home/ada/Documents/a.txt", "abc");
    let service = Service::new(scope).expect("service");

    let places = ask(&service, serde_json::json!({"id": 1, "op": "places"}));
    assert_eq!(places["ok"], true);
    assert_eq!(places["value"]["home"], text(&home));
    assert_eq!(places["value"]["favourites"][0]["key"], "documents");
    assert_eq!(places["value"]["volumes"][0]["name"], "USB Stick");
    assert_eq!(places["value"]["trash"], 0);

    let listed = ask(
        &service,
        serde_json::json!({"id": 2, "op": "list", "path": home.join("Documents")}),
    );
    assert_eq!(listed["value"]["entries"][0]["name"], "a.txt");
    assert_eq!(listed["value"]["entries"][0]["size"], 3);

    let refused = ask(
        &service,
        serde_json::json!({"id": 3, "op": "list", "path": "/etc"}),
    );
    assert_eq!(
        (refused["ok"].clone(), refused["error"].clone()),
        (false.into(), "refused".into())
    );
    let unknown = ask(&service, serde_json::json!({"id": 4, "op": "format"}));
    assert_eq!(unknown["error"], "refused");
    let garbage = service::answer(
        &Scope::new(
            &home,
            &home,
            &home,
            vec![],
            PathBuf::from("/nonexistent"),
            1000,
        )
        .expect("scope"),
        &Watcher::new(Path::new("/nonexistent"), |_| {}).expect("watcher"),
        "not json",
    );
    assert!(garbage.contains("\"ok\":false"), "{garbage}");

    // Several at once: each its own result.
    let trashed = ask(
        &service,
        serde_json::json!({"id": 5, "op": "trash", "paths": [home.join("Documents/a.txt"), "/etc/passwd", home]}),
    );
    let results = trashed["value"].as_array().expect("results");
    assert_eq!(results[0]["ok"], true);
    assert_eq!(results[1]["error"], "refused");
    assert_eq!(results[2]["error"], "refused");
    let listed = ask(&service, serde_json::json!({"id": 6, "op": "trashList"}));
    assert_eq!(listed["value"][0]["name"], "a.txt");
    assert_eq!(
        listed["value"][0]["original"],
        text(&home.join("Documents/a.txt"))
    );

    // A watch says when its folder changes, as {"changed": N} in the same queue.
    let watch = ask(
        &service,
        serde_json::json!({"id": 7, "op": "watch", "path": home.join("Documents")}),
    );
    let number = watch["value"].as_u64().expect("watch");
    let made = ask(
        &service,
        serde_json::json!({"id": 8, "op": "createFolder", "parent": home.join("Documents"), "name": "New Folder"}),
    );
    assert_eq!(made["value"]["folder"], true);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(answer) = service.take() {
            let answer: serde_json::Value = serde_json::from_str(&answer).expect("json");
            if answer["changed"].as_u64() == Some(number) {
                break;
            }
        }
        assert!(std::time::Instant::now() < deadline, "no change said");
        std::thread::sleep(Duration::from_millis(5));
    }
}
