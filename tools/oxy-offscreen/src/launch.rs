//! An application launched as its own process (the native Bloom host, or any client of the
//! protocol), shown in a window and driven by scripted input.
//!
//! The director waits for the application's first frames to settle, then presses each requested
//! point (window-local logical coordinates) with the left button and waits for the application
//! to present a changed window. Every step is snapshotted. Nothing here knows what the
//! application draws: a press passes only if the application answered it with new pixels.

use super::*;

/// Time a window's frames must stay unchanged to count as settled.
const SETTLE: Duration = Duration::from_millis(750);
/// The application starts its runtime and loads its bundle inside this budget.
const LAUNCH_DEADLINE: Duration = Duration::from_secs(60);
const CLIENT: ClientId = 1;
const WINDOW: u32 = 1;

pub struct Launch {
    pub command: Vec<std::ffi::OsString>,
    /// Window-local logical points to press, in order.
    pub presses: Vec<(i32, i32)>,
}

impl Launch {
    pub fn parse(args: &[std::ffi::OsString]) -> Result<(PathBuf, Self)> {
        let mut out = PathBuf::from("target/oxy-offscreen");
        let mut presses = Vec::new();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--out") => out = args.next().ok_or("--out needs a directory")?.into(),
                Some("--press") => {
                    let point = args
                        .next()
                        .and_then(|p| p.to_str())
                        .ok_or("--press needs X,Y")?;
                    let (x, y) = point.split_once(',').ok_or("--press needs X,Y")?;
                    presses.push((x.trim().parse()?, y.trim().parse()?));
                }
                Some("--launch") => {}
                Some("--") => {
                    let command: Vec<_> = args.cloned().collect();
                    if command.is_empty() {
                        return Err("no command after --".into());
                    }
                    return Ok((out, Self { command, presses }));
                }
                _ => return Err(format!("unexpected argument {arg:?}").into()),
            }
        }
        Err(
            "usage: oxy-offscreen --launch [--out DIR] [--press X,Y]... -- COMMAND [ARGS]..."
                .into(),
        )
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

    for (index, &(x, y)) in launch.presses.iter().enumerate() {
        let window = session
            .compositor
            .surface_rect(CLIENT, WINDOW)
            .ok_or("the window disappeared")?;
        if !window.contains_point(window.x + x, window.y + y) {
            return Err(format!("press ({x}, {y}) is outside the {window:?} window").into());
        }
        let before = session
            .compositor
            .surface_sequence(CLIENT, WINDOW)
            .expect("shown");
        let scene = &mut session.compositor.scene;
        scene.pointer_motion(window.x + x, window.y + y);
        scene.pointer_button(BUTTON, true);
        scene.pointer_button(BUTTON, false);
        let what = format!("an answer to press {} at ({x}, {y})", index + 1);
        let window = settle(&mut session, &mut children, before, deadline, &what)?;
        let now = window_pixels(&session.compositor, window);
        let changed = now.iter().zip(&pixels).filter(|(a, b)| a != b).count()
            + now.len().abs_diff(pixels.len());
        if changed == 0 {
            return Err(format!("press {} at ({x}, {y}) changed no pixel", index + 1).into());
        }
        answered.push(format!(
            "{{\"x\":{x},\"y\":{y},\"changed_pixels\":{changed}}}"
        ));
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
        "{{\"mode\":\"offscreen-cpu\",\"drm\":false,\"gpu\":false,\"launched\":{:?},\"window\":[{},{},{},{}],\"frames\":{frames},\"presses\":[{}],\"descriptors_settled\":true,\"snapshots\":{:?},\"status\":\"passed\"}}",
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
