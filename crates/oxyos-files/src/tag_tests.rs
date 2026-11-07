//! Tags (docs/adr/0049-file-tags.mdx) on real files the tests make: the attributes themselves, a
//! Mac's property list, copies, moves and the Trash keeping them, and the index following them.
//! A test skips, saying so, where the temporary directory's file system keeps no `user.`
//! attributes.

use crate::access::{self, Relocation};
use crate::tag_index::TagIndex;
use crate::tags::{self, Color, Tag, MAC, XDG};
use crate::tests::{scope_in, text, Place};
use crate::{trash, uri, Error, Scope};
use rustix::fs::XattrFlags;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Whether `place`'s file system keeps `user.` attributes; a test without them skips, saying so.
fn keeps_attributes(place: &Place) -> bool {
    let probe = place.file("probe", "");
    let kept = rustix::fs::setxattr(&probe, "user.oxyos.probe", b"1", XattrFlags::empty()).is_ok();
    let _ = std::fs::remove_file(probe);
    if !kept {
        eprintln!(
            "skipped: {} keeps no user. extended attributes",
            place.root.display()
        );
    }
    kept
}

fn set(path: &Path, name: &str, value: &[u8]) {
    rustix::fs::setxattr(path, name, value, XattrFlags::empty()).expect("setxattr");
}

fn get(path: &Path, name: &str) -> Option<Vec<u8>> {
    let mut value = vec![0u8; 4096];
    match rustix::fs::getxattr(path, name, &mut value[..]) {
        Ok(length) => {
            value.truncate(length);
            Some(value)
        }
        Err(rustix::io::Errno::NODATA) => None,
        Err(errno) => panic!("getxattr {name}: {errno}"),
    }
}

fn xdg(path: &Path) -> Option<String> {
    get(path, XDG).map(|value| String::from_utf8(value).expect("UTF-8"))
}

/// A Mac's value as Finder writes it: a binary property list of strings.
fn bplist(items: &[&str]) -> Vec<u8> {
    let list = plist::Value::Array(
        items
            .iter()
            .map(|item| plist::Value::String(item.to_string()))
            .collect(),
    );
    let mut bytes = Vec::new();
    list.to_writer_binary(&mut bytes).expect("bplist");
    bytes
}

fn mac(path: &Path) -> Option<Vec<String>> {
    let value = get(path, MAC)?;
    assert!(value.starts_with(b"bplist00"), "a binary property list");
    let list = plist::Value::from_reader(std::io::Cursor::new(value)).expect("plist");
    Some(
        list.as_array()
            .expect("array")
            .iter()
            .map(|item| item.as_string().expect("string").to_string())
            .collect(),
    )
}

fn names(tags: &[Tag]) -> Vec<&str> {
    tags.iter().map(|tag| tag.name.as_str()).collect()
}

fn tag(name: &str) -> Tag {
    Tag::new(name, None)
}

#[test]
fn tags_are_written_as_freedesktop_says_and_compared_case_aside() {
    let place = Place::new("tags-xdg");
    if !keeps_attributes(&place) {
        return;
    }
    let file = place.file("report.txt", "report");
    assert_eq!(tags::read(&file).unwrap(), vec![]);
    let now = tags::change(&file, &[tag("Work"), tag(" Urgent "), tag("work")], &[]).unwrap();
    assert_eq!(names(&now), ["Work", "Urgent"]);
    assert_eq!(xdg(&file).as_deref(), Some("Work,Urgent"));
    // The first spelling is kept; a colour given is not, the item having no Mac's attribute.
    let now = tags::change(&file, &[Tag::new("WORK", Some(Color::Red))], &[]).unwrap();
    assert_eq!(names(&now), ["Work", "Urgent"]);
    assert_eq!(now[0].color, None);
    // Removed case aside; removing the last removes the attribute.
    assert_eq!(
        names(&tags::change(&file, &[], &["urgent".into()]).unwrap()),
        ["Work"]
    );
    assert_eq!(xdg(&file).as_deref(), Some("Work"));
    tags::change(&file, &[], &["WORK".into()]).unwrap();
    assert_eq!(xdg(&file), None);
    // Remove first, then add: a tag respelt in place, one removed and added again still there.
    tags::change(&file, &[tag("work"), tag("Home"), tag("Later")], &[]).unwrap();
    let now = tags::change(
        &file,
        &[tag("Work"), tag("Later")],
        &["work".into(), "later".into()],
    )
    .unwrap();
    assert_eq!(names(&now), ["Work", "Home", "Later"]);
    assert_eq!(xdg(&file).as_deref(), Some("Work,Home,Later"));
    // A name removed and a new one added: renamed in place, as Finder renames a tag.
    let now = tags::change(&file, &[tag("Office")], &["home".into()]).unwrap();
    assert_eq!(names(&now), ["Work", "Office", "Later"]);
    // Every name removed and the new list added: that list, in its order.
    let now = tags::change(
        &file,
        &[tag("Desk"), tag("Work"), tag("Later"), tag("Extra")],
        &["work".into(), "office".into(), "later".into()],
    )
    .unwrap();
    assert_eq!(names(&now), ["Work", "Desk", "Later", "Extra"]);
    // A new name with nothing removed goes last; one removed that is not there changes nothing.
    let now = tags::change(&file, &[tag("Last")], &["absent".into()]).unwrap();
    assert_eq!(names(&now), ["Work", "Desk", "Later", "Extra", "Last"]);
    tags::change(&file, &[], &["desk".into(), "extra".into(), "last".into()]).unwrap();
    tags::change(&file, &[tag("Home")], &["later".into()]).unwrap();
    assert_eq!(xdg(&file).as_deref(), Some("Work,Home"));
    tags::change(&file, &[tag("Later")], &[]).unwrap();
    // What another program wrote is read leniently: spaces and empty names left out, once each.
    set(&file, XDG, b" Blue ,,blue,Green,");
    assert_eq!(names(&tags::read(&file).unwrap()), ["Blue", "Green"]);
}

#[test]
fn a_tags_name_is_checked() {
    for bad in [
        "",
        "   ",
        "a,b",
        "two\nlines",
        "tab\there",
        &"x".repeat(256),
    ] {
        assert!(
            matches!(tags::valid(bad), Err(Error::Invalid(_))),
            "{bad:?} taken"
        );
    }
    assert_eq!(tags::valid("  Été  ").unwrap(), "Été");
    assert_eq!(tags::valid(&"é".repeat(127)).unwrap().len(), 254);
    let place = Place::new("tags-invalid");
    if !keeps_attributes(&place) {
        return;
    }
    let file = place.file("a.txt", "a");
    // Nothing changes when one of the names cannot be a tag's.
    assert!(tags::change(&file, &[tag("Fine"), tag("not,fine")], &[]).is_err());
    assert_eq!(xdg(&file), None);
    assert!(matches!(
        tags::change(&place.path("gone.txt"), &[tag("Fine")], &[]),
        Err(Error::NotFound)
    ));
}

#[test]
fn a_macs_tags_are_read_and_kept_true_when_changed() {
    let place = Place::new("tags-mac");
    if !keeps_attributes(&place) {
        return;
    }
    let file = place.file("from a Mac.pages", "pages");
    set(
        &file,
        MAC,
        &bplist(&["Red\n6", "Project X", "Blue\n4", "red\n6"]),
    );
    let read = tags::read(&file).unwrap();
    assert_eq!(
        read,
        vec![
            Tag::new("Red", Some(Color::Red)),
            Tag::new("Project X", None),
            Tag::new("Blue", Some(Color::Blue)),
        ]
    );
    // A change writes both: freedesktop's, and the Mac's with the change's colours.
    let now = tags::change(
        &file,
        &[
            Tag::new("Green", Some(Color::Green)),
            Tag::new("project x", Some(Color::Purple)),
        ],
        &["blue".into()],
    )
    .unwrap();
    assert_eq!(names(&now), ["Red", "Project X", "Green"]);
    assert_eq!(xdg(&file).as_deref(), Some("Red,Project X,Green"));
    assert_eq!(mac(&file).unwrap(), ["Red\n6", "Project X\n3", "Green\n2"]);
    // Read back: freedesktop's names, the Mac's colours.
    assert_eq!(tags::read(&file).unwrap(), now);
    // freedesktop's wins, even empty.
    set(&file, XDG, b"");
    assert_eq!(tags::read(&file).unwrap(), vec![]);
    // Removing every tag removes both attributes.
    set(&file, XDG, b"Red");
    tags::change(&file, &[], &["Red".into()]).unwrap();
    assert_eq!((xdg(&file), mac(&file)), (None, None));
    // An item without the Mac's attribute never gains it.
    let other = place.file("plain.txt", "plain");
    tags::change(&other, &[Tag::new("Red", Some(Color::Red))], &[]).unwrap();
    assert_eq!((xdg(&other).as_deref(), mac(&other)), (Some("Red"), None));
    // A value that is not a property list says no tags, and a change replaces it.
    let garbled = place.file("garbled.txt", "g");
    set(&garbled, MAC, b"not a plist");
    assert_eq!(tags::read(&garbled).unwrap(), vec![]);
    tags::change(&garbled, &[Tag::new("Work", None)], &[]).unwrap();
    assert_eq!(mac(&garbled).unwrap(), ["Work"]);
}

#[test]
fn a_links_tags_are_what_it_leads_tos() {
    let place = Place::new("tags-link");
    if !keeps_attributes(&place) {
        return;
    }
    let file = place.file("real.txt", "real");
    let link = place.path("link.txt");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    tags::change(&link, &[tag("Linked")], &[]).unwrap();
    assert_eq!(xdg(&file).as_deref(), Some("Linked"));
    assert_eq!(names(&tags::read(&link).unwrap()), ["Linked"]);
}

/// A tagged file, a read-only tagged file, and a tagged folder holding them, with another
/// `user.` attribute beside the tags.
fn tagged_tree(place: &Place, at: &str) -> PathBuf {
    let folder = place.dir(at);
    let file = place.file(&format!("{at}/notes.txt"), "notes");
    let locked = place.file(&format!("{at}/Inner/locked.txt"), "locked");
    tags::change(&folder, &[tag("Project")], &[]).unwrap();
    tags::change(&file, &[tag("Work")], &[]).unwrap();
    set(&file, "user.xdg.comment", b"a comment");
    tags::change(&locked, &[tag("Keep")], &[]).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o444)).unwrap();
    folder
}

fn assert_tagged_tree(folder: &Path) {
    assert_eq!(xdg(folder).as_deref(), Some("Project"));
    assert_eq!(xdg(&folder.join("notes.txt")).as_deref(), Some("Work"));
    assert_eq!(
        get(&folder.join("notes.txt"), "user.xdg.comment").as_deref(),
        Some(&b"a comment"[..])
    );
    let locked = folder.join("Inner/locked.txt");
    assert_eq!(xdg(&locked).as_deref(), Some("Keep"));
    assert_eq!(
        std::fs::metadata(&locked).unwrap().permissions().mode() & 0o777,
        0o444
    );
}

fn relocation<'a>(from: &'a str, to: &'a str) -> Relocation<'a> {
    Relocation {
        uri: from,
        directory: true,
        to,
        to_directory: true,
        overwrite: false,
    }
}

#[test]
fn copies_moves_and_the_trash_keep_an_items_attributes() {
    let place = Place::new("tags-integrity");
    if !keeps_attributes(&place) {
        return;
    }
    let scope = scope_in(&place);
    let folder = tagged_tree(&place, "home/ada/Projects");
    let home = place.path("home/ada");
    let documents = place.dir("home/ada/Documents");
    // A copy, the folder and everything in it.
    let from = uri::of(&folder, true);
    let copied = access::copy(&scope, &relocation(&from, &uri::of(&documents, true))).unwrap();
    assert_eq!(copied, uri::of(&documents.join("Projects"), true));
    assert_tagged_tree(&documents.join("Projects"));
    // A file copied alone.
    let file_uri = uri::of(&folder.join("notes.txt"), false);
    let to = uri::of(&home.join("notes copy.txt"), false);
    access::copy(
        &scope,
        &Relocation {
            directory: false,
            to_directory: false,
            ..relocation(&file_uri, &to)
        },
    )
    .unwrap();
    assert_eq!(xdg(&home.join("notes copy.txt")).as_deref(), Some("Work"));
    // A rename.
    let renamed = access::rename(
        &scope,
        &uri::of(&documents.join("Projects"), true),
        true,
        "Old",
    )
    .unwrap();
    assert!(renamed.ends_with("/Documents/Old/"));
    assert_tagged_tree(&documents.join("Old"));
    // The Trash, and back.
    let id = trash::trash(&scope, &documents.join("Old")).unwrap();
    assert_tagged_tree(&id);
    let back = trash::restore(&scope, text(&id)).unwrap();
    assert_eq!(back, documents.join("Old"));
    assert_tagged_tree(&back);
    // A move to another volume: a copy, then the original deleted.
    let Some(elsewhere) = other_file_system(&place) else {
        return;
    };
    let volume_scope = Scope::new(
        &home,
        &home.join(".local/share"),
        &home.join(".config"),
        vec![elsewhere.parent().unwrap().to_path_buf()],
        place.file(
            "mountinfo-elsewhere",
            &format!("90 22 0:99 / {} rw - tmpfs tmpfs rw\n", elsewhere.display()),
        ),
        1000,
    )
    .unwrap();
    let moved = access::relocate(
        &volume_scope,
        &relocation(&uri::of(&back, true), &uri::of(&elsewhere, true)),
    )
    .unwrap();
    assert_eq!(moved, uri::of(&elsewhere.join("Old"), true));
    assert!(!back.exists());
    assert_tagged_tree(&elsewhere.join("Old"));
    let _ = std::process::Command::new("chmod")
        .args(["-R", "u+rwx"])
        .arg(&elsewhere)
        .status();
    let _ = std::fs::remove_dir_all(elsewhere.parent().unwrap());
}

/// A folder of the test's own on another file system than `place`'s (/dev/shm), as a volume
/// mounted for the person would be: `…/media/Elsewhere`. None (the move between volumes not
/// tried) where there is none that keeps attributes.
fn other_file_system(place: &Place) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let shm = Path::new("/dev/shm");
    let device = |path: &Path| std::fs::metadata(path).map(|found| found.dev()).ok();
    if device(shm).is_none() || device(shm) == device(&place.root) {
        eprintln!("skipped: no /dev/shm on another file system than the temporary directory's");
        return None;
    }
    let media = shm.join(format!(
        "oxyos-files-tags-{}-{}",
        std::process::id(),
        place.root.file_name()?.to_str()?
    ));
    let folder = media.join("Elsewhere");
    std::fs::create_dir_all(&folder).ok()?;
    if rustix::fs::setxattr(&folder, "user.oxyos.probe", b"1", XattrFlags::empty()).is_err() {
        eprintln!("skipped: /dev/shm keeps no user. extended attributes");
        let _ = std::fs::remove_dir_all(&media);
        return None;
    }
    let _ = rustix::fs::removexattr(&folder, "user.oxyos.probe");
    std::fs::canonicalize(folder).ok()
}

/// An index of `scope` whose changes are counted.
fn index_of(scope: &Scope) -> (TagIndex, Arc<AtomicUsize>) {
    let changes = Arc::new(AtomicUsize::new(0));
    let counted = changes.clone();
    let index = TagIndex::start(scope.clone(), move || {
        counted.fetch_add(1, Ordering::SeqCst);
    })
    .expect("index");
    (index, changes)
}

/// What the index answers for `name`: each item's path relative to `root`, and its tags.
fn tagged(index: &TagIndex, root: &Path, name: &str) -> Vec<(String, Vec<String>)> {
    let (sent, received) = mpsc::channel();
    index.tagged(name, move |found| {
        let _ = sent.send(found);
    });
    received
        .recv_timeout(Duration::from_secs(10))
        .expect("an answer")
        .into_iter()
        .map(|(path, tags)| {
            // Each path exactly as the item's (one ending in `/` names no file).
            let text = path.to_str().expect("UTF-8");
            assert!(!text.ends_with('/'), "the index holds {text}");
            (
                path.strip_prefix(root).unwrap().display().to_string(),
                tags.into_iter().map(|tag| tag.name).collect(),
            )
        })
        .collect()
}

fn tag_names(index: &TagIndex) -> Vec<(String, usize)> {
    let (sent, received) = mpsc::channel();
    index.names(move |names| {
        let _ = sent.send(names);
    });
    received
        .recv_timeout(Duration::from_secs(10))
        .expect("an answer")
}

/// Waits until the index answers `name` with `want`, and says it changed since `before`.
fn until(
    index: &TagIndex,
    root: &Path,
    name: &str,
    want: &[(&str, &[&str])],
    changes: &AtomicUsize,
    before: usize,
) {
    let want: Vec<(String, Vec<String>)> = want
        .iter()
        .map(|(path, tags)| {
            (
                path.to_string(),
                tags.iter().map(|tag| tag.to_string()).collect(),
            )
        })
        .collect();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let found = tagged(index, root, name);
        if found == want && changes.load(Ordering::SeqCst) > before {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the index has {found:?} for {name}, not {want:?} (changes {} since {before})",
            changes.load(Ordering::SeqCst)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_index_finds_tagged_items_in_the_places_and_leaves_out_what_baloo_does() {
    let place = Place::new("tags-index");
    if !keeps_attributes(&place) {
        return;
    }
    let scope = scope_in(&place);
    let root = place.root.clone();
    for (path, name) in [
        ("home/ada/Documents/report.txt", "Work"),
        ("home/ada/Documents/Plans", "work"),
        ("home/ada/Documents/Plans/q3.txt", "Urgent"),
        ("home/ada/.hidden.txt", "Work"),
        ("home/ada/.config/app/settings.json", "Work"),
        ("home/ada/code/node_modules/pkg/index.js", "Work"),
        ("home/ada/code/__pycache__/x.pyc", "Work"),
        ("home/ada/.local/share/Trash/files/old.txt", "Work"),
        ("media/ada/USB Stick/photo.jpg", "Work"),
        ("media/ada/USB Stick/.Trash-1000/files/gone.jpg", "Work"),
        ("elsewhere/outside.txt", "Work"),
    ] {
        let path = if name == "work" {
            place.dir(path)
        } else {
            place.file(path, "x")
        };
        tags::change(&path, &[tag(name)], &[]).unwrap();
    }
    let link = place.path("home/ada/link to outside.txt");
    std::os::unix::fs::symlink(place.path("elsewhere/outside.txt"), &link).unwrap();
    let (index, _) = index_of(&scope);
    assert_eq!(
        tagged(&index, &root, "WORK"),
        vec![
            ("home/ada/.hidden.txt".into(), vec!["Work".into()]),
            ("home/ada/Documents/Plans".into(), vec!["work".into()]),
            ("home/ada/Documents/report.txt".into(), vec!["Work".into()]),
            ("media/ada/USB Stick/photo.jpg".into(), vec!["Work".into()]),
        ]
    );
    assert!(index.ready());
    assert!(!index.partial());
    // Each name once, case aside, spelt as the first item by path spells it.
    assert_eq!(
        tag_names(&index),
        vec![("Urgent".into(), 1), ("Work".into(), 4)]
    );
}

#[test]
fn the_index_follows_tags_changed_items_moved_and_the_trash() {
    let place = Place::new("tags-index-live");
    if !keeps_attributes(&place) {
        return;
    }
    let scope = scope_in(&place);
    let root = place.root.clone();
    let documents = place.dir("home/ada/Documents");
    let report = place.file("home/ada/Documents/report.txt", "report");
    let (index, changes) = index_of(&scope);
    assert_eq!(tagged(&index, &root, "Work"), vec![]);
    let seen = |count: &AtomicUsize| count.load(Ordering::SeqCst);

    // Another program tags it (setfattr).
    let before = seen(&changes);
    set(&report, XDG, b"Work,Draft");
    until(
        &index,
        &root,
        "work",
        &[("home/ada/Documents/report.txt", &["Work", "Draft"])],
        &changes,
        before,
    );
    // Renamed: the index has it under its new name.
    let before = seen(&changes);
    std::fs::rename(&report, documents.join("final.txt")).unwrap();
    until(
        &index,
        &root,
        "Work",
        &[("home/ada/Documents/final.txt", &["Work", "Draft"])],
        &changes,
        before,
    );
    // A folder moved, with what it holds: still followed where it went.
    let before = seen(&changes);
    std::fs::rename(&documents, place.path("home/ada/Papers")).unwrap();
    until(
        &index,
        &root,
        "Work",
        &[("home/ada/Papers/final.txt", &["Work", "Draft"])],
        &changes,
        before,
    );
    let before = seen(&changes);
    set(&place.path("home/ada/Papers/final.txt"), XDG, b"Work");
    until(
        &index,
        &root,
        "Work",
        &[("home/ada/Papers/final.txt", &["Work"])],
        &changes,
        before,
    );
    // A folder made with a tagged file in it, moved in from outside the places: crawled.
    let outside = place.dir("elsewhere/Incoming");
    let incoming = place.file("elsewhere/Incoming/new.txt", "new");
    set(&incoming, XDG, b"Work");
    let before = seen(&changes);
    std::fs::rename(&outside, place.path("home/ada/Papers/Incoming")).unwrap();
    until(
        &index,
        &root,
        "Work",
        &[
            ("home/ada/Papers/Incoming/new.txt", &["Work"]),
            ("home/ada/Papers/final.txt", &["Work"]),
        ],
        &changes,
        before,
    );
    // Into the Trash: gone from the index; restored: back.
    let before = seen(&changes);
    let id = trash::trash(&scope, &place.path("home/ada/Papers/final.txt")).unwrap();
    until(
        &index,
        &root,
        "Work",
        &[("home/ada/Papers/Incoming/new.txt", &["Work"])],
        &changes,
        before,
    );
    let before = seen(&changes);
    trash::restore(&scope, text(&id)).unwrap();
    until(
        &index,
        &root,
        "Work",
        &[
            ("home/ada/Papers/Incoming/new.txt", &["Work"]),
            ("home/ada/Papers/final.txt", &["Work"]),
        ],
        &changes,
        before,
    );
    // Made hidden: left out, as hidden folders are.
    let before = seen(&changes);
    std::fs::rename(
        place.path("home/ada/Papers/Incoming"),
        place.path("home/ada/Papers/.Incoming"),
    )
    .unwrap();
    until(
        &index,
        &root,
        "Work",
        &[("home/ada/Papers/final.txt", &["Work"])],
        &changes,
        before,
    );
    // Untagged, then deleted.
    let before = seen(&changes);
    tags::change(
        &place.path("home/ada/Papers/final.txt"),
        &[],
        &["work".into()],
    )
    .unwrap();
    until(&index, &root, "Work", &[], &changes, before);
    assert_eq!(tag_names(&index), vec![]);
    let tagged_again = place.file("home/ada/Papers/again.txt", "again");
    let before = seen(&changes);
    set(&tagged_again, XDG, b"Red");
    until(
        &index,
        &root,
        "red",
        &[("home/ada/Papers/again.txt", &["Red"])],
        &changes,
        before,
    );
    let before = seen(&changes);
    std::fs::remove_file(&tagged_again).unwrap();
    until(&index, &root, "red", &[], &changes, before);
}

#[test]
fn the_index_follows_volumes_mounted_and_unmounted() {
    let place = Place::new("tags-index-volumes");
    if !keeps_attributes(&place) {
        return;
    }
    let scope = scope_in(&place);
    let root = place.root.clone();
    let photo = place.file("media/ada/Camera/photo.jpg", "jpeg");
    set(&photo, XDG, b"Holiday");
    let (index, changes) = index_of(&scope);
    assert_eq!(tagged(&index, &root, "Holiday"), vec![]);
    // The camera mounted: the mount table says it, and the index crawls it.
    let mountinfo = place.path("mountinfo");
    let table = std::fs::read_to_string(&mountinfo).unwrap();
    std::fs::write(
        &mountinfo,
        format!(
            "{table}92 22 8:33 / {} rw - vfat /dev/sdc1 rw\n",
            place.path("media/ada/Camera").display()
        ),
    )
    .unwrap();
    let before = changes.load(Ordering::SeqCst);
    index.volumes_changed();
    until(
        &index,
        &root,
        "holiday",
        &[("media/ada/Camera/photo.jpg", &["Holiday"])],
        &changes,
        before,
    );
    // Unmounted: forgotten.
    std::fs::write(&mountinfo, table).unwrap();
    let before = changes.load(Ordering::SeqCst);
    index.volumes_changed();
    until(&index, &root, "holiday", &[], &changes, before);
}

#[test]
fn a_question_asked_while_the_index_crawls_is_answered_when_it_ends() {
    let place = Place::new("tags-index-wait");
    if !keeps_attributes(&place) {
        return;
    }
    let scope = scope_in(&place);
    for folder in 0..40 {
        for file in 0..25 {
            place.file(&format!("home/ada/Many/f{folder}/{file}.txt"), "x");
        }
    }
    let last = place.path("home/ada/Many/f39/24.txt");
    set(&last, XDG, b"Found");
    let (index, _) = index_of(&scope);
    // Asked at once: answered once the crawl ended, with what it found.
    let found = tagged(&index, &place.root, "found");
    assert_eq!(
        found,
        vec![("home/ada/Many/f39/24.txt".into(), vec!["Found".into()])]
    );
}

#[test]
fn the_index_follows_the_services_own_copies_moves_and_renames() {
    let place = Place::new("tags-index-service");
    if !keeps_attributes(&place) {
        return;
    }
    let scope = scope_in(&place);
    let root = place.root.clone();
    let documents = place.dir("home/ada/Documents");
    let downloads = place.dir("home/ada/Downloads");
    let projects = place.dir("home/ada/Documents/Projects");
    let report = place.file("home/ada/Documents/report.txt", "report");
    let plan = place.file("home/ada/Documents/plan.txt", "plan");
    let (index, changes) = index_of(&scope);
    assert_eq!(tagged(&index, &root, "Q3"), vec![]);
    let before = changes.load(Ordering::SeqCst);
    tags::change(&report, &[tag("Q3")], &[]).unwrap();
    tags::change(&plan, &[tag("Q3")], &[]).unwrap();
    until(
        &index,
        &root,
        "Q3",
        &[
            ("home/ada/Documents/plan.txt", &["Q3"]),
            ("home/ada/Documents/report.txt", &["Q3"]),
        ],
        &changes,
        before,
    );
    // A copy is made under a hidden name, tagged and then put in place, all before the index
    // reads the events: it is read again where it ends.
    let file = |path: &Path| uri::of(path, false);
    let folder = |path: &Path| uri::of(path, true);
    let before = changes.load(Ordering::SeqCst);
    access::copy(
        &scope,
        &Relocation {
            directory: false,
            ..relocation(&file(&report), &folder(&downloads))
        },
    )
    .unwrap();
    access::relocate(
        &scope,
        &Relocation {
            directory: false,
            ..relocation(&file(&plan), &folder(&projects))
        },
    )
    .unwrap();
    access::rename(&scope, &file(&report), false, "final.txt").unwrap();
    until(
        &index,
        &root,
        "Q3",
        &[
            ("home/ada/Documents/Projects/plan.txt", &["Q3"]),
            ("home/ada/Documents/final.txt", &["Q3"]),
            ("home/ada/Downloads/report.txt", &["Q3"]),
        ],
        &changes,
        before,
    );
    // A folder tagged inside, renamed at once: what it holds read again where it went.
    let before = changes.load(Ordering::SeqCst);
    let inside = place.file("home/ada/Documents/Projects/new.txt", "new");
    set(&inside, XDG, b"Q3");
    std::fs::rename(&projects, documents.join("Archive")).unwrap();
    until(
        &index,
        &root,
        "Q3",
        &[
            ("home/ada/Documents/Archive/new.txt", &["Q3"]),
            ("home/ada/Documents/Archive/plan.txt", &["Q3"]),
            ("home/ada/Documents/final.txt", &["Q3"]),
            ("home/ada/Downloads/report.txt", &["Q3"]),
        ],
        &changes,
        before,
    );
}
