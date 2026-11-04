//! Where each window minimizes into (ADR 0011's genie): the rectangles the shell's dock says, by
//! window number, and the global rectangle the compositor asks for as a window minimizes.

use super::*;
use oxyos_protocol::MAX_MINIMIZE_TARGETS;

/// The shell's dock: a panel 56 px tall at the bottom of output 1, its surface 4.
fn dock(h: &mut Harness) -> SurfaceKey {
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 4,
            kind: ShellKind::Panel,
            edge: PanelEdge::Bottom,
            output: 1,
            size: Size::new(1, 56),
            exclusive: 56,
        },
    )
    .unwrap();
    h.present(key(SHELL, 4));
    key(SHELL, 4)
}

/// The shell's list of targets on its surface 4, held from its next frame.
fn place_targets(h: &mut Harness, targets: &[(u32, Rect)]) -> Result<(), ErrorCode> {
    h.request(SHELL, Request::ClearMinimizeTargets { surface: 4 })?;
    for &(window, rect) in targets {
        h.request(
            SHELL,
            Request::PlaceMinimizeTarget {
                surface: 4,
                window,
                rect,
            },
        )?;
    }
    Ok(())
}

#[test]
fn a_window_minimizes_into_where_the_dock_says_from_its_next_frame() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 800, 600);
    let other = h.toplevel(OTHER, 1, 400, 300);
    let (number, other_number) = (
        h.scene.window_number(window).unwrap(),
        h.scene.window_number(other).unwrap(),
    );
    let dock = dock(&mut h);
    // The dock is the output's bottom 56 px.
    assert_eq!(
        h.scene.global_rect(dock),
        Some(Rect::new(0, 1024, 1920, 56))
    );
    assert_eq!(h.scene.minimize_target(window), None);
    place_targets(
        &mut h,
        &[
            (number, Rect::new(512, 7, 42, 42)),
            (other_number, Rect::new(570, 7, 42, 42)),
        ],
    )
    .unwrap();
    // Not before the dock's frame that goes with the list.
    assert_eq!(h.scene.minimize_target(window), None);
    h.present(dock);
    // In the global space: the dock's origin and the rectangle in it.
    assert_eq!(
        h.scene.minimize_target(window),
        Some(Rect::new(512, 1031, 42, 42))
    );
    assert_eq!(
        h.scene.minimize_target(other),
        Some(Rect::new(570, 1031, 42, 42))
    );
    // A list made again replaces the one held, from the next frame: a window placed again is
    // where it was placed last; one not placed has none.
    place_targets(
        &mut h,
        &[
            (number, Rect::new(512, 7, 42, 42)),
            (number, Rect::new(628, 7, 42, 42)),
        ],
    )
    .unwrap();
    assert_eq!(
        h.scene.minimize_target(window),
        Some(Rect::new(512, 1031, 42, 42)),
        "held until the dock's next frame"
    );
    h.present(dock);
    assert_eq!(
        h.scene.minimize_target(window),
        Some(Rect::new(628, 1031, 42, 42))
    );
    assert_eq!(h.scene.minimize_target(other), None);
    // A frame with no list made since holds the one it had.
    h.present(dock);
    assert_eq!(
        h.scene.minimize_target(window),
        Some(Rect::new(628, 1031, 42, 42))
    );
    // A minimized window still has its target: it is restored out of it.
    h.scene.minimize(window);
    assert_eq!(
        h.scene.minimize_target(window),
        Some(Rect::new(628, 1031, 42, 42))
    );
    // An empty rectangle is none.
    place_targets(&mut h, &[(number, Rect::new(628, 7, 0, 42))]).unwrap();
    h.present(dock);
    assert_eq!(h.scene.minimize_target(window), None);
    // The dock gone, its list is.
    place_targets(&mut h, &[(number, Rect::new(512, 7, 42, 42))]).unwrap();
    h.present(dock);
    assert!(h.scene.minimize_target(window).is_some());
    h.request(SHELL, Request::Destroy { surface: 4 }).unwrap();
    assert_eq!(h.scene.minimize_target(window), None);
    assert!(h.scene.minimize_targets.is_empty());
}

#[test]
fn only_the_shell_says_where_windows_minimize_and_only_on_its_shell_surfaces() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 800, 600);
    let number = h.scene.window_number(window).unwrap();
    let rect = Rect::new(0, 0, 42, 42);
    let place = |surface| Request::PlaceMinimizeTarget {
        surface,
        window: number,
        rect,
    };
    // An application cannot, not even on its own window.
    assert_eq!(
        h.scene
            .handle(APP, Request::ClearMinimizeTargets { surface: 1 }),
        Err(ErrorCode::Unauthorized)
    );
    assert_eq!(h.scene.handle(APP, place(1)), Err(ErrorCode::Unauthorized));
    // The shell, on a toplevel of its own: not a shell surface.
    h.toplevel(SHELL, 1, 200, 200);
    assert_eq!(
        h.scene
            .handle(SHELL, Request::ClearMinimizeTargets { surface: 1 }),
        Err(ErrorCode::InvalidRole)
    );
    assert_eq!(
        h.scene
            .handle(SHELL, Request::ClearMinimizeTargets { surface: 5 }),
        Err(ErrorCode::UnknownSurface)
    );
    dock(&mut h);
    // A target is placed in a list begun.
    assert_eq!(
        h.scene.handle(SHELL, place(4)),
        Err(ErrorCode::InvalidOrder)
    );
    // At most MAX_MINIMIZE_TARGETS windows, each its own; a window placed again takes no more.
    let many: Vec<_> = (0..MAX_MINIMIZE_TARGETS as u32)
        .map(|window| (window + 1000, rect))
        .collect();
    place_targets(&mut h, &many).unwrap();
    assert_eq!(
        h.scene.handle(
            SHELL,
            Request::PlaceMinimizeTarget {
                surface: 4,
                window: 1000,
                rect,
            }
        ),
        Ok(())
    );
    assert_eq!(
        h.scene.handle(SHELL, place(4)),
        Err(ErrorCode::QuotaExceeded)
    );
}
