//! Displays changed from Settings (docs/adr/0018-several-displays.mdx): an output given another
//! size and scale keeps its windows within it and its panels on its edges, the primary changes
//! with nothing moving, and Settings (its `settings` grant alone) is told each display and what
//! changes of it, and asks for changes the program judges.

use super::*;

const SETTINGS: ClientId = 6;

fn with_settings() -> Harness {
    let mut h = Harness::new();
    h.scene
        .connect(
            SETTINGS,
            Grants {
                settings: true,
                ..Grants::default()
            },
        )
        .unwrap();
    h
}

fn panel(h: &mut Harness, id: u32, height: u32) -> SurfaceKey {
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: id,
            kind: ShellKind::Panel,
            edge: PanelEdge::Top,
            output: 1,
            size: Size::new(1, height),
            exclusive: height,
        },
    )
    .unwrap();
    h.present(key(SHELL, id));
    key(SHELL, id)
}

/// The geometry (in the global space) and scale `surface` was last configured with.
fn configured(h: &Harness, surface: SurfaceKey) -> (Rect, u32) {
    let (_, _, scale) = h.configures[&surface];
    (h.scene.upcoming_rect(surface).expect("configured"), scale)
}

fn info(output: OutputId) -> DisplayInfo {
    DisplayInfo {
        output,
        name: "Built-in Display".into(),
        builtin: true,
        size_mm: (301, 188),
        modes: vec![
            ModeInfo {
                width: 1920,
                height: 1200,
                refresh_mhz: 60_000,
                preferred: true,
            },
            ModeInfo {
                width: 1280,
                height: 800,
                refresh_mhz: 59_910,
                preferred: false,
            },
        ],
        mode: 0,
        scales: vec![120, 150, 180, 210],
        recommended: 150,
    }
}

/// What Settings was told of displays since the log was cleared.
fn displays_told(h: &Harness) -> Vec<Event> {
    h.events_for(SETTINGS)
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                Event::DisplayName { .. }
                    | Event::DisplayMode { .. }
                    | Event::DisplayScale { .. }
                    | Event::Display { .. }
                    | Event::DisplayRemoved { .. }
                    | Event::DisplayConfirmation { .. }
            )
        })
        .collect()
}

#[test]
fn an_output_resized_keeps_its_windows_within_it_and_its_panels_on_its_edges() {
    let mut h = Harness::new();
    let bar = panel(&mut h, 1, 28);
    let window = h.toplevel(APP, 1, 400, 300);
    let big = h.toplevel(OTHER, 1, 1800, 1000);
    // Left near the output's bottom-right corner, as a person would leave it.
    h.scene.move_window_to(window, (1450, 700));
    h.present(window);
    let at = h.rect(window);
    assert_eq!(at, Rect::new(1450, 700, 400, 300));
    // The output becomes the ThinkPad's panel at 1.25: 1536x960 logical pixels.
    h.scene
        .set_output_geometry(1, Rect::new(0, 0, 1536, 960), 150)
        .unwrap();
    h.pump();
    // The top bar spans the new width, asked for at the new scale.
    assert_eq!(configured(&h, bar), (Rect::new(0, 0, 1536, 28), 150));
    // A window that still fits keeps its place, moved only to stay within the work area.
    let (rect, scale) = configured(&h, window);
    assert_eq!(scale, 150);
    let area = h.scene.work_area(1).unwrap();
    assert_eq!(area, Rect::new(0, 28, 1536, 932));
    assert!(area.contains(rect), "{rect:?}");
    assert_eq!((rect.width, rect.height), (400, 300));
    assert_eq!((rect.x, rect.y), (1536 - 400, 960 - 300));
    // One larger than the work area is shrunk to it.
    let (rect, _) = configured(&h, big);
    assert_eq!(rect, area);
    // Nothing changes when nothing does.
    h.clear_log();
    h.scene
        .set_output_geometry(1, Rect::new(0, 0, 1536, 960), 150)
        .unwrap();
    assert!(h
        .pump()
        .iter()
        .all(|(_, event)| !matches!(event, Event::Configure { .. })));
    // A scale outside the scene's range, or no such output, is refused.
    assert_eq!(
        h.scene
            .set_output_geometry(1, Rect::new(0, 0, 100, 100), 600),
        Err(ErrorCode::InvalidGeometry)
    );
    assert_eq!(
        h.scene
            .set_output_geometry(9, Rect::new(0, 0, 100, 100), 120),
        Err(ErrorCode::InvalidGeometry)
    );
}

#[test]
fn a_full_screen_window_fills_its_output_once_resized() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: true,
        },
    )
    .unwrap();
    h.present(window);
    h.scene
        .set_output_geometry(1, Rect::new(0, 0, 1280, 800), 180)
        .unwrap();
    h.pump();
    assert_eq!(configured(&h, window), (Rect::new(0, 0, 1280, 800), 180));
}

#[test]
fn the_primary_changes_with_nothing_moving() {
    let mut h = Harness::new();
    let right = Rect::new(1920, 0, 1280, 720);
    h.scene
        .add_output(Output {
            id: 2,
            rect: right,
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let window = h.toplevel(APP, 1, 400, 300);
    let before = h.rect(window);
    h.pump();
    h.clear_log();
    h.scene.set_primary_output(2).unwrap();
    h.pump();
    assert_eq!(h.scene.outputs()[0].id, 2);
    let primary: Vec<(u32, bool)> = h
        .events_for(SHELL)
        .into_iter()
        .filter_map(|event| match event {
            Event::Output {
                output, primary, ..
            } => Some((output, primary)),
            _ => None,
        })
        .collect();
    assert_eq!(primary, [(1, false), (2, true)]);
    assert_eq!(h.rect(window), before);
    assert_eq!(h.scene.outputs()[0].rect, right);
    assert_eq!(
        h.scene.set_primary_output(9),
        Err(ErrorCode::InvalidGeometry)
    );
}

#[test]
fn settings_is_told_each_display_and_then_only_what_changes() {
    let mut h = with_settings();
    h.scene.set_displays(vec![info(1)]);
    h.pump();
    let told = displays_told(&h);
    assert_eq!(
        told,
        [
            Event::DisplayName {
                output: 1,
                index: 0,
                last: true,
                text: PreeditText::new("Built-in Display").unwrap(),
            },
            Event::DisplayMode {
                output: 1,
                index: 0,
                width: 1920,
                height: 1200,
                refresh_mhz: 60_000,
                preferred: true,
            },
            Event::DisplayMode {
                output: 1,
                index: 1,
                width: 1280,
                height: 800,
                refresh_mhz: 59_910,
                preferred: false,
            },
            Event::DisplayScale {
                output: 1,
                index: 0,
                scale_120: 120,
                recommended: false,
            },
            Event::DisplayScale {
                output: 1,
                index: 1,
                scale_120: 150,
                recommended: true,
            },
            Event::DisplayScale {
                output: 1,
                index: 2,
                scale_120: 180,
                recommended: false,
            },
            Event::DisplayScale {
                output: 1,
                index: 3,
                scale_120: 210,
                recommended: false,
            },
            Event::Display {
                output: 1,
                rect: SCREEN,
                scale_120: 120,
                mode: 0,
                width_mm: 301,
                height_mm: 188,
                primary: true,
                builtin: true,
            },
        ]
    );
    // Nothing changed: nothing told.
    h.clear_log();
    h.pump();
    assert!(displays_told(&h).is_empty());
    // Its scale changes: the display alone is told again, its modes and choices are not.
    h.scene
        .set_output_geometry(1, Rect::new(0, 0, 1536, 864), 150)
        .unwrap();
    h.pump();
    assert_eq!(
        displays_told(&h),
        [Event::Display {
            output: 1,
            rect: Rect::new(0, 0, 1536, 864),
            scale_120: 150,
            mode: 0,
            width_mm: 301,
            height_mm: 188,
            primary: true,
            builtin: true,
        }]
    );
    // No one else is ever told.
    assert!(h.log.iter().all(|(client, event)| *client == SETTINGS
        || !matches!(event, Event::Display { .. } | Event::DisplayMode { .. })));
    // The output goes: so does its display.
    h.clear_log();
    h.scene.remove_output(1);
    h.pump();
    assert_eq!(displays_told(&h), [Event::DisplayRemoved { output: 1 }]);
}

#[test]
fn only_settings_asks_for_display_changes_and_a_mode_on_trial_is_told() {
    let mut h = with_settings();
    // An application without the grant is refused (and its connection closed).
    assert_eq!(
        h.scene.handle(
            APP,
            Request::SetDisplayScale {
                serial: 1,
                output: 1,
                scale_120: 150,
            },
        ),
        Err(ErrorCode::Unauthorized)
    );
    assert_eq!(
        h.scene.handle(
            SHELL,
            Request::SetPrimaryDisplay {
                serial: 1,
                output: 1,
            },
        ),
        Err(ErrorCode::Unauthorized)
    );
    for request in [
        Request::SetDisplayMode {
            serial: 2,
            output: 1,
            width: 1280,
            height: 800,
            refresh_mhz: 59_910,
        },
        Request::ConfirmDisplays {
            serial: 3,
            keep: true,
        },
    ] {
        h.scene.handle(SETTINGS, request).unwrap();
    }
    assert_eq!(
        h.scene.take_display_changes(),
        [
            DisplayChange {
                client: SETTINGS,
                serial: 2,
                request: DisplayRequest::Mode {
                    output: 1,
                    width: 1280,
                    height: 800,
                    refresh_mhz: 59_910,
                },
            },
            DisplayChange {
                client: SETTINGS,
                serial: 3,
                request: DisplayRequest::Confirm { keep: true },
            },
        ]
    );
    // The mode on trial is told to Settings, and to a Settings that connects meanwhile.
    h.scene.set_displays(vec![info(1)]);
    h.scene.display_confirmation(1, 15);
    h.pump();
    assert!(displays_told(&h).contains(&Event::DisplayConfirmation {
        output: 1,
        seconds: 15,
    }));
    h.scene.disconnect(SETTINGS);
    h.clear_log();
    h.scene
        .connect(
            SETTINGS,
            Grants {
                settings: true,
                ..Grants::default()
            },
        )
        .unwrap();
    h.pump();
    let told = displays_told(&h);
    assert!(matches!(
        told.last(),
        Some(Event::DisplayConfirmation {
            output: 1,
            seconds: 15
        })
    ));
    assert!(told
        .iter()
        .any(|event| matches!(event, Event::Display { .. })));
    // A change asked for by a client that then goes is dropped.
    h.scene
        .handle(
            SETTINGS,
            Request::SetDisplayOrigin {
                serial: 4,
                output: 1,
                x: 0,
                y: 0,
            },
        )
        .unwrap();
    h.scene.disconnect(SETTINGS);
    assert!(h.scene.take_display_changes().is_empty());
}
