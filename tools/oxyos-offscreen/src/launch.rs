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
use oxyos_compositor_core::{physical, AppearanceChange, TextTransaction};
use oxyos_protocol::{AccessibleAction, AccessibleState, GesturePhase, PreeditStyle};

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
/// Caps Lock's Linux input code.
const CAPS_LOCK: u32 = 58;

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Left-button press and release at a window-local logical point.
    Press { x: i32, y: i32 },
    /// The same at a point of the application's popup, relative to the popup.
    PressPopup { x: i32, y: i32 },
    /// The same at a point of the output, wherever it is (the shell's panels and overlays).
    PressOutput { x: i32, y: i32 },
    /// A secondary (right) button press and release at a window-local logical point: a context
    /// menu's.
    PressSecondary { x: i32, y: i32 },
    /// Two left-button presses at a window-local logical point in quick succession, with nothing
    /// between them: a double click.
    DoublePress { x: i32, y: i32 },
    /// Scroll by `dy` logical pixels (positive moves the view down) with the pointer at a point.
    Scroll { x: i32, y: i32, dy: i32 },
    /// A touchpad pinch with the pointer at a window-local point: the fingers spread to `percent`
    /// of their spread and turn `degrees` clockwise, in eight even steps (`drag_points`) on the
    /// scene, or through the virtual touchpad, where libinput recognizes it.
    Pinch {
        x: i32,
        y: i32,
        percent: i32,
        degrees: i32,
    },
    /// Text typed into the focused surface: committed as it is, or, through the virtual
    /// devices, typed key by key on the layout.
    Type(String),
    /// Text an input method commits to the focused surface: committed as it is, never typed.
    Commit(String),
    /// The input method's composition (preedit) for the focused surface, cursor at its end.
    Preedit(String),
    /// One change as an input method makes it (ADR 0008): bytes deleted before and after the
    /// selection, committed text, then a composition with its cursor range and styled segments.
    Transaction {
        delete: (u32, u32),
        commit: String,
        preedit: String,
        cursor: (u32, u32),
        segments: Vec<(u32, u32, PreeditStyle)>,
    },
    /// Text typed key by key through the compositor's keyboard path (lowercase letters, digits,
    /// space, '.' and Enter as a newline): with `--input-method`, through the input method, which
    /// gives each key back, or (`compose`) composes the keys' text and commits it on Enter.
    Keyboard { text: String, compose: bool },
    /// The input method sends a change of its own, for the field's latest state or (`stale`) the
    /// state before it.
    ImChange {
        delete: (u32, u32),
        commit: String,
        preedit: String,
        stale: bool,
    },
    /// Left-button drag in the window: press at the first point, move to the second, release.
    Drag { from: (i32, i32), to: (i32, i32) },
    /// The same on the output, wherever it is (the shell's panels and overlays), moving in eight
    /// even steps, as a hand drags something across the screen.
    DragOutput { from: (i32, i32), to: (i32, i32) },
    /// A press at a window-local point held while the application answers it (a window's drag
    /// region asks the compositor to move the window, ADR 0016), then a drag by `by` in eight even
    /// steps and the release, as a hand grabs a window by its toolbar.
    Grab { from: (i32, i32), by: (i32, i32) },
    /// A press on the output held and moved to `to` in `motions` even steps, `hz` of them a
    /// second, the compositor serving the application between them, then the release: a hand
    /// dragging (a window's edge) at a mouse's pace. What the window showed meanwhile is measured
    /// (`drag_paced`).
    DragPaced {
        from: (i32, i32),
        to: (i32, i32),
        motions: u32,
        hz: u32,
    },
    /// The pointer at a window-local point scrolling by `dy` in `events` even steps, `hz` of them a
    /// second, the compositor serving the application between them: a touchpad's two fingers
    /// scrolling a list. What the window showed meanwhile is measured (`scroll_paced`).
    ScrollPaced {
        at: (i32, i32),
        dy: i32,
        events: u32,
        hz: u32,
    },
    /// The pointer moved over the window, no button held, from one window-local point to another
    /// in `motions` even steps, `hz` of them a second, the compositor serving the application
    /// between them: a hand moving the mouse across a list. What the window showed meanwhile is
    /// measured (`move_paced`).
    MovePaced {
        from: (i32, i32),
        to: (i32, i32),
        motions: u32,
        hz: u32,
    },
    /// Press and release of a key or a chord (Linux input codes): pressed in order, released in
    /// reverse, so `42+106` is Shift+Right.
    Key(Vec<u32>),
    /// Keys pressed in order and held, as a hand holds Super through the application switcher
    /// (ADR 0024), until a `KeyUp` releases them.
    KeyDown(Vec<u32>),
    /// Held keys released, in order.
    KeyUp(Vec<u32>),
    /// A press and release of Caps Lock: on the scene, the key as any other and the compositor's
    /// keymap's change of Caps Lock between them, as `oxyos-input` reports it; through the
    /// devices, the key itself.
    CapsLock,
    /// The shell moves the window by (dx, dy), as a keyboard move or tiling does.
    MoveWindow { dx: i32, dy: i32 },
    /// The shell resizes the window, as maximizing or tiling does.
    ResizeWindow { width: u32, height: u32 },
    /// The compositor minimizes the window, as a shell's shortcut does; only the shell brings it
    /// back (its dock).
    MinimizeWindow,
    /// The assistive technology acts on the node named `name` (the topmost surface first): any
    /// action (ADR 0003), a custom one by its label, a selection by its byte range.
    Accessibility {
        action: AccessibleAction,
        name: String,
        custom: Option<String>,
        selection: (u32, u32),
    },
    /// The output's scale changes (in 120ths, so 180 is 1.5): the compositor asks every surface
    /// on it for buffers at the new scale; logical geometry is unchanged.
    Scale(u32),
    /// No input: a popup of the application must be shown, extending beyond its window.
    ExpectPopup,
    /// Through the devices (`--uinput`, `--kms`): Ctrl+Alt+F`n` to terminal `n`, and Ctrl+Alt
    /// with the compositor's own terminal's key from there, which that terminal's owner handles
    /// (the console's keyboard, a desktop). Both switches must happen and the frame be shown
    /// again.
    SwitchTerminal(u16),
    /// No input for this many seconds: the idle lock counts them as passed at once, as if the
    /// person had left the machine alone that long (ADR 0010). Its own time; nothing waits.
    Idle(u64),
    /// Milliseconds of real time with no input, the session served all along: what fades on its
    /// own clock (a hardware key's bezel, ADR 0025) has the time to.
    Wait(u64),
}

/// The points of a drag on the output after its first: eight even steps to its last.
fn drag_points(from: (i32, i32), to: (i32, i32)) -> impl Iterator<Item = (i32, i32)> {
    const STEPS: i32 = 8;
    (1..=STEPS).map(move |i| {
        (
            from.0 + (to.0 - from.0) * i / STEPS,
            from.1 + (to.1 - from.1) * i / STEPS,
        )
    })
}

impl Step {
    fn describe(&self) -> String {
        match self {
            Step::Press { x, y } => format!("press at ({x}, {y})"),
            Step::PressPopup { x, y } => format!("press at ({x}, {y}) in the popup"),
            Step::PressOutput { x, y } => format!("press at ({x}, {y}) on the output"),
            Step::PressSecondary { x, y } => format!("secondary press at ({x}, {y})"),
            Step::DoublePress { x, y } => format!("double press at ({x}, {y})"),
            Step::Scroll { x, y, dy } => format!("scroll by {dy} at ({x}, {y})"),
            Step::Pinch {
                x,
                y,
                percent,
                degrees,
            } => format!("pinch to {percent}% turning {degrees} degrees at ({x}, {y})"),
            Step::Type(text) => format!("typing {text:?}"),
            Step::Commit(text) => format!("committing {text:?}"),
            Step::Preedit(text) => format!("composing {text:?}"),
            Step::Transaction {
                delete,
                commit,
                preedit,
                ..
            } => format!(
                "a transaction deleting {delete:?}, committing {commit:?}, composing {preedit:?}"
            ),
            Step::Keyboard { text, compose } => {
                format!("keys {text:?}{}", if *compose { ", composed" } else { "" })
            }
            Step::ImChange {
                delete,
                commit,
                preedit,
                stale,
            } => format!(
                "the input method deleting {delete:?}, committing {commit:?}, composing {preedit:?}{}",
                if *stale { " for an older state" } else { "" }
            ),
            Step::Drag { from, to } => format!("drag from {from:?} to {to:?}"),
            Step::DragOutput { from, to } => format!("drag from {from:?} to {to:?} on the output"),
            Step::Grab { from, by } => format!("grab at {from:?}, moved by {by:?}"),
            Step::DragPaced {
                from,
                to,
                motions,
                hz,
            } => format!("drag from {from:?} to {to:?} on the output in {motions} motions at {hz} Hz"),
            Step::ScrollPaced { at, dy, events, hz } => {
                format!("scroll by {dy} at {at:?} in {events} events at {hz} Hz")
            }
            Step::MovePaced {
                from,
                to,
                motions,
                hz,
            } => format!("pointer moved from {from:?} to {to:?} in {motions} motions at {hz} Hz"),
            Step::Key(codes) => format!("key {}", chord(codes)),
            Step::KeyDown(codes) => format!("key {} held", chord(codes)),
            Step::KeyUp(codes) => format!("key {} released", chord(codes)),
            Step::MoveWindow { dx, dy } => format!("window moved by ({dx}, {dy})"),
            Step::ResizeWindow { width, height } => format!("window resized to {width}x{height}"),
            Step::MinimizeWindow => "window minimized".to_string(),
            Step::CapsLock => "Caps Lock".to_string(),
            Step::Accessibility { action, name, .. } => format!("assistive {action:?} on {name:?}"),
            Step::Scale(scale) => format!("output scale {scale}/120"),
            Step::ExpectPopup => "a popup beyond the window".to_string(),
            Step::SwitchTerminal(number) => format!("a switch to terminal {number} and back"),
            Step::Idle(seconds) => format!("{seconds} s with no input"),
            Step::Wait(ms) => format!("{ms} ms waited"),
        }
    }

    fn json(&self) -> String {
        match self {
            Step::Press { x, y } => format!("\"press\":[{x},{y}]"),
            Step::PressPopup { x, y } => format!("\"press_popup\":[{x},{y}]"),
            Step::PressOutput { x, y } => format!("\"press_output\":[{x},{y}]"),
            Step::PressSecondary { x, y } => format!("\"press_secondary\":[{x},{y}]"),
            Step::DoublePress { x, y } => format!("\"double_press\":[{x},{y}]"),
            Step::Scroll { x, y, dy } => format!("\"scroll\":[{x},{y},{dy}]"),
            Step::Pinch {
                x,
                y,
                percent,
                degrees,
            } => format!("\"pinch\":[{x},{y},{percent},{degrees}]"),
            Step::Type(text) => format!("\"type\":{text:?}"),
            Step::Commit(text) => format!("\"commit\":{text:?}"),
            Step::Preedit(text) => format!("\"preedit\":{text:?}"),
            Step::Transaction {
                delete,
                commit,
                preedit,
                cursor,
                segments,
            } => {
                let segments: Vec<String> = segments
                    .iter()
                    .map(|(begin, end, style)| {
                        format!("[{begin},{end},\"{}\"]", style_name(*style))
                    })
                    .collect();
                format!(
                    "\"transaction\":{{\"delete\":[{},{}],\"commit\":{},\"preedit\":{},\"cursor\":[{},{}],\"segments\":[{}]}}",
                    delete.0,
                    delete.1,
                    json_string(commit),
                    json_string(preedit),
                    cursor.0,
                    cursor.1,
                    segments.join(",")
                )
            }
            Step::Keyboard { text, compose } => format!(
                "\"{}\":{}",
                if *compose { "compose_keys" } else { "keyboard" },
                json_string(text)
            ),
            Step::ImChange {
                delete,
                commit,
                preedit,
                stale,
            } => format!(
                "\"im_change\":{{\"delete\":[{},{}],\"commit\":{},\"preedit\":{},\"stale\":{stale}}}",
                delete.0,
                delete.1,
                json_string(commit),
                json_string(preedit)
            ),
            Step::Drag { from, to } => {
                format!("\"drag\":[{},{},{},{}]", from.0, from.1, to.0, to.1)
            }
            Step::DragOutput { from, to } => {
                format!("\"drag_output\":[{},{},{},{}]", from.0, from.1, to.0, to.1)
            }
            Step::Grab { from, by } => {
                format!("\"grab\":[{},{},{},{}]", from.0, from.1, by.0, by.1)
            }
            Step::DragPaced {
                from,
                to,
                motions,
                hz,
            } => format!(
                "\"drag_paced\":[{},{},{},{},{motions},{hz}]",
                from.0, from.1, to.0, to.1
            ),
            Step::ScrollPaced { at, dy, events, hz } => {
                format!("\"scroll_paced\":[{},{},{dy},{events},{hz}]", at.0, at.1)
            }
            Step::MovePaced {
                from,
                to,
                motions,
                hz,
            } => format!(
                "\"move_paced\":[{},{},{},{},{motions},{hz}]",
                from.0, from.1, to.0, to.1
            ),
            Step::Key(codes) => format!("\"key\":{codes:?}"),
            Step::KeyDown(codes) => format!("\"key_down\":{codes:?}"),
            Step::KeyUp(codes) => format!("\"key_up\":{codes:?}"),
            Step::MoveWindow { dx, dy } => format!("\"move_window\":[{dx},{dy}]"),
            Step::ResizeWindow { width, height } => {
                format!("\"resize_window\":[{width},{height}]")
            }
            Step::MinimizeWindow => "\"minimize_window\":true".to_string(),
            Step::CapsLock => "\"caps_lock\":true".to_string(),
            Step::Accessibility { action, name, .. } => {
                format!("\"a11y_action\":[\"{action:?}\",{}]", json_string(name))
            }
            Step::Scale(scale) => format!("\"scale\":{scale}"),
            Step::ExpectPopup => "\"expect_popup\":true".to_string(),
            Step::SwitchTerminal(number) => format!("\"switch_terminal\":{number}"),
            Step::Idle(seconds) => format!("\"idle\":{seconds}"),
            Step::Wait(ms) => format!("\"wait\":{ms}"),
        }
    }
}

fn style_name(style: PreeditStyle) -> &'static str {
    match style {
        PreeditStyle::Composing => "composing",
        PreeditStyle::Converting => "converting",
        PreeditStyle::Error => "error",
    }
}

/// `B-E:STYLE;...` (or `-` for none): a transaction's styled segments.
fn segments(value: &str) -> Result<Vec<(u32, u32, PreeditStyle)>> {
    if value == "-" {
        return Ok(Vec::new());
    }
    value
        .split(';')
        .map(|segment| {
            let (range, style) = segment.split_once(':').ok_or(USAGE)?;
            let (begin, end) = range.split_once('-').ok_or(USAGE)?;
            let style = [
                PreeditStyle::Composing,
                PreeditStyle::Converting,
                PreeditStyle::Error,
            ]
            .into_iter()
            .find(|s| style_name(*s) == style)
            .ok_or_else(|| format!("unknown segment style {style:?}"))?;
            Ok((begin.parse()?, end.parse()?, style))
        })
        .collect()
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
    /// `--gpu-only`: compose on the Vulkan device alone, with no CPU reference (on a display,
    /// once, straight into the scanout buffer); the director checks the frame the device
    /// composed, read back after it is shown. Measures the compositor as it would run.
    pub gpu_only: bool,
    /// `--dump-frames DIR`: the frame shown at launch and after each step, as layers
    /// (`oxyos_gpu::frame_file`), to compose again elsewhere (`oxyos-gpu`'s `replay` example).
    pub dump_frames: Option<PathBuf>,
    /// `--kms CARD`: show every composed frame on the card's first connected output (needs DRM
    /// master); frames complete at their flip times.
    pub kms: Option<PathBuf>,
    /// `--input`: the seat's devices through libinput drive the scene; `--uinput` also makes
    /// the director point, scroll and type through virtual devices instead of the scene.
    pub input: bool,
    pub uinput: bool,
    /// `--seat`: with `--kms`, run in the caller's logind session as its user, devices opened
    /// through logind, instead of as root taking the terminal (docs/adr/0006-seat.mdx).
    pub seat: bool,
    /// `--layout LAYOUT`: the XKB layout keys are typed and read with (`us` by default).
    pub layout: String,
    /// `--input-method`: the session plays an input method (ime.rs).
    pub input_method: bool,
    /// `--shell PROGRAM,ARG...`: the session's shell, started before the application on a socket
    /// granting `shell`, run as `PROGRAM ARG... --shell OUTPUT` (docs/adr/0009-session-shell.mdx).
    pub shell: Option<Vec<std::ffi::OsString>>,
    /// `--lock-password PASSWORD`: the lock screen's password unlocks when it is this one, the
    /// development compositor's fixture in place of the session's PAM (lock.rs).
    pub lock_password: Option<String>,
    /// `--reader PROGRAM`: the session's screen reader (docs/adr/0014-screen-reader.mdx), started
    /// before the application on a socket granting `accessibility` and `screen_reader`, its voice
    /// a record (`speech.jsonl` in the output directory); `--reader-locale LOCALE` is its
    /// language (`LC_ALL`), the session's otherwise.
    pub reader: Option<PathBuf>,
    pub reader_locale: Option<String>,
    /// `--preferences FILE`: the user's preferences, kept in FILE as the session keeps its own
    /// (docs/adr/0019-settings.mdx); the defaults, kept for the run alone, without it.
    pub preferences: Option<PathBuf>,
    /// `--grant-settings ID`: the application ID is given `settings` when it is launched, as a
    /// session gives it to the system application the image's policy names. A fixture: a
    /// scenario's entries are the developer's.
    pub grant_settings: Option<String>,
    /// `--grant ID=GRANT,...`: the application ID is given those system grants (`settings`,
    /// `store`, `updates`, `privacy`) when it is launched, as the image's policy gives them: a
    /// fixture, as `--grant-settings` is.
    pub grants: Vec<(String, oxyos_apps::Grants)>,
    /// `--installd SOCKET`: the install service the store's, Settings' and applications'
    /// requests go to (ADR 0030), one a scenario runs on directories of its own.
    pub installd: Option<PathBuf>,
    /// `--runtimes DIR`: where the runtimes an application's entry names are
    /// (docs/adr/0030-installing-applications.mdx, decision 2): a scenario's, in place of the
    /// image's (`/usr/lib/oxyos/runtimes`).
    pub runtimes: Option<PathBuf>,
    /// `--installed-fixture ROOT`: the install service's state directory, as a scenario wrote it
    /// (docs/adr/0030-installing-applications.mdx, decision 7): its entries for this user are the
    /// registry's second source, taken as the service's. A fixture: a session's are root's.
    pub installed_fixture: Option<PathBuf>,
    /// `--controls FILE`: the machine the hardware keys and the Control Center act on
    /// (docs/adr/0025-hardware-keys-and-control-center.mdx), played as FILE describes it
    /// (`oxyos_controls::Fake`), which it writes back as the session changes it; each step's
    /// result says it (`controls`). Without it the keys act on nothing and the shell is told
    /// nothing of the machine.
    pub controls: Option<PathBuf>,
    /// `--outputs WxH[@SCALE][:HZ],...`: the session's outputs, of these pixels at these scales,
    /// arranged as a session arranges its displays (docs/adr/0018-several-displays.mdx): left to
    /// right, top-aligned, the first the primary; a shell runs on each. One of 1280x720 at 1
    /// without it. With a refresh, an output is paced as a display at that rate: each frame is
    /// shown at its next vblank, and its surfaces are told the rate; without one, a frame is
    /// shown as soon as it is composed, and its surfaces are told 60 Hz.
    pub outputs: Vec<OutputSpec>,
    /// `--displays FILE`: the session's displays instead, on the virtual connectors FILE describes
    /// (displays.rs), shown, arranged and changed from Settings as a session's are
    /// (docs/adr/0018-several-displays.mdx), as the preferences keep them; a shell runs on each.
    pub displays: Option<PathBuf>,
    /// `--login SOCKET`: the session is a login screen (docs/adr/0020-oxy-account-login.mdx),
    /// locked from the start, with no application: the shell (the Bloom host's `OxyOSLogin`) is
    /// all there is, and its passwords go to the login program listening at SOCKET (`oxyos-login
    /// --dev`), for the account its lock surface names, never to this fixture or to PAM here.
    pub login: Option<PathBuf>,
}

const USAGE: &str = "usage: oxyos-offscreen --launch [--out DIR] [--outputs WxH[@SCALE][:HZ],... | --displays FILE] [--gpu | --gpu-only] [--dump-frames DIR] [--kms CARD] [--input | --uinput] [--seat] [--layout LAYOUT] [--input-method] [--shell PROGRAM,ARG...] [--lock-password PASSWORD] [--login SOCKET] [--reader PROGRAM [--reader-locale LOCALE]] [--preferences FILE] [--grant-settings ID] [--grant ID=GRANT,...] [--installd SOCKET] [--runtimes DIR] [--installed-fixture ROOT] [--controls FILE] [[--allow-unchanged] --press X,Y | \
                     --press-popup X,Y | --press-output X,Y | --scroll X,Y,DY | --pinch X,Y,PERCENT,DEGREES | --type TEXT | --preedit TEXT | --commit TEXT | --transaction BEFORE,AFTER COMMIT PREEDIT BEGIN,END SEGMENTS|- | --keyboard TEXT | --compose-keys TEXT | --im-change BEFORE,AFTER COMMIT PREEDIT current|stale | --key CODE[+CODE]... | --key-down CODE[+CODE]... | --key-up CODE[+CODE]... | --drag X0,Y0,X1,Y1 | --drag-output X0,Y0,X1,Y1 | --drag-paced X0,Y0,X1,Y1,MOTIONS,HZ | --scroll-paced X,Y,DY,EVENTS,HZ | --move-paced X0,Y0,X1,Y1,MOTIONS,HZ | --scale SCALE_120 | --a11y-action activate|focus|increment|decrement|scroll-forward|scroll-backward|dismiss|custom:LABEL|select:START-END,NAME | --move-window DX,DY | --resize-window W,H | --minimize-window | --caps-lock | --switch-terminal N | --idle SECONDS | --wait MS | --expect-popup]... \
                     -- COMMAND [ARGS]... (no COMMAND with --login)";

/// An output of `--outputs`: its pixels, its scale (in 120ths) and the refresh it is paced at, if
/// given (in mHz).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputSpec {
    pub pixels: (u32, u32),
    pub scale_120: u32,
    pub refresh_mhz: Option<u32>,
}

impl OutputSpec {
    /// Of these pixels at scale 1, unpaced.
    pub const fn of(width: u32, height: u32) -> Self {
        Self {
            pixels: (width, height),
            scale_120: 120,
            refresh_mhz: None,
        }
    }
}

/// `WxH[@SCALE][:HZ],...`: each output's pixels, scale (1 without one) and refresh in hertz (59.94,
/// 120, 143.998; unpaced without one).
fn outputs(value: &str) -> Result<Vec<OutputSpec>> {
    value
        .split(',')
        .map(|output| {
            let (rest, refresh) = match output.split_once(':') {
                Some((rest, hz)) => {
                    let hz: f64 = hz.parse()?;
                    if !(1.0..=1000.0).contains(&hz) {
                        return Err(format!("{output:?}: a refresh from 1 to 1000 Hz").into());
                    }
                    (rest, Some((hz * 1000.0).round() as u32))
                }
                None => (output, None),
            };
            let (size, scale) = rest.split_once('@').unwrap_or((rest, "1"));
            let (width, height) = size.split_once('x').ok_or(USAGE)?;
            let scale: f64 = scale.parse()?;
            let scale_120 = (scale * 120.0).round();
            if !(120.0..=480.0).contains(&scale_120) {
                return Err(format!("{output:?}: a scale from 1 to 4").into());
            }
            Ok(OutputSpec {
                pixels: (width.parse()?, height.parse()?),
                scale_120: scale_120 as u32,
                refresh_mhz: refresh,
            })
        })
        .collect()
}

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
        let mut out = PathBuf::from("target/oxyos-offscreen");
        let mut steps: Vec<(Step, bool)> = Vec::new();
        // `--allow-unchanged` applies to the step after it.
        let mut allow = false;
        let (mut gpu, mut gpu_only) = (false, false);
        let mut dump_frames = None;
        let mut kms = None;
        let (mut input, mut uinput, mut layout) = (false, false, "us".to_string());
        let (mut seat, mut input_method) = (false, false);
        let mut shell = None;
        let mut lock_password = None;
        let (mut reader, mut reader_locale) = (None, None);
        let (mut preferences, mut grant_settings, mut controls) = (None, None, None);
        let (mut grants, mut installd) = (Vec::new(), None);
        let mut runtimes = None;
        let mut installed_fixture = None;
        let mut screens = Vec::new();
        let mut displays = None;
        let mut command = Vec::new();
        let mut login = None;
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
                Some("--gpu-only") => {
                    gpu = true;
                    gpu_only = true;
                    continue;
                }
                Some("--input") => {
                    input = true;
                    continue;
                }
                Some("--uinput") => {
                    input = true;
                    uinput = true;
                    continue;
                }
                Some("--seat") => {
                    seat = true;
                    continue;
                }
                Some("--shell") => {
                    let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    shell = Some(value.split(',').map(Into::into).collect());
                    continue;
                }
                Some("--reader") => {
                    reader = Some(args.next().ok_or(USAGE)?.into());
                    continue;
                }
                Some("--reader-locale") => {
                    let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    reader_locale = Some(value.to_string());
                    continue;
                }
                Some("--input-method") => {
                    input_method = true;
                    continue;
                }
                Some("--preferences") => {
                    preferences = Some(args.next().ok_or(USAGE)?.into());
                    continue;
                }
                Some("--controls") => {
                    controls = Some(args.next().ok_or(USAGE)?.into());
                    continue;
                }
                Some("--installed-fixture") => {
                    installed_fixture = Some(args.next().ok_or(USAGE)?.into());
                    continue;
                }
                Some("--runtimes") => {
                    runtimes = Some(args.next().ok_or(USAGE)?.into());
                    continue;
                }
                Some("--grant") => {
                    let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    let (id, list) = value.split_once('=').ok_or(USAGE)?;
                    let names: Vec<String> = list.split(',').map(str::to_string).collect();
                    let granted = oxyos_apps::Grants::named(&names).map_err(|_| USAGE)?;
                    grants.push((id.to_string(), granted));
                    continue;
                }
                Some("--installd") => {
                    installd = Some(args.next().ok_or(USAGE)?.into());
                    continue;
                }
                Some("--grant-settings") => {
                    let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    grant_settings = Some(value.to_string());
                    continue;
                }
                Some("--outputs") => {
                    let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    screens = outputs(value)?;
                    continue;
                }
                Some("--displays") => {
                    displays = Some(PathBuf::from(args.next().ok_or(USAGE)?));
                    continue;
                }
                Some("--lock-password") => {
                    let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    lock_password = Some(value.to_string());
                    continue;
                }
                Some("--layout") => {
                    layout = args
                        .next()
                        .and_then(|l| l.to_str())
                        .ok_or(USAGE)?
                        .to_string();
                    continue;
                }
                Some("--kms") => {
                    kms = Some(args.next().ok_or(USAGE)?.into());
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
                Some("--press-output") => {
                    let n = numbers(args.next(), 2)?;
                    Step::PressOutput { x: n[0], y: n[1] }
                }
                Some("--press-secondary") => {
                    let n = numbers(args.next(), 2)?;
                    Step::PressSecondary { x: n[0], y: n[1] }
                }
                Some("--double-press") => {
                    let n = numbers(args.next(), 2)?;
                    Step::DoublePress { x: n[0], y: n[1] }
                }
                Some("--scroll") => {
                    let n = numbers(args.next(), 3)?;
                    Step::Scroll {
                        x: n[0],
                        y: n[1],
                        dy: n[2],
                    }
                }
                Some("--pinch") => {
                    let n = numbers(args.next(), 4)?;
                    Step::Pinch {
                        x: n[0],
                        y: n[1],
                        percent: n[2],
                        degrees: n[3],
                    }
                }
                Some("--type") => {
                    let text = args.next().and_then(|t| t.to_str()).ok_or(USAGE)?;
                    Step::Type(text.to_string())
                }
                Some(flag @ ("--key" | "--key-down" | "--key-up")) => {
                    let codes = args.next().and_then(|c| c.to_str()).ok_or(USAGE)?;
                    let codes = codes
                        .split('+')
                        .map(str::parse)
                        .collect::<std::result::Result<_, _>>()?;
                    match flag {
                        "--key" => Step::Key(codes),
                        "--key-down" => Step::KeyDown(codes),
                        _ => Step::KeyUp(codes),
                    }
                }
                Some("--drag") => {
                    let n = numbers(args.next(), 4)?;
                    Step::Drag {
                        from: (n[0], n[1]),
                        to: (n[2], n[3]),
                    }
                }
                Some("--drag-output") => {
                    let n = numbers(args.next(), 4)?;
                    Step::DragOutput {
                        from: (n[0], n[1]),
                        to: (n[2], n[3]),
                    }
                }
                Some("--grab") => {
                    let n = numbers(args.next(), 4)?;
                    Step::Grab {
                        from: (n[0], n[1]),
                        by: (n[2], n[3]),
                    }
                }
                Some("--drag-paced") => {
                    let n = numbers(args.next(), 6)?;
                    if n[4] < 1 || n[5] < 1 {
                        return Err("--drag-paced needs at least one motion at 1 Hz or more".into());
                    }
                    Step::DragPaced {
                        from: (n[0], n[1]),
                        to: (n[2], n[3]),
                        motions: n[4] as u32,
                        hz: n[5] as u32,
                    }
                }
                Some("--scroll-paced") => {
                    let n = numbers(args.next(), 5)?;
                    if n[3] < 1 || n[4] < 1 {
                        return Err(
                            "--scroll-paced needs at least one event at 1 Hz or more".into()
                        );
                    }
                    Step::ScrollPaced {
                        at: (n[0], n[1]),
                        dy: n[2],
                        events: n[3] as u32,
                        hz: n[4] as u32,
                    }
                }
                Some("--move-paced") => {
                    let n = numbers(args.next(), 6)?;
                    if n[4] < 1 || n[5] < 1 {
                        return Err("--move-paced needs at least one motion at 1 Hz or more".into());
                    }
                    Step::MovePaced {
                        from: (n[0], n[1]),
                        to: (n[2], n[3]),
                        motions: n[4] as u32,
                        hz: n[5] as u32,
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
                Some("--minimize-window") => Step::MinimizeWindow,
                Some("--caps-lock") => Step::CapsLock,
                Some("--a11y-action") => {
                    let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    let (spec, name) = value.split_once(',').ok_or(USAGE)?;
                    let (mut custom, mut selection) = (None, (0, 0));
                    let action = match spec {
                        "activate" => AccessibleAction::Activate,
                        "focus" => AccessibleAction::Focus,
                        "increment" => AccessibleAction::Increment,
                        "decrement" => AccessibleAction::Decrement,
                        "scroll-forward" => AccessibleAction::ScrollForward,
                        "scroll-backward" => AccessibleAction::ScrollBackward,
                        "dismiss" => AccessibleAction::Dismiss,
                        _ if spec.starts_with("custom:") => {
                            custom = Some(spec["custom:".len()..].to_string());
                            AccessibleAction::Custom
                        }
                        _ if spec.starts_with("select:") => {
                            let range = &spec["select:".len()..];
                            let (start, end) = range.split_once('-').ok_or(USAGE)?;
                            selection = (start.parse()?, end.parse()?);
                            AccessibleAction::SetSelection
                        }
                        _ => return Err(format!("unknown action {spec:?}\n{USAGE}").into()),
                    };
                    Step::Accessibility {
                        action,
                        name: name.to_string(),
                        custom,
                        selection,
                    }
                }
                Some("--idle") => {
                    let seconds = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    Step::Idle(seconds.parse()?)
                }
                Some("--wait") => {
                    let ms = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    Step::Wait(ms.parse()?)
                }
                Some("--scale") => {
                    let scale = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    Step::Scale(scale.parse()?)
                }
                Some("--preedit") => {
                    let text = args.next().and_then(|t| t.to_str()).ok_or(USAGE)?;
                    Step::Preedit(text.to_string())
                }
                Some(flag @ ("--keyboard" | "--compose-keys")) => {
                    let text = args.next().and_then(|t| t.to_str()).ok_or(USAGE)?;
                    Step::Keyboard {
                        text: text.to_string(),
                        compose: flag == "--compose-keys",
                    }
                }
                Some("--im-change") => {
                    let delete = numbers(args.next(), 2)?;
                    let mut text = || args.next().and_then(|t| t.to_str()).ok_or(USAGE);
                    let (commit, preedit) = (text()?.to_string(), text()?.to_string());
                    let stale = match text()? {
                        "current" => false,
                        "stale" => true,
                        other => return Err(format!("--im-change for {other:?}\n{USAGE}").into()),
                    };
                    Step::ImChange {
                        delete: (u32::try_from(delete[0])?, u32::try_from(delete[1])?),
                        commit,
                        preedit,
                        stale,
                    }
                }
                Some("--transaction") => {
                    let delete = numbers(args.next(), 2)?;
                    let mut text = || args.next().and_then(|t| t.to_str()).ok_or(USAGE);
                    let (commit, preedit) = (text()?.to_string(), text()?.to_string());
                    let cursor = numbers(args.next(), 2)?;
                    let styled = args.next().and_then(|t| t.to_str()).ok_or(USAGE)?;
                    Step::Transaction {
                        delete: (u32::try_from(delete[0])?, u32::try_from(delete[1])?),
                        commit,
                        preedit,
                        cursor: (u32::try_from(cursor[0])?, u32::try_from(cursor[1])?),
                        segments: segments(styled)?,
                    }
                }
                Some("--switch-terminal") => {
                    let number = args.next().and_then(|n| n.to_str()).ok_or(USAGE)?;
                    Step::SwitchTerminal(number.parse()?)
                }
                Some("--commit") => {
                    let text = args.next().and_then(|t| t.to_str()).ok_or(USAGE)?;
                    Step::Commit(text.to_string())
                }
                Some("--") => {
                    command = args.by_ref().cloned().collect();
                    if command.is_empty() {
                        return Err("no command after --".into());
                    }
                    break;
                }
                Some("--login") => {
                    login = Some(args.next().ok_or(USAGE)?.into());
                    continue;
                }
                _ => return Err(format!("unexpected argument {arg:?}\n{USAGE}").into()),
            };
            steps.push((step, std::mem::take(&mut allow)));
        }
        // A login screen has no application; anything else is shown with one.
        if command.is_empty() && login.is_none() {
            return Err(USAGE.into());
        }
        Ok((
            out,
            Self {
                command,
                steps,
                gpu,
                gpu_only,
                dump_frames,
                kms,
                input,
                uinput,
                seat,
                layout,
                input_method,
                shell,
                lock_password,
                reader,
                reader_locale,
                preferences,
                grant_settings,
                grants,
                installd,
                runtimes,
                installed_fixture,
                controls,
                outputs: screens,
                displays,
                login,
            },
        ))
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

/// The shell's surfaces, bottom to top: clients of the shell socket (numbered down from the top,
/// below the scene-only clients the session plays).
fn shell_surfaces(compositor: &Compositor) -> Vec<(SurfaceKey, u64, Rect)> {
    compositor
        .scene
        .render_list()
        .into_iter()
        .filter_map(|item| match item {
            RenderItem::Surface { key, rect, frame }
                if key.client > ClientId::MAX / 2 && key.client < crate::ime::IME =>
            {
                Some((key, frame.sequence, rect))
            }
            _ => None,
        })
        .collect()
}

/// Starts the shell on a socket of its own, granting `shell`, one on each output as a session
/// starts them (docs/adr/0018-several-displays.mdx), and serves the session until their panels have
/// set every output's work area (as `oxyos-session` waits for them) and their surfaces are then
/// unchanged for SETTLE. A shell's background is made before its JavaScript runs, so surfaces
/// unchanged alone may be that background, still without panels, on a loaded machine: the
/// application started then would be placed in the whole output.
fn start_shell(
    session: &mut Session,
    command: &[std::ffi::OsString],
) -> Result<Vec<std::process::Child>> {
    let socket = session.socket.with_file_name("shell");
    let listener = oxyos_ipc::Listener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    session.compositor.shell_listener = Some(listener);
    let outputs: Vec<OutputId> = session.compositor.screens.iter().map(|s| s.id).collect();
    let mut children = Vec::new();
    for output in outputs {
        let mut shell = Command::new(command.first().ok_or(USAGE)?);
        shell
            .args(&command[1..])
            .arg("--shell")
            .arg(output.to_string())
            .env("OXYOS_COMPOSITOR_SOCKET", &socket)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        // The shell lists the fixture's installed applications too (ADR 0030, decision 7).
        if let Some(root) = &session.installed_fixture {
            shell.env("OXYOS_INSTALLED_FIXTURE", root);
        }
        // A Bloom host's bundle is known to its JavaScript by its source's URL, beside its
        // assets: a package's compiled bundle by the source its manifest names (ADR 0030).
        if let Some(bundle) = command
            .get(1)
            .map(std::path::Path::new)
            .filter(|b| b.is_file())
        {
            let url = oxyos_apps::script_url(bundle);
            shell.env("OXYOS_SCRIPT_URL", format!("file://{}", url.display()));
        }
        // A Bloom host's report is its own: the primary's shell's goes beside the application's,
        // another output's beside that.
        if let Some(mut report) = std::env::var_os("OXYOS_HOST_REPORT") {
            report.push(".shell");
            if output != OUTPUT_ID {
                report.push(format!(".{output}"));
            }
            shell.env("OXYOS_HOST_REPORT", report);
        }
        children.push(shell.spawn()?);
    }
    let deadline = Instant::now() + LAUNCH_DEADLINE;
    let mut last = (Vec::new(), Instant::now());
    loop {
        if Instant::now() > deadline {
            for child in &mut children {
                let _ = child.kill();
            }
            return Err("deadline exceeded waiting for the shell".into());
        }
        for child in &mut children {
            if let Some(status) = child.try_wait()? {
                return Err(format!("the shell exited ({status}) before it was shown").into());
            }
        }
        session.compositor.serve(&session.listener)?;
        let now = shell_surfaces(&session.compositor);
        let scene = &session.compositor.scene;
        // A login screen keeps nothing out of the work area: its lock surfaces are what it shows.
        let reserved = if session.compositor.login {
            !scene
                .render_list()
                .iter()
                .any(|item| matches!(item, RenderItem::Cover { .. }))
        } else {
            scene
                .outputs()
                .iter()
                .all(|output| scene.work_area(output.id) != Some(output.rect))
        };
        if now != last.0 {
            last = (now, Instant::now());
        } else if reserved && last.1.elapsed() >= SETTLE {
            eprintln!("[shell] {} surfaces shown", last.0.len());
            return Ok(children);
        }
    }
}

/// Starts the screen reader on a socket of its own, granting `accessibility` and `screen_reader`,
/// saying what it says into `record`, and serves the session until it is connected.
fn start_reader(
    session: &mut Session,
    program: &Path,
    locale: Option<&str>,
    record: &Path,
) -> Result<std::process::Child> {
    let socket = session.socket.with_file_name("reader");
    let listener = oxyos_ipc::Listener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    session.compositor.reader_listener = Some(listener);
    if record.exists() {
        fs::remove_file(record)?;
    }
    let mut reader = Command::new(program);
    reader
        .env("OXYOS_COMPOSITOR_SOCKET", &socket)
        .env("OXYOS_SPEECH", format!("record:{}", record.display()))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    if let Some(locale) = locale {
        reader.env("LC_ALL", locale);
    }
    let mut child = reader.spawn()?;
    let deadline = Instant::now() + LAUNCH_DEADLINE;
    while session.compositor.scene.screen_reader().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err("deadline exceeded waiting for the screen reader".into());
        }
        if let Some(status) = child.try_wait()? {
            return Err(format!("the screen reader exited ({status}) before it connected").into());
        }
        session.compositor.serve(&session.listener)?;
    }
    eprintln!("[reader] connected");
    Ok(child)
}

/// The shell's surfaces and the work area they leave (the primary's, and each output's), as a
/// step records them.
fn shell_json(compositor: &Compositor) -> String {
    let rect = |r: &Rect| format!("[{},{},{},{}]", r.x, r.y, r.width, r.height);
    let surfaces: Vec<String> = shell_surfaces(compositor)
        .iter()
        .map(|(_, _, r)| rect(r))
        .collect();
    let area = compositor
        .scene
        .work_area(OUTPUT_ID)
        .map_or("null".to_string(), |r| rect(&r));
    let areas: Vec<String> = compositor
        .scene
        .work_areas()
        .iter()
        .map(|(id, r)| format!("\"{id}\":{}", rect(r)))
        .collect();
    format!(
        "{{\"surfaces\":[{}],\"work_area\":{area},\"work_areas\":{{{}}},\"locked\":{},\"signed_in\":{}}}",
        surfaces.join(","),
        areas.join(","),
        compositor.scene.is_locked(),
        compositor.signed_in
    )
}

/// The application's first popup, if one is shown.
fn popup(compositor: &Compositor) -> Option<Rect> {
    shown(compositor)
        .into_iter()
        .find(|(id, _, _)| *id != WINDOW)
        .map(|(_, _, rect)| rect)
}

/// CLOCK_MONOTONIC in nanoseconds, the clock the application's report (OXYOS_HOST_REPORT) uses.
fn median(values: &[f64]) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.get(sorted.len() / 2).copied()
}

/// The frames composed since `from_ns` (CLOCK_MONOTONIC), as JSON: each [ms from then to when its
/// composition started, how long composing it took, how long of that reading the clients'
/// buffers, in ms] (`Compositor::frame_times`).
fn composed_since(compositor: &Compositor, from_ns: i64) -> String {
    let frames: Vec<String> = compositor
        .frame_times
        .iter()
        .filter_map(|&(started, ms, read)| {
            let at = started.as_nanos() as i64 - from_ns;
            (at >= 0).then(|| format!("[{:.2},{ms:.2},{read:.2}]", at as f64 / 1e6))
        })
        .collect();
    format!("[{}]", frames.join(","))
}

fn monotonic_ns() -> i64 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    now.tv_sec * 1_000_000_000 + now.tv_nsec
}

/// A paced input's time: `due` after `origin` (CLOCK_MONOTONIC ns), the time it is scheduled for,
/// which its events carry as their device's time (`Scene::set_input_time`) whenever the
/// compositor got to them.
fn paced_time(origin: i64, due: Duration) -> u64 {
    u64::try_from(origin).unwrap_or(0) + u64::try_from(due.as_nanos()).unwrap_or(u64::MAX)
}

/// When a step's answer first appeared, when it settled, and the window then.
struct Settled {
    window: Rect,
    first_frame_ns: Option<i64>,
    /// On a display: the kernel's flip time of the first frame showing the answer.
    first_flip_ns: Option<i64>,
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
    // Where the window was: a step may leave it off the screen (minimized), never the first frame.
    window_before: Option<Rect>,
) -> Result<Settled> {
    // The shell's surfaces count too: a press on a panel changes the shell, not the application.
    let mut last = (before.to_vec(), Instant::now());
    let mut shell_last = shell_surfaces(&session.compositor);
    let mut first_frame_ns = None;
    let mut first_flip_ns = None;
    let shown_before = session.compositor.screens[0].flips.len();
    // The flip that will show the answer, when it was not yet shown as it appeared.
    let mut answer_flip = None;
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
        session.serve_launches();
        session.serve_installs();
        session.serve_preferences();
        if let Some(ime) = session.ime.as_mut() {
            ime.serve(&mut session.compositor)?;
        }
        session.reader.absorb(&mut session.compositor);
        let now = shown(&session.compositor);
        let shell_now = shell_surfaces(&session.compositor);
        let shell_changed = shell_now != shell_last;
        if shell_changed {
            shell_last = shell_now;
            last.1 = Instant::now();
            if first_frame_ns.is_none() && last.0 == before {
                first_frame_ns = Some(monotonic_ns());
            }
        }
        let changed = now != last.0;
        if changed {
            if first_frame_ns.is_none() {
                first_frame_ns = Some(monotonic_ns());
                // The display's (the primary's: the fixture shows one output).
                let compositor = &session.compositor;
                let screen = &compositor.screens[0];
                if screen.flipped == compositor.scene.render_list() {
                    // The last flip shows the answer: its completion made it.
                    first_flip_ns = screen.flips[shown_before..]
                        .last()
                        .map(|(_, flip)| flip.as_nanos() as i64);
                } else {
                    // Not on screen yet (a surface went away, say): the frame on its way shows it
                    // if it was composed with it, or else the next one.
                    let composed_with_it = screen.pending_flip.is_some()
                        && screen.composed == compositor.scene.frame_list();
                    answer_flip = Some(
                        screen.flips.len()
                            + usize::from(screen.pending_flip.is_some() && !composed_with_it),
                    );
                }
            }
            last = (now, Instant::now());
        }
        if let Some(index) = answer_flip {
            if let Some((_, flip)) = session.compositor.screens[0].flips.get(index) {
                first_flip_ns = Some(flip.as_nanos() as i64);
                answer_flip = None;
            }
        }
        // A window configured (zoomed, resized) has answered only once it shows a frame for its
        // latest configure: its last frame may stay unchanged a while before that one comes.
        let window = SurfaceKey {
            client: CLIENT,
            id: WINDOW,
        };
        let answered = session
            .compositor
            .scene
            .render_list()
            .iter()
            .find_map(|item| match item {
                RenderItem::Surface { key, frame, .. } if *key == window => Some(frame.configure),
                _ => None,
            })
            .is_none_or(|shown| Some(shown) == session.compositor.scene.configure_serial(window));
        if !changed
            && !shell_changed
            && answered
            && (last.0 != before || first_frame_ns.is_some() || may_stay)
            && last.1.elapsed() >= SETTLE
        {
            if let Some(window) = session
                .compositor
                .surface_rect(CLIENT, WINDOW)
                .or(window_before)
            {
                return Ok(Settled {
                    window,
                    first_frame_ns,
                    first_flip_ns,
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

/// What the application's focused surface told of its text field (ADR 0008), or null.
fn field_json(compositor: &Compositor) -> String {
    let Some(key) = compositor.scene.focus().filter(|key| key.client == CLIENT) else {
        return "null".to_string();
    };
    let Some(field) = compositor.scene.text_input(key) else {
        return "null".to_string();
    };
    let caret = field.caret.map_or("null".to_string(), |rect| {
        format!("[{},{},{},{}]", rect.x, rect.y, rect.width, rect.height)
    });
    format!(
        "{{\"surface\":{},\"purpose\":\"{:?}\",\"hints\":{},\"text\":{},\"anchor\":{},\"cursor\":{},\"caret\":{caret},\"serial\":{}}}",
        key.id,
        field.purpose,
        field.hints.bits(),
        json_string(&field.surrounding),
        field.anchor,
        field.cursor,
        field.serial,
    )
}

/// The input routed during a step: for the application, counts per surface and kind; for the
/// session's shell, its count (`shell`); for the applications the shell launched, counts per kind
/// (`launched`); for any other client, a count (`leaked`).
fn delivered_json(session: &Session) -> String {
    let compositor = &session.compositor;
    let mut counts: std::collections::BTreeMap<u32, std::collections::BTreeMap<&str, usize>> =
        Default::default();
    let mut launched: std::collections::BTreeMap<&str, usize> = Default::default();
    let (mut shell, mut leaked) = (0, 0);
    for &(client, surface, kind) in &compositor.input_log {
        if client == CLIENT {
            *counts.entry(surface).or_default().entry(kind).or_default() += 1;
        } else if client > ClientId::MAX / 2 && client < crate::ime::IME {
            shell += 1;
        } else if session.launched_clients.contains(&client) {
            *launched.entry(kind).or_default() += 1;
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
    if shell > 0 {
        fields.push(format!("\"shell\":{shell}"));
    }
    // What applications the shell launched received, whichever window (ADR 0013).
    if !launched.is_empty() {
        let kinds: Vec<String> = launched
            .iter()
            .map(|(k, n)| format!("\"{k}\":{n}"))
            .collect();
        fields.push(format!("\"launched\":{{{}}}", kinds.join(",")));
    }
    fields.push(format!("\"leaked\":{leaked}"));
    format!("{{{}}}", fields.join(","))
}

/// The played machine as it is (ADR 0025): each of its lines as a key, numbers and flags as
/// themselves, the rest as text.
fn controls_json(fake: &oxyos_controls::Fake) -> String {
    let fields: Vec<String> = fake
        .text()
        .lines()
        .filter_map(|line| line.split_once(" = "))
        .map(|(key, value)| {
            let value = if value == "true" || value == "false" || value.parse::<f64>().is_ok() {
                value.to_string()
            } else {
                json_string(value)
            };
            format!("{}:{value}", json_string(key))
        })
        .collect();
    format!("{{{}}}", fields.join(","))
}

pub fn json_string(text: &str) -> String {
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

/// The assistive technology acts on the node named `name` it holds on the topmost shown surface
/// that has one (the application's, the shell's or the lock screen's), as a screen reader acts on
/// what is on screen. A custom action is named by the label the node gives it; a custom label
/// the node does not have is sent as an index past its own, which the compositor refuses.
fn act(
    session: &mut Session,
    action: AccessibleAction,
    name: &str,
    custom: Option<&str>,
    selection: (u32, u32),
) -> Result<()> {
    let shown: Vec<SurfaceKey> = session
        .compositor
        .scene
        .render_list()
        .into_iter()
        .filter_map(|item| match item {
            RenderItem::Surface { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    let (key, index) = shown
        .into_iter()
        .rev()
        .find_map(|key| {
            let nodes = session.reader.trees.get(&key)?;
            let index = nodes.iter().position(|node| node.name == name)?;
            Some((key, index as u32))
        })
        .ok_or_else(|| format!("the assistive technology holds no node named {name:?}"))?;
    let labels = &session.reader.trees[&key][index as usize].custom;
    let custom = custom.map_or(0, |label| {
        labels
            .iter()
            .position(|held| held == label)
            .unwrap_or(labels.len()) as u32
    });
    session
        .compositor
        .scene
        .handle(
            READER,
            Request::AccessibilityAction {
                client: key.client,
                surface: key.id,
                index,
                action,
                custom,
                start: selection.0,
                end: selection.1,
            },
        )
        .map_err(|code| format!("accessibility action: {code:?}").into())
}

/// The name a step's report gives a surface: the application's by its id, the session's
/// shell's as `shell-<id>`, any other client's as `<client>-<id>`.
fn surface_name(key: SurfaceKey) -> String {
    if key.client == CLIENT {
        key.id.to_string()
    } else if key.client > ClientId::MAX / 2 && key.client < crate::ime::IME {
        format!("shell-{}", key.id)
    } else {
        format!("{}-{}", key.client, key.id)
    }
}

/// What the reader would have said in a step: [{politeness, text, live}].
fn announcements_json(announcements: &[crate::reader::Announcement]) -> String {
    let items: Vec<String> = announcements
        .iter()
        .map(|said| {
            format!(
                "{{\"politeness\":\"{}\",\"text\":{},\"live\":{}}}",
                match said.politeness {
                    oxyos_protocol::Politeness::Polite => "polite",
                    oxyos_protocol::Politeness::Assertive => "assertive",
                },
                json_string(&said.text),
                said.live
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// What the assistive technology holds, as it received it (reader.rs), by surface name:
/// {surface: [{role, name, value, state, actions, custom, selection}]}.
fn accessibility_json(reader: &crate::reader::Reader) -> String {
    let states = [
        (AccessibleState::DISABLED, "disabled"),
        (AccessibleState::SELECTED, "selected"),
        (AccessibleState::CHECKED, "checked"),
        (AccessibleState::MIXED, "mixed"),
        (AccessibleState::EXPANDED, "expanded"),
        (AccessibleState::COLLAPSED, "collapsed"),
        (AccessibleState::BUSY, "busy"),
        (AccessibleState::FOCUSED, "focused"),
        (AccessibleState::LIVE_POLITE, "live-polite"),
        (AccessibleState::LIVE_ASSERTIVE, "live-assertive"),
    ];
    let offered = [
        (oxyos_protocol::AccessibleActions::INCREMENT, "increment"),
        (oxyos_protocol::AccessibleActions::DECREMENT, "decrement"),
        (
            oxyos_protocol::AccessibleActions::SCROLL_FORWARD,
            "scroll-forward",
        ),
        (
            oxyos_protocol::AccessibleActions::SCROLL_BACKWARD,
            "scroll-backward",
        ),
        (oxyos_protocol::AccessibleActions::DISMISS, "dismiss"),
        (oxyos_protocol::AccessibleActions::SET_SELECTION, "select"),
    ];
    let surfaces: Vec<String> = reader
        .trees
        .iter()
        .map(|(key, nodes)| {
            let nodes: Vec<String> = nodes
                .iter()
                .map(|node| {
                    let state: Vec<String> = states
                        .iter()
                        .filter(|(bit, _)| node.state.contains(*bit))
                        .map(|(_, name)| format!("\"{name}\""))
                        .collect();
                    let actions: Vec<String> = offered
                        .iter()
                        .filter(|(bit, _)| node.actions.contains(*bit))
                        .map(|(_, name)| format!("\"{name}\""))
                        .collect();
                    let custom: Vec<String> =
                        node.custom.iter().map(|label| json_string(label)).collect();
                    let selection = node
                        .selection
                        .map_or("null".to_string(), |(start, end)| format!("[{start},{end}]"));
                    format!(
                        "{{\"role\":\"{:?}\",\"name\":{},\"value\":{},\"state\":[{}],\"actions\":[{}],\"custom\":[{}],\"selection\":{selection}}}",
                        node.role,
                        json_string(&node.name),
                        json_string(&node.value),
                        state.join(","),
                        actions.join(","),
                        custom.join(",")
                    )
                })
                .collect();
            format!("\"{}\":[{}]", surface_name(*key), nodes.join(","))
        })
        .collect();
    format!("{{{}}}", surfaces.join(","))
}

/// The application's popups now shown, as [x, y, width, height] on the output.
/// The transitions a step ran (docs/adr/0011-transitions.mdx): the frames composed while any
/// ran, and for each its kind, whether a surface arrived, the opacities of the first and last
/// frames drawn in it, how many those were, whether it ran to its end, and each frame drawn
/// (`path`: its time in the transition, its opacity, and how far along its size and place were,
/// in thousandths). Taken: the next step reports its own.
fn transitions_json(compositor: &mut Compositor) -> String {
    let seen: Vec<String> = compositor
        .transitions
        .take_seen()
        .iter()
        .map(|seen| {
            let opacity = |o: Option<u8>| o.map_or("null".to_string(), |o| o.to_string());
            let change = match seen.change {
                AppearanceChange::Arrived => "arrived",
                AppearanceChange::Gone => "gone",
                AppearanceChange::Minimized => "minimized",
                AppearanceChange::Restored => "restored",
                AppearanceChange::WorkspaceIn { forward: true } => "workspace_in_forward",
                AppearanceChange::WorkspaceIn { forward: false } => "workspace_in_back",
                AppearanceChange::WorkspaceOut { forward: true } => "workspace_out_forward",
                AppearanceChange::WorkspaceOut { forward: false } => "workspace_out_back",
                AppearanceChange::Zoomed => "zoomed",
            };
            let path: Vec<String> = seen
                .path
                .iter()
                .map(|step| format!("[{},{},{}]", step.ms, step.opacity, step.moved_permille))
                .collect();
            format!(
                "{{\"kind\":\"{}\",\"change\":\"{change}\",\"arriving\":{},\"first\":{},\"last\":{},\"frames\":{},\"ended\":{},\"path\":[{}]}}",
                format!("{:?}", seen.kind).to_lowercase(),
                seen.arriving(),
                opacity(seen.first),
                opacity(seen.last),
                seen.frames,
                seen.ended,
                path.join(",")
            )
        })
        .collect();
    let frames = std::mem::take(&mut compositor.transition_frames);
    format!("{{\"frames\":{frames},\"seen\":[{}]}}", seen.join(","))
}

fn popups_json(compositor: &Compositor) -> String {
    let popups: Vec<String> = shown(compositor)
        .into_iter()
        .filter(|(id, _, _)| *id != WINDOW)
        .map(|(_, _, r)| format!("[{},{},{},{}]", r.x, r.y, r.width, r.height))
        .collect();
    format!("[{}]", popups.join(","))
}

/// Injects a step through the virtual devices (`--uinput`): pointer steps as an absolute
/// pointer and wheel, keys and text as a keyboard. Returns how many characters no key types
/// and were sent as committed text instead, or `None` for a step the devices do not make.
///
/// The compositor reads each pointer event, each character's keys and each wheel notch before
/// the next is sent, as its event loop would from a person. Sent at once, a step could overflow
/// what the kernel holds for each reader of a device (64 events): past that it drops them all,
/// and libinput resynchronises to keys that are already up. The devices deliver synchronously,
/// so what was sent is there to read.
fn inject_through(
    devices: &mut crate::uinput::Devices,
    compositor: &mut Compositor,
    window: Rect,
    step: &Step,
) -> Result<Option<usize>> {
    let (ox, oy) = compositor.display_offset;
    let outputs: Vec<Rect> = compositor
        .screens
        .iter()
        .map(|screen| screen.rect)
        .collect();
    let on_outputs = |x: i32, y: i32| on_outputs(&outputs, x, y);
    let inside = |rect: Rect, x: i32, y: i32| -> Result<()> {
        if rect.contains_point(rect.x + x, rect.y + y) {
            Ok(())
        } else {
            Err(format!("({x}, {y}) is outside {rect:?}").into())
        }
    };
    // Output coordinates to the display's pixels.
    let at = |x: i32, y: i32| (x + ox, y + oy);
    let press = |devices: &mut crate::uinput::Devices,
                 compositor: &mut Compositor,
                 (x, y): (i32, i32)|
     -> Result<()> {
        devices.move_to(x, y)?;
        compositor.read_input();
        devices.button(true)?;
        compositor.read_input();
        devices.button(false)?;
        Ok(())
    };
    match step {
        Step::Press { x, y } => {
            inside(window, *x, *y)?;
            press(devices, compositor, at(window.x + x, window.y + y))?;
        }
        Step::PressPopup { x, y } => {
            let popup = popup(compositor).ok_or("no popup is shown")?;
            inside(popup, *x, *y)?;
            press(devices, compositor, at(popup.x + x, popup.y + y))?;
        }
        Step::PressOutput { x, y } => {
            on_outputs(*x, *y)?;
            press(devices, compositor, at(*x, *y))?;
        }
        Step::PressSecondary { x, y } => {
            inside(window, *x, *y)?;
            let (px, py) = at(window.x + x, window.y + y);
            devices.move_to(px, py)?;
            compositor.read_input();
            devices.button_code(SECONDARY_BUTTON, true)?;
            compositor.read_input();
            devices.button_code(SECONDARY_BUTTON, false)?;
        }
        Step::DoublePress { x, y } => {
            inside(window, *x, *y)?;
            let point = at(window.x + x, window.y + y);
            press(devices, compositor, point)?;
            compositor.read_input();
            press(devices, compositor, point)?;
        }
        Step::Scroll { x, y, dy } => {
            inside(window, *x, *y)?;
            let (px, py) = at(window.x + x, window.y + y);
            devices.move_to(px, py)?;
            compositor.read_input();
            devices.wheel(*dy, || compositor.read_input())?;
        }
        Step::Pinch {
            x,
            y,
            percent,
            degrees,
        } => {
            inside(window, *x, *y)?;
            let (px, py) = at(window.x + x, window.y + y);
            devices.move_to(px, py)?;
            compositor.read_input();
            devices.pinch(*percent, *degrees, || compositor.read_input())?;
        }
        Step::Drag { from, to } => {
            inside(window, from.0, from.1)?;
            inside(window, to.0, to.1)?;
            let (fx, fy) = at(window.x + from.0, window.y + from.1);
            let (tx, ty) = at(window.x + to.0, window.y + to.1);
            devices.move_to(fx, fy)?;
            compositor.read_input();
            devices.button(true)?;
            compositor.read_input();
            devices.move_to((fx + tx) / 2, (fy + ty) / 2)?;
            compositor.read_input();
            devices.move_to(tx, ty)?;
            compositor.read_input();
            devices.button(false)?;
        }
        Step::DragOutput { from, to } => {
            on_outputs(from.0, from.1)?;
            on_outputs(to.0, to.1)?;
            let (fx, fy) = at(from.0, from.1);
            let (tx, ty) = at(to.0, to.1);
            devices.move_to(fx, fy)?;
            compositor.read_input();
            devices.button(true)?;
            compositor.read_input();
            for (x, y) in drag_points((fx, fy), (tx, ty)) {
                devices.move_to(x, y)?;
                compositor.read_input();
            }
            devices.button(false)?;
        }
        Step::CapsLock => {
            devices.key(CAPS_LOCK as u16, true)?;
            compositor.read_input();
            devices.key(CAPS_LOCK as u16, false)?;
            compositor.read_input();
        }
        Step::Key(codes) => {
            for code in codes {
                devices.key(*code as u16, true)?;
                compositor.read_input();
            }
            for code in codes.iter().rev() {
                devices.key(*code as u16, false)?;
                compositor.read_input();
            }
        }
        Step::KeyDown(codes) | Step::KeyUp(codes) => {
            let pressed = matches!(step, Step::KeyDown(_));
            for code in codes {
                devices.key(*code as u16, pressed)?;
                compositor.read_input();
            }
        }
        Step::Type(text) => {
            let mut as_text = 0;
            for character in text.chars() {
                if !devices.type_char(character)? {
                    compositor.scene.text(&character.to_string());
                    as_text += 1;
                }
                compositor.read_input();
            }
            return Ok(Some(as_text));
        }
        _ => return Ok(None),
    }
    Ok(Some(0))
}

/// The Linux key code typing `c` on a US layout, and the text it types (none for Enter).
fn key_for(c: char) -> Result<(u32, Option<String>)> {
    const LETTERS: [u32; 26] = [
        30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17,
        45, 21, 44,
    ];
    let code = match c {
        'a'..='z' => LETTERS[c as usize - 'a' as usize],
        '1'..='9' => c as u32 - '1' as u32 + 2,
        '0' => 11,
        ' ' => 57,
        '.' => 52,
        '\n' => return Ok((28, None)),
        _ => return Err(format!("{c:?} has no key in --keyboard").into()),
    };
    Ok((code, Some(c.to_string())))
}

/// The steps the session's input method takes part in.
fn inject_input_method(session: &mut Session, step: &Step) -> Result<()> {
    match step {
        Step::Keyboard { text, compose } => {
            match session.ime.as_mut() {
                Some(ime) => ime.takes_keys = *compose,
                None if *compose => return Err("--compose-keys needs --input-method".into()),
                None => {}
            }
            for c in text.chars() {
                let (code, typed) = key_for(c)?;
                let scene = &mut session.compositor.scene;
                scene.keyboard(code, true, typed.as_deref());
                scene.keyboard(code, false, None);
            }
        }
        Step::ImChange {
            delete,
            commit,
            preedit,
            stale,
        } => session
            .ime
            .as_mut()
            .ok_or("--im-change needs --input-method")?
            .change(&mut session.compositor, *delete, commit, preedit, *stale)?,
        _ => unreachable!("not a step of the input method"),
    }
    Ok(())
}

/// Goes to terminal `number` and back through the devices (`Step::SwitchTerminal`); returns
/// how long the compositor was away, in milliseconds.
fn switch_terminal(
    session: &mut Session,
    children: &mut Children,
    devices: &mut crate::uinput::Devices,
    number: u16,
) -> Result<f64> {
    let own = match (&session.compositor.terminal, &session.compositor.seat) {
        (Some(terminal), _) => terminal.number(),
        (None, Some(seat)) => seat
            .vt()
            .and_then(|vt| u16::try_from(vt).ok())
            .ok_or("the session has no terminal")?,
        (None, None) => return Err("--switch-terminal needs --kms".into()),
    };
    if number == own {
        return Err(format!("terminal {number} is the compositor's own").into());
    }
    const CTRL: u16 = 29;
    const ALT: u16 = 56;
    let function_key = |n: u16| match n {
        1..=10 => Ok(58 + n),
        11 => Ok(87),
        12 => Ok(88),
        _ => Err(format!("no function key for terminal {n}")),
    };
    let chord = |devices: &mut crate::uinput::Devices,
                 compositor: &mut Compositor,
                 key: u16|
     -> Result<()> {
        for code in [CTRL, ALT, key] {
            devices.key(code, true)?;
            compositor.read_input();
        }
        for code in [key, ALT, CTRL] {
            devices.key(code, false)?;
            compositor.read_input();
        }
        Ok(())
    };
    let wait = |session: &mut Session,
                children: &mut Children,
                done: &dyn Fn(&Compositor) -> bool,
                what: &str|
     -> Result<()> {
        let deadline = Instant::now() + STEP_DEADLINE;
        while !done(&session.compositor) {
            if Instant::now() > deadline {
                return Err(format!("deadline exceeded waiting for {what}").into());
            }
            if let Some(status) = children.0[0].try_wait()? {
                return Err(format!("the application exited ({status}) waiting for {what}").into());
            }
            session.compositor.serve(&session.listener)?;
        }
        Ok(())
    };
    chord(devices, &mut session.compositor, function_key(number)?)?;
    wait(session, children, &|c| c.away, "the switch away")?;
    let left = Instant::now();
    let shown = session.compositor.screens[0].flips.len();
    // The compositor has let go of the devices: this chord is the other terminal's.
    chord(devices, &mut session.compositor, function_key(own)?)?;
    wait(
        session,
        children,
        &|c| !c.away && c.screens[0].flips.len() > shown,
        "the switch back and the frame shown again",
    )?;
    Ok(left.elapsed().as_secs_f64() * 1e3)
}

/// A point of the global space on one of the outputs, or an error naming them.
fn on_outputs(outputs: &[Rect], x: i32, y: i32) -> Result<()> {
    if outputs.iter().any(|output| output.contains_point(x, y)) {
        Ok(())
    } else {
        Err(format!("({x}, {y}) is on none of the outputs {outputs:?}").into())
    }
}

/// What every output composed last, the primary's first: a step changed it if it changed any.
fn composed(compositor: &Compositor) -> Vec<u32> {
    compositor
        .screens
        .iter()
        .flat_map(|screen| screen.framebuffer.iter().copied())
        .collect()
}

/// Injects one step's input into the scene.
/// A grab (`Step::Grab`): the press, then the compositor serving the application until it is
/// moving the window or two seconds have passed (it never asked: the drag then moves nothing),
/// then the drag and the release. On the scene, with or without the virtual devices.
fn grab(session: &mut Session, window: Rect, from: (i32, i32), by: (i32, i32)) -> Result<()> {
    let start = (window.x + from.0, window.y + from.1);
    if !window.contains_point(start.0, start.1) {
        return Err(format!("({}, {}) is outside {window:?}", from.0, from.1).into());
    }
    session.compositor.scene.pointer_motion(start.0, start.1);
    session.compositor.scene.pointer_button(BUTTON, true);
    let deadline = Instant::now() + Duration::from_secs(2);
    while session.compositor.scene.moving_window().is_none() && Instant::now() < deadline {
        session.compositor.serve(&session.listener)?;
        std::thread::sleep(Duration::from_millis(2));
    }
    for (x, y) in drag_points(start, (start.0 + by.0, start.1 + by.1)) {
        session.compositor.scene.pointer_motion(x, y);
    }
    session.compositor.scene.pointer_button(BUTTON, false);
    Ok(())
}

/// A paced drag (`Step::DragPaced`) on the scene: the press, each motion when its time comes with
/// the compositor serving the application until then, the release, and the compositor serving it
/// until its window shows a frame for the latest configure (or five seconds pass). Returns what
/// the window did, as JSON:
/// - `motions`; `configures`, how many the window was sent; `answered`, how many of those a frame
///   it showed answers; `presented`, how many frames it showed; `sizes`, how many sizes those had;
/// - `latency_ms` (median and max), from each motion to the first frame shown answering a
///   configure sent at or after it;
/// - `edges`, the least and most each edge of the window was at across the frames shown (an edge
///   not dragged stays put), and `final`, the window's rectangle at the end;
/// - `composed`, the frames composed meanwhile, `composed_gap_ms` (median and max) the time between
///   them while the motions lasted (a paced output's refresh, if the hand is faster), and `scaled`,
///   how many drew the window at another size than its buffer's (none should: a window resizing
///   shows its last frame at its size);
/// - `bytes_read`, the application's pixels the compositor read meanwhile (none for a window
///   moved: nothing it shows changes);
/// - `shown`, each frame shown: [ms since the press, configure serial, x, y, width, height].
fn drag_paced(
    session: &mut Session,
    from: (i32, i32),
    to: (i32, i32),
    motions: u32,
    hz: u32,
) -> Result<String> {
    inside_output(from)?;
    inside_output(to)?;
    let key = SurfaceKey {
        client: CLIENT,
        id: WINDOW,
    };
    let displayed = |compositor: &Compositor| {
        compositor
            .scene
            .render_list()
            .into_iter()
            .find_map(|item| match item {
                RenderItem::Surface {
                    key: k,
                    rect,
                    frame,
                } if k == key => Some((frame.sequence, frame.configure, rect)),
                _ => None,
            })
    };
    let serial = |compositor: &Compositor| compositor.scene.configure_serial(key).unwrap_or(0);
    let first_serial = serial(&session.compositor);
    let read = |compositor: &Compositor| compositor.bytes_read.get(&CLIENT).copied().unwrap_or(0);
    let bytes_before = read(&session.compositor);
    let start = Instant::now();
    // Configures first seen sent (serial, when) and frames shown (when, sequence, serial, rect).
    let mut sent: Vec<(u64, Duration)> = Vec::new();
    let mut frames: Vec<(Duration, u64, u64, Rect)> = Vec::new();
    let mut last_sequence = displayed(&session.compositor).map(|(sequence, ..)| sequence);
    // Frames composed meanwhile, and those drawing the window at another size than its buffer's.
    let (mut composed, mut scaled) = (0usize, 0usize);
    let mut composed_at: Vec<Duration> = Vec::new();
    let mut last_frame = session.compositor.frames;
    let mut observe = |compositor: &Compositor,
                       sent: &mut Vec<(u64, Duration)>,
                       frames: &mut Vec<(Duration, u64, u64, Rect)>| {
        let now = start.elapsed();
        if compositor.frames != last_frame {
            last_frame = compositor.frames;
            composed += 1;
            composed_at.push(now);
            // Each screen's frame draws the window from its buffer, made at the scale of the
            // output its centre is on: the protocol's size for its rectangle there (ADR 0001).
            let at_its_size = |rect: Rect, extent: oxyos_protocol::Size| {
                let scale = compositor
                    .scene
                    .output_at_point(
                        rect.x + rect.width as i32 / 2,
                        rect.y + rect.height as i32 / 2,
                    )
                    .map_or(120, |output| output.scale_120);
                physical(oxyos_protocol::Size::new(rect.width, rect.height), scale) == extent
            };
            scaled += compositor
                .screens
                .iter()
                .flat_map(|screen| &screen.composed)
                .filter(|item| {
                    matches!(item, RenderItem::Surface { key: k, rect, frame }
                        if *k == key && !at_its_size(*rect, frame.extent))
                })
                .count();
        }
        let latest = serial(compositor);
        if latest > sent.last().map_or(first_serial, |(serial, _)| *serial) {
            sent.push((latest, now));
        }
        if let Some((sequence, configure, rect)) = displayed(compositor) {
            if Some(sequence) != last_sequence {
                last_sequence = Some(sequence);
                frames.push((now, sequence, configure, rect));
            }
        }
    };
    let origin = monotonic_ns() - i64::try_from(start.elapsed().as_nanos()).unwrap_or(0);
    session
        .compositor
        .scene
        .set_input_time(paced_time(origin, start.elapsed()));
    session.compositor.scene.pointer_motion(from.0, from.1);
    session.compositor.scene.pointer_button(BUTTON, true);
    let interval = Duration::from_secs(1) / hz;
    let mut moved: Vec<Duration> = Vec::new();
    let steps = i64::from(motions);
    // Each motion when its time comes: the turns before it wait at most until then, and the turn
    // the motion wakes composes its frame at once, as a device's event wakes a session's turn.
    let idle_wait = session.compositor.idle_wait;
    for i in 1..=steps {
        let due = interval * i as u32;
        while start.elapsed() < due {
            session.compositor.idle_wait = idle_wait.min(due.saturating_sub(start.elapsed()));
            session.compositor.serve(&session.listener)?;
            observe(&session.compositor, &mut sent, &mut frames);
        }
        let at = |a: i32, b: i32| (i64::from(a) + (i64::from(b) - i64::from(a)) * i / steps) as i32;
        moved.push(start.elapsed());
        session
            .compositor
            .scene
            .set_input_time(paced_time(origin, due));
        session
            .compositor
            .scene
            .pointer_motion(at(from.0, to.0), at(from.1, to.1));
        session.compositor.idle_wait = Duration::ZERO;
        session.compositor.serve(&session.listener)?;
        observe(&session.compositor, &mut sent, &mut frames);
    }
    session.compositor.idle_wait = idle_wait;
    let due = interval * (motions + 1);
    while start.elapsed() < due {
        session.compositor.serve(&session.listener)?;
        observe(&session.compositor, &mut sent, &mut frames);
    }
    session
        .compositor
        .scene
        .set_input_time(paced_time(origin, due));
    session.compositor.scene.pointer_button(BUTTON, false);
    let deadline = start.elapsed() + Duration::from_secs(5);
    loop {
        session.compositor.serve(&session.listener)?;
        observe(&session.compositor, &mut sent, &mut frames);
        let answered = displayed(&session.compositor)
            .is_some_and(|(_, configure, _)| configure == serial(&session.compositor));
        if answered || start.elapsed() > deadline {
            break;
        }
    }
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    // While the hand moved: from its first motion to its last.
    let mut composed_gaps: Vec<f64> = match (moved.first(), moved.last()) {
        (Some(&first), Some(&last)) => {
            let during: Vec<Duration> = composed_at
                .iter()
                .copied()
                .filter(|&at| at >= first && at <= last)
                .collect();
            during
                .windows(2)
                .map(|pair| ms(pair[1] - pair[0]))
                .collect()
        }
        _ => Vec::new(),
    };
    composed_gaps.sort_by(f64::total_cmp);
    let mut latencies: Vec<f64> = moved
        .iter()
        .filter_map(|&t| {
            let (serial, _) = sent.iter().find(|(_, when)| *when >= t)?;
            let (shown, ..) = frames
                .iter()
                .find(|(_, _, configure, _)| configure >= serial)?;
            Some(ms(*shown - t))
        })
        .collect();
    latencies.sort_by(f64::total_cmp);
    let answered = sent
        .iter()
        .filter(|(serial, _)| {
            frames
                .iter()
                .any(|(_, _, configure, _)| configure == serial)
        })
        .count();
    let mut sizes: Vec<(u32, u32)> = frames
        .iter()
        .map(|(_, _, _, rect)| (rect.width, rect.height))
        .collect();
    sizes.sort_unstable();
    sizes.dedup();
    let span = |edge: fn(&Rect) -> i64| {
        let values = frames.iter().map(|(_, _, _, rect)| edge(rect));
        match (values.clone().min(), values.max()) {
            (Some(lo), Some(hi)) => format!("[{lo},{hi}]"),
            _ => "null".to_string(),
        }
    };
    let last = displayed(&session.compositor).map(|(_, _, rect)| rect);
    Ok(format!(
        "{{\"motions\":{motions},\"bytes_read\":{},\"configures\":{},\"answered\":{answered},\"presented\":{},\"sizes\":{},\"latency_ms\":{{\"median\":{},\"max\":{}}},\"edges\":{{\"left\":{},\"top\":{},\"right\":{},\"bottom\":{}}},\"final\":{},\"composed\":{composed},\"composed_gap_ms\":{{\"median\":{},\"max\":{}}},\"scaled\":{scaled},\"shown\":[{}]}}",
        read(&session.compositor) - bytes_before,
        sent.len(),
        frames.len(),
        sizes.len(),
        median(&latencies).map_or("null".to_string(), |ms| format!("{ms:.1}")),
        latencies.last().map_or("null".to_string(), |ms| format!("{ms:.1}")),
        span(|r| i64::from(r.x)),
        span(|r| i64::from(r.y)),
        span(|r| r.right()),
        span(|r| r.bottom()),
        last.map_or("null".to_string(), |r| format!("[{},{},{},{}]", r.x, r.y, r.width, r.height)),
        median(&composed_gaps).map_or("null".to_string(), |ms| format!("{ms:.2}")),
        composed_gaps
            .last()
            .map_or("null".to_string(), |ms| format!("{ms:.2}")),
        frames
            .iter()
            .map(|(t, _, configure, r)| format!(
                "[{:.1},{configure},{},{},{},{}]",
                ms(*t),
                r.x,
                r.y,
                r.width,
                r.height
            ))
            .collect::<Vec<_>>()
            .join(","),
    ))
}

/// A paced scroll (`Step::ScrollPaced`) on the scene: the pointer over the window at `at`, each
/// event when its time comes with the compositor serving the application until then and the turn
/// the event wakes composing at once, as a touchpad's wakes a session's; then the compositor
/// serving it until its window shows no new frame for SETTLE (or five seconds pass). Returns what
/// the window did, as JSON: `events`; `presented`, the frames it showed meanwhile; and
/// `latency_ms` (median and max), from each event to the first frame shown after it.
fn scroll_paced(
    session: &mut Session,
    window: Rect,
    at: (i32, i32),
    dy: i32,
    events: u32,
    hz: u32,
) -> Result<String> {
    if !Rect::new(0, 0, window.width, window.height).contains_point(at.0, at.1) {
        return Err(format!("({}, {}) is outside the window {window:?}", at.0, at.1).into());
    }
    let sequence = |compositor: &Compositor| compositor.surface_sequence(CLIENT, WINDOW);
    session
        .compositor
        .scene
        .pointer_motion(window.x + at.0, window.y + at.1);
    let start = Instant::now();
    let origin = monotonic_ns();
    let interval = Duration::from_secs(1) / hz;
    let mut last = sequence(&session.compositor);
    let mut shown: Vec<Duration> = Vec::new();
    let mut observe = |compositor: &Compositor, shown: &mut Vec<Duration>| {
        let now = sequence(compositor);
        if now != last {
            last = now;
            shown.push(start.elapsed());
        }
    };
    let idle_wait = session.compositor.idle_wait;
    let mut scrolled: Vec<Duration> = Vec::new();
    // In 256ths of a pixel (`Scene::pointer_scroll`), spread evenly over the events.
    let total = i64::from(dy) * 256;
    for i in 1..=i64::from(events) {
        let due = interval * i as u32;
        while start.elapsed() < due {
            session.compositor.idle_wait = idle_wait.min(due.saturating_sub(start.elapsed()));
            session.compositor.serve(&session.listener)?;
            observe(&session.compositor, &mut shown);
        }
        let step = total * i / i64::from(events) - total * (i - 1) / i64::from(events);
        scrolled.push(start.elapsed());
        session
            .compositor
            .scene
            .set_input_time(paced_time(origin, due));
        session.compositor.scene.pointer_scroll(0, step as i32);
        session.compositor.idle_wait = Duration::ZERO;
        session.compositor.serve(&session.listener)?;
        observe(&session.compositor, &mut shown);
    }
    session.compositor.idle_wait = idle_wait;
    let deadline = start.elapsed() + Duration::from_secs(5);
    let mut quiet_since = start.elapsed();
    while start.elapsed() < deadline && start.elapsed() - quiet_since < SETTLE {
        let before = shown.len();
        session.compositor.serve(&session.listener)?;
        observe(&session.compositor, &mut shown);
        if shown.len() != before {
            quiet_since = start.elapsed();
        }
    }
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let mut latencies: Vec<f64> = scrolled
        .iter()
        .filter_map(|&t| shown.iter().find(|&&s| s >= t).map(|&s| ms(s - t)))
        .collect();
    latencies.sort_by(f64::total_cmp);
    Ok(format!(
        "{{\"events\":{events},\"presented\":{},\"latency_ms\":{{\"median\":{},\"max\":{}}}}}",
        shown.len(),
        median(&latencies).map_or("null".to_string(), |ms| format!("{ms:.1}")),
        latencies
            .last()
            .map_or("null".to_string(), |ms| format!("{ms:.1}")),
    ))
}

/// A paced pointer motion (`Step::MovePaced`) over the window, no button held: each motion when
/// its time comes with the compositor serving the application until then and the turn the motion
/// wakes composing at once, as a mouse's wakes a session's; then the compositor serving it until
/// its window shows no new frame for SETTLE (or five seconds pass). Returns what the window did,
/// as JSON: `motions`; `presented`, the frames it showed meanwhile; and `latency_ms` (median and
/// max), from each motion to the first frame shown after it, for the motions a frame followed
/// before the next motion (a hover that changes nothing shown is followed by none).
fn move_paced(
    session: &mut Session,
    window: Rect,
    from: (i32, i32),
    to: (i32, i32),
    motions: u32,
    hz: u32,
) -> Result<String> {
    let inside = Rect::new(0, 0, window.width, window.height);
    for (x, y) in [from, to] {
        if !inside.contains_point(x, y) {
            return Err(format!("({x}, {y}) is outside the window {window:?}").into());
        }
    }
    let sequence = |compositor: &Compositor| compositor.surface_sequence(CLIENT, WINDOW);
    session
        .compositor
        .scene
        .pointer_motion(window.x + from.0, window.y + from.1);
    let start = Instant::now();
    let origin = monotonic_ns();
    let interval = Duration::from_secs(1) / hz;
    let mut last = sequence(&session.compositor);
    let mut shown: Vec<Duration> = Vec::new();
    let mut observe = |compositor: &Compositor, shown: &mut Vec<Duration>| {
        let now = sequence(compositor);
        if now != last {
            last = now;
            shown.push(start.elapsed());
        }
    };
    let idle_wait = session.compositor.idle_wait;
    let mut moved: Vec<Duration> = Vec::new();
    let (dx, dy) = (i64::from(to.0 - from.0), i64::from(to.1 - from.1));
    for i in 1..=i64::from(motions) {
        let due = interval * i as u32;
        while start.elapsed() < due {
            session.compositor.idle_wait = idle_wait.min(due.saturating_sub(start.elapsed()));
            session.compositor.serve(&session.listener)?;
            observe(&session.compositor, &mut shown);
        }
        let x = i64::from(from.0) + dx * i / i64::from(motions);
        let y = i64::from(from.1) + dy * i / i64::from(motions);
        moved.push(start.elapsed());
        session
            .compositor
            .scene
            .set_input_time(paced_time(origin, due));
        session
            .compositor
            .scene
            .pointer_motion(window.x + x as i32, window.y + y as i32);
        session.compositor.idle_wait = Duration::ZERO;
        session.compositor.serve(&session.listener)?;
        observe(&session.compositor, &mut shown);
    }
    session.compositor.idle_wait = idle_wait;
    let deadline = start.elapsed() + Duration::from_secs(5);
    let mut quiet_since = start.elapsed();
    while start.elapsed() < deadline && start.elapsed() - quiet_since < SETTLE {
        let before = shown.len();
        session.compositor.serve(&session.listener)?;
        observe(&session.compositor, &mut shown);
        if shown.len() != before {
            quiet_since = start.elapsed();
        }
    }
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let mut latencies: Vec<f64> = moved
        .iter()
        .enumerate()
        .filter_map(|(k, &t)| {
            let next = moved.get(k + 1).copied().unwrap_or(Duration::MAX);
            shown
                .iter()
                .find(|&&s| s >= t && s < next)
                .map(|&s| ms(s - t))
        })
        .collect();
    latencies.sort_by(f64::total_cmp);
    Ok(format!(
        "{{\"motions\":{motions},\"presented\":{},\"latency_ms\":{{\"median\":{},\"max\":{}}}}}",
        shown.len(),
        median(&latencies).map_or("null".to_string(), |ms| format!("{ms:.1}")),
        latencies
            .last()
            .map_or("null".to_string(), |ms| format!("{ms:.1}")),
    ))
}

fn inside_output((x, y): (i32, i32)) -> Result<()> {
    if OUTPUT.contains_point(x, y) {
        Ok(())
    } else {
        Err(format!("({x}, {y}) is outside the output {OUTPUT:?}").into())
    }
}

fn inject(compositor: &mut Compositor, window: Rect, step: &Step) -> Result<()> {
    let outputs: Vec<Rect> = compositor
        .screens
        .iter()
        .map(|screen| screen.rect)
        .collect();
    let on_outputs = |x: i32, y: i32| on_outputs(&outputs, x, y);
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
        Step::PressOutput { x, y } => {
            on_outputs(*x, *y)?;
            press(&mut compositor.scene, *x, *y);
        }
        Step::PressSecondary { x, y } => {
            inside(window, *x, *y)?;
            compositor.scene.pointer_motion(window.x + x, window.y + y);
            compositor.scene.pointer_button(SECONDARY_BUTTON, true);
            compositor.scene.pointer_button(SECONDARY_BUTTON, false);
        }
        Step::DoublePress { x, y } => {
            inside(window, *x, *y)?;
            press(&mut compositor.scene, window.x + x, window.y + y);
            press(&mut compositor.scene, window.x + x, window.y + y);
        }
        Step::Scroll { x, y, dy } => {
            inside(window, *x, *y)?;
            compositor.scene.pointer_motion(window.x + x, window.y + y);
            compositor.scene.pointer_scroll(0, dy.saturating_mul(256));
        }
        Step::Pinch {
            x,
            y,
            percent,
            degrees,
        } => {
            inside(window, *x, *y)?;
            if *percent <= 0 {
                return Err("a pinch spreads to more than 0%".into());
            }
            let scene = &mut compositor.scene;
            scene.pointer_motion(window.x + x, window.y + y);
            scene.pointer_pinch(GesturePhase::Begin, 65536, 0);
            // libinput's scale and turn are since the pinch began: the spread changes by an even
            // factor at each step, as fingers moving steadily apart do.
            const STEPS: i32 = 8;
            for i in 1..=STEPS {
                let done = f64::from(i) / f64::from(STEPS);
                let scale = (f64::from(*percent) / 100.0).powf(done);
                scene.pointer_pinch(
                    GesturePhase::Update,
                    (scale * 65536.0).round() as u32,
                    (f64::from(*degrees) * done * 256.0).round() as i32,
                );
            }
            scene.pointer_pinch(
                GesturePhase::End,
                (f64::from(*percent) / 100.0 * 65536.0).round() as u32,
                degrees.saturating_mul(256),
            );
        }
        Step::Type(text) | Step::Commit(text) => compositor.scene.text(text),
        Step::Preedit(text) => {
            if !compositor.scene.preedit(text, text.len()) {
                return Err(format!("preedit {text:?} is not a valid composition").into());
            }
        }
        Step::Transaction {
            delete,
            commit,
            preedit,
            cursor,
            segments,
        } => {
            let change = TextTransaction {
                delete_before: delete.0,
                delete_after: delete.1,
                commit,
                preedit,
                cursor: *cursor,
                segments,
            };
            if !compositor.scene.text_transaction(&change) {
                return Err(format!("{change:?} is not a valid transaction").into());
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
        Step::DragOutput { from, to } => {
            on_outputs(from.0, from.1)?;
            on_outputs(to.0, to.1)?;
            let scene = &mut compositor.scene;
            scene.pointer_motion(from.0, from.1);
            scene.pointer_button(BUTTON, true);
            for (x, y) in drag_points(*from, *to) {
                scene.pointer_motion(x, y);
            }
            scene.pointer_button(BUTTON, false);
        }
        Step::CapsLock => {
            let on = !compositor.scene.caps_lock();
            compositor.scene.key(CAPS_LOCK, true);
            compositor.scene.set_caps_lock(on);
            compositor.scene.key(CAPS_LOCK, false);
        }
        Step::Key(codes) => {
            for code in codes {
                compositor.scene.key(*code, true);
            }
            for code in codes.iter().rev() {
                compositor.scene.key(*code, false);
            }
        }
        Step::KeyDown(codes) | Step::KeyUp(codes) => {
            let pressed = matches!(step, Step::KeyDown(_));
            for code in codes {
                compositor.scene.key(*code, pressed);
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
            oxyos_protocol::Size::new(*width, *height),
        ),
        Step::MinimizeWindow => compositor.scene.minimize(SurfaceKey {
            client: CLIENT,
            id: WINDOW,
        }),
        // No input: the idle lock's time passes, and the next turn follows it.
        Step::Idle(seconds) => compositor.idle.pass(Duration::from_secs(*seconds)),
        Step::Scale(scale) => compositor
            .scene
            .set_output_scale(OUTPUT_ID, *scale)
            .map_err(|code| format!("scale {scale}: {code:?}"))?,
        // Handled by the step loop: no input to inject, the session's input method's, or the
        // assistive technology's.
        Step::ExpectPopup
        | Step::Wait(_)
        | Step::Keyboard { .. }
        | Step::ImChange { .. }
        | Step::Accessibility { .. }
        | Step::Grab { .. }
        | Step::DragPaced { .. }
        | Step::ScrollPaced { .. }
        | Step::MovePaced { .. } => {}
        // The step loop runs it through the devices; there is no scene equivalent of a terminal.
        Step::SwitchTerminal(_) => return Err("--switch-terminal needs --uinput and --kms".into()),
    }
    Ok(())
}

pub fn run(out: &Path, launch: &Launch) -> Result<()> {
    fs::create_dir_all(out)?;
    // Before the session counts its descriptors: logind's connection lives as long as the
    // process. What is opened through it is closed before the count is checked.
    let seat = match (&launch.kms, launch.seat) {
        (Some(_), true) => {
            let seat = std::rc::Rc::new(oxyos_seat::Seat::take()?);
            eprintln!("[seat] logind session on terminal {:?}", seat.vt());
            Some(seat)
        }
        (None, true) => return Err("--seat needs --kms".into()),
        _ => None,
    };
    if launch.displays.is_some() && (!launch.outputs.is_empty() || launch.kms.is_some()) {
        return Err(
            "--displays plays the session's displays: no --outputs or --kms with it".into(),
        );
    }
    let mut session = match launch.outputs.as_slice() {
        // The displays are shown below, as the preferences keep them.
        [] if launch.displays.is_some() => Session::with_outputs(&[])?,
        [] => Session::start()?,
        [_, _, ..] if launch.kms.is_some() => {
            return Err("--kms shows one output: --outputs names one at most".into())
        }
        outputs => Session::with_outputs(outputs)?,
    };
    // The preferences before any client connects, so each is told them with its Welcome.
    session.settings = oxyos_compositor::settings::Settings::from_file(launch.preferences.clone());
    session
        .settings
        .apply(&mut session.compositor, oxyos_compositor::settings::now());
    if let Some(file) = &launch.displays {
        session.follow_displays(file)?;
    }
    session.grant_settings = launch.grant_settings.clone();
    session.grants = launch.grants.clone();
    if let Some(socket) = &launch.installd {
        let installs = oxyos_compositor::installs::Installs::new(
            socket.clone(),
            std::env::var_os("HOME").map(PathBuf::from),
        )?;
        let waker = installs.wake_fd()?;
        session.installs_waker = Some(std::os::fd::AsRawFd::as_raw_fd(&waker));
        session.compositor.wakers.push(waker);
        session.installs = Some(installs);
    }
    session.installed_fixture = launch.installed_fixture.clone();
    if let Some(file) = &launch.controls {
        // A played machine, acted on in the turn itself, so a scenario is the same every run.
        let fake = oxyos_controls::Fake::from_file(file.clone())?;
        session.compositor.controls = Some(oxyos_compositor::controls::Controls::new(
            oxyos_controls::Worker::inline(Box::new(fake.clone())),
            Instant::now(),
        ));
        session.controls = Some(fake);
    }
    if launch.input_method {
        session.ime = Some(crate::ime::Ime::connect(&mut session.compositor)?);
    }
    // The shell's applications, from the registry, run with the shell's own Bloom host and bundle.
    if let Some(shell) = &launch.shell {
        session.launcher = Some(
            oxyos_apps::Launcher::new(
                oxyos_apps::Runtime {
                    socket: session.socket.clone(),
                    bloom_host: shell.first().map(PathBuf::from),
                    bloom_bundle: shell.get(1).map(PathBuf::from),
                    runtimes: launch
                        .runtimes
                        .clone()
                        .unwrap_or_else(|| PathBuf::from(oxyos_apps::RUNTIMES)),
                    window: (600, 360),
                    // A launched Bloom host's report goes beside the application's, not into it.
                    env: std::env::var_os("OXYOS_HOST_REPORT")
                        .map(|mut report| {
                            report.push(".launched");
                            vec![("OXYOS_HOST_REPORT".into(), report)]
                        })
                        .unwrap_or_default(),
                },
                oxyos_apps::directories(),
                oxyos_apps::languages(),
            )
            .with_installed(None),
        );
        if let Some(root) = &launch.installed_fixture {
            let user = oxyos_apps::session_user().unwrap_or_default();
            session.launcher = session.launcher.take().map(|launcher| {
                launcher.with_installed_fixture(oxyos_apps::installed_applications(root, &user))
            });
        }
    }
    if let Some(password) = &launch.lock_password {
        session.compositor.authentication =
            Some(Box::new(crate::lock::Fixture::new(password.clone())?));
    }
    // A login screen: locked before any client connects, its passwords the login program's.
    if let Some(socket) = &launch.login {
        let greeter = oxyos_auth::greeter::Greeter::connect(socket)
            .map_err(|error| format!("the login program at {}: {error}", socket.display()))?;
        session.compositor.authentication =
            Some(Box::new(oxyos_compositor::login::LoginProgram(greeter)));
        session.compositor.login = true;
        session.compositor.scene.lock();
    }
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
    // Played here: what the compositor sends it is kept for it (reader.rs).
    session.compositor.local_clients.insert(READER);
    session.reader = crate::reader::Reader::new(READER);
    // With logind, a probe of the session's terminal shows that typing never reached it.
    let mut console = None;
    if let Some(card) = &launch.kms {
        let card_file = match &seat {
            Some(seat) => {
                let (fd, inactive) = seat.open(card)?;
                if inactive {
                    return Err("the session is not active: its terminal must be shown".into());
                }
                session.compositor.card_device = Some(oxyos_seat::Device::of(card)?);
                session.compositor.seat = Some(seat.clone());
                if let (Some(vt), true) = (seat.vt(), launch.input) {
                    console = match oxyos_vt::Console::open(vt) {
                        Ok(console) => Some(console),
                        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                            eprintln!(
                                "[seat] terminal {vt} is not readable: what reached its console is not checked"
                            );
                            None
                        }
                        Err(error) => return Err(error.into()),
                    };
                }
                (oxyos_kms::Card::from_fd(fd)?, true)
            }
            None => {
                // The display's terminal first, so the console draws nothing once the display
                // is ours.
                let terminal = oxyos_vt::Terminal::take(launch.input)?;
                eprintln!(
                    "[vt] terminal {}: graphics, switches by request{}",
                    terminal.number(),
                    if launch.input { ", keyboard off" } else { "" }
                );
                session.compositor.terminal = Some(terminal);
                (oxyos_kms::Card::open(card)?, false)
            }
        };
        let display = match card_file {
            (card, true) => oxyos_kms::Display::open_from_seat(card)?,
            (card, false) => oxyos_kms::Display::open(card)?,
        };
        eprintln!(
            "[kms] showing on {} ({})",
            card.display(),
            display.mode_name()
        );
        // Its frames are paced by the display's refresh, which its surfaces are told.
        let (id, refresh_mhz) = (session.compositor.screens[0].id, display.mode().refresh_mhz);
        session.compositor.set_output_refresh(id, refresh_mhz)?;
        session.compositor.screens[0].display = Some(display);
    }
    // Real input: the display's pixels (or the outputs', without a display), the output
    // centred.
    let mut devices = None;
    if launch.input {
        let screens = &session.compositor.screens;
        let bounds = match &screens[0].display {
            Some(display) => display.size(),
            None => screens.iter().fold((0, 0), |(width, height), screen| {
                (
                    width.max(screen.rect.right() as u32),
                    height.max(screen.rect.bottom() as u32),
                )
            }),
        };
        let primary = screens[0].rect;
        session.compositor.display_offset = match &screens[0].display {
            Some(_) => (
                (bounds.0.saturating_sub(primary.width) / 2) as i32,
                (bounds.1.saturating_sub(primary.height) / 2) as i32,
            ),
            None => (0, 0),
        };
        if launch.uinput {
            devices = Some(crate::uinput::Devices::new(bounds, &launch.layout)?);
            // udev announces the new devices before libinput can open them.
            std::thread::sleep(Duration::from_millis(500));
        }
        // Through logind with `--seat`: devices are the session's, revoked when it goes away.
        let devices = seat.clone().map(|seat| {
            let closer = seat.clone();
            let open: oxyos_input::Open = Box::new(move |path| seat.open(path).map(|(fd, _)| fd));
            let close: oxyos_input::Close = Box::new(move |fd| {
                use std::os::fd::AsFd;
                if let Ok(device) = oxyos_seat::Device::of_fd(fd.as_fd()) {
                    let _ = closer.close(device);
                }
            });
            (open, close)
        });
        let layout = oxyos_input::Layout::named(&launch.layout);
        let input = oxyos_input::Input::open_with(bounds, &layout, devices)?;
        eprintln!(
            "[input] libinput on seat0, layout {}, {}",
            launch.layout,
            if input.composes() {
                "composing with the locale's table"
            } else {
                "no compose table: dead keys type nothing"
            }
        );
        session.compositor.input = Some(input);
        session.compositor.confine_pointer();
    }
    if let Some(dir) = &launch.dump_frames {
        fs::create_dir_all(dir)?;
        session.compositor.keep_frame = true;
    }
    if launch.gpu {
        let compositor = oxyos_gpu::VulkanCompositor::new()?;
        eprintln!(
            "[gpu] composing on {} ({:?})",
            compositor.device_name(),
            compositor.device_kind()
        );
        let mut compositor = compositor;
        // On a display, compose straight into DMA-BUF scanout buffers it imports.
        let screen = &mut session.compositor.screens[0];
        if let Some(display) = &screen.display {
            let (width, height) = display.size();
            for (index, target) in compositor
                .scanout_targets(2, width, height)?
                .into_iter()
                .enumerate()
            {
                use std::os::fd::AsFd;
                let framebuffer = display.card().import_dmabuf(
                    target.fd.as_fd(),
                    target.width,
                    target.height,
                    target.pitch,
                )?;
                screen.scanout.push((index, framebuffer));
            }
            eprintln!("[kms] zero-copy: composing into the display's DMA-BUF framebuffers");
        }
        session.compositor.gpu_only = launch.gpu_only;
        if launch.gpu_only {
            eprintln!("[gpu] composing on the device alone, with no CPU reference");
        }
        session.compositor.gpu = Some(Gpu {
            compositor,
            frames: 0,
            mismatches: 0,
            device_ms: Vec::new(),
            readback_ms: Vec::new(),
            host_ms: Vec::new(),
            uploaded: Vec::new(),
            phases_ms: Vec::new(),
        });
    }
    // The session's shell first, as a session starts it: the application's window is then placed
    // in the work area its panels leave.
    let shell = match &launch.shell {
        Some(command) => Some(start_shell(&mut session, command)?),
        None => None,
    };
    // The screen reader before the application, as a session that runs one starts it.
    let screen_reader = match &launch.reader {
        Some(program) => Some(start_reader(
            &mut session,
            program,
            launch.reader_locale.as_deref(),
            &out.join("speech.jsonl"),
        )?),
        None => None,
    };
    // A login screen has no application: its shell is the first child, which must not exit.
    let mut children = Children(Vec::new());
    if let Some(program) = launch.command.first() {
        children.0.push(
            Command::new(program)
                .args(&launch.command[1..])
                .env("OXYOS_COMPOSITOR_SOCKET", &session.socket)
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY")
                .spawn()?,
        );
    }
    // Its own launch, as the session records one, when the scenario names the application;
    // unless the scenario has it be a process the session did not start
    // (`OXYOS_APP_UNRECORDED`), whose id is then only its claim.
    if let (Ok(id), None, false) = (
        std::env::var("OXYOS_APP_ID"),
        std::env::var_os("OXYOS_APP_UNRECORDED"),
        launch.command.is_empty(),
    ) {
        session.app = Some((children.0[0].id(), id));
    }
    children.0.extend(shell.into_iter().flatten());
    children.0.extend(screen_reader);
    let deadline = Instant::now() + LAUNCH_DEADLINE;

    let launched_ns = monotonic_ns();
    // A login screen has no window: the output stands for it, and its first frame is the one
    // its shell has already shown.
    let login = launch.command.is_empty();
    let primary = session.compositor.screens[0].rect;
    let first = settle(
        &mut session,
        &mut children,
        &[],
        login,
        deadline,
        "the first frame",
        login.then_some(primary),
    )?;
    let mut window = first.window;
    if !login
        && (window.y..window.bottom() as i32).all(|y| {
            (window.x..window.right() as i32).all(|x| session.compositor.pixel(x, y) == DESKTOP)
        })
    {
        return Err("the window shows nothing".into());
    }
    let mut output = composed(&session.compositor);
    let mut snapshots = vec![snapshot(&session.compositor, out, "launch-1.png")?];
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
        if let Step::SwitchTerminal(number) = step {
            let devices = devices.as_mut().ok_or("--switch-terminal needs --uinput")?;
            let away_ms = switch_terminal(&mut session, &mut children, devices, *number)?;
            answered.push(format!(
                "{{{},\"t_settled\":{},\"away_ms\":{away_ms:.1},\"focus\":{},\"delivered\":{},\"popups\":{}}}",
                step.json(),
                monotonic_ns(),
                focus_json(&session.compositor),
                delivered_json(&session),
                popups_json(&session.compositor),
            ));
            continue;
        }
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
        // What a step's transitions are is what the step started.
        transitions_json(&mut session.compositor);
        // What the reader says in a step is what the step made it say.
        session.reader.absorb(&mut session.compositor);
        session.reader.take_announcements();
        let injected_ns = monotonic_ns();
        // A step's input happened when it is injected; a paced step's each when it is due.
        session
            .compositor
            .scene
            .set_input_time(u64::try_from(injected_ns).unwrap_or(0));
        let mut dragged = None;
        let mut scrolled = None;
        let mut moved = None;
        if let Some(ime) = session.ime.as_mut() {
            (ime.keys, ime.given_back) = (0, 0);
        }
        let typed_as_text = match devices.as_mut() {
            _ if matches!(step, Step::Keyboard { .. } | Step::ImChange { .. }) => {
                inject_input_method(&mut session, step)?;
                0
            }
            _ if matches!(step, Step::Wait(_)) => {
                if let Step::Wait(ms) = step {
                    let until = Instant::now() + Duration::from_millis(*ms);
                    while Instant::now() < until {
                        session.compositor.serve(&session.listener)?;
                        session.serve_launches();
                        session.serve_installs();
                        session.serve_preferences();
                        session.reader.absorb(&mut session.compositor);
                    }
                }
                0
            }
            _ if matches!(step, Step::Grab { .. }) => {
                if let Step::Grab { from, by } = step {
                    grab(&mut session, window, *from, *by)?;
                }
                0
            }
            _ if matches!(step, Step::DragPaced { .. }) => {
                if let Step::DragPaced {
                    from,
                    to,
                    motions,
                    hz,
                } = step
                {
                    dragged = Some(drag_paced(&mut session, *from, *to, *motions, *hz)?);
                }
                0
            }
            _ if matches!(step, Step::ScrollPaced { .. }) => {
                if let Step::ScrollPaced { at, dy, events, hz } = step {
                    scrolled = Some(scroll_paced(&mut session, window, *at, *dy, *events, *hz)?);
                }
                0
            }
            _ if matches!(step, Step::MovePaced { .. }) => {
                if let Step::MovePaced {
                    from,
                    to,
                    motions,
                    hz,
                } = step
                {
                    moved = Some(move_paced(&mut session, window, *from, *to, *motions, *hz)?);
                }
                0
            }
            _ if matches!(step, Step::Accessibility { .. }) => {
                if let Step::Accessibility {
                    action,
                    name,
                    custom,
                    selection,
                } = step
                {
                    act(&mut session, *action, name, custom.as_deref(), *selection)?;
                }
                0
            }
            Some(devices) => {
                match inject_through(devices, &mut session.compositor, window, step)? {
                    Some(count) => count,
                    None => {
                        inject(&mut session.compositor, window, step)?;
                        0
                    }
                }
            }
            None => {
                inject(&mut session.compositor, window, step)?;
                0
            }
        };
        let what = format!("an answer to step {} ({})", index + 1, step.describe());
        let answer = settle(
            &mut session,
            &mut children,
            &before,
            *may_stay,
            Instant::now() + STEP_DEADLINE,
            &what,
            Some(window),
        )?;
        window = answer.window;
        let now = composed(&session.compositor);
        let changed = now.iter().zip(&output).filter(|(a, b)| a != b).count();
        if changed == 0 && !may_stay {
            return Err(
                format!("step {} ({}) changed no pixel", index + 1, step.describe()).into(),
            );
        }
        let since_input = |ns: Option<i64>| match ns {
            Some(ns) => format!("{:.2}", (ns - injected_ns) as f64 / 1e6),
            None => "null".to_string(),
        };
        let latency = since_input(answer.first_frame_ns);
        let flip_latency = since_input(answer.first_flip_ns);
        session.reader.absorb(&mut session.compositor);
        let announcements = announcements_json(&session.reader.take_announcements());
        answered.push(format!(
            "{{{},\"changed_pixels\":{changed},\"t_inject\":{injected_ns},\"t_settled\":{},\"latency_ms\":{latency},\"flip_latency_ms\":{flip_latency},\"focus\":{},\"delivered\":{},\"popups\":{},\"window\":[{},{},{},{}],\"a11y\":{},\"announcements\":{announcements},\"typed_as_text\":{typed_as_text},\"field\":{},\"im\":{},\"shell\":{},\"window_shown\":{},\"pointer_shape\":\"{:?}\",\"transitions\":{},\"controls\":{},\"composed\":{}{}{}{}}}",
            step.json(),
            answer.settled_ns,
            focus_json(&session.compositor),
            delivered_json(&session),
            popups_json(&session.compositor),
            answer.window.x,
            answer.window.y,
            answer.window.width,
            answer.window.height,
            accessibility_json(&session.reader),
            field_json(&session.compositor),
            session.ime.as_ref().map_or("null".to_string(), |ime| ime.json()),
            shell_json(&session.compositor),
            session.compositor.surface_rect(CLIENT, WINDOW).is_some(),
            session.compositor.scene.pointer_shape(),
            transitions_json(&mut session.compositor),
            session.controls.as_ref().map_or("null".to_string(), controls_json),
            composed_since(&session.compositor, injected_ns),
            dragged.map_or(String::new(), |drag| format!(",\"drag\":{drag}")),
            scrolled.map_or(String::new(), |scroll| format!(",\"scrolled\":{scroll}")),
            moved.map_or(String::new(), |motion| format!(",\"moved\":{motion}")),
        ));
        snapshots.push(snapshot(
            &session.compositor,
            out,
            &format!("launch-{}.png", index + 2),
        )?);
        dump(&session.compositor, index + 2)?;
        output = now;
    }

    let frames = session.compositor.frames;
    // How much of the output each frame composed (1 when whole): transitions damage their
    // surfaces alone (ADR 0011).
    let damaged = median(&session.compositor.damaged).unwrap_or(1.0);
    let whole = session
        .compositor
        .damaged
        .iter()
        .filter(|&&share| share >= 1.0)
        .count();
    // Input devices close with the session: their descriptors are not leaks; nor is the lock's
    // fixture's.
    session.compositor.input = None;
    session.compositor.authentication = None;
    drop(devices);
    // The assistive technology leaves with the session, like every other client.
    session.compositor.scene.disconnect(READER);
    if session.ime.take().is_some() {
        session.compositor.scene.disconnect(crate::ime::IME);
    }
    // Its socket closes with the session; the shell, a client like the others, is waited for.
    session.compositor.shell_listener = None;
    session.compositor.reader_listener = None;
    let gpu = match session.compositor.gpu.take() {
        Some(gpu) if gpu.mismatches > 0 => {
            return Err(if launch.gpu_only {
                format!(
                    "Vulkan composition failed in {} of {} frames",
                    gpu.mismatches, gpu.frames
                )
            } else {
                format!(
                    "Vulkan composition differed from the CPU reference in {} of {} frames",
                    gpu.mismatches, gpu.frames
                )
            }
            .into())
        }
        Some(gpu) => format!(
            "{{\"device\":{},\"kind\":\"{:?}\",\"frames\":{},\"device_ms_median\":{},\"readback_ms_median\":{},\"upload_copy_ms_median\":{},\"uploaded_kib_median\":{},\"submit_to_fence_ms_median\":{},\"device_before_ms_median\":{},\"device_after_ms_median\":{},{}}}",
            json_string(gpu.compositor.device_name()),
            gpu.compositor.device_kind(),
            gpu.frames,
            median(&gpu.device_ms).map_or("null".to_string(), |ms| format!("{ms:.2}")),
            median(&gpu.readback_ms).map_or("null".to_string(), |ms| format!("{ms:.2}")),
            median(&gpu.host_ms.iter().map(|t| t.0).collect::<Vec<_>>())
                .map_or("null".to_string(), |ms| format!("{ms:.2}")),
            median(&gpu.uploaded.iter().map(|&bytes| bytes as f64 / 1024.0).collect::<Vec<_>>())
                .map_or("null".to_string(), |kib| format!("{kib:.0}")),
            median(&gpu.host_ms.iter().map(|t| t.1).collect::<Vec<_>>())
                .map_or("null".to_string(), |ms| format!("{ms:.2}")),
            median(&gpu.phases_ms.iter().map(|t| t.0).collect::<Vec<_>>())
                .map_or("null".to_string(), |ms| format!("{ms:.2}")),
            median(&gpu.phases_ms.iter().map(|t| t.1).collect::<Vec<_>>())
                .map_or("null".to_string(), |ms| format!("{ms:.2}")),
            if launch.gpu_only {
                "\"reference\":false"
            } else {
                "\"reference\":true,\"identical\":true"
            }
        ),
        None => "null".to_string(),
    };
    // With its keyboard off, the console under the compositor got none of what was typed.
    // `None`: the console could not be read (a session's terminal its user cannot read).
    let console_input = match (&session.compositor.terminal, &console) {
        (Some(terminal), _) if launch.input => Some(terminal.console_input()?),
        (None, Some(console)) => Some(console.input()?),
        (None, None) if launch.seat && launch.input => None,
        _ => Some(0),
    };
    if let Some(bytes) = console_input.filter(|&bytes| bytes > 0) {
        return Err(format!("{bytes} bytes typed reached the console").into());
    }
    let console_input = console_input.map_or("null".to_string(), |bytes| bytes.to_string());
    // The last frame on its way to the display completes before the display is closed.
    session.compositor.complete_flip(true);
    let display = match session.compositor.screens[0].display.take() {
        Some(display) => {
            // A frame is handed over at any point of a refresh and should flip at the next
            // vblank: later than one refresh is a missed deadline.
            let refresh_ms = 1e3 / f64::from(display.refresh_hz().max(1));
            let mut waits: Vec<f64> = session.compositor.screens[0]
                .flips
                .iter()
                .map(|(submitted, flip)| flip.saturating_sub(*submitted).as_secs_f64() * 1e3)
                .collect();
            let missed = waits.iter().filter(|&&ms| ms > refresh_ms * 1.05).count();
            waits.sort_by(f64::total_cmp);
            let compositions = &session.compositor.compositions;
            let reads = &session.compositor.reads;
            let damaged = &session.compositor.damaged;
            format!(
                "{{\"mode\":{},\"composition\":\"{}\",\"shown\":{},\"compose_ms_median\":{:.2},\"compose_ms_max\":{:.2},\"read_ms_median\":{:.2},\"damaged_fraction_median\":{:.3},\"submit_to_flip_ms_median\":{:.2},\"submit_to_flip_ms_max\":{:.2},\"missed_deadlines\":{missed}}}",
                json_string(&display.mode_name()),
                if session.compositor.screens[0].scanout.is_empty() { "cpu" } else { "vulkan-dma-buf" },
                waits.len(),
                median(compositions).unwrap_or(0.0),
                compositions.iter().copied().fold(0.0, f64::max),
                median(reads).unwrap_or(0.0),
                median(damaged).unwrap_or(1.0),
                waits.get(waits.len() / 2).copied().unwrap_or(0.0),
                waits.last().copied().unwrap_or(0.0)
            )
        }
        None => "null".to_string(),
    };
    // The terminal is given back after the display, and closes before the descriptor check, as
    // the console probe does; the logind session is given back after it.
    session.compositor.terminal = None;
    session.compositor.seat = None;
    drop(console);
    // Each output: its id, its rectangle in the global space and its scale in 120ths.
    let session_outputs = session
        .compositor
        .screens
        .iter()
        .map(|s| {
            format!(
                "[{},{},{},{},{},{}]",
                s.id, s.rect.x, s.rect.y, s.rect.width, s.rect.height, s.scale_120
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    // What the shell had started, as each was launched (ADR 0030, decision 7).
    let applications = session
        .application_launches
        .iter()
        .map(|(id, origin, settings)| {
            format!("{{\"id\":{id:?},\"origin\":{origin:?},\"settings\":{settings}}}")
        })
        .collect::<Vec<_>>()
        .join(",");
    session.finish(&mut children)?;
    drop(seat);
    println!(
        "{{\"mode\":\"offscreen-cpu\",\"outputs\":[{}],\"drm\":{display},\"gpu\":{gpu},\"launched\":{:?},\"t_launch\":{launched_ns},\"first_frame_ms\":{:.1},\"window\":[{},{},{},{}],\"frames\":{frames},\"damaged_fraction_median\":{damaged:.3},\"frames_whole\":{whole},\"steps\":[{}],\"applications\":[{}],\"console_input_bytes\":{console_input},\"descriptors_settled\":true,\"snapshots\":{:?},\"status\":\"passed\"}}",
        session_outputs,
        launch.command.first().cloned().unwrap_or_default(),
        first.first_frame_ns.map_or(0.0, |ns| (ns - launched_ns) as f64 / 1e6),
        window.x,
        window.y,
        window.width,
        window.height,
        answered.join(","),
        applications,
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
            "--gpu-only",
            "--dump-frames",
            "frames",
            "--kms",
            "/dev/dri/card1",
            "--seat",
            "--out",
            "o",
            "--press",
            "1,2",
            "--scroll",
            "3,4,-50",
            "--pinch",
            "5,6,150,-30",
            "--type",
            "hi",
            "--commit",
            "mundo",
            "--transaction",
            "3,0",
            "日本",
            "ごが",
            "3,6",
            "0-3:composing;3-6:converting",
            "--input-method",
            "--keyboard",
            "hola",
            "--compose-keys",
            "nihon\n",
            "--im-change",
            "5,0",
            "x",
            "",
            "stale",
            "--switch-terminal",
            "5",
            "--key",
            "14",
            "--key",
            "42+106",
            "--drag",
            "1,2,3,4",
            "--drag-output",
            "10,20,90,100",
            "--scale",
            "180",
            "--idle",
            "300",
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
        assert!(launch.gpu && launch.gpu_only);
        assert_eq!(launch.dump_frames, Some(PathBuf::from("frames")));
        assert_eq!(launch.kms, Some(PathBuf::from("/dev/dri/card1")));
        assert!(launch.seat && launch.input_method);
        let steps: Vec<Step> = launch.steps.iter().map(|(step, _)| step.clone()).collect();
        let allowed: Vec<bool> = launch.steps.iter().map(|(_, allow)| *allow).collect();
        assert_eq!(
            allowed,
            [
                false, false, false, false, false, false, false, false, false, false, false, false,
                false, false, false, false, false, false, false, true
            ]
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
                Step::Pinch {
                    x: 5,
                    y: 6,
                    percent: 150,
                    degrees: -30
                },
                Step::Type("hi".into()),
                Step::Commit("mundo".into()),
                Step::Transaction {
                    delete: (3, 0),
                    commit: "日本".into(),
                    preedit: "ごが".into(),
                    cursor: (3, 6),
                    segments: vec![
                        (0, 3, PreeditStyle::Composing),
                        (3, 6, PreeditStyle::Converting)
                    ],
                },
                Step::Keyboard {
                    text: "hola".into(),
                    compose: false
                },
                Step::Keyboard {
                    text: "nihon\n".into(),
                    compose: true
                },
                Step::ImChange {
                    delete: (5, 0),
                    commit: "x".into(),
                    preedit: String::new(),
                    stale: true
                },
                Step::SwitchTerminal(5),
                Step::Key(vec![14]),
                Step::Key(vec![42, 106]),
                Step::Drag {
                    from: (1, 2),
                    to: (3, 4)
                },
                Step::DragOutput {
                    from: (10, 20),
                    to: (90, 100)
                },
                Step::Scale(180),
                Step::Idle(300),
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
    fn outputs_are_their_pixels_at_their_scales() {
        let (_, launch) = Launch::parse(&args(&[
            "--outputs",
            "1920x1200,2560x1440@1.5:144,800x600@1.25,1280x720:59.94",
            "--",
            "app",
        ]))
        .unwrap();
        let paced = |spec: OutputSpec, mhz| OutputSpec {
            refresh_mhz: Some(mhz),
            ..spec
        };
        assert_eq!(
            launch.outputs,
            [
                OutputSpec::of(1920, 1200),
                paced(
                    OutputSpec {
                        scale_120: 180,
                        ..OutputSpec::of(2560, 1440)
                    },
                    144_000
                ),
                OutputSpec {
                    scale_120: 150,
                    ..OutputSpec::of(800, 600)
                },
                paced(OutputSpec::of(1280, 720), 59_940),
            ]
        );
        for bad in [
            "1920",
            "1920x",
            "x1080",
            "1920x1080@0.5",
            "1920x1080@5",
            "1920x1080@x",
            "1920x1080:0",
            "1920x1080:fast",
            "1920x1080:120@1.5",
        ] {
            assert!(
                Launch::parse(&args(&["--outputs", bad, "--", "app"])).is_err(),
                "{bad:?}"
            );
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
