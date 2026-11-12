//! Windows coming back where they were (docs/adr/0070): where a new window goes, what the record
//! keeps and when it is written, what opens at login, and what is never done.

use super::*;
use oxyos_compositor_core::scene::LaunchedWindow;
use oxyos_compositor_core::{ClientId, Grants, Output};
use oxyos_protocol::{Clock, Event, PanelEdge, Rect, Request, ShellKind};

const SHELL: ClientId = 1;
const INBOX: ClientId = 2;
const ALLO: ClientId = 3;
const CHAT: ClientId = 4;
const MAIL: ClientId = 5;
/// A client of the session's own that no launch record names.
const OTHER: ClientId = 6;

/// A folder of the test's own, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("oxyos-places-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn record(&self) -> PathBuf {
        self.0.join("oxyos").join("windows")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A display of 1280 × 720 with the shell's top bar (30) and the applications' clients, each
/// named by its launch record, but OTHER.
struct Session {
    scene: Scene,
    now: Instant,
    buffer: u64,
}

impl Session {
    fn new() -> Self {
        let mut scene = Scene::new();
        scene
            .add_output(Output {
                id: 1,
                rect: Rect::new(0, 0, 1280, 720),
                scale_120: 120,
                refresh_mhz: 60_000,
            })
            .unwrap();
        let mut session = Self {
            scene,
            now: Instant::now(),
            buffer: 0,
        };
        session
            .scene
            .connect(
                SHELL,
                Grants {
                    shell: true,
                    ..Grants::default()
                },
            )
            .unwrap();
        session
            .scene
            .handle(
                SHELL,
                Request::CreateShellSurface {
                    surface: 50,
                    kind: ShellKind::Panel,
                    edge: PanelEdge::Top,
                    output: 1,
                    size: Size::new(1, 30),
                    exclusive: 30,
                },
            )
            .unwrap();
        session.draw(SHELL, 50);
        for client in [INBOX, ALLO, CHAT, MAIL, OTHER] {
            session.scene.connect(client, Grants::default()).unwrap();
        }
        session.scene.set_application(INBOX, "so.oxy.inbox");
        session.scene.set_application(ALLO, "so.oxy.allo");
        session.scene.set_application(CHAT, "so.oxy.chat");
        session.scene.set_application(MAIL, "so.oxy.mail");
        session
    }

    /// `client` draws `surface` at its latest configure, and the frame is shown.
    fn draw(&mut self, client: ClientId, surface: u32) {
        let (serial, rect) = self
            .scene
            .drain_events()
            .into_iter()
            .rev()
            .filter(|(to, _)| *to == client)
            .filter_map(|(_, event)| match event {
                Event::Configure {
                    surface: configured,
                    serial,
                    rect,
                    ..
                } if configured == surface => Some((serial, rect)),
                _ => None,
            })
            .next()
            .expect("configured");
        self.submit(client, surface, serial, Size::new(rect.width, rect.height));
    }

    /// `client` makes a window of 400 × 300 and the program's turn runs before its first frame.
    fn make(&mut self, places: &mut Places, client: ClientId, surface: u32) -> SurfaceKey {
        self.scene
            .handle(
                client,
                Request::CreateToplevel {
                    surface,
                    size: Size::new(400, 300),
                },
            )
            .unwrap();
        places.serve(&mut self.scene, self.now);
        SurfaceKey {
            client,
            id: surface,
        }
    }

    /// `client` opens a window: made, placed, drawn and shown.
    fn open(&mut self, places: &mut Places, client: ClientId, surface: u32) -> SurfaceKey {
        let key = self.make(places, client, surface);
        self.draw(client, surface);
        places.serve(&mut self.scene, self.now);
        key
    }

    fn close(&mut self, places: &mut Places, client: ClientId, surface: u32) {
        self.scene
            .handle(client, Request::Destroy { surface })
            .unwrap();
        places.serve(&mut self.scene, self.now);
    }

    fn pass(&mut self, places: &mut Places, by: Duration) {
        self.now += by;
        places.serve(&mut self.scene, self.now);
    }

    fn window(&self, key: SurfaceKey) -> LaunchedWindow {
        self.scene
            .launched_windows()
            .into_iter()
            .find(|window| window.key == key)
            .expect("shown")
    }
}

fn kept(application: &str, frame: (i32, i32, u32, u32)) -> Kept {
    Kept {
        application: application.to_owned(),
        x: frame.0,
        y: frame.1,
        width: frame.2,
        height: frame.3,
        zoomed: false,
        desktop: 0,
        minimized: false,
    }
}

fn place(frame: (i32, i32, u32, u32)) -> WindowPlace {
    WindowPlace {
        origin: (frame.0, frame.1),
        size: Size::new(frame.2, frame.3),
        zoomed: false,
        desktop: 0,
        minimized: false,
    }
}

fn written(scratch: &Scratch, record: &Record) {
    write(&scratch.record(), record).unwrap();
}

fn read(scratch: &Scratch) -> Record {
    serde_json::from_slice(&std::fs::read(scratch.record()).unwrap()).unwrap()
}

#[test]
fn a_window_opens_where_its_application_s_was() {
    let scratch = Scratch::new("opens");
    written(
        &scratch,
        &Record {
            desktops: 1,
            windows: vec![kept("so.oxy.inbox", (100, 200, 500, 380))],
        },
    );
    let mut session = Session::new();
    let mut places = Places::at(Some(scratch.record()));
    places.start(&mut session.scene);
    let inbox = session.open(&mut places, INBOX, 1);
    assert_eq!(session.window(inbox).place, place((100, 200, 500, 380)));
    // Another application's window opens where windows open: centred.
    let allo = session.open(&mut places, ALLO, 1);
    assert_eq!(session.window(allo).place, place((440, 225, 400, 300)));
}

#[test]
fn it_is_configured_at_its_frame_before_its_first_frame() {
    let scratch = Scratch::new("first-frame");
    written(
        &scratch,
        &Record {
            desktops: 1,
            windows: vec![kept("so.oxy.inbox", (100, 200, 500, 380))],
        },
    );
    let mut session = Session::new();
    let mut places = Places::at(Some(scratch.record()));
    let inbox = session.make(&mut places, INBOX, 1);
    // Told its size as it was made, then the frame kept, before it drew anything: its first
    // frame is at the frame kept, and nothing of it is shown before.
    let configures: Vec<(u64, Rect)> = session
        .scene
        .drain_events()
        .into_iter()
        .filter_map(|(client, event)| match event {
            Event::Configure {
                surface: 1,
                serial,
                rect,
                ..
            } if client == INBOX => Some((serial, rect)),
            _ => None,
        })
        .collect();
    assert_eq!(
        configures.iter().map(|(_, rect)| *rect).collect::<Vec<_>>(),
        vec![Rect::new(0, 0, 400, 300), Rect::new(0, 0, 500, 380)]
    );
    assert!(session.scene.launched_windows().is_empty());
    session.submit(INBOX, 1, configures[1].0, Size::new(500, 380));
    assert_eq!(session.window(inbox).place, place((100, 200, 500, 380)));
}

impl Session {
    /// `client` draws `surface` for the configure `serial` at `extent`, and the frame is shown.
    fn submit(&mut self, client: ClientId, surface: u32, serial: u64, extent: Size) {
        self.buffer += 1;
        self.scene
            .handle(
                client,
                Request::AttachBuffer {
                    buffer: self.buffer,
                    format: oxyos_protocol::Format::Xrgb8888,
                    extent,
                    stride: extent.width * 4,
                    offset: 0,
                },
            )
            .unwrap();
        self.scene
            .handle(
                client,
                Request::Submit {
                    surface,
                    sequence: self.buffer,
                    buffer: self.buffer,
                    configure: serial,
                    extent,
                },
            )
            .unwrap();
        self.scene.begin_frame();
        self.scene.end_frame(self.buffer, Clock::Synthetic);
    }
}

#[test]
fn the_record_follows_the_windows_and_keeps_an_application_s_as_its_last_one_went() {
    let scratch = Scratch::new("follows");
    let mut session = Session::new();
    let mut places = Places::at(Some(scratch.record()));
    let inbox = session.open(&mut places, INBOX, 1);
    session.scene.move_window_to(inbox, (60, 90));
    places.serve(&mut session.scene, session.now);
    // Not on the turn that changed it: a second later.
    assert!(!scratch.record().exists());
    assert_eq!(places.deadline(), Some(session.now + WRITE_AFTER));
    session.pass(&mut places, WRITE_AFTER);
    assert_eq!(
        read(&scratch),
        Record {
            desktops: 1,
            windows: vec![kept("so.oxy.inbox", (60, 90, 400, 300))],
        }
    );
    assert_eq!(places.deadline(), None);
    // Its window goes (it quit): what is kept stays.
    session.close(&mut places, INBOX, 1);
    session.pass(&mut places, WRITE_AFTER);
    assert_eq!(
        read(&scratch).windows,
        vec![kept("so.oxy.inbox", (60, 90, 400, 300))]
    );
    // Opened again: where it was.
    let again = session.open(&mut places, INBOX, 2);
    assert_eq!(session.window(again).place, place((60, 90, 400, 300)));
    // A file of mode 0600: the person's.
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(scratch.record())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn an_application_s_windows_come_back_in_their_order_and_one_closed_is_forgotten() {
    let scratch = Scratch::new("several");
    written(
        &scratch,
        &Record {
            desktops: 1,
            windows: vec![
                kept("so.oxy.inbox", (10, 40, 300, 200)),
                kept("so.oxy.allo", (500, 300, 300, 200)),
                kept("so.oxy.inbox", (600, 60, 320, 240)),
            ],
        },
    );
    let mut session = Session::new();
    let mut places = Places::at(Some(scratch.record()));
    let first = session.open(&mut places, INBOX, 1);
    // The record follows its first window while the second is opening: the second still comes
    // back to its own place.
    session.pass(&mut places, WRITE_AFTER);
    let second = session.open(&mut places, INBOX, 2);
    assert_eq!(session.window(first).place, place((10, 40, 300, 200)));
    assert_eq!(session.window(second).place, place((600, 60, 320, 240)));
    session.pass(&mut places, WRITE_AFTER);
    // Allo, not open, keeps its place in the order.
    assert_eq!(
        read(&scratch).windows,
        vec![
            kept("so.oxy.inbox", (10, 40, 300, 200)),
            kept("so.oxy.allo", (500, 300, 300, 200)),
            kept("so.oxy.inbox", (600, 60, 320, 240)),
        ]
    );
    // The person closes the first; the second stays open: the first is forgotten.
    session.close(&mut places, INBOX, 1);
    session.pass(&mut places, WRITE_AFTER);
    assert_eq!(
        read(&scratch).windows,
        vec![
            kept("so.oxy.inbox", (600, 60, 320, 240)),
            kept("so.oxy.allo", (500, 300, 300, 200)),
        ]
    );
}

#[test]
fn a_window_is_kept_on_a_display_that_shows_it_and_out_of_the_top_bar() {
    let scratch = Scratch::new("kept-on");
    written(
        &scratch,
        &Record {
            desktops: 1,
            windows: vec![
                // On a display no longer there.
                kept("so.oxy.inbox", (1500, 100, 500, 380)),
                // Under the top bar and partly off the right.
                kept("so.oxy.allo", (1100, 0, 400, 300)),
                // Larger than the work area now.
                kept("so.oxy.chat", (0, 30, 2560, 1400)),
            ],
        },
    );
    let mut session = Session::new();
    let mut places = Places::at(Some(scratch.record()));
    let inbox = session.open(&mut places, INBOX, 1);
    assert_eq!(session.window(inbox).place, place((390, 185, 500, 380)));
    let allo = session.open(&mut places, ALLO, 1);
    assert_eq!(session.window(allo).place, place((1100, 30, 400, 300)));
    let chat = session.open(&mut places, CHAT, 1);
    assert_eq!(session.window(chat).place, place((0, 30, 1280, 690)));
}

#[test]
fn a_zoomed_window_comes_back_zoomed_with_its_frame_to_go_back_to() {
    let scratch = Scratch::new("zoomed");
    let mut session = Session::new();
    let mut places = Places::at(Some(scratch.record()));
    let inbox = session.open(&mut places, INBOX, 1);
    session.scene.move_window_to(inbox, (80, 100));
    session
        .scene
        .handle(
            INBOX,
            Request::SetMaximized {
                surface: 1,
                enabled: true,
            },
        )
        .unwrap();
    session.draw(INBOX, 1);
    session.pass(&mut places, WRITE_AFTER);
    let zoomed = Kept {
        zoomed: true,
        ..kept("so.oxy.inbox", (80, 100, 400, 300))
    };
    assert_eq!(read(&scratch).windows, vec![zoomed]);
    session.close(&mut places, INBOX, 1);
    let again = session.open(&mut places, INBOX, 2);
    assert_eq!(
        session.window(again).place,
        WindowPlace {
            zoomed: true,
            ..place((80, 100, 400, 300))
        }
    );
    // Its frame now is the work area's.
    assert_eq!(
        session.scene.upcoming_rect(again),
        Some(Rect::new(0, 30, 1280, 690))
    );
    // Unzoomed, it goes back to where it was.
    session
        .scene
        .handle(
            INBOX,
            Request::SetMaximized {
                surface: 2,
                enabled: false,
            },
        )
        .unwrap();
    assert_eq!(
        session.scene.upcoming_rect(again),
        Some(Rect::new(80, 100, 400, 300))
    );
}

fn at_login_record() -> Record {
    Record {
        desktops: 3,
        windows: vec![
            kept("so.oxy.allo", (40, 60, 400, 300)),
            Kept {
                minimized: true,
                ..kept("so.oxy.mail", (300, 300, 400, 300))
            },
            Kept {
                desktop: 2,
                ..kept("so.oxy.chat", (500, 100, 400, 300))
            },
            kept("so.oxy.inbox", (200, 120, 400, 300)),
        ],
    }
}

#[test]
fn what_opens_at_login_takes_its_desktop_minimized_state_and_order_too() {
    let scratch = Scratch::new("at-login");
    written(&scratch, &at_login_record());
    let mut session = Session::new();
    let mut places = Places::at(Some(scratch.record()));
    places.start(&mut session.scene);
    // The desktops the person had.
    assert_eq!(session.scene.workspaces(), (3, 0));
    for application in ["so.oxy.allo", "so.oxy.mail", "so.oxy.chat", "so.oxy.inbox"] {
        places.opened_at_login(application);
    }
    // Inbox, in front last time, is ready first; Allo, behind it, after.
    let inbox = session.open(&mut places, INBOX, 1);
    let chat = session.open(&mut places, CHAT, 1);
    let mail = session.open(&mut places, MAIL, 1);
    let allo = session.open(&mut places, ALLO, 1);
    let windows = session.scene.launched_windows();
    let order: Vec<SurfaceKey> = windows.iter().map(|window| window.key).collect();
    assert_eq!(order, vec![allo, mail, chat, inbox]);
    // Inbox has the keyboard, as it had; Allo, arriving behind it, took nothing.
    assert_eq!(session.scene.focus(), Some(inbox));
    assert_eq!(
        session.window(chat).place,
        WindowPlace {
            desktop: 2,
            ..place((500, 100, 400, 300))
        }
    );
    assert!(session.window(mail).place.minimized);
    assert!(!session.scene.shown(mail));
    assert!(!session.scene.shown(chat));
    assert!(session.scene.shown(allo));
    assert_eq!(session.window(allo).place, place((40, 60, 400, 300)));
    // Nothing changed: the record is as it was, never rewritten.
    session.pass(&mut places, WRITE_AFTER);
    assert_eq!(read(&scratch), at_login_record());
    // Mail quit, and the person opens it again later: at its frame, in front, not minimized.
    session.close(&mut places, MAIL, 1);
    let again = session.open(&mut places, MAIL, 2);
    assert!(!session.window(again).place.minimized);
    assert!(session.scene.shown(again));
    assert_eq!(session.scene.focus(), Some(again));
}

#[test]
fn ready_in_their_order_they_end_in_it_too() {
    let scratch = Scratch::new("at-login-order");
    written(&scratch, &at_login_record());
    let mut session = Session::new();
    let mut places = Places::at(Some(scratch.record()));
    places.start(&mut session.scene);
    for application in ["so.oxy.allo", "so.oxy.inbox"] {
        places.opened_at_login(application);
    }
    let allo = session.open(&mut places, ALLO, 1);
    let inbox = session.open(&mut places, INBOX, 1);
    let order: Vec<SurfaceKey> = session
        .scene
        .launched_windows()
        .iter()
        .map(|window| window.key)
        .collect();
    assert_eq!(order, vec![allo, inbox]);
    assert_eq!(session.scene.focus(), Some(inbox));
}

#[test]
fn what_the_person_opens_later_opens_in_front_on_their_desktop() {
    let scratch = Scratch::new("later");
    written(&scratch, &at_login_record());
    let mut session = Session::new();
    let mut places = Places::at(Some(scratch.record()));
    places.start(&mut session.scene);
    let inbox = session.open(&mut places, INBOX, 1);
    let chat = session.open(&mut places, CHAT, 1);
    let mail = session.open(&mut places, MAIL, 1);
    // Its frame, on the desktop shown, shown, in front, with the keyboard.
    assert_eq!(session.window(chat).place, place((500, 100, 400, 300)));
    assert_eq!(session.window(mail).place, place((300, 300, 400, 300)));
    assert!(session.scene.shown(mail));
    assert_eq!(session.scene.focus(), Some(mail));
    let order: Vec<SurfaceKey> = session
        .scene
        .launched_windows()
        .iter()
        .map(|window| window.key)
        .collect();
    assert_eq!(order, vec![inbox, chat, mail]);
}

#[test]
fn a_client_no_launch_record_names_is_never_placed_nor_kept() {
    let scratch = Scratch::new("other");
    let mut session = Session::new();
    let mut places = Places::at(Some(scratch.record()));
    session.open(&mut places, OTHER, 1);
    session.pass(&mut places, WRITE_AFTER);
    assert!(!scratch.record().exists());
    assert!(session.scene.launched_windows().is_empty());
}

#[test]
fn reset_forgets_an_application_s_windows() {
    let scratch = Scratch::new("reset");
    written(
        &scratch,
        &Record {
            desktops: 1,
            windows: vec![
                kept("so.oxy.inbox", (100, 200, 500, 380)),
                kept("so.oxy.allo", (40, 60, 400, 300)),
            ],
        },
    );
    let mut session = Session::new();
    let mut places = Places::at(Some(scratch.record()));
    places.forget("so.oxy.inbox", session.now);
    session.pass(&mut places, WRITE_AFTER);
    assert_eq!(
        read(&scratch).windows,
        vec![kept("so.oxy.allo", (40, 60, 400, 300))]
    );
    let inbox = session.open(&mut places, INBOX, 1);
    assert_eq!(session.window(inbox).place, place((440, 225, 400, 300)));
}

#[test]
fn a_record_that_cannot_be_read_is_none_and_the_login_screen_keeps_nothing() {
    let scratch = Scratch::new("unreadable");
    std::fs::create_dir_all(scratch.record().parent().unwrap()).unwrap();
    std::fs::write(scratch.record(), b"{ not json").unwrap();
    let mut session = Session::new();
    let mut places = Places::at(Some(scratch.record()));
    places.start(&mut session.scene);
    assert_eq!(session.scene.workspaces(), (1, 0));
    let inbox = session.open(&mut places, INBOX, 1);
    assert_eq!(session.window(inbox).place, place((440, 225, 400, 300)));
    // No file: nothing written, ever.
    let mut nowhere = Places::at(None);
    let mut other = Session::new();
    other.open(&mut nowhere, INBOX, 1);
    nowhere.flush();
    assert_eq!(nowhere.deadline(), None);
}

#[test]
fn the_order_keeps_the_places_of_applications_not_open_and_is_bounded() {
    let a = kept("a", (0, 0, 10, 10));
    let b = kept("b", (0, 0, 10, 10));
    let c = kept("c", (0, 0, 10, 10));
    let moved = |kept: &Kept| Kept {
        x: 5,
        ..kept.clone()
    };
    // C quits first as the session ends, then B: their places stay.
    assert_eq!(
        follow(&[a.clone(), b.clone(), c.clone()], &[a.clone(), b.clone()]),
        vec![a.clone(), b.clone(), c.clone()]
    );
    // Open ones take the order they are in now; one new goes in front.
    let d = kept("d", (0, 0, 10, 10));
    assert_eq!(
        follow(
            &[a.clone(), b.clone(), c.clone()],
            &[moved(&b), a.clone(), d.clone()]
        ),
        vec![moved(&b), a.clone(), c.clone(), d.clone()]
    );
    // Bounded: those of applications not open go first, the back-most first.
    let many: Vec<Kept> = (0..MAX_KEPT)
        .map(|n| kept(&format!("app{n}"), (0, 0, 10, 10)))
        .collect();
    let followed = follow(&many, &[d.clone()]);
    assert_eq!(followed.len(), MAX_KEPT);
    assert_eq!(followed[0].application, "app1");
    assert_eq!(followed.last(), Some(&d));
}
