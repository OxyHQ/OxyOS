//! Settings › Storage's figures, on homes the tests make and a played disk.

use super::*;
use oxyos_compositor_core::Grants;
use oxyos_protocol::{Event, PreeditText, Request};
use std::time::Instant;

const SETTINGS: ClientId = 4;
const ATLAS: ClientId = 5;

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "oxyos-compositor-storage-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::canonicalize(&directory).unwrap()
}

fn scope_of(home: &Path) -> Scope {
    let mountinfo = home.join(".mountinfo");
    std::fs::write(&mountinfo, "").unwrap();
    Scope::new(
        home,
        &home.join(".local/share"),
        &home.join(".config"),
        Vec::new(),
        mountinfo,
        rustix::process::getuid().as_raw(),
    )
    .unwrap()
}

fn listed(id: &str, size: u64, last_used: Option<i64>) -> oxyos_install::install::Listed {
    oxyos_install::install::Listed {
        id: id.into(),
        version: "1.0.0".into(),
        version_code: 1,
        entry: format!("/var/lib/oxyos/apps/users/ana/applications/{id}.desktop"),
        capabilities: BTreeMap::new(),
        staged: None,
        size,
        last_used,
    }
}

/// Each id's own data under `home`, as a session's environment would give it.
fn own_under(home: &Path) -> OwnDataOf {
    let home = home.to_path_buf();
    Arc::new(move |id: &str| {
        Some(OwnData {
            data: home.join(".local/share/oxyos/apps").join(id),
            caches: home.join(".cache/oxyos/apps").join(id),
        })
    })
}

fn file(path: &Path, bytes: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![1u8; bytes]).unwrap();
}

fn du(path: &Path) -> u64 {
    usage::allocated(path, &|| false).unwrap()
}

#[test]
fn dpkgs_installed_packages_are_what_it_says_they_occupy() {
    let status = "Package: base-files\nStatus: install ok installed\nInstalled-Size: 341\n\n\
                  Package: gone\nStatus: deinstall ok config-files\nInstalled-Size: 9999\n\n\
                  Package: half\nStatus: install reinstreq half-installed\nInstalled-Size: 7\n\n\
                  Package: libc6\nStatus: install ok installed\nInstalled-Size: 12988\n";
    assert_eq!(installed_size(status), (341 + 12988) * 1024);
    assert_eq!(installed_size(""), 0);
}

#[test]
fn the_system_is_counted_on_its_volume_and_nowhere_else() {
    let root = scratch("system");
    file(&root.join("usr/bin/true"), 10);
    std::fs::create_dir_all(root.join("var/lib/dpkg")).unwrap();
    std::fs::write(
        root.join("var/lib/dpkg/status"),
        "Package: a\nStatus: install ok installed\nInstalled-Size: 100\n",
    )
    .unwrap();
    file(&root.join("boot/initrd.img-6.12.0-1-amd64"), 64 * 1024);
    file(&root.join("boot/config-6.12.0-1-amd64"), 64 * 1024);
    let device = std::fs::metadata(&root).unwrap().dev();
    let initrd = du(&root.join("boot/initrd.img-6.12.0-1-amd64"));
    assert_eq!(system_on(device, &root), 100 * 1024 + initrd);
    // A system on a volume of its own (a published install's `/usr`) is not this volume's.
    assert_eq!(system_on(device + 1, &root), 0);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_volume_name_is_its_labels_and_its_reserve_is_the_systems() {
    assert_eq!(unescape_label("OxyOS\\x20Disk"), "OxyOS Disk");
    assert_eq!(unescape_label("plain"), "plain");
    assert_eq!(unescape_label("odd\\xZZ"), "odd\\xZZ");
    let volume = Volume {
        capacity: 500,
        free: 200,
        available: 175,
    };
    assert_eq!((volume.used(), volume.reserved()), (300, 25));
    // Used, reserved and available are the whole volume.
    assert_eq!(
        volume.used() + volume.reserved() + volume.available,
        volume.capacity
    );
    let played = PlayedDisk {
        name: Some("Macintosh HD".into()),
        capacity: 500,
        available: 175,
        reserved: 25,
        system: 40,
    };
    assert_eq!(played.volume(Path::new("/")).unwrap(), volume);
}

#[test]
fn each_application_is_its_package_and_its_own_data_the_largest_first() {
    let home = scratch("applications");
    let own = own_under(&home);
    file(
        &home.join(".local/share/oxyos/apps/so.oxy.allo/db/messages.db"),
        300_000,
    );
    file(
        &home.join(".cache/oxyos/apps/so.oxy.allo/images/1.jpg"),
        50_000,
    );
    file(
        &home.join(".local/share/oxyos/apps/so.oxy.mention/state.json"),
        10_000,
    );
    file(
        &home.join(".local/share/oxyos/apps/so.oxy.atlas/cache.db"),
        1_000,
    );
    let installed = BTreeMap::from([
        (
            "so.oxy.mention".to_string(),
            listed("so.oxy.mention", 2_000_000, Some(1_700_000_000)),
        ),
        // A system application the store updates: its package is the account's, and it is never
        // removed.
        (
            "so.oxy.atlas".to_string(),
            listed("so.oxy.atlas", 900_000, Some(1_790_000_000)),
        ),
    ]);
    let registered = vec![
        (
            "so.oxy.allo".to_string(),
            "Allo".to_string(),
            Origin::Development,
        ),
        (
            "so.oxy.mention".to_string(),
            "Mention".to_string(),
            Origin::Installed { version_code: 1 },
        ),
        (
            "so.oxy.atlas".to_string(),
            "Atlas".to_string(),
            Origin::System,
        ),
        // No data at all: listed, with nothing.
        (
            "so.oxy.settings".to_string(),
            "Settings".to_string(),
            Origin::System,
        ),
    ];
    let apps = applications(registered, &installed, &*own, &|| false).unwrap();
    let names: Vec<&str> = apps.iter().map(|app| app.name.as_str()).collect();
    assert_eq!(names, ["Mention", "Atlas", "Allo", "Settings"]);
    let allo = &apps[2];
    assert_eq!(
        (
            allo.package,
            allo.data,
            allo.caches,
            allo.removable,
            allo.origin
        ),
        (
            None,
            du(&home.join(".local/share/oxyos/apps/so.oxy.allo")),
            du(&home.join(".cache/oxyos/apps/so.oxy.allo")),
            false,
            "development"
        )
    );
    let mention = &apps[0];
    assert_eq!(mention.package, Some(2_000_000));
    assert_eq!(mention.last_used, Some(1_700_000_000_000));
    assert!(mention.removable);
    let atlas = &apps[1];
    assert_eq!((atlas.package, atlas.removable), (Some(900_000), false));
    assert_eq!((apps[3].data, apps[3].caches), (0, 0));
    // A measurement asked to stop stops.
    let registered = vec![(
        "so.oxy.allo".to_string(),
        "Allo".to_string(),
        Origin::Development,
    )];
    let error = applications(registered, &installed, &*own, &|| true).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    let _ = std::fs::remove_dir_all(&home);
}

/// What `to` was told, each answer's chunks put together.
fn told(scene: &mut Scene, to: ClientId) -> Vec<String> {
    let mut answers = Vec::new();
    let mut answer = String::new();
    for (client, event) in scene.drain_events() {
        if let (true, Event::SystemText { text, last, .. }) = (client == to, event) {
            answer.push_str(text.as_str());
            if last {
                answers.push(std::mem::take(&mut answer));
            }
        }
    }
    answers
}

/// A request as the client sent it, in chunks, citing no input: the scene hands it to the
/// program.
fn ask(scene: &mut Scene, client: ClientId, serial: u32, text: &str) {
    let chunks = oxyos_protocol::TextChunk::split(text);
    let count = chunks.len();
    for (index, chunk) in chunks.into_iter().enumerate() {
        scene
            .handle(
                client,
                Request::SystemRequest {
                    serial,
                    input: 0,
                    index: index as u16,
                    last: index + 1 == count,
                    text: PreeditText::new(chunk.as_str()).unwrap(),
                },
            )
            .unwrap();
    }
}

/// A request as the scene hands it over, citing an input of the client's.
fn pressed(client: ClientId, serial: u32, text: &str) -> SystemRequest {
    SystemRequest {
        client,
        serial,
        with_input: true,
        text: text.into(),
    }
}

fn scene() -> Scene {
    let mut scene = Scene::new();
    scene
        .connect(
            SETTINGS,
            Grants {
                settings: true,
                ..Grants::default()
            },
        )
        .unwrap();
    // Another system application (the store, Atlas): it may ask the session, not for this.
    scene
        .connect(
            ATLAS,
            Grants {
                store: true,
                ..Grants::default()
            },
        )
        .unwrap();
    scene
}

/// Turns until `client` is told something, at most ten seconds.
fn until_told(storage: &mut Storage, scene: &mut Scene, client: ClientId) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        storage.serve(scene);
        let said = told(scene, client);
        if !said.is_empty() {
            return said;
        }
        assert!(Instant::now() < deadline, "never answered");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn settings_alone_is_told_the_volume_measured_when_it_asks_and_nothing_runs_otherwise() {
    let home = scratch("measured");
    let scope = scope_of(&home);
    let disk = PlayedDisk {
        name: Some("OxyOS".into()),
        capacity: 256_000_000_000,
        available: 100_000_000_000,
        reserved: 12_800_000_000,
        system: 9_000_000_000,
    };
    let mut storage = Storage::new(
        scope.clone(),
        Arc::new(disk),
        home.join("no-installd.socket"),
    )
    .unwrap()
    .with_own_data(own_under(&home));
    let mut scene = scene();
    // Nothing is asked: however many turns run, nothing is measured.
    for _ in 0..10 {
        storage.serve(&mut scene);
    }
    assert_eq!(storage.measured, 0);
    assert!(storage.measuring.is_none());
    // An application without the grant is refused.
    ask(&mut scene, ATLAS, 1, r#"{"storage":{}}"#);
    storage.serve(&mut scene);
    assert_eq!(
        told(&mut scene, ATLAS),
        [r#"{"refused":{"reason":"notPermitted"}}"#]
    );
    assert_eq!(storage.measured, 0);
    // Settings asks: the volume's figures, the Trash's, measured once.
    let item = home.join("old project");
    file(&item.join("a.bin"), 120_000);
    trash::trash(&scope, &item).unwrap();
    ask(&mut scene, SETTINGS, 2, r#"{"storage":{}}"#);
    ask(&mut scene, SETTINGS, 3, r#"{"storage":{}}"#);
    let said = until_told(&mut storage, &mut scene, SETTINGS);
    assert_eq!(said.len(), 2, "both asks answered by one measurement");
    assert_eq!(said[0], said[1]);
    assert_eq!(storage.measured, 1);
    let answer: serde_json::Value = serde_json::from_str(&said[0]).unwrap();
    let measured = &answer["storage"];
    assert_eq!(measured["name"], "OxyOS");
    assert_eq!(measured["capacity"], 256_000_000_000u64);
    assert_eq!(measured["available"], 100_000_000_000u64);
    assert_eq!(measured["reserved"], 12_800_000_000u64);
    assert_eq!(measured["used"], 143_200_000_000u64);
    assert_eq!(measured["system"], 9_000_000_000u64);
    let trashed = du(&home.join(".local/share/Trash/files/old project"));
    assert_eq!(measured["trash"], trashed);
    assert_eq!(measured["apps"], serde_json::json!([]));
    // Done: the measuring thread ended, and nothing runs again until asked.
    assert!(storage.measuring.is_none());
    for _ in 0..10 {
        storage.serve(&mut scene);
    }
    assert_eq!(storage.measured, 1);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_measurement_whose_asker_went_stops_and_answers_no_one() {
    let home = scratch("gone");
    file(&home.join(".local/share/oxyos/apps/so.oxy.allo/notes/1"), 1);
    // The measurement held as it comes to Allo's own data, until the asker is gone: under way
    // whatever the machine's load (a walk long enough to outlast a turn was not, on a loaded
    // machine, where the turn's thread waited while the walk ran to its end).
    let (reached, held) = std::sync::mpsc::channel();
    let (releaser, released) = std::sync::mpsc::channel::<()>();
    let released = std::sync::Mutex::new(released);
    let own = own_under(&home);
    let own: OwnDataOf = Arc::new(move |id: &str| {
        let _ = reached.send(id.to_string());
        let _ = released.lock().unwrap().recv();
        own(id)
    });
    let mut storage = Storage::new(
        scope_of(&home),
        Arc::new(PlayedDisk {
            name: None,
            capacity: 10,
            available: 5,
            reserved: 0,
            system: 0,
        }),
        home.join("no-installd.socket"),
    )
    .unwrap()
    .with_own_data(own);
    // Dropped before the storage, which waits for its measurement: a failure below ends it.
    let release = releaser;
    let apps = home.join("applications");
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(
        apps.join("so.oxy.allo.desktop"),
        "[Desktop Entry]\nType=Application\nName=Allo\nExec=allo\n",
    )
    .unwrap();
    storage.list_applications(Listing {
        policy: oxyos_apps::Policy::none(),
        installed: None,
        directories: vec![apps],
        fixture: None,
    });
    let mut scene = scene();
    ask(&mut scene, SETTINGS, 1, r#"{"storage":{}}"#);
    storage.serve(&mut scene);
    assert_eq!(
        held.recv_timeout(Duration::from_secs(10)).unwrap(),
        "so.oxy.allo"
    );
    assert!(storage.measuring.is_some());
    // Settings closes: the measurement is told to stop, and its answer goes to no one.
    scene.disconnect(SETTINGS);
    storage.serve(&mut scene);
    assert!(storage
        .measuring
        .as_ref()
        .is_some_and(|measuring| measuring.stop.load(Ordering::Acquire)));
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while storage.measuring.is_some() {
        storage.serve(&mut scene);
        assert!(Instant::now() < deadline, "never ended");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(told(&mut scene, SETTINGS).is_empty());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn an_item_is_revealed_in_the_file_manager_only_for_settings_citing_an_input() {
    let home = scratch("reveal");
    let scope = scope_of(&home);
    file(&home.join("Movies/trip.mp4"), 10);
    let mut storage = Storage::new(
        scope,
        Arc::new(MachineDisk),
        home.join("no-installd.socket"),
    )
    .unwrap();
    let mut scene = scene();
    let uri = oxyos_files::uri::of(&home.join("Movies/trip.mp4"), false);
    let reveal = |uri: &str| format!(r#"{{"reveal":{{"uri":"{uri}"}}}}"#);
    // Without an input of the person's, nothing opens.
    ask(&mut scene, SETTINGS, 1, &reveal(&uri));
    assert!(storage.serve(&mut scene).is_empty());
    assert_eq!(
        told(&mut scene, SETTINGS),
        [r#"{"refused":{"reason":"needsInput"}}"#]
    );
    // An item not the person's, or not there, is not shown.
    for other in [
        "file:///etc/hostname".to_string(),
        oxyos_files::uri::of(&home.join("Movies/gone.mp4"), false),
    ] {
        assert_eq!(
            storage.handle(&mut scene, pressed(SETTINGS, 2, &reveal(&other))),
            None
        );
        assert_eq!(
            told(&mut scene, SETTINGS),
            [r#"{"refused":{"reason":"notFound"}}"#]
        );
    }
    // Nor for an application without the grant, even with its press.
    assert_eq!(
        storage.handle(&mut scene, pressed(ATLAS, 3, &reveal(&uri))),
        None
    );
    assert_eq!(
        told(&mut scene, ATLAS),
        [r#"{"refused":{"reason":"notPermitted"}}"#]
    );
    // Settings, with the person's press: the file manager's URL, its folder and the item.
    let reveal = storage
        .handle(&mut scene, pressed(SETTINGS, 4, &reveal(&uri)))
        .expect("revealed");
    assert_eq!(
        reveal,
        Reveal {
            client: SETTINGS,
            serial: 4,
            url: oxyos_apps::reveal_url(&uri),
        }
    );
    storage.revealed(&mut scene, &reveal, true);
    assert_eq!(told(&mut scene, SETTINGS), [r#"{"revealed":{}}"#]);
    let _ = std::fs::remove_dir_all(&home);
}
