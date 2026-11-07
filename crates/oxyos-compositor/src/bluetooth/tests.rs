//! The session's side of Bluetooth over a BlueZ played in memory (its commands seen, its events
//! given): who may ask, what is told to whom, discovery held and let go, and the agent's questions
//! as the shell's sheets, a code reaching BlueZ alone.

use super::*;
use oxyos_bluetooth::{Adapter, Ask as Asked, Device, Kind};
use oxyos_compositor_core::Grants;
use oxyos_protocol::{Event as Told, Request};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const SHELL: ClientId = 1;
const SETTINGS: ClientId = 2;
const APP: ClientId = 3;

/// BlueZ played in memory: every command it is sent, and the events the test gives it.
#[derive(Clone, Default)]
struct Played {
    commands: Arc<Mutex<Vec<Command>>>,
    events: Arc<Mutex<Vec<Event>>>,
}

impl Played {
    fn give(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }

    fn sent(&self) -> Vec<Command> {
        self.commands.lock().unwrap().clone()
    }
}

impl Reach for Played {
    fn run(
        &self,
        stop: &Stop,
        commands: &async_channel::Receiver<Command>,
        emit: &mut dyn FnMut(Event),
    ) -> io::Result<()> {
        emit(Event::State(machine()));
        while !stop.given() {
            while let Ok(command) = commands.try_recv() {
                self.commands.lock().unwrap().push(command);
            }
            for event in std::mem::take(&mut *self.events.lock().unwrap()) {
                emit(event);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }

    fn socket(&self) -> Option<PathBuf> {
        None
    }
}

fn device(id: &str, name: &str, kind: Kind, paired: bool, connected: bool) -> Device {
    Device {
        id: id.into(),
        address: id.into(),
        name: name.into(),
        kind,
        paired,
        connected,
        busy: None,
        battery: None,
        audio: kind == Kind::Headphones,
        input: kind == Kind::Keyboard,
    }
}

fn machine() -> State {
    State {
        available: true,
        adapter: Some(Adapter {
            name: "ThinkPad".into(),
            address: "00:1A:7D:DA:71:13".into(),
            powered: true,
            ..Adapter::default()
        }),
        devices: vec![
            device(
                "AC:90:85:11:22:33",
                "AirPods Pro",
                Kind::Headphones,
                true,
                true,
            ),
            device("3C:28:6D:01:02:03", "Pixel 8", Kind::Phone, false, false),
        ],
    }
}

struct Session {
    played: Played,
    bluetooth: Bluetooth,
    scene: Scene,
    serial: u32,
    told: BTreeMap<ClientId, Vec<String>>,
    partial: BTreeMap<(ClientId, String), String>,
}

impl Session {
    fn new() -> Self {
        let played = Played::default();
        let mut scene = Scene::new();
        for (client, grants) in [
            (
                SHELL,
                Grants {
                    shell: true,
                    ..Grants::default()
                },
            ),
            (
                SETTINGS,
                Grants {
                    bluetooth: true,
                    settings: true,
                    ..Grants::default()
                },
            ),
            (
                APP,
                Grants {
                    network: true,
                    ..Grants::default()
                },
            ),
        ] {
            scene.connect(client, grants).unwrap();
        }
        let bluetooth = Bluetooth::new(Arc::new(played.clone())).unwrap();
        let mut session = Self {
            played,
            bluetooth,
            scene,
            serial: 0,
            told: BTreeMap::new(),
            partial: BTreeMap::new(),
        };
        session.until(|s| s.bluetooth.state().is_some());
        session
    }

    fn turn(&mut self) {
        self.bluetooth.serve(&mut self.scene);
        for (client, event) in self.scene.drain_events() {
            let (what, text, last) = match event {
                Told::SystemText {
                    serial, text, last, ..
                } => (format!("{serial}:"), text.as_str().to_string(), last),
                Told::SheetText {
                    sheet, text, last, ..
                } => (format!("sheet {sheet}:"), text.as_str().to_string(), last),
                Told::SheetDismissed { sheet } => {
                    (format!("dismissed {sheet}"), String::new(), true)
                }
                _ => continue,
            };
            let whole = self.partial.entry((client, what.clone())).or_default();
            whole.push_str(&text);
            if last {
                let whole = self.partial.remove(&(client, what.clone())).unwrap();
                self.told
                    .entry(client)
                    .or_default()
                    .push(format!("{what}{whole}"));
            }
        }
    }

    fn until(&mut self, mut condition: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.turn();
            if condition(self) {
                return;
            }
            assert!(Instant::now() < deadline, "not in time: {:?}", self.told);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn ask(&mut self, client: ClientId, text: &str, with_input: bool) -> u32 {
        self.serial += 1;
        self.bluetooth.handle(
            &mut self.scene,
            SystemRequest {
                client,
                serial: self.serial,
                with_input,
                text: text.into(),
            },
        );
        self.serial
    }

    fn told(&self, client: ClientId) -> Vec<String> {
        self.told.get(&client).cloned().unwrap_or_default()
    }

    fn answer(&mut self, client: ClientId, serial: u32) -> Option<String> {
        self.turn();
        let prefix = format!("{serial}:");
        self.told(client)
            .iter()
            .find_map(|t| t.strip_prefix(&prefix).map(str::to_string))
    }

    /// Waits until BlueZ was sent a command `matching`; it.
    fn sent(&mut self, matching: impl Fn(&Command) -> bool) -> Command {
        self.until(|s| s.played.sent().iter().any(&matching));
        self.played.sent().into_iter().find(matching).unwrap()
    }

    fn discovering(&self) -> Option<bool> {
        self.played.sent().iter().rev().find_map(|c| match c {
            Command::Discover(on) => Some(*on),
            _ => None,
        })
    }

    fn sheets(&self) -> Vec<(u32, Value)> {
        self.told(SHELL)
            .iter()
            .filter_map(|t| {
                let rest = t.strip_prefix("sheet ")?;
                let (number, text) = rest.split_once(':')?;
                Some((number.parse().ok()?, serde_json::from_str(text).ok()?))
            })
            .collect()
    }

    fn wait_sheet(&mut self, after: usize) -> (u32, Value) {
        self.until(|s| s.sheets().len() > after);
        self.sheets().pop().unwrap()
    }

    fn dismissed(&self, sheet: u32) -> bool {
        self.told(SHELL)
            .iter()
            .any(|t| *t == format!("dismissed {sheet}"))
    }

    fn refuse_sheet(&mut self, sheet: u32) {
        self.scene
            .handle(
                SHELL,
                Request::SheetAnswer {
                    sheet,
                    accepted: false,
                    serial: 0,
                },
            )
            .unwrap();
    }
}

#[test]
fn the_vocabulary_is_this_parts_and_read_strictly() {
    assert!(ours(r#"{"bluetooth":{}}"#));
    assert!(ours(r#"{"bluetoothConnect":{"id":"AC:90:85:11:22:33"}}"#));
    assert!(!ours(r#"{"join":{"ssid":"Home"}}"#), "the networks'");
    assert!(!ours(r#"{"bluetooth":{},"bluetoothPower":{"on":true}}"#));
    let ask = |text: &str| serde_json::from_str::<Ask>(text);
    assert!(ask(r#"{"bluetoothPower":{"on":"yes"}}"#).is_err());
    assert!(ask(r#"{"bluetoothConnect":{"id":"x","pin":"0000"}}"#).is_err());
    assert!(ask(r#"{"bluetoothCode":{"sheet":1}}"#).is_err());
}

#[test]
fn the_state_is_told_to_the_shell_and_the_bluetooth_grant_alone() {
    let mut s = Session::new();
    let told = |lines: Vec<String>| lines.iter().any(|t| t.starts_with("0:{\"bluetooth\""));
    assert!(told(s.told(SHELL)) && told(s.told(SETTINGS)));
    assert!(!told(s.told(APP)));
    let asked = s.ask(SETTINGS, r#"{"bluetooth":{}}"#, false);
    let state: Value = serde_json::from_str(&s.answer(SETTINGS, asked).unwrap()).unwrap();
    assert_eq!(state["bluetooth"]["devices"][0]["name"], "AirPods Pro");
    assert_eq!(state["bluetooth"]["adapter"]["name"], "ThinkPad");
    let refused = s.ask(APP, r#"{"bluetooth":{}}"#, false);
    assert_eq!(
        s.answer(APP, refused).unwrap(),
        r#"{"refused":{"reason":"notPermitted"}}"#
    );
    // The shell may not discover, forget or rename; only Settings may.
    for text in [
        r#"{"bluetoothDiscover":{"on":true}}"#,
        r#"{"bluetoothForget":{"id":"AC:90:85:11:22:33"}}"#,
        r#"{"bluetoothRename":{"id":"AC:90:85:11:22:33","name":"x"}}"#,
    ] {
        let asked = s.ask(SHELL, text, true);
        assert_eq!(
            s.answer(SHELL, asked).unwrap(),
            r#"{"refused":{"reason":"notPermitted"}}"#,
            "{text}"
        );
    }
    // The top bar's status from it: on, one connected.
    let status = s.bluetooth.status().unwrap();
    assert_eq!(status.state, oxyos_protocol::BluetoothState::On);
    assert_eq!(status.connected, 1);
    assert!(s.bluetooth.take_changed());
}

#[test]
fn changes_need_an_input_and_none_is_taken_while_locked() {
    let mut s = Session::new();
    for text in [
        r#"{"bluetoothPower":{"on":false}}"#,
        r#"{"bluetoothConnect":{"id":"3C:28:6D:01:02:03"}}"#,
        r#"{"bluetoothDisconnect":{"id":"AC:90:85:11:22:33"}}"#,
        r#"{"bluetoothForget":{"id":"AC:90:85:11:22:33"}}"#,
        r#"{"bluetoothRename":{"id":"AC:90:85:11:22:33","name":"x"}}"#,
    ] {
        let asked = s.ask(SETTINGS, text, false);
        assert_eq!(
            s.answer(SETTINGS, asked).unwrap(),
            r#"{"refused":{"reason":"needsInput"}}"#,
            "{text}"
        );
    }
    s.scene.lock();
    let asked = s.ask(SHELL, r#"{"bluetoothPower":{"on":false}}"#, true);
    assert_eq!(
        s.answer(SHELL, asked).unwrap(),
        r#"{"refused":{"reason":"locked"}}"#
    );
    s.turn();
    assert!(
        s.played
            .sent()
            .iter()
            .all(|c| matches!(c, Command::Discover(_))),
        "nothing reached BlueZ: {:?}",
        s.played.sent()
    );
}

#[test]
fn discovery_is_held_by_settings_while_its_page_is_open() {
    let mut s = Session::new();
    s.turn();
    assert_eq!(s.discovering(), None, "nothing discovers by itself");
    let asked = s.ask(SETTINGS, r#"{"bluetoothDiscover":{"on":true}}"#, false);
    assert_eq!(
        s.answer(SETTINGS, asked).unwrap(),
        r#"{"discovering":{"on":true}}"#
    );
    s.until(|s| s.discovering() == Some(true));
    s.ask(SETTINGS, r#"{"bluetoothDiscover":{"on":false}}"#, false);
    s.until(|s| s.discovering() == Some(false));
    // Held again: locked, it stops; unlocked, it goes on; Settings gone, it ends.
    s.ask(SETTINGS, r#"{"bluetoothDiscover":{"on":true}}"#, false);
    s.until(|s| s.discovering() == Some(true));
    s.scene.lock();
    s.until(|s| s.discovering() == Some(false));
    s.scene.unlock();
    s.until(|s| s.discovering() == Some(true));
    s.scene.disconnect(SETTINGS);
    s.until(|s| s.discovering() == Some(false));
    let discovers = s
        .played
        .sent()
        .iter()
        .filter(|c| matches!(c, Command::Discover(_)))
        .count();
    assert_eq!(discovers, 6, "each change once: {:?}", s.played.sent());
}

#[test]
fn a_connection_asks_its_code_in_the_shells_sheet_and_answers_its_asker() {
    let mut s = Session::new();
    let asked = s.ask(
        SETTINGS,
        r#"{"bluetoothConnect":{"id":"3C:28:6D:01:02:03"}}"#,
        true,
    );
    let Command::Connect { call, .. } = s.sent(|c| matches!(c, Command::Connect { .. })) else {
        unreachable!()
    };
    // BlueZ asks to compare a code: the shell's sheet, never Settings'.
    let (question, answered) = Question::new(
        7,
        Asked::Confirm(4821),
        "3C:28:6D:01:02:03".into(),
        "Pixel 8".into(),
        Kind::Phone,
    );
    s.played.give(Event::Question(question));
    let (sheet, text) = s.wait_sheet(0);
    assert_eq!(
        text,
        json!({"kind": "bluetooth-confirm", "name": "Pixel 8", "device": "phone", "code": "004821"})
    );
    assert!(!s
        .told(SETTINGS)
        .iter()
        .any(|t| t.contains("bluetooth-confirm")));
    // Refused by the person: BlueZ is told no, the connection ends cancelled, and nothing more
    // is said.
    s.refuse_sheet(sheet);
    s.until(|_| !answered.is_empty());
    assert_eq!(answered.try_recv(), Ok(Answer::Refuse));
    s.played.give(Event::Done {
        call,
        outcome: Err(Failure::Cancelled),
    });
    s.until(|s| s.answer(SETTINGS, asked).is_some());
    assert_eq!(
        s.answer(SETTINGS, asked).unwrap(),
        r#"{"failed":{"reason":"cancelled"}}"#
    );
    assert_eq!(s.sheets().len(), 1, "no failure said for a cancel");

    // Again, from the Control Center: it fails, and the session says so.
    let asked = s.ask(
        SHELL,
        r#"{"bluetoothConnect":{"id":"3C:28:6D:01:02:03"}}"#,
        true,
    );
    let call = loop {
        if let Some(Command::Connect { call: c, .. }) = s
            .played
            .sent()
            .into_iter()
            .filter(|c| matches!(c, Command::Connect { .. }))
            .nth(1)
        {
            break c;
        }
        s.turn();
    };
    s.played.give(Event::Done {
        call,
        outcome: Err(Failure::Timeout),
    });
    let (_, note) = s.wait_sheet(1);
    assert_eq!(
        note,
        json!({"kind": "bluetooth-failed", "name": "Pixel 8", "device": "phone", "reason": "timeout"})
    );
    assert_eq!(
        s.answer(SHELL, asked).unwrap(),
        r#"{"failed":{"reason":"timeout"}}"#
    );
}

#[test]
fn a_code_typed_reaches_bluez_alone_from_the_shell() {
    let mut s = Session::new();
    let (question, answered) = Question::new(
        3,
        Asked::Passkey,
        "00:1F:20:0A:0B:0C".into(),
        "Old Keyboard".into(),
        Kind::Keyboard,
    );
    s.played.give(Event::Question(question));
    let (sheet, text) = s.wait_sheet(0);
    assert_eq!(text["kind"], "bluetooth-code");
    assert_eq!(text["code"], "passkey");
    let code = |sheet: u32, code: &str| {
        json!({"bluetoothCode": {"sheet": sheet, "code": code}}).to_string()
    };
    // Not from Settings, not without an input, not for another sheet, not what BlueZ takes.
    let asked = s.ask(SETTINGS, &code(sheet, "482913"), true);
    assert_eq!(
        s.answer(SETTINGS, asked).unwrap(),
        r#"{"refused":{"reason":"notPermitted"}}"#
    );
    let asked = s.ask(SHELL, &code(sheet, "482913"), false);
    assert_eq!(
        s.answer(SHELL, asked).unwrap(),
        r#"{"refused":{"reason":"needsInput"}}"#
    );
    let asked = s.ask(SHELL, &code(sheet + 9, "482913"), true);
    assert_eq!(
        s.answer(SHELL, asked).unwrap(),
        r#"{"refused":{"reason":"noSuchSheet"}}"#
    );
    let asked = s.ask(SHELL, &code(sheet, "48a913"), true);
    assert_eq!(
        s.answer(SHELL, asked).unwrap(),
        r#"{"refused":{"reason":"invalidRequest"}}"#
    );
    assert!(answered.is_empty());
    s.ask(SHELL, &code(sheet, "482913"), true);
    s.turn();
    assert_eq!(answered.try_recv(), Ok(Answer::Code("482913".into())));
    assert!(s.dismissed(sheet));
    // The code is in nothing told to any client.
    for client in [SHELL, SETTINGS, APP] {
        assert!(!s
            .told(client)
            .iter()
            .any(|t| t.contains("482913") && !t.contains("bluetoothCode")));
    }
}

#[test]
fn a_code_shown_ends_with_its_pairing_or_cancels_it() {
    let mut s = Session::new();
    let (question, _answered) = Question::new(
        4,
        Asked::ShowPasskey(7302),
        "00:1F:20:0A:0B:0C".into(),
        "Old Keyboard".into(),
        Kind::Keyboard,
    );
    s.played.give(Event::Question(question));
    let (sheet, text) = s.wait_sheet(0);
    assert_eq!(
        text,
        json!({"kind": "bluetooth-display", "name": "Old Keyboard", "device": "keyboard", "code": "007302"})
    );
    // Typed on the keyboard, the pairing ends it.
    s.played.give(Event::QuestionEnded(4));
    s.until(|s| s.dismissed(sheet));
    // Shown again and cancelled by the person: the pairing is cancelled.
    let (question, _answered) = Question::new(
        5,
        Asked::ShowPinCode("0451".into()),
        "00:1F:20:0A:0B:0C".into(),
        "Old Keyboard".into(),
        Kind::Keyboard,
    );
    s.played.give(Event::Question(question));
    let (sheet, text) = s.wait_sheet(1);
    assert_eq!(text["code"], "0451");
    s.refuse_sheet(sheet);
    let cancelled = s.sent(|c| matches!(c, Command::CancelPairing { .. }));
    assert_eq!(
        cancelled,
        Command::CancelPairing {
            id: "00:1F:20:0A:0B:0C".into()
        }
    );
}

#[test]
fn a_device_asking_while_locked_is_refused_unasked() {
    let mut s = Session::new();
    s.scene.lock();
    let (question, answered) = Question::new(
        9,
        Asked::Authorize,
        "3C:28:6D:01:02:03".into(),
        "Pixel 8".into(),
        Kind::Phone,
    );
    s.played.give(Event::Question(question));
    s.until(|_| !answered.is_empty());
    assert_eq!(answered.try_recv(), Ok(Answer::Refuse));
    assert!(s.sheets().is_empty());
    s.scene.unlock();
    // Unlocked, a service asked is the shell's sheet.
    let (question, _answered) = Question::new(
        10,
        Asked::Service("0000110d-0000-1000-8000-00805f9b34fb".into()),
        "AC:90:85:11:22:33".into(),
        "AirPods Pro".into(),
        Kind::Headphones,
    );
    s.played.give(Event::Question(question));
    let (_, text) = s.wait_sheet(0);
    assert_eq!(
        text,
        json!({"kind": "bluetooth-service", "name": "AirPods Pro", "device": "headphones", "service": "audio"})
    );
}

#[test]
fn outcomes_answer_their_askers() {
    let mut s = Session::new();
    let power = s.ask(SHELL, r#"{"bluetoothPower":{"on":false}}"#, true);
    let rename = s.ask(
        SETTINGS,
        r#"{"bluetoothRename":{"id":"AC:90:85:11:22:33","name":"Nate's AirPods"}}"#,
        true,
    );
    let Command::Power { call: powered, on } = s.sent(|c| matches!(c, Command::Power { .. }))
    else {
        unreachable!()
    };
    assert!(!on);
    let Command::Rename {
        call: renamed,
        name,
        ..
    } = s.sent(|c| matches!(c, Command::Rename { .. }))
    else {
        unreachable!()
    };
    assert_eq!(name, "Nate's AirPods");
    s.played.give(Event::Done {
        call: renamed,
        outcome: Ok(Done::Renamed(name)),
    });
    s.played.give(Event::Done {
        call: powered,
        outcome: Ok(Done::Powered(false)),
    });
    s.until(|s| s.answer(SHELL, power).is_some() && s.answer(SETTINGS, rename).is_some());
    assert_eq!(
        s.answer(SHELL, power).unwrap(),
        r#"{"powered":{"on":false}}"#
    );
    assert_eq!(
        s.answer(SETTINGS, rename).unwrap(),
        r#"{"renamed":{"id":"AC:90:85:11:22:33","name":"Nate's AirPods"}}"#
    );
    // A state changed is told to each watcher once.
    let mut off = machine();
    off.adapter.as_mut().unwrap().powered = false;
    s.played.give(Event::State(off.clone()));
    s.played.give(Event::State(off));
    s.until(|s| s.bluetooth.state().is_some_and(|state| !state.powered()));
    s.turn();
    let offs = s
        .told(SHELL)
        .iter()
        .filter(|t| t.starts_with("0:") && t.contains(r#""powered":false"#))
        .count();
    assert_eq!(offs, 1);
}

#[test]
fn bluez_gone_answers_what_was_under_way_and_takes_discovery_up_again() {
    let mut s = Session::new();
    s.ask(SETTINGS, r#"{"bluetoothDiscover":{"on":true}}"#, false);
    s.until(|s| s.discovering() == Some(true));
    let asked = s.ask(
        SETTINGS,
        r#"{"bluetoothConnect":{"id":"3C:28:6D:01:02:03"}}"#,
        true,
    );
    s.sent(|c| matches!(c, Command::Connect { .. }));
    let (question, _answered) = Question::new(
        11,
        Asked::Confirm(1),
        "3C:28:6D:01:02:03".into(),
        "Pixel 8".into(),
        Kind::Phone,
    );
    s.played.give(Event::Question(question));
    let (sheet, _) = s.wait_sheet(0);
    // BlueZ stops: the connection is answered, its question withdrawn.
    s.played.give(Event::State(State::unavailable()));
    s.until(|s| s.answer(SETTINGS, asked).is_some());
    assert_eq!(
        s.answer(SETTINGS, asked).unwrap(),
        r#"{"failed":{"reason":"unavailable"}}"#
    );
    assert!(s.dismissed(sheet));
    // Back: discovery, still held, is asked again.
    let before = s
        .played
        .sent()
        .iter()
        .filter(|c| **c == Command::Discover(true))
        .count();
    s.played.give(Event::State(machine()));
    s.until(|s| {
        s.played
            .sent()
            .iter()
            .filter(|c| **c == Command::Discover(true))
            .count()
            > before
    });
}
