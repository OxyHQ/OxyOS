//! The session's side of installing over a service of the tests' own, answering as the install
//! service does, and the person's answers given as the compositor's turn would.

use super::*;
use oxyos_install::install::{
    AvailableUpdate, Committed, Listed, Plan, PlannedCapability, UpdateState, Updated,
};
use oxyos_install::ipc::PlanAnswer;
use oxyos_install::socket::Listener;
use oxyos_install::state::Consent;
use std::sync::{Arc, Mutex};

#[test]
fn the_vocabulary_is_read_strictly_by_grant_and_input() {
    let ask = |text: &str| serde_json::from_str::<Ask>(text);
    assert_eq!(
        ask(r#"{"install":{"id":"so.oxy.notes"}}"#).unwrap(),
        Ask::Install {
            id: Some("so.oxy.notes".into()),
            listing: None,
        }
    );
    assert_eq!(
        ask(r#"{"install":{"listing":"notes"}}"#).unwrap(),
        Ask::Install {
            id: None,
            listing: Some("notes".into()),
        }
    );
    assert_eq!(
        ask(r#"{"status":{"listings":["notes"]}}"#).unwrap(),
        Ask::Status {
            ids: vec![],
            listings: vec!["notes".into()],
        }
    );
    // An install names exactly one application.
    for unnamed in [
        r#"{"install":{}}"#,
        r#"{"install":{"id":"so.oxy.notes","listing":"notes"}}"#,
    ] {
        assert!(!ask(unnamed).unwrap().well_formed(), "{unnamed}");
    }
    assert_eq!(ask(r#"{"state":{}}"#).unwrap(), Ask::State {});
    for refused in [
        r#"{"install":{"id":"so.oxy.notes","path":"/x"}}"#,
        r#"{"install":{"listing":7}}"#,
        r#"{"run":{"command":"rm"}}"#,
        r#"{"state":{},"check":{}}"#,
        r#""state""#,
        "",
    ] {
        assert!(ask(refused).is_err(), "{refused} was read");
    }
    let grants = |store, updates, privacy| oxyos_compositor_core::Grants {
        store,
        updates,
        privacy,
        ..Default::default()
    };
    let install = Ask::Install {
        id: Some("x".into()),
        listing: None,
    };
    assert!(install.allowed(&grants(true, false, false)));
    assert!(!install.allowed(&grants(false, true, true)));
    assert!(Ask::UpdateAll {}.allowed(&grants(false, true, false)));
    assert!(Ask::UpdateAll {}.allowed(&grants(true, false, false)));
    assert!(!Ask::UpdateAll {}.allowed(&grants(false, false, true)));
    assert!(Ask::Privacy {}.allowed(&grants(false, false, true)));
    assert!(!Ask::Privacy {}.allowed(&grants(true, true, false)));
    assert!(install.changes() && Ask::Check {}.changes() && Ask::UpdateAll {}.changes());
    assert!(!Ask::State {}.changes() && !Ask::Privacy {}.changes());
    assert!(!Ask::Status {
        ids: vec![],
        listings: vec![]
    }
    .changes());
}

#[test]
fn only_plain_relative_data_directories_are_removed() {
    let home = Path::new("/home/alice");
    assert_eq!(
        under(home, ".local/share/oxyos/apps/so.oxy.notes"),
        Some(PathBuf::from(
            "/home/alice/.local/share/oxyos/apps/so.oxy.notes"
        ))
    );
    for refused in ["/etc", "../bob", ".local/../../bob", "", "./x"] {
        assert_eq!(under(home, refused), None, "{refused}");
    }
}

/// A service of the tests' own: each connection's requests, recorded, answered by `answer`.
fn service(
    name: &str,
    answer: impl Fn(&Request) -> Vec<Answer> + Send + Sync + 'static,
) -> (PathBuf, Arc<Mutex<Vec<Request>>>) {
    let dir = std::env::temp_dir().join(format!("oxyos-installs-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("socket");
    let listener = Listener::bind(&socket, 0o600).unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let (recorded, answer) = (asked.clone(), Arc::new(answer));
    std::thread::spawn(move || loop {
        let Ok(connection) = listener.accept() else {
            return;
        };
        let (recorded, answer) = (recorded.clone(), answer.clone());
        std::thread::spawn(move || {
            while let Ok(Some(Some(request))) = connection.receive::<Request>() {
                recorded.lock().unwrap().push(request.clone());
                for answer in answer(&request) {
                    if connection.send(&answer).is_err() {
                        return;
                    }
                }
            }
        });
    });
    (socket, asked)
}

fn plan() -> PlanAnswer {
    PlanAnswer {
        plan_id: "p1".into(),
        plan: Plan {
            id: "so.oxy.notes".into(),
            name: "Notes".into(),
            names: BTreeMap::new(),
            version: "1.12.0".into(),
            version_code: 12,
            runtime_abi: 1,
            download: 71,
            capabilities: vec![PlannedCapability {
                name: "network".into(),
                purpose: BTreeMap::from([("en".into(), "Sync".into())]),
                asked_at_first_use: false,
            }],
            new_capabilities: Vec::new(),
            key_rotation: false,
            update: false,
        },
    }
}

/// Runs `ask` as the compositor's turn would: each sheet answered `accepting`, every answer kept.
fn run(
    socket: &Path,
    home: Option<&Path>,
    request: Ask,
    accepting: bool,
) -> (Vec<String>, Vec<Value>) {
    let (done, received) = mpsc::channel();
    let (wake, _woken) = UnixStream::pair().unwrap();
    let back = Back {
        done,
        wake: Arc::new(wake),
    };
    let (socket, home) = (socket.to_path_buf(), home.map(Path::to_path_buf));
    let worker = std::thread::spawn(move || flow(&socket, home.as_deref(), &back, 9, 1, request));
    let (mut sheets, mut answers) = (Vec::new(), Vec::new());
    for done in received {
        match done {
            Done::Sheet { text, reply, .. } => {
                sheets.push(text);
                reply.send(accepting).unwrap();
            }
            Done::Answer {
                client,
                serial,
                text,
            } => {
                assert_eq!((client, serial), (9, 1));
                answers.push(serde_json::from_str(&text).unwrap());
            }
            _ => {}
        }
    }
    worker.join().unwrap();
    (sheets, answers)
}

#[test]
fn an_install_is_committed_only_once_the_sheet_is_accepted() {
    let (socket, asked) = service("install", |request| match request {
        Request::Prepare { .. } => vec![Answer::Plan(plan())],
        Request::Commit { .. } => vec![
            Answer::Progress(oxyos_install::install::Progress::Compiling),
            Answer::Committed(Committed {
                id: "so.oxy.notes".into(),
                version_code: 12,
                switched: true,
                at_next_launch: false,
            }),
        ],
        _ => vec![Answer::refused(oxyos_install::ipc::Reason::InvalidRequest)],
    });
    let (sheets, answers) = run(
        &socket,
        None,
        Ask::Install {
            id: Some("so.oxy.notes".into()),
            listing: None,
        },
        false,
    );
    assert_eq!(sheets.len(), 1);
    let sheet: Value = serde_json::from_str(&sheets[0]).unwrap();
    assert_eq!(sheet["kind"], "install");
    assert_eq!(sheet["plan"]["capabilities"][0]["name"], "network");
    assert_eq!(answers, [json!({"cancelled": {}})]);
    assert!(!asked
        .lock()
        .unwrap()
        .iter()
        .any(|r| matches!(r, Request::Commit { .. })));

    let (_, answers) = run(
        &socket,
        None,
        Ask::Install {
            id: Some("so.oxy.notes".into()),
            listing: None,
        },
        true,
    );
    assert_eq!(answers[0], json!({"progress": "compiling"}));
    assert_eq!(answers[1]["installed"]["versionCode"], 12);
    assert!(asked
        .lock()
        .unwrap()
        .iter()
        .any(|r| matches!(r, Request::Commit { plan_id, .. } if plan_id == "p1")));
}

#[test]
fn removing_asks_the_sheet_then_removes_the_accounts_data_as_the_account() {
    let home = std::env::temp_dir().join(format!("oxyos-installs-{}-home", std::process::id()));
    let data = home.join(".local/share/oxyos/apps/so.oxy.notes");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("notes.db"), b"x").unwrap();
    let entry = home.join("so.oxy.notes.desktop");
    std::fs::write(&entry, "[Desktop Entry]\nName=Notes\n").unwrap();
    let entry_path = entry.display().to_string();
    let (socket, _) = service("uninstall", move |request| match request {
        Request::List { .. } => vec![Answer::Applications(vec![Listed {
            id: "so.oxy.notes".into(),
            version: "1.12.0".into(),
            version_code: 12,
            entry: entry_path.clone(),
            capabilities: BTreeMap::new(),
            staged: None,
        }])],
        Request::Uninstall { id, .. } => vec![Answer::Uninstalled {
            id: id.clone(),
            data_directories: vec![
                ".local/share/oxyos/apps/so.oxy.notes".into(),
                "../escape".into(),
            ],
        }],
        _ => vec![],
    });
    let (sheets, answers) = run(
        &socket,
        Some(&home),
        Ask::Uninstall {
            id: "so.oxy.notes".into(),
        },
        false,
    );
    let sheet: Value = serde_json::from_str(&sheets[0]).unwrap();
    assert_eq!(
        (sheet["kind"].as_str(), sheet["name"].as_str()),
        (Some("uninstall"), Some("Notes"))
    );
    assert_eq!(answers, [json!({"cancelled": {}})]);
    assert!(data.exists(), "cancelled, nothing is removed");
    let (_, answers) = run(
        &socket,
        Some(&home),
        Ask::Uninstall {
            id: "so.oxy.notes".into(),
        },
        true,
    );
    assert_eq!(answers, [json!({"uninstalled": {"id": "so.oxy.notes"}})]);
    assert!(!data.exists());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn an_update_needing_consent_is_asked_and_only_consented_ones_switch() {
    let state = UpdateState {
        checking: false,
        checked_at: None,
        check_failed: None,
        automatic: true,
        updates: vec![AvailableUpdate {
            id: "so.oxy.notes".into(),
            name: "Notes".into(),
            names: BTreeMap::new(),
            from_version: "1.12.0".into(),
            from_version_code: 12,
            to_version: "1.13.0".into(),
            to_version_code: 13,
            download: 40,
            release_notes: BTreeMap::new(),
            status: "awaitingConsent".into(),
            consent: Some(Consent {
                capabilities: vec!["oxy-account".into()],
                key_rotation: false,
            }),
        }],
    };
    let updated = |outcome: &str| Updated {
        id: "so.oxy.notes".into(),
        version_code: Some(13),
        outcome: outcome.into(),
        reason: None,
    };
    let (socket, asked) = service("consent", move |request| match request {
        Request::UpdateAll { .. } => vec![Answer::Updated(vec![updated("awaitingConsent")])],
        Request::UpdateState { .. } => vec![Answer::UpdateState(state.clone())],
        Request::Consent { .. } => vec![Answer::Updated(vec![updated("switched")])],
        _ => vec![],
    });
    let (sheets, answers) = run(&socket, None, Ask::UpdateAll {}, false);
    let sheet: Value = serde_json::from_str(&sheets[0]).unwrap();
    assert_eq!(sheet["kind"], "consent");
    assert_eq!(sheet["update"]["consent"]["capabilities"][0], "oxy-account");
    assert_eq!(answers[0]["updated"][0]["outcome"], "awaitingConsent");
    assert!(!asked
        .lock()
        .unwrap()
        .iter()
        .any(|r| matches!(r, Request::Consent { .. })));
    let (_, answers) = run(&socket, None, Ask::UpdateAll {}, true);
    assert_eq!(answers[0]["updated"][0]["outcome"], "switched");
}

#[test]
fn without_the_service_a_request_fails_and_says_so() {
    let (sheets, answers) = run(Path::new("/nonexistent/socket"), None, Ask::State {}, true);
    assert_eq!(answers[0]["failed"]["reason"], "unavailable");
    assert!(sheets.is_empty(), "a read shows no sheet");
    assert_eq!(answers[0]["failed"].get("told"), None);
}

#[test]
fn without_the_service_a_change_the_person_asked_for_is_said_in_the_sheet() {
    for (request, action) in [
        (
            Ask::Install {
                id: None,
                listing: Some("notes".into()),
            },
            "install",
        ),
        (
            Ask::Uninstall {
                id: "so.oxy.notes".into(),
            },
            "uninstall",
        ),
        (Ask::UpdateAll {}, "update"),
    ] {
        let (sheets, answers) = run(Path::new("/nonexistent/socket"), None, request, true);
        assert_eq!(sheets.len(), 1, "{action}");
        let sheet: Value = serde_json::from_str(&sheets[0]).unwrap();
        assert_eq!(sheet, json!({"kind": "unavailable", "action": action}));
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0]["failed"]["reason"], "unavailable");
        assert_eq!(answers[0]["failed"]["told"], true);
    }
    // A check, which the person did not ask to change anything with, shows none.
    let (sheets, _) = run(Path::new("/nonexistent/socket"), None, Ask::Check {}, true);
    assert!(sheets.is_empty());
}

#[test]
fn a_listing_is_resolved_by_the_service_then_installed_or_said_to_have_no_release() {
    let (socket, asked) = service("listing", |request| match request {
        Request::Resolve { listings, .. } => vec![Answer::Resolved(
            listings
                .iter()
                .map(|l| {
                    (
                        l.clone(),
                        (l == "notes").then(|| "so.oxy.notes".to_string()),
                    )
                })
                .collect(),
        )],
        Request::Prepare { .. } => vec![Answer::Plan(plan())],
        Request::Commit { .. } => vec![Answer::Committed(Committed {
            id: "so.oxy.notes".into(),
            version_code: 12,
            switched: true,
            at_next_launch: false,
        })],
        Request::List { .. } => vec![Answer::Applications(Vec::new())],
        Request::UpdateState { .. } => vec![Answer::UpdateState(UpdateState {
            checking: false,
            checked_at: None,
            check_failed: None,
            automatic: true,
            updates: Vec::new(),
        })],
        _ => vec![Answer::refused(oxyos_install::ipc::Reason::InvalidRequest)],
    });
    let (sheets, answers) = run(
        &socket,
        None,
        Ask::Install {
            id: None,
            listing: Some("notes".into()),
        },
        true,
    );
    assert_eq!(sheets.len(), 1);
    assert_eq!(answers.last().unwrap()["installed"]["id"], "so.oxy.notes");
    assert!(asked
        .lock()
        .unwrap()
        .iter()
        .any(|r| matches!(r, Request::Prepare { id, .. } if id == "so.oxy.notes")));

    // No release of it for this machine: refused, and no sheet is shown.
    let (sheets, answers) = run(
        &socket,
        None,
        Ask::Install {
            id: None,
            listing: Some("mention".into()),
        },
        true,
    );
    assert!(sheets.is_empty());
    assert_eq!(answers, [json!({"refused": {"reason": "noRelease"}})]);

    // Status by listing: the resolved one with its id, the other with none.
    let (_, answers) = run(
        &socket,
        None,
        Ask::Status {
            ids: vec![],
            listings: vec!["notes".into(), "mention".into()],
        },
        true,
    );
    assert_eq!(
        answers[0]["status"],
        json!([
            {"id": "so.oxy.notes", "listing": "notes", "installed": false, "version": null,
             "versionCode": null, "update": null},
            {"listing": "mention", "id": null},
        ])
    );
}
