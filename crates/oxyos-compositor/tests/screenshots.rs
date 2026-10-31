//! Screenshots (docs/adr/0024-system-shortcuts.mdx, decision 10), through real sockets: a capture
//! of every display is each screen's composed frame pixel for pixel, at its own pixels (1 and
//! 1.25); an area is the pixels its rectangle covers; a window comes with its shadow over nothing;
//! secure regions are black and what the shell leaves out is not there; locked, nothing is
//! captured; and the session's writer saves a PNG of the frame and tells the shell. On the CPU
//! reference (the GPU composes the same frames, ADR 0004).

use oxyos_client::Client;
use oxyos_compositor::screenshots::{Capture, Language, Shot, Writer};
use oxyos_compositor::Compositor;
use oxyos_compositor_core::{Output, Scene};
use oxyos_ipc::Listener;
use oxyos_protocol::{
    CaptureDestination, CaptureKind, CaptureMode, Event, Format, PanelEdge, Rect, Request,
    ShellKind, Size,
};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The primary at 1, and right of it 400 × 300 logical pixels at 1.25 (500 × 375 pixels).
const LEFT: Rect = Rect::new(0, 0, 640, 480);
const RIGHT: Rect = Rect::new(640, 0, 400, 300);
const SUPER: u32 = 125;
const SHIFT: u32 = 42;
const KEY_3: u32 = 4;
const KEY_4: u32 = 5;
const SPACE: u32 = 57;
/// The shell's background on the primary, and the square its excluded overlay shows.
const BACKGROUND: u32 = 0x0020_3040;
const GREEN: u32 = 0x0000_c000;
const SQUARE: Rect = Rect::new(20, 300, 100, 100);
/// The left window's secure region, surface-local.
const SECURE: Rect = Rect::new(10, 10, 40, 20);

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A window's pixels at `extent`: a gradient no two nearby pixels share (B, G, R, X).
fn pattern(extent: Size, seed: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((extent.width * extent.height * 4) as usize);
    for y in 0..extent.height {
        for x in 0..extent.width {
            let colour = (((x * 7 + seed) % 256) << 16) | (((y * 13) % 256) << 8) | 0x40;
            pixels.extend((0xff00_0000 | colour).to_le_bytes());
        }
    }
    pixels
}

/// An application with one 100 × 60 window drawn as `pattern` at each configure's scale, its
/// secure region `secure` sent with every frame.
fn application(socket: std::path::PathBuf, seed: u32, secure: Option<Rect>) {
    std::thread::spawn(move || {
        let mut client = Client::connect(&socket).unwrap();
        client
            .send(&Request::CreateToplevel {
                surface: 1,
                size: Size::new(100, 60),
            })
            .unwrap();
        loop {
            match client.next_event() {
                Ok(Some(Event::Configure {
                    serial,
                    rect,
                    scale_120,
                    ..
                })) => {
                    let extent = |length: u32| (length * scale_120).div_ceil(120);
                    let extent = Size::new(extent(rect.width), extent(rect.height));
                    if let Some(rect) = secure {
                        client
                            .send(&Request::ClearSecureRegions { surface: 1 })
                            .unwrap();
                        client
                            .send(&Request::AddSecureRegion { surface: 1, rect })
                            .unwrap();
                    }
                    client
                        .present(1, serial, extent, &pattern(extent, seed))
                        .unwrap();
                }
                Ok(Some(_)) => {}
                Ok(None) => std::thread::sleep(Duration::from_millis(2)),
                Err(_) => return,
            }
        }
    });
}

/// The shell: its background on the primary and an overlay it leaves out of captures (a green
/// square), forwarding what it is told of screenshots and windows, and sending what it is given.
fn shell(
    socket: std::path::PathBuf,
    told: mpsc::Sender<Event>,
    send: mpsc::Receiver<Request>,
) {
    std::thread::spawn(move || {
        let mut shell = Client::connect(&socket).unwrap();
        for (surface, kind) in [(1, ShellKind::Background), (2, ShellKind::Overlay)] {
            shell
                .send(&Request::CreateShellSurface {
                    surface,
                    kind,
                    edge: PanelEdge::None,
                    output: 1,
                    size: Size::new(LEFT.width, LEFT.height),
                    exclusive: 0,
                })
                .unwrap();
        }
        shell
            .send(&Request::ExcludeFromCapture {
                surface: 2,
                excluded: true,
            })
            .unwrap();
        loop {
            match shell.poll() {
                Ok(oxyos_client::Poll::Event(Event::Configure { surface, serial, .. })) => {
                    let (format, pixels): (Format, Vec<u8>) = if surface == 1 {
                        let colour = (0xff00_0000 | BACKGROUND).to_le_bytes();
                        (Format::Xrgb8888, colour.repeat((LEFT.width * LEFT.height) as usize))
                    } else {
                        let mut pixels = vec![0; (LEFT.width * LEFT.height * 4) as usize];
                        for y in SQUARE.y..SQUARE.y + SQUARE.height as i32 {
                            for x in SQUARE.x..SQUARE.x + SQUARE.width as i32 {
                                let at = ((y as u32 * LEFT.width + x as u32) * 4) as usize;
                                pixels[at..at + 4]
                                    .copy_from_slice(&(0xff00_0000 | GREEN).to_le_bytes());
                            }
                        }
                        (Format::Argb8888, pixels)
                    };
                    shell
                        .present_format(
                            surface,
                            serial,
                            Size::new(LEFT.width, LEFT.height),
                            format,
                            &pixels,
                        )
                        .unwrap();
                }
                Ok(oxyos_client::Poll::Event(
                    event @ (Event::CaptureMode { .. }
                    | Event::ScreenshotSaved { .. }
                    | Event::WindowState { .. }),
                )) => {
                    if told.send(event).is_err() {
                        return;
                    }
                }
                Ok(oxyos_client::Poll::Event(_)) => {}
                Ok(oxyos_client::Poll::Empty) => match send.try_recv() {
                    Ok(request) => shell.send(&request).unwrap(),
                    Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(2)),
                    Err(mpsc::TryRecvError::Disconnected) => return,
                },
                _ => return,
            }
        }
    });
}

struct Session {
    compositor: Compositor,
    listener: Listener,
    told: mpsc::Receiver<Event>,
    send: mpsc::Sender<Request>,
    directory: TempDir,
}

impl Session {
    /// Serves until `done` holds, then until every transition has ended (on the test's clock).
    fn until(&mut self, what: &str, done: impl Fn(&Compositor) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(&self.compositor) {
            assert!(Instant::now() < deadline, "{what} not in time");
            self.compositor.serve(&self.listener).unwrap();
        }
        let clock = self.compositor.clock.expect("the test's clock");
        self.compositor.clock = Some(clock + Duration::from_secs(1));
        while !self.compositor.transitions.is_empty() {
            assert!(Instant::now() < deadline, "{what}'s transition not in time");
            self.compositor.serve(&self.listener).unwrap();
        }
        self.compositor.serve(&self.listener).unwrap();
    }

    /// Super+Shift with `key`, then a turn: what it asked for is composed with that turn's frame.
    fn shortcut(&mut self, key: u32) {
        for code in [SUPER, SHIFT, key] {
            self.compositor.scene.keyboard(code, true, None);
        }
        for code in [key, SHIFT, SUPER] {
            self.compositor.scene.keyboard(code, false, None);
        }
        self.compositor.serve(&self.listener).unwrap();
    }

    /// Serves until the shell was told something `matches` picks.
    fn told<T>(&mut self, what: &str, matches: impl Fn(&Event) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            while let Ok(event) = self.told.try_recv() {
                if let Some(found) = matches(&event) {
                    return found;
                }
            }
            assert!(Instant::now() < deadline, "{what} not told in time");
            self.compositor.serve(&self.listener).unwrap();
        }
    }

    /// The shell sends `request`; served until the compositor has a shot.
    fn shell_captures(&mut self, request: Request) -> Shot {
        self.send.send(request).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.compositor.shots.is_empty() {
            assert!(Instant::now() < deadline, "the capture not in time");
            self.compositor.serve(&self.listener).unwrap();
        }
        self.compositor.shots.remove(0)
    }

    /// Super+Shift+4, and the serial of the interface the shell is told to show.
    fn crosshair(&mut self) -> u64 {
        self.shortcut(KEY_4);
        self.told("the crosshair", |event| match *event {
            Event::CaptureMode {
                mode: CaptureMode::Selection,
                serial,
                ..
            } => Some(serial),
            _ => None,
        })
    }

    fn rect(&self, client: u32) -> Rect {
        self.compositor.surface_rect(client, 1).unwrap()
    }
}

/// Two outputs; a window on each, the right one at 1.25, the left one with a secure region; the
/// shell; all shown and settled.
fn session(name: &str) -> Session {
    let directory =
        std::env::temp_dir().join(format!("oxyos-screenshots-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("compositor");
    let shell_socket = directory.join("shell");
    let listener = Listener::bind(&socket).unwrap();
    let mut scene = Scene::new();
    for (id, rect, scale_120) in [(1, LEFT, 120), (2, RIGHT, 150)] {
        scene
            .add_output(Output {
                id,
                rect,
                scale_120,
                refresh_mhz: 60_000,
            })
            .unwrap();
    }
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&directory).unwrap());
    let mut compositor = Compositor::new(scene, uid);
    compositor.shell_listener = Some(Listener::bind(&shell_socket).unwrap());
    compositor.clock = Some(Duration::from_secs(100));
    let (told_tx, told) = mpsc::channel();
    let (send, send_rx) = mpsc::channel();
    shell(shell_socket, told_tx, send_rx);
    let mut s = Session {
        compositor,
        listener,
        told,
        send,
        directory: TempDir(directory),
    };
    s.until("the shell", |c| c.scene.render_list().len() >= 2);
    // Each window opens on the output under the pointer: the shell's client is numbered from the
    // top, the applications from 1.
    s.compositor.scene.pointer_motion(300, 100);
    application(socket.clone(), 0, Some(SECURE));
    s.until("the left window", |c| c.surface_rect(1, 1).is_some());
    s.compositor.scene.pointer_motion(900, 100);
    application(socket, 90, None);
    s.until("the right window", |c| c.surface_rect(2, 1).is_some());
    s.compositor.scene.pointer_motion(5, 5);
    s.until("settled", |_| true);
    s
}

/// The pixels of `screen`'s frame within `x0..x1`, `y0..y1`.
fn crop(c: &Compositor, screen: usize, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> Vec<u32> {
    let screen = &c.screens[screen];
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .map(|(x, y)| screen.pixel(x, y))
        .collect()
}

/// A logical offset on the right screen in its pixels, rounded as the screen places it.
fn on_right(offset: i32) -> u32 {
    ((offset * 150 * 2 + 120) / 240) as u32
}

fn pixel(capture: &Capture, x: u32, y: u32) -> u32 {
    capture.pixels[(y * capture.width + x) as usize]
}

#[test]
fn every_display_is_captured_as_its_screen_composed_it_at_its_own_pixels() {
    let mut s = session("screens");
    assert!(RIGHT.contains(s.rect(2)), "{:?}", s.rect(2));
    s.shortcut(KEY_3);
    assert_eq!(s.compositor.shots.len(), 1);
    let shot = s.compositor.shots.remove(0);
    assert_eq!(
        (shot.kind, shot.destination),
        (CaptureKind::Screens, CaptureDestination::Desktop)
    );
    let sizes: Vec<_> = shot
        .captures
        .iter()
        .map(|c| (c.output, c.width, c.height, c.scale_120))
        .collect();
    assert_eq!(sizes, [(1, 640, 480, 120), (2, 500, 375, 150)]);
    // The right screen at 1.25: its real pixels, the window's 100 × 60 drawn as its 125 × 75.
    let right = &shot.captures[1];
    let window = s.rect(2);
    let (wx, wy) = (
        on_right(window.x - RIGHT.x),
        on_right(window.y - RIGHT.y),
    );
    let drawn = pattern(Size::new(125, 75), 90);
    let at = |x: u32, y: u32| {
        let i = ((y * 125 + x) * 4) as usize;
        u32::from_le_bytes([drawn[i], drawn[i + 1], drawn[i + 2], 0])
    };
    for (x, y) in [(62, 0), (0, 37), (124, 37), (62, 74), (40, 30)] {
        assert_eq!(pixel(right, wx + x, wy + y), at(x, y), "({x}, {y})");
    }
    // Pixel for pixel what the screens show, but what a capture leaves out: the left screen's
    // square (the shell's excluded overlay) shows what is beneath it, and the left window's
    // secure region is black.
    let left_window = s.rect(1);
    let secure = Rect::new(
        left_window.x + SECURE.x,
        left_window.y + SECURE.y,
        SECURE.width,
        SECURE.height,
    );
    for (index, capture) in shot.captures.iter().enumerate() {
        let screen = &s.compositor.screens[index];
        assert_eq!(capture.alpha, None);
        let mut differing = 0;
        for y in 0..capture.height {
            for x in 0..capture.width {
                let (gx, gy) = (x as i32, y as i32);
                let expected = if index == 0 && secure.contains_point(gx, gy) {
                    0
                } else if index == 0 && SQUARE.contains_point(gx, gy) {
                    BACKGROUND
                } else {
                    screen.pixel(x, y)
                };
                differing += usize::from(pixel(capture, x, y) != expected);
            }
        }
        assert_eq!(differing, 0, "screen {index}");
    }
    // The screen itself shows them: the square and the field's pixels.
    assert_eq!(s.compositor.pixel(SQUARE.x + 5, SQUARE.y + 5), GREEN);
    assert_ne!(s.compositor.pixel(secure.x + 1, secure.y + 1), 0);
}

#[test]
fn an_area_at_1_25_is_the_pixels_its_rectangle_covers() {
    let mut s = session("area");
    let serial = s.crosshair();
    let shot = s.shell_captures(Request::Capture {
        serial,
        kind: CaptureKind::Area,
        output: 2,
        rect: Rect::new(700, 50, 120, 80),
        window: 0,
        destination: CaptureDestination::Documents,
    });
    assert_eq!(
        (shot.kind, shot.destination),
        (CaptureKind::Area, CaptureDestination::Documents)
    );
    let capture = &shot.captures[0];
    // (60, 50) to (180, 130) logical on the screen are pixels (75, 63) to (225, 163), its edges
    // rounded as the screen places them.
    assert_eq!((capture.output, capture.width, capture.height), (2, 150, 100));
    assert_eq!(capture.pixels, crop(&s.compositor, 1, (75, 63, 225, 163)));
    // The crosshair ended with it.
    s.told("the end", |event| {
        matches!(event, Event::CaptureMode { mode: CaptureMode::Ended, .. }).then_some(())
    });
    assert!(!s.compositor.scene.capturing());
}

#[test]
fn a_window_is_captured_with_its_shadow_over_nothing() {
    let mut s = session("window");
    let number = s.told("the right window's number", |event| match *event {
        Event::WindowState { window, .. } if window > 1 => Some(window),
        _ => None,
    });
    let serial = s.crosshair();
    s.compositor.scene.keyboard(SPACE, true, None);
    s.compositor.scene.keyboard(SPACE, false, None);
    let shot = s.shell_captures(Request::Capture {
        serial,
        kind: CaptureKind::Window,
        output: 0,
        rect: Rect::new(0, 0, 0, 0),
        window: number,
        destination: CaptureDestination::Desktop,
    });
    assert_eq!(shot.kind, CaptureKind::Window);
    let capture = &shot.captures[0];
    let alpha = capture.alpha.as_ref().expect("a window's alpha");
    assert_eq!((capture.output, capture.scale_120), (2, 150));
    // The window is where its capture is opaque: 125 × 75 pixels, its shadow around it.
    let opaque: Vec<(u32, u32)> = (0..capture.height)
        .flat_map(|y| (0..capture.width).map(move |x| (x, y)))
        .filter(|&(x, y)| alpha[(y * capture.width + x) as usize] == 255)
        .collect();
    let left = opaque.iter().map(|p| p.0).min().unwrap();
    let top = opaque.iter().map(|p| p.1).min().unwrap();
    let right = opaque.iter().map(|p| p.0).max().unwrap();
    let bottom = opaque.iter().map(|p| p.1).max().unwrap();
    assert_eq!((right - left + 1, bottom - top + 1), (125, 75));
    assert!(left > 0 && top > 0 && right < capture.width - 1 && bottom < capture.height - 1);
    // Inside its corners, its own pixels, exactly the screen's.
    let window = s.rect(2);
    let (wx, wy) = (
        on_right(window.x - RIGHT.x),
        on_right(window.y - RIGHT.y),
    );
    for y in 16..75 - 16 {
        for x in 16..125 - 16 {
            assert_eq!(
                pixel(capture, left + x, top + y),
                s.compositor.screens[1].pixel(wx + x, wy + y),
                "({x}, {y})"
            );
        }
    }
    // Its shadow fades to nothing: some pixels between, none at the capture's corners.
    assert!(alpha.iter().any(|&a| a > 0 && a < 255));
    for (x, y) in [
        (0, 0),
        (capture.width - 1, 0),
        (0, capture.height - 1),
        (capture.width - 1, capture.height - 1),
    ] {
        assert_eq!(alpha[(y * capture.width + x) as usize], 0, "({x}, {y})");
    }
}

#[test]
fn locked_nothing_is_captured() {
    let mut s = session("locked");
    // Asked for as the session locks, before the frame composes it: nothing.
    for code in [SUPER, SHIFT, KEY_3] {
        s.compositor.scene.keyboard(code, true, None);
    }
    for code in [KEY_3, SHIFT, SUPER] {
        s.compositor.scene.keyboard(code, false, None);
    }
    s.compositor.scene.lock();
    for _ in 0..5 {
        s.compositor.serve(&s.listener).unwrap();
    }
    // Locked, the shortcuts do nothing.
    s.shortcut(KEY_3);
    s.shortcut(KEY_4);
    for _ in 0..5 {
        s.compositor.serve(&s.listener).unwrap();
    }
    assert!(s.compositor.shots.is_empty());
    assert!(!s.compositor.scene.capturing());
    while let Ok(event) = s.told.try_recv() {
        assert!(
            !matches!(event, Event::CaptureMode { .. }),
            "locked, no interface: {event:?}"
        );
    }
}

#[test]
fn the_session_writes_each_displays_png_and_tells_the_shell() {
    let mut s = session("writer");
    let desktop = s.directory.0.join("home/Escritorio");
    let folder = desktop.clone();
    s.compositor.screenshots = Some(
        Writer::spawn(
            Box::new(move |_| folder.clone()),
            Language::Spanish,
            None,
        )
        .unwrap(),
    );
    s.shortcut(KEY_3);
    // Each screenshot's path in chunks, by its number: (output, width, height, path, whole).
    let mut told: std::collections::BTreeMap<u32, (u32, u32, u32, String, bool)> =
        Default::default();
    while told.values().filter(|t| t.4).count() < 2 {
        let (screenshot, output, width, height, last, text) =
            s.told("a screenshot saved", |event| match *event {
                Event::ScreenshotSaved {
                    screenshot,
                    output,
                    width,
                    height,
                    last,
                    text,
                    ..
                } => Some((screenshot, output, width, height, last, text.as_str().to_owned())),
                _ => None,
            });
        let entry = told
            .entry(screenshot)
            .or_insert((output, width, height, String::new(), false));
        entry.3.push_str(&text);
        entry.4 = last;
    }
    let mut saved: Vec<(u32, u32, u32, String)> = told
        .into_values()
        .map(|(output, width, height, path, _)| (output, width, height, path))
        .collect();
    saved.sort();
    let names: Vec<String> = saved
        .iter()
        .map(|(_, _, _, path)| {
            std::path::Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(names[0].starts_with("Captura de pantalla ") && names[0].ends_with(".png"));
    assert!(names[0].contains(" a las "));
    assert_eq!(names[1], names[0].replace(".png", " (2).png"));
    assert_eq!((saved[0].1, saved[0].2), (640, 480));
    assert_eq!((saved[1].1, saved[1].2), (500, 375));
    // The right display's file holds its frame: RGB, row by row.
    let file = std::fs::read(&saved[1].3).unwrap();
    assert!(std::path::Path::new(&saved[1].3).starts_with(&desktop));
    let mut reader = png::Decoder::new(std::io::Cursor::new(file))
        .read_info()
        .unwrap();
    let mut data = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut data).unwrap();
    assert_eq!((info.width, info.height, info.color_type), (500, 375, png::ColorType::Rgb));
    let screen = &s.compositor.screens[1];
    for (index, rgb) in data[..info.buffer_size()].chunks_exact(3).enumerate() {
        let (x, y) = (index as u32 % 500, index as u32 / 500);
        let expected = screen.pixel(x, y);
        let got = (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
        assert_eq!(got, expected, "({x}, {y})");
    }
}
