//! Screenshots (ADR 0024, decision 10): Super+Shift+3, 4 and 5, the interface's keys, the window
//! under the pointer, the shell's captures and what is never captured.

use super::*;
use crate::scene::CaptureRequest;
use oxyos_protocol::{CaptureDestination, CaptureKind, CaptureMode};

const SHIFT: u32 = 42;
const KEY_3: u32 = 4;
const KEY_4: u32 = 5;
const KEY_5: u32 = 6;
const ENTER: u32 = 28;
const KEY_M: u32 = 50;
/// The shell of a second output.
const SHELL_2: ClientId = 9;

/// The shell's dock on output 1, so it is that output's shell.
fn dock(h: &mut Harness) {
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Bottom,
            output: 1,
            size: Size::new(1, 48),
            exclusive: 48,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
}

/// A second output right of the first, 1280×720 at 1.25, with a shell of its own.
fn second_output(h: &mut Harness) {
    h.scene
        .add_output(Output {
            id: 2,
            rect: Rect::new(1920, 0, 1280, 720),
            scale_120: 150,
            refresh_mhz: 60_000,
        })
        .unwrap();
    h.scene
        .connect(
            SHELL_2,
            Grants {
                shell: true,
                shortcuts: true,
                ..Grants::default()
            },
        )
        .unwrap();
    h.request(
        SHELL_2,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Background,
            edge: PanelEdge::None,
            output: 2,
            size: Size::new(1, 1),
            exclusive: 0,
        },
    )
    .unwrap();
    h.present(key(SHELL_2, 1));
}

/// Keys pressed in order and released in reverse, through the keyboard's path.
fn keys(h: &mut Harness, codes: &[u32]) {
    for code in codes {
        h.scene.keyboard(*code, true, None);
    }
    for code in codes.iter().rev() {
        h.scene.keyboard(*code, false, None);
    }
}

/// Super+Shift with `code`.
fn shot(h: &mut Harness, code: u32) {
    keys(h, &[SUPER, SHIFT, code]);
}

fn modes_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<(CaptureMode, u64, u32)> {
    to(events, client)
        .iter()
        .filter_map(|event| match *event {
            Event::CaptureMode {
                mode,
                serial,
                output,
            } => Some((mode, serial, output)),
            _ => None,
        })
        .collect()
}

fn targets_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<(u32, Rect)> {
    to(events, client)
        .iter()
        .filter_map(|event| match *event {
            Event::CaptureTarget { window, rect } => Some((window, rect)),
            _ => None,
        })
        .collect()
}

/// Everything sent to `client` that is input from the keyboard.
fn typed_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<Event> {
    to(events, client)
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                Event::Key { .. } | Event::Text { .. } | Event::Preedit { .. }
            )
        })
        .collect()
}

/// The serial of the interface the shell was told to show.
fn interface_serial(events: &[(ClientId, Event)], client: ClientId) -> u64 {
    modes_to(events, client).first().expect("an interface").1
}

fn capture(serial: u64, kind: CaptureKind, output: u32, rect: Rect, window: u32) -> Request {
    Request::Capture {
        serial,
        kind,
        output,
        rect,
        window,
        destination: CaptureDestination::Desktop,
    }
}

#[test]
fn super_shift_3_captures_every_display_and_its_keys_reach_nobody() {
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    shot(&mut h, KEY_3);
    let events = logged(&mut h);
    assert_eq!(typed_to(&events, APP), []);
    assert_eq!(modes_to(&events, SHELL), [], "no interface for the whole screen");
    assert_eq!(
        h.scene.take_captures(),
        [CaptureRequest::Screens {
            destination: CaptureDestination::Desktop
        }]
    );
    assert_eq!(h.scene.take_captures(), []);
    assert_eq!(h.scene.focus(), Some(window));
    // Without Shift, Super+3 is nothing (and still nobody's).
    keys(&mut h, &[SUPER, KEY_3]);
    assert_eq!(typed_to(&logged(&mut h), APP), []);
    assert_eq!(h.scene.take_captures(), []);
}

#[test]
fn super_shift_4_opens_the_crosshair_on_every_display_and_its_keys_are_the_systems() {
    let mut h = Harness::new();
    dock(&mut h);
    second_output(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    shot(&mut h, KEY_4);
    let events = logged(&mut h);
    let serial = interface_serial(&events, SHELL);
    assert_eq!(
        modes_to(&events, SHELL),
        [(CaptureMode::Selection, serial, 1)]
    );
    assert_eq!(
        modes_to(&events, SHELL_2),
        [(CaptureMode::Selection, serial, 1)],
        "every display's shell, with the same serial"
    );
    assert!(h.scene.capturing());
    assert_eq!(typed_to(&events, APP), []);
    // Typing does nothing and reaches nobody; the window keeps the keyboard focus.
    keys(&mut h, &[KEY_A]);
    keys(&mut h, &[SHIFT, KEY_B]);
    let events = logged(&mut h);
    assert_eq!(typed_to(&events, APP), []);
    assert_eq!(modes_to(&events, SHELL), []);
    assert_eq!(h.scene.focus(), Some(window));
    // Space: picking a window; again, the crosshair. Every shell is told.
    keys(&mut h, &[SPACE]);
    keys(&mut h, &[SPACE]);
    let events = logged(&mut h);
    for shell in [SHELL, SHELL_2] {
        assert_eq!(
            modes_to(&events, shell),
            [
                (CaptureMode::Window, serial, 1),
                (CaptureMode::Selection, serial, 1)
            ]
        );
    }
    assert_eq!(typed_to(&events, APP), []);
    // Escape ends it everywhere, and is nobody's either.
    keys(&mut h, &[ESCAPE]);
    let events = logged(&mut h);
    assert_eq!(modes_to(&events, SHELL), [(CaptureMode::Ended, serial, 1)]);
    assert_eq!(modes_to(&events, SHELL_2), [(CaptureMode::Ended, serial, 1)]);
    assert_eq!(typed_to(&events, APP), []);
    assert!(!h.scene.capturing());
    assert_eq!(h.scene.take_captures(), []);
    // Closed, keys are the application's again.
    keys(&mut h, &[KEY_A]);
    assert_eq!(
        keys_to(&logged(&mut h), APP),
        [(KEY_A, true), (KEY_A, false)]
    );
}

#[test]
fn the_shells_are_told_the_window_under_the_pointer() {
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    let rect = h.rect(window);
    let number = h.scene.window_number(window).unwrap();
    h.scene.pointer_motion(5, 5);
    logged(&mut h);
    shot(&mut h, KEY_4);
    let events = logged(&mut h);
    assert_eq!(targets_to(&events, SHELL), [(0, Rect::new(0, 0, 0, 0))]);
    // Onto the window, within it, and off it again: told each change once.
    h.scene.pointer_motion(rect.x + 10, rect.y + 10);
    h.scene.pointer_motion(rect.x + 20, rect.y + 30);
    h.scene.pointer_motion(5, 5);
    let events = logged(&mut h);
    assert_eq!(
        targets_to(&events, SHELL),
        [(number, rect), (0, Rect::new(0, 0, 0, 0))]
    );
    // Closed, nobody is told.
    keys(&mut h, &[ESCAPE]);
    h.scene.pointer_motion(rect.x + 10, rect.y + 10);
    assert_eq!(targets_to(&logged(&mut h), SHELL), []);
}

#[test]
fn the_shell_captures_what_it_was_told_to_show_and_nothing_else() {
    let mut h = Harness::new();
    dock(&mut h);
    second_output(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    let number = h.scene.window_number(window).unwrap();
    logged(&mut h);
    // Nothing open: a capture is ignored.
    h.request(
        SHELL,
        capture(1, CaptureKind::Screens, 0, Rect::new(0, 0, 0, 0), 0),
    )
    .unwrap();
    assert_eq!(h.scene.take_captures(), []);
    shot(&mut h, KEY_4);
    let serial = interface_serial(&logged(&mut h), SHELL);
    // An application may not; another serial is ignored and leaves it open.
    assert_eq!(
        h.request(
            APP,
            capture(serial, CaptureKind::Screens, 0, Rect::new(0, 0, 0, 0), 0)
        ),
        Err(ErrorCode::Unauthorized)
    );
    h.request(
        SHELL,
        capture(serial + 1, CaptureKind::Screens, 0, Rect::new(0, 0, 0, 0), 0),
    )
    .unwrap();
    assert_eq!(h.scene.take_captures(), []);
    assert!(h.scene.capturing());
    // A window that is no window, or a rectangle off its display: nothing, and it stays open.
    h.request(
        SHELL,
        capture(serial, CaptureKind::Window, 0, Rect::new(0, 0, 0, 0), 999),
    )
    .unwrap();
    h.request(
        SHELL,
        capture(serial, CaptureKind::Area, 1, Rect::new(3000, 0, 10, 10), 0),
    )
    .unwrap();
    assert_eq!(h.scene.take_captures(), []);
    assert!(h.scene.capturing());
    // The second display's shell drags across its edge: the part on its display, to Documents.
    h.request(
        SHELL_2,
        Request::Capture {
            serial,
            kind: CaptureKind::Area,
            output: 2,
            rect: Rect::new(1900, 100, 300, 200),
            window: 0,
            destination: CaptureDestination::Documents,
        },
    )
    .unwrap();
    assert_eq!(
        h.scene.take_captures(),
        [CaptureRequest::Area {
            output: 2,
            rect: Rect::new(1920, 100, 280, 200),
            destination: CaptureDestination::Documents
        }]
    );
    let events = logged(&mut h);
    assert_eq!(modes_to(&events, SHELL), [(CaptureMode::Ended, serial, 1)]);
    assert_eq!(
        modes_to(&events, SHELL_2),
        [(CaptureMode::Ended, serial, 1)]
    );
    // Its choice of folder is kept: Super+Shift+3 saves there too.
    shot(&mut h, KEY_3);
    assert_eq!(
        h.scene.take_captures(),
        [CaptureRequest::Screens {
            destination: CaptureDestination::Documents
        }]
    );
    // A window picked: by its number.
    shot(&mut h, KEY_4);
    let serial = interface_serial(&logged(&mut h), SHELL);
    keys(&mut h, &[SPACE]);
    h.request(
        SHELL,
        capture(serial, CaptureKind::Window, 0, Rect::new(0, 0, 0, 0), number),
    )
    .unwrap();
    assert_eq!(
        h.scene.take_captures(),
        [CaptureRequest::Window {
            window,
            destination: CaptureDestination::Desktop
        }]
    );
    assert!(!h.scene.capturing());
    // The serial of an interface that ended is no longer cited.
    h.request(
        SHELL,
        capture(serial, CaptureKind::Screens, 0, Rect::new(0, 0, 0, 0), 0),
    )
    .unwrap();
    assert_eq!(h.scene.take_captures(), []);
}

#[test]
fn super_shift_5_opens_the_toolbar_on_the_display_in_front() {
    let mut h = Harness::new();
    dock(&mut h);
    second_output(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    h.scene.move_window_to(window, (2000, 100));
    h.present(window);
    logged(&mut h);
    shot(&mut h, KEY_5);
    let events = logged(&mut h);
    let serial = interface_serial(&events, SHELL_2);
    assert_eq!(modes_to(&events, SHELL), []);
    assert_eq!(
        modes_to(&events, SHELL_2),
        [(CaptureMode::Toolbar, serial, 2)]
    );
    // Space does nothing in the toolbar; Return captures what it shows.
    keys(&mut h, &[SPACE]);
    keys(&mut h, &[ENTER]);
    let events = logged(&mut h);
    assert_eq!(
        modes_to(&events, SHELL_2),
        [(CaptureMode::Confirm, serial, 2)]
    );
    assert_eq!(typed_to(&events, APP), []);
    // Super's other shortcuts do nothing while it is open; Super+Shift+4 replaces it.
    keys(&mut h, &[SUPER, KEY_M]);
    assert!(h.scene.live(window).is_some_and(|node| !node.hidden));
    shot(&mut h, KEY_4);
    let events = logged(&mut h);
    let crosshair = interface_serial(&events, SHELL);
    assert_eq!(
        modes_to(&events, SHELL_2),
        [
            (CaptureMode::Ended, serial, 2),
            (CaptureMode::Selection, crosshair, 2)
        ]
    );
    // The shell closes it from its toolbar.
    h.request(SHELL, Request::EndCapture { serial: crosshair })
        .unwrap();
    let events = logged(&mut h);
    assert_eq!(
        modes_to(&events, SHELL),
        [(CaptureMode::Ended, crosshair, 2)]
    );
    assert!(!h.scene.capturing());
}

#[test]
fn locked_nothing_is_captured_and_the_lock_screen_receives_none_of_its_keys() {
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    // A capture asked for and not composed yet, and the interface open, as the session locks:
    // the capture is dropped and the interface ends.
    shot(&mut h, KEY_3);
    shot(&mut h, KEY_4);
    let serial = interface_serial(&logged(&mut h), SHELL);
    h.scene.lock();
    assert_eq!(
        modes_to(&logged(&mut h), SHELL),
        [(CaptureMode::Ended, serial, 1)]
    );
    assert_eq!(h.scene.take_captures(), []);
    let lock = h.lock_surface(1);
    assert_eq!(h.scene.focus(), Some(lock));
    logged(&mut h);
    for code in [KEY_3, KEY_4, KEY_5] {
        shot(&mut h, code);
    }
    let events = logged(&mut h);
    assert_eq!(typed_to(&events, LOCKER), []);
    assert_eq!(modes_to(&events, SHELL), []);
    assert_eq!(h.scene.take_captures(), []);
    assert!(!h.scene.capturing());
    // The lock screen is never in a capture.
    assert!(!h.scene.captured(lock));
    // The shell's old serial captures nothing.
    h.request(
        SHELL,
        capture(serial, CaptureKind::Screens, 0, Rect::new(0, 0, 0, 0), 0),
    )
    .unwrap();
    assert_eq!(h.scene.take_captures(), []);
}

#[test]
fn secure_regions_go_with_the_frame_they_were_sent_with() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let region = Rect::new(20, 40, 240, 28);
    assert_eq!(
        h.request(
            APP,
            Request::AddSecureRegion {
                surface: 1,
                rect: region
            }
        ),
        Err(ErrorCode::InvalidOrder),
        "a list begins with ClearSecureRegions"
    );
    h.request(APP, Request::ClearSecureRegions { surface: 1 })
        .unwrap();
    h.request(
        APP,
        Request::AddSecureRegion {
            surface: 1,
            rect: region,
        },
    )
    .unwrap();
    assert_eq!(h.scene.secure_regions(window), []);
    h.submit(window);
    assert_eq!(h.scene.secure_regions(window), [], "not before it presents");
    h.frame();
    assert_eq!(h.scene.secure_regions(window), [region]);
    // A frame without a new list keeps it; an empty list clears it with its frame.
    h.present(window);
    assert_eq!(h.scene.secure_regions(window), [region]);
    h.request(APP, Request::ClearSecureRegions { surface: 1 })
        .unwrap();
    h.present(window);
    assert_eq!(h.scene.secure_regions(window), []);
    // At most MAX_SECURE_REGIONS, and only on its own surfaces.
    h.request(APP, Request::ClearSecureRegions { surface: 1 })
        .unwrap();
    for _ in 0..oxyos_protocol::MAX_SECURE_REGIONS {
        h.request(
            APP,
            Request::AddSecureRegion {
                surface: 1,
                rect: region,
            },
        )
        .unwrap();
    }
    assert_eq!(
        h.request(
            APP,
            Request::AddSecureRegion {
                surface: 1,
                rect: region
            }
        ),
        Err(ErrorCode::QuotaExceeded)
    );
    assert_eq!(
        h.request(OTHER, Request::ClearSecureRegions { surface: 1 }),
        Err(ErrorCode::UnknownSurface)
    );
}

#[test]
fn only_the_shell_leaves_its_own_shell_surfaces_out_of_captures() {
    let mut h = Harness::new();
    dock(&mut h);
    let window = h.toplevel(APP, 1, 400, 300);
    assert_eq!(
        h.request(
            APP,
            Request::ExcludeFromCapture {
                surface: 1,
                excluded: true
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
    assert!(h.scene.captured(window));
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 2,
            kind: ShellKind::Overlay,
            edge: PanelEdge::None,
            output: 1,
            size: Size::new(1920, 1080),
            exclusive: 0,
        },
    )
    .unwrap();
    let overlay = key(SHELL, 2);
    h.present(overlay);
    h.request(
        SHELL,
        Request::ExcludeFromCapture {
            surface: 2,
            excluded: true,
        },
    )
    .unwrap();
    assert!(!h.scene.captured(overlay));
    assert!(h.scene.captured(key(SHELL, 1)), "its dock stays");
    h.request(
        SHELL,
        Request::ExcludeFromCapture {
            surface: 2,
            excluded: false,
        },
    )
    .unwrap();
    assert!(h.scene.captured(overlay));
}

#[test]
fn the_shell_opens_a_screenshot_citing_its_own_press() {
    let mut h = Harness::new();
    dock(&mut h);
    h.request(
        SHELL,
        Request::OpenScreenshot {
            screenshot: 3,
            serial: 12345,
        },
    )
    .unwrap();
    assert_eq!(h.scene.take_screenshot_opens(), [], "no such input");
    let press = h.press(10, 1070);
    h.scene.pointer_button(BUTTON, false);
    h.request(
        SHELL,
        Request::OpenScreenshot {
            screenshot: 3,
            serial: press,
        },
    )
    .unwrap();
    assert_eq!(h.scene.take_screenshot_opens(), [3]);
    assert_eq!(
        h.request(
            APP,
            Request::OpenScreenshot {
                screenshot: 3,
                serial: press
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
}

#[test]
fn a_saved_screenshot_is_told_to_the_shell_of_its_display() {
    let mut h = Harness::new();
    dock(&mut h);
    second_output(&mut h);
    logged(&mut h);
    let path = "/home/ada/Escritorio/Captura de pantalla 2026-10-31 a las 10.04.05 (2).png";
    h.scene.screenshot_saved(4, 2, (1600, 900), path);
    let events = logged(&mut h);
    assert!(to(&events, SHELL).is_empty());
    let chunks: Vec<_> = to(&events, SHELL_2)
        .into_iter()
        .filter_map(|event| match event {
            Event::ScreenshotSaved {
                screenshot,
                output,
                width,
                height,
                index,
                last,
                text,
            } => Some((screenshot, output, width, height, index, last, text)),
            _ => None,
        })
        .collect();
    assert_eq!(chunks.len(), path.len().div_ceil(32));
    assert!(chunks
        .iter()
        .enumerate()
        .all(|(i, c)| (c.0, c.1, c.2, c.3, usize::from(c.4)) == (4, 2, 1600, 900, i)));
    assert!(chunks.last().unwrap().5 && chunks.iter().filter(|c| c.5).count() == 1);
    let told: String = chunks.iter().map(|c| c.6.as_str()).collect();
    assert_eq!(told, path);
}
