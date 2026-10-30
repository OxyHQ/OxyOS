//! Tests of `expo-file-system`'s operations, the application's folders, URIs, typed watches and
//! the service's protocol, on directories the tests make (`tests::Place`).

use crate::access::{self, Encoding, Handles, Mode, Relocation};
use crate::service::{Reply, Service};
use crate::tests::{scope_in, text, Place};
use crate::watch::{Change, Event, Kind, Watcher};
use crate::{mime, uri, Access, Error, Root, Scope};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const ID: &str = "so.oxy.probe";

/// A home with a volume, for the application `so.oxy.probe` run from a bundle outside it.
fn application(place: &Place) -> Scope {
    let bundle = place.dir("bundle");
    std::fs::write(bundle.join("index.js"), "bundle").expect("bundle");
    let home = place.path("home/ada");
    scope_in(place)
        .for_application(
            ID,
            &home.join(".local/share/oxyos/apps").join(ID),
            &home.join(".cache/oxyos/apps").join(ID),
        )
        .expect("application")
        .with_bundle(&bundle)
        .expect("bundle")
}

fn u(path: &Path) -> String {
    uri::of(path, false)
}

fn d(path: &Path) -> String {
    uri::of(path, true)
}

#[test]
fn uris_name_local_paths_percent_encoded() {
    assert_eq!(
        uri::path_of("file:///home/ada/My%20Notes.md").expect("uri"),
        "/home/ada/My Notes.md"
    );
    assert_eq!(uri::path_of("file://localhost/a/b/").expect("uri"), "/a/b");
    assert_eq!(uri::path_of("file:/a").expect("uri"), "/a");
    assert_eq!(uri::path_of("file:///").expect("uri"), "/");
    assert_eq!(uri::path_of("file:///100%").expect("uri"), "/100%");
    assert_eq!(uri::path_of("file:///A%C3%B1o").expect("uri"), "/Año");
    for refused in [
        "/home/ada/x",
        "content://media/1",
        "asset:///x",
        "https://oxy.so/x",
        "file://server/share/x",
        "file:///a?b",
        "file:///a#b",
        "file:///%FF",
    ] {
        assert!(
            matches!(uri::path_of(refused), Err(Error::Refused(_))),
            "{refused}"
        );
    }
    let path = Path::new("/home/ada/Año nuevo/50% #1?.txt");
    let written = u(path);
    assert_eq!(
        written,
        "file:///home/ada/A%C3%B1o%20nuevo/50%25%20%231%3F.txt"
    );
    assert_eq!(uri::path_of(&written).expect("back"), text(path));
    assert_eq!(d(Path::new("/home/ada")), "file:///home/ada/");
    assert_eq!(
        uri::child("file:///home/ada/", "a b", false),
        "file:///home/ada/a%20b"
    );
    assert_eq!(
        uri::child("file:///home/ada", "Sub", true),
        "file:///home/ada/Sub/"
    );
    assert_eq!(uri::parent("file:///home/ada/a.txt"), "file:///home/ada/");
    assert_eq!(uri::parent("file:///home/ada/Sub/"), "file:///home/ada/");
}

#[test]
fn types_come_from_the_extension() {
    assert!(mime::sorted());
    assert_eq!(mime::of_name("Photo.JPG"), Some("image/jpeg"));
    assert_eq!(mime::of_name("notes.md"), Some("text/markdown"));
    assert_eq!(mime::of_name(".bashrc"), None);
    assert_eq!(mime::of_name("README"), None);
    assert_eq!(mime::of_name("x.unknown"), None);
    assert_eq!(mime::extension_of("image/jpeg"), Some("jpg"));
    assert_eq!(mime::extension_of("text/html; charset=utf-8"), Some("html"));
    assert_eq!(mime::extension_of("application/pdf"), Some("pdf"));
    assert_eq!(mime::extension_of("application/x-unknown"), None);
}

#[test]
fn an_application_has_folders_of_its_own_and_reads_its_bundle() {
    let place = Place::new("app");
    let scope = application(&place);
    let home = scope.home().to_path_buf();
    let app = scope.application().expect("application").clone();
    assert_eq!(app.documents, home.join(".local/share/oxyos/apps").join(ID));
    assert_eq!(app.cache, home.join(".cache/oxyos/apps").join(ID));
    for folder in [&app.documents, &app.cache] {
        let mode = std::fs::metadata(folder)
            .expect("made")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700, "{}", folder.display());
    }
    assert_eq!(
        access::paths(&scope),
        json!({
            "documentDirectory": d(&app.documents),
            "cacheDirectory": d(&app.cache),
            "bundleDirectory": d(&place.path("bundle")),
        })
    );
    assert_eq!(
        access::paths(&scope_in(&place)),
        json!({ "documentDirectory": null, "cacheDirectory": null, "bundleDirectory": null })
    );
    let (documents, cache) = (home.join("d"), home.join("c"));
    assert!(scope_in(&place)
        .for_application("../escape", &documents, &cache)
        .is_err());
    assert!(scope_in(&place)
        .for_application("", &documents, &cache)
        .is_err());
    // Without an application, its bundle is still read, and still never written.
    let bundled = scope_in(&place)
        .with_bundle(&place.path("bundle"))
        .expect("bundle");
    assert_eq!(
        access::paths(&bundled),
        json!({ "documentDirectory": null, "cacheDirectory": null,
                "bundleDirectory": d(&place.path("bundle")) })
    );
    assert!(matches!(
        access::delete(&bundled, &u(&place.path("bundle/index.js"))),
        Err(Error::Refused(_))
    ));

    let volumes = scope.volumes();
    assert_eq!(
        scope.root_of(&app.documents, &volumes),
        Some(Root::Documents)
    );
    assert_eq!(
        scope.root_of(&app.cache.join("x"), &volumes),
        Some(Root::Cache)
    );
    assert_eq!(
        scope.root_of(&place.path("bundle/index.js"), &volumes),
        Some(Root::Bundle)
    );
    // The bundle is read, never written; the folders themselves never change.
    let bundled = u(&place.path("bundle/index.js"));
    assert_eq!(
        access::read(&scope, &bundled, Encoding::Text).expect("read"),
        b"bundle"
    );
    for refused in [
        access::write(&scope, &bundled, b"x", Encoding::Text, false),
        access::delete(&scope, &bundled),
        access::create_file(&scope, &u(&place.path("bundle/new")), false, false),
        access::delete(&scope, &d(&app.documents)),
        access::delete(&scope, &d(&app.cache)),
        access::delete(&scope, &d(&home)),
        access::delete(&scope, &d(&place.path("media/ada/USB Stick"))),
        access::rename(&scope, &d(&app.documents), true, "Other").map(|_| ()),
        // Nor a folder holding them, or a Trash.
        access::delete(&scope, &d(&home.join(".local"))),
        access::delete(&scope, &d(&home.join(".cache"))),
        scope.item(text(&home.join(".local/share"))).map(|_| ()),
    ] {
        assert!(matches!(refused, Err(Error::Refused(_))), "{refused:?}");
    }
    // Reaching what is not there yet: under a root, through the deepest folder that is.
    assert_eq!(
        scope
            .reach(text(&app.documents.join("a/b/c.txt")), Access::Write)
            .expect("reach"),
        app.documents.join("a/b/c.txt")
    );
    let outside = place.dir("outside");
    std::os::unix::fs::symlink(&outside, home.join("away")).expect("link");
    std::os::unix::fs::symlink(outside.join("nothing"), home.join("dangling")).expect("link");
    for refused in [
        text(&home.join("away/new.txt")).to_string(),
        text(&home.join("dangling")).to_string(),
        text(&home.join("dangling/below")).to_string(),
        "/etc/passwd".to_string(),
        text(&home.join(".local/share/Trash/files/x")).to_string(),
    ] {
        assert!(
            matches!(scope.reach(&refused, Access::Write), Err(Error::Refused(_))),
            "{refused}"
        );
    }
}

#[test]
fn files_are_read_written_and_described() {
    let place = Place::new("files");
    let scope = application(&place);
    let documents = scope.application().expect("app").documents.clone();
    let notes = u(&documents.join("notes.txt"));

    assert_eq!(access::stat(&scope, &notes).expect("stat")["exists"], false);
    access::write(&scope, &notes, "Hola, ".as_bytes(), Encoding::Text, false).expect("write");
    access::write(&scope, &notes, "món".as_bytes(), Encoding::Text, true).expect("append");
    assert_eq!(
        access::read(&scope, &notes, Encoding::Text).expect("text"),
        "Hola, món".as_bytes()
    );
    assert_eq!(
        access::read(&scope, &notes, Encoding::Base64).expect("base64"),
        b"SG9sYSwgbcOzbg=="
    );
    let facts = access::stat(&scope, &notes).expect("stat");
    assert_eq!(
        (
            facts["exists"].clone(),
            facts["directory"].clone(),
            facts["size"].clone()
        ),
        (true.into(), false.into(), 10.into())
    );
    assert!(facts["modified"].as_i64().expect("modified") > 1_600_000_000_000);
    // Base64 written as Android's decoder takes it: padding optional, whitespace ignored.
    access::write(&scope, &notes, b"AAEC\n/w", Encoding::Base64, false).expect("base64");
    assert_eq!(
        access::read(&scope, &notes, Encoding::Bytes).expect("bytes"),
        [0, 1, 2, 255]
    );
    assert!(matches!(
        access::write(&scope, &notes, b"not*base64", Encoding::Base64, false),
        Err(Error::Invalid(_))
    ));
    // Text that is not UTF-8 reads with U+FFFD, as Java reads it.
    assert_eq!(
        access::read(&scope, &notes, Encoding::Text).expect("text"),
        "\u{0}\u{1}\u{2}\u{FFFD}".as_bytes()
    );
    // Its folder must be there; a folder is not a file.
    assert!(matches!(
        access::write(
            &scope,
            &u(&documents.join("missing/a.txt")),
            b"x",
            Encoding::Text,
            false
        ),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        access::read(&scope, &u(&documents), Encoding::Text),
        Err(Error::NotAFile)
    ));
    assert!(matches!(
        access::read(&scope, &u(&documents.join("none")), Encoding::Text),
        Err(Error::NotFound)
    ));

    // Digests, against FIPS 180's and RFC 1321's "abc".
    access::write(&scope, &notes, b"abc", Encoding::Text, false).expect("write");
    for (algorithm, digest) in [
        ("MD5", "900150983cd24fb0d6963f7d28e17f72"),
        ("SHA-1", "a9993e364706816aba3e25717850c26c9cd0d89d"),
        (
            "SHA-256",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "SHA-384",
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed\
             8086072ba1e7cc2358baeca134c825a7",
        ),
        (
            "SHA-512",
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
             2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        ),
    ] {
        assert_eq!(
            access::digest(&scope, &notes, algorithm).expect(algorithm),
            digest
        );
    }
    assert!(matches!(
        access::digest(&scope, &notes, "CRC32"),
        Err(Error::Invalid(_))
    ));

    // Made empty; never over a file unless asked, then emptied.
    let made = u(&documents.join("deep/er/made.txt"));
    assert!(matches!(
        access::create_file(&scope, &made, false, false),
        Err(Error::NotFound)
    ));
    access::create_file(&scope, &made, true, false).expect("intermediates");
    assert!(matches!(
        access::create_file(&scope, &notes, false, false),
        Err(Error::Exists)
    ));
    access::create_file(&scope, &notes, false, true).expect("overwrite");
    assert_eq!(
        std::fs::read(documents.join("notes.txt")).expect("empty"),
        b""
    );
    assert!(matches!(
        access::create_file(&scope, &u(&documents.join("deep")), false, true),
        Err(Error::NotAFile)
    ));
    let space = access::disk_space(&scope).expect("space");
    assert!(space["total"].as_u64() >= space["available"].as_u64());
}

#[test]
fn folders_are_made_listed_and_deleted_with_what_they_hold() {
    let place = Place::new("folders");
    let scope = application(&place);
    let home = scope.home().to_path_buf();
    let documents = scope.application().expect("app").documents.clone();
    let projects = d(&documents.join("Projects"));

    access::create_directory(&scope, &projects, false, false, false).expect("made");
    assert!(matches!(
        access::create_directory(&scope, &projects, false, false, false),
        Err(Error::Exists)
    ));
    access::create_directory(&scope, &projects, false, false, true).expect("idempotent");
    assert!(matches!(
        access::create_directory(&scope, &d(&documents.join("a/b")), false, false, false),
        Err(Error::NotFound)
    ));
    access::create_directory(&scope, &d(&documents.join("a/b")), true, false, false)
        .expect("intermediates");
    let child = access::create_child(&scope, &projects, "Plan B.md", false).expect("child");
    assert_eq!(child, format!("{projects}Plan%20B.md"));
    assert_eq!(
        access::create_child(&scope, &projects, "Plan B.md", false).expect("again"),
        child
    );
    let sub = access::create_child(&scope, &projects, "Sub", true).expect("folder");
    assert_eq!(sub, format!("{projects}Sub/"));
    for name in ["", ".", "..", "a/b", "a\\b"] {
        assert!(
            matches!(
                access::create_child(&scope, &projects, name, false),
                Err(Error::InvalidName)
            ),
            "{name:?}"
        );
    }
    assert!(matches!(
        access::create_child(&scope, &projects, "Sub", false),
        Err(Error::NotAFile)
    ));
    access::write(
        &scope,
        &format!("{sub}x.bin"),
        &[7; 5],
        Encoding::Bytes,
        false,
    )
    .expect("x");
    assert_eq!(access::size(&scope, &projects).expect("size"), 5);

    // Listed by name, each under the folder's URI as the application wrote it.
    std::os::unix::fs::symlink(&documents, home.join("docs")).expect("link");
    let through = d(&home.join("docs/Projects"));
    assert_eq!(
        access::list(&scope, &through).expect("list"),
        json!([
            { "uri": format!("{through}Plan%20B.md"), "isDirectory": false },
            { "uri": format!("{through}Sub/"), "isDirectory": true },
        ])
    );
    assert!(matches!(
        access::list(&scope, &d(&documents.join("none"))),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        access::list(&scope, &child),
        Err(Error::NotAFolder)
    ));

    // Made again empty when asked to overwrite it.
    access::create_directory(&scope, &sub, false, true, false).expect("overwrite");
    assert_eq!(
        std::fs::read_dir(documents.join("Projects/Sub"))
            .expect("sub")
            .count(),
        0
    );

    // Deleted for good, with everything in it; a link goes, never what it leads to.
    std::os::unix::fs::symlink(documents.join("Projects"), documents.join("link")).expect("link");
    access::delete(&scope, &u(&documents.join("link"))).expect("link");
    assert!(documents.join("Projects/Plan B.md").exists());
    access::delete(&scope, &projects).expect("delete");
    assert!(!documents.join("Projects").exists());
    assert!(matches!(
        access::delete(&scope, &projects),
        Err(Error::NotFound)
    ));
}

/// A copy's or a move's ends.
fn to<'a>(
    uri: &'a str,
    directory: bool,
    target: &'a str,
    to_directory: bool,
    overwrite: bool,
) -> Relocation<'a> {
    Relocation {
        uri,
        directory,
        to: target,
        to_directory,
        overwrite,
    }
}

#[test]
fn copies_and_moves_go_where_expo_says_and_never_over_an_item_unless_asked() {
    let place = Place::new("relocate");
    let scope = application(&place);
    let home = scope.home().to_path_buf();
    let a = place.file("home/ada/a.txt", "A");
    place.file("home/ada/b.txt", "B");
    let folder = place.dir("home/ada/Folder");
    place.file("home/ada/Folder/inner.txt", "inner");
    std::os::unix::fs::symlink("/etc/passwd", folder.join("passwd")).expect("link");

    // A file to a file, and into a folder under its name.
    let copied = access::copy(
        &scope,
        &to(&u(&a), false, &u(&home.join("c.txt")), false, false),
    )
    .expect("copy");
    assert_eq!(copied, u(&home.join("c.txt")));
    let into = access::copy(&scope, &to(&u(&a), false, &d(&folder), true, false)).expect("into");
    assert_eq!(into, format!("{}a.txt", d(&folder)));
    assert_eq!(
        std::fs::read_to_string(folder.join("a.txt")).expect("copy"),
        "A"
    );
    // Never over an item unless asked; then replaced.
    let (a_uri, b_uri) = (u(&a), u(&home.join("b.txt")));
    let over = to(&a_uri, false, &b_uri, false, false);
    assert!(matches!(access::copy(&scope, &over), Err(Error::Exists)));
    assert_eq!(
        std::fs::read_to_string(home.join("b.txt")).expect("kept"),
        "B"
    );
    access::copy(
        &scope,
        &Relocation {
            overwrite: true,
            ..over.clone()
        },
    )
    .expect("overwrite");
    assert_eq!(
        std::fs::read_to_string(home.join("b.txt")).expect("replaced"),
        "A"
    );
    // Where the destination's folder is not there; a folder never to a file.
    assert!(matches!(
        access::copy(
            &scope,
            &to(&u(&a), false, &u(&home.join("none/x.txt")), false, false)
        ),
        Err(Error::NoDestination)
    ));
    assert!(matches!(
        access::copy(
            &scope,
            &to(&u(&a), false, &d(&home.join("none")), true, false)
        ),
        Err(Error::NoDestination)
    ));
    assert!(matches!(
        access::copy(
            &scope,
            &to(&d(&folder), true, &u(&home.join("f.txt")), false, false)
        ),
        Err(Error::Invalid(_))
    ));
    // The object's kind and what is there agree.
    assert!(matches!(
        access::copy(
            &scope,
            &to(&u(&folder), false, &d(&home.join("x")), true, false)
        ),
        Err(Error::NotAFile)
    ));

    // A folder to one not there yet is it; to one that is, inside it. Links stay links.
    let copy = access::copy(
        &scope,
        &to(&d(&folder), true, &d(&home.join("Copy")), true, false),
    )
    .expect("folder");
    assert_eq!(copy, d(&home.join("Copy")));
    assert_eq!(
        std::fs::read_link(home.join("Copy/passwd")).expect("link"),
        PathBuf::from("/etc/passwd")
    );
    assert_eq!(
        std::fs::read_to_string(home.join("Copy/inner.txt")).expect("inner"),
        "inner"
    );
    let nested = access::copy(
        &scope,
        &to(&d(&folder), true, &d(&home.join("Copy")), true, false),
    )
    .expect("nested");
    assert_eq!(nested, format!("{}Folder/", d(&home.join("Copy"))));
    assert!(matches!(
        access::copy(&scope, &to(&d(&folder), true, &d(&folder), true, false)),
        Err(Error::Refused(_))
    ));
    // Nothing staged is left behind.
    let leftovers: Vec<_> = std::fs::read_dir(&home)
        .expect("home")
        .flatten()
        .filter(|item| item.file_name().to_string_lossy().contains(".oxyos-copy-"))
        .collect();
    assert!(leftovers.is_empty());

    // Moves: the item itself, never over another unless asked.
    let moved = access::relocate(
        &scope,
        &to(&u(&home.join("c.txt")), false, &d(&folder), true, false),
    )
    .expect("move");
    assert_eq!(moved, format!("{}c.txt", d(&folder)));
    assert!(!home.join("c.txt").exists());
    let c_uri = u(&folder.join("c.txt"));
    let onto = to(&c_uri, false, &b_uri, false, false);
    assert!(matches!(
        access::relocate(&scope, &onto),
        Err(Error::Exists)
    ));
    access::relocate(
        &scope,
        &Relocation {
            overwrite: true,
            ..onto
        },
    )
    .expect("overwrite");
    assert_eq!(
        std::fs::read_to_string(home.join("b.txt")).expect("moved"),
        "A"
    );
    // A folder replacing a folder, in one step.
    let replaced = access::relocate(
        &scope,
        &to(&d(&home.join("Copy")), true, &d(&folder), true, false),
    )
    .expect("inside");
    assert_eq!(replaced, format!("{}Copy/", d(&folder)));
    access::relocate(
        &scope,
        &to(
            &d(&folder.join("Copy")),
            true,
            &d(&home.join("Folder2")),
            true,
            false,
        ),
    )
    .expect("out");
    place.dir("home/ada/Target/old");
    access::relocate(
        &scope,
        &to(
            &d(&home.join("Folder2")),
            true,
            &d(&home.join("Target")),
            true,
            true,
        ),
    )
    .expect("into the folder, replacing nothing");
    assert!(home.join("Target/Folder2/inner.txt").exists());
    assert!(home.join("Target/old").exists());
    // A folder replacing one, asked to: exchanged in one step, the old one gone.
    place.file("home/ada/P/Sub/old.txt", "old");
    place.file("home/ada/Sub/new.txt", "new");
    let over_folder = (d(&home.join("Sub")), d(&home.join("P")));
    assert!(matches!(
        access::relocate(
            &scope,
            &to(&over_folder.0, true, &over_folder.1, true, false)
        ),
        Err(Error::Exists)
    ));
    access::relocate(
        &scope,
        &to(&over_folder.0, true, &over_folder.1, true, true),
    )
    .expect("replace");
    assert!(home.join("P/Sub/new.txt").exists() && !home.join("P/Sub/old.txt").exists());
    assert!(!home.join("Sub").exists());

    // Renamed in its folder, never over another.
    assert!(matches!(
        access::rename(&scope, &u(&a), false, "b.txt"),
        Err(Error::Exists)
    ));
    assert_eq!(
        access::rename(&scope, &u(&a), false, "Renamed.txt").expect("rename"),
        u(&home.join("Renamed.txt"))
    );
    assert_eq!(
        access::rename(&scope, &d(&home.join("Target")), true, "Done").expect("folder"),
        d(&home.join("Done"))
    );
    // The home and the bundle never move.
    assert!(matches!(
        access::relocate(&scope, &to(&d(&home), true, &d(&folder), true, false)),
        Err(Error::Refused(_))
    ));
}

#[test]
fn open_files_read_and_write_at_their_own_offset() {
    let place = Place::new("handles");
    let scope = application(&place);
    let documents = scope.application().expect("app").documents.clone();
    let data = u(&documents.join("data.bin"));
    let handles = Handles::default();
    assert!(matches!(
        handles.open(&scope, &data, Mode::Read),
        Err(Error::NotFound)
    ));
    let writer = handles.open(&scope, &data, Mode::ReadWrite).expect("made");
    handles.write(writer, b"0123456789").expect("write");
    assert_eq!(handles.offset(writer).expect("offset"), 10);
    handles.seek(writer, 2).expect("seek");
    assert_eq!(handles.read(writer, 3).expect("read"), b"234");
    assert_eq!(handles.offset(writer).expect("offset"), 5);
    assert_eq!(handles.read(writer, 100).expect("rest"), b"56789");
    assert_eq!(handles.read(writer, 100).expect("end"), b"");
    assert_eq!(handles.size(writer).expect("size"), 10);
    handles.close(writer);
    assert!(matches!(handles.read(writer, 1), Err(Error::Invalid(_))));

    let reader = handles.open(&scope, &data, Mode::Read).expect("read");
    assert!(matches!(
        handles.write(reader, b"x"),
        Err(Error::Invalid(_))
    ));
    let appender = handles.open(&scope, &data, Mode::Append).expect("append");
    assert!(matches!(handles.read(appender, 1), Err(Error::Invalid(_))));
    handles.write(appender, b"AB").expect("append");
    let from_start = handles.open(&scope, &data, Mode::Write).expect("write");
    handles.write(from_start, b"ab").expect("over");
    assert_eq!(handles.read(reader, 100).expect("all"), b"ab23456789AB");
    let emptied = handles
        .open(&scope, &data, Mode::Truncate)
        .expect("truncate");
    assert_eq!(handles.size(emptied).expect("size"), 0);
    // A handle's offset past the end writes there.
    handles.seek(emptied, 3).expect("seek");
    handles.write(emptied, b"z").expect("write");
    assert_eq!(
        std::fs::read(documents.join("data.bin")).expect("file"),
        b"\0\0\0z"
    );
    assert!(matches!(Mode::parse("x"), Err(Error::Invalid(_))));
    assert!(matches!(
        handles.open(&scope, &u(&place.path("bundle/index.js")), Mode::ReadWrite),
        Err(Error::Refused(_))
    ));
}

/// What a watcher says, until nothing more for 700 ms.
fn said(heard: &mpsc::Receiver<Change>) -> Vec<Change> {
    let mut all = Vec::new();
    while let Ok(change) = heard.recv_timeout(Duration::from_millis(700)) {
        all.push(change);
    }
    all
}

/// An event as a test compares it: its kind, path, whether a folder, and where it went.
type Seen = (Kind, PathBuf, bool, Option<PathBuf>);

fn events(change: &Change) -> (Vec<Seen>, bool) {
    let Change::Events { events, ended, .. } = change else {
        panic!("not a typed watch's change: {change:?}");
    };
    (
        events
            .iter()
            .map(|event: &Event| {
                (
                    event.kind,
                    event.path.clone(),
                    event.directory,
                    event.new_path.as_ref().map(|(path, _)| path.clone()),
                )
            })
            .collect(),
        *ended,
    )
}

#[test]
fn a_typed_watch_says_what_changed_after_its_debounce() {
    let place = Place::new("typed");
    let scope = application(&place);
    let home = scope.home().to_path_buf();
    let documents = place.dir("home/ada/Documents");
    std::os::unix::fs::symlink(&documents, home.join("docs")).expect("link");
    let named = home.join("docs");
    let (told, heard) = mpsc::channel();
    let watcher = Watcher::new(scope.mountinfo(), move |change| {
        let _ = told.send(change);
    })
    .expect("watcher");
    let watch = watcher
        .watch_path(&documents, &named, Duration::from_millis(250))
        .expect("watch");

    // Twenty writes and a new folder in one burst: one change, each item's events merged.
    let started = Instant::now();
    for n in 0..20 {
        std::fs::write(documents.join("a.txt"), format!("{n}")).expect("write");
    }
    std::fs::create_dir(documents.join("New")).expect("folder");
    let changes = said(&heard);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(started.elapsed() >= Duration::from_millis(250));
    assert_eq!(
        events(&changes[0]),
        (
            vec![
                (Kind::Created, named.join("a.txt"), false, None),
                (Kind::Modified, named.join("a.txt"), false, None),
                (Kind::Created, named.join("New"), true, None),
            ],
            false
        )
    );
    let Change::Events {
        watch: which,
        events: list,
        ..
    } = &changes[0]
    else {
        unreachable!()
    };
    assert_eq!(*which, watch);
    assert_eq!(list[0].flags & 0x100, 0x100, "IN_CREATE");

    // A rename in the folder is one `renamed` with both paths; a delete is a delete.
    std::fs::rename(documents.join("a.txt"), documents.join("b.txt")).expect("rename");
    std::fs::remove_dir(documents.join("New")).expect("remove");
    assert_eq!(
        said(&heard).iter().map(events).collect::<Vec<_>>(),
        vec![(
            vec![
                (
                    Kind::Renamed,
                    named.join("a.txt"),
                    false,
                    Some(named.join("b.txt"))
                ),
                (Kind::Deleted, named.join("New"), true, None),
            ],
            false
        )]
    );
    // Moved out to another folder: a `renamed` alone, the other end not watched.
    let music = place.dir("home/ada/Music");
    std::fs::rename(documents.join("b.txt"), music.join("b.txt")).expect("away");
    assert_eq!(
        said(&heard).iter().map(events).collect::<Vec<_>>(),
        vec![(
            vec![(Kind::Renamed, named.join("b.txt"), false, None)],
            false
        )]
    );

    // A file's own watch: its writes, then its deletion, said at once, ends it.
    let watched = music.join("b.txt");
    let file = watcher
        .watch_path(&watched, &watched, Duration::from_millis(50))
        .expect("file");
    std::fs::write(&watched, "more").expect("write");
    assert_eq!(
        said(&heard).iter().map(events).collect::<Vec<_>>(),
        vec![(vec![(Kind::Modified, watched.clone(), false, None)], false)]
    );
    std::fs::remove_file(&watched).expect("delete");
    let changes = said(&heard);
    assert_eq!(
        changes.iter().map(events).collect::<Vec<_>>(),
        vec![(vec![(Kind::Deleted, watched.clone(), false, None)], true)]
    );
    assert!(matches!(&changes[0], Change::Events { watch, .. } if *watch == file));

    // The folder deleted, with what was gathered: said at once as deleted, and the watch ends.
    std::fs::write(documents.join("last.txt"), "x").expect("write");
    std::fs::remove_file(documents.join("last.txt")).expect("remove");
    std::fs::remove_dir(&documents).expect("remove");
    let changes = said(&heard);
    assert_eq!(
        changes.iter().map(events).collect::<Vec<_>>(),
        vec![(vec![(Kind::Deleted, named.clone(), true, None)], true)]
    );
    assert_eq!(watcher.len(), 0);

    // An ended watch says nothing more; unwatching stops one at once.
    let again = place.dir("home/ada/Again");
    let quiet = watcher
        .watch_path(&again, &again, Duration::from_millis(50))
        .expect("watch");
    watcher.unwatch(quiet);
    std::fs::write(again.join("x"), "x").expect("write");
    assert!(said(&heard).is_empty());
}

/// A request answered through the queue, as the host takes it.
fn ask(service: &Service, id: u64, request: Value, bytes: Vec<u8>) -> (Value, Vec<u8>) {
    service.request(id, &request.to_string(), bytes);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(Reply {
            id: answered,
            json,
            bytes,
        }) = service.take()
        {
            if answered == id {
                return (serde_json::from_str(&json).expect("json"), bytes);
            }
        }
        assert!(Instant::now() < deadline, "no answer to {request}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn call(service: &Service, request: Value, bytes: &[u8]) -> (Value, Vec<u8>) {
    let reply = service.call(&request.to_string(), bytes);
    assert_eq!(reply.id, 0);
    (
        serde_json::from_str(&reply.json).expect("json"),
        reply.bytes,
    )
}

#[test]
fn the_service_answers_at_once_or_in_order_with_bytes_beside() {
    let place = Place::new("service");
    let scope = application(&place);
    let home = scope.home().to_path_buf();
    let documents = scope.application().expect("app").documents.clone();
    place.dir("home/ada/Documents");
    let service = Service::new(scope).expect("service");

    let (media, _) = call(
        &service,
        json!({"op": "mimeType", "uri": "file:///a/b%20c.PDF"}),
        &[],
    );
    assert_eq!(media["value"], "application/pdf");
    let (paths, _) = call(&service, json!({"op": "paths"}), &[]);
    assert_eq!(paths["value"]["documentDirectory"], d(&documents));
    let notes = u(&documents.join("notes.txt"));
    let (written, _) = call(
        &service,
        json!({"op": "write", "uri": notes, "encoding": "utf8"}),
        "¡Hola!".as_bytes(),
    );
    assert_eq!(written, json!({"ok": true, "value": null}));
    let (read, bytes) = ask(
        &service,
        7,
        json!({"op": "read", "uri": notes, "encoding": "utf8"}),
        Vec::new(),
    );
    assert_eq!(
        (read["ok"].clone(), bytes),
        (true.into(), "¡Hola!".as_bytes().to_vec())
    );
    let (refused, _) = call(
        &service,
        json!({"op": "read", "uri": "file:///etc/passwd"}),
        &[],
    );
    assert_eq!(
        (refused["ok"].clone(), refused["error"].clone()),
        (false.into(), "refused".into())
    );
    assert!(refused["message"]
        .as_str()
        .expect("message")
        .contains("outside"));
    let (unknown, _) = call(&service, json!({"op": "format"}), &[]);
    assert_eq!(unknown["error"], "invalid");
    let garbage = service.call("not json", &[]);
    assert!(garbage.json.contains("\"ok\":false"), "{}", garbage.json);

    // Handles through the service.
    let (handle, _) = call(
        &service,
        json!({"op": "open", "uri": notes, "mode": "r"}),
        &[],
    );
    let handle = handle["value"].as_u64().expect("handle");
    let (_, first) = call(
        &service,
        json!({"op": "handleRead", "handle": handle, "length": 2}),
        &[],
    );
    assert_eq!(first, "¡".as_bytes());
    let (offset, _) = call(
        &service,
        json!({"op": "handleOffset", "handle": handle}),
        &[],
    );
    assert_eq!(offset["value"], 2);

    // The places and the Trash, by URI.
    let (places, _) = ask(&service, 8, json!({"op": "places"}), Vec::new());
    assert_eq!(places["value"]["home"], d(&home));
    assert_eq!(places["value"]["favourites"][0]["key"], "documents");
    assert_eq!(
        places["value"]["favourites"][0]["uri"],
        d(&home.join("Documents"))
    );
    assert_eq!(places["value"]["volumes"][0]["name"], "USB Stick");
    let (trashed, _) = ask(
        &service,
        9,
        json!({"op": "trash", "uris": [notes, "file:///etc/passwd", d(&home)]}),
        Vec::new(),
    );
    let results = trashed["value"].as_array().expect("results");
    let in_trash = u(&home.join(".local/share/Trash/files/notes.txt"));
    assert_eq!(
        results[0],
        json!({"uri": notes, "ok": true, "result": in_trash})
    );
    assert_eq!(results[1]["error"], "refused");
    assert_eq!(results[2]["error"], "refused");
    let (listed, _) = ask(&service, 10, json!({"op": "trashed"}), Vec::new());
    let item = &listed["value"][0];
    assert_eq!(
        (
            item["uri"].clone(),
            item["name"].clone(),
            item["originalUri"].clone()
        ),
        (
            in_trash.clone().into(),
            "notes.txt".into(),
            notes.clone().into()
        )
    );
    assert_eq!(
        (item["isDirectory"].clone(), item["size"].clone()),
        (false.into(), 7.into())
    );
    assert!(item["deletedAt"].as_i64().is_some());
    let (watch, _) = ask(&service, 11, json!({"op": "watchTrash"}), Vec::new());
    let watch = watch["value"].as_u64().expect("watch");
    // At once, so no reply is taken before the event is looked for.
    let (restored, _) = call(&service, json!({"op": "restore", "uris": [in_trash]}), &[]);
    assert_eq!(restored["value"][0]["result"], notes);
    // The Trash's watch says it changed, as an event (id 0) in the same queue.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(reply) = service.take() {
            let event: Value = serde_json::from_str(&reply.json).expect("json");
            if reply.id == 0 && event["watch"].as_u64() == Some(watch) {
                break;
            }
        }
        assert!(Instant::now() < deadline, "no change said");
        std::thread::sleep(Duration::from_millis(5));
    }

    // A typed watch's events, by URI under the watched one's spelling.
    let (typed, _) = call(
        &service,
        json!({"op": "watchPath", "uri": d(&documents), "debounce": 20}),
        &[],
    );
    let typed = typed["value"].as_u64().expect("watch");
    call(
        &service,
        json!({"op": "createFile", "uri": u(&documents.join("new file.txt"))}),
        &[],
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let event = loop {
        if let Some(reply) = service.take() {
            let event: Value = serde_json::from_str(&reply.json).expect("json");
            if reply.id == 0 && event["watch"].as_u64() == Some(typed) {
                break event;
            }
        }
        assert!(Instant::now() < deadline, "no event said");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        event,
        json!({
            "watch": typed,
            "ended": false,
            "events": [{
                "type": "created",
                "uri": u(&documents.join("new file.txt")),
                "isDirectory": false,
                "flags": 0x100,
            }],
        })
    );
    let (missing, _) = call(
        &service,
        json!({"op": "watchPath", "uri": u(&documents.join("none"))}),
        &[],
    );
    assert_eq!(missing["error"], "not-found");
}

/// Each request a test server was sent: its line, its headers (names lowercase) and its body.
type Sent = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<(String, String)>, Vec<u8>)>>>;

/// A server for the transfers' tests, on a port of its own; returns its base URL and what it was
/// sent, request by request.
fn transfer_server() -> (String, Sent) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listen");
    let address = listener.local_addr().expect("address");
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let kept = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let kept = kept.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let mut headers = Vec::new();
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).is_err() {
                        return;
                    }
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    let (name, value) = header.split_once(':').expect("header");
                    headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
                }
                let length = headers
                    .iter()
                    .find(|(name, _)| name == "content-length")
                    .map_or(0, |(_, value)| value.parse().expect("length"));
                let mut body = vec![0u8; length];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                let range = headers
                    .iter()
                    .find(|(name, _)| name == "range")
                    .map(|(_, value)| value.clone());
                kept.lock()
                    .expect("lock")
                    .push((line.trim().to_string(), headers, body));
                let target = line.split_whitespace().nth(1).unwrap_or("");
                let path = target.split('?').next().unwrap_or("").to_string();
                let mut stream = stream;
                let reply = |stream: &mut std::net::TcpStream, head: &str, body: &[u8]| {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(body);
                };
                match path.as_str() {
                    "/report" => reply(
                        &mut stream,
                        "200 OK\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=\"Q3 report.txt\"",
                        b"quarterly",
                    ),
                    "/pictures/photo%201" => {
                        reply(&mut stream, "200 OK\r\nContent-Type: image/png", b"png")
                    }
                    "/missing" => reply(&mut stream, "404 Not Found", b"no"),
                    // Twenty pieces of a kilobyte, 50 ms apart; a range from where it asks.
                    "/slow" => {
                        let from: usize = range
                            .as_deref()
                            .and_then(|range| range.strip_prefix("bytes="))
                            .and_then(|range| range.trim_end_matches('-').parse().ok())
                            .unwrap_or(0);
                        let all: Vec<u8> = (0..20 * 1024).map(|n| (n / 1024) as u8 + b'a').collect();
                        let status = if from > 0 { "206 Partial Content" } else { "200 OK" };
                        let _ = write!(
                            stream,
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            all.len() - from
                        );
                        for piece in all[from..].chunks(1024) {
                            if stream.write_all(piece).is_err() {
                                return;
                            }
                            let _ = stream.flush();
                            std::thread::sleep(Duration::from_millis(50));
                        }
                    }
                    _ => reply(&mut stream, "201 Created\r\nX-Oxy: yes", b"stored"),
                }
            });
        }
    });
    (format!("http://{address}"), seen)
}

#[test]
fn files_are_downloaded_into_the_scope_and_uploaded_from_it() {
    let place = Place::new("transfers");
    let scope = application(&place);
    let documents = scope.application().expect("app").documents.clone();
    let (base, seen) = transfer_server();
    let network = oxyos_net::Network::new().expect("network");
    let told = std::sync::Mutex::new(Vec::new());
    let progress = |value: Value| told.lock().expect("lock").push(value);
    let download = |request: Value| {
        crate::transfer::download(&network, &scope, &Default::default(), &request, &progress)
    };

    // Into a folder, named by the response; again, refused unless it may replace it.
    let asked =
        json!({ "url": format!("{base}/report"), "to": d(&documents), "toDirectory": true });
    let got = download(asked.clone()).expect("download");
    assert_eq!(got, format!("{}Q3%20report.txt", d(&documents)));
    assert_eq!(
        std::fs::read(documents.join("Q3 report.txt")).expect("file"),
        b"quarterly"
    );
    assert_eq!(
        told.lock().expect("lock").last(),
        Some(&json!({ "bytesWritten": 9, "totalBytes": 9 }))
    );
    assert!(matches!(download(asked.clone()), Err(Error::Exists)));
    let mut idempotent = asked;
    idempotent["idempotent"] = true.into();
    download(idempotent).expect("replaced");
    // Named by the URL, its type's extension added; to a file, as it.
    let got = download(json!({
        "url": format!("{base}/pictures/photo%201?size=2"), "to": d(&documents), "toDirectory": true,
    }))
    .expect("by url");
    assert_eq!(got, format!("{}photo%201.png", d(&documents)));
    let to_file = u(&documents.join("named.bin"));
    assert_eq!(
        download(json!({ "url": format!("{base}/report"), "to": to_file })).expect("file"),
        to_file
    );
    // A failure's status, and no file; nowhere the scope refuses.
    let failed =
        download(json!({ "url": format!("{base}/missing"), "to": u(&documents.join("x")) }));
    assert!(
        matches!(&failed, Err(Error::Network(reason)) if reason.contains("404")),
        "{failed:?}"
    );
    assert!(!documents.join("x").exists());
    assert!(matches!(
        download(json!({ "url": format!("{base}/report"), "to": "file:///etc/report" })),
        Err(Error::Refused(_))
    ));
    assert!(matches!(
        download(json!({ "url": format!("{base}/report"), "to": u(&documents.join("no/x")) })),
        Err(Error::NoDestination)
    ));
    assert_eq!(
        crate::transfer::file_name("https://a/", None, None),
        "downloadfile.bin"
    );
    assert_eq!(
        crate::transfer::file_name(
            "https://a/b",
            Some("attachment; filename=\"plain.txt\"; filename*=UTF-8''A%C3%B1o.txt"),
            None
        ),
        "Año.txt"
    );
    assert_eq!(
        crate::transfer::file_name(
            "https://a/x",
            Some("attachment; filename=\"../../etc/passwd\""),
            None
        ),
        "passwd.bin"
    );

    // Paused part way, it keeps what it wrote; resumed, it asks for the rest.
    let live = std::sync::Arc::new(crate::transfer::Live::default());
    let slow = u(&documents.join("slow.bin"));
    let resume_data = std::thread::scope(|threads| {
        let pausing = live.clone();
        let paused = threads.spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            pausing.pause()
        });
        let ended = crate::transfer::download(
            &network,
            &scope,
            &live,
            &json!({ "url": format!("{base}/slow"), "to": slow }),
            &|_| {},
        );
        assert_eq!(ended.expect("paused"), Value::Null);
        paused.join().expect("pause").expect("resume data")
    });
    assert!(resume_data > 0 && resume_data < 20 * 1024, "{resume_data}");
    let resumed = crate::transfer::download(
        &network,
        &scope,
        &Default::default(),
        &json!({ "url": format!("{base}/slow"), "to": slow, "resume": resume_data }),
        &progress,
    )
    .expect("resumed");
    assert_eq!(resumed, slow);
    let all: Vec<u8> = (0..20 * 1024).map(|n| (n / 1024) as u8 + b'a').collect();
    assert_eq!(
        std::fs::read(documents.join("slow.bin")).expect("whole"),
        all
    );
    assert!(seen.lock().expect("lock").iter().any(
        |(_, headers, _)| headers.contains(&("range".into(), format!("bytes={resume_data}-")))
    ));
    // Cancelled, it fails as cancelled.
    let live = crate::transfer::Live::default();
    live.cancel();
    assert!(matches!(
        crate::transfer::download(
            &network,
            &scope,
            &live,
            &json!({ "url": format!("{base}/slow"), "to": u(&documents.join("c.bin")) }),
            &|_| {},
        ),
        Err(Error::Cancelled)
    ));

    // Uploads: the file as the body, its length known; a form with its parameters.
    told.lock().expect("lock").clear();
    let notes = documents.join("notes.txt");
    std::fs::write(&notes, "hello there").expect("notes");
    let upload = |request: Value| {
        crate::transfer::upload(&network, &scope, &Default::default(), &request, &progress)
    };
    let answered = upload(json!({
        "uri": u(&notes), "url": format!("{base}/put"), "method": "PUT",
        "headers": { "X-Part": "1" },
    }))
    .expect("upload");
    assert_eq!(answered["status"], 201);
    assert_eq!(answered["body"], "stored");
    assert_eq!(answered["headers"]["x-oxy"], "yes");
    {
        let seen = seen.lock().expect("lock");
        let (line, headers, body) = seen.last().expect("request");
        assert_eq!(line, "PUT /put HTTP/1.1");
        assert_eq!(body, b"hello there");
        assert!(headers.contains(&("content-length".into(), "11".into())));
        assert!(headers.contains(&("x-part".into(), "1".into())));
        assert!(!headers.iter().any(|(name, _)| name == "content-type"));
    }
    assert_eq!(
        told.lock().expect("lock").last(),
        Some(&json!({ "bytesSent": 11, "totalBytes": 11 }))
    );
    upload(json!({
        "uri": u(&notes), "url": format!("{base}/form"), "uploadType": 1,
        "fieldName": "document", "parameters": { "owner": "ada" },
    }))
    .expect("multipart");
    {
        let seen = seen.lock().expect("lock");
        let (_, headers, body) = seen.last().expect("request");
        let content_type = &headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .expect("type")
            .1;
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .expect("multipart");
        assert_eq!(
            String::from_utf8_lossy(body),
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"owner\"\r\nContent-Length: 3\r\n\r\nada\r\n\
                 --{boundary}\r\nContent-Disposition: form-data; name=\"document\"; filename=\"notes.txt\"\r\n\
                 Content-Type: text/plain\r\nContent-Length: 11\r\n\r\nhello there\r\n--{boundary}--\r\n"
            )
        );
    }
    assert!(matches!(
        upload(json!({ "uri": "file:///etc/passwd", "url": format!("{base}/x") })),
        Err(Error::Refused(_))
    ));
    assert!(matches!(
        upload(json!({ "uri": u(&documents.join("none")), "url": format!("{base}/x") })),
        Err(Error::NotFound)
    ));
}

#[test]
fn transfers_are_requested_answered_when_they_end_and_cancelled_by_id() {
    let place = Place::new("service-transfers");
    let scope = application(&place);
    let documents = scope.application().expect("app").documents.clone();
    let (base, _) = transfer_server();
    let service = Service::new(scope).expect("service");
    let (refused, _) = call(&service, json!({"op": "download", "url": base}), &[]);
    assert_eq!(refused["error"], "invalid");
    let request = json!({ "op": "download", "url": format!("{base}/slow"), "to": u(&documents.join("s.bin")) });
    service.request(5, &request.to_string(), Vec::new());
    // Its progress comes as events while the queue answers other requests.
    let (places, _) = ask(&service, 6, json!({"op": "paths"}), Vec::new());
    assert_eq!(places["ok"], true);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut progressed = false;
    let answer = loop {
        if let Some(reply) = service.take() {
            let json: Value = serde_json::from_str(&reply.json).expect("json");
            if reply.id == 0 && json["transfer"] == 5 {
                assert!(json["progress"]["bytesWritten"].as_u64().is_some());
                assert_eq!(json["progress"]["totalBytes"], 20 * 1024);
                if !progressed {
                    progressed = true;
                    let (cancelled, _) = call(
                        &service,
                        json!({"op": "cancelTransfer", "transfer": 5}),
                        &[],
                    );
                    assert_eq!(cancelled["ok"], true);
                }
            } else if reply.id == 5 {
                break json;
            }
        }
        assert!(Instant::now() < deadline, "no end");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(progressed);
    assert_eq!(
        (answer["ok"].clone(), answer["error"].clone()),
        (false.into(), "cancelled".into())
    );
    let (paused, _) = call(&service, json!({"op": "pauseTransfer", "transfer": 5}), &[]);
    assert_eq!(paused["error"], "invalid");
}

#[test]
fn a_move_between_volumes_copies_and_then_deletes() {
    use std::os::unix::fs::MetadataExt;
    let place = Place::new("cross-device");
    // A volume on another file system: /dev/shm, where the test's home (in the temporary
    // directory) is not.
    let other =
        PathBuf::from("/dev/shm").join(format!("oxyos-files-volume-{}", std::process::id()));
    let device = |path: &Path| std::fs::metadata(path).map(|found| found.dev());
    if device(Path::new("/dev/shm")).ok() == device(&place.root).ok() {
        eprintln!("skipped: /dev/shm is on the temporary directory's file system");
        return;
    }
    let media = other.join("media/ada");
    std::fs::create_dir_all(media.join("Stick")).expect("volume");
    let mountinfo = place.file(
        "mountinfo",
        &format!(
            "90 22 8:17 / {}/Stick rw - vfat /dev/sdb1 rw\n",
            media.display()
        ),
    );
    let home = place.dir("home/ada");
    let scope = Scope::new(
        &home,
        &home.join(".local/share"),
        &home.join(".config"),
        vec![media.clone()],
        mountinfo,
        1000,
    )
    .expect("scope");
    let stick = std::fs::canonicalize(media.join("Stick")).expect("stick");
    place.file("home/ada/notes.txt", "notes");
    place.file("home/ada/Folder/inner/deep.txt", "deep");
    std::os::unix::fs::symlink("deep.txt", home.join("Folder/inner/link")).expect("link");

    let (notes, folder, stick_uri) = (
        u(&home.join("notes.txt")),
        d(&home.join("Folder")),
        d(&stick),
    );
    let moved =
        access::relocate(&scope, &to(&notes, false, &stick_uri, true, false)).expect("file");
    assert_eq!(moved, format!("{stick_uri}notes.txt"));
    assert_eq!(
        std::fs::read_to_string(stick.join("notes.txt")).expect("there"),
        "notes"
    );
    assert!(!home.join("notes.txt").exists());
    access::relocate(&scope, &to(&folder, true, &stick_uri, true, false)).expect("folder");
    assert_eq!(
        std::fs::read_to_string(stick.join("Folder/inner/deep.txt")).expect("deep"),
        "deep"
    );
    assert_eq!(
        std::fs::read_link(stick.join("Folder/inner/link")).expect("link"),
        PathBuf::from("deep.txt")
    );
    assert!(!home.join("Folder").exists());
    // Never over an item there unless asked, across volumes too.
    place.file("home/ada/notes.txt", "again");
    assert!(matches!(
        access::relocate(&scope, &to(&notes, false, &stick_uri, true, false)),
        Err(Error::Exists)
    ));
    assert_eq!(
        std::fs::read_to_string(home.join("notes.txt")).expect("kept"),
        "again"
    );
    access::relocate(&scope, &to(&notes, false, &stick_uri, true, true)).expect("overwrite");
    assert_eq!(
        std::fs::read_to_string(stick.join("notes.txt")).expect("replaced"),
        "again"
    );
    let _ = std::fs::remove_dir_all(&other);
}
