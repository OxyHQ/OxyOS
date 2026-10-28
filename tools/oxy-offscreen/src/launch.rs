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
use oxy_protocol::{AccessibleAction, AccessibleState};

/// Time the application's frames must stay unchanged to count as settled.
const SETTLE: Duration = Duration::from_millis(750);
/// The application starts its runtime and loads its bundle inside this budget.
const LAUNCH_DEADLINE: Duration = Duration::from_secs(60);
/// Each step's answer must settle inside this budget.
const STEP_DEADLINE: Duration = Duration::from_secs(30);
const CLIENT: ClientId = 1;
/// The assistive technology the director plays: a client of the scene only, granted
/// `accessibility`, reading what the compositor holds and sending actions (ADR 0003).
const READER: ClientId = ClientId::MAX;
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
    /// The input method's composition (preedit) for the focused surface, cursor at its end.
    Preedit(String),
    /// Left-button drag in the window: press at the first point, move to the second, release.
    Drag { from: (i32, i32), to: (i32, i32) },
    /// Press and release of a key or a chord (Linux input codes): pressed in order, released in
    /// reverse, so `42+106` is Shift+Right.
    Key(Vec<u32>),
    /// The shell moves the window by (dx, dy), as a keyboard move or tiling does.
    MoveWindow { dx: i32, dy: i32 },
    /// The shell resizes the window, as maximizing or tiling does.
    ResizeWindow { width: u32, height: u32 },
    /// The assistive technology acts on the application's node named `name` (the topmost
    /// surface first): activate or focus.
    Accessibility {
        action: AccessibleAction,
        name: String,
    },
    /// The output's scale changes (in 120ths, so 180 is 1.5): the compositor asks every surface
    /// on it for buffers at the new scale; logical geometry is unchanged.
    Scale(u32),
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
            Step::Preedit(text) => format!("composing {text:?}"),
            Step::Drag { from, to } => format!("drag from {from:?} to {to:?}"),
            Step::Key(codes) => format!("key {}", chord(codes)),
            Step::MoveWindow { dx, dy } => format!("window moved by ({dx}, {dy})"),
            Step::ResizeWindow { width, height } => format!("window resized to {width}x{height}"),
            Step::Accessibility { action, name } => format!("assistive {action:?} on {name:?}"),
            Step::Scale(scale) => format!("output scale {scale}/120"),
            Step::ExpectPopup => "a popup beyond the window".to_string(),
        }
    }

    fn json(&self) -> String {
        match self {
            Step::Press { x, y } => format!("\"press\":[{x},{y}]"),
            Step::PressPopup { x, y } => format!("\"press_popup\":[{x},{y}]"),
            Step::Scroll { x, y, dy } => format!("\"scroll\":[{x},{y},{dy}]"),
            Step::Type(text) => format!("\"type\":{text:?}"),
            Step::Preedit(text) => format!("\"preedit\":{text:?}"),
            Step::Drag { from, to } => {
                format!("\"drag\":[{},{},{},{}]", from.0, from.1, to.0, to.1)
            }
            Step::Key(codes) => format!("\"key\":{codes:?}"),
            Step::MoveWindow { dx, dy } => format!("\"move_window\":[{dx},{dy}]"),
            Step::ResizeWindow { width, height } => {
                format!("\"resize_window\":[{width},{height}]")
            }
            Step::Accessibility { action, name } => {
                format!("\"a11y_action\":[\"{action:?}\",{}]", json_string(name))
            }
            Step::Scale(scale) => format!("\"scale\":{scale}"),
            Step::ExpectPopup => "\"expect_popup\":true".to_string(),
        }
    }
}

fn chord(codes: &[u32]) -> String {
    codes
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join("+")
}

pub struct Launch {
    pub command: Vec<std::ffi::OsString>,
    /// Each step, and whether it may leave the output unchanged (`--allow-unchanged` before it).
    pub steps: Vec<(Step, bool)>,
    /// `--gpu`: compose every frame on a Vulkan device too; any difference from the CPU
    /// reference fails the run.
    pub gpu: bool,
    /// `--dump-frames DIR`: the frame shown at launch and after each step, as layers
    /// (`oxy_gpu::frame_file`), to compose again elsewhere (`oxy-gpu`'s `replay` example).
    pub dump_frames: Option<PathBuf>,
}

const USAGE: &str = "usage: oxy-offscreen --launch [--out DIR] [--gpu] [--dump-frames DIR] [[--allow-unchanged] --press X,Y | \
                     --press-popup X,Y | --scroll X,Y,DY | --type TEXT | --preedit TEXT | --key CODE[+CODE]... | --drag X0,Y0,X1,Y1 | --scale SCALE_120 | --a11y-action activate|focus,NAME | --move-window DX,DY | --resize-window W,H | --expect-popup]... \
                     -- COMMAND [ARGS]...";

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
        let mut steps: Vec<(Step, bool)> = Vec::new();
        // `--allow-unchanged` applies to the step after it.
        let mut allow = false;
        let mut gpu = false;
        let mut dump_frames = None;
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let step = match arg.to_str() {
                Some("--out") => {
                    out = args.next().ok_or(USAGE)?.into();
                    continue;
                }
                Some("--allow-unchanged") => {
                    allow = true;
                    continue;
                }
                Some("--launch") => continue,
                Some("--gpu") => {
                    gpu = true;
                    continue;
                }
                Some("--dump-frames") => {
                    dump_frames = Some(args.next().ok_or(USAGE)?.into());
                    continue;
                }
                Some("--press") => {
                    let n = numbers(args.next(), 2)?;
                    Step::Press { x: n[0], y: n[1] }
                }
                Some("--press-popup") => {
                    let n = numbers(args.next(), 2)?;
                    Step::PressPopup { x: n[0], y: n[1] }
                }
                Some("--scroll") => {
                    let n = numbers(args.next(), 3)?;
                    Step::Scroll {
                        x: n[0],
                        y: n[1],
                        dy: n[2],
                    }
                }
                Some("--type") => {
                    let text = args.next().and_then(|t| t.to_str()).ok_or(USAGE)?;
                    Step::Type(text.to_string())
                }
                Some("--key") => {
                    let codes = args.next().and_then(|c| c.to_str()).ok_or(USAGE)?;
                    Step::Key(
                        codes
                            .split('+')
                            .map(str::parse)
                            .collect::<std::result::Result<_, _>>()?,
                    )
                }
                Some("--drag") => {
                    let n = numbers(args.next(), 4)?;
                    Step::Drag {
                        from: (n[0], n[1]),
                        to: (n[2], n[3]),
                    }
                }
                Some("--expect-popup") => Step::ExpectPopup,
                Some("--move-window") => {
                    let n = numbers(args.next(), 2)?;
                    Step::MoveWindow { dx: n[0], dy: n[1] }
                }
                Some("--resize-window") => {
                    let n = numbers(args.next(), 2)?;
                    Step::ResizeWindow {
                        width: u32::try_from(n[0])?,
                        height: u32::try_from(n[1])?,
                    }
                }
                Some("--a11y-action") => {
                    let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    let (action, name) = value.split_once(',').ok_or(USAGE)?;
                    let action = match action {
                        "activate" => AccessibleAction::Activate,
                        "focus" => AccessibleAction::Focus,
                        _ => return Err(format!("unknown action {action:?}\n{USAGE}").into()),
                    };
                    Step::Accessibility {
                        action,
                        name: name.to_string(),
                    }
                }
                Some("--scale") => {
                    let scale = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    Step::Scale(scale.parse()?)
                }
                Some("--preedit") => {
                    let text = args.next().and_then(|t| t.to_str()).ok_or(USAGE)?;
                    Step::Preedit(text.to_string())
                }
                Some("--") => {
                    let command: Vec<_> = args.cloned().collect();
                    if command.is_empty() {
                        return Err("no command after --".into());
                    }
                    return Ok((
                        out,
                        Self {
                            command,
                            steps,
                            gpu,
                            dump_frames,
                        },
                    ));
                }
                _ => return Err(format!("unexpected argument {arg:?}\n{USAGE}").into()),
            };
            steps.push((step, std::mem::take(&mut allow)));
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

/// CLOCK_MONOTONIC in nanoseconds, the clock the application's report (OXY_HOST_REPORT) uses.
fn monotonic_ns() -> i64 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    now.tv_sec * 1_000_000_000 + now.tv_nsec
}

/// When a step's answer first appeared, when it settled, and the window then.
struct Settled {
    window: Rect,
    first_frame_ns: Option<i64>,
    settled_ns: i64,
}

/// Serves the session until the application's surfaces differ from `before` (a new frame, a
/// popup opened or closed) and then stay unchanged for SETTLE. With `may_stay`, surfaces that do
/// not change for SETTLE also count as settled (a step whose expected answer is nothing).
fn settle(
    session: &mut Session,
    children: &mut Children,
    before: &[(u32, u64, Rect)],
    may_stay: bool,
    deadline: Instant,
    waiting_for: &str,
) -> Result<Settled> {
    let mut last = (before.to_vec(), Instant::now());
    let mut first_frame_ns = None;
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
            first_frame_ns.get_or_insert_with(monotonic_ns);
            last = (now, Instant::now());
        } else if (now != before || may_stay) && last.1.elapsed() >= SETTLE {
            if let Some(window) = session.compositor.surface_rect(CLIENT, WINDOW) {
                return Ok(Settled {
                    window,
                    first_frame_ns,
                    settled_ns: monotonic_ns(),
                });
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

/// The surface of the application holding keyboard focus, or null.
fn focus_json(compositor: &Compositor) -> String {
    match compositor.scene.focus() {
        Some(key) if key.client == CLIENT => key.id.to_string(),
        _ => "null".to_string(),
    }
}

/// The input routed during a step: for the application, counts per surface and kind; for any
/// other client, a count (`leaked`).
fn delivered_json(compositor: &Compositor) -> String {
    let mut counts: std::collections::BTreeMap<u32, std::collections::BTreeMap<&str, usize>> =
        Default::default();
    let mut leaked = 0;
    for &(client, surface, kind) in &compositor.input_log {
        if client == CLIENT {
            *counts.entry(surface).or_default().entry(kind).or_default() += 1;
        } else {
            leaked += 1;
        }
    }
    let mut fields: Vec<String> = counts
        .iter()
        .map(|(surface, kinds)| {
            let kinds: Vec<String> = kinds.iter().map(|(k, n)| format!("\"{k}\":{n}")).collect();
            format!("\"{surface}\":{{{}}}", kinds.join(","))
        })
        .collect();
    fields.push(format!("\"leaked\":{leaked}"));
    format!("{{{}}}", fields.join(","))
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// What the assistive technology has for each of the application's shown surfaces: the
/// compositor's committed snapshots, as {surface: [{role, name, value, state}]}.
fn accessibility_json(compositor: &Compositor) -> String {
    let states = [
        (AccessibleState::DISABLED, "disabled"),
        (AccessibleState::SELECTED, "selected"),
        (AccessibleState::CHECKED, "checked"),
        (AccessibleState::MIXED, "mixed"),
        (AccessibleState::EXPANDED, "expanded"),
        (AccessibleState::COLLAPSED, "collapsed"),
        (AccessibleState::BUSY, "busy"),
        (AccessibleState::FOCUSED, "focused"),
    ];
    let surfaces: Vec<String> = shown(compositor)
        .into_iter()
        .filter_map(|(id, _, _)| {
            let nodes = compositor
                .scene
                .accessibility_snapshot(SurfaceKey { client: CLIENT, id })?;
            let nodes: Vec<String> = nodes
                .iter()
                .map(|node| {
                    let state: Vec<String> = states
                        .iter()
                        .filter(|(bit, _)| node.state.contains(*bit))
                        .map(|(_, name)| format!("\"{name}\""))
                        .collect();
                    format!(
                        "{{\"role\":\"{:?}\",\"name\":{},\"value\":{},\"state\":[{}]}}",
                        node.role,
                        json_string(&node.name),
                        json_string(&node.value),
                        state.join(",")
                    )
                })
                .collect();
            Some(format!("\"{id}\":[{}]", nodes.join(",")))
        })
        .collect();
    format!("{{{}}}", surfaces.join(","))
}

/// The application's popups now shown, as [x, y, width, height] on the output.
fn popups_json(compositor: &Compositor) -> String {
    let popups: Vec<String> = shown(compositor)
        .into_iter()
        .filter(|(id, _, _)| *id != WINDOW)
        .map(|(_, _, r)| format!("[{},{},{},{}]", r.x, r.y, r.width, r.height))
        .collect();
    format!("[{}]", popups.join(","))
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
        Step::Preedit(text) => {
            if !compositor.scene.preedit(text, text.len()) {
                return Err(format!("preedit {text:?} does not fit one chunk").into());
            }
        }
        Step::Drag { from, to } => {
            inside(window, from.0, from.1)?;
            inside(window, to.0, to.1)?;
            let scene = &mut compositor.scene;
            scene.pointer_motion(window.x + from.0, window.y + from.1);
            scene.pointer_button(BUTTON, true);
            // Through a midpoint, as a hand moves, then to the end.
            scene.pointer_motion(
                window.x + (from.0 + to.0) / 2,
                window.y + (from.1 + to.1) / 2,
            );
            scene.pointer_motion(window.x + to.0, window.y + to.1);
            scene.pointer_button(BUTTON, false);
        }
        Step::Key(codes) => {
            for code in codes {
                compositor.scene.key(*code, true);
            }
            for code in codes.iter().rev() {
                compositor.scene.key(*code, false);
            }
        }
        Step::MoveWindow { dx, dy } => compositor.scene.move_window_to(
            SurfaceKey {
                client: CLIENT,
                id: WINDOW,
            },
            (window.x + dx, window.y + dy),
        ),
        Step::ResizeWindow { width, height } => compositor.scene.resize_window(
            SurfaceKey {
                client: CLIENT,
                id: WINDOW,
            },
            oxy_protocol::Size::new(*width, *height),
        ),
        Step::Accessibility { action, name } => {
            // Popups (later surfaces) are above the window: the topmost node with that name.
            let found = shown(compositor).into_iter().rev().find_map(|(id, _, _)| {
                let key = SurfaceKey { client: CLIENT, id };
                let nodes = compositor.scene.accessibility_snapshot(key)?;
                let index = nodes.iter().position(|node| node.name == *name)?;
                Some((id, index as u32))
            });
            let (surface, index) =
                found.ok_or_else(|| format!("no accessible node named {name:?}"))?;
            compositor
                .scene
                .handle(
                    READER,
                    Request::AccessibilityAction {
                        client: CLIENT,
                        surface,
                        index,
                        action: *action,
                    },
                )
                .map_err(|code| format!("accessibility action: {code:?}"))?;
        }
        Step::Scale(scale) => compositor
            .scene
            .set_output_scale(OUTPUT_ID, *scale)
            .map_err(|code| format!("scale {scale}: {code:?}"))?,
        Step::ExpectPopup => {}
    }
    Ok(())
}

pub fn run(out: &Path, launch: &Launch) -> Result<()> {
    fs::create_dir_all(out)?;
    let mut session = Session::start()?;
    session
        .compositor
        .scene
        .connect(
            READER,
            Grants {
                accessibility: true,
                ..Grants::default()
            },
        )
        .map_err(|code| format!("the assistive technology: {code:?}"))?;
    if let Some(dir) = &launch.dump_frames {
        fs::create_dir_all(dir)?;
        session.compositor.keep_frame = true;
    }
    if launch.gpu {
        let compositor = oxy_gpu::VulkanCompositor::new()?;
        eprintln!(
            "[gpu] composing on {} ({:?})",
            compositor.device_name(),
            compositor.device_kind()
        );
        session.compositor.gpu = Some(Gpu {
            compositor,
            frames: 0,
            mismatches: 0,
        });
    }
    let mut children = Children(vec![Command::new(&launch.command[0])
        .args(&launch.command[1..])
        .env("OXY_COMPOSITOR_SOCKET", &session.socket)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .spawn()?]);
    let deadline = Instant::now() + LAUNCH_DEADLINE;

    let launched_ns = monotonic_ns();
    let first = settle(
        &mut session,
        &mut children,
        &[],
        false,
        deadline,
        "the first frame",
    )?;
    let mut window = first.window;
    if (window.y..window.bottom() as i32).all(|y| {
        (window.x..window.right() as i32).all(|x| session.compositor.pixel(x, y) == DESKTOP)
    }) {
        return Err("the window shows nothing".into());
    }
    let mut output = session.compositor.framebuffer.clone();
    let mut snapshots = vec![session.compositor.snapshot(out, "launch-1.png")?];
    let dump = |compositor: &Compositor, index: usize| -> Result<()> {
        if let Some(dir) = &launch.dump_frames {
            fs::write(
                dir.join(format!("frame-{index:03}.oxyframe")),
                &compositor.last_frame,
            )?;
        }
        Ok(())
    };
    dump(&session.compositor, 1)?;
    let mut answered = Vec::new();

    session.compositor.log_input = true;
    for (index, (step, may_stay)) in launch.steps.iter().enumerate() {
        session.compositor.input_log.clear();
        if *step == Step::ExpectPopup {
            let popup = popup(&session.compositor).ok_or("no popup is shown")?;
            let reach = beyond(window, popup);
            if reach.iter().all(|&r| r == 0) {
                return Err(
                    format!("the popup {popup:?} stays inside the window {window:?}").into(),
                );
            }
            answered.push(format!(
                "{{{},\"t_settled\":{},\"popup\":[{},{},{},{}],\"beyond_window\":{{\"left\":{},\"top\":{},\"right\":{},\"bottom\":{}}}}}",
                step.json(),
                monotonic_ns(),
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
        let injected_ns = monotonic_ns();
        inject(&mut session.compositor, window, step)?;
        let what = format!("an answer to step {} ({})", index + 1, step.describe());
        let answer = settle(
            &mut session,
            &mut children,
            &before,
            *may_stay,
            Instant::now() + STEP_DEADLINE,
            &what,
        )?;
        window = answer.window;
        let now = session.compositor.framebuffer.clone();
        let changed = now.iter().zip(&output).filter(|(a, b)| a != b).count();
        if changed == 0 && !may_stay {
            return Err(
                format!("step {} ({}) changed no pixel", index + 1, step.describe()).into(),
            );
        }
        let latency = match answer.first_frame_ns {
            Some(ns) => format!("{:.2}", (ns - injected_ns) as f64 / 1e6),
            None => "null".to_string(),
        };
        answered.push(format!(
            "{{{},\"changed_pixels\":{changed},\"t_inject\":{injected_ns},\"t_settled\":{},\"latency_ms\":{latency},\"focus\":{},\"delivered\":{},\"popups\":{},\"window\":[{},{},{},{}],\"a11y\":{}}}",
            step.json(),
            answer.settled_ns,
            focus_json(&session.compositor),
            delivered_json(&session.compositor),
            popups_json(&session.compositor),
            answer.window.x,
            answer.window.y,
            answer.window.width,
            answer.window.height,
            accessibility_json(&session.compositor),
        ));
        snapshots.push(
            session
                .compositor
                .snapshot(out, &format!("launch-{}.png", index + 2))?,
        );
        dump(&session.compositor, index + 2)?;
        output = now;
    }

    let frames = session.compositor.frames;
    // The assistive technology leaves with the session, like every other client.
    session.compositor.scene.disconnect(READER);
    let gpu = match session.compositor.gpu.take() {
        Some(gpu) if gpu.mismatches > 0 => {
            return Err(format!(
                "Vulkan composition differed from the CPU reference in {} of {} frames",
                gpu.mismatches, gpu.frames
            )
            .into())
        }
        Some(gpu) => format!(
            "{{\"device\":{},\"kind\":\"{:?}\",\"frames\":{},\"identical\":true}}",
            json_string(gpu.compositor.device_name()),
            gpu.compositor.device_kind(),
            gpu.frames
        ),
        None => "null".to_string(),
    };
    session.finish(&mut children)?;
    println!(
        "{{\"mode\":\"offscreen-cpu\",\"drm\":false,\"gpu\":{gpu},\"launched\":{:?},\"t_launch\":{launched_ns},\"first_frame_ms\":{:.1},\"window\":[{},{},{},{}],\"frames\":{frames},\"steps\":[{}],\"descriptors_settled\":true,\"snapshots\":{:?},\"status\":\"passed\"}}",
        launch.command[0],
        first.first_frame_ns.map_or(0.0, |ns| (ns - launched_ns) as f64 / 1e6),
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
            "--gpu",
            "--dump-frames",
            "frames",
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
            "--key",
            "42+106",
            "--drag",
            "1,2,3,4",
            "--scale",
            "180",
            "--move-window",
            "-10,5",
            "--resize-window",
            "800,600",
            "--expect-popup",
            "--allow-unchanged",
            "--press-popup",
            "5,6",
            "--",
            "app",
            "--flag",
        ]))
        .unwrap();
        assert_eq!(out, PathBuf::from("o"));
        assert!(launch.gpu);
        assert_eq!(launch.dump_frames, Some(PathBuf::from("frames")));
        let steps: Vec<Step> = launch.steps.iter().map(|(step, _)| step.clone()).collect();
        let allowed: Vec<bool> = launch.steps.iter().map(|(_, allow)| *allow).collect();
        assert_eq!(
            allowed,
            [false, false, false, false, false, false, false, false, false, false, true]
        );
        assert_eq!(
            steps,
            [
                Step::Press { x: 1, y: 2 },
                Step::Scroll {
                    x: 3,
                    y: 4,
                    dy: -50
                },
                Step::Type("hi".into()),
                Step::Key(vec![14]),
                Step::Key(vec![42, 106]),
                Step::Drag {
                    from: (1, 2),
                    to: (3, 4)
                },
                Step::Scale(180),
                Step::MoveWindow { dx: -10, dy: 5 },
                Step::ResizeWindow {
                    width: 800,
                    height: 600
                },
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
            &["--key", "42+", "--", "app"],
            &["--drag", "1,2,3", "--", "app"],
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
