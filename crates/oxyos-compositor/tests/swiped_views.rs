//! A touchpad's swipe driving App Exposé (docs/adr/0046-interactive-gestures.mdx), through real
//! sockets on the compositor's clock set by the test, as the session's shell answers it: the
//! window moved into App Exposé's view is drawn exactly as far from where it is toward its view
//! as the fingers have gone, in the frame composed after their motion; let go, it goes on from
//! there on the overlay's spring, at the fingers' speed, or back. The swipe back closes it the
//! same way, the shell told only once the fingers let it go past closing.

use oxyos_client::Client;
use oxyos_compositor::{Compositor, FlightSeen};
use oxyos_compositor_core::{Output, RenderItem, Scene};
use oxyos_input::{Action, GesturePhase};
use oxyos_ipc::Listener;
use oxyos_protocol::{
    Event, Format, GestureOverlay, PanelEdge, Rect, Request, ShellKind, ShortcutIntent, Size,
    SystemShortcut,
};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Where the shell places the window on its overlay: a 200 × 100 window fitted into it is
/// 100 × 50 at (40, 325).
const CARD: Rect = Rect::new(40, 300, 100, 100);
const VIEW: Rect = Rect::new(40, 325, 100, 50);

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Session {
    compositor: Compositor,
    listener: Listener,
    /// What the shell was told, in order.
    told: mpsc::Receiver<ShortcutIntent>,
    _directory: TempDir,
}

/// `from` `share` of the way to `to`, edge by edge, rounded as the compositor rounds a flight.
fn between(from: Rect, to: Rect, share: f64) -> Rect {
    let along = |a: i64, b: i64| (a as f64 + (b - a) as f64 * share).round() as i64;
    let left = along(i64::from(from.x), i64::from(to.x));
    let top = along(i64::from(from.y), i64::from(to.y));
    let right = along(from.right(), to.right()).max(left + 1);
    let bottom = along(from.bottom(), to.bottom()).max(top + 1);
    Rect::new(
        left as i32,
        top as i32,
        (right - left) as u32,
        (bottom - top) as u32,
    )
}

impl Session {
    fn at_ms(&mut self, ms: u64) {
        self.compositor.clock = Some(Duration::from_secs(100) + Duration::from_millis(ms));
        self.compositor.serve(&self.listener).unwrap();
    }

    fn until(&mut self, ms: u64, what: &str, done: impl Fn(&Compositor) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(&self.compositor) {
            assert!(Instant::now() < deadline, "{what} did not happen in time");
            self.at_ms(ms);
        }
    }

    /// Three fingers, at `ms` on the device's clock (the test's), by (0, `dy`) logical pixels.
    fn swipe(&mut self, phase: GesturePhase, dy: f64, ms: u64) {
        self.compositor
            .scene
            .set_input_time((100_000 + ms) * 1_000_000);
        self.compositor.apply_input(Action::Swipe {
            phase,
            fingers: 3,
            dx: 0.0,
            dy,
        });
    }

    fn window(&self) -> Rect {
        self.compositor.surface_rect(1, 1).unwrap()
    }

    /// Where the window was drawn last in a flight, and whether it was flying to its view.
    fn flown(&mut self) -> (Rect, bool) {
        let flights: Vec<FlightSeen> = self.compositor.take_flights();
        let last = flights.last().expect("a flight");
        (*last.path.last().unwrap(), last.arriving)
    }

    fn told(&self) -> Vec<ShortcutIntent> {
        self.told.try_iter().collect()
    }

    fn shown(&self) -> bool {
        app_expose(&self.compositor)
    }
}

/// Whether App Exposé's overlay is shown.
fn app_expose(c: &Compositor) -> bool {
    c.scene.render_list().iter().any(|item| {
        matches!(*item, RenderItem::Surface { key, .. }
            if c.scene.gesture_overlay(key) == GestureOverlay::AppExpose)
    })
}

/// A compositor on its test clock with an application's window, settled, and a shell with a dock
/// that shows App Exposé when told to open it (its overlay said to be App Exposé's, the window
/// moved into `CARD`) and takes it away when told to close it.
fn session(name: &str) -> Session {
    session_with(name, None)
}

/// The same; with `held`, App Exposé is two surfaces, as the session's shell draws it: its
/// backdrop (surface 3), drawn at once, and the overlay its window is moved into (surface 2),
/// drawn only once `held` says so, as a loaded machine's shell gets to it.
fn session_with(name: &str, held: Option<mpsc::Receiver<()>>) -> Session {
    let directory =
        std::env::temp_dir().join(format!("oxyos-swiped-views-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let shell_socket = directory.join("shell");
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: Rect::new(0, 0, 640, 480),
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    compositor.shell_listener = Some(Listener::bind(&shell_socket).unwrap());
    compositor.clock = Some(Duration::from_secs(100));
    std::thread::spawn(move || {
        let mut client = Client::connect(&socket).unwrap();
        client
            .send(&Request::CreateToplevel {
                surface: 1,
                size: Size::new(200, 100),
            })
            .unwrap();
        let configure = loop {
            if let Some(Event::Configure { serial, .. }) = client.next_event().unwrap() {
                break serial;
            }
        };
        let red: Vec<u8> = (0..200 * 100)
            .flat_map(|_| 0xffff_0000u32.to_le_bytes())
            .collect();
        client
            .present(1, configure, Size::new(200, 100), &red)
            .unwrap();
        while client.next_event().is_ok() {}
    });
    let (tell, told) = mpsc::channel();
    std::thread::spawn(move || {
        let mut shell = Client::connect(&shell_socket).unwrap();
        // Its dock: what makes it the shell of output 1.
        shell
            .send(&Request::CreateShellSurface {
                surface: 1,
                kind: ShellKind::Panel,
                edge: PanelEdge::Bottom,
                output: 1,
                size: Size::new(640, 20),
                exclusive: 20,
            })
            .unwrap();
        let mut window = None;
        let mut shown = false;
        loop {
            let Ok(event) = shell.next_event() else {
                return;
            };
            match event {
                Some(Event::Configure {
                    surface: 1, serial, ..
                }) => {
                    shell
                        .present(1, serial, Size::new(640, 20), &vec![0x80; 640 * 20 * 4])
                        .unwrap();
                }
                Some(Event::WindowState { window: number, .. }) => window = Some(number),
                Some(Event::SystemShortcut {
                    shortcut: SystemShortcut::AppExpose,
                    intent,
                    ..
                }) => {
                    tell.send(intent).unwrap();
                    match intent {
                        ShortcutIntent::Open if !shown => {
                            shown = true;
                            if held.is_some() {
                                shell
                                    .send(&Request::CreateShellSurface {
                                        surface: 3,
                                        kind: ShellKind::Overlay,
                                        edge: PanelEdge::None,
                                        output: 1,
                                        size: Size::new(640, 480),
                                        exclusive: 0,
                                    })
                                    .unwrap();
                                shell
                                    .send(&Request::SetGestureOverlay {
                                        surface: 3,
                                        overlay: GestureOverlay::AppExpose,
                                    })
                                    .unwrap();
                            }
                            shell
                                .send(&Request::CreateShellSurface {
                                    surface: 2,
                                    kind: ShellKind::Overlay,
                                    edge: PanelEdge::None,
                                    output: 1,
                                    size: Size::new(640, 480),
                                    exclusive: 0,
                                })
                                .unwrap();
                            shell
                                .send(&Request::SetGestureOverlay {
                                    surface: 2,
                                    overlay: GestureOverlay::AppExpose,
                                })
                                .unwrap();
                        }
                        ShortcutIntent::Close if shown => {
                            shown = false;
                            shell.send(&Request::Destroy { surface: 2 }).unwrap();
                            if held.is_some() {
                                shell.send(&Request::Destroy { surface: 3 }).unwrap();
                            }
                        }
                        _ => {}
                    }
                }
                Some(Event::Configure {
                    surface: 3, serial, ..
                }) if shown => {
                    shell
                        .present_format(
                            3,
                            serial,
                            Size::new(640, 480),
                            Format::Argb8888,
                            &vec![0x20; 640 * 480 * 4],
                        )
                        .unwrap();
                }
                Some(Event::Configure {
                    surface: 2, serial, ..
                }) if shown => {
                    if let Some(held) = &held {
                        if held.recv().is_err() {
                            return;
                        }
                    }
                    shell
                        .send(&Request::ClearWindowViews { surface: 2 })
                        .unwrap();
                    shell
                        .send(&Request::PlaceWindowView {
                            surface: 2,
                            window: window.unwrap(),
                            rect: CARD,
                            clip: CARD,
                            radius: 0,
                            opacity: 255,
                        })
                        .unwrap();
                    shell
                        .send(&Request::MoveWindowsToViews {
                            surface: 2,
                            moved: true,
                        })
                        .unwrap();
                    shell
                        .present_format(
                            2,
                            serial,
                            Size::new(640, 480),
                            Format::Argb8888,
                            &vec![0; 640 * 480 * 4],
                        )
                        .unwrap();
                }
                _ => {}
            }
        }
    });
    let mut session = Session {
        compositor,
        listener,
        told,
        _directory: TempDir(directory),
    };
    session.until(600, "the window and the dock", |c| {
        c.scene.render_list().len() == 2
    });
    session.at_ms(1000);
    session.compositor.scene.pointer_motion(320, 240);
    session
}

/// App Exposé follows the fingers down: at a quarter, half and three quarters of the swipe's
/// distance (300 logical pixels), the window is drawn exactly that share of the way from where it
/// is to its view, in the frame after the motion. Let go at three quarters, slowly, it goes on to
/// its view on the overlay's spring, every frame further; the window never moved.
#[test]
fn the_window_follows_the_fingers_and_goes_on_from_where_they_let_it_go() {
    let mut s = session("follows");
    let window = s.window();
    s.swipe(GesturePhase::Begin, 0.0, 0);
    s.swipe(GesturePhase::Update, 75.0, 100);
    s.until(1100, "App Exposé arriving", app_expose);
    assert_eq!(s.told(), [ShortcutIntent::Open]);
    // App Exposé's first frame has the window where it is, and it catches up with the fingers
    // from there: a quarter of the way once caught up.
    let flights = s.compositor.take_flights();
    assert_eq!(flights[0].path[0], window);
    s.at_ms(1400);
    assert_eq!(s.flown(), (between(window, VIEW, 0.25), true));
    for (step, share) in [(2u64, 0.5), (3, 0.75)] {
        s.swipe(GesturePhase::Update, 75.0, 400 + step * 100);
        // The frame after the motion, whatever the clock: the fingers', not a spring's.
        s.at_ms(1400 + step * 100);
        assert_eq!(
            s.flown(),
            (between(window, VIEW, share), true),
            "at {share}"
        );
    }
    // Held still, nothing moves.
    s.at_ms(1750);
    s.at_ms(1800);
    assert!(s.compositor.take_flights().is_empty());
    s.swipe(GesturePhase::End, 0.0, 630);
    assert!(s.told().is_empty());
    s.at_ms(1800);
    let mut drawn = vec![s.flown().0];
    for t in (1816..=2400).step_by(16) {
        s.at_ms(t);
        if let Some(flight) = s.compositor.take_flights().last() {
            drawn.push(*flight.path.last().unwrap());
        }
    }
    assert!(drawn.windows(2).all(|w| w[1].x <= w[0].x), "{drawn:?}");
    assert_eq!(*drawn.last().unwrap(), VIEW, "{drawn:?}");
    assert_eq!(s.window(), window, "the window never moved");
}

/// A fast, short flick (a sixth of the way in 30 ms) goes through, ahead of a spring from rest;
/// let go short of halfway, slowly, it goes back and the shell is told to close it; the swipe
/// back up closes it as the fingers go, the shell told only once they let it go.
#[test]
fn a_flick_goes_through_a_slow_release_short_goes_back_and_the_swipe_back_closes() {
    let mut s = session("flick");
    let window = s.window();
    // The flick.
    s.swipe(GesturePhase::Begin, 0.0, 0);
    for i in 1..=3 {
        s.swipe(GesturePhase::Update, 17.0, i * 10);
    }
    s.until(1100, "App Exposé arriving", app_expose);
    s.swipe(GesturePhase::End, 0.0, 30);
    assert_eq!(s.told(), [ShortcutIntent::Open]);
    s.at_ms(1100);
    s.compositor.take_flights();
    s.at_ms(1200);
    let (at, _) = s.flown();
    // Further along in 100 ms than the same spring from rest at a sixth of the way gets.
    let rest = oxyos_compositor::motion::Released::new(
        oxyos_compositor::motion::OVERLAY_ARRIVES,
        51.0 / 300.0,
        0.0,
        1.0,
    );
    let share = f64::from(window.x - at.x) / f64::from(window.x - VIEW.x);
    assert!(share > rest.at(Duration::from_millis(100)), "{share}");
    s.at_ms(2000);
    assert!(s.shown());
    assert!(s.told().is_empty());
    // The swipe back, up: as it goes, nothing told; a third of the way closed, the window a third
    // of the way home.
    s.swipe(GesturePhase::Begin, 0.0, 3000);
    s.swipe(GesturePhase::Update, -100.0, 3100);
    s.at_ms(3100);
    assert_eq!(s.flown().0, between(window, VIEW, 2.0 / 3.0));
    // On past halfway, slowly, and let go: closed, the shell told, the window flying home from
    // where the fingers left it, and drawn there.
    s.swipe(GesturePhase::Update, -100.0, 3300);
    s.swipe(GesturePhase::End, 0.0, 3400);
    s.until(3400, "App Exposé gone", |c| !app_expose(c));
    assert_eq!(s.told(), [ShortcutIntent::Close]);
    let mut path = Vec::new();
    for t in (3416..=4200).step_by(16) {
        s.at_ms(t);
        for flight in s.compositor.take_flights() {
            assert!(!flight.arriving, "{flight:?}");
            path.extend(flight.path);
        }
    }
    assert_eq!(path[0], between(window, VIEW, 1.0 / 3.0));
    assert!(path.windows(2).all(|w| w[1].x >= w[0].x), "{path:?}");
    // Home: drawn where it is.
    let key = *s
        .compositor
        .windows_drawn
        .keys()
        .find(|key| key.id == 1 && key.client == 1)
        .unwrap();
    assert_eq!(s.compositor.windows_drawn[&key], window);
    // Down again, a quarter, slowly let go: back, the shell told to close what it opened.
    s.swipe(GesturePhase::Begin, 0.0, 5000);
    s.swipe(GesturePhase::Update, 75.0, 5100);
    s.until(5100, "App Exposé arriving", app_expose);
    s.swipe(GesturePhase::End, 0.0, 5300);
    s.until(5300, "App Exposé gone", |c| !app_expose(c));
    assert_eq!(s.told(), [ShortcutIntent::Open, ShortcutIntent::Close]);
    s.at_ms(6000);
    assert_eq!(s.window(), window, "the window never moved");
}

/// Whether the shell's surface `id` is shown.
fn shows(c: &Compositor, id: u32) -> bool {
    c.scene.render_list().iter().any(
        |item| matches!(*item, RenderItem::Surface { key, .. } if key.client != 1 && key.id == id),
    )
}

/// The shell draws App Exposé's backdrop and the overlay its window is moved into one after the
/// other, and on a loaded machine the second comes long after the first: here 150 ms after a fast,
/// short flick's fingers lifted. Nothing of App Exposé moves until the shell draws all of it; then
/// the window sets off from exactly where it is, with the backdrop, at the fingers' speed, and
/// flies on to its view: never from part of the way, however long the shell took.
#[test]
fn a_flick_whose_windows_surface_the_shell_draws_late_flies_from_where_the_window_is() {
    let (draw, held) = mpsc::channel();
    let mut s = session_with("late", Some(held));
    let window = s.window();
    // 45 pixels in 25 ms, lifted at once.
    s.swipe(GesturePhase::Begin, 0.0, 0);
    for i in 1..=3 {
        s.swipe(GesturePhase::Update, 15.0, i * 25 / 3);
    }
    s.swipe(GesturePhase::End, 0.0, 25);
    s.until(1100, "App Exposé's backdrop", |c| shows(c, 3));
    assert!(!shows(&s.compositor, 2));
    assert_eq!(s.told(), [ShortcutIntent::Open]);
    for t in (1116..=1250).step_by(16) {
        s.at_ms(t);
    }
    assert!(s.compositor.take_flights().is_empty());
    assert_eq!(
        s.compositor
            .transitions
            .drives
            .overlay
            .unwrap()
            .openness(Duration::from_millis(101_250)),
        0.0,
        "nothing of App Exposé moves before the shell draws all of it"
    );
    draw.send(()).unwrap();
    s.until(1266, "App Exposé's window overlay", |c| shows(c, 2));
    let mut path = Vec::new();
    for t in (1266..=2400).step_by(16) {
        s.at_ms(t);
        for flight in s.compositor.take_flights() {
            assert!(flight.arriving, "{flight:?}");
            path.extend(flight.path);
        }
    }
    assert_eq!(path[0], window, "{path:?}");
    assert!(path.windows(2).all(|w| w[1].x <= w[0].x), "{path:?}");
    assert!(path.len() > 2, "{path:?}");
    assert_eq!(*path.last().unwrap(), VIEW, "{path:?}");
    assert_eq!(s.window(), window, "the window never moved");
}
