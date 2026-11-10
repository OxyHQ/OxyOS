//! Sleep, restart, shut down and log out (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx),
//! over a logind played on a bus of the test's own (never the machine's): the question and its
//! deadline, the applications asked and any one cancelling, logind asked first and only when it
//! says yes, the power key by logind's policy, the login screen's buttons, and reopening at login.

use super::*;
use oxyos_compositor_core::{Grants, Scene};
use oxyos_power::played::{Machine, Played};
use oxyos_preferences::Preferences;
use oxyos_protocol::{
    Clock, Event, PanelEdge, PointerKind, PreeditText, Request, ShellKind, Size,
    QUIT_REQUESTS_VERSION,
};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

const SHELL: ClientId = 1;
const APP: ClientId = 2;
const OLD: ClientId = 3;

struct Bus(Child);

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("oxyos-session-end-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn private_bus(directory: &Path) -> (String, Bus) {
    let mut bus = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .arg(format!(
            "--address=unix:path={}",
            directory.join("bus").display()
        ))
        .stdout(Stdio::piped())
        .spawn()
        .expect("dbus-daemon (the dbus-daemon package)");
    let mut address = String::new();
    BufReader::new(bus.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    (address.trim().to_string(), Bus(bus))
}

struct Session {
    compositor: Compositor,
    settings: Settings,
    part: SessionEnding,
    played: Played,
    reopen: PathBuf,
    told: Vec<(ClientId, Event)>,
    logged_out: bool,
    _bus: Bus,
}

fn name_of(client: ClientId) -> String {
    match client {
        APP => "Pages".into(),
        OLD => "Notes".into(),
        _ => String::new(),
    }
}

const RUNNING: [&str; 3] = ["so.oxy.pages", "so.oxy.notes", "so.oxy.pages"];

impl Session {
    fn new(name: &str, machine: serde_json::Value, greeter: bool) -> Self {
        let directory = scratch(name);
        let (address, bus) = private_bus(&directory);
        let machine: Machine = serde_json::from_value(machine).unwrap();
        let played = Played::serve(&address, machine).unwrap();
        let mut scene = Scene::new();
        scene
            .add_output(oxyos_compositor_core::Output {
                id: 1,
                rect: oxyos_protocol::Rect::new(0, 0, 1920, 1080),
                scale_120: 120,
                refresh_mhz: 60_000,
            })
            .unwrap();
        scene
            .connect(
                SHELL,
                Grants {
                    shell: true,
                    lock: greeter,
                    ..Grants::default()
                },
            )
            .unwrap();
        if !greeter {
            for (client, version) in [
                (APP, QUIT_REQUESTS_VERSION),
                (OLD, QUIT_REQUESTS_VERSION - 1),
            ] {
                scene.connect(client, Grants::default()).unwrap();
                scene.set_version(client, version);
            }
        }
        let reopen = directory.join("state/oxyos/reopen-at-login");
        let mut session = Self {
            compositor: Compositor::new(scene, 0),
            settings: Settings::new(Preferences::default(), None, None),
            part: SessionEnding::inline(Logind::Bus(address), greeter, Some(reopen.clone())),
            played,
            reopen,
            told: Vec::new(),
            logged_out: false,
            _bus: bus,
        };
        if greeter {
            session.compositor.login = true;
            session.compositor.scene.lock();
            session.surface(Request::CreateLockSurface {
                surface: 50,
                output: 1,
            });
        } else {
            session.surface(Request::CreateShellSurface {
                surface: 50,
                kind: ShellKind::Panel,
                edge: PanelEdge::Bottom,
                output: 1,
                size: Size::new(1, 48),
                exclusive: 48,
            });
        }
        session
    }

    fn pump(&mut self) {
        let events = self.compositor.scene.drain_events();
        self.told.extend(events);
    }

    fn request(&mut self, client: ClientId, request: Request) {
        self.compositor.scene.handle(client, request).unwrap();
        self.pump();
    }

    /// The shell's dock (the login screen's lock surface): what the person presses on.
    fn surface(&mut self, create: Request) {
        self.request(SHELL, create);
        let (serial, rect) = self
            .told
            .iter()
            .find_map(|(_, event)| match event {
                Event::Configure {
                    surface: 50,
                    serial,
                    rect,
                    ..
                } => Some((*serial, *rect)),
                _ => None,
            })
            .expect("configured");
        let extent = Size::new(rect.width, rect.height);
        self.request(
            SHELL,
            Request::AttachBuffer {
                buffer: 1,
                format: oxyos_protocol::Format::Xrgb8888,
                extent,
                stride: extent.width * 4,
                offset: 0,
            },
        );
        self.request(
            SHELL,
            Request::Submit {
                surface: 50,
                sequence: 1,
                buffer: 1,
                configure: serial,
                extent,
            },
        );
        self.compositor.scene.begin_frame();
        self.compositor.scene.end_frame(1, Clock::Synthetic);
        self.pump();
        self.told.clear();
    }

    /// A press on the shell's dock: an input the shell received.
    fn press(&mut self) -> u64 {
        self.compositor.scene.pointer_motion(960, 1060);
        self.compositor.scene.pointer_button(272, true);
        self.pump();
        let serial = self
            .told
            .iter()
            .rev()
            .find_map(|(_, event)| match event {
                Event::Pointer {
                    kind: PointerKind::Press,
                    serial,
                    ..
                } => Some(*serial),
                _ => None,
            })
            .expect("pressed");
        self.compositor.scene.pointer_button(272, false);
        self.pump();
        serial
    }

    fn turn(&mut self) {
        let running: Vec<String> = RUNNING.iter().map(|id| id.to_string()).collect();
        let served = self
            .part
            .serve(&mut self.compositor, &mut self.settings, &name_of, &running);
        self.logged_out |= served.log_out;
        self.pump();
    }

    /// The shell asks, citing a press (or no input), and a turn answers: its answer.
    fn ask(&mut self, client: ClientId, serial: u32, text: &str, with_input: bool) -> String {
        let input = if with_input { self.press() } else { 0 };
        let chunks = oxyos_protocol::TextChunk::split(text);
        let count = chunks.len();
        for (index, chunk) in chunks.into_iter().enumerate() {
            self.request(
                client,
                Request::SystemRequest {
                    serial,
                    input,
                    index: index as u16,
                    last: index + 1 == count,
                    text: PreeditText::new(chunk.as_str()).unwrap(),
                },
            );
        }
        self.turn();
        self.answer(client, serial)
    }

    fn answer(&mut self, client: ClientId, serial: u32) -> String {
        let mut text = String::new();
        self.told.retain(|(to, event)| match event {
            Event::SystemText {
                serial: told,
                text: chunk,
                ..
            } if *to == client && *told == serial => {
                text.push_str(chunk.as_str());
                false
            }
            _ => true,
        });
        text
    }

    /// The sheets shown since the last call, each its number and text.
    fn sheets(&mut self) -> Vec<(u32, serde_json::Value)> {
        let mut sheets = Vec::new();
        let mut text = String::new();
        for (to, event) in std::mem::take(&mut self.told) {
            if let Event::SheetText {
                sheet,
                text: chunk,
                last,
                ..
            } = event
            {
                assert_eq!(to, SHELL);
                text.push_str(chunk.as_str());
                if last {
                    sheets.push((sheet, serde_json::from_str(&text).unwrap()));
                    text.clear();
                }
            } else {
                self.told.push((to, event));
            }
        }
        sheets
    }

    fn dismissed(&mut self) -> Vec<u32> {
        self.told
            .iter()
            .filter_map(|(_, event)| match event {
                Event::SheetDismissed { sheet } => Some(*sheet),
                _ => None,
            })
            .collect()
    }

    /// The person answers the sheet `sheet` in the shell.
    fn sheet_answer(&mut self, sheet: u32, accepted: bool) {
        let serial = self.press();
        self.request(
            SHELL,
            Request::SheetAnswer {
                sheet,
                accepted,
                serial,
            },
        );
        self.turn();
    }

    /// What each client was asked: `QuitRequested` and `CloseRequested`.
    fn asked(&mut self) -> Vec<(ClientId, Event)> {
        self.told
            .iter()
            .filter(|(_, event)| {
                matches!(
                    event,
                    Event::QuitRequested { .. } | Event::CloseRequested { .. }
                )
            })
            .copied()
            .collect()
    }

    fn quit_serial(&mut self) -> u32 {
        self.asked()
            .into_iter()
            .find_map(|(to, event)| match event {
                Event::QuitRequested { serial, .. } if to == APP => Some(serial),
                _ => None,
            })
            .expect("asked to quit")
    }

    fn logind(&self) -> serde_json::Value {
        self.played.describe()
    }
}

fn yes() -> serde_json::Value {
    json!({})
}

#[test]
fn logind_never_turns_the_machine_off_at_a_press_while_the_session_runs() {
    let session = Session::new("inhibitor", yes(), false);
    assert_eq!(session.logind()["inhibitors"], json!(["handle-power-key"]));
    let greeter = Session::new("inhibitor-greeter", yes(), true);
    assert_eq!(greeter.logind()["inhibitors"], json!(["handle-power-key"]));
}

#[test]
fn restart_is_asked_then_every_application_and_logind_once_all_have_gone() {
    let mut s = Session::new("restart", yes(), false);
    let answer = s.ask(
        SHELL,
        1,
        r#"{"endSession":{"action":"restart","confirm":true}}"#,
        true,
    );
    assert_eq!(answer, r#"{"confirming":{}}"#);
    let sheets = s.sheets();
    assert_eq!(
        sheets[..],
        [(
            sheets[0].0,
            json!({"kind": "end-session", "action": "restart", "seconds": 60, "reopenWindows": true})
        )]
    );
    // Nothing is asked of anyone while the question is shown.
    assert!(s.asked().is_empty());
    assert_eq!(s.logind()["calls"], json!([]));
    s.sheet_answer(sheets[0].0, true);
    // The new application is asked; the older one's windows are asked to close (it has none).
    assert_eq!(
        s.asked(),
        [(
            APP,
            Event::QuitRequested {
                serial: 1,
                reason: Reason::Restart
            }
        )]
    );
    assert_eq!(s.part.state_name(), "quitting");
    // The applications running, written for the next login, each once.
    assert_eq!(
        std::fs::read_to_string(&s.reopen).unwrap(),
        "so.oxy.pages\nso.oxy.notes\n"
    );
    // Answered quit, but still there: logind is not asked yet.
    s.request(
        APP,
        Request::QuitReply {
            serial: 1,
            quit: true,
        },
    );
    s.turn();
    assert_eq!(s.logind()["calls"], json!([]));
    s.compositor.scene.disconnect(APP);
    s.turn();
    assert_eq!(s.logind()["calls"], json!(["Reboot false"]));
    assert_eq!(s.logind()["reboots"], 1);
    assert_eq!(s.part.state_name(), "ended");
    assert!(!s.logged_out);
    // Nothing more is taken.
    let answer = s.ask(SHELL, 2, r#"{"sleep":{}}"#, true);
    assert_eq!(answer, r#"{"refused":{"reason":"busy"}}"#);
    assert_eq!(s.logind()["calls"], json!(["Reboot false"]));
}

#[test]
fn the_question_goes_on_by_itself_at_sixty_seconds_and_one_application_cancels_it() {
    let mut s = Session::new("deadline", yes(), false);
    s.ask(
        SHELL,
        1,
        r#"{"endSession":{"action":"shutDown","confirm":true}}"#,
        true,
    );
    let sheet = s.sheets()[0].0;
    assert_eq!(s.part.wait().map(|wait| wait.as_secs() >= 59), Some(true));
    s.part.pass(Duration::from_secs(59));
    s.turn();
    assert!(s.asked().is_empty(), "not before its time");
    s.part.pass(Duration::from_secs(1));
    s.turn();
    // Withdrawn by the session, and the applications asked as if accepted.
    assert_eq!(s.dismissed(), [sheet]);
    let serial = s.quit_serial();
    assert!(s.reopen.exists());
    s.request(
        APP,
        Request::QuitReply {
            serial,
            quit: false,
        },
    );
    s.turn();
    assert_eq!(
        s.sheets()
            .into_iter()
            .map(|(_, text)| text)
            .collect::<Vec<_>>(),
        [json!({"kind": "end-session-cancelled", "action": "shutDown", "application": "Pages"})]
    );
    assert_eq!(s.logind()["calls"], json!([]));
    assert_eq!(s.part.state_name(), "idle");
    // Nothing reopens of an end that did not happen, and a late answer is nobody's.
    assert!(!s.reopen.exists());
    s.request(APP, Request::QuitReply { serial, quit: true });
    s.turn();
    assert_eq!(s.logind()["calls"], json!([]));
}

#[test]
fn cancel_ends_the_question_and_a_new_choice_replaces_it() {
    let mut s = Session::new("cancel", yes(), false);
    s.ask(
        SHELL,
        1,
        r#"{"endSession":{"action":"logOut","confirm":true}}"#,
        true,
    );
    let first = s.sheets()[0].0;
    // Restart chosen while Log Out is asked: the question is Restart's now.
    s.ask(
        SHELL,
        2,
        r#"{"endSession":{"action":"restart","confirm":true}}"#,
        true,
    );
    assert_eq!(s.dismissed(), [first]);
    let second = s.sheets();
    assert_eq!(second[0].1["action"], "restart");
    s.sheet_answer(second[0].0, false);
    assert_eq!(s.part.state_name(), "idle");
    assert_eq!(s.part.wait(), None);
    s.part.pass(Duration::from_secs(120));
    s.turn();
    assert!(s.asked().is_empty());
    assert_eq!(s.logind()["calls"], json!([]));
}

#[test]
fn an_application_that_does_not_answer_in_time_is_named() {
    let mut s = Session::new("timeout", yes(), false);
    let answer = s.ask(
        SHELL,
        1,
        r#"{"endSession":{"action":"logOut","confirm":false}}"#,
        true,
    );
    assert_eq!(answer, r#"{"ending":{"action":"logOut"}}"#);
    s.quit_serial();
    s.part.pass(QUIT_DEADLINE - Duration::from_secs(1));
    s.turn();
    assert!(s.sheets().is_empty());
    s.part.pass(Duration::from_secs(1));
    s.turn();
    assert_eq!(
        s.sheets()[0].1,
        json!({"kind": "end-session-timed-out", "action": "logOut", "application": "Pages"})
    );
    assert!(!s.logged_out);
}

#[test]
fn log_out_ends_the_session_once_every_application_has_quit() {
    let mut s = Session::new("log-out", yes(), false);
    s.settings
        .set_preference(&mut s.compositor, "reopen_windows", "false", 0)
        .unwrap();
    std::fs::create_dir_all(s.reopen.parent().unwrap()).unwrap();
    std::fs::write(&s.reopen, "so.oxy.stale\n").unwrap();
    // Super+Alt+Shift+Q: at once.
    for key in [125, 56, 42, 16] {
        s.compositor.scene.key(key, true);
    }
    s.turn();
    let serial = s.quit_serial();
    // Unchecked: nothing is reopened, not even what an earlier end left.
    assert!(!s.reopen.exists());
    s.request(APP, Request::QuitReply { serial, quit: true });
    s.compositor.scene.disconnect(APP);
    s.turn();
    assert!(s.logged_out);
    assert_eq!(s.part.state_name(), "ended");
    assert_eq!(s.logind()["calls"], json!([]));
    // logind sees the session let the power key go as it ends: the session closes its descriptor
    // at once, but the bus daemon and the played logind's connection let their copies of it go
    // on their own threads, after the reply was delivered (a loaded machine sees them late).
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.logind()["inhibitors"] != json!([]) {
        assert!(
            Instant::now() < deadline,
            "the power key still held after five seconds"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn logind_saying_no_refuses_before_any_application_is_asked() {
    for can in ["challenge", "no", "na"] {
        let mut s = Session::new(
            &format!("refused-{can}"),
            json!({"can_reboot": can, "can_power_off": can}),
            false,
        );
        let answer = s.ask(
            SHELL,
            1,
            r#"{"endSession":{"action":"restart","confirm":true}}"#,
            true,
        );
        let reason = if can == "na" {
            "unsupported"
        } else {
            "notPermitted"
        };
        assert_eq!(answer, format!(r#"{{"refused":{{"reason":"{reason}"}}}}"#));
        assert_eq!(
            s.sheets()[0].1,
            json!({"kind": "end-session-failed", "action": "restart", "reason": reason})
        );
        assert!(s.asked().is_empty());
        assert_eq!(s.logind()["calls"], json!([]));
        assert_eq!(s.part.state_name(), "idle");
    }
}

#[test]
fn only_the_shell_asks_citing_the_persons_input_and_never_while_locked() {
    let mut s = Session::new("asking", yes(), false);
    let shut_down = r#"{"endSession":{"action":"shutDown","confirm":false}}"#;
    assert_eq!(
        s.ask(SHELL, 1, shut_down, false),
        r#"{"refused":{"reason":"needsInput"}}"#
    );
    // An application's request is refused by the scene before this part sees it.
    assert_eq!(
        s.ask(APP, 2, shut_down, true),
        r#"{"refused":{"reason":"notPermitted"}}"#
    );
    assert_eq!(
        s.ask(
            SHELL,
            3,
            r#"{"endSession":{"action":"reboot","confirm":false}}"#,
            true
        ),
        r#"{"refused":{"reason":"invalidRequest"}}"#
    );
    s.compositor.scene.lock();
    assert_eq!(
        s.ask(SHELL, 4, shut_down, true),
        r#"{"refused":{"reason":"locked"}}"#
    );
    assert!(s.asked().is_empty());
    assert_eq!(s.logind()["calls"], json!([]));
}

#[test]
fn sleep_and_the_reopen_checkbox() {
    let mut s = Session::new("sleep", yes(), false);
    assert_eq!(
        s.ask(SHELL, 1, r#"{"sleep":{}}"#, true),
        r#"{"sleeping":{}}"#
    );
    assert_eq!(s.logind()["calls"], json!(["Suspend false"]));
    assert_eq!(
        s.ask(SHELL, 2, r#"{"reopenWindows":{"enabled":false}}"#, true),
        r#"{"reopenWindows":{"enabled":false}}"#
    );
    assert!(!s.settings.preferences().reopen_windows);
    let mut refused = Session::new("sleep-refused", json!({"deny": ["suspend"]}), false);
    assert_eq!(
        refused.ask(SHELL, 1, r#"{"sleep":{}}"#, true),
        r#"{"refused":{"reason":"notPermitted"}}"#
    );
    assert_eq!(
        refused.sheets()[0].1,
        json!({"kind": "end-session-failed", "action": "sleep", "reason": "notPermitted"})
    );
}

#[test]
fn the_power_key_asks_restart_sleep_or_shut_down_never_powering_off_at_once() {
    let mut s = Session::new("power-key", yes(), false);
    s.compositor.scene.key(KEY_POWER_CODE, true);
    s.compositor.scene.key(KEY_POWER_CODE, false);
    s.turn();
    let sheets = s.sheets();
    assert_eq!(
        sheets[..],
        [(
            sheets[0].0,
            json!({"kind": "power-key", "reopenWindows": true})
        )]
    );
    assert_eq!(s.logind()["calls"], json!([]));
    // Another press while it is shown does nothing more.
    s.compositor.scene.key(KEY_POWER_CODE, true);
    s.turn();
    assert!(s.sheets().is_empty());
    // Its Sleep: the sheet goes, and the machine sleeps.
    let choice = format!(
        r#"{{"powerKeyChoice":{{"sheet":{},"choice":"sleep"}}}}"#,
        sheets[0].0
    );
    assert_eq!(s.ask(SHELL, 1, &choice, true), r#"{"sleeping":{}}"#);
    assert_eq!(s.dismissed(), [sheets[0].0]);
    assert_eq!(s.logind()["calls"], json!(["Suspend false"]));
    // Again, and its Shut Down: the applications asked with no further question.
    s.told.clear();
    s.compositor.scene.key(KEY_POWER_CODE, true);
    s.turn();
    let sheet = s.sheets()[0].0;
    s.sheet_answer(sheet, true);
    assert_eq!(
        s.asked(),
        [(
            APP,
            Event::QuitRequested {
                serial: 1,
                reason: Reason::ShutDown
            }
        )]
    );
    // A stale choice of a sheet no longer shown is refused.
    assert_eq!(
        s.ask(SHELL, 2, &choice, true),
        r#"{"refused":{"reason":"invalidRequest"}}"#
    );
}

/// `KEY_POWER`.
const KEY_POWER_CODE: u32 = 116;

#[test]
fn the_power_key_follows_loginds_policy_and_sleeps_a_locked_session() {
    let mut locked = Session::new("power-key-locked", yes(), false);
    locked.compositor.scene.lock();
    locked.compositor.scene.key(KEY_POWER_CODE, true);
    locked.turn();
    assert!(locked.sheets().is_empty());
    assert_eq!(locked.logind()["calls"], json!(["Suspend false"]));
    for (policy, calls, locks) in [
        ("suspend", json!(["Suspend false"]), false),
        ("ignore", json!([]), false),
        ("lock", json!([]), true),
    ] {
        let mut s = Session::new(
            &format!("power-key-{policy}"),
            json!({"handle_power_key": policy}),
            false,
        );
        s.compositor.scene.key(KEY_POWER_CODE, true);
        s.turn();
        assert!(s.sheets().is_empty(), "{policy}");
        assert_eq!(s.logind()["calls"], calls, "{policy}");
        assert_eq!(s.compositor.scene.is_locked(), locks, "{policy}");
    }
}

#[test]
fn the_login_screen_acts_at_once_never_logs_out_and_asks_itself_at_the_power_key() {
    let mut s = Session::new("greeter", yes(), true);
    assert_eq!(
        s.ask(
            SHELL,
            1,
            r#"{"endSession":{"action":"logOut","confirm":false}}"#,
            true
        ),
        r#"{"refused":{"reason":"notPermitted"}}"#
    );
    assert_eq!(
        s.ask(SHELL, 2, r#"{"reopenWindows":{"enabled":false}}"#, true),
        r#"{"refused":{"reason":"notPermitted"}}"#
    );
    // The power key: told to the login screen unasked, which shows its own choice.
    s.compositor.scene.key(KEY_POWER_CODE, true);
    s.turn();
    assert_eq!(s.answer(SHELL, 0), r#"{"powerKey":{}}"#);
    assert!(s.sheets().is_empty());
    // Shut Down: nobody is signed in, so nothing is asked or confirmed.
    assert_eq!(
        s.ask(
            SHELL,
            3,
            r#"{"endSession":{"action":"shutDown","confirm":true}}"#,
            true
        ),
        r#"{"ending":{"action":"shutDown"}}"#
    );
    assert!(s.sheets().is_empty());
    assert_eq!(s.logind()["calls"], json!(["PowerOff false"]));
    assert_eq!(s.logind()["power_offs"], 1);
    assert!(!s.reopen.exists());
}

#[test]
fn the_applications_to_reopen_are_read_once_each_once() {
    let directory = scratch("take-reopen");
    let path = directory.join("reopen-at-login");
    assert!(take_reopen(&path).is_empty());
    write_reopen(&path, &["a".to_string(), "b".to_string(), "a".to_string()]).unwrap();
    std::fs::write(&path, "a\n\n b \na\n").unwrap();
    assert_eq!(take_reopen(&path), ["a", "b"]);
    assert!(!path.exists());
    assert!(take_reopen(&path).is_empty());
}
