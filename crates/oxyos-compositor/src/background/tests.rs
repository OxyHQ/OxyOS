//! The session's login items and background activity: who may have them (the launch record, never
//! a claim), registering into the person's one Open at Login list, an installed application's
//! capability granted as it registers and the person told once, and the person's decisions applied
//! at once to its processes.

use super::*;
use oxyos_compositor_core::{Grants, Scene};
use oxyos_install::ipc::{Answer, Request};
use oxyos_preferences::Preferences;
use oxyos_protocol::{Event, NotificationField};
use oxyos_service_socket::Listener;
use std::path::Path;
use std::sync::{Arc, Mutex};

const SHELL: ClientId = 1;
const APP: ClientId = 2;
const SECOND: ClientId = 3;
const NOW: i64 = 1_800_000_000;

/// A directory of the test's own: the install service's entries for the account, beside which its
/// decisions are kept.
fn installed(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oxyos-background-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let applications = dir.join("users/nate/applications");
    std::fs::create_dir_all(&applications).unwrap();
    applications
}

/// The account's decisions on `id`'s capabilities, as the install service keeps them.
fn decide(installed: &Path, id: &str, decisions: &[(&str, &str)]) {
    let capabilities: serde_json::Map<String, serde_json::Value> = decisions
        .iter()
        .map(|(name, decision)| (name.to_string(), serde_json::json!(decision)))
        .collect();
    std::fs::write(
        installed.parent().unwrap().join(format!("{id}.json")),
        serde_json::json!({ "capabilities": capabilities }).to_string(),
    )
    .unwrap();
}

fn launched(id: &str, name: &str, origin: Origin) -> Launched {
    Launched {
        id: id.into(),
        name: name.into(),
        grants: oxyos_apps::Grants::default(),
        origin,
        capabilities: BTreeMap::new(),
        opened: None,
        chooser: false,
        url: None,
    }
}

/// What a client was told of one of its items: the client, the serial answered, the kind and how
/// it stands.
type Told = (ClientId, u32, BackgroundItemKind, BackgroundStatus);

struct Session {
    compositor: Compositor,
    settings: Settings,
    background: BackgroundActivity,
}

impl Session {
    fn new(installed: Option<PathBuf>) -> Self {
        let mut compositor = Compositor::new(Scene::new(), 0);
        compositor
            .scene
            .connect(
                SHELL,
                Grants {
                    shell: true,
                    ..Grants::default()
                },
            )
            .unwrap();
        for client in [APP, SECOND] {
            compositor.scene.connect(client, Grants::default()).unwrap();
        }
        let mut session = Self {
            compositor,
            settings: Settings::new(Preferences::default(), None, None),
            background: BackgroundActivity::new(installed),
        };
        // The first turn sees Open at Login as the session starts.
        session.turn(Turn::default(), &[], &BTreeSet::new(), None);
        session.compositor.scene.drain_events();
        session
    }

    fn turn(
        &mut self,
        turn: Turn,
        clients: &[Client],
        showing: &BTreeSet<u32>,
        installs: Option<&mut Installs>,
    ) -> Vec<u32> {
        self.background.serve_clients(
            &mut self.compositor,
            &mut self.settings,
            turn,
            clients,
            showing,
            installs,
            NOW,
        )
    }

    fn ask(
        &mut self,
        client: ClientId,
        serial: u32,
        kind: BackgroundItemKind,
        action: BackgroundItemAction,
        clients: &[Client],
        showing: &BTreeSet<u32>,
    ) -> Vec<u32> {
        let turn = Turn {
            requests: vec![BackgroundItemRequest {
                client,
                serial,
                kind,
                action,
            }],
            ..Turn::default()
        };
        self.turn(turn, clients, showing, None)
    }

    /// What each client was told of its items, and the notifications the shell was told (each
    /// one's title and body), since the last call.
    fn told(&mut self) -> (Vec<Told>, Vec<String>) {
        let mut states = Vec::new();
        let mut notifications = Vec::new();
        let mut text = String::new();
        for (client, event) in self.compositor.scene.drain_events() {
            match event {
                Event::BackgroundItemState {
                    serial,
                    kind,
                    status,
                } => states.push((client, serial, kind, status)),
                Event::NotificationText {
                    field: NotificationField::Title | NotificationField::Body,
                    text: chunk,
                    last,
                    ..
                } if client == SHELL => {
                    text.push_str(chunk.as_str());
                    if last {
                        text.push('|');
                    }
                }
                Event::Notification { .. } if client == SHELL => {
                    notifications.push(std::mem::take(&mut text));
                }
                _ => {}
            }
        }
        (states, notifications)
    }
}

fn none() -> BTreeSet<u32> {
    BTreeSet::new()
}

fn showing(pids: &[u32]) -> BTreeSet<u32> {
    pids.iter().copied().collect()
}

#[test]
fn a_process_the_session_did_not_launch_or_a_development_entry_has_no_items() {
    let mut session = Session::new(None);
    let development = [(
        APP,
        100,
        Some(launched("so.oxy.inbox", "Inbox", Origin::Development)),
    )];
    let unknown = [(APP, 100, None)];
    for clients in [&development[..], &unknown[..]] {
        for kind in [
            BackgroundItemKind::LoginItem,
            BackgroundItemKind::Background,
        ] {
            for action in [BackgroundItemAction::Register, BackgroundItemAction::Status] {
                session.ask(APP, 4, kind, action, clients, &none());
                assert_eq!(
                    session.told(),
                    (vec![(APP, 4, kind, BackgroundStatus::NotPermitted)], vec![])
                );
            }
        }
    }
    assert!(session.settings.preferences().open_at_login.is_empty());
    assert!(!session.background.watching());
}

#[test]
fn a_system_application_adds_itself_to_open_at_login_once_and_the_person_is_told_once() {
    let mut session = Session::new(None);
    let clients = [(
        APP,
        100,
        Some(launched("so.oxy.dropsea", "Dropsea", Origin::System)),
    )];
    let register = |session: &mut Session| {
        session.ask(
            APP,
            1,
            BackgroundItemKind::LoginItem,
            BackgroundItemAction::Register,
            &clients,
            &none(),
        )
    };
    register(&mut session);
    assert_eq!(
        session.settings.preferences().open_at_login,
        ["so.oxy.dropsea"]
    );
    let (states, notifications) = session.told();
    assert_eq!(
        states,
        [(
            APP,
            1,
            BackgroundItemKind::LoginItem,
            BackgroundStatus::Enabled
        )]
    );
    assert_eq!(
        notifications,
        ["Background Items Added|“Dropsea” added an item that can run in the background. You can manage this in Login Items & Extensions.|"]
    );
    // Again: it is there, nothing changes and the person is not told twice; nor is the
    // application told unasked the list it changed itself.
    register(&mut session);
    session.turn(Turn::default(), &clients, &none(), None);
    assert_eq!(
        session.told(),
        (
            vec![(
                APP,
                1,
                BackgroundItemKind::LoginItem,
                BackgroundStatus::Enabled
            )],
            vec![]
        )
    );
    // Unregistered, it leaves the list.
    session.ask(
        APP,
        2,
        BackgroundItemKind::LoginItem,
        BackgroundItemAction::Unregister,
        &clients,
        &none(),
    );
    assert!(session.settings.preferences().open_at_login.is_empty());
    assert_eq!(
        session.told(),
        (
            vec![(
                APP,
                2,
                BackgroundItemKind::LoginItem,
                BackgroundStatus::NotRegistered
            )],
            vec![]
        )
    );
}

#[test]
fn the_person_changing_open_at_login_tells_the_application_unasked() {
    let mut session = Session::new(None);
    let clients = [
        (
            APP,
            100,
            Some(launched("so.oxy.dropsea", "Dropsea", Origin::System)),
        ),
        (
            SECOND,
            101,
            Some(launched("so.oxy.atlas", "Atlas", Origin::System)),
        ),
    ];
    // Settings (or the dock's menu) adds Dropsea: Dropsea is told, Atlas is not, nobody is
    // notified (the person did it).
    session
        .settings
        .set_open_at_login(
            &mut session.compositor,
            &["so.oxy.dropsea".to_string()],
            NOW,
        )
        .unwrap();
    session.compositor.scene.drain_events();
    session.turn(Turn::default(), &clients, &none(), None);
    assert_eq!(
        session.told(),
        (
            vec![(
                APP,
                0,
                BackgroundItemKind::LoginItem,
                BackgroundStatus::Enabled
            )],
            vec![]
        )
    );
    // Nothing changed: nothing told.
    session.turn(Turn::default(), &clients, &none(), None);
    assert_eq!(session.told(), (vec![], vec![]));
    session
        .settings
        .set_open_at_login(&mut session.compositor, &[], NOW)
        .unwrap();
    session.compositor.scene.drain_events();
    session.turn(Turn::default(), &clients, &none(), None);
    assert_eq!(
        session.told().0,
        [(
            APP,
            0,
            BackgroundItemKind::LoginItem,
            BackgroundStatus::NotRegistered
        )]
    );
}

/// An install service of the test's own, keeping each decision it is asked.
fn service(name: &str, installed: PathBuf) -> (PathBuf, Arc<Mutex<Vec<Request>>>) {
    let dir = std::env::temp_dir().join(format!(
        "oxyos-background-service-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("socket");
    let listener = Listener::bind(&socket, 0o600).unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let recorded = asked.clone();
    std::thread::spawn(move || loop {
        let Ok(connection) = listener.accept() else {
            return;
        };
        let (recorded, installed) = (recorded.clone(), installed.clone());
        std::thread::spawn(move || {
            while let Ok(Some(Some(request))) = connection.receive::<Request>() {
                recorded.lock().unwrap().push(request.clone());
                if let Request::Decide {
                    id,
                    capability,
                    granted,
                    ..
                } = &request
                {
                    let decision = if *granted { "granted" } else { "denied" };
                    decide(&installed, id, &[(capability, decision)]);
                }
                if connection.send(&Answer::Decided).is_err() {
                    return;
                }
            }
        });
    });
    (socket, asked)
}

/// The install service's answers, taken as the compositor's turn takes them.
fn answered(installs: &mut Installs, scene: &mut Scene) -> Turn {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        installs.serve(scene, &|_| crate::installs::Asker::Unknown);
        let grants = installs.take_grants();
        if !grants.is_empty() || std::time::Instant::now() > deadline {
            return Turn {
                grants,
                decisions: installs.take_decisions(),
                ..Turn::default()
            };
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn an_installed_application_is_granted_as_it_registers_and_the_person_told_once() {
    let installed = installed("granted");
    decide(
        &installed,
        "so.oxy.chat",
        &[("background", "undecided"), ("login-item", "undecided")],
    );
    let (socket, asked) = service("granted", installed.clone());
    let mut installs = Installs::new(socket, None).unwrap();
    let mut session = Session::new(Some(installed.clone()));
    let chat = launched("so.oxy.chat", "Chat", Origin::Installed { version_code: 3 });
    let clients = [(APP, 100, Some(chat))];
    // Registering asks the service to keep the account's decision, and answers once it has.
    let turn = Turn {
        requests: vec![BackgroundItemRequest {
            client: APP,
            serial: 5,
            kind: BackgroundItemKind::Background,
            action: BackgroundItemAction::Register,
        }],
        ..Turn::default()
    };
    session.turn(turn, &clients, &showing(&[100]), Some(&mut installs));
    assert_eq!(session.told(), (vec![], vec![]));
    let turn = answered(&mut installs, &mut session.compositor.scene);
    session.turn(turn, &clients, &showing(&[100]), Some(&mut installs));
    let (states, notifications) = session.told();
    assert_eq!(
        states,
        [(
            APP,
            5,
            BackgroundItemKind::Background,
            BackgroundStatus::Enabled
        )]
    );
    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].starts_with("Background Items Added|“Chat” added an item"));
    let decided: Vec<(String, String, bool)> = asked
        .lock()
        .unwrap()
        .iter()
        .filter_map(|request| match request {
            Request::Decide {
                id,
                capability,
                granted,
                uid: None,
            } => Some((id.clone(), capability.clone(), *granted)),
            _ => None,
        })
        .collect();
    assert_eq!(
        decided,
        [("so.oxy.chat".to_string(), "background".to_string(), true)]
    );
    // Its next process registers with the decision kept: nothing asked, nobody told again.
    session.ask(
        SECOND,
        6,
        BackgroundItemKind::Background,
        BackgroundItemAction::Register,
        &[(
            SECOND,
            101,
            Some(launched(
                "so.oxy.chat",
                "Chat",
                Origin::Installed { version_code: 3 },
            )),
        )],
        &showing(&[101]),
    );
    assert_eq!(
        session.told(),
        (
            vec![(
                SECOND,
                6,
                BackgroundItemKind::Background,
                BackgroundStatus::Enabled
            )],
            vec![]
        )
    );
    // The service is asked no other decision (the update state it is watched for aside).
    assert_eq!(
        asked
            .lock()
            .unwrap()
            .iter()
            .filter(|request| matches!(request, Request::Decide { .. }))
            .count(),
        1
    );
    let _ = std::fs::remove_dir_all(
        installed
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap(),
    );
}

#[test]
fn an_application_that_did_not_declare_it_or_was_turned_off_is_refused_and_nobody_told() {
    let installed = installed("refused");
    decide(&installed, "so.oxy.chat", &[("background", "denied")]);
    let mut session = Session::new(Some(installed.clone()));
    let clients = [(
        APP,
        100,
        Some(launched(
            "so.oxy.chat",
            "Chat",
            Origin::Installed { version_code: 3 },
        )),
    )];
    session.ask(
        APP,
        1,
        BackgroundItemKind::Background,
        BackgroundItemAction::Register,
        &clients,
        &showing(&[100]),
    );
    session.ask(
        APP,
        2,
        BackgroundItemKind::LoginItem,
        BackgroundItemAction::Register,
        &clients,
        &showing(&[100]),
    );
    assert_eq!(
        session.told(),
        (
            vec![
                (
                    APP,
                    1,
                    BackgroundItemKind::Background,
                    BackgroundStatus::RequiresApproval
                ),
                (
                    APP,
                    2,
                    BackgroundItemKind::LoginItem,
                    BackgroundStatus::NotPermitted
                ),
            ],
            vec![]
        )
    );
    assert!(session.settings.preferences().open_at_login.is_empty());
    assert!(!session.background.watching());
    let _ = std::fs::remove_dir_all(
        installed
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap(),
    );
}

#[test]
fn its_window_closed_it_runs_in_the_background_once_per_application() {
    let installed = installed("once");
    decide(&installed, "so.oxy.chat", &[("background", "granted")]);
    let mut session = Session::new(Some(installed.clone()));
    let chat = || {
        Some(launched(
            "so.oxy.chat",
            "Chat",
            Origin::Installed { version_code: 3 },
        ))
    };
    let clients = [(APP, 100, chat()), (SECOND, 101, chat())];
    for (client, serial) in [(APP, 1), (SECOND, 2)] {
        session.ask(
            client,
            serial,
            BackgroundItemKind::Background,
            BackgroundItemAction::Register,
            &clients,
            &showing(&[100, 101]),
        );
    }
    session.told();
    // The first's window closes: it runs on, in the background.
    assert!(session
        .turn(Turn::default(), &clients, &showing(&[101]), None)
        .is_empty());
    assert_eq!(
        session.background.in_background(),
        BTreeSet::from(["so.oxy.chat".to_string()])
    );
    // The second's closes too: its application already runs there, and it ends.
    assert_eq!(
        session.turn(Turn::default(), &clients, &none(), None),
        [101]
    );
    // The first unregisters: it may not run without a window, and ends.
    assert_eq!(
        session.ask(
            APP,
            3,
            BackgroundItemKind::Background,
            BackgroundItemAction::Unregister,
            &clients[..1],
            &none(),
        ),
        [100]
    );
    assert_eq!(
        session.told().0,
        [(
            APP,
            3,
            BackgroundItemKind::Background,
            BackgroundStatus::NotRegistered
        )]
    );
    assert!(session.background.in_background().is_empty());
    assert!(!session.background.watching());
    let _ = std::fs::remove_dir_all(
        installed
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap(),
    );
}

#[test]
fn turned_off_its_process_in_the_background_ends_and_those_with_windows_lose_the_right() {
    let installed = installed("off");
    decide(&installed, "so.oxy.chat", &[("background", "granted")]);
    let mut session = Session::new(Some(installed.clone()));
    let chat = || {
        Some(launched(
            "so.oxy.chat",
            "Chat",
            Origin::Installed { version_code: 3 },
        ))
    };
    let clients = [(APP, 100, chat()), (SECOND, 101, chat())];
    for (client, serial) in [(APP, 1), (SECOND, 2)] {
        session.ask(
            client,
            serial,
            BackgroundItemKind::Background,
            BackgroundItemAction::Register,
            &clients,
            &showing(&[100, 101]),
        );
    }
    session.turn(Turn::default(), &clients, &showing(&[101]), None);
    session.told();
    // The person turns it off (the install service kept it): the one in the background ends at
    // once, and the one with a window is told, unasked.
    decide(&installed, "so.oxy.chat", &[("background", "denied")]);
    let turn = Turn {
        decisions: vec![Decision {
            id: "so.oxy.chat".into(),
            capability: "background".into(),
            granted: false,
        }],
        ..Turn::default()
    };
    assert_eq!(session.turn(turn, &clients, &showing(&[101]), None), [100]);
    assert_eq!(
        session.told().0,
        [(
            SECOND,
            0,
            BackgroundItemKind::Background,
            BackgroundStatus::RequiresApproval
        )]
    );
    // The other's window closes: without the right, it ends as any application's process does.
    assert_eq!(
        session.turn(Turn::default(), &clients[1..], &none(), None),
        [101]
    );
    assert!(session.background.in_background().is_empty());
    // Asked again, it requires the person's approval.
    session.ask(
        SECOND,
        3,
        BackgroundItemKind::Background,
        BackgroundItemAction::Register,
        &clients[1..],
        &none(),
    );
    assert_eq!(
        session.told().0,
        [(
            SECOND,
            3,
            BackgroundItemKind::Background,
            BackgroundStatus::RequiresApproval
        )]
    );
    let _ = std::fs::remove_dir_all(
        installed
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap(),
    );
}

#[test]
fn quit_ends_it_with_its_windows_where_closing_them_keeps_it_running() {
    let installed = installed("quit");
    decide(&installed, "so.oxy.chat", &[("background", "granted")]);
    let mut session = Session::new(Some(installed.clone()));
    let chat = || {
        Some(launched(
            "so.oxy.chat",
            "Chat",
            Origin::Installed { version_code: 3 },
        ))
    };
    let clients = [(APP, 100, chat())];
    session.ask(
        APP,
        1,
        BackgroundItemKind::Background,
        BackgroundItemAction::Register,
        &clients,
        &showing(&[100]),
    );
    session.told();
    // The person quits Chat (the dock's Quit): its window is asked to close and closes; it ends.
    let turn = Turn {
        quits: vec![APP],
        ..Turn::default()
    };
    assert!(session
        .turn(turn, &clients, &showing(&[100]), None)
        .is_empty());
    assert_eq!(
        session.turn(Turn::default(), &clients, &none(), None),
        [100]
    );
    assert!(session.background.in_background().is_empty());
    assert!(!session.background.watching());
    let _ = std::fs::remove_dir_all(
        installed
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap(),
    );
}

#[test]
fn in_safe_mode_no_application_but_the_systems_runs_in_the_background_and_nothing_is_changed() {
    let installed = installed("safe-mode");
    decide(
        &installed,
        "so.oxy.chat",
        &[("background", "granted"), ("login-item", "granted")],
    );
    let mut session = Session::new(Some(installed.clone()));
    session.background.set_safe_mode();
    let chat = || {
        Some(launched(
            "so.oxy.chat",
            "Chat",
            Origin::Installed { version_code: 3 },
        ))
    };
    let clients = [
        (APP, 100, chat()),
        (
            SECOND,
            101,
            Some(launched("so.oxy.dropsea", "Dropsea", Origin::System)),
        ),
    ];
    for (client, serial) in [(APP, 1), (SECOND, 2)] {
        session.ask(
            client,
            serial,
            BackgroundItemKind::Background,
            BackgroundItemAction::Register,
            &clients,
            &showing(&[100, 101]),
        );
    }
    // The installed application is not permitted; the image's own is.
    assert_eq!(
        session.told(),
        (
            vec![
                (
                    APP,
                    1,
                    BackgroundItemKind::Background,
                    BackgroundStatus::NotPermitted
                ),
                (
                    SECOND,
                    2,
                    BackgroundItemKind::Background,
                    BackgroundStatus::Enabled
                ),
            ],
            vec![]
        )
    );
    // Its window closed, Chat ends as any application does (it registered nothing); Dropsea runs on.
    assert!(session
        .turn(Turn::default(), &clients, &none(), None)
        .is_empty());
    assert_eq!(
        session.background.in_background(),
        BTreeSet::from(["so.oxy.dropsea".to_string()])
    );
    // Its login item is the person's still: it may add itself for the next normal start.
    session.ask(
        APP,
        3,
        BackgroundItemKind::LoginItem,
        BackgroundItemAction::Register,
        &clients,
        &none(),
    );
    assert_eq!(
        session.settings.preferences().open_at_login,
        ["so.oxy.chat"]
    );
    // The account's decision is as it was.
    assert_eq!(
        oxyos_apps::capability_decisions(&installed, "so.oxy.chat")
            .get("background")
            .map(String::as_str),
        Some("granted")
    );
    let _ = std::fs::remove_dir_all(
        installed
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap(),
    );
}
