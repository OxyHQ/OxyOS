//! The search broker with a played service: the other end of the socket, in the test.

use super::*;
use oxyos_compositor_core::Grants;
use oxyos_protocol::{Event, PreeditText, Request, TextChunk};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};

const SHELL: ClientId = 1;
const SETTINGS: ClientId = 2;
/// An application the session launched, Dropsea.
const DROPSEA: ClientId = 3;
/// A client the session did not launch.
const STRANGER: ClientId = 4;

/// The played service's process: it exits when the test says.
struct Played {
    exited: Arc<AtomicBool>,
}

impl Service for Played {
    fn exited(&mut self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }
    fn end(&mut self) {}
}

/// Starts the played service, handing the test its end of the socket; or fails while `failing`.
struct Starts {
    started: mpsc::Sender<(UnixStream, Arc<AtomicBool>)>,
    failing: Arc<AtomicBool>,
}

impl Start for Starts {
    fn start(&mut self, stdin: OwnedFd) -> io::Result<Box<dyn Service>> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(io::Error::other("no such program"));
        }
        let exited = Arc::new(AtomicBool::new(false));
        let _ = self.started.send((UnixStream::from(stdin), exited.clone()));
        Ok(Box::new(Played { exited }))
    }
}

/// The service's side: what it reads, line by line, and what it writes.
struct Side {
    lines: BufReader<UnixStream>,
    socket: UnixStream,
    exited: Arc<AtomicBool>,
}

impl Side {
    /// The next thing the service is told.
    fn told(&mut self) -> Value {
        let mut line = String::new();
        self.lines.read_line(&mut line).unwrap();
        assert!(line.ends_with('\n'), "one object a line: {line:?}");
        serde_json::from_str(&line).unwrap()
    }

    fn answer(&mut self, client: ClientId, serial: u32, text: &str) {
        let line = json!({ "answer": { "client": client, "serial": serial, "text": text } });
        writeln!(self.socket, "{line}").unwrap();
    }
}

struct World {
    scene: Scene,
    search: Search,
    started: mpsc::Receiver<(UnixStream, Arc<AtomicBool>)>,
    failing: Arc<AtomicBool>,
    /// Each client's answers, whole, by serial.
    told: BTreeMap<ClientId, Vec<(u32, String)>>,
    partial: BTreeMap<(ClientId, u32), String>,
}

impl World {
    fn new(eager: bool) -> Self {
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
                    settings: true,
                    ..Grants::default()
                },
            ),
            (DROPSEA, Grants::default()),
            (STRANGER, Grants::default()),
        ] {
            scene.connect(client, grants).unwrap();
        }
        // The session's launch record names Dropsea's process; nothing names the stranger's.
        scene.set_application(DROPSEA, "so.oxy.dropsea");
        let (started, receiving) = mpsc::channel();
        let failing = Arc::new(AtomicBool::new(false));
        let mut search = Search::with_start(
            Box::new(Starts {
                started,
                failing: failing.clone(),
            }),
            eager,
        )
        .unwrap();
        search.restart = Duration::from_millis(200);
        search.set_locale("es-ES");
        Self {
            scene,
            search,
            started: receiving,
            failing,
            told: BTreeMap::new(),
            partial: BTreeMap::new(),
        }
    }

    fn turn(&mut self) {
        self.search.serve(&mut self.scene);
        for (client, event) in self.scene.drain_events() {
            let Event::SearchText {
                serial, last, text, ..
            } = event
            else {
                continue;
            };
            let whole = self.partial.entry((client, serial)).or_default();
            whole.push_str(text.as_str());
            if last {
                let whole = self.partial.remove(&(client, serial)).unwrap();
                self.told.entry(client).or_default().push((serial, whole));
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
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// `client` asks `text`, in chunks.
    fn ask(&mut self, client: ClientId, serial: u32, text: &str) {
        let chunks = TextChunk::split(text);
        let count = chunks.len();
        for (index, chunk) in chunks.iter().enumerate() {
            self.scene
                .handle(
                    client,
                    Request::SearchRequest {
                        serial,
                        index: index as u16,
                        last: index + 1 == count,
                        text: PreeditText::new(chunk.as_str()).unwrap(),
                    },
                )
                .unwrap();
        }
    }

    /// The service just started: its end of the socket.
    fn service(&mut self) -> Side {
        let (socket, exited) = self
            .started
            .recv_timeout(Duration::from_secs(10))
            .expect("the service started");
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        Side {
            lines: BufReader::new(socket.try_clone().unwrap()),
            socket,
            exited,
        }
    }

    fn answers(&self, client: ClientId) -> Vec<(u32, String)> {
        self.told.get(&client).cloned().unwrap_or_default()
    }
}

/// The service, started, told the languages and the applications: what it is told first.
fn started(world: &mut World) -> Side {
    let mut service = world.service();
    assert_eq!(service.told(), json!({ "languages": ["es", "en"] }));
    assert_eq!(service.told(), json!({ "applications": [] }));
    service
}

#[test]
fn nothing_is_started_before_it_is_needed() {
    let mut world = World::new(false);
    for _ in 0..3 {
        world.turn();
    }
    assert!(world.started.try_recv().is_err());
    assert_eq!(world.search.deadline(), None);
    // The first ask starts it, and is told it.
    world.ask(STRANGER, 1, r#"{"status":{}}"#);
    world.turn();
    let mut service = started(&mut world);
    assert_eq!(service.told()["ask"]["serial"], 1);
}

#[test]
fn eager_it_starts_with_the_session() {
    let mut world = World::new(true);
    world.turn();
    started(&mut world);
}

#[test]
fn each_ask_is_told_with_who_asked() {
    let mut world = World::new(false);
    let query = r#"{"query":{"text":"informe trimestral de ventas"}}"#;
    world.ask(SHELL, 1, query);
    world.ask(SETTINGS, 2, r#"{"preferences":{}}"#);
    world.ask(DROPSEA, 3, r#"{"donate":{"items":[]}}"#);
    world.ask(STRANGER, 4, r#"{"status":{}}"#);
    world.turn();
    let mut service = started(&mut world);
    let asker = |id: Value, launched, shell, settings| json!({ "id": id, "launched": launched, "shell": shell, "settings": settings });
    for (client, serial, text, who) in [
        (SHELL, 1, query, asker(Value::Null, false, true, false)),
        (
            SETTINGS,
            2,
            r#"{"preferences":{}}"#,
            asker(Value::Null, false, false, true),
        ),
        (
            DROPSEA,
            3,
            r#"{"donate":{"items":[]}}"#,
            asker(json!("so.oxy.dropsea"), true, false, false),
        ),
        (
            STRANGER,
            4,
            r#"{"status":{}}"#,
            asker(Value::Null, false, false, false),
        ),
    ] {
        assert_eq!(
            service.told(),
            json!({ "ask": { "client": client, "serial": serial, "asker": who, "text": text } })
        );
    }
}

#[test]
fn answers_reach_their_askers_and_a_watcher_is_told_unasked() {
    let mut world = World::new(false);
    world.ask(DROPSEA, 7, r#"{"query":{"text":"a"}}"#);
    world.ask(STRANGER, 7, r#"{"query":{"text":"b"}}"#);
    world.turn();
    let mut service = started(&mut world);
    service.told();
    service.told();
    let long = format!(r#"{{"results":{{"items":"{}"}}}}"#, "ñ".repeat(400));
    service.answer(STRANGER, 7, r#"{"results":{"items":[]}}"#);
    service.answer(DROPSEA, 7, &long);
    service.answer(DROPSEA, 0, r#"{"changed":{}}"#);
    // An answer to what was never asked, or asked and answered, reaches no one.
    service.answer(SHELL, 9, r#"{"results":{}}"#);
    service.answer(DROPSEA, 7, r#"{"results":{}}"#);
    world.until(|w| w.answers(DROPSEA).len() == 2);
    world.turn();
    assert_eq!(
        world.answers(DROPSEA),
        [(7, long), (0, r#"{"changed":{}}"#.into())]
    );
    assert_eq!(
        world.answers(STRANGER),
        [(7, r#"{"results":{"items":[]}}"#.into())]
    );
    assert!(world.answers(SHELL).is_empty());
}

#[test]
fn an_asker_that_goes_is_told_gone() {
    let mut world = World::new(false);
    world.ask(DROPSEA, 1, r#"{"watch":{}}"#);
    world.turn();
    let mut service = started(&mut world);
    service.told();
    world.scene.disconnect(DROPSEA);
    world.turn();
    assert_eq!(service.told(), json!({ "gone": { "client": DROPSEA } }));
    // Its answer, late, reaches no one.
    service.answer(DROPSEA, 1, r#"{"done":{}}"#);
    world.turn();
    assert!(world.answers(DROPSEA).is_empty());
}

#[test]
fn the_service_going_answers_its_asks_unavailable_and_it_starts_again_on_the_next() {
    let mut world = World::new(false);
    world.ask(SHELL, 1, r#"{"query":{"text":"a"}}"#);
    world.ask(DROPSEA, 2, r#"{"status":{}}"#);
    world.turn();
    let mut service = started(&mut world);
    service.told();
    service.told();
    service.answer(SHELL, 1, r#"{"results":{}}"#);
    world.until(|w| !w.answers(SHELL).is_empty());
    // It exits: what it had not answered is answered unavailable.
    service.exited.store(true, Ordering::SeqCst);
    drop(service);
    let unavailable = r#"{"refused":{"reason":"unavailable"}}"#.to_string();
    world.until(|w| !w.answers(DROPSEA).is_empty());
    assert_eq!(world.answers(DROPSEA), [(2, unavailable.clone())]);
    assert_eq!(world.answers(SHELL), [(1, r#"{"results":{}}"#.into())]);
    // Nothing waits: no clock.
    assert_eq!(world.search.deadline(), None);
    // The next ask, before RESTART is up, waits for it, then starts it.
    world.ask(STRANGER, 3, r#"{"status":{}}"#);
    world.turn();
    assert!(world.search.deadline().is_some());
    let mut service = {
        world.until(|w| w.search.deadline().is_none());
        started(&mut world)
    };
    assert_eq!(service.told()["ask"]["client"], STRANGER);
    service.answer(STRANGER, 3, r#"{"status":{}}"#);
    world.until(|w| !w.answers(STRANGER).is_empty());
}

#[test]
fn a_service_that_cannot_start_answers_every_ask_unavailable() {
    let mut world = World::new(false);
    world.failing.store(true, Ordering::SeqCst);
    world.ask(SHELL, 1, r#"{"query":{"text":"a"}}"#);
    world.turn();
    world.ask(SETTINGS, 2, r#"{"status":{}}"#);
    world.turn();
    let unavailable = r#"{"refused":{"reason":"unavailable"}}"#.to_string();
    assert_eq!(world.answers(SHELL), [(1, unavailable.clone())]);
    assert_eq!(world.answers(SETTINGS), [(2, unavailable)]);
    assert_eq!(world.search.deadline(), None);
    // Once it can, the next ask after RESTART starts it.
    world.failing.store(false, Ordering::SeqCst);
    std::thread::sleep(world.search.restart);
    world.ask(SHELL, 3, r#"{"query":{"text":"b"}}"#);
    world.turn();
    let mut service = started(&mut world);
    assert_eq!(service.told()["ask"]["serial"], 3);
}

#[test]
fn a_service_writing_what_is_not_its_vocabulary_is_ended() {
    let mut world = World::new(false);
    world.ask(SHELL, 1, r#"{"status":{}}"#);
    world.turn();
    let mut service = started(&mut world);
    service.told();
    writeln!(service.socket, r#"{{"answer":{{"client":1}}}}"#).unwrap();
    world.until(|w| !w.answers(SHELL).is_empty());
    assert_eq!(
        world.answers(SHELL),
        [(1, r#"{"refused":{"reason":"unavailable"}}"#.into())]
    );
    // Its socket is closed on this side.
    let mut rest = String::new();
    assert_eq!(service.lines.read_line(&mut rest).unwrap(), 0);
}

#[test]
fn the_languages_are_told_as_they_change() {
    let mut world = World::new(true);
    world.turn();
    let mut service = started(&mut world);
    world.search.set_locale("en-GB");
    assert_eq!(service.told(), json!({ "languages": ["en"] }));
    // The same locale again tells nothing; the next thing told is the next ask.
    world.search.set_locale("en-GB");
    world.ask(SHELL, 1, r#"{"status":{}}"#);
    world.turn();
    assert_eq!(service.told()["ask"]["serial"], 1);
}

#[test]
fn the_applications_the_launcher_lists_are_told_at_the_start_and_as_they_change() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-search-applications-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("so.oxy.inbox.desktop"),
        "[Desktop Entry]\nType=Application\nName=Inbox\nName[es]=Bandeja\nKeywords=mail;\nKeywords[es]=correo;\nIcon=/usr/share/icons/inbox.png\nExec=inbox\n",
    )
    .unwrap();
    let mut world = World::new(true);
    world.search.list_applications(oxyos_apps::Listing {
        policy: oxyos_apps::Policy::none(),
        installed: None,
        directories: vec![directory.clone()],
        fixture: None,
    });
    world.turn();
    let mut service = world.service();
    assert_eq!(service.told(), json!({ "languages": ["es", "en"] }));
    // Named and searched in the session's language; a native client has no package.
    let inbox = json!({
        "id": "so.oxy.inbox", "name": "Bandeja", "keywords": ["correo"],
        "icon": "/usr/share/icons/inbox.png", "package": null,
    });
    assert_eq!(service.told(), json!({ "applications": [inbox] }));
    // One more installed: told again, whole.
    std::fs::write(
        directory.join("so.oxy.allo.desktop"),
        "[Desktop Entry]\nType=Application\nName=Allo\nExec=allo\n",
    )
    .unwrap();
    let allo = json!({
        "id": "so.oxy.allo", "name": "Allo", "keywords": [], "icon": null, "package": null,
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let told = loop {
        assert!(Instant::now() < deadline, "the change was not told");
        world.turn();
        service
            .socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let mut line = String::new();
        if service.lines.read_line(&mut line).is_ok() && !line.is_empty() {
            break serde_json::from_str::<Value>(&line).unwrap();
        }
    };
    assert_eq!(told, json!({ "applications": [allo, inbox] }));
    let _ = std::fs::remove_dir_all(&directory);
}
