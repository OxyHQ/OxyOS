//! An application launched as its own process (the native Bloom host, or any client of the
//! protocol), shown in a window and driven by scripted input.
//!
//! The director waits for the application's first frames to settle, then runs each step in
//! order: a left-button press at a window-local logical point (or at a point of its popup), a
//! scroll with the pointer at a point, committed text, or a key. After each step it waits for the
//! application's surfaces (its window and any popups) to present changed pixels on the output.
//! `--expect-popup` asserts that a popup of the application is shown beyond its window. Every
//! step is snapshotted. Nothing here knows what the application draws: a step passes only if the
//! application answered it with new pixels.

use super::*;

/// Time the application's frames must stay unchanged to count as settled.
const SETTLE: Duration = Duration::from_millis(750);
/// The application starts its runtime and loads its bundle inside this budget.
const LAUNCH_DEADLINE: Duration = Duration::from_secs(60);
const CLIENT: ClientId = 1;
const WINDOW: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Left-button press and release at a window-local logical point.
    Press { x: i32, y: i32 },
    /// The same at a point of the application's popup, relative to the popup.
    PressPopup { x: i32, y: i32 },
    /// Scroll by `dy` logical pixels (positive moves the view down) with the pointer at a point.
    Scroll { x: i32, y: i32, dy: i32 },
    /// Text committed to the focused surface, as a keyboard layout or input method would.
    Type(String),
    /// Press and release of a key (Linux input code).
    Key(u32),
    /// No input: a popup of the application must be shown, extending beyond its window.
    ExpectPopup,
}

impl Step {
    fn describe(&self) -> String {
        match self {
            Step::Press { x, y } => format!("press at ({x}, {y})"),
            Step::PressPopup { x, y } => format!("press at ({x}, {y}) in the popup"),
            Step::Scroll { x, y, dy } => format!("scroll by {dy} at ({x}, {y})"),
            Step::Type(text) => format!("typing {text:?}"),
            Step::Key(code) => format!("key {code}"),
            Step::ExpectPopup => "a popup beyond the window".to_string(),
        }
    }

    fn json(&self) -> String {
        match self {
            Step::Press { x, y } => format!("\"press\":[{x},{y}]"),
            Step::PressPopup { x, y } => format!("\"press_popup\":[{x},{y}]"),
            Step::Scroll { x, y, dy } => format!("\"scroll\":[{x},{y},{dy}]"),
            Step::Type(text) => format!("\"type\":{text:?}"),
            Step::Key(code) => format!("\"key\":{code}"),
            Step::ExpectPopup => "\"expect_popup\":true".to_string(),
        }
    }
}

pub struct Launch {
    pub command: Vec<std::ffi::OsString>,
    pub steps: Vec<Step>,
}

const USAGE: &str = "usage: oxy-offscreen --launch [--out DIR] [--press X,Y | --press-popup X,Y | \
                     --scroll X,Y,DY | --type TEXT | --key CODE | --expect-popup]... -- COMMAND [ARGS]...";

fn numbers(value: Option<&std::ffi::OsString>, count: usize) -> Result<Vec<i32>> {
    let value = value.and_then(|v| v.to_str()).ok_or(USAGE)?;
    let numbers = value
        .split(',')
        .map(|n| n.trim().parse::<i32>())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if numbers.len() != count {
        return Err(format!("{value:?}: expected {count} numbers").into());
    }
    Ok(numbers)
}

impl Launch {
    pub fn parse(args: &[std::ffi::OsString]) -> Result<(PathBuf, Self)> {
        let mut out = PathBuf::from("target/oxy-offscreen");
        let mut steps = Vec::new();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--out") => out = args.next().ok_or(USAGE)?.into(),
                Some("--press") => {
                    let n = numbers(args.next(), 2)?;
                    steps.push(Step::Press { x: n[0], y: n[1] });
                }
                Some("--press-popup") => {
                    let n = numbers(args.next(), 2)?;
                    steps.push(Step::PressPopup { x: n[0], y: n[1] });
                }
                Some("--scroll") => {
                    let n = numbers(args.next(), 3)?;
                    steps.push(Step::Scroll {
                        x: n[0],
                        y: n[1],
                        dy: n[2],
                    });
                }
                Some("--type") => {
                    let text = args.next().and_then(|t| t.to_str()).ok_or(USAGE)?;
                    steps.push(Step::Type(text.to_string()));
                }
                Some("--key") => {
                    let code = args.next().and_then(|c| c.to_str()).ok_or(USAGE)?;
                    steps.push(Step::Key(code.parse()?));
                }
                Some("--expect-popup") => steps.push(Step::ExpectPopup),
                Some("--launch") => {}
                Some("--") => {
                    let command: Vec<_> = args.cloned().collect();
                    if command.is_empty() {
                        return Err("no command after --".into());
                    }
                    return Ok((out, Self { command, steps }));
                }
                _ => return Err(format!("unexpected argument {arg:?}\n{USAGE}").into()),
            }
        }
        Err(USAGE.into())
    }
}

/// The application's shown surfaces: (surface id, frame sequence, output rectangle).
fn shown(compositor: &Compositor) -> Vec<(u32, u64, Rect)> {
    compositor
        .scene
        .render_list()
        .into_iter()
        .filter_map(|item| match item {
            RenderItem::Surface { key, rect, frame } if key.client == CLIENT => {
                Some((key.id, frame.sequence, rect))
            }
            _ => None,
        })
        .collect()
}

/// The application's first popup, if one is shown.
fn popup(compositor: &Compositor) -> Option<Rect> {
    shown(compositor)
        .into_iter()
        .find(|(id, _, _)| *id != WINDOW)
        .map(|(_, _, rect)| rect)
}

/// Serves the session until the application's surfaces differ from `before` (a new frame, a
/// popup opened or closed) and then stay unchanged for SETTLE. Returns the window's rectangle.
fn settle(
    session: &mut Session,
    children: &mut Children,
    before: &[(u32, u64, Rect)],
    deadline: Instant,
    waiting_for: &str,
) -> Result<Rect> {
    let mut last = (before.to_vec(), Instant::now());
    loop {
        if Instant::now() > deadline {
            return Err(format!("deadline exceeded waiting for {waiting_for}").into());
        }
        if let Some(status) = children.0[0].try_wait()? {
            return Err(
                format!("the application exited ({status}) waiting for {waiting_for}").into(),
            );
        }
        session.compositor.serve(&session.listener)?;
        let now = shown(&session.compositor);
        if now != last.0 {
            last = (now, Instant::now());
        } else if now != before && last.1.elapsed() >= SETTLE {
            if let Some(window) = session.compositor.surface_rect(CLIENT, WINDOW) {
                return Ok(window);
            }
        }
    }
}

/// How far `popup` reaches beyond each edge of `window` (left, top, right, bottom), in logical
/// pixels; zero where it stays inside.
fn beyond(window: Rect, popup: Rect) -> [i64; 4] {
    [
        (i64::from(window.x) - i64::from(popup.x)).max(0),
        (i64::from(window.y) - i64::from(popup.y)).max(0),
        (popup.right() - window.right()).max(0),
        (popup.bottom() - window.bottom()).max(0),
    ]
}

/// Injects one step's input into the scene.
fn inject(compositor: &mut Compositor, window: Rect, step: &Step) -> Result<()> {
    let inside = |rect: Rect, x: i32, y: i32| -> Result<()> {
        if rect.contains_point(rect.x + x, rect.y + y) {
            Ok(())
        } else {
            Err(format!("({x}, {y}) is outside {rect:?}").into())
        }
    };
    let press = |scene: &mut Scene, x: i32, y: i32| {
        scene.pointer_motion(x, y);
        scene.pointer_button(BUTTON, true);
        scene.pointer_button(BUTTON, false);
    };
    match step {
        Step::Press { x, y } => {
            inside(window, *x, *y)?;
            press(&mut compositor.scene, window.x + x, window.y + y);
        }
        Step::PressPopup { x, y } => {
            let popup = popup(compositor).ok_or("no popup is shown")?;
            inside(popup, *x, *y)?;
            press(&mut compositor.scene, popup.x + x, popup.y + y);
        }
        Step::Scroll { x, y, dy } => {
            inside(window, *x, *y)?;
            compositor.scene.pointer_motion(window.x + x, window.y + y);
            compositor.scene.pointer_scroll(0, dy.saturating_mul(256));
        }
        Step::Type(text) => compositor.scene.text(text),
        Step::Key(code) => {
            compositor.scene.key(*code, true);
            compositor.scene.key(*code, false);
        }
        Step::ExpectPopup => {}
    }
    Ok(())
}

pub fn run(out: &Path, launch: &Launch) -> Result<()> {
    fs::create_dir_all(out)?;
    let mut session = Session::start()?;
    let mut children = Children(vec![Command::new(&launch.command[0])
        .args(&launch.command[1..])
        .env("OXY_COMPOSITOR_SOCKET", &session.socket)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .spawn()?]);
    let deadline = Instant::now() + LAUNCH_DEADLINE;

    let mut window = settle(
        &mut session,
        &mut children,
        &[],
        deadline,
        "the first frame",
    )?;
    if (window.y..window.bottom() as i32).all(|y| {
        (window.x..window.right() as i32).all(|x| session.compositor.pixel(x, y) == DESKTOP)
    }) {
        return Err("the window shows nothing".into());
    }
    let mut output = session.compositor.framebuffer.clone();
    let mut snapshots = vec![session.compositor.snapshot(out, "launch-1.png")?];
    let mut answered = Vec::new();

    for (index, step) in launch.steps.iter().enumerate() {
        if *step == Step::ExpectPopup {
            let popup = popup(&session.compositor).ok_or("no popup is shown")?;
            let reach = beyond(window, popup);
            if reach.iter().all(|&r| r == 0) {
                return Err(
                    format!("the popup {popup:?} stays inside the window {window:?}").into(),
                );
            }
            answered.push(format!(
                "{{{},\"popup\":[{},{},{},{}],\"beyond_window\":{{\"left\":{},\"top\":{},\"right\":{},\"bottom\":{}}}}}",
                step.json(),
                popup.x,
                popup.y,
                popup.width,
                popup.height,
                reach[0],
                reach[1],
                reach[2],
                reach[3]
            ));
            continue;
        }
        let before = shown(&session.compositor);
        inject(&mut session.compositor, window, step)?;
        let what = format!("an answer to step {} ({})", index + 1, step.describe());
        window = settle(&mut session, &mut children, &before, deadline, &what)?;
        let now = session.compositor.framebuffer.clone();
        let changed = now.iter().zip(&output).filter(|(a, b)| a != b).count();
        if changed == 0 {
            return Err(
                format!("step {} ({}) changed no pixel", index + 1, step.describe()).into(),
            );
        }
        answered.push(format!("{{{},\"changed_pixels\":{changed}}}", step.json()));
        snapshots.push(
            session
                .compositor
                .snapshot(out, &format!("launch-{}.png", index + 2))?,
        );
        output = now;
    }

    let frames = session.compositor.frames;
    session.finish(&mut children)?;
    println!(
        "{{\"mode\":\"offscreen-cpu\",\"drm\":false,\"gpu\":false,\"launched\":{:?},\"window\":[{},{},{},{}],\"frames\":{frames},\"steps\":[{}],\"descriptors_settled\":true,\"snapshots\":{:?},\"status\":\"passed\"}}",
        launch.command[0],
        window.x,
        window.y,
        window.width,
        window.height,
        answered.join(","),
        snapshots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<std::ffi::OsString> {
        list.iter().map(Into::into).collect()
    }

    #[test]
    fn steps_parse_in_order() {
        let (out, launch) = Launch::parse(&args(&[
            "--launch",
            "--out",
            "o",
            "--press",
            "1,2",
            "--scroll",
            "3,4,-50",
            "--type",
            "hi",
            "--key",
            "14",
            "--expect-popup",
            "--press-popup",
            "5,6",
            "--",
            "app",
            "--flag",
        ]))
        .unwrap();
        assert_eq!(out, PathBuf::from("o"));
        assert_eq!(
            launch.steps,
            [
                Step::Press { x: 1, y: 2 },
                Step::Scroll {
                    x: 3,
                    y: 4,
                    dy: -50
                },
                Step::Type("hi".into()),
                Step::Key(14),
                Step::ExpectPopup,
                Step::PressPopup { x: 5, y: 6 },
            ]
        );
        assert_eq!(launch.command, args(&["app", "--flag"]));
    }

    #[test]
    fn malformed_steps_are_rejected() {
        for bad in [
            &["--press", "1", "--", "app"][..],
            &["--scroll", "1,2", "--", "app"],
            &["--key", "x", "--", "app"],
            &["--press", "1,2"],
            &["--press", "1,2", "--"],
        ] {
            assert!(Launch::parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn reach_beyond_the_window_is_measured_per_edge() {
        let window = Rect::new(100, 100, 400, 300);
        assert_eq!(beyond(window, Rect::new(150, 150, 50, 50)), [0, 0, 0, 0]);
        assert_eq!(
            beyond(window, Rect::new(450, 80, 120, 400)),
            [0, 20, 70, 80]
        );
    }
}
