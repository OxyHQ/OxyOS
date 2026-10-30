//! The user's preferences through real sockets (docs/adr/0019-settings.mdx): every application is
//! told the ones it draws with when it connects; Settings, given its grant by its launch record,
//! is told every one, changes one, and every application is told the change; an application
//! without the grant that asks is closed.

use oxyos_client::Client;
use oxyos_compositor::settings::Settings;
use oxyos_compositor::{Compositor, LaunchRecord};
use oxyos_compositor_core::{Grants, Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{ErrorCode, Event, Rect};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The next Preferences `client` is told: whether it is dark.
fn next_drawing(client: &mut Client) -> bool {
    loop {
        if let Event::Preferences { dark, .. } = client.next_event().unwrap().expect("connected") {
            return dark;
        }
    }
}

#[test]
fn settings_changes_a_preference_and_every_application_follows() {
    let directory = std::env::temp_dir().join(format!("oxyos-preferences-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let output = Rect::new(0, 0, 640, 480);
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: output,
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    let file = directory.join("oxyos").join("preferences");
    // An automatic appearance with no sun to follow (no place): light.
    let mut settings = Settings::load(Some(file.clone()), None);
    settings.apply(&mut compositor, 0);

    // An application, told how to draw when it connects and when that changes; then it asks to
    // change a preference itself, and is closed.
    let (told, drawing) = mpsc::channel();
    let path = socket.clone();
    let application = std::thread::spawn(move || {
        let mut client = Client::connect(&path).unwrap();
        told.send(next_drawing(&mut client)).unwrap();
        told.send(next_drawing(&mut client)).unwrap();
        client.set_preference(1, "appearance", "light").unwrap();
        // The library gives the compositor's ProtocolError as the error it is.
        loop {
            match client.next_event() {
                Ok(Some(_)) => {}
                Ok(None) => return None,
                Err(error) => return Some(error.to_string()),
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let serve = |compositor: &mut Compositor, settings: &mut Settings, record: bool| {
        assert!(
            Instant::now() < deadline,
            "the preferences did not get there"
        );
        compositor.serve(&listener).unwrap();
        // Settings is the process the session started; the application, one it did not.
        compositor.name_clients(&|_| {
            record.then(|| LaunchRecord {
                application: "so.oxy.settings".into(),
                grants: Grants {
                    settings: true,
                    ..Grants::default()
                },
            })
        });
        settings.serve(compositor, 0);
    };
    let first = loop {
        serve(&mut compositor, &mut settings, false);
        if let Ok(dark) = drawing.try_recv() {
            break dark;
        }
    };
    assert!(!first, "light, as the preferences say");

    // Settings: told every preference once granted, then changes the appearance.
    let path = socket.clone();
    let settings_app = std::thread::spawn(move || {
        let mut client = Client::connect(&path).unwrap();
        let (mut kept, mut text) = (Vec::new(), String::new());
        let (mut answered, mut dark) = (false, false);
        loop {
            match client.next_event().unwrap().expect("connected") {
                Event::Preference {
                    index,
                    last,
                    text: chunk,
                } => {
                    if index == 0 {
                        text.clear();
                    }
                    text.push_str(chunk.as_str());
                    if last {
                        kept.push(std::mem::take(&mut text));
                        if kept.len() == oxyos_preferences::KEYS.len() {
                            client.set_preference(9, "appearance", "dark").unwrap();
                        }
                    }
                }
                Event::PreferenceResult { serial, accepted } => {
                    assert_eq!((serial, accepted), (9, true));
                    answered = true;
                }
                Event::Preferences { dark: told, .. } => dark = told,
                _ => {}
            }
            // Answered, drawing dark, and told the change as it is kept.
            if answered && dark && kept.len() > oxyos_preferences::KEYS.len() {
                return kept;
            }
        }
    });
    while !settings_app.is_finished() {
        serve(&mut compositor, &mut settings, true);
    }
    let kept = settings_app.join().unwrap();
    assert_eq!(kept.len(), oxyos_preferences::KEYS.len() + 1, "{kept:?}");
    assert_eq!(kept[0], "appearance=auto");
    assert_eq!(kept.last().unwrap(), "appearance=dark");
    assert!(kept.contains(&"accent=oxy".to_string()));
    // The application follows, with no restart.
    let second = loop {
        serve(&mut compositor, &mut settings, false);
        if let Ok(dark) = drawing.try_recv() {
            break dark;
        }
    };
    assert!(second);
    // The file holds it.
    assert_eq!(
        oxyos_preferences::Preferences::load(&file).appearance,
        oxyos_preferences::Appearance::Dark
    );
    // The application that asked without the grant was closed.
    while !application.is_finished() {
        serve(&mut compositor, &mut settings, false);
    }
    assert_eq!(
        application.join().unwrap(),
        Some(format!("{:?}", ErrorCode::Unauthorized))
    );
    assert_eq!(
        settings.preferences().appearance,
        oxyos_preferences::Appearance::Dark
    );
    std::fs::remove_dir_all(directory).unwrap();
}
