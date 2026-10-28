//! An application launched as its own process (the native Bloom host, or any client of the
//! protocol), shown in a window and driven by scripted input.
//!
//! The director waits for the application's first frames to settle, then runs each step in
//! order: a left-button press at a window-local logical point, a scroll with the pointer at a
//! point, committed text, or a key. After each step it waits for the application to present a
//! changed window. Every step is snapshotted. Nothing here knows what the application draws: a
//! step passes only if the application answered it with new pixels.

use super::*;

/// Time a window's frames must stay unchanged to count as settled.
const SETTLE: Duration = Duration::from_millis(750);
/// The application starts its runtime and loads its bundle inside this budget.
const LAUNCH_DEADLINE: Duration = Duration::from_secs(60);
const CLIENT: ClientId = 1;
const WINDOW: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Left-button press and release at a window-local logical point.
    Press { x: i32, y: i32 },
    /// Scroll by `dy` logical pixels (positive moves the view down) with the pointer at a point.
    Scroll { x: i32, y: i32, dy: i32 },
    /// Text committed to the focused surface, as a keyboard layout or input method would.
    Type(String),
    /// Press and release of a key (Linux input code).
    Key(u32),
}

impl Step {
    fn describe(&self) -> String {
        match self {
            Step::Press { x, y } => format!("press at ({x}, {y})"),
            Step::Scroll { x, y, dy } => format!("scroll by {dy} at ({x}, {y})"),
            Step::Type(text) => format!("typing {text:?}"),
            Step::Key(code) => format!("key {code}"),
        }
    }

    fn json(&self) -> String {
        match self {
            Step::Press { x, y } => format!("\"press\":[{x},{y}]"),
            Step::Scroll { x, y, dy } => format!("\"scroll\":[{x},{y},{dy}]"),
            Step::Type(text) => format!("\"type\":{text:?}"),
            Step::Key(code) => format!("\"key\":{code}"),
        }
    }
}

pub struct Launch {
    pub command: Vec<std::ffi::OsString>,
    pub steps: Vec<Step>,
}

const USAGE: &str = "usage: oxy-offscreen --launch [--out DIR] [--press X,Y | --scroll X,Y,DY | \
                     --type TEXT | --key CODE]... -- COMMAND [ARGS]...";

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

/// The window's pixels, row by row, clipped to the output.
fn window_pixels(compositor: &Compositor, rect: Rect) -> Vec<u32> {
    let mut pixels = Vec::new();
    for y in rect.y.max(0)..(rect.bottom().min(OUTPUT.bottom()) as i32) {
        for x in rect.x.max(0)..(rect.right().min(OUTPUT.right()) as i32) {
            pixels.push(compositor.pixel(x, y));
        }
    }
    pixels
}

/// Serves the session until the window has presented a frame newer than `after` and then stayed
/// unchanged for SETTLE. Returns its rectangle.
fn settle(
    session: &mut Session,
    children: &mut Children,
    after: u64,
    deadline: Instant,
    waiting_for: &str,
) -> Result<Rect> {
    let mut last = (after, Instant::now());
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
        let Some(sequence) = session.compositor.surface_sequence(CLIENT, WINDOW) else {
            continue;
        };
        if sequence != last.0 {
            last = (sequence, Instant::now());
        } else if sequence > after && last.1.elapsed() >= SETTLE {
            return Ok(session
                .compositor
                .surface_rect(CLIENT, WINDOW)
                .expect("a window with a frame is shown"));
        }
    }
}

/// Injects one step's input into the scene.
fn inject(scene: &mut Scene, window: Rect, step: &Step) -> Result<()> {
    let inside = |x: i32, y: i32| -> Result<()> {
        if window.contains_point(window.x + x, window.y + y) {
            Ok(())
        } else {
            Err(format!("({x}, {y}) is outside the {window:?} window").into())
        }
    };
    match step {
        Step::Press { x, y } => {
            inside(*x, *y)?;
            scene.pointer_motion(window.x + x, window.y + y);
            scene.pointer_button(BUTTON, true);
            scene.pointer_button(BUTTON, false);
        }
        Step::Scroll { x, y, dy } => {
            inside(*x, *y)?;
            scene.pointer_motion(window.x + x, window.y + y);
            scene.pointer_scroll(0, dy.saturating_mul(256));
        }
        Step::Type(text) => scene.text(text),
        Step::Key(code) => {
            scene.key(*code, true);
            scene.key(*code, false);
        }
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

    let window = settle(&mut session, &mut children, 0, deadline, "the first frame")?;
    let mut pixels = window_pixels(&session.compositor, window);
    if pixels.iter().all(|&p| p == DESKTOP) {
        return Err("the window shows nothing".into());
    }
    let mut snapshots = vec![session.compositor.snapshot(out, "launch-1.png")?];
    let mut answered = Vec::new();

    for (index, step) in launch.steps.iter().enumerate() {
        let window = session
            .compositor
            .surface_rect(CLIENT, WINDOW)
            .ok_or("the window disappeared")?;
        let before = session
            .compositor
            .surface_sequence(CLIENT, WINDOW)
            .expect("shown");
        inject(&mut session.compositor.scene, window, step)?;
        let what = format!("an answer to step {} ({})", index + 1, step.describe());
        let window = settle(&mut session, &mut children, before, deadline, &what)?;
        let now = window_pixels(&session.compositor, window);
        let changed = now.iter().zip(&pixels).filter(|(a, b)| a != b).count()
            + now.len().abs_diff(pixels.len());
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
        pixels = now;
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
            "--launch", "--out", "o", "--press", "1,2", "--scroll", "3,4,-50", "--type", "hi",
            "--key", "14", "--", "app", "--flag",
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
}
