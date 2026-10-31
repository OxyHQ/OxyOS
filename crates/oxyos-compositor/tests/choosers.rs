//! File choosers through real sockets (docs/adr/0029-chromium-on-oxyos.mdx, decision 8): an
//! application asks for its window citing its press; the session's side starts a chooser, whose
//! connection is attached as it says Hello, so its window is the asking window's sheet; the
//! chooser's choice, citing its own press, reaches the asker alone, the sheet goes, and the
//! chooser's process is ended.

use oxyos_client::{Client, Poll};
use oxyos_compositor::choosers::{Asker, Choosers};
use oxyos_compositor::Compositor;
use oxyos_compositor_core::{Output, Scene, SHEET_TOP};
use oxyos_files::Scope;
use oxyos_ipc::Listener;
use oxyos_protocol::{Event, PointerKind, Rect, Request, Size};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const OUTPUT: Rect = Rect::new(0, 0, 1280, 800);

/// What a test client is told to send, on its own thread.
type Ask = Box<dyn FnOnce(&Client) + Send>;
const BUTTON: u32 = 272;

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A client with one window of `size`, every configure answered with a white frame; it sends what
/// `asks` gives it, and reports each press's serial and each answer whole.
fn application(
    socket: std::path::PathBuf,
    size: Size,
    asks: mpsc::Receiver<Ask>,
    presses: mpsc::Sender<u64>,
    answers: mpsc::Sender<(u32, String)>,
) {
    std::thread::spawn(move || {
        let mut client = Client::connect(&socket).unwrap();
        client
            .send(&Request::CreateToplevel { surface: 1, size })
            .unwrap();
        let mut answer = String::new();
        loop {
            match client.poll() {
                Ok(Poll::Event(Event::Configure {
                    surface,
                    serial,
                    rect,
                    ..
                })) => {
                    let pixels = vec![0xff; (rect.width * rect.height * 4) as usize];
                    client
                        .present(surface, serial, Size::new(rect.width, rect.height), &pixels)
                        .unwrap();
                }
                Ok(Poll::Event(Event::Pointer {
                    kind: PointerKind::Press,
                    serial,
                    ..
                })) => {
                    let _ = presses.send(serial);
                }
                Ok(Poll::Event(Event::FilesChosen {
                    serial,
                    index,
                    last,
                    text,
                })) => {
                    if index == 0 {
                        answer.clear();
                    }
                    answer.push_str(text.as_str());
                    if last {
                        let _ = answers.send((serial, std::mem::take(&mut answer)));
                    }
                }
                Ok(Poll::Event(_)) => {}
                Ok(Poll::Closed) | Err(_) => return,
                Ok(Poll::Empty) => match asks.try_recv() {
                    Ok(ask) => ask(&client),
                    Err(mpsc::TryRecvError::Disconnected) => return,
                    Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(1)),
                },
            }
        }
    });
}

struct Session {
    compositor: Compositor,
    listener: Listener,
    choosers: Choosers,
}

impl Session {
    fn until(&mut self, what: &str, mut done: impl FnMut(&mut Session) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(self) {
            assert!(Instant::now() < deadline, "{what} in time");
            self.compositor.serve(&self.listener).unwrap();
        }
    }

    fn press(&mut self, x: i32, y: i32) {
        self.compositor.scene.pointer_motion(x, y);
        self.compositor.scene.pointer_button(BUTTON, true);
        self.compositor.scene.pointer_button(BUTTON, false);
        self.compositor.serve(&self.listener).unwrap();
    }
}

#[test]
fn a_chooser_started_for_a_window_is_its_sheet_and_its_choice_reaches_the_asker_alone() {
    let directory = std::env::temp_dir().join(format!("oxyos-choosers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let home = directory.join("home");
    std::fs::create_dir_all(home.join("Pictures")).unwrap();
    std::fs::write(home.join("Pictures/a b.png"), b"png").unwrap();
    std::fs::write(directory.join("mountinfo"), b"").unwrap();
    let _cleanup = TempDir(directory.clone());
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let scope = Scope::new(
        &home,
        &home.join(".local/share"),
        &home.join(".config"),
        Vec::new(),
        directory.join("mountinfo"),
        uid,
    )
    .unwrap();
    let socket = directory.join("compositor");
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: OUTPUT,
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let mut s = Session {
        compositor: Compositor::new(scene, uid),
        listener,
        choosers: Choosers::new(Some(scope)),
    };
    s.compositor.clock = Some(Duration::from_secs(100));

    // The application: a window of 800x600, pressed; it asks for an image citing that press.
    let (ask, asks) = mpsc::channel::<Ask>();
    let (pressed, presses) = mpsc::channel();
    let (answered, answers) = mpsc::channel();
    application(socket.clone(), Size::new(800, 600), asks, pressed, answered);
    s.until("the window was not shown", |s| {
        s.compositor.surface_rect(1, 1).is_some()
    });
    let window = s.compositor.surface_rect(1, 1).unwrap();
    s.press(window.x + 40, window.y + 40);
    let input = presses.recv_timeout(Duration::from_secs(10)).unwrap();
    ask.send(Box::new(move |client: &Client| {
        client
            .choose_files(9, 1, input, r#"{"open":{"types":["image/png"]}}"#)
            .unwrap();
    }))
    .unwrap();
    let mut starts = Vec::new();
    let astro = Asker {
        id: "so.oxy.astro".into(),
        name: "Astro".into(),
    };
    s.until("the request was not handed on", |s| {
        starts = s
            .choosers
            .serve(&mut s.compositor.scene, &|_| Some(astro.clone()));
        !starts.is_empty()
    });
    assert_eq!(starts.len(), 1);
    let shown: serde_json::Value = serde_json::from_str(&starts[0].shown).unwrap();
    assert_eq!(shown["mode"], "open");
    assert_eq!(shown["types"], serde_json::json!(["image/png"]));
    assert_eq!(shown["application"]["name"], "Astro");

    // The chooser: started as a process of its own (this one's pid, in the tests), attached as
    // it says Hello; its window of 600x400 is the window's sheet, at its top.
    let id = starts[0].id;
    s.compositor.expect_chooser(std::process::id(), id);
    s.choosers.started(&mut s.compositor.scene, id, Some(4242));
    let (choose, chooses) = mpsc::channel::<Ask>();
    let (chooser_pressed, chooser_presses) = mpsc::channel();
    let (chooser_answered, chooser_answers) = mpsc::channel();
    application(
        socket.clone(),
        Size::new(600, 400),
        chooses,
        chooser_pressed,
        chooser_answered,
    );
    s.until("the sheet was not shown", |s| {
        s.compositor.surface_rect(2, 1).is_some()
    });
    assert_eq!(
        s.compositor.surface_rect(2, 1),
        Some(Rect::new(
            window.x + 100,
            window.y + SHEET_TOP as i32,
            600,
            400
        ))
    );
    assert_eq!(s.compositor.scene.chooser_of(2), Some(id));
    // A press beside the sheet does not reach the window; one on the sheet reaches the chooser.
    s.press(window.x + 20, window.y + 300);
    s.press(window.x + 600, window.y + SHEET_TOP as i32 + 380);
    let open = chooser_presses
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    assert!(presses.try_recv().is_err(), "the window was pressed");
    let file = oxyos_files::uri::of(&home.join("Pictures/a b.png"), false);
    let text = format!(r#"{{"chosen":["{file}"]}}"#);
    choose
        .send(Box::new(move |client: &Client| {
            client.answer_file_chooser(open, &text).unwrap();
        }))
        .unwrap();
    s.until("the answer was not checked", |s| {
        s.choosers.serve(&mut s.compositor.scene, &|_| None);
        s.compositor.surface_rect(2, 1).is_none()
    });
    s.compositor.serve(&s.listener).unwrap();
    assert_eq!(
        answers.recv_timeout(Duration::from_secs(10)).unwrap(),
        (9, format!(r#"{{"chosen":["{file}"]}}"#))
    );
    assert!(chooser_answers.try_recv().is_err(), "the chooser was told");
    assert_eq!(s.choosers.take_ended(), vec![4242]);
}
