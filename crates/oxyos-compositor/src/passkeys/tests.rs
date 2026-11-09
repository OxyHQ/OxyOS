//! The passkeys' sheet: shown over the asking application's own window, answered by the shell
//! alone with the password, which reaches the service and no one else.

use super::*;
use oxyos_compositor_core::Grants;
use oxyos_protocol::{Event, Request};
use std::cell::RefCell;
use std::rc::Rc;

const APP: ClientId = 1;
const OTHER: ClientId = 2;
const SHELL: ClientId = 3;
const APP_PID: i32 = 4100;
const OTHER_PID: i32 = 4200;
const PASSWORD: &str = "fixture account password";

#[derive(Default)]
struct Service {
    prompts: Vec<Prompt>,
    replies: Vec<Reply>,
}

struct Played(Rc<RefCell<Service>>);

impl PasskeyPrompts for Played {
    fn take(&mut self) -> Vec<Prompt> {
        std::mem::take(&mut self.0.borrow_mut().prompts)
    }

    fn reply(&mut self, reply: Reply) {
        self.0.borrow_mut().replies.push(reply);
    }
}

struct Session {
    scene: Scene,
    passkeys: Passkeys,
    service: Rc<RefCell<Service>>,
    serial: u32,
    /// What the shell was told: "sheet N:TEXT", "dismissed N", "SERIAL:TEXT".
    shell: Vec<String>,
    told: Vec<(ClientId, String)>,
    /// Texts arriving in chunks, by client and what they belong to.
    partial: std::collections::BTreeMap<(ClientId, String), String>,
}

impl Session {
    fn new() -> Self {
        let mut scene = Scene::new();
        scene.connect(APP, Grants::default()).unwrap();
        scene.connect(OTHER, Grants::default()).unwrap();
        scene
            .connect(
                SHELL,
                Grants {
                    shell: true,
                    ..Grants::default()
                },
            )
            .unwrap();
        scene.set_application(APP, "so.oxy.mention");
        scene.set_application(OTHER, "so.oxy.dropsea");
        let service = Rc::new(RefCell::new(Service::default()));
        Self {
            scene,
            passkeys: Passkeys::new(Box::new(Played(service.clone()))),
            service,
            serial: 0,
            shell: Vec::new(),
            told: Vec::new(),
            partial: std::collections::BTreeMap::new(),
        }
    }

    fn turn(&mut self) {
        let client_of = |pid: i32| match pid {
            APP_PID => Some(APP),
            OTHER_PID => Some(OTHER),
            _ => None,
        };
        self.passkeys.serve_scene(&mut self.scene, &client_of);
        for (client, event) in self.scene.drain_events() {
            let (what, text, last) = match event {
                Event::SheetText {
                    sheet, text, last, ..
                } => (format!("sheet {sheet}:"), text.as_str().to_string(), last),
                Event::SheetDismissed { sheet } => {
                    (format!("dismissed {sheet}"), String::new(), true)
                }
                Event::SystemText {
                    serial, text, last, ..
                } => (format!("{serial}:"), text.as_str().to_string(), last),
                _ => continue,
            };
            let whole = self.partial.entry((client, what.clone())).or_default();
            whole.push_str(&text);
            if !last {
                continue;
            }
            let whole = self.partial.remove(&(client, what.clone())).unwrap();
            let text = format!("{what}{whole}");
            if client == SHELL {
                self.shell.push(text.clone());
            }
            self.told.push((client, text));
        }
    }

    fn prompt(&mut self, prompt: Prompt) {
        self.service.borrow_mut().prompts.push(prompt);
        self.turn();
    }

    /// The last sheet the shell was sent: its number and text.
    fn sheet(&self) -> (u32, Value) {
        self.shell
            .iter()
            .rev()
            .find_map(|t| {
                let (number, text) = t.strip_prefix("sheet ")?.split_once(':')?;
                Some((number.parse().ok()?, serde_json::from_str(text).ok()?))
            })
            .unwrap_or_else(|| panic!("no sheet: {:?} {:?}", self.told, self.replies()))
    }

    fn sheets(&self) -> usize {
        self.shell
            .iter()
            .filter(|t| t.starts_with("sheet "))
            .count()
    }

    /// `client` asks `text`, citing an input or not; what it was answered.
    fn ask(&mut self, client: ClientId, text: &str, with_input: bool) -> Option<String> {
        self.serial += 1;
        let serial = self.serial;
        self.passkeys.handle(
            &mut self.scene,
            SystemRequest {
                client,
                serial,
                with_input,
                text: text.into(),
            },
        );
        self.turn();
        let prefix = format!("{serial}:");
        self.told
            .iter()
            .filter(|(to, _)| *to == client)
            .find_map(|(_, t)| t.strip_prefix(&prefix).map(str::to_string))
    }

    fn replies(&self) -> Vec<String> {
        self.service
            .borrow()
            .replies
            .iter()
            .map(|reply| match reply {
                Reply::Confirm {
                    id,
                    choice,
                    password,
                } => format!(
                    "confirm {id} {choice:?} {}",
                    password.as_ref().map_or("-", |p| p.expose())
                ),
                Reply::Cancel { id } => format!("cancel {id}"),
            })
            .collect()
    }
}

fn confirm(kind: Kind, choices: usize) -> Prompt {
    Prompt::Confirm {
        id: "1-abc".into(),
        desktop_entry_id: "so.oxy.mention".into(),
        pid: APP_PID,
        rp_id: "mention.earth".into(),
        user_name: Some(Secret::new("ada@example.com")),
        kind,
        choices: (0..choices)
            .map(|n| Choice {
                user_name: format!("user{n}@example.com"),
                display_name: format!("User {n}"),
            })
            .collect(),
    }
}

fn answer(sheet: u32, choice: Option<usize>, password: &str) -> String {
    let mut text = json!({"passkey": {"sheet": sheet, "password": password}});
    if let Some(choice) = choice {
        text["passkey"]["choice"] = json!(choice);
    }
    text.to_string()
}

#[test]
fn a_get_is_shown_over_its_application_and_answered_by_the_shell_alone() {
    let mut s = Session::new();
    s.prompt(confirm(Kind::Get, 2));
    let (sheet, text) = s.sheet();
    assert_eq!(
        text,
        json!({"kind": "passkey", "mode": "get", "id": "so.oxy.mention",
        "rpId": "mention.earth", "userName": "ada@example.com", "incorrect": false,
        "choices": [
            {"userName": "user0@example.com", "displayName": "User 0"},
            {"userName": "user1@example.com", "displayName": "User 1"},
        ]})
    );
    // The application, an answer citing no input, another sheet and a choice that is none: refused.
    // (The application holds no grant that is answered: it is told nothing at all.)
    assert_eq!(s.ask(APP, &answer(sheet, Some(1), PASSWORD), true), None);
    assert!(s.replies().is_empty());
    assert_eq!(
        s.ask(SHELL, &answer(sheet, Some(1), PASSWORD), false)
            .unwrap(),
        refused_text("needsInput")
    );
    assert_eq!(
        s.ask(SHELL, &answer(sheet + 5, Some(1), PASSWORD), true)
            .unwrap(),
        refused_text("noSuchSheet")
    );
    assert_eq!(
        s.ask(SHELL, &answer(sheet, Some(2), PASSWORD), true)
            .unwrap(),
        refused_text("invalidRequest")
    );
    // A bare acceptance is no answer to a sheet that asks for the password.
    s.scene
        .handle(
            SHELL,
            Request::SheetAnswer {
                sheet,
                accepted: true,
                serial: 0,
            },
        )
        .unwrap();
    s.turn();
    assert!(s.replies().iter().all(|r| !r.starts_with("confirm")));
    assert!(s.replies().is_empty() || s.replies() == ["cancel 1-abc"]);
}

#[test]
fn the_password_reaches_the_service_and_a_wrong_one_asks_again() {
    let mut s = Session::new();
    s.prompt(confirm(Kind::Get, 2));
    let (sheet, _) = s.sheet();
    assert_eq!(s.ask(SHELL, &answer(sheet, Some(1), "wrong"), true), None);
    assert_eq!(s.replies(), ["confirm 1-abc Some(1) wrong"]);
    // Asked again: a new sheet, saying so.
    s.prompt(Prompt::Retry {
        id: "1-abc".into(),
        throttled: false,
        retry_after: None,
    });
    assert!(s.shell.contains(&format!("dismissed {sheet}")));
    let (again, text) = s.sheet();
    assert_ne!(again, sheet);
    assert_eq!(text["incorrect"], true);
    // The account chosen stays chosen.
    assert_eq!(text["chosen"], 1);
    // Too many: how long to wait.
    s.prompt(Prompt::Retry {
        id: "1-abc".into(),
        throttled: true,
        retry_after: Some(30),
    });
    let (third, text) = s.sheet();
    assert_eq!(text["incorrect"], false);
    assert_eq!(text["retryAfter"], 30);
    s.ask(SHELL, &answer(third, None, PASSWORD), true);
    assert_eq!(
        s.replies().last().unwrap(),
        &format!("confirm 1-abc Some(0) {PASSWORD}")
    );
    // Done: the sheet goes.
    s.prompt(Prompt::Done { id: "1-abc".into() });
    assert!(s.shell.contains(&format!("dismissed {third}")));
    // No one but the service was told the password.
    for (client, text) in &s.told {
        assert!(!text.contains(PASSWORD), "{client} was told the password");
    }
    // Answering a sheet gone does nothing.
    assert_eq!(
        s.ask(SHELL, &answer(third, None, PASSWORD), true).unwrap(),
        refused_text("noSuchSheet")
    );
}

#[test]
fn a_create_takes_no_choice_and_a_cancel_is_the_services() {
    let mut s = Session::new();
    s.prompt(confirm(Kind::Create, 0));
    let (sheet, text) = s.sheet();
    assert_eq!(text["mode"], "create");
    s.ask(SHELL, &answer(sheet, Some(3), PASSWORD), true);
    assert_eq!(s.replies(), [format!("confirm 1-abc None {PASSWORD}")]);
    s.service.borrow_mut().replies.clear();
    s.prompt(Prompt::Retry {
        id: "1-abc".into(),
        throttled: false,
        retry_after: None,
    });
    let (again, _) = s.sheet();
    s.scene
        .handle(
            SHELL,
            Request::SheetAnswer {
                sheet: again,
                accepted: false,
                serial: 0,
            },
        )
        .unwrap();
    s.turn();
    assert_eq!(s.replies(), ["cancel 1-abc"]);
}

#[test]
fn a_sheet_that_only_tells_is_acknowledged_without_a_password() {
    for kind in [Kind::Excluded, Kind::None] {
        let mut s = Session::new();
        s.prompt(confirm(kind, 0));
        let (sheet, text) = s.sheet();
        assert_eq!(text["mode"], kind.name());
        // Not answered with a password.
        assert_eq!(
            s.ask(SHELL, &answer(sheet, None, PASSWORD), true).unwrap(),
            refused_text("noSuchSheet")
        );
        // Its one button acknowledges it (the scene takes it only citing the shell's input);
        // a refusal cancels it.
        assert_eq!(
            settled("1".into(), kind, true),
            Reply::Confirm {
                id: "1".into(),
                choice: None,
                password: None
            }
        );
        assert_eq!(
            settled("1".into(), kind, false),
            Reply::Cancel { id: "1".into() }
        );
    }
    for kind in [Kind::Create, Kind::Get] {
        assert_eq!(
            settled("1".into(), kind, true),
            Reply::Cancel { id: "1".into() }
        );
    }
}

#[test]
fn only_over_a_window_of_its_own_application() {
    let mut s = Session::new();
    // A process that is no client, or a client of another application: nothing shown, cancelled.
    for pid in [9999, OTHER_PID] {
        s.service.borrow_mut().replies.clear();
        let Prompt::Confirm {
            id,
            desktop_entry_id,
            rp_id,
            user_name,
            kind,
            choices,
            ..
        } = confirm(Kind::Get, 1)
        else {
            unreachable!()
        };
        s.prompt(Prompt::Confirm {
            id,
            desktop_entry_id,
            pid,
            rp_id,
            user_name,
            kind,
            choices,
        });
        assert_eq!(s.sheets(), 0);
        assert_eq!(s.replies(), ["cancel 1-abc"]);
    }
}

#[test]
fn the_application_going_cancels_and_locked_takes_no_password() {
    let mut s = Session::new();
    s.prompt(confirm(Kind::Get, 1));
    let (sheet, _) = s.sheet();
    s.scene.lock();
    assert_eq!(
        s.ask(SHELL, &answer(sheet, None, PASSWORD), true).unwrap(),
        refused_text("locked")
    );
    assert!(s.replies().is_empty());
    s.scene.disconnect(APP);
    s.turn();
    assert_eq!(s.replies(), ["cancel 1-abc"]);
}

#[test]
fn the_vocabulary_is_read_strictly_and_choices_hide_in_debug() {
    assert!(ours(r#"{"passkey":{"sheet":1,"password":"x"}}"#));
    assert!(!ours(r#"{"password":{"sheet":1,"password":"x"}}"#));
    let mut s = Session::new();
    s.prompt(confirm(Kind::Get, 1));
    let (sheet, _) = s.sheet();
    let extra = json!({"passkey": {"sheet": sheet, "password": "x", "rpId": "evil.example"}});
    assert_eq!(
        s.ask(SHELL, &extra.to_string(), true).unwrap(),
        refused_text("invalidRequest")
    );
    let shown = format!(
        "{:?} {:?}",
        confirm(Kind::Get, 1),
        Reply::Confirm {
            id: "1".into(),
            choice: None,
            password: Some(Secret::new(PASSWORD)),
        }
    );
    assert!(
        !shown.contains("example.com") && !shown.contains(PASSWORD),
        "{shown}"
    );
}
