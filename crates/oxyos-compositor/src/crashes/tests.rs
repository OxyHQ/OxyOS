//! Applications that quit unexpectedly and resetting one (docs/adr/0061): what counts, what is
//! written and kept, what the person is told and when, who may reset, and what a reset erases.

use super::*;
use oxyos_apps::{Grants as AppGrants, Launched, Runtime};
use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::{Grants, Output};
use oxyos_protocol::{
    Clock, Event, PanelEdge, PointerKind, PreeditText, Rect, Request, ShellKind, Size,
};
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;

const SHELL: ClientId = 1;
const SETTINGS: ClientId = 2;
const APP: ClientId = 3;
/// A client that asks the session for something else (the store).
const STORE: ClientId = 4;
const MENTION: &str = "so.oxy.mention";

/// A folder of the test's own, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("oxyos-crashes-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A scene with a display, the shell (with a panel to press on), Settings and an application.
struct Session {
    scene: Scene,
    told: Vec<(ClientId, Event)>,
    next_serial: u32,
}

impl Session {
    fn new() -> Self {
        let mut scene = Scene::new();
        scene
            .add_output(Output {
                id: 1,
                rect: Rect::new(0, 0, 1920, 1080),
                scale_120: 120,
                refresh_mhz: 60_000,
            })
            .unwrap();
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
                    settings: true,
                    ..Grants::default()
                },
            ),
            (APP, Grants::default()),
            (
                STORE,
                Grants {
                    store: true,
                    ..Grants::default()
                },
            ),
        ] {
            scene.connect(client, grants).unwrap();
        }
        let mut session = Self {
            scene,
            told: Vec::new(),
            next_serial: 0,
        };
        // The shell's dock: a panel the person presses on.
        session.request(
            SHELL,
            Request::CreateShellSurface {
                surface: 50,
                kind: ShellKind::Panel,
                edge: PanelEdge::Bottom,
                output: 1,
                size: Size::new(1, 48),
                exclusive: 48,
            },
        );
        let (serial, rect) = session
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
        session.request(
            SHELL,
            Request::AttachBuffer {
                buffer: 1,
                format: oxyos_protocol::Format::Xrgb8888,
                extent,
                stride: extent.width * 4,
                offset: 0,
            },
        );
        session.request(
            SHELL,
            Request::Submit {
                surface: 50,
                sequence: 1,
                buffer: 1,
                configure: serial,
                extent,
            },
        );
        session.scene.begin_frame();
        session.scene.end_frame(1, Clock::Synthetic);
        session.pump();
        session
    }

    fn pump(&mut self) {
        let events = self.scene.drain_events();
        self.told.extend(events);
    }

    fn request(&mut self, client: ClientId, request: Request) {
        self.scene.handle(client, request).unwrap();
        self.pump();
    }

    /// A press on the shell's dock: an input the shell received.
    fn shell_press(&mut self) -> u64 {
        self.told.clear();
        self.scene.pointer_motion(960, 1060);
        self.scene.pointer_button(272, true);
        self.pump();
        let serial = self
            .told
            .iter()
            .find_map(|(_, event)| match event {
                Event::Pointer {
                    kind: PointerKind::Press,
                    serial,
                    ..
                } => Some(*serial),
                _ => None,
            })
            .expect("pressed");
        self.scene.pointer_button(272, false);
        self.pump();
        serial
    }

    /// The person answers the sheet `sheet` in the shell.
    fn answer(&mut self, sheet: u32, accepted: bool) {
        let serial = if accepted { self.shell_press() } else { 0 };
        self.request(
            SHELL,
            Request::SheetAnswer {
                sheet,
                accepted,
                serial,
            },
        );
    }

    /// The person presses the notification `number` (action 0) or one of its actions.
    fn activate(&mut self, number: u32, action: u8) {
        let serial = self.shell_press();
        self.request(
            SHELL,
            Request::ActivateNotification {
                notification: number,
                action,
                serial,
            },
        );
    }

    /// `client`'s request, citing an input or not.
    fn ask(&mut self, client: ClientId, text: &str, with_input: bool) -> SystemRequest {
        self.next_serial += 1;
        SystemRequest {
            client,
            serial: self.next_serial,
            with_input,
            text: text.into(),
        }
    }

    /// The sheets the shell was sent since the log was last cleared, whole: by number, their JSON.
    fn sheets(&self) -> Vec<(u32, Value)> {
        let mut texts: BTreeMap<u32, String> = BTreeMap::new();
        let mut whole = Vec::new();
        for (client, event) in &self.told {
            if let (
                SHELL,
                Event::SheetText {
                    sheet, text, last, ..
                },
            ) = (*client, event)
            {
                texts.entry(*sheet).or_default().push_str(text.as_str());
                if *last {
                    let text = texts.remove(sheet).unwrap();
                    whole.push((*sheet, serde_json::from_str(&text).unwrap()));
                }
            }
        }
        whole
    }

    /// The answers `client` was told, by serial, as JSON.
    fn answers(&self, client: ClientId) -> Vec<(u32, Value)> {
        let mut texts: BTreeMap<u32, String> = BTreeMap::new();
        let mut whole = Vec::new();
        for (to, event) in &self.told {
            if let Event::SystemText {
                serial, text, last, ..
            } = event
            {
                if *to != client {
                    continue;
                }
                texts.entry(*serial).or_default().push_str(text.as_str());
                if *last {
                    let text = texts.remove(serial).unwrap();
                    whole.push((*serial, serde_json::from_str(&text).unwrap()));
                }
            }
        }
        whole
    }

    /// The notifications the shell was told, whole: their number, application, title and actions'
    /// labels.
    fn notifications(&self) -> Vec<(u32, String, String, Vec<String>)> {
        use oxyos_protocol::NotificationField;
        let mut texts: BTreeMap<(u32, u8, u8), String> = BTreeMap::new();
        let mut whole = Vec::new();
        for (to, event) in &self.told {
            if *to != SHELL {
                continue;
            }
            match event {
                Event::NotificationText {
                    notification,
                    field,
                    action,
                    text,
                    ..
                } => texts
                    .entry((*notification, *field as u8, *action))
                    .or_default()
                    .push_str(text.as_str()),
                Event::Notification {
                    notification,
                    actions,
                    ..
                } => {
                    let text = |field: NotificationField, action: u8| {
                        texts
                            .get(&(*notification, field as u8, action))
                            .cloned()
                            .unwrap_or_default()
                    };
                    whole.push((
                        *notification,
                        text(NotificationField::Application, 0),
                        text(NotificationField::Title, 0),
                        (1..=*actions)
                            .map(|n| text(NotificationField::ActionLabel, n))
                            .collect(),
                    ));
                }
                _ => {}
            }
        }
        whole
    }
}

fn launched(id: &str, name: &str) -> Launched {
    Launched {
        id: id.into(),
        name: name.into(),
        grants: AppGrants::default(),
        origin: Origin::Installed { version_code: 12 },
        capabilities: BTreeMap::new(),
        opened: None,
        chooser: false,
        url: None,
    }
}

/// An exit of `id`: by `status` (a raw wait status), asked to or not, saying `report`.
fn exit(id: &str, status: i32, asked: bool, report: Option<&str>) -> Exited {
    Exited {
        pid: 4242,
        launched: launched(id, "Mention"),
        status: Some(std::process::ExitStatus::from_raw(status)),
        ran: Duration::from_secs(13),
        asked,
        report: report.map(str::to_string),
    }
}

/// A signal's raw wait status, and an exit status's.
const SEGV: i32 = 11;
fn exit_code(code: i32) -> i32 {
    code << 8
}

fn utc(seconds: i64) -> jiff::Zoned {
    jiff::Timestamp::from_second(seconds)
        .unwrap()
        .to_zoned(jiff::tz::TimeZone::UTC)
}

#[test]
fn quits_count_within_ten_minutes() {
    let mut counter = Counter::default();
    let start = Instant::now();
    assert_eq!(counter.quit(MENTION, start), 1);
    assert_eq!(counter.quit(MENTION, start + Duration::from_secs(300)), 2);
    assert_eq!(counter.quit("so.oxy.allo", start), 1, "each its own");
    // The first is ten minutes old: it no longer counts.
    assert_eq!(counter.quit(MENTION, start + REPEATED), 2);
    assert_eq!(counter.quit(MENTION, start + REPEATED * 3), 1);
    counter.forget(MENTION);
    assert_eq!(counter.quit(MENTION, start + REPEATED * 3), 1);
}

#[test]
fn a_report_says_what_quit_how_and_what_it_said() {
    let text = report_text(&Report {
        id: MENTION,
        name: "Mention",
        origin: &Origin::Installed { version_code: 12 },
        ended: utc(1_794_133_953),
        ran: Duration::from_secs(242),
        status: Some(std::process::ExitStatus::from_raw(SEGV)),
        times: 2,
        system: Some("OxyOS, Linux 6.12.0"),
        said: Some("[host] fatal signal 11 (code 1) at address 0x8"),
        hang: None,
    });
    assert_eq!(
        text,
        "Mention quit unexpectedly.\n\n\
         Application:  Mention\n\
         Identifier:   so.oxy.mention\n\
         Kind:         installed, version 12\n\
         Ended:        2026-11-08 10:32:33 +00:00\n\
         Running for:  4 minutes 2 seconds\n\
         Reason:       signal 11 (SIGSEGV)\n\
         Repeated:     2 times in the last 10 minutes\n\
         System:       OxyOS, Linux 6.12.0\n\
         \nWhat it reported:\n[host] fatal signal 11 (code 1) at address 0x8\n"
    );
    let quiet = report_text(&Report {
        id: MENTION,
        name: "Mention",
        origin: &Origin::System,
        ended: utc(0),
        ran: Duration::from_secs(1),
        status: Some(std::process::ExitStatus::from_raw(exit_code(70))),
        times: 1,
        system: None,
        said: None,
        hang: None,
    });
    assert!(quiet.contains("Kind:         a system application\n"));
    assert!(quiet.contains("Running for:  1 second\n"));
    assert!(quiet.contains("Reason:       exit status 70\n"));
    assert!(!quiet.contains("Repeated") && !quiet.contains("System"));
    assert!(quiet.ends_with("\nIt reported nothing of its end.\n"));
    // A hang the person force-quit (docs/adr/0067): said so, with how long it did not answer and
    // the stack the host wrote.
    let hang = report_text(&Report {
        id: MENTION,
        name: "Mention",
        origin: &Origin::System,
        ended: utc(0),
        ran: Duration::from_secs(70),
        status: Some(std::process::ExitStatus::from_raw(9)),
        times: 1,
        system: None,
        said: Some("[host] hang: asked by process 1 (SIGQUIT); the JavaScript thread's stack:"),
        hang: Some(Hang {
            unanswered: Duration::from_secs(7),
            by_the_person: true,
        }),
    });
    assert!(hang.starts_with("Mention stopped responding and was force quit.\n"));
    assert!(
        hang.contains("Reason:       hang: no response for 7 seconds; force quit by the person\n")
    );
    assert!(hang.contains("the JavaScript thread's stack"));
    // The shell the session ended for not answering (decision 6, its watch): started again.
    let shell = report_text(&Report {
        id: "so.oxy.shell",
        name: "OxyOS",
        origin: &Origin::System,
        ended: utc(0),
        ran: Duration::from_secs(70),
        status: Some(std::process::ExitStatus::from_raw(9)),
        times: 1,
        system: None,
        said: None,
        hang: Some(Hang {
            unanswered: Duration::from_secs(8),
            by_the_person: false,
        }),
    });
    assert!(shell.starts_with("OxyOS stopped responding and was started again.\n"));
    assert!(shell.contains("Reason:       hang: no response for 8 seconds; ended by the session\n"));
    assert_eq!(reason(None), "it could not be waited for");
    assert_eq!(
        reason(Some(std::process::ExitStatus::from_raw(64))),
        "signal 64"
    );
}

#[test]
fn reports_are_the_persons_alone_and_bounded() {
    let scratch = Scratch::new("reports");
    let reports = Reports::new(scratch.0.join("state/oxyos/crash-reports"));
    let first = reports.write(MENTION, &utc(1_000), "one").unwrap();
    assert_eq!(
        first.file_name().unwrap(),
        "so.oxy.mention_1970-01-01-001640.crash"
    );
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "one");
    assert_eq!(
        std::fs::metadata(&first).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(reports.directory())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    // Another at the same second keeps the first.
    let second = reports.write(MENTION, &utc(1_000), "two").unwrap();
    assert_eq!(
        second.file_name().unwrap(),
        "so.oxy.mention_1970-01-01-001640-2.crash"
    );
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "one");
    // At most ten of one application's: the oldest go.
    for n in 0..MAX_REPORTS_PER_APPLICATION as i64 {
        reports.write(MENTION, &utc(2_000 + n), "more").unwrap();
    }
    let kept: Vec<_> = reports
        .list()
        .into_iter()
        .filter(|(_, id)| id == MENTION)
        .collect();
    assert_eq!(kept.len(), MAX_REPORTS_PER_APPLICATION);
    assert!(!first.exists() && !second.exists());
    // An id with an underscore is still its own.
    reports.write("so.oxy.my_app", &utc(3_000), "x").unwrap();
    assert!(reports.list().iter().any(|(_, id)| id == "so.oxy.my_app"));
    // At most fifty in all: the oldest go first, whichever application's.
    for n in 0..60 {
        reports
            .write(&format!("so.oxy.app{n}"), &utc(4_000 + n), "x")
            .unwrap();
    }
    let all = reports.list();
    assert_eq!(all.len(), MAX_REPORTS);
    assert!(all.iter().all(|(_, id)| id.starts_with("so.oxy.app")));
}

#[test]
fn reports_written_within_one_tick_of_the_clock_go_oldest_first() {
    // The file system's timestamps are the kernel's coarse clock: on a fast machine many reports
    // are written within one tick, and the order is then their names', never the ids'.
    let scratch = Scratch::new("tick");
    let reports = Reports::new(scratch.0.join("reports"));
    let mut written = vec![
        reports.write("so.oxy.zebra", &utc(1_000), "z").unwrap(),
        reports.write("so.oxy.zebra", &utc(1_000), "z").unwrap(),
    ];
    for n in 0..(MAX_REPORTS - 2) as i64 {
        written.push(
            reports
                .write(&format!("so.oxy.a{n}"), &utc(2_000 + n), "a")
                .unwrap(),
        );
    }
    let tick = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    for path in &written {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(tick).unwrap();
    }
    let listed: Vec<_> = reports.list().into_iter().map(|(path, _)| path).collect();
    assert_eq!(listed, written);
    // One more: the first written goes, and then the second.
    reports.write("so.oxy.b", &utc(9_000), "b").unwrap();
    assert!(!written[0].exists() && written[1].exists());
}

#[test]
fn the_first_quit_is_a_notification_and_the_next_the_sheet_once() {
    let scratch = Scratch::new("notice");
    let mut session = Session::new();
    let reports = Reports::new(scratch.0.join("reports"));
    let mut crashes = Crashes::new(Some(reports.clone()), Some("OxyOS".into()));
    let start = Instant::now();
    // What the session asked to end, and what ended cleanly, are not unexpected.
    crashes.exited_at(
        &mut session.scene,
        &[
            exit(MENTION, 15, true, None),
            exit(MENTION, SEGV, true, None),
            exit(MENTION, exit_code(0), false, None),
        ],
        start,
        utc(0),
    );
    session.pump();
    assert!(session.notifications().is_empty() && session.sheets().is_empty());
    assert!(reports.list().is_empty());
    // The first: a notification about it, its icon and name the shell's, Reopen and Report….
    crashes.exited_at(
        &mut session.scene,
        &[exit(MENTION, SEGV, false, Some("[host] fatal signal 11"))],
        start,
        utc(100),
    );
    session.pump();
    let told = session.notifications();
    assert_eq!(told.len(), 1);
    let (number, application, title, actions) = &told[0];
    assert_eq!(application, MENTION);
    assert_eq!(title, "Mention quit unexpectedly");
    assert_eq!(actions, &["Reopen", "Report…"]);
    assert!(session.sheets().is_empty(), "one quit does not nag");
    let written = reports.list();
    assert_eq!(written.len(), 1);
    let text = std::fs::read_to_string(&written[0].0).unwrap();
    assert!(text.contains("Reason:       signal 11 (SIGSEGV)\n"));
    assert!(text.ends_with("What it reported:\n[host] fatal signal 11\n"));
    // The application cannot withdraw what the session says about it.
    session.scene.set_application(APP, MENTION);
    session.request(
        APP,
        Request::WithdrawNotification {
            notification: *number,
        },
    );
    assert!(!session.told.iter().any(|(_, e)| matches!(
        e,
        Event::NotificationRemoved { notification } if notification == number
    )));
    // The second within ten minutes: the sheet, at once, in the notification's place.
    session.told.clear();
    crashes.exited_at(
        &mut session.scene,
        &[exit(
            MENTION,
            exit_code(70),
            false,
            Some("Fatal JavaScript error: boom"),
        )],
        start + Duration::from_secs(60),
        utc(160),
    );
    session.pump();
    assert!(session.told.iter().any(|(to, e)| *to == SHELL
        && matches!(e, Event::NotificationRemoved { notification } if notification == number)));
    let sheets = session.sheets();
    assert_eq!(sheets.len(), 1);
    let (sheet, shown) = &sheets[0];
    assert_eq!(shown["kind"], "crash");
    assert_eq!(shown["id"], MENTION);
    assert_eq!(shown["name"], "Mention");
    assert_eq!(shown["times"], 2);
    assert_eq!(shown["show"], "notice");
    let report = shown["report"].as_str().unwrap();
    assert!(report.starts_with("Mention quit unexpectedly.\n"));
    assert!(report.contains("Repeated:     2 times in the last 10 minutes\n"));
    assert!(report.ends_with("Fatal JavaScript error: boom\n"));
    let file = PathBuf::from(shown["file"].as_str().unwrap());
    assert_eq!(std::fs::read_to_string(file).unwrap(), report);
    assert!(session.notifications().is_empty());
    // Another while its sheet asks: reported, and nothing more said.
    session.told.clear();
    crashes.exited_at(
        &mut session.scene,
        &[exit(MENTION, SEGV, false, None)],
        start + Duration::from_secs(70),
        utc(170),
    );
    session.pump();
    assert!(session.sheets().is_empty() && session.notifications().is_empty());
    assert_eq!(reports.list().len(), 3);
    assert_eq!(session.scene.sheets_waiting(), 1);
    let _ = sheet;
}

/// The development registry of a test: Mention, which sleeps until asked to quit, and one that
/// will not quit when asked.
fn registry(scratch: &Scratch) -> Launcher {
    let applications = scratch.0.join("applications");
    std::fs::create_dir_all(&applications).unwrap();
    std::fs::write(
        applications.join("so.oxy.mention.desktop"),
        "[Desktop Entry]\nType=Application\nName=Mention\nExec=/bin/sleep 30\n",
    )
    .unwrap();
    std::fs::write(
        applications.join("so.oxy.stubborn.desktop"),
        "[Desktop Entry]\nType=Application\nName=Stubborn\nExec=/bin/sh -c \"trap '' TERM; while :; do sleep 1; done\"\n",
    )
    .unwrap();
    std::fs::write(
        applications.join("so.oxy.settings.desktop"),
        "[Desktop Entry]\nType=Application\nName=Settings\nExec=/bin/sleep 30\n",
    )
    .unwrap();
    Launcher::new(
        Runtime {
            socket: PathBuf::from("/run/oxyos-test"),
            bloom_host: None,
            bloom_bundle: None,
            runtimes: PathBuf::from("/nonexistent-runtimes"),
            window: (960, 600),
            env: Vec::new(),
        },
        vec![applications],
        vec![],
    )
}

/// The tests' own folders for each application's own data.
fn own_data_in(root: &Path) -> impl Fn(&str) -> Option<OwnData> {
    let root = root.to_path_buf();
    move |id| {
        oxyos_apps::valid_id(id).then(|| OwnData {
            data: root.join("data/oxyos/apps").join(id),
            caches: root.join("cache/oxyos/apps").join(id),
        })
    }
}

#[test]
fn reopen_is_the_sheets_answer_and_the_notifications_press() {
    let scratch = Scratch::new("reopen");
    let mut session = Session::new();
    let mut launcher = registry(&scratch);
    let mut crashes = Crashes::new(None, None);
    let start = Instant::now();
    crashes.exited_at(
        &mut session.scene,
        &[exit(MENTION, SEGV, false, None)],
        start,
        utc(0),
    );
    session.pump();
    let number = session.notifications()[0].0;
    // Its Report…: the sheet, at the report; no application is told anything.
    session.activate(number, 2);
    let opened = crashes.serve_at(&mut session.scene, &mut launcher, false, start);
    session.pump();
    assert_eq!(opened, Served::default());
    let sheets = session.sheets();
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0].1["show"], "report");
    assert_eq!(sheets[0].1["times"], 1);
    assert!(!session
        .told
        .iter()
        .any(|(to, e)| *to == APP && matches!(e, Event::NotificationResponse { .. })));
    // Ignored: nothing is opened.
    session.answer(sheets[0].0, false);
    assert_eq!(
        crashes.serve_at(&mut session.scene, &mut launcher, false, start),
        Served::default()
    );
    // Quit again, the sheet; Reopen opens it, once.
    session.told.clear();
    crashes.exited_at(
        &mut session.scene,
        &[exit(MENTION, SEGV, false, None)],
        start + Duration::from_secs(5),
        utc(5),
    );
    session.pump();
    let sheet = session.sheets()[0].0;
    // An acceptance that cites no input of the shell's is no answer.
    session.request(
        SHELL,
        Request::SheetAnswer {
            sheet,
            accepted: true,
            serial: 0,
        },
    );
    assert_eq!(
        crashes.serve_at(&mut session.scene, &mut launcher, false, start),
        Served::default()
    );
    session.answer(sheet, true);
    assert_eq!(
        crashes
            .serve_at(&mut session.scene, &mut launcher, false, start)
            .open,
        [MENTION]
    );
    assert_eq!(
        crashes.serve_at(&mut session.scene, &mut launcher, false, start),
        Served::default()
    );
    // A quit long after is one again: a notification, whose press reopens it.
    session.told.clear();
    crashes.exited_at(
        &mut session.scene,
        &[exit(MENTION, SEGV, false, None)],
        start + REPEATED * 2,
        utc(2_000),
    );
    session.pump();
    assert!(session.sheets().is_empty());
    let number = session.notifications()[0].0;
    session.activate(number, 0);
    assert_eq!(
        crashes
            .serve_at(&mut session.scene, &mut launcher, false, start)
            .open,
        [MENTION]
    );
    // Opened meanwhile (from the dock, say): Reopen opens no second one.
    let running = launcher.launch(MENTION).unwrap();
    session.told.clear();
    crashes.exited_at(
        &mut session.scene,
        &[exit(MENTION, SEGV, false, None)],
        start + REPEATED * 4,
        utc(4_000),
    );
    session.pump();
    let number = session.notifications()[0].0;
    session.activate(number, 1);
    assert_eq!(
        crashes.serve_at(&mut session.scene, &mut launcher, false, start),
        Served::default()
    );
    assert!(launcher.kill(running));
    launcher.end(Duration::from_secs(2));
}

#[test]
fn a_reset_is_the_shells_or_settings_with_an_input_and_never_the_asker_itself() {
    let scratch = Scratch::new("who");
    let mut session = Session::new();
    let launcher = registry(&scratch);
    let mut crashes = Crashes::new(None, None).with_own_data(own_data_in(&scratch.0));
    session.scene.set_application(SETTINGS, "so.oxy.settings");
    let mention = r#"{"resetApplication":{"id":"so.oxy.mention"}}"#;
    let cases = [
        (STORE, mention, true, false, "notPermitted"),
        (SETTINGS, mention, false, false, "needsInput"),
        (SHELL, mention, true, true, "notPermitted"),
        (
            SETTINGS,
            r#"{"resetApplication":{"id":"so.oxy.settings"}}"#,
            true,
            false,
            "notPermitted",
        ),
        (
            SETTINGS,
            r#"{"resetApplication":{"id":"so.oxy.nobody"}}"#,
            true,
            false,
            "notFound",
        ),
        (
            SETTINGS,
            r#"{"resetApplication":{"id":"../x"}}"#,
            true,
            false,
            "notFound",
        ),
        (
            SETTINGS,
            r#"{"resetApplication":{"id":"so.oxy.mention","path":"/"}}"#,
            true,
            false,
            "invalidRequest",
        ),
    ];
    for (client, text, with_input, greeter, reason) in cases {
        session.told.clear();
        let request = session.ask(client, text, with_input);
        let serial = request.serial;
        crashes.handle(&mut session.scene, &launcher, greeter, request);
        session.pump();
        assert_eq!(
            session.answers(client),
            [(serial, json!({"refused": {"reason": reason}}))],
            "{text} from {client}"
        );
        assert!(session.sheets().is_empty());
    }
    // Asked properly: the system's sheet, which says what and whether it runs; the same asked
    // again meanwhile is busy.
    for client in [SETTINGS, SHELL] {
        session.told.clear();
        let request = session.ask(client, mention, true);
        crashes.handle(&mut session.scene, &launcher, false, request);
        session.pump();
        if client == SETTINGS {
            let sheets = session.sheets();
            assert_eq!(
                sheets[0].1,
                json!({"kind": "reset", "id": MENTION, "name": "Mention", "running": false})
            );
        } else {
            assert_eq!(
                session.answers(SHELL)[0].1,
                json!({"refused": {"reason": "busy"}})
            );
        }
    }
    assert!(ours(mention) && !ours(r#"{"signOut":{}}"#));
}

#[test]
fn a_reset_quits_the_application_then_erases_its_own_data_alone() {
    let scratch = Scratch::new("reset");
    let mut session = Session::new();
    let mut launcher = registry(&scratch);
    let own = own_data_in(&scratch.0);
    let mut crashes = Crashes::new(None, None).with_own_data(own_data_in(&scratch.0));
    let mention = own(MENTION).unwrap();
    let allo = own("so.oxy.allo").unwrap();
    for directory in [&mention.data, &mention.caches, &allo.data] {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(directory.join("async-storage.json"), "{corrupt").unwrap();
    }
    // It runs twice, and posted a notification of its own; it quit unexpectedly before.
    let first = launcher.launch(MENTION).unwrap();
    let second = launcher.launch(MENTION).unwrap();
    crashes.exited_at(
        &mut session.scene,
        &[exit(MENTION, SEGV, false, None)],
        Instant::now(),
        utc(0),
    );
    session.scene.set_application(APP, MENTION);
    let posted = session
        .scene
        .post_system_notification(MENTION, "chat", "Ana", "Hi", false)
        .unwrap();
    session.told.clear();
    // Refused at the sheet: nothing happens, and Settings is told it was cancelled.
    let request = session.ask(
        SETTINGS,
        r#"{"resetApplication":{"id":"so.oxy.mention"}}"#,
        true,
    );
    let asked = request.serial;
    crashes.handle(&mut session.scene, &launcher, false, request);
    session.pump();
    let sheet = session.sheets()[0].0;
    assert_eq!(session.sheets()[0].1["running"], true);
    session.answer(sheet, false);
    crashes.serve_at(&mut session.scene, &mut launcher, false, Instant::now());
    session.pump();
    assert_eq!(
        session.answers(SETTINGS),
        [(asked, json!({"refused": {"reason": "cancelled"}}))]
    );
    assert!(launcher.runs(MENTION) && mention.data.exists());
    // Accepted, from the shell's notice: every process of it asked to quit, and nothing of it
    // starts until it is reset.
    session.told.clear();
    let request = session.ask(
        SHELL,
        r#"{"resetApplication":{"id":"so.oxy.mention","reopen":true}}"#,
        true,
    );
    let asked = request.serial;
    crashes.handle(&mut session.scene, &launcher, false, request);
    session.pump();
    let sheet = session.sheets()[0].0;
    session.answer(sheet, true);
    let start = Instant::now();
    assert_eq!(
        crashes.serve_at(&mut session.scene, &mut launcher, false, start),
        Served::default()
    );
    assert!(launcher.launch(MENTION).is_err(), "held while it is reset");
    assert!(mention.data.exists(), "not while it runs");
    assert_eq!(crashes.deadline(), Some(start + QUIT_GRACE));
    // Once both ended: its own data is gone, the other application's is not; its notifications
    // are withdrawn; the shell is told it was reset; it is opened again, as new.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut ended = Vec::new();
    while ended.len() < 2 {
        assert!(Instant::now() < deadline, "it did not quit");
        ended.extend(launcher.reap());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ended
        .iter()
        .all(|exited| exited.asked && !exited.abnormal()));
    let mut pids: Vec<u32> = ended.iter().map(|exited| exited.pid).collect();
    pids.sort_unstable();
    let mut both = vec![first, second];
    both.sort_unstable();
    assert_eq!(pids, both);
    crashes.exited_at(&mut session.scene, &ended, Instant::now(), utc(10));
    let opened = crashes.serve_at(&mut session.scene, &mut launcher, false, Instant::now());
    session.pump();
    assert_eq!(
        opened,
        Served {
            open: vec![MENTION.into()],
            reset: vec![MENTION.into()],
        }
    );
    assert!(!mention.data.exists() && !mention.caches.exists());
    assert!(allo.data.join("async-storage.json").exists());
    assert_eq!(
        session.answers(SHELL),
        [(asked, json!({"reset": {"id": MENTION}}))]
    );
    assert!(session.told.iter().any(|(to, e)| *to == SHELL
        && matches!(e, Event::NotificationRemoved { notification } if *notification == posted)));
    crashes.finish_deleting();
    assert!(!scratch.0.join("data/oxyos/apps/.erasing").exists());
    // It starts again, and starts over: its next quit is a first one.
    let pid = launcher.launch(MENTION).unwrap();
    assert!(launcher.kill(pid));
    session.told.clear();
    crashes.exited_at(
        &mut session.scene,
        &[exit(MENTION, SEGV, false, None)],
        Instant::now(),
        utc(20),
    );
    session.pump();
    assert!(session.sheets().is_empty());
    assert_eq!(session.notifications().len(), 1);
    launcher.end(Duration::from_secs(2));
}

#[test]
fn an_application_that_will_not_quit_for_its_reset_is_ended() {
    let scratch = Scratch::new("stubborn");
    let mut session = Session::new();
    let mut launcher = registry(&scratch);
    let mut crashes = Crashes::new(None, None).with_own_data(own_data_in(&scratch.0));
    let own = own_data_in(&scratch.0)("so.oxy.stubborn").unwrap();
    std::fs::create_dir_all(&own.data).unwrap();
    launcher.launch("so.oxy.stubborn").unwrap();
    // Its trap set before it is asked.
    std::thread::sleep(Duration::from_millis(200));
    let request = session.ask(
        SETTINGS,
        r#"{"resetApplication":{"id":"so.oxy.stubborn"}}"#,
        true,
    );
    crashes.handle(&mut session.scene, &launcher, false, request);
    session.pump();
    let sheet = session.sheets()[0].0;
    session.answer(sheet, true);
    let start = Instant::now();
    crashes.serve_at(&mut session.scene, &mut launcher, false, start);
    std::thread::sleep(Duration::from_millis(300));
    launcher.reap();
    assert!(launcher.runs("so.oxy.stubborn"), "it ignores the request");
    crashes.serve_at(&mut session.scene, &mut launcher, false, start);
    assert!(own.data.exists());
    // Its time is up: it is ended, then reset.
    crashes.serve_at(&mut session.scene, &mut launcher, false, start + QUIT_GRACE);
    assert_eq!(crashes.deadline(), None);
    let deadline = Instant::now() + Duration::from_secs(10);
    while launcher.runs("so.oxy.stubborn") {
        assert!(Instant::now() < deadline, "it was not ended");
        launcher.reap();
        std::thread::sleep(Duration::from_millis(10));
    }
    crashes.serve_at(&mut session.scene, &mut launcher, false, start + QUIT_GRACE);
    assert!(!own.data.exists());
}

#[test]
fn the_notification_is_in_the_sessions_language() {
    let mut session = Session::new();
    let mut crashes = Crashes::new(None, None);
    crashes.set_locale("es-ES");
    crashes.exited_at(
        &mut session.scene,
        &[exit(MENTION, SEGV, false, None)],
        Instant::now(),
        utc(0),
    );
    session.pump();
    let (_, _, title, actions) = session.notifications().remove(0);
    assert_eq!(title, "Mention se cerró inesperadamente");
    assert_eq!(actions, ["Volver a abrir", "Informe…"]);
    let _ = PreeditText::default();
}
