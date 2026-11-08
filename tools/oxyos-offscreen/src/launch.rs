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
use oxyos_protocol::{
    AccessibleAction, AccessibleState, GesturePhase, PreeditStyle, ScrollSource, TextChunk,
};

/// Time the application's frames must stay unchanged to count as settled.
const SETTLE: Duration = Duration::from_millis(750);
/// Escape's Linux input code: what cancels a drag (`Step::DragPaced`'s `escape_after`).
const KEY_ESCAPE: u32 = 1;
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
    /// The pointer moved to a point of the output, no button held: a hover over the shell's
    /// panels and their popups (a menu's row). With a button held (a `DragPaced` that `held` it),
    /// the drag goes on there.
    MoveOutput { x: i32, y: i32 },
    /// The pointer moved to a point of the output and the primary button, held since a
    /// `DragPaced` that `held` it, released there: the drag's drop, or its end over nothing.
    ReleaseOutput { x: i32, y: i32 },
    /// A scroll by `dy` at a point of the output (a dock icon's, ADR 0045): a wheel's notch with
    /// `events` 0, else a touchpad's two fingers in `events` even steps, then lifted; through the
    /// virtual devices, the hand's, as `Scroll`'s.
    ScrollOutput {
        x: i32,
        y: i32,
        dy: i32,
        events: u32,
    },
    /// Three or more fingers swiped on the touchpad by (dx, dy) in `steps` even steps, `hz` a
    /// second, through the compositor's input as libinput's swipe gesture is (Mission Control, App
    /// Exposé and the desktops, ADR 0045, ADR 0046): `fingers` put down first (none: they are down
    /// since an earlier step's), and lifted `lift` milliseconds after their last motion (none:
    /// they stay down into the next steps, the swipe partway). Through the virtual devices, a
    /// whole swipe is the fingers on the virtual touchpad, which libinput recognizes and counts.
    Swipe {
        fingers: Option<u8>,
        dx: i32,
        dy: i32,
        steps: u32,
        hz: u32,
        lift: Option<u32>,
    },
    /// A secondary (right) button press and release at a window-local logical point: a context
    /// menu's.
    PressSecondary { x: i32, y: i32 },
    /// The same at a point of the output, wherever it is: a dock icon's menu (ADR 0034).
    PressSecondaryOutput { x: i32, y: i32 },
    /// A left-button press at a point of the output held still for `ms` milliseconds, the
    /// compositor serving the clients meanwhile, then released: a long press (ADR 0034).
    HoldOutput { x: i32, y: i32, ms: u32 },
    /// One of the output's steps (`press_output`, `press_secondary_output`, `move_output`,
    /// `hold_output`) at the middle of the node named `name` the assistive technology holds on the
    /// topmost shown surface that has one, as a person aims at what they see: a dock's icon by its
    /// label, wherever the dock's row places it. Resolved to that step as it is run.
    Named {
        kind: NamedKind,
        name: String,
        ms: u32,
    },
    /// A paced drag (`DragPaced`) from the middle of the node named `from` to the middle of the
    /// node named `to`, as the assistive technology holds them: a file dragged onto the dock's
    /// Trash, wherever the dock's row places it (docs/adr/0035-drag-and-drop.mdx). Resolved to a
    /// `DragPaced` as it is run.
    /// `to` may be a point of the output instead, `@x,y` (an icon dragged off the dock, ADR
    /// 0034's decision 10); with `held`, the button stays held at the end, as `DragPaced`'s.
    DragPacedNamed {
        from: String,
        to: String,
        motions: u32,
        hz: u32,
        escape_after: Option<u32>,
        rest_ms: u32,
        held: bool,
    },
    /// Two left-button presses at a window-local logical point in quick succession, with nothing
    /// between them: a double click.
    DoublePress { x: i32, y: i32 },
    /// The same at a logical point of the output (an icon on the shell's desktop, ADR 0047).
    DoublePressOutput { x: i32, y: i32 },
    /// Three such presses: a triple click.
    TriplePress { x: i32, y: i32 },
    /// A press and release at the first point, then a press there held and moved to the second
    /// point, then released: a double click whose second press drags.
    DoubleDrag { from: (i32, i32), to: (i32, i32) },
    /// Scroll by `dy` logical pixels (positive moves the view down) with the pointer at a
    /// window-local point: a wheel's notches with `events` 0, else a touchpad's two fingers in
    /// `events` even steps, then lifted. Through the virtual devices it is the hand's: the wheel
    /// turned toward the person, or the fingers moved down, for a positive `dy`, which libinput
    /// turns as the device's natural scrolling says (docs/adr/0042-scrolling.mdx, decision 6).
    Scroll {
        x: i32,
        y: i32,
        dy: i32,
        events: u32,
    },
    /// A touchpad pinch of `fingers` with the pointer at a window-local point: the fingers spread
    /// to `percent` of their spread and turn `degrees` clockwise, in eight even steps
    /// (`drag_points`) on the scene, or through the virtual touchpad, where libinput recognizes
    /// it and counts them (the thumb and three fingers: Launchpad's, ADR 0046).
    Pinch {
        x: i32,
        y: i32,
        percent: i32,
        degrees: i32,
        fingers: u8,
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
    /// (`drag_paced`). With `escape_after`, Escape is pressed and released after that motion and
    /// the drag goes on: a drag between applications cancelled (docs/adr/0035-drag-and-drop.mdx).
    /// With `rest_ms`, the hand rests at `to` that long after its last motion, before an Escape
    /// after that motion and the release: a file held over the dock's Trash
    /// (docs/adr/0044-system-sounds.mdx).
    /// With `held`, the button stays held at the end (`--drag-hold`): what the drag shows midway
    /// is the step's frame, and later steps go on with it (`MoveOutput`, the modifiers held,
    /// `ReleaseOutput`).
    DragPaced {
        from: (i32, i32),
        to: (i32, i32),
        motions: u32,
        hz: u32,
        escape_after: Option<u32>,
        rest_ms: u32,
        held: bool,
    },
    /// The pointer at a window-local point scrolling by `dy` in `events` even steps, `hz` of them a
    /// second, the compositor serving the application between them: a touchpad's two fingers
    /// scrolling a list. What the window showed meanwhile is measured (`scroll_paced`). With
    /// `lift`, the fingers then lift (the scroll's stop) and leave it gliding: a flick. With
    /// `hold_after_ms`, two fingers then come to rest on the touchpad that long after the lift
    /// (libinput's hold gesture: its begin, then its end 150 ms later as they lift unmoved), as a
    /// hand stops a list gliding.
    ScrollPaced {
        at: (i32, i32),
        dy: i32,
        events: u32,
        hz: u32,
        lift: bool,
        hold_after_ms: Option<u32>,
    },
    /// `count` presses of the primary button at a window-local point, `interval_ms` apart by
    /// their device's time, each injected when its time comes (the compositor serving the
    /// application meanwhile, however long it takes to answer) and carrying that time: a hand
    /// pressing twice for a double press, whatever the application was doing in between.
    PressPaced {
        at: (i32, i32),
        count: u32,
        interval_ms: u32,
    },
    /// A double click as a hand makes it on a window's title area, at a window-local point: the
    /// primary button held while the compositor serves the application until it begins a move
    /// with the press (as a title bar's drag region does; two seconds at most), released, then
    /// `gap_ms` later pressed again, held `hold_ms` and released.
    DoubleClick {
        at: (i32, i32),
        hold_ms: u32,
        gap_ms: u32,
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
    /// Keys typed as a person types them on the session's keyboard: each chord pressed in order
    /// and released in reverse, one after the other, through the session's keymap (`--layout`) and
    /// compose table, as its devices' keys are (`Keyboard::actions`): a dead key's composition, the
    /// text a key types on that layout, Caps Lock. Through the virtual devices, the same keys.
    Strokes(Vec<Vec<u32>>),
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
    /// The clock the compositor's transitions run on (`Compositor::clock`, ADR 0011), for a
    /// scenario to look at a transition at an exact point of it, whatever the machine's load:
    /// held where it is now, set to so many milliseconds after where it was held, or following
    /// the monotonic clock again (a transition held then ends at once). The session is served
    /// meanwhile; nothing else waits.
    TransitionClock(TransitionClock),
    /// Real time, the session served all along, until the assistive technology holds a node named
    /// `name` on a shown surface, at most `ms` milliseconds: an answer that comes on the
    /// application's own clock (a store that is down asked again until it says it could not be
    /// loaded), never after a set time it might outlast.
    WaitNamed { name: String, ms: u64 },
    /// Another application copies these types with their contents (pasteboard.rs): the
    /// clipboard holds them, for the application to paste (ADR 0007).
    Clipboard(Vec<(TextChunk, Vec<u8>)>),
    /// The played network daemon changes the machine (`--network-bus`, docs/adr/0038-networks.mdx):
    /// its control interface's `Act`, a cable plugged or an access point appearing.
    Network(String),
    /// The played sound changes (`--sound`, docs/adr/0056-sound-devices.mdx): headphones plugged
    /// in, a display unplugged, another program's volume (`oxyos_audio::Played::act`).
    Sound(String),
    /// The played BlueZ changes the machine (`--bluetooth-bus`, docs/adr/0055-bluetooth.mdx): its
    /// control interface's `Act`, a device appearing, asking to pair, a code typed on it.
    Bluetooth(String),
    /// The played power services change the machine (`--power-bus`, docs/adr/0057-battery-and-
    /// energy.mdx): unplugged, a lower charge, UPower's warning.
    Power(String),
    /// The person chooses the command at this path of the window's menus in the session's bar
    /// (`View > Show Full URL`), as the shell delivers its choice (ADR 0021).
    Menu(String),
}

/// What a `Step::Named` does at the node's middle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedKind {
    Press,
    PressSecondary,
    Move,
    Hold,
    /// A double press (`DoublePressOutput`'s).
    DoublePress,
    /// A scroll by `dy` (`ScrollOutput`'s).
    Scroll {
        dy: i32,
        events: u32,
    },
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
            Step::ScrollOutput { x, y, dy, events } => {
                format!("scroll by {dy} at ({x}, {y}) on the output in {events} events")
            }
            Step::Swipe {
                fingers,
                dx,
                dy,
                steps,
                hz,
                lift,
            } => format!(
                "{} swiped by ({dx}, {dy}) in {steps} steps at {hz} Hz{}",
                fingers.map_or("the fingers".to_string(), |n| format!("{n} fingers")),
                lift.map_or(", held".to_string(), |ms| format!(", lifted {ms} ms after"))
            ),
            Step::MoveOutput { x, y } => format!("pointer to ({x}, {y}) on the output"),
            Step::ReleaseOutput { x, y } => format!("release at ({x}, {y}) on the output"),
            Step::PressSecondary { x, y } => format!("secondary press at ({x}, {y})"),
            Step::PressSecondaryOutput { x, y } => {
                format!("secondary press at ({x}, {y}) on the output")
            }
            Step::HoldOutput { x, y, ms } => {
                format!("press at ({x}, {y}) on the output held {ms} ms")
            }
            Step::Named { kind, name, ms } => format!("{kind:?} at {name:?} ({ms} ms)"),
            Step::DragPacedNamed {
                from,
                to,
                motions,
                hz,
                ..
            } => format!("drag from {from:?} to {to:?} in {motions} motions at {hz} Hz"),
            Step::DoublePress { x, y } => format!("double press at ({x}, {y})"),
            Step::DoublePressOutput { x, y } => format!("double press at ({x}, {y}) on the output"),
            Step::TriplePress { x, y } => format!("triple press at ({x}, {y})"),
            Step::DoubleDrag { from, to } => format!("double press at {from:?} dragged to {to:?}"),
            Step::Scroll {
                x,
                y,
                dy,
                events: 0,
            } => format!("scroll by {dy} at ({x}, {y})"),
            Step::Scroll { x, y, dy, events } => {
                format!("scroll by {dy} at ({x}, {y}) in {events} events")
            }
            Step::Pinch {
                x,
                y,
                percent,
                degrees,
                fingers,
            } => format!(
                "{fingers}-finger pinch to {percent}% turning {degrees} degrees at ({x}, {y})"
            ),
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
                escape_after,
                rest_ms,
                held,
            } => format!(
                "drag from {from:?} to {to:?} on the output in {motions} motions at {hz} Hz{}{}{}",
                if *rest_ms > 0 { format!(", resting {rest_ms} ms") } else { String::new() },
                escape_after.map_or(String::new(), |at| format!(", Escape after motion {at}")),
                if *held { ", held" } else { "" }
            ),
            Step::ScrollPaced {
                at,
                dy,
                events,
                hz,
                lift,
                hold_after_ms,
            } => format!(
                "{} by {dy} at {at:?} in {events} events at {hz} Hz{}",
                if *lift { "flick" } else { "scroll" },
                hold_after_ms.map_or(String::new(), |ms| format!(
                    ", fingers resting {ms} ms after the lift"
                ))
            ),
            Step::MovePaced {
                from,
                to,
                motions,
                hz,
            } => format!("pointer moved from {from:?} to {to:?} in {motions} motions at {hz} Hz"),
            Step::PressPaced {
                at,
                count,
                interval_ms,
            } => format!("{count} presses at {at:?}, {interval_ms} ms apart"),
            Step::DoubleClick { at, hold_ms, gap_ms } => {
                format!("double click at {at:?}, each press held {hold_ms} ms, {gap_ms} ms apart")
            }
            Step::Key(codes) => format!("key {}", chord(codes)),
            Step::Strokes(chords) => format!(
                "keys {} on the keymap",
                chords.iter().map(|codes| chord(codes)).collect::<Vec<_>>().join(", ")
            ),
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
            Step::TransitionClock(TransitionClock::Hold) => "the transitions' clock held".to_string(),
            Step::TransitionClock(TransitionClock::At(ms)) => {
                format!("the transitions' clock {ms} ms after it was held")
            }
            Step::TransitionClock(TransitionClock::Run) => {
                "the transitions' clock running".to_string()
            }
            Step::WaitNamed { name, ms } => format!("waiting for {name:?} (at most {ms} ms)"),
            Step::Clipboard(contents) => format!("another application copying {} types", contents.len()),
            Step::Network(what) => format!("the network daemon: {what}"),
            Step::Sound(what) => format!("the sound: {what}"),
            Step::Bluetooth(what) => format!("BlueZ: {what}"),
            Step::Power(what) => format!("the power services: {what}"),
            Step::Menu(path) => format!("choosing {path:?} in the menu bar"),
        }
    }

    fn json(&self) -> String {
        match self {
            Step::Press { x, y } => format!("\"press\":[{x},{y}]"),
            Step::PressPopup { x, y } => format!("\"press_popup\":[{x},{y}]"),
            Step::PressOutput { x, y } => format!("\"press_output\":[{x},{y}]"),
            Step::ScrollOutput { x, y, dy, events } => {
                format!("\"scroll_output\":[{x},{y},{dy},{events}]")
            }
            Step::Swipe {
                fingers,
                dx,
                dy,
                steps,
                hz,
                lift,
            } => match (fingers, lift) {
                (Some(n), Some(0)) => format!("\"swipe\":[{n},{dx},{dy},{steps},{hz}]"),
                (Some(n), None) => format!("\"swipe_begin\":[{n},{dx},{dy},{steps},{hz}]"),
                (None, None) => format!("\"swipe_move\":[{dx},{dy},{steps},{hz}]"),
                (_, Some(ms)) if *steps == 0 => format!("\"swipe_end\":{ms}"),
                (_, lift) => format!(
                    "\"swipe\":[{},{dx},{dy},{steps},{hz},{}]",
                    fingers.map_or(0, u32::from),
                    lift.unwrap_or(0)
                ),
            },
            Step::MoveOutput { x, y } => format!("\"move_output\":[{x},{y}]"),
            Step::ReleaseOutput { x, y } => format!("\"release_output\":[{x},{y}]"),
            Step::PressSecondary { x, y } => format!("\"press_secondary\":[{x},{y}]"),
            Step::PressSecondaryOutput { x, y } => {
                format!("\"press_secondary_output\":[{x},{y}]")
            }
            Step::HoldOutput { x, y, ms } => format!("\"hold_output\":[{x},{y},{ms}]"),
            Step::Named { kind, name, ms } => {
                format!("\"named\":[\"{kind:?}\",{},{ms}]", json_string(name))
            }
            Step::DragPacedNamed {
                from,
                to,
                motions,
                hz,
                escape_after,
                rest_ms,
                held,
            } => format!(
                "\"drag_paced_named\":[{},{},{motions},{hz}{}{}]",
                json_string(from),
                json_string(to),
                rest_suffix(*escape_after, *rest_ms),
                if *held { ",\"held\"" } else { "" }
            ),
            Step::DoublePress { x, y } => format!("\"double_press\":[{x},{y}]"),
            Step::DoublePressOutput { x, y } => format!("\"double_press_output\":[{x},{y}]"),
            Step::TriplePress { x, y } => format!("\"triple_press\":[{x},{y}]"),
            Step::DoubleDrag { from, to } => {
                format!("\"double_drag\":[{},{},{},{}]", from.0, from.1, to.0, to.1)
            }
            Step::Scroll {
                x,
                y,
                dy,
                events: 0,
            } => format!("\"scroll\":[{x},{y},{dy}]"),
            Step::Scroll { x, y, dy, events } => format!("\"scroll\":[{x},{y},{dy},{events}]"),
            Step::Pinch {
                x,
                y,
                percent,
                degrees,
                fingers: 2,
            } => format!("\"pinch\":[{x},{y},{percent},{degrees}]"),
            Step::Pinch {
                x,
                y,
                percent,
                degrees,
                fingers,
            } => format!("\"pinch\":[{x},{y},{percent},{degrees},{fingers}]"),
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
                escape_after,
                rest_ms,
                held,
            } => format!(
                "\"{}\":[{},{},{},{},{motions},{hz}{}]",
                if *held { "drag_hold" } else { "drag_paced" },
                from.0,
                from.1,
                to.0,
                to.1,
                rest_suffix(*escape_after, *rest_ms)
            ),
            Step::ScrollPaced {
                at,
                dy,
                events,
                hz,
                lift,
                hold_after_ms: Some(ms),
            } if *lift => format!(
                "\"flick_hold\":[{},{},{dy},{events},{hz},{ms}]",
                at.0, at.1
            ),
            Step::ScrollPaced {
                at,
                dy,
                events,
                hz,
                lift,
                ..
            } => format!(
                "\"{}\":[{},{},{dy},{events},{hz}]",
                if *lift { "flick" } else { "scroll_paced" },
                at.0,
                at.1
            ),
            Step::MovePaced {
                from,
                to,
                motions,
                hz,
            } => format!(
                "\"move_paced\":[{},{},{},{},{motions},{hz}]",
                from.0, from.1, to.0, to.1
            ),
            Step::PressPaced {
                at,
                count,
                interval_ms,
            } => format!("\"press_paced\":[{},{},{count},{interval_ms}]", at.0, at.1),
            Step::DoubleClick { at, hold_ms, gap_ms } => {
                format!("\"double_click\":[{},{},{hold_ms},{gap_ms}]", at.0, at.1)
            }
            Step::Key(codes) => format!("\"key\":{codes:?}"),
            Step::Strokes(chords) => format!("\"strokes\":{chords:?}"),
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
            Step::Network(what) => format!("\"network\":{}", json_string(what)),
            Step::Sound(what) => format!("\"sound\":{}", json_string(what)),
            Step::Bluetooth(what) => format!("\"bluetooth\":{}", json_string(what)),
            Step::Power(what) => format!("\"power\":{}", json_string(what)),
            Step::Menu(path) => format!("\"menu\":{}", json_string(path)),
            Step::Clipboard(contents) => {
                let types: Vec<String> = contents
                    .iter()
                    .map(|(mime, _)| json_string(mime.as_str()))
                    .collect();
                format!("\"clipboard\":[{}]", types.join(","))
            }
            Step::Wait(ms) => format!("\"wait\":{ms}"),
            Step::TransitionClock(TransitionClock::Hold) => {
                "\"transition_clock\":\"hold\"".to_string()
            }
            Step::TransitionClock(TransitionClock::At(ms)) => format!("\"transition_clock\":{ms}"),
            Step::TransitionClock(TransitionClock::Run) => {
                "\"transition_clock\":\"run\"".to_string()
            }
            Step::WaitNamed { name, ms } => {
                format!("\"wait_named\":[{},{ms}]", json_string(name))
            }
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

/// How a step's answer settles, from the flags before it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settling {
    /// `--allow-unchanged`: it may leave the output unchanged.
    pub may_stay: bool,
    /// `--until-sheet`: its answer is a system sheet the session asks (ADR 0030), which may come
    /// long after the step's first answer (an update staged before its consent is asked): it
    /// settles only once the session has a sheet waiting and the shell shows a surface it did not
    /// show before the step.
    pub sheet: bool,
    /// `--until-named MS NAME`: its answer comes on the application's own clock (an install the
    /// person confirmed, done once the machine has fetched and compiled it): it settles only once
    /// the assistive technology holds a node so named on a shown surface, within MS, as
    /// `--wait-named` waits, and so what the step said is all its own.
    pub until_named: Option<(u64, String)>,
}

pub struct Launch {
    pub command: Vec<std::ffi::OsString>,
    /// Each step, and how its answer settles.
    pub steps: Vec<(Step, Settling)>,
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
    /// `--launch-settle MS`: how long the application's first frames must stay unchanged before
    /// the steps begin (SETTLE by default): a browser shows its window before its interface draws
    /// in it, for longer than SETTLE on a loaded machine.
    pub launch_settle: Duration,
    /// `--input-method`: the session plays an input method (ime.rs).
    pub input_method: bool,
    /// `--im-candidates horizontal|vertical`: it hands the system candidates for what it
    /// composes, which the session's shell draws (ADR 0041).
    pub im_candidates: Option<crate::ime::Layout>,
    /// `--autofill-fixture ID`: AutoFill answered by the fixture keychain, its items associated
    /// with the application `ID` (docs/adr/0050-keychain-autofill-and-passkeys.mdx).
    pub autofill_fixture: Option<String>,
    /// `--passkey-fixture ID`: passkeys' sheets asked by the fixture accounts' service for the
    /// application `ID` (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7).
    pub passkey_fixture: Option<String>,
    /// `--autofill-saves`: with `--autofill-fixture`, applications' requests to save a login are
    /// asked of the person as the session asks them: told by the accounts' service at
    /// `OXYOS_ACCOUNTD_SESSION_SOCKET` (a run's played one), asked in the shell's sheet, answered
    /// back (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 5).
    pub autofill_saves: bool,
    /// `--shell PROGRAM,ARG...`: the session's shell, started before the application on a socket
    /// granting `shell`, run as `PROGRAM ARG... --shell OUTPUT` (docs/adr/0009-session-shell.mdx).
    pub shell: Option<Vec<std::ffi::OsString>>,
    /// `--file-chooser PROGRAM,ARG...`: the file chooser the session starts for an application's
    /// request (docs/adr/0029-chromium-on-oxyos.mdx, decision 8), run as `PROGRAM ARG... --window
    /// W H` and told what to show in `OXYOS_FILE_CHOOSER`, as the file manager is in a session.
    /// Without it, the shell's registry's file manager (with `--shell`), else none.
    pub file_chooser: Option<Vec<std::ffi::OsString>>,
    /// `--companion PROGRAM,ARG...`: a second application, started once the first's window is
    /// shown, on its socket, its report the application's with ".companion": the other side of a
    /// drag between applications (docs/adr/0035-drag-and-drop.mdx). What it is delivered is
    /// counted as the companion's.
    pub companion: Option<Vec<std::ffi::OsString>>,
    /// `--companion-at X,Y`: where the companion's window is moved once shown (output
    /// coordinates), beside the application's rather than centred over it.
    pub companion_at: Option<(i32, i32)>,
    /// `--companion-app ID`: the application the companion is launched for, recorded as the
    /// session records a launch (else it is an unrecorded `so.oxy.companion`): the application's
    /// own makes the two windows one application's (docs/adr/0024-system-shortcuts.mdx, decision 2),
    /// as Super+` goes through them.
    pub companion_app: Option<String>,
    /// `--lock-password PASSWORD`: the lock screen's password unlocks when it is this one, the
    /// development compositor's fixture in place of the session's PAM (lock.rs).
    pub lock_password: Option<String>,
    /// `--lock-pin PIN`: with `--lock-password`, a PIN said as one (`AuthenticatePasscode`)
    /// unlocks when it is this one (lock.rs; ADR 0020 decision 2).
    pub lock_pin: Option<String>,
    /// `--reader PROGRAM`: the session's screen reader (docs/adr/0014-screen-reader.mdx), started
    /// before the application on a socket granting `accessibility` and `screen_reader`, its voice
    /// a record (`speech.jsonl` in the output directory); `--reader-locale LOCALE` is its
    /// environment's locale (`LC_ALL`), the session's otherwise: what it speaks until the
    /// session's `Locale` tells it (ADR 0040), which it is on connecting. With `--reader-off` it is the session's but
    /// off, as `oxyos-session --reader` without `--reader-on` has it: Super+F5 typed on the keymap
    /// (`strokes`) starts it, and again stops it (ADR 0014, decision 6), locked or not.
    pub reader: Option<PathBuf>,
    pub reader_locale: Option<String>,
    pub reader_off: bool,
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
    /// `--trash`: the user's Trashes followed for the dock as the session follows them
    /// (docs/adr/0034-the-dock.mdx); a scenario's own home's, never the developer's.
    pub trash: bool,
    /// `--safe-mode`: the session started in Safe Mode (docs/adr/0054-recovery-and-erasing.mdx,
    /// decision 10), as `oxyos-session` reads it from the kernel's command line: nothing opened at
    /// login, no background activity but the system's, the shell and the login screen told.
    pub safe_mode: bool,
    /// `--storage FILE`: Settings › Storage as the session serves it (docs/adr/0062-storage.mdx),
    /// on the run's own home, the volume's figures and the system's a played disk's (FILE, JSON:
    /// `name`, `capacity`, `available`, `reserved`, `system`), never the developer's disk.
    pub storage: Option<PathBuf>,
    /// `--runtimes DIR`: where the runtimes an application's entry names are
    /// (docs/adr/0030-installing-applications.mdx, decision 2): a scenario's, in place of the
    /// image's (`/usr/lib/oxyos/runtimes`).
    pub runtimes: Option<PathBuf>,
    /// `--installed-fixture ROOT`: the install service's state directory, as a scenario wrote it
    /// (docs/adr/0030-installing-applications.mdx, decision 7): its entries for this user are the
    /// registry's second source, taken as the service's. A fixture: a session's are root's.
    pub installed_fixture: Option<PathBuf>,
    /// `--screenshots DIR`: screenshots are written as the session writes them
    /// (docs/adr/0024-system-shortcuts.mdx, decision 10), in DIR's `Desktop` and `Documents`, named
    /// in the session's language; each step's result says what was saved and opened.
    pub screenshots: Option<PathBuf>,
    /// `--controls FILE`: the machine the hardware keys and the Control Center act on
    /// (docs/adr/0025-hardware-keys-and-control-center.mdx), played as FILE describes it
    /// (`oxyos_controls::Fake`), which it writes back as the session changes it; each step's
    /// result says it (`controls`). Without it the keys act on nothing and the shell is told
    /// nothing of the machine.
    pub controls: Option<PathBuf>,
    /// `--network-bus ADDRESS`: the session's networks (docs/adr/0038-networks.mdx) are the
    /// NetworkManager on this bus, a played one (`oxyos-fake-networkmanager`), never the system's;
    /// the played machine's network (`--controls`) follows it too, and each step's result says
    /// what the daemon holds (`network`).
    pub network_bus: Option<String>,
    /// `--sound FILE`: the session's sound (docs/adr/0056-sound-devices.mdx) is a machine played
    /// as FILE describes it (`oxyos_audio::Played`), acted on in the turn; the played machine's
    /// volumes (`--controls`) are its chosen devices', and each step's result says what it is
    /// then (`sound`).
    pub sound: Option<PathBuf>,
    /// `--bluetooth-bus ADDRESS`: the session's Bluetooth (docs/adr/0055-bluetooth.mdx) is the
    /// BlueZ on this bus, a played one (`oxyos-fake-bluez`), never the system's; the played
    /// machine's Bluetooth (`--controls`) follows it too, and each step's result says what BlueZ
    /// holds (`bluetooth`).
    pub bluetooth_bus: Option<String>,
    /// `--power-bus ADDRESS`: UPower, power-profiles-daemon and logind as played on this bus
    /// (`oxyos-offscreen --play-power`), never the system's; the session's battery and energy
    /// (docs/adr/0057-battery-and-energy.mdx) and the played machine's battery (`--controls`) are
    /// theirs, and each step's result says what they hold (`power`).
    pub power_bus: Option<String>,
    /// `--input-methods PREFIX`: the session's input methods (docs/adr/0041-input-methods.mdx),
    /// the engine built under PREFIX (`tools/input-methods/build.sh`) run as the session runs it,
    /// in a directory of the run's own with nothing learned before.
    pub input_methods: Option<PathBuf>,
    /// `--input-sources-available METHOD,...`: what the session tells of the input sources is
    /// played (input_sources.rs, docs/adr/0041-input-methods.mdx), its engine having these input
    /// methods (an empty list: none installed), for scenarios that need the state alone (Settings'
    /// Input Sources) without the engine. Refused with `--input-methods`, which tells the real one.
    pub input_sources_available: Option<String>,
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

const USAGE: &str = "usage: oxyos-offscreen --launch [--out DIR] [--outputs WxH[@SCALE][:HZ],... | --displays FILE] [--gpu | --gpu-only] [--dump-frames DIR] [--kms CARD] [--input | --uinput] [--seat] [--layout LAYOUT] [--launch-settle MS] [--input-method [--im-candidates horizontal|vertical]] [--autofill-fixture ID [--autofill-saves]] [--passkey-fixture ID] [--shell PROGRAM,ARG...] [--file-chooser PROGRAM,ARG...] [--companion PROGRAM,ARG... [--companion-at X,Y] [--companion-app ID]] [--lock-password PASSWORD [--lock-pin PIN]] [--login SOCKET] [--reader PROGRAM [--reader-locale LOCALE] [--reader-off]] [--preferences FILE] [--grant-settings ID] [--grant ID=GRANT,...] [--installd SOCKET] [--trash] [--storage FILE] [--runtimes DIR] [--installed-fixture ROOT] [--controls FILE] [--network-bus ADDRESS] [--bluetooth-bus ADDRESS] [--power-bus ADDRESS] [--sound FILE] [--input-methods PREFIX] [--input-sources-available METHOD,...] [[--allow-unchanged] [--until-sheet] [--until-named MS NAME] --press X,Y | \
                     --press-popup X,Y | --press-output X,Y | --press-secondary-output X,Y | --hold-output X,Y,MS | --press-named NAME | --press-secondary-named NAME | --double-press-named NAME | --double-press-output X,Y | --move-named NAME | --hold-named MS NAME | --move-output X,Y | --double-press X,Y | --triple-press X,Y | --double-drag X0,Y0,X1,Y1 | --scroll X,Y,DY[,EVENTS] | --scroll-output X,Y,DY,EVENTS | --scroll-named DY,EVENTS NAME | --swipe FINGERS,DX,DY,STEPS[,HZ] | --swipe-begin FINGERS,DX,DY,STEPS[,HZ] | --swipe-move DX,DY,STEPS[,HZ] | --swipe-end MS | --pinch X,Y,PERCENT,DEGREES | --type TEXT | --preedit TEXT | --commit TEXT | --transaction BEFORE,AFTER COMMIT PREEDIT BEGIN,END SEGMENTS|- | --keyboard TEXT | --compose-keys TEXT | --im-change BEFORE,AFTER COMMIT PREEDIT current|stale | --key CODE[+CODE]... | --key-down CODE[+CODE]... | --key-up CODE[+CODE]... | --drag X0,Y0,X1,Y1 | --drag-output X0,Y0,X1,Y1 | --drag-paced X0,Y0,X1,Y1,MOTIONS,HZ | --scroll-paced X,Y,DY,EVENTS,HZ | --flick X,Y,DY,EVENTS,HZ | --flick-hold X,Y,DY,EVENTS,HZ,AFTER_MS | --move-paced X0,Y0,X1,Y1,MOTIONS,HZ | --press-paced X,Y,COUNT,INTERVAL_MS | --double-click X,Y,HOLD_MS,GAP_MS | --scale SCALE_120 | --a11y-action activate|focus|increment|decrement|scroll-forward|scroll-backward|dismiss|custom:LABEL|select:START-END,NAME | --move-window DX,DY | --resize-window W,H | --minimize-window | --caps-lock | --clipboard LIST | --switch-terminal N | --idle SECONDS | --wait MS | --wait-named MS NAME | --transition-clock hold|run|MS | --expect-popup]... \
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

/// `least` to `most` comma-separated numbers.
fn numbers_within(
    value: Option<&std::ffi::OsString>,
    least: usize,
    most: usize,
) -> Result<Vec<i32>> {
    let given = value
        .and_then(|v| v.to_str())
        .ok_or(USAGE)?
        .split(',')
        .count();
    numbers(value, given.clamp(least, most))
}

impl Launch {
    pub fn parse(args: &[std::ffi::OsString]) -> Result<(PathBuf, Self)> {
        let mut out = PathBuf::from("target/oxyos-offscreen");
        let mut steps: Vec<(Step, Settling)> = Vec::new();
        // `--allow-unchanged` and `--until-sheet` apply to the step after them.
        let mut settling = Settling::default();
        let (mut gpu, mut gpu_only) = (false, false);
        let mut dump_frames = None;
        let mut kms = None;
        let (mut input, mut uinput, mut layout) = (false, false, "us".to_string());
        let mut launch_settle = SETTLE;
        let (mut seat, mut input_method) = (false, false);
        let mut im_candidates = None;
        let mut autofill_fixture = None;
        let mut passkey_fixture = None;
        let mut autofill_saves = false;
        let mut shell = None;
        let mut file_chooser = None;
        let mut companion = None;
        let mut companion_at = None;
        let mut companion_app = None;
        let mut lock_password = None;
        let mut lock_pin = None;
        let (mut reader, mut reader_locale, mut reader_off) = (None, None, false);
        let (mut preferences, mut grant_settings, mut controls) = (None, None, None);
        let mut network_bus = None;
        let mut sound = None;
        let mut bluetooth_bus = None;
        let mut power_bus = None;
        let mut input_methods = None;
        let mut input_sources_available = None;
        let (mut grants, mut installd) = (Vec::new(), None);
        let mut trash = false;
        let mut safe_mode = false;
        let mut storage = None;
        let mut runtimes = None;
        let mut installed_fixture = None;
        let mut screenshots = None;
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
                    settling.may_stay = true;
                    continue;
                }
                Some("--until-sheet") => {
                    settling.sheet = true;
                    continue;
                }
                Some("--until-named") => {
                    let ms = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    let ms = ms.parse().map_err(|_| "--until-named MS NAME")?;
                    let name = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    settling.until_named = Some((ms, name.to_string()));
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
                Some("--file-chooser") => {
                    let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    file_chooser = Some(value.split(',').map(Into::into).collect());
                    continue;
                }
                Some("--companion") => {
                    let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    companion = Some(value.split(',').map(Into::into).collect());
                    continue;
                }
                Some("--companion-at") => {
                    let n = numbers(args.next(), 2)?;
                    companion_at = Some((n[0], n[1]));
                    continue;
                }
                Some("--companion-app") => {
                    let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    companion_app = Some(value.to_string());
                    continue;
                }
                Some("--reader") => {
                    reader = Some(args.next().ok_or(USAGE)?.into());
                    continue;
                }
                Some("--reader-off") => {
                    reader_off = true;
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
                Some("--im-candidates") => {
                    let layout = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    im_candidates = Some(
                        crate::ime::Layout::named(layout)
                            .ok_or_else(|| format!("--im-candidates {layout:?}\n{USAGE}"))?,
                    );
                    continue;
                }
                Some("--passkey-fixture") => {
                    let id = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    passkey_fixture = Some(id.to_string());
                    continue;
                }
                Some("--autofill-fixture") => {
                    let id = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    autofill_fixture = Some(id.to_string());
                    continue;
                }
                Some("--autofill-saves") => {
                    autofill_saves = true;
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
                Some("--input-methods") => {
                    input_methods = Some(PathBuf::from(args.next().ok_or(USAGE)?));
                    continue;
                }
                Some("--sound") => {
                    sound = Some(PathBuf::from(args.next().ok_or(USAGE)?));
                    continue;
                }
                Some("--network-bus") => {
                    let address = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    network_bus = Some(address.to_string());
                    continue;
                }
                Some("--bluetooth-bus") => {
                    let address = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    bluetooth_bus = Some(address.to_string());
                    continue;
                }
                Some("--power-bus") => {
                    let address = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    power_bus = Some(address.to_string());
                    continue;
                }
                Some("--input-sources-available") => {
                    let methods = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    input_sources_available = Some(methods.to_string());
                    continue;
                }
                Some("--installed-fixture") => {
                    installed_fixture = Some(args.next().ok_or(USAGE)?.into());
                    continue;
                }
                Some("--screenshots") => {
                    screenshots = Some(args.next().ok_or(USAGE)?.into());
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
                Some("--trash") => {
                    trash = true;
                    continue;
                }
                Some("--safe-mode") => {
                    safe_mode = true;
                    continue;
                }
                Some("--storage") => {
                    storage = Some(args.next().ok_or(USAGE)?.into());
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
                Some("--lock-pin") => {
                    let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    lock_pin = Some(value.to_string());
                    continue;
                }
                Some("--launch-settle") => {
                    let ms = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    launch_settle = Duration::from_millis(ms.parse()?);
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
                Some("--move-output") => {
                    let n = numbers(args.next(), 2)?;
                    Step::MoveOutput { x: n[0], y: n[1] }
                }
                Some("--release-output") => {
                    let n = numbers(args.next(), 2)?;
                    Step::ReleaseOutput { x: n[0], y: n[1] }
                }
                Some("--press-secondary") => {
                    let n = numbers(args.next(), 2)?;
                    Step::PressSecondary { x: n[0], y: n[1] }
                }
                Some("--press-secondary-output") => {
                    let n = numbers(args.next(), 2)?;
                    Step::PressSecondaryOutput { x: n[0], y: n[1] }
                }
                Some(flag @ ("--swipe" | "--swipe-begin")) => {
                    // F,DX,DY,STEPS[,HZ]: 60 a second unless said.
                    let n = numbers_within(args.next(), 4, 5)?;
                    Step::Swipe {
                        fingers: Some(u8::try_from(n[0])?),
                        dx: n[1],
                        dy: n[2],
                        steps: u32::try_from(n[3])?.max(1),
                        hz: n.get(4).map_or(Ok(60), |hz| u32::try_from(*hz))?.max(1),
                        lift: (flag == "--swipe").then_some(0),
                    }
                }
                Some("--swipe-move") => {
                    let n = numbers_within(args.next(), 3, 4)?;
                    Step::Swipe {
                        fingers: None,
                        dx: n[0],
                        dy: n[1],
                        steps: u32::try_from(n[2])?.max(1),
                        hz: n.get(3).map_or(Ok(60), |hz| u32::try_from(*hz))?.max(1),
                        lift: None,
                    }
                }
                Some("--swipe-end") => {
                    let n = numbers(args.next(), 1)?;
                    Step::Swipe {
                        fingers: None,
                        dx: 0,
                        dy: 0,
                        steps: 0,
                        hz: 60,
                        lift: Some(u32::try_from(n[0])?),
                    }
                }
                Some("--scroll-output") => {
                    let n = numbers(args.next(), 4)?;
                    Step::ScrollOutput {
                        x: n[0],
                        y: n[1],
                        dy: n[2],
                        events: u32::try_from(n[3])?,
                    }
                }
                Some("--scroll-named") => {
                    // DY,EVENTS NAME: a wheel's notch (EVENTS 0) or a touchpad's fingers.
                    let n = numbers(args.next(), 2)?;
                    let name = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    Step::Named {
                        kind: NamedKind::Scroll {
                            dy: n[0],
                            events: u32::try_from(n[1])?,
                        },
                        name: name.to_string(),
                        ms: 0,
                    }
                }
                Some(
                    flag @ ("--press-named"
                    | "--press-secondary-named"
                    | "--double-press-named"
                    | "--move-named"
                    | "--hold-named"),
                ) => {
                    let kind = match flag {
                        "--press-named" => NamedKind::Press,
                        "--press-secondary-named" => NamedKind::PressSecondary,
                        "--double-press-named" => NamedKind::DoublePress,
                        "--move-named" => NamedKind::Move,
                        _ => NamedKind::Hold,
                    };
                    let ms = if kind == NamedKind::Hold {
                        let value = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                        value.parse::<u32>().map_err(|_| "--hold-named NAME MS")?
                    } else {
                        0
                    };
                    let name = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    Step::Named {
                        kind,
                        name: name.to_string(),
                        ms,
                    }
                }
                Some("--drag-paced-named") => {
                    let value = args.next();
                    let given = value
                        .and_then(|v| v.to_str())
                        .map_or(0, |v| v.split(',').count());
                    let n = numbers(value, given.clamp(2, 5))?;
                    if n[0] < 1 || n[1] < 1 {
                        return Err(
                            "--drag-paced-named needs at least one motion at 1 Hz or more".into(),
                        );
                    }
                    let from = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    let to = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    Step::DragPacedNamed {
                        from: from.to_string(),
                        to: to.to_string(),
                        motions: n[0] as u32,
                        hz: n[1] as u32,
                        escape_after: n.get(2).filter(|at| **at > 0).map(|at| *at as u32),
                        rest_ms: n.get(3).map_or(0, |ms| (*ms).max(0) as u32),
                        held: n.get(4).is_some_and(|held| *held != 0),
                    }
                }
                Some("--hold-output") => {
                    let n = numbers(args.next(), 3)?;
                    if n[2] < 1 {
                        return Err("--hold-output holds the press for 1 ms or more".into());
                    }
                    Step::HoldOutput {
                        x: n[0],
                        y: n[1],
                        ms: n[2] as u32,
                    }
                }
                Some("--double-press") => {
                    let n = numbers(args.next(), 2)?;
                    Step::DoublePress { x: n[0], y: n[1] }
                }
                Some("--double-press-output") => {
                    let n = numbers(args.next(), 2)?;
                    Step::DoublePressOutput { x: n[0], y: n[1] }
                }
                Some("--triple-press") => {
                    let n = numbers(args.next(), 2)?;
                    Step::TriplePress { x: n[0], y: n[1] }
                }
                Some("--double-drag") => {
                    let n = numbers(args.next(), 4)?;
                    Step::DoubleDrag {
                        from: (n[0], n[1]),
                        to: (n[2], n[3]),
                    }
                }
                Some("--scroll") => {
                    // X,Y,DY a wheel's notches, X,Y,DY,EVENTS a touchpad's fingers.
                    let value = args.next();
                    let n = numbers(value, 4).or_else(|_| numbers(value, 3))?;
                    Step::Scroll {
                        x: n[0],
                        y: n[1],
                        dy: n[2],
                        events: n
                            .get(3)
                            .copied()
                            .map(u32::try_from)
                            .transpose()?
                            .unwrap_or(0),
                    }
                }
                Some("--pinch") => {
                    // X,Y,PERCENT,DEGREES[,FINGERS]: two fingers unless said.
                    let n = numbers_within(args.next(), 4, 5)?;
                    Step::Pinch {
                        x: n[0],
                        y: n[1],
                        percent: n[2],
                        degrees: n[3],
                        fingers: n.get(4).map_or(Ok(2), |f| u8::try_from(*f))?,
                    }
                }
                Some("--type") => {
                    let text = args.next().and_then(|t| t.to_str()).ok_or(USAGE)?;
                    Step::Type(text.to_string())
                }
                Some("--strokes") => {
                    // Chords separated by spaces, each its codes joined by `+`: "40 18" is ´ then e.
                    let chords = args.next().and_then(|c| c.to_str()).ok_or(USAGE)?;
                    Step::Strokes(
                        chords
                            .split_whitespace()
                            .map(|chord| chord.split('+').map(str::parse).collect())
                            .collect::<std::result::Result<_, _>>()?,
                    )
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
                Some(flag @ ("--drag-paced" | "--drag-hold")) => {
                    let held = flag == "--drag-hold";
                    let value = args.next();
                    let given = value
                        .and_then(|v| v.to_str())
                        .map_or(0, |v| v.split(',').count());
                    let n = numbers(value, given.clamp(6, 8))?;
                    if n[4] < 1 || n[5] < 1 {
                        return Err(
                            format!("{flag} needs at least one motion at 1 Hz or more").into()
                        );
                    }
                    Step::DragPaced {
                        from: (n[0], n[1]),
                        to: (n[2], n[3]),
                        motions: n[4] as u32,
                        hz: n[5] as u32,
                        escape_after: n.get(6).filter(|at| **at > 0).map(|at| *at as u32),
                        rest_ms: n.get(7).map_or(0, |ms| (*ms).max(0) as u32),
                        held,
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
                        lift: false,
                        hold_after_ms: None,
                    }
                }
                Some("--flick") => {
                    let n = numbers(args.next(), 5)?;
                    if n[3] < 1 || n[4] < 1 {
                        return Err("--flick needs at least one event at 1 Hz or more".into());
                    }
                    Step::ScrollPaced {
                        at: (n[0], n[1]),
                        dy: n[2],
                        events: n[3] as u32,
                        hz: n[4] as u32,
                        lift: true,
                        hold_after_ms: None,
                    }
                }
                Some("--flick-hold") => {
                    let n = numbers(args.next(), 6)?;
                    if n[3] < 1 || n[4] < 1 || n[5] < 0 {
                        return Err(
                            "--flick-hold needs at least one event at 1 Hz or more, and a delay"
                                .into(),
                        );
                    }
                    Step::ScrollPaced {
                        at: (n[0], n[1]),
                        dy: n[2],
                        events: n[3] as u32,
                        hz: n[4] as u32,
                        lift: true,
                        hold_after_ms: Some(n[5] as u32),
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
                Some("--double-click") => {
                    let n = numbers(args.next(), 4)?;
                    if n[2] < 0 || n[3] < 0 {
                        return Err("--double-click needs times of 0 ms or more".into());
                    }
                    Step::DoubleClick {
                        at: (n[0], n[1]),
                        hold_ms: n[2] as u32,
                        gap_ms: n[3] as u32,
                    }
                }
                Some("--press-paced") => {
                    let n = numbers(args.next(), 4)?;
                    if n[2] < 1 || n[3] < 0 {
                        return Err(
                            "--press-paced needs at least one press, 0 ms or more apart".into()
                        );
                    }
                    Step::PressPaced {
                        at: (n[0], n[1]),
                        count: n[2] as u32,
                        interval_ms: n[3] as u32,
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
                Some("--menu") => {
                    let path = args.next().and_then(|p| p.to_str()).ok_or(USAGE)?;
                    Step::Menu(path.to_string())
                }
                Some("--sound-act") => {
                    let what = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    Step::Sound(what.to_string())
                }
                Some("--network") => {
                    let what = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    Step::Network(what.to_string())
                }
                Some("--bluetooth") => {
                    let what = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    Step::Bluetooth(what.to_string())
                }
                Some("--power") => {
                    let what = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    Step::Power(what.to_string())
                }
                Some("--clipboard") => {
                    let list = args.next().ok_or(USAGE)?;
                    let mut contents = Vec::new();
                    for line in std::fs::read_to_string(list)?.lines() {
                        let (mime, path) = line.split_once('\t').ok_or(USAGE)?;
                        let mime = TextChunk::new(mime).ok_or("a type is 1 to 32 bytes")?;
                        contents.push((mime, std::fs::read(path)?));
                    }
                    Step::Clipboard(contents)
                }
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
                Some("--transition-clock") => {
                    let at = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    // Nothing of the application changes: the step's answer is the frame.
                    settling.may_stay = true;
                    Step::TransitionClock(match at {
                        "hold" => TransitionClock::Hold,
                        "run" => TransitionClock::Run,
                        ms => TransitionClock::At(
                            ms.parse().map_err(|_| "--transition-clock hold|run|MS")?,
                        ),
                    })
                }
                Some("--wait-named") => {
                    let ms = args.next().and_then(|s| s.to_str()).ok_or(USAGE)?;
                    let ms = ms.parse().map_err(|_| "--wait-named MS NAME")?;
                    let name = args.next().and_then(|v| v.to_str()).ok_or(USAGE)?;
                    Step::WaitNamed {
                        name: name.to_string(),
                        ms,
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
            steps.push((step, std::mem::take(&mut settling)));
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
                launch_settle,
                input_method,
                im_candidates,
                autofill_fixture,
                passkey_fixture,
                autofill_saves,
                shell,
                file_chooser,
                companion,
                companion_at,
                companion_app,
                lock_password,
                lock_pin,
                reader,
                reader_locale,
                reader_off,
                preferences,
                grant_settings,
                grants,
                installd,
                trash,
                safe_mode,
                storage,
                runtimes,
                installed_fixture,
                screenshots,
                controls,
                network_bus,
                sound,
                bluetooth_bus,
                power_bus,
                input_methods,
                input_sources_available,
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
                if key.client > ClientId::MAX / 2 && key.client < crate::pasteboard::PASTEBOARD =>
            {
                Some((key, frame.sequence, rect))
            }
            _ => None,
        })
        .collect()
}

/// The windows of the applications the session launched (the shell's dock, a store's open), bottom
/// to top: each where it is and the configure its shown frame answers. Not each frame's sequence:
/// an application's content is checked from its own report (`.launched`) and may animate on for as
/// long as it likes (a storefront's skeleton while the store is asked again), but what the session
/// does to its window (it arrives, is zoomed, moved, resized or closed) is a step's answer.
fn launched_windows(session: &Session) -> Vec<(SurfaceKey, Rect, u64)> {
    session
        .compositor
        .scene
        .render_list()
        .into_iter()
        .filter_map(|item| match item {
            RenderItem::Surface { key, rect, frame }
                if session.launched_clients.contains(&key.client) =>
            {
                Some((key, rect, frame.configure))
            }
            _ => None,
        })
        .collect()
}

/// The companion application's surfaces (`--companion`), bottom to top, with their frames.
fn companion_frames(session: &Session) -> Vec<(SurfaceKey, u64, Rect)> {
    let Some(client) = session.companion_client() else {
        return Vec::new();
    };
    session
        .compositor
        .scene
        .render_list()
        .into_iter()
        .filter_map(|item| match item {
            RenderItem::Surface { key, rect, frame } if key.client == client => {
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

/// Starts the companion application (`--companion`) on the application's socket, as a second
/// application the person opened, and serves the session until its window is shown and nothing
/// has changed for SETTLE.
fn start_companion(
    session: &mut Session,
    children: &mut Children,
    command: &[std::ffi::OsString],
    at: Option<(i32, i32)>,
    app: Option<&str>,
) -> Result<()> {
    let mut companion = Command::new(command.first().ok_or(USAGE)?);
    companion
        .args(&command[1..])
        .env("OXYOS_COMPOSITOR_SOCKET", &session.socket)
        .env("OXYOS_APP_ID", app.unwrap_or("so.oxy.companion"))
        .env("OXYOS_APP_TITLE", "Companion")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    if let Some(bundle) = command
        .get(1)
        .map(std::path::Path::new)
        .filter(|b| b.is_file())
    {
        let url = oxyos_apps::script_url(bundle);
        companion.env("OXYOS_SCRIPT_URL", format!("file://{}", url.display()));
    }
    if let Some(mut report) = std::env::var_os("OXYOS_HOST_REPORT") {
        report.push(".companion");
        companion.env("OXYOS_HOST_REPORT", report);
    }
    if app.is_none() {
        companion.env("OXYOS_APP_UNRECORDED", "1");
    }
    let child = companion.spawn()?;
    let pid = child.id();
    session.companion = Some(pid);
    session.companion_app = app.map(str::to_string);
    children.0.push(child);
    let deadline = Instant::now() + LAUNCH_DEADLINE;
    let mut last = (Vec::new(), Instant::now());
    loop {
        if Instant::now() > deadline {
            return Err("deadline exceeded waiting for the companion's window".into());
        }
        session.compositor.serve(&session.listener)?;
        let client = session.companion_client();
        let shown =
            client.is_some_and(|client| {
                session.compositor.scene.render_list().iter().any(
                    |item| matches!(item, RenderItem::Surface { key, .. } if key.client == client),
                )
            });
        if let (true, Some(at), Some(client)) = (shown, at, client) {
            let key = SurfaceKey { client, id: WINDOW };
            if session
                .compositor
                .scene
                .global_rect(key)
                .map(|r| (r.x, r.y))
                != Some(at)
            {
                session.compositor.scene.move_window_to(key, at);
            }
        }
        let now = composed(&session.compositor);
        if now != last.0 {
            last = (now, Instant::now());
        } else if shown && last.1.elapsed() >= SETTLE {
            eprintln!("[companion] shown");
            return Ok(());
        }
    }
}

/// The screen reader's socket, granting `accessibility` and `screen_reader`, as the session
/// listens on one whether its reader is on or off; what it will say is recorded in `record`.
fn listen_for_reader(session: &mut Session, record: &Path) -> Result<PathBuf> {
    let socket = session.socket.with_file_name("reader");
    let listener = oxyos_ipc::Listener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    session.compositor.reader_listener = Some(listener);
    if record.exists() {
        fs::remove_file(record)?;
    }
    Ok(socket)
}

/// Starts the screen reader on its socket, saying what it says into `record` (appended to, so
/// every time it is on is in it), and serves the session until it is connected.
fn start_reader(
    session: &mut Session,
    socket: &Path,
    program: &Path,
    locale: Option<&str>,
    record: &Path,
) -> Result<std::process::Child> {
    let mut reader = Command::new(program);
    reader
        .env("OXYOS_COMPOSITOR_SOCKET", socket)
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

/// Super+F5 (`Action::ToggleReader`, which the compositor keeps for the program running it):
/// starts the screen reader if it is off and stops it if it is on, as `oxyos-session` does
/// (`switch_reader`), whether the session is locked or not. Without a reader named, nothing.
fn toggle_reader(
    session: &mut Session,
    running: &mut Children,
    reader: Option<(&Path, &Path, Option<&str>)>,
    record: &Path,
) -> Result<()> {
    if !session.compositor.take_reader_toggle() {
        return Ok(());
    }
    let Some((socket, program, locale)) = reader else {
        eprintln!("[reader] no screen reader (--reader)");
        return Ok(());
    };
    match running.0.pop() {
        Some(mut child) => {
            eprintln!("[reader] stopping");
            child.kill()?;
            child.wait()?;
            // Its connection's end, as the scene sees it go.
            let deadline = Instant::now() + LAUNCH_DEADLINE;
            while session.compositor.scene.screen_reader().is_some() {
                if Instant::now() > deadline {
                    return Err("deadline exceeded waiting for the screen reader to go".into());
                }
                session.compositor.serve(&session.listener)?;
            }
        }
        None => {
            eprintln!("[reader] starting");
            running
                .0
                .push(start_reader(session, socket, program, locale, record)?);
        }
    }
    Ok(())
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
    // The candidates surface shown (ADR 0041): where the compositor placed it, on the output.
    let candidates = compositor
        .scene
        .shown_candidates()
        .first()
        .map_or("null".to_string(), |(_, r)| rect(r));
    format!(
        "{{\"surfaces\":[{}],\"candidates\":{candidates},\"work_area\":{area},\"work_areas\":{{{}}},\"locked\":{},\"signed_in\":{}}}",
        surfaces.join(","),
        areas.join(","),
        compositor.scene.is_locked(),
        compositor.signed_in
    )
}

/// Passkeys' sheets during a step (docs/adr/0050-keychain-autofill-and-passkeys.mdx), with
/// `--passkey-fixture`: what the fixture accounts' service was answered so far, in order (never a
/// password); `null` without it.
fn passkeys_json(session: &Session) -> String {
    let Some(service) = session.passkey_service.as_ref() else {
        return "null".to_string();
    };
    let told: Vec<String> = service
        .borrow()
        .told
        .iter()
        .map(|told| json_string(told))
        .collect();
    format!("{{\"told\":[{}]}}", told.join(","))
}

/// AutoFill during a step (docs/adr/0050-keychain-autofill-and-passkeys.mdx), with
/// `--autofill-fixture`: the list a shell draws now (its items, the highlighted one) and what the
/// fixture keychain was asked since the last step; `null` without it.
fn autofill_json(session: &Session) -> String {
    let Some(keychain) = session.autofill_keychain.as_ref() else {
        return "null".to_string();
    };
    let asked: Vec<String> = std::mem::take(&mut keychain.borrow_mut().asked)
        .iter()
        .map(|asked| json_string(asked))
        .collect();
    let shown =
        session
            .compositor
            .scene
            .autofill_shown()
            .map_or("null".to_string(), |(_, list)| {
                let items: Vec<String> = list
                    .items
                    .iter()
                    .map(|item| {
                        format!(
                            "{{\"kind\":\"{:?}\",\"title\":{},\"detail\":{}}}",
                            item.kind,
                            json_string(&item.title),
                            json_string(&item.detail)
                        )
                    })
                    .collect();
                format!(
                    "{{\"highlighted\":{},\"items\":[{}]}}",
                    list.highlighted,
                    items.join(",")
                )
            });
    format!("{{\"shown\":{shown},\"asked\":[{}]}}", asked.join(","))
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

/// Where the frame composed last drew each window, in order.
fn drawn_windows(compositor: &Compositor) -> Vec<(SurfaceKey, Rect)> {
    let mut drawn: Vec<(SurfaceKey, Rect)> = compositor
        .windows_drawn
        .iter()
        .map(|(key, rect)| (*key, *rect))
        .collect();
    drawn.sort_by_key(|(key, _)| (key.client, key.id));
    drawn
}

/// The windows in the order they are drawn, bottom to top: a window raised (an application brought
/// forward from the dock, ADR 0034's decision 9) answers a step though no client draws anew.
fn window_order(compositor: &Compositor) -> Vec<SurfaceKey> {
    compositor
        .scene
        .render_list()
        .into_iter()
        .filter_map(|item| match item {
            RenderItem::Surface { key, .. } => compositor.scene.window_number(key).map(|_| key),
            _ => None,
        })
        .collect()
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

/// `Step::TransitionClock`'s settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionClock {
    Hold,
    At(u64),
    Run,
}

thread_local! {
    /// The fingers a `Step::Swipe` left down.
    static SWIPING: std::cell::Cell<Option<(u8, u64)>> = const { std::cell::Cell::new(None) };
    /// Where `--transition-clock hold` held the transitions' clock.
    static HELD: std::cell::Cell<Option<Duration>> = const { std::cell::Cell::new(None) };
}

/// When a step's answer first appeared, when it settled, and the window then.
struct Settled {
    window: Rect,
    first_frame_ns: Option<i64>,
    /// On a display: the kernel's flip time of the first frame showing the answer.
    first_flip_ns: Option<i64>,
    settled_ns: i64,
}

/// What `settle` waits for: until when, what (to say when it gives up), how long the surfaces
/// must then stay unchanged, whether the answer is a system sheet (`Settling::sheet`) and the
/// node the answer shows (`Settling::until_named`).
struct Wait<'a> {
    deadline: Instant,
    waiting_for: &'a str,
    still: Duration,
    sheet: bool,
    named: Option<&'a str>,
}

/// Serves the session until the application's surfaces differ from `before` (a new frame, a
/// popup opened or closed) and then stay unchanged for the wait's `still`. With `may_stay`, surfaces
/// that do not change for it also count as settled (a step whose expected answer is nothing).
fn settle(
    session: &mut Session,
    children: &mut Children,
    before: &[(u32, u64, Rect)],
    may_stay: bool,
    Wait {
        deadline,
        waiting_for,
        still,
        sheet,
        named,
    }: Wait,
    // Where the window was: a step may leave it off the screen (minimized), never the first frame.
    window_before: Option<Rect>,
) -> Result<Settled> {
    // The shell's surfaces count too: a press on a panel changes the shell, not the application.
    let mut last = (before.to_vec(), Instant::now());
    let mut shell_last = shell_surfaces(&session.compositor);
    let shell_before: Vec<SurfaceKey> = shell_last.iter().map(|(key, _, _)| *key).collect();
    // The launched applications' windows count as the shell's do: a zoom of one changes neither
    // the director's application nor the shell, and would otherwise settle only on something
    // else changing (the menu bar's clock turning a minute).
    let mut launched_last = launched_windows(session);
    // The companion's frames count as the application's do: it is an application the person
    // opened, and what it shows (a caret blinking in its field) answers a step as much.
    let mut companion_last = companion_frames(session);
    // Where the compositor draws the windows (ADR 0046): a swipe's answer is the windows moved,
    // which no client draws.
    let mut drawn_last = drawn_windows(&session.compositor);
    let mut order_last = window_order(&session.compositor);
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
        session.serve_background();
        session.serve_preferences();
        if let Some(ime) = session.ime.as_mut() {
            ime.serve(&mut session.compositor)?;
        }
        if let Some(pasteboard) = session.pasteboard.as_mut() {
            pasteboard.serve(&mut session.compositor);
        }
        session.reader.absorb(&mut session.compositor);
        let now = shown(&session.compositor);
        let shell_now = shell_surfaces(&session.compositor);
        let launched_now = launched_windows(session);
        let launched_changed = launched_now != launched_last;
        launched_last = launched_now;
        let companion_now = companion_frames(session);
        let companion_changed = companion_now != companion_last;
        companion_last = companion_now;
        let drawn_now = drawn_windows(&session.compositor);
        let drawn_changed = drawn_now != drawn_last;
        drawn_last = drawn_now;
        let order_now = window_order(&session.compositor);
        let order_changed = order_now != order_last;
        order_last = order_now;
        let shell_changed = shell_now != shell_last
            || launched_changed
            || companion_changed
            || drawn_changed
            || order_changed;
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
            .is_none_or(|shown| Some(shown) == session.compositor.scene.configure_serial(window))
            // A launched application's window too, zoomed from its title bar, say.
            && launched_last.iter().all(|(key, _, shown)| {
                Some(*shown) == session.compositor.scene.configure_serial(*key)
            });
        // A sheet asked for is shown once the shell shows a surface it did not show before.
        let sheet_shown = !sheet
            || session.compositor.scene.sheets_waiting() > 0
                && shell_last
                    .iter()
                    .any(|(key, _, _)| !shell_before.contains(key));
        // A node the answer shows, once the assistive technology holds it.
        let named_shown = named.is_none_or(|name| named_point(session, name).is_some());
        if !changed
            && !shell_changed
            && answered
            && sheet_shown
            && named_shown
            && (last.0 != before || first_frame_ns.is_some() || may_stay)
            && last.1.elapsed() >= still
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

/// Where the application's window minimizes into, as the shell's dock says it (ADR 0011's genie):
/// `[x, y, width, height]` in the global space, or null.
fn minimize_target_json(compositor: &Compositor) -> String {
    let window = SurfaceKey {
        client: CLIENT,
        id: WINDOW,
    };
    match compositor.scene.minimize_target(window) {
        Some(r) => format!("[{},{},{},{}]", r.x, r.y, r.width, r.height),
        None => "null".to_string(),
    }
}

/// Each window as the shell is told of it once the step settled, by its number: its application,
/// its desktop, and whether it is minimized, hidden with its application and active (the dock's
/// press, ADR 0034's decision 9).
fn window_states_json(compositor: &Compositor) -> String {
    let states: Vec<String> = compositor
        .scene
        .shell_window_states()
        .iter()
        .map(|w| {
            format!(
                "\"{}\":{{\"app\":{},\"workspace\":{},\"minimized\":{},\"hidden\":{},\"focused\":{}}}",
                w.number,
                json_string(&w.app_id),
                w.workspace,
                w.minimized,
                w.hidden,
                w.focused
            )
        })
        .collect();
    format!("{{{}}}", states.join(","))
}

/// The surface of the application holding keyboard focus, or null.
/// Where the person's input goes once a step settled, whichever client's: the surface with the
/// keyboard focus and the surface the pointer is over (its enter), each as {"client", "pid",
/// "surface", "shown"}, or null; regress.py checks them against the events each host received,
/// and that the keyboard's is shown (keys to a surface that is not reach nobody).
fn input_focus_json(compositor: &Compositor) -> String {
    let key = |key: Option<oxyos_compositor_core::SurfaceKey>| {
        key.map_or("null".to_string(), |key| {
            let pid = compositor.peers.get(&key.client).map_or(0, |peer| peer.pid);
            format!(
                "{{\"client\":{},\"pid\":{pid},\"surface\":{},\"shown\":{}}}",
                key.client,
                key.id,
                compositor.scene.shown(key)
            )
        })
    };
    format!(
        "{{\"keyboard\":{},\"pointer\":{}}}",
        key(compositor.scene.focus()),
        key(compositor.scene.pointer_focus())
    )
}

/// Every input event routed to a client in the run, in order (`Compositor::delivery_log`): each
/// [t_ns, client, pid, surface, kind, serial].
fn deliveries_json(compositor: &Compositor) -> String {
    let entries: Vec<String> = compositor
        .delivery_log
        .iter()
        .flatten()
        .map(|d| {
            format!(
                "[{},{},{},{},\"{}\",{}]",
                d.t_ns, d.client, d.pid, d.surface, d.kind, d.serial
            )
        })
        .collect();
    format!("[{}]", entries.join(","))
}

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

/// The text-input transactions the application was given during a step (ADR 0008), in order,
/// each as its parts: the bytes deleted around the selection, the text committed, the
/// composition, its cursor and its segments with their styles; for a field whose text is a
/// secret, only `withheld`.
fn transactions_json(session: &Session) -> String {
    let mut transactions = Vec::new();
    let (mut delete, mut commit, mut preedit) = ((0, 0), String::new(), String::new());
    let (mut cursor, mut segments) = ((0, 0), Vec::new());
    // AutoFill's fill of the field and of its pair, by its bytes.
    let (mut fill, mut paired_fill): (Option<String>, Option<usize>) = (None, None);
    for (client, event) in &session.compositor.text_input_log {
        if *client != CLIENT {
            continue;
        }
        match event {
            oxyos_protocol::Event::TextInputDelete { before, after, .. } => {
                delete = (*before, *after)
            }
            oxyos_protocol::Event::TextInputCommitText { text, .. } => {
                commit.push_str(text.as_str())
            }
            oxyos_protocol::Event::TextInputPreeditText { text, .. } => {
                preedit.push_str(text.as_str())
            }
            oxyos_protocol::Event::TextInputPreeditCursor { begin, end, .. } => {
                cursor = (*begin, *end)
            }
            oxyos_protocol::Event::TextInputFill { paired, text, .. } => {
                if *paired {
                    *paired_fill.get_or_insert(0) += text.as_str().len();
                } else {
                    fill.get_or_insert_with(String::new).push_str(text.as_str());
                }
            }
            oxyos_protocol::Event::TextInputPreeditSegment {
                begin, end, style, ..
            } => segments.push(format!(
                "[{begin},{end},\"{}\"]",
                format!("{style:?}").to_lowercase()
            )),
            oxyos_protocol::Event::TextInputDone { surface, .. } => {
                // What is typed into a password, a PIN or a sensitive field is never written
                // anywhere (ADR 0008, decision 4): only that a transaction was given.
                let field = SurfaceKey {
                    client: CLIENT,
                    id: *surface,
                };
                let secret = session
                    .compositor
                    .scene
                    .text_input(field)
                    .is_none_or(|state| {
                        matches!(
                            state.purpose,
                            oxyos_protocol::TextPurpose::Password
                                | oxyos_protocol::TextPurpose::Pin
                        ) || state.hints.contains(oxyos_protocol::TextHints::SENSITIVE)
                    });
                // A fill (docs/adr/0050-keychain-autofill-and-passkeys.mdx): the field's text, as
                // any transaction's, and only how long its paired field's is (a password).
                if fill.is_some() || paired_fill.is_some() {
                    let own = match fill.take() {
                        Some(_) if secret => "{\"withheld\":true}".to_string(),
                        Some(text) => json_string(&text),
                        None => "null".to_string(),
                    };
                    let paired = paired_fill
                        .take()
                        .map_or("null".to_string(), |bytes| bytes.to_string());
                    transactions.push(format!("{{\"fill\":{own},\"paired_fill_bytes\":{paired}}}"));
                    continue;
                }
                if secret {
                    transactions.push("{\"withheld\":true}".to_string());
                    (delete, cursor) = ((0, 0), (0, 0));
                    commit.clear();
                    preedit.clear();
                    segments.clear();
                    continue;
                }
                transactions.push(format!(
                    "{{\"delete\":[{},{}],\"commit\":{},\"preedit\":{},\"cursor\":[{},{}],\"segments\":[{}]}}",
                    delete.0,
                    delete.1,
                    json_string(&commit),
                    json_string(&preedit),
                    cursor.0,
                    cursor.1,
                    segments.join(",")
                ));
                delete = (0, 0);
                commit.clear();
                preedit.clear();
                cursor = (0, 0);
                segments.clear();
            }
            _ => {}
        }
    }
    format!("[{}]", transactions.join(","))
}

/// The input routed during a step: for the application, counts per surface and kind; for the
/// session's shell, its count (`shell`); for the applications the shell launched, counts per kind
/// (`launched`); for any other client, a count (`leaked`).
fn delivered_json(session: &Session) -> String {
    let compositor = &session.compositor;
    let mut counts: std::collections::BTreeMap<u32, std::collections::BTreeMap<&str, usize>> =
        Default::default();
    let mut launched: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut chooser: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut companion: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut shell_drags: std::collections::BTreeMap<&str, usize> = Default::default();
    let companion_client = session.companion_client();
    let (mut shell, mut leaked) = (0, 0);
    for &(client, surface, kind) in &compositor.input_log {
        if client == CLIENT {
            *counts.entry(surface).or_default().entry(kind).or_default() += 1;
        } else if Some(client) == companion_client {
            *companion.entry(kind).or_default() += 1;
        } else if client > ClientId::MAX / 2 && client < crate::pasteboard::PASTEBOARD {
            shell += 1;
            // A drag's events, which the shell's dock may be the target of (ADR 0035), by kind.
            if kind.starts_with("drag.") {
                *shell_drags.entry(kind).or_default() += 1;
            }
        } else if session.chooser_clients.contains(&client) {
            *chooser.entry(kind).or_default() += 1;
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
    // What the file chooser received (ADR 0029, decision 8).
    if !chooser.is_empty() {
        let kinds: Vec<String> = chooser
            .iter()
            .map(|(k, n)| format!("\"{k}\":{n}"))
            .collect();
        fields.push(format!("\"chooser\":{{{}}}", kinds.join(",")));
    }
    // What the companion received (ADR 0035), and the drag events the shell's surfaces did.
    for (name, kinds) in [("companion", &companion), ("shell_drags", &shell_drags)] {
        if !kinds.is_empty() {
            let kinds: Vec<String> = kinds.iter().map(|(k, n)| format!("\"{k}\":{n}")).collect();
            fields.push(format!("\"{name}\":{{{}}}", kinds.join(",")));
        }
    }
    fields.push(format!("\"leaked\":{leaked}"));
    format!("{{{}}}", fields.join(","))
}

/// The file chooser's sheet shown (docs/adr/0029-chromium-on-oxyos.mdx, decision 8): where it is
/// on the output, whether it has the keyboard and whether the application's window is the active
/// one; null with none.
fn chooser_json(compositor: &Compositor) -> String {
    let scene = &compositor.scene;
    let Some((client, sheet)) = compositor.peers.keys().find_map(|client| {
        let id = scene.chooser_of(*client)?;
        Some((*client, scene.chooser_sheet(id)?))
    }) else {
        return "null".to_string();
    };
    let Some(rect) = compositor.surface_rect(client, sheet.id) else {
        return "null".to_string();
    };
    format!(
        "{{\"sheet\":[{},{},{},{}],\"focused\":{},\"window_active\":{}}}",
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        scene.focus() == Some(sheet),
        scene.active_window()
            == Some(SurfaceKey {
                client: CLIENT,
                id: WINDOW
            }),
    )
}

/// The screenshots saved in a step (ADR 0024, decision 10), each its path, output and size, and
/// those the shell opened, each the application that opened it.
fn screenshots_json(session: &mut Session) -> String {
    let saved: Vec<String> = std::mem::take(&mut session.compositor.saved)
        .into_iter()
        .map(|saved| match saved.path {
            Ok(path) => format!(
                "{{\"path\":{},\"output\":{},\"width\":{},\"height\":{}}}",
                json_string(&path.to_string_lossy()),
                saved.output,
                saved.width,
                saved.height
            ),
            Err(error) => format!("{{\"error\":{}}}", json_string(&error)),
        })
        .collect();
    let opened: Vec<String> = std::mem::take(&mut session.screenshot_opens)
        .into_iter()
        .map(|(path, application)| {
            format!(
                "{{\"path\":{},\"application\":{}}}",
                json_string(&path.to_string_lossy()),
                json_string(&application)
            )
        })
        .collect();
    format!(
        "{{\"saved\":[{}],\"opened\":[{}]}}",
        saved.join(","),
        opened.join(",")
    )
}

/// The URLs the shell opened in the step (docs/adr/0034-the-dock.mdx), each with the application
/// started for it.
/// A paced drag's numbers after its rate, as a step's JSON gives them: the Escape's motion (0 for
/// none) and the rest, each only when given.
fn rest_suffix(escape_after: Option<u32>, rest_ms: u32) -> String {
    match (escape_after, rest_ms) {
        (None, 0) => String::new(),
        (Some(at), 0) => format!(",{at}"),
        (at, rest) => format!(",{},{rest}", at.unwrap_or(0)),
    }
}

/// The system sounds played in a step (docs/adr/0044-system-sounds.mdx), in order, by their names
/// in the sound theme (`file-trash`).
fn sounds_json(session: &Session) -> String {
    let played: Vec<String> = session
        .sounds
        .take()
        .into_iter()
        .map(|sound| json_string(oxyos_sound::name(sound)))
        .collect();
    format!("[{}]", played.join(","))
}

/// The files the shell had opened with an application in the step (ADR 0034, OpenFiles): each
/// file's path and the application, in order. Taken: the next step reports its own.
fn files_opened_json(session: &mut Session) -> String {
    let opened: Vec<String> = std::mem::take(&mut session.file_opens)
        .into_iter()
        .map(|(path, application)| {
            format!(
                "{{\"path\":{},\"application\":{}}}",
                json_string(&path.to_string_lossy()),
                json_string(&application)
            )
        })
        .collect();
    format!("[{}]", opened.join(","))
}

fn urls_json(session: &mut Session) -> String {
    let opened: Vec<String> = std::mem::take(&mut session.url_opens)
        .into_iter()
        .map(|(url, application)| {
            format!(
                "{{\"url\":{},\"application\":{}}}",
                json_string(&url),
                json_string(&application)
            )
        })
        .collect();
    format!("[{}]", opened.join(","))
}

/// The applications' background activity (docs/adr/0060-login-items-and-background-activity.mdx):
/// those with a process running in the background once the step settled, and the processes the
/// session ended in the step (each its application's id, in order).
fn background_json(session: &mut Session) -> String {
    let running: Vec<String> = session
        .background
        .as_ref()
        .map(|background| background.in_background().into_iter().collect())
        .unwrap_or_default();
    let ended = std::mem::take(&mut session.background_ended);
    let list = |ids: &[String]| {
        ids.iter()
            .map(|id| json_string(id))
            .collect::<Vec<_>>()
            .join(",")
    };
    // What each process in the background has used so far: its CPU time and how many times its
    // threads woke (their context switches), which a step's idle check compares with the last.
    let used: Vec<String> = session
        .background
        .as_ref()
        .map(|background| {
            background
                .background_processes()
                .iter()
                .filter_map(|(id, pid)| {
                    let (cpu_ms, wakeups) = process_use(*pid)?;
                    Some(format!(
                        "{}:{{\"cpu_ms\":{cpu_ms:.1},\"wakeups\":{wakeups}}}",
                        json_string(id)
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    format!(
        "{{\"running\":[{}],\"ended\":[{}],\"used\":{{{}}}}}",
        list(&running),
        list(&ended),
        used.join(",")
    )
}

/// A process's CPU time so far, in ms (its threads' run time, the scheduler's), and how many times
/// its threads were switched out (woken and run again): read from /proc.
fn process_use(pid: u32) -> Option<(f64, u64)> {
    let mut cpu_ns = 0u64;
    let mut switches = 0u64;
    for task in std::fs::read_dir(format!("/proc/{pid}/task"))
        .ok()?
        .flatten()
    {
        if let Ok(stat) = std::fs::read_to_string(task.path().join("schedstat")) {
            cpu_ns += stat
                .split_whitespace()
                .next()
                .and_then(|ns| ns.parse::<u64>().ok())
                .unwrap_or(0);
        }
        if let Ok(status) = std::fs::read_to_string(task.path().join("status")) {
            switches += status
                .lines()
                .filter(|line| line.contains("ctxt_switches"))
                .filter_map(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
                .sum::<u64>();
        }
    }
    Some((cpu_ns as f64 / 1e6, switches))
}

/// The notifications the session keeps once the step settled (docs/adr/0026-notifications.mdx),
/// oldest first: each its number (one more for every post), application and title.
fn notifications_json(session: &Session) -> String {
    let kept: Vec<String> = session
        .compositor
        .scene
        .kept_notifications()
        .into_iter()
        .map(|(number, application, title)| {
            format!(
                "{{\"number\":{number},\"application\":{},\"title\":{}}}",
                json_string(&application),
                json_string(&title)
            )
        })
        .collect();
    format!("[{}]", kept.join(","))
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

/// Each entry of the window's menus by its path in the bar (`View > Appearance > Dark`), in
/// order, with its chord, whether it is disabled and, for a checkable command, whether it is
/// checked; null when the window declared none (ADR 0021).
fn menus_json(compositor: &Compositor, window: Option<SurfaceKey>) -> String {
    let Some(menus) = window.and_then(|window| compositor.scene.menus(window)) else {
        return "null".to_string();
    };
    let lines: Vec<String> = menus
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.kind != oxyos_protocol::MenuKind::Separator)
        .map(|(index, entry)| {
            let checked = if entry.flags.contains(oxyos_protocol::MenuFlags::CHECKABLE) {
                entry
                    .flags
                    .contains(oxyos_protocol::MenuFlags::CHECKED)
                    .to_string()
            } else {
                "null".to_string()
            };
            format!(
                "{{\"path\":{},\"shortcut\":{},\"disabled\":{},\"checked\":{checked}}}",
                json_string(&menu_path(&menus.entries, index)),
                json_string(&entry.shortcut),
                entry.flags.contains(oxyos_protocol::MenuFlags::DISABLED),
            )
        })
        .collect();
    format!("[{}]", lines.join(","))
}

/// An entry's path in the bar: its menus' labels and its own, joined by ` > `.
fn menu_path(entries: &[oxyos_compositor_core::scene::MenuEntry], index: usize) -> String {
    let mut labels = Vec::new();
    let mut at = Some(index);
    while let Some(index) = at {
        let Some(entry) = entries.get(index) else {
            break;
        };
        labels.push(entry.label.as_str());
        at = (entry.parent != oxyos_protocol::NO_MENU_PARENT).then_some(entry.parent as usize);
    }
    labels.reverse();
    labels.join(" > ")
}

/// The companion application's window (ADR 0035), the toplevel of its client, if it has one.
fn companion_window(session: &Session) -> Option<SurfaceKey> {
    companion_frames(session)
        .into_iter()
        .map(|(key, _, _)| key)
        .find(|key| session.compositor.scene.window_number(*key).is_some())
}

/// Chooses the command at `path` of the active window's menus (the director's application's or
/// the companion's), as the shell's bar does.
fn choose_menu_item(compositor: &mut Compositor, path: &str) -> Result<()> {
    let key = compositor
        .scene
        .active_window()
        .ok_or("no window is active")?;
    let menus = compositor
        .scene
        .menus(key)
        .ok_or("the window declared no menus")?;
    let serial = menus.serial;
    let index = (0..menus.entries.len())
        .find(|&index| menu_path(&menus.entries, index) == path)
        .ok_or_else(|| format!("no {path:?} in the window's menus"))?;
    if !compositor
        .scene
        .choose_active_menu_item(serial, index as u32)
    {
        return Err(format!("{path:?} could not be chosen").into());
    }
    Ok(())
}

/// The client's image the pointer shows once a step settled (ADR 0029): its size and hotspot in
/// its pixels, the scale it was drawn for and its middle pixel as `#aarrggbb`; null for a shape.
fn pointer_image_json(compositor: &Compositor) -> String {
    let Some(pointer) = compositor.scene.pointer_image().and_then(|(key, id)| {
        compositor
            .client_pointers
            .get(&key)
            .filter(|pointer| pointer.id == id)
    }) else {
        return "null".to_string();
    };
    let image = &pointer.image;
    let middle = image.pixels[((image.height / 2) * image.width + image.width / 2) as usize];
    format!(
        "{{\"width\":{},\"height\":{},\"hotspot\":[{},{}],\"scale_120\":{},\"middle\":\"#{middle:08x}\"}}",
        image.width, image.height, image.hotspot.0, image.hotspot.1, pointer.scale_120
    )
}

/// What the clipboard holds once a step settled, as the session reads it from its owner, each
/// type through a pipe as any receiver pastes: `{"types": [...], "contents": {type: content}}`,
/// or null when it holds nothing. Read again only when it changed.
fn clipboard_json(session: &mut Session) -> Result<String> {
    let Some((selection, types)) = session
        .compositor
        .scene
        .clipboard()
        .map(|(selection, types)| (selection, types.to_vec()))
    else {
        return Ok("null".to_string());
    };
    if let Some(json) = session
        .pasteboard
        .as_ref()
        .and_then(|pasteboard| pasteboard.reported(selection))
    {
        return Ok(json.to_string());
    }
    let mut contents = Vec::new();
    for mime in &types {
        let reading = crate::pasteboard::Pasteboard::read(&mut session.compositor, *mime)?;
        let bytes = match reading {
            Some(reading) => {
                while !reading.done() {
                    session.compositor.serve(&session.listener)?;
                    if let Some(pasteboard) = session.pasteboard.as_mut() {
                        pasteboard.serve(&mut session.compositor);
                    }
                }
                reading.take()
            }
            None => None,
        };
        contents.push(format!(
            "{}:{}",
            json_string(mime.as_str()),
            crate::pasteboard::content_json(bytes.as_deref())
        ));
    }
    let names: Vec<String> = types
        .iter()
        .map(|mime| json_string(mime.as_str()))
        .collect();
    let json = format!(
        "{{\"types\":[{}],\"contents\":{{{}}}}}",
        names.join(","),
        contents.join(",")
    );
    if let Some(pasteboard) = session.pasteboard.as_mut() {
        pasteboard.remember(selection, json.clone());
    }
    Ok(json)
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

/// `step` with a `Step::Named` resolved to the output's step at the middle of its node: the node
/// named so on the topmost shown surface that has one, as the assistive technology holds it (its
/// rectangle in global coordinates, ADR 0003). Any other step as it is.
fn named_step(session: &Session, step: &Step) -> Result<Step> {
    if let Step::DragPacedNamed {
        from,
        to,
        motions,
        hz,
        escape_after,
        rest_ms,
        held,
    } = step
    {
        let at = |name: &str| {
            // `@x,y`: a point of the output, not a node.
            if let Some((x, y)) = name
                .strip_prefix('@')
                .and_then(|point| point.split_once(','))
            {
                if let (Ok(x), Ok(y)) = (x.trim().parse(), y.trim().parse()) {
                    return Ok((x, y));
                }
            }
            named_point(session, name)
                .ok_or_else(|| format!("the assistive technology holds no node named {name:?}"))
        };
        return Ok(Step::DragPaced {
            from: at(from)?,
            to: at(to)?,
            motions: *motions,
            hz: *hz,
            escape_after: *escape_after,
            rest_ms: *rest_ms,
            held: *held,
        });
    }
    let Step::Named { kind, name, ms } = step else {
        return Ok(step.clone());
    };
    let (x, y) = named_point(session, name)
        .ok_or_else(|| format!("the assistive technology holds no node named {name:?}"))?;
    Ok(match kind {
        NamedKind::Press => Step::PressOutput { x, y },
        NamedKind::PressSecondary => Step::PressSecondaryOutput { x, y },
        NamedKind::DoublePress => Step::DoublePressOutput { x, y },
        NamedKind::Move => Step::MoveOutput { x, y },
        NamedKind::Hold => Step::HoldOutput { x, y, ms: *ms },
        NamedKind::Scroll { dy, events } => Step::ScrollOutput {
            x,
            y,
            dy: *dy,
            events: *events,
        },
    })
}

/// The middle of the node named `name` on the topmost shown surface that has one, as the
/// assistive technology holds it (global coordinates), if any.
fn named_point(session: &Session, name: &str) -> Option<(i32, i32)> {
    session
        .compositor
        .scene
        .render_list()
        .into_iter()
        .rev()
        .find_map(|item| match item {
            RenderItem::Surface { key, .. } => {
                let node = session
                    .reader
                    .trees
                    .get(&key)?
                    .iter()
                    .find(|node| node.name == name)?;
                let (x, y, width, height) = node.rect;
                Some((x + (width / 2) as i32, y + (height / 2) as i32))
            }
            _ => None,
        })
}

/// The name a step's report gives a surface: the application's by its id, the session's
/// shell's as `shell-<id>`, any other client's as `<client>-<id>`.
fn surface_name(key: SurfaceKey) -> String {
    if key.client == CLIENT {
        key.id.to_string()
    } else if key.client > ClientId::MAX / 2 && key.client < crate::pasteboard::PASTEBOARD {
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
/// {surface: [{role, name, value, state, actions, custom, selection, parent, rect}]}: `parent` the
/// index of the node it is nested in (null at the top), `rect` [x, y, width, height] on its surface.
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
                    let parent = node
                        .parent
                        .map_or("null".to_string(), |index| index.to_string());
                    let position = node
                        .position
                        .map_or("null".to_string(), |(at, count)| format!("[{at},{count}]"));
                    let (x, y, width, height) = node.rect;
                    format!(
                        "{{\"role\":\"{:?}\",\"name\":{},\"value\":{},\"hint\":{},\"position\":{position},\"state\":[{}],\"actions\":[{}],\"custom\":[{}],\"selection\":{selection},\"parent\":{parent},\"rect\":[{x},{y},{width},{height}]}}",
                        node.role,
                        json_string(&node.name),
                        json_string(&node.value),
                        json_string(&node.hint),
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
            // When its clock ends it, and when the frame that ended it came (ms since it began).
            let ms = |ms: Option<u32>| ms.map_or("null".to_string(), |ms| ms.to_string());
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
                AppearanceChange::Returned { .. } => "returned",
                AppearanceChange::Vanished { .. } => "vanished",
            };
            // Where a drag's image went back to (its centre), for a scenario to check it is the
            // item's own place.
            let to = match seen.change {
                AppearanceChange::Returned { x, y } => format!(",\"to\":[{x},{y}]"),
                // Where it puffed away (its centre): where it was dropped.
                AppearanceChange::Vanished { x, y } => format!(",\"at\":[{x},{y}]"),
                _ => String::new(),
            };
            let path: Vec<String> = seen
                .path
                .iter()
                .map(|step| format!("[{},{},{}]", step.ms, step.opacity, step.moved_permille))
                .collect();
            format!(
                "{{\"kind\":\"{}\",\"change\":\"{change}\"{to},\"arriving\":{},\"first\":{},\"last\":{},\"frames\":{},\"ended\":{},\"ends_ms\":{},\"ended_ms\":{},\"path\":[{}]}}",
                format!("{:?}", seen.kind).to_lowercase(),
                seen.arriving(),
                opacity(seen.first),
                opacity(seen.last),
                seen.frames,
                seen.ended,
                ms(seen.ends_ms),
                ms(seen.ended_ms),
                path.join(",")
            )
        })
        .collect();
    let frames = std::mem::take(&mut compositor.transition_frames);
    format!("{{\"frames\":{frames},\"seen\":[{}]}}", seen.join(","))
}

/// Every drag icon surface (ADR 0035), each with where it is shown (null while hidden).
fn drag_icons_json(compositor: &Compositor) -> String {
    let icons: Vec<String> = compositor
        .scene
        .drag_icons()
        .into_iter()
        .map(|(key, rect)| {
            format!(
                "{{\"client\":{},\"rect\":{}}}",
                key.client,
                rect.map_or("null".to_string(), |r| format!(
                    "[{},{},{},{}]",
                    r.x, r.y, r.width, r.height
                ))
            )
        })
        .collect();
    format!("[{}]", icons.join(","))
}

/// The windows the shell's surfaces show live once the step settled (ADR 0022), each by its number:
/// where it is drawn (fitted, the global space's logical pixels), its clip, and whether it is
/// moved there (ADR 0045); every window drawn where it is (`windows_placed`), by number; and each
/// window that flew to its view or back in the step (`flights`), each frame's rectangle. Taken: the
/// next step reports its own flights.
fn views_json(compositor: &mut Compositor) -> String {
    let rect = |r: Rect| format!("[{},{},{},{}]", r.x, r.y, r.width, r.height);
    let number = |compositor: &Compositor, key: SurfaceKey| {
        compositor
            .scene
            .window_number(key)
            .map_or("null".to_string(), |n| n.to_string())
    };
    let list = compositor.scene.render_list();
    let views: Vec<String> = list
        .iter()
        .filter_map(|item| match *item {
            RenderItem::View {
                window,
                rect: at,
                clip,
                moved,
                ..
            } => Some(format!(
                "{{\"window\":{},\"rect\":{},\"clip\":{},\"moved\":{moved}}}",
                number(compositor, window),
                rect(at),
                rect(clip)
            )),
            _ => None,
        })
        .collect();
    let placed: Vec<String> = list
        .iter()
        .filter_map(|item| match *item {
            RenderItem::Surface { key, rect: at, .. } => {
                let n = compositor.scene.window_number(key)?;
                Some(format!("\"{n}\":{}", rect(at)))
            }
            _ => None,
        })
        .collect();
    let flights: Vec<String> = compositor
        .take_flights()
        .into_iter()
        .map(|flight| {
            let path: Vec<String> = flight.path.iter().map(|at| rect(*at)).collect();
            format!(
                "{{\"window\":{},\"arriving\":{},\"path\":[{}]}}",
                number(compositor, flight.window),
                flight.arriving,
                path.join(",")
            )
        })
        .collect();
    // Where the frame composed last drew each window (ADR 0046): in its view, on its way there or
    // back, or slid with its desktop, with its desktop's number.
    let mut drawn: Vec<(u32, String)> = compositor
        .windows_drawn
        .iter()
        .filter_map(|(&key, &at)| {
            let n = compositor.scene.window_number(key)?;
            let desktop = compositor.scene.window_workspace(key)?;
            let home = compositor.scene.global_rect(key)?;
            Some((
                n,
                format!(
                    "\"{n}\":{{\"rect\":{},\"home\":{},\"workspace\":{desktop}}}",
                    rect(at),
                    rect(home)
                ),
            ))
        })
        .collect();
    drawn.sort();
    let (count, active) = compositor.scene.workspaces();
    format!(
        ",\"window_views\":[{}],\"windows_placed\":{{{}}},\"flights\":[{}],\"windows_drawn\":{{{}}},\"workspaces\":[{count},{active}]",
        views.join(","),
        placed.join(","),
        flights.join(","),
        drawn.into_iter().map(|(_, json)| json).collect::<Vec<_>>().join(",")
    )
}

/// The menus open in each frame composed since the last step, as [[client, surface, x, y, width,
/// height], ...] per frame (`Compositor::menu_frames`). Taken: the next step reports its own.
fn menu_frames_json(compositor: &mut Compositor) -> String {
    let frames: Vec<String> = std::mem::take(&mut compositor.menu_frames)
        .into_iter()
        .map(|menus| {
            let menus: Vec<String> = menus
                .iter()
                .map(|(key, r)| {
                    format!(
                        "[{},{},{},{},{},{}]",
                        key.client, key.id, r.x, r.y, r.width, r.height
                    )
                })
                .collect();
            format!("[{}]", menus.join(","))
        })
        .collect();
    format!("[{}]", frames.join(","))
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
        Step::MoveOutput { x, y } => {
            on_outputs(*x, *y)?;
            let (px, py) = at(*x, *y);
            devices.move_to(px, py)?;
            compositor.read_input();
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
        Step::PressSecondaryOutput { x, y } => {
            on_outputs(*x, *y)?;
            let (px, py) = at(*x, *y);
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
        Step::DoublePressOutput { x, y } => {
            on_outputs(*x, *y)?;
            let point = at(*x, *y);
            press(devices, compositor, point)?;
            compositor.read_input();
            press(devices, compositor, point)?;
        }
        Step::TriplePress { x, y } => {
            inside(window, *x, *y)?;
            let point = at(window.x + x, window.y + y);
            for _ in 0..3 {
                press(devices, compositor, point)?;
                compositor.read_input();
            }
        }
        Step::DoubleDrag { from, to } => {
            inside(window, from.0, from.1)?;
            inside(window, to.0, to.1)?;
            let (fx, fy) = at(window.x + from.0, window.y + from.1);
            let (tx, ty) = at(window.x + to.0, window.y + to.1);
            press(devices, compositor, (fx, fy))?;
            compositor.read_input();
            devices.button(true)?;
            compositor.read_input();
            devices.move_to((fx + tx) / 2, (fy + ty) / 2)?;
            compositor.read_input();
            devices.move_to(tx, ty)?;
            compositor.read_input();
            devices.button(false)?;
        }
        Step::Scroll { x, y, dy, events } => {
            inside(window, *x, *y)?;
            let (px, py) = at(window.x + x, window.y + y);
            devices.move_to(px, py)?;
            compositor.read_input();
            scroll_through(devices, compositor, *dy, *events)?;
        }
        Step::ScrollOutput { x, y, dy, events } => {
            on_outputs(*x, *y)?;
            let (px, py) = at(*x, *y);
            devices.move_to(px, py)?;
            compositor.read_input();
            scroll_through(devices, compositor, *dy, *events)?;
        }
        Step::Pinch {
            x,
            y,
            percent,
            degrees,
            fingers,
        } => {
            inside(window, *x, *y)?;
            let (px, py) = at(window.x + x, window.y + y);
            devices.move_to(px, py)?;
            compositor.read_input();
            devices.pinch(*fingers, *percent, *degrees, || compositor.read_input())?;
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
        Step::Strokes(chords) => {
            for codes in chords {
                for code in codes {
                    devices.key(*code as u16, true)?;
                    compositor.read_input();
                }
                for code in codes.iter().rev() {
                    devices.key(*code as u16, false)?;
                    compositor.read_input();
                }
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

/// A scroll on the scene where the pointer is: a wheel's notch by `dy` with `events` 0, else a
/// touchpad's two fingers by `dy` in `events` even steps, then lifted.
fn scroll_scene(scene: &mut Scene, dy: i32, events: u32) {
    if events == 0 {
        scene.pointer_scroll(0, dy.saturating_mul(256), ScrollSource::Wheel, false);
        return;
    }
    let each = dy.saturating_mul(256) / events as i32;
    for _ in 0..events {
        scene.pointer_scroll(0, each, ScrollSource::Finger, false);
    }
    scene.pointer_scroll(0, 0, ScrollSource::Finger, true);
}

/// A scroll through the virtual devices where the pointer is: the wheel turned with `events` 0,
/// else two fingers moved on the touchpad in `events` frames, then lifted; the compositor reading
/// each notch and frame as it is sent.
fn scroll_through(
    devices: &mut crate::uinput::Devices,
    compositor: &mut Compositor,
    dy: i32,
    events: u32,
) -> Result<()> {
    if events == 0 {
        devices.wheel(dy, || compositor.read_input())?;
    } else {
        devices.two_fingers(dy, events, || compositor.read_input())?;
    }
    Ok(())
}

/// The preferences file as the step left it (`--preferences`), as a JSON string; null without one
/// or before the session first wrote it.
fn preferences_file_json(file: Option<&Path>) -> String {
    file.and_then(|file| fs::read_to_string(file).ok())
        .map_or("null".to_string(), |text| {
            serde_json::Value::String(text).to_string()
        })
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
    (motions, hz): (u32, u32),
    escape_after: Option<u32>,
    rest_ms: u32,
    held: bool,
) -> Result<String> {
    inside_output(&session.compositor, from)?;
    inside_output(&session.compositor, to)?;
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
        // Resting at the end first, with Escape after the rest.
        if escape_after == Some(i as u32) && !(i == steps && rest_ms > 0) {
            session.compositor.scene.key(KEY_ESCAPE, true);
            session.compositor.scene.key(KEY_ESCAPE, false);
            session.compositor.serve(&session.listener)?;
        }
    }
    session.compositor.idle_wait = idle_wait;
    // The hand resting at `to` (a file held over the dock's Trash), then the Escape after its last
    // motion.
    let rested = interval * motions + Duration::from_millis(u64::from(rest_ms));
    while start.elapsed() < rested {
        session.compositor.serve(&session.listener)?;
        observe(&session.compositor, &mut sent, &mut frames);
    }
    if rest_ms > 0 && escape_after == Some(motions) {
        session.compositor.scene.key(KEY_ESCAPE, true);
        session.compositor.scene.key(KEY_ESCAPE, false);
        session.compositor.serve(&session.listener)?;
    }
    let due = rested + interval;
    while start.elapsed() < due {
        session.compositor.serve(&session.listener)?;
        observe(&session.compositor, &mut sent, &mut frames);
    }
    session
        .compositor
        .scene
        .set_input_time(paced_time(origin, due));
    // Held, the drag goes on into the steps after this one.
    if !held {
        session.compositor.scene.pointer_button(BUTTON, false);
    }
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

/// A paced swipe (`Step::Swipe`, ADR 0046) through the compositor's input, as libinput's swipe
/// gesture reaches it: `fingers` put down (none: they are down since an earlier step), each of
/// `steps` even motions by (dx, dy) when its time comes (`hz` a second) with its device's time, the
/// compositor serving everything until then and the turn the motion wakes composing at once, as a
/// touchpad's wakes a session's; the fingers lifted `lift` milliseconds after their last motion
/// (none: they stay down into the next steps).
fn swipe_paced(
    session: &mut Session,
    fingers: Option<u8>,
    (dx, dy): (i32, i32),
    (steps, hz): (u32, u32),
    lift: Option<u32>,
) -> Result<()> {
    let start = Instant::now();
    let origin = monotonic_ns();
    let idle_wait = session.compositor.idle_wait;
    let held = SWIPING.get().map(|(n, _)| n);
    let fingers_down = fingers.or(held).ok_or("no fingers down to swipe")?;
    let swipe = |phase, dx: f64, dy: f64| oxyos_input::Action::Swipe {
        phase,
        fingers: u32::from(fingers_down),
        dx,
        dy,
    };
    let act = |session: &mut Session, due: Duration, action| -> Result<()> {
        while start.elapsed() < due {
            session.compositor.idle_wait = idle_wait.min(due.saturating_sub(start.elapsed()));
            session.compositor.serve(&session.listener)?;
        }
        session
            .compositor
            .scene
            .set_input_time(paced_time(origin, due));
        session.compositor.apply_input(action);
        session.compositor.idle_wait = Duration::ZERO;
        session.compositor.serve(&session.listener)?;
        Ok(())
    };
    if fingers.is_some() {
        act(
            session,
            Duration::ZERO,
            swipe(oxyos_input::GesturePhase::Begin, 0.0, 0.0),
        )?;
        SWIPING.set(Some((fingers_down, 0)));
    }
    let interval = Duration::from_secs(1) / hz.max(1);
    for i in 1..=steps {
        act(
            session,
            interval * i,
            swipe(
                oxyos_input::GesturePhase::Update,
                f64::from(dx) / f64::from(steps),
                f64::from(dy) / f64::from(steps),
            ),
        )?;
    }
    if let Some(ms) = lift {
        act(
            session,
            interval * steps + Duration::from_millis(u64::from(ms)),
            swipe(oxyos_input::GesturePhase::End, 0.0, 0.0),
        )?;
        SWIPING.set(None);
    }
    session.compositor.idle_wait = idle_wait;
    Ok(())
}

/// A paced scroll (`Step::ScrollPaced`) on the scene: the pointer over the window at `at`, each
/// event when its time comes with the compositor serving the application until then and the turn
/// the event wakes composing at once, as a touchpad's wakes a session's; then the compositor
/// serving it until its window shows no new frame for SETTLE (or five seconds pass). Returns what
/// the window did, as JSON: `events`; `presented`, the frames it showed meanwhile; and
/// `latency_ms` (median and max), from each event to the first frame shown after it. The events
/// are a touchpad's; with `lift`, its stop follows the last one, a frame of the hand later, as
/// the fingers lift; with `hold`, two fingers come to rest that many milliseconds after the lift
/// (libinput's hold begin, each when its time comes), and lift unmoved 150 ms later (its end).
fn scroll_paced(
    session: &mut Session,
    window: Rect,
    at: (i32, i32),
    dy: i32,
    (events, hz): (u32, u32),
    lift: bool,
    hold: Option<u32>,
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
        session
            .compositor
            .scene
            .pointer_scroll(0, step as i32, ScrollSource::Finger, false);
        session.compositor.idle_wait = Duration::ZERO;
        session.compositor.serve(&session.listener)?;
        observe(&session.compositor, &mut shown);
    }
    if lift {
        let due = interval * (events + 1);
        while start.elapsed() < due {
            session.compositor.idle_wait = idle_wait.min(due.saturating_sub(start.elapsed()));
            session.compositor.serve(&session.listener)?;
            observe(&session.compositor, &mut shown);
        }
        session
            .compositor
            .scene
            .set_input_time(paced_time(origin, due));
        session
            .compositor
            .scene
            .pointer_scroll(0, 0, ScrollSource::Finger, true);
        session.compositor.idle_wait = Duration::ZERO;
        session.compositor.serve(&session.listener)?;
        observe(&session.compositor, &mut shown);
        if let Some(after) = hold {
            let lifted = due;
            for (phase, offset) in [
                (GesturePhase::Begin, Duration::from_millis(after.into())),
                (
                    GesturePhase::End,
                    Duration::from_millis(u64::from(after) + 150),
                ),
            ] {
                let due = lifted + offset;
                while start.elapsed() < due {
                    session.compositor.idle_wait =
                        idle_wait.min(due.saturating_sub(start.elapsed()));
                    session.compositor.serve(&session.listener)?;
                    observe(&session.compositor, &mut shown);
                }
                session
                    .compositor
                    .scene
                    .set_input_time(paced_time(origin, due));
                session.compositor.scene.pointer_hold(phase, 2);
                session.compositor.idle_wait = Duration::ZERO;
                session.compositor.serve(&session.listener)?;
                observe(&session.compositor, &mut shown);
            }
        }
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

/// A long press on the output (`Step::HoldOutput`): pressed, held still for `ms` while the
/// compositor serves the clients (the shell times the hold itself), then released.
fn hold_output(session: &mut Session, at: (i32, i32), ms: u32) -> Result<()> {
    let outputs: Vec<Rect> = session
        .compositor
        .screens
        .iter()
        .map(|screen| screen.rect)
        .collect();
    on_outputs(&outputs, at.0, at.1)?;
    session.compositor.scene.pointer_motion(at.0, at.1);
    session.compositor.scene.pointer_button(BUTTON, true);
    let until = Instant::now() + Duration::from_millis(u64::from(ms));
    let idle_wait = session.compositor.idle_wait;
    while Instant::now() < until {
        session.compositor.idle_wait =
            idle_wait.min(until.saturating_duration_since(Instant::now()));
        session.compositor.serve(&session.listener)?;
        session.serve_launches();
        session.serve_preferences();
    }
    session.compositor.idle_wait = idle_wait;
    session
        .compositor
        .scene
        .set_input_time(monotonic_ns() as u64);
    session.compositor.scene.pointer_button(BUTTON, false);
    Ok(())
}

/// Paced presses (`Step::PressPaced`): each press and its release when its time comes, carrying
/// that time, with the compositor serving the application until then.
fn press_paced(
    session: &mut Session,
    window: Rect,
    at: (i32, i32),
    count: u32,
    interval_ms: u32,
) -> Result<()> {
    if !Rect::new(0, 0, window.width, window.height).contains_point(at.0, at.1) {
        return Err(format!("({}, {}) is outside the window {window:?}", at.0, at.1).into());
    }
    let start = Instant::now();
    let origin = monotonic_ns();
    let idle_wait = session.compositor.idle_wait;
    for i in 0..count {
        let due = Duration::from_millis(u64::from(interval_ms) * u64::from(i));
        while start.elapsed() < due {
            session.compositor.idle_wait = idle_wait.min(due.saturating_sub(start.elapsed()));
            session.compositor.serve(&session.listener)?;
        }
        session
            .compositor
            .scene
            .set_input_time(paced_time(origin, due));
        session
            .compositor
            .scene
            .pointer_motion(window.x + at.0, window.y + at.1);
        session.compositor.scene.pointer_button(BUTTON, true);
        session.compositor.scene.pointer_button(BUTTON, false);
        session.compositor.idle_wait = Duration::ZERO;
        session.compositor.serve(&session.listener)?;
    }
    session.compositor.idle_wait = idle_wait;
    Ok(())
}

/// A hand's double click on a title area (`Step::DoubleClick`): the first press held while the
/// compositor serves the application until it is moving the window (two seconds at most), as a
/// hand's press is held while a drag region answers it, then released; the second pressed `gap_ms`
/// after, held `hold_ms` and released.
fn double_click(
    session: &mut Session,
    window: Rect,
    at: (i32, i32),
    hold_ms: u32,
    gap_ms: u32,
) -> Result<()> {
    if !Rect::new(0, 0, window.width, window.height).contains_point(at.0, at.1) {
        return Err(format!("({}, {}) is outside the window {window:?}", at.0, at.1).into());
    }
    let idle_wait = session.compositor.idle_wait;
    let serve_for = |session: &mut Session, ms: u32| -> Result<()> {
        let until = Instant::now() + Duration::from_millis(u64::from(ms));
        while Instant::now() < until {
            session.compositor.idle_wait =
                idle_wait.min(until.saturating_duration_since(Instant::now()));
            session.compositor.serve(&session.listener)?;
        }
        Ok(())
    };
    session
        .compositor
        .scene
        .pointer_motion(window.x + at.0, window.y + at.1);
    for press in 0..2 {
        if press == 1 {
            serve_for(session, gap_ms)?;
        }
        session.compositor.scene.pointer_button(BUTTON, true);
        if press == 0 {
            let deadline = Instant::now() + Duration::from_secs(2);
            while session.compositor.scene.moving_window().is_none() && Instant::now() < deadline {
                serve_for(session, 2)?;
            }
        } else {
            serve_for(session, hold_ms)?;
        }
        session.compositor.scene.pointer_button(BUTTON, false);
    }
    session.compositor.idle_wait = Duration::ZERO;
    session.compositor.serve(&session.listener)?;
    session.compositor.idle_wait = idle_wait;
    Ok(())
}

/// A point on one of the session's outputs (a scaled one's logical pixels).
fn inside_output(compositor: &Compositor, (x, y): (i32, i32)) -> Result<()> {
    if compositor
        .screens
        .iter()
        .any(|screen| screen.rect.contains_point(x, y))
    {
        Ok(())
    } else {
        Err(format!("({x}, {y}) is outside every output").into())
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
        Step::MoveOutput { x, y } => {
            on_outputs(*x, *y)?;
            compositor.scene.pointer_motion(*x, *y);
        }
        Step::ReleaseOutput { x, y } => {
            on_outputs(*x, *y)?;
            compositor.scene.pointer_motion(*x, *y);
            compositor.scene.pointer_button(BUTTON, false);
        }
        Step::PressSecondary { x, y } => {
            inside(window, *x, *y)?;
            compositor.scene.pointer_motion(window.x + x, window.y + y);
            compositor.scene.pointer_button(SECONDARY_BUTTON, true);
            compositor.scene.pointer_button(SECONDARY_BUTTON, false);
        }
        Step::PressSecondaryOutput { x, y } => {
            on_outputs(*x, *y)?;
            compositor.scene.pointer_motion(*x, *y);
            compositor.scene.pointer_button(SECONDARY_BUTTON, true);
            compositor.scene.pointer_button(SECONDARY_BUTTON, false);
        }
        Step::DoublePress { x, y } => {
            inside(window, *x, *y)?;
            press(&mut compositor.scene, window.x + x, window.y + y);
            press(&mut compositor.scene, window.x + x, window.y + y);
        }
        Step::DoublePressOutput { x, y } => {
            on_outputs(*x, *y)?;
            press(&mut compositor.scene, *x, *y);
            press(&mut compositor.scene, *x, *y);
        }
        Step::TriplePress { x, y } => {
            inside(window, *x, *y)?;
            for _ in 0..3 {
                press(&mut compositor.scene, window.x + x, window.y + y);
            }
        }
        Step::DoubleDrag { from, to } => {
            inside(window, from.0, from.1)?;
            inside(window, to.0, to.1)?;
            let scene = &mut compositor.scene;
            press(scene, window.x + from.0, window.y + from.1);
            scene.pointer_button(BUTTON, true);
            scene.pointer_motion(
                window.x + (from.0 + to.0) / 2,
                window.y + (from.1 + to.1) / 2,
            );
            scene.pointer_motion(window.x + to.0, window.y + to.1);
            scene.pointer_button(BUTTON, false);
        }
        Step::ScrollOutput { x, y, dy, events } => {
            on_outputs(*x, *y)?;
            compositor.scene.pointer_motion(*x, *y);
            scroll_scene(&mut compositor.scene, *dy, *events);
        }
        Step::Scroll { x, y, dy, events } => {
            inside(window, *x, *y)?;
            compositor.scene.pointer_motion(window.x + x, window.y + y);
            scroll_scene(&mut compositor.scene, *dy, *events);
        }
        Step::Pinch {
            x,
            y,
            percent,
            degrees,
            fingers,
        } => {
            inside(window, *x, *y)?;
            if *percent <= 0 {
                return Err("a pinch spreads to more than 0%".into());
            }
            let fingers = *fingers;
            let scene = &mut compositor.scene;
            scene.pointer_motion(window.x + x, window.y + y);
            scene.pointer_pinch(GesturePhase::Begin, fingers, 65536, 0);
            // libinput's scale and turn are since the pinch began: the spread changes by an even
            // factor at each step, as fingers moving steadily apart do.
            const STEPS: i32 = 8;
            for i in 1..=STEPS {
                let done = f64::from(i) / f64::from(STEPS);
                let scale = (f64::from(*percent) / 100.0).powf(done);
                scene.pointer_pinch(
                    GesturePhase::Update,
                    fingers,
                    (scale * 65536.0).round() as u32,
                    (f64::from(*degrees) * done * 256.0).round() as i32,
                );
            }
            scene.pointer_pinch(
                GesturePhase::End,
                fingers,
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
        // Typed through the session's keymap, which the director holds (`Session::keyboard`).
        Step::Strokes(_) => return Err("keys on the keymap are typed by the session".into()),
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
        Step::TransitionClock(clock) => {
            compositor.clock = match clock {
                TransitionClock::Hold => {
                    let now = oxyos_compositor::monotonic();
                    HELD.set(Some(now));
                    Some(now)
                }
                TransitionClock::At(ms) => Some(
                    HELD.get().ok_or("--transition-clock MS before hold")?
                        + Duration::from_millis(*ms),
                ),
                TransitionClock::Run => None,
            }
        }
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
        | Step::WaitNamed { .. }
        | Step::Clipboard(_)
        | Step::Network(_)
        | Step::Sound(_)
        | Step::Bluetooth(_)
        | Step::Power(_)
        | Step::Menu(_)
        | Step::Keyboard { .. }
        | Step::ImChange { .. }
        | Step::Accessibility { .. }
        | Step::Grab { .. }
        | Step::DragPaced { .. }
        | Step::Swipe { .. }
        | Step::ScrollPaced { .. }
        | Step::MovePaced { .. }
        | Step::HoldOutput { .. }
        | Step::DoubleClick { .. }
        | Step::PressPaced { .. } => {}
        // The step loop runs it through the devices; there is no scene equivalent of a terminal.
        Step::SwitchTerminal(_) => return Err("--switch-terminal needs --uinput and --kms".into()),
        // Resolved to the output's step before it is run (`named_step`).
        Step::Named { .. } | Step::DragPacedNamed { .. } => {
            return Err("a named step is resolved before it is run".into())
        }
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
    if let Some(address) = launch
        .network_bus
        .as_ref()
        .or(launch.bluetooth_bus.as_ref())
    {
        // D-Bus's runtime (zbus's, on async-io) keeps a reactor for the life of the process (its
        // poller, its wake and its timer): started now, a connection made and gone, it is the
        // process's before the session counts its descriptors, as the session's own would be.
        drop(zbus::blocking::connection::Builder::address(address.as_str())?.build()?);
    }
    if let Some(address) = launch.power_bus.as_ref().or(launch.network_bus.as_ref()) {
        // D-Bus's runtime (zbus's, on async-io) keeps a reactor for the life of the process (its
        // poller, its wake and its timer): started now, a connection made and gone, it is the
        // process's before the session counts its descriptors, as the session's own would be.
        drop(zbus::blocking::connection::Builder::address(address.as_str())?.build()?);
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
    // Every input event routed to a client from before the first connects, for regress.py to
    // check against what each host says it received (`deliveries` in the summary).
    session.compositor.delivery_log = Some(Vec::new());
    // The preferences before any client connects, so each is told them with its Welcome, and
    // the session's locale (ADR 0040): the person's language and region over this program's
    // environment's (`LC_ALL`, `LC_MESSAGES`, `LANG`); a login screen's is the environment's alone.
    let settings = oxyos_compositor::settings::Settings::from_file(launch.preferences.clone());
    session.settings = if launch.login.is_some() {
        settings.system_locale_only()
    } else {
        settings
    };
    // The system's sounds (docs/adr/0044-system-sounds.mdx), recorded as the session would play
    // them, never played: a step says which it asked for (`sounds`).
    session.compositor.sounds = Some(oxyos_compositor::sounds::Sounds::new(Box::new(
        session.sounds.clone(),
    )));
    session
        .settings
        .apply(&mut session.compositor, oxyos_compositor::settings::now());
    if let Some(file) = &launch.displays {
        session.follow_displays(file)?;
    }
    session.grant_settings = launch.grant_settings.clone();
    // Safe Mode, told to the shell and the login screen as they connect.
    if launch.safe_mode {
        session.compositor.scene.set_safe_mode();
    }
    session.chooser_command = launch.file_chooser.clone();
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
    if launch.trash {
        let trash = oxyos_files::Scope::for_user().and_then(oxyos_compositor::trash::Trash::new)?;
        let waker = trash.wake_fd()?;
        session.trash_waker = Some(std::os::fd::AsRawFd::as_raw_fd(&waker));
        session.compositor.wakers.push(waker);
        session.trash = Some(trash);
    }
    session.installed_fixture = launch.installed_fixture.clone();
    // Settings › Storage (docs/adr/0062-storage.mdx), on the run's home and a played disk.
    if let Some(file) = &launch.storage {
        let text = std::fs::read_to_string(file)?;
        let disk: oxyos_compositor::storage::PlayedDisk =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", file.display()))?;
        let mut storage = oxyos_compositor::storage::Storage::new(
            oxyos_files::Scope::for_user()?,
            std::sync::Arc::new(disk),
            launch
                .installd
                .clone()
                .unwrap_or_else(|| PathBuf::from("/nonexistent/oxyos-installd.socket")),
        )?;
        storage.set_locale(&session.settings.locale());
        storage.list_applications(oxyos_apps::Listing::read(session.installed_fixture.clone()));
        let waker = storage.wake_fd()?;
        session.storage_waker = Some(std::os::fd::AsRawFd::as_raw_fd(&waker));
        session.compositor.wakers.push(waker);
        session.storage = Some(storage);
    }
    // Search (docs/adr/0051-search.mdx): the index as the session runs it, this program's child
    // (`--search-service`), started at the first ask rather than with the run; told the
    // applications the shell lists, the fixture's installed ones among them.
    {
        let mut command = Command::new(std::env::current_exe()?);
        command.arg("--search-service");
        let mut search = oxyos_compositor::search::Search::new(command, false)?;
        search.set_locale(&session.settings.locale());
        search.list_applications(oxyos_apps::Listing::read(session.installed_fixture.clone()));
        let waker = search.wake_fd()?;
        session.search_waker = Some(std::os::fd::AsRawFd::as_raw_fd(&waker));
        session.compositor.wakers.push(waker);
        session.search = Some(search);
    }
    if let Some(folder) = &launch.screenshots {
        // Written as the session writes them, into the run's own folders; the shutter recorded
        // with the system's other sounds (`sounds`).
        let (desktop, documents) = (folder.join("Desktop"), folder.join("Documents"));
        session.compositor.screenshots = Some(oxyos_compositor::screenshots::Writer::spawn(
            Box::new(move |destination| match destination {
                oxyos_protocol::CaptureDestination::Desktop => desktop.clone(),
                oxyos_protocol::CaptureDestination::Documents => documents.clone(),
            }),
            oxyos_compositor::screenshots::Language::of(&[session.settings.locale()]),
        )?);
    }
    if let Some(application) = &launch.passkey_fixture {
        // Passkeys' sheets as the session shows them, the accounts' service the fixture's.
        let service = std::rc::Rc::new(std::cell::RefCell::new(
            crate::passkeys::FixtureService::new(application.clone()),
        ));
        session.passkeys = Some(oxyos_compositor::passkeys::Passkeys::new(Box::new(
            crate::passkeys::Shared(service.clone()),
        )));
        session.passkey_service = Some(service);
    }
    if let Some(application) = &launch.autofill_fixture {
        // AutoFill as the session serves it, its keychain the fixture's.
        let keychain = std::rc::Rc::new(std::cell::RefCell::new(
            crate::autofill::FixtureKeychain::new(application.clone()),
        ));
        let mut autofill = oxyos_compositor::autofill::Autofill::new(Box::new(
            crate::autofill::Shared(keychain.clone()),
        ));
        if launch.autofill_saves {
            let socket = std::env::var_os("OXYOS_ACCOUNTD_SESSION_SOCKET")
                .map(std::path::PathBuf::from)
                .ok_or("--autofill-saves asks the service at OXYOS_ACCOUNTD_SESSION_SOCKET")?;
            let saves = oxyos_account::keychain::SaveConfirmations::spawn(socket, |said| {
                eprintln!("[offscreen] saving passwords: {said}")
            })?;
            let waker = saves.wake_fd()?;
            session.autofill_saves_waker = Some(std::os::fd::AsRawFd::as_raw_fd(&waker));
            session.compositor.wakers.push(waker);
            autofill = autofill.with_saves(Box::new(crate::autofill::Saves(saves)));
        }
        session.autofill = Some(autofill);
        session.autofill_keychain = Some(keychain);
    }
    if let Some(methods) = &launch.input_sources_available {
        // The state played or the engine run, never both telling it.
        if launch.input_methods.is_some() {
            return Err(
                "--input-sources-available plays what --input-methods runs: give one".into(),
            );
        }
        session.input_sources = Some(crate::input_sources::InputSources::new(
            methods,
            &launch.layout,
        )?);
    }
    if let Some(address) = &launch.network_bus {
        // The session's networks, over the played daemon on the run's own bus, Wi-Fi Diagnostics
        // probing the played network behind it.
        let control = zbus::blocking::connection::Builder::address(address.as_str())?.build()?;
        let networks = oxyos_compositor::networks::Networks::on_bus_probed(
            Some(address.clone()),
            std::sync::Arc::new(crate::played_network::PlayedProbe {
                bus: control.clone(),
            }),
        )?;
        let waker = networks.wake_fd()?;
        session.networks_waker = Some(std::os::fd::AsRawFd::as_raw_fd(&waker));
        session.compositor.wakers.push(waker);
        session.networks = Some(networks);
        session.network_control = Some(control);
    }
    if let Some(address) = &launch.bluetooth_bus {
        // The session's Bluetooth and its pairing agent, over the played BlueZ on the run's own
        // bus.
        let bluetooth = oxyos_compositor::bluetooth::Bluetooth::on_bus(Some(address.clone()))?;
        let waker = bluetooth.wake_fd()?;
        session.bluetooth_waker = Some(std::os::fd::AsRawFd::as_raw_fd(&waker));
        session.compositor.wakers.push(waker);
        session.bluetooth = Some(bluetooth);
        session.bluetooth_control =
            Some(zbus::blocking::connection::Builder::address(address.as_str())?.build()?);
    }
    if let Some(prefix) = &launch.input_methods {
        // The session's input methods (ADR 0041): the real engine, in a directory of this run's.
        let engine = oxyos_input_methods::Engine::at(prefix)
            .ok_or_else(|| format!("--input-methods: no engine at {}", prefix.display()))?;
        // Under the session's own directory, removed with it: nothing learned outlives the run.
        let run = session.directory().join("input-methods");
        let paths = oxyos_input_methods::Paths {
            runtime: run.join("run"),
            data: run.join("data"),
        };
        let methods = oxyos_compositor::input_methods::start(
            &mut session.compositor,
            &session.settings,
            engine,
            paths,
        )
        .ok_or("--input-methods: the engine did not start")?;
        let mut methods = methods;
        // The scenario's first key reaches the engine, not the layout it falls back to.
        if !methods.wait_ready(&mut session.compositor, Duration::from_secs(30)) {
            return Err("--input-methods: the engine did not start in 30 s".into());
        }
        session.input_methods_waker = session
            .compositor
            .wakers
            .last()
            .map(std::os::fd::AsRawFd::as_raw_fd);
        session.input_methods = Some(methods);
    }
    // The sound played (ADR 0056), acted on in the turn itself, as the controls' machine.
    if let Some(file) = &launch.sound {
        let played = std::sync::Arc::new(oxyos_audio::Played::parse(&fs::read_to_string(file)?)?);
        session.sound = Some(oxyos_compositor::sound_devices::SoundDevices::new(
            played.clone(),
        ));
        session.played_sound = Some(played);
    }
    if let Some(address) = &launch.power_bus {
        // The played power services' control: a step changes the machine and its result says
        // what they hold.
        session.power_control =
            Some(zbus::blocking::connection::Builder::address(address.as_str())?.build()?);
    }
    if let Some(file) = &launch.controls {
        // A played machine, acted on in the turn itself, so a scenario is the same every run.
        let mut fake = oxyos_controls::Fake::from_file(file.clone())?;
        if let Some(played) = &session.played_sound {
            fake = fake.with_sound(played.clone());
        }
        if let Some(address) = &launch.network_bus {
            fake = fake.with_network_bus(address)?;
        }
        if let Some(address) = &launch.power_bus {
            fake = fake.with_power_bus(address)?;
        }
        session.compositor.controls = Some(oxyos_compositor::controls::Controls::new(
            oxyos_controls::Worker::inline(Box::new(fake.clone())),
        ));
        session.controls = Some(fake);
    }
    if let Some(address) = &launch.power_bus {
        // The machine's power services, played on the run's own bus, and the session's side of
        // battery and energy over them (ADR 0057); its history in the session's own directory.
        if session.compositor.controls.is_none() {
            let fake = oxyos_controls::Fake::parse("")?.with_power_bus(address)?;
            session.compositor.controls = Some(oxyos_compositor::controls::Controls::new(
                oxyos_controls::Worker::inline(Box::new(fake.clone())),
            ));
            session.controls = Some(fake);
        }
        let power = oxyos_compositor::power::Power::new(
            Some(address.clone()),
            Some(session.directory().join("battery-history")),
        )?;
        let waker = power.wake_fd()?;
        session.power_waker = Some(std::os::fd::AsRawFd::as_raw_fd(&waker));
        session.compositor.wakers.push(waker);
        session.power = Some(power);
    }
    if launch.input_method {
        let mut ime = crate::ime::Ime::connect(&mut session.compositor)?;
        ime.candidates = launch.im_candidates;
        session.ime = Some(ime);
    } else if launch.im_candidates.is_some() {
        return Err("--im-candidates needs --input-method".into());
    }
    session
        .compositor
        .scene
        .set_keyboard_layout(&oxyos_input::Layout::named(&launch.layout).names())
        .map_err(|code| format!("layout {}: {code:?}", launch.layout))?;
    session.pasteboard = Some(crate::pasteboard::Pasteboard::connect(
        &mut session.compositor,
    )?);
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
            .with_installed(None)
            // What `--grant` gives, told to the applications it starts as the policy's would be.
            .with_fixture_grants(launch.grants.clone())
            // Where each says why it ends, if it ends for a fault (docs/adr/0061), in the run's
            // own folder.
            .with_crash_reports(session.directory().join("reports"))?,
        );
        // What an erasure a crash interrupted left aside, deleted as a session starts.
        if let Err(error) = oxyos_apps::data::sweep(&oxyos_apps::data::bases()) {
            eprintln!("[launch] what resets left is not all deleted: {error}");
        }
        if let Some(root) = &launch.installed_fixture {
            let user = oxyos_apps::session_user().unwrap_or_default();
            session.launcher = session.launcher.take().map(|launcher| {
                launcher.with_installed_fixture(oxyos_apps::installed_applications(root, &user))
            });
        }
        // The applications' login items and background activity
        // (docs/adr/0060-login-items-and-background-activity.mdx), their installed applications'
        // decisions read beside the fixture's entries, as the session reads the service's.
        session.background = Some(oxyos_compositor::background::BackgroundActivity::new(
            launch.installed_fixture.as_ref().map(|root| {
                oxyos_apps::installed_applications(
                    root,
                    &oxyos_apps::session_user().unwrap_or_default(),
                )
            }),
        ));
        // The applications the person opens at login (docs/adr/0034-the-dock.mdx), as a session
        // starts them; none in Safe Mode (docs/adr/0054-recovery-and-erasing.mdx, decision 10).
        let open_at_login = session.settings.preferences().open_at_login.clone();
        if launch.safe_mode {
            eprintln!("[launch] Safe Mode: nothing opened at login");
            if let Some(background) = session.background.as_mut() {
                background.set_safe_mode();
            }
        } else if let Some(launcher) = session.launcher.as_mut() {
            for id in &open_at_login {
                if let Err(error) = launcher.launch(id) {
                    eprintln!("[launch] {id} not opened at login: {error}");
                }
            }
        }
    }
    if let Some(password) = &launch.lock_password {
        session.compositor.authentication = Some(Box::new(
            crate::lock::Fixture::new(password.clone())?.with_pin(launch.lock_pin.clone()),
        ));
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
        // With `--uinput`, the run's own virtual devices and no other: another run's, made alike,
        // or the machine's would feed its steps.
        let mut nodes = Vec::new();
        if launch.uinput {
            let mut made = crate::uinput::Devices::new(bounds, &launch.layout)?;
            nodes = made.nodes()?;
            devices = Some(made);
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
        let (input, read) = if launch.uinput {
            let paths: Vec<_> = nodes.iter().map(PathBuf::as_path).collect();
            let named: Vec<_> = nodes
                .iter()
                .map(|node| node.display().to_string())
                .collect();
            (
                oxyos_input::Input::open_paths(bounds, &layout, &paths, devices)?,
                format!("the run's own devices ({})", named.join(", ")),
            )
        } else {
            (
                oxyos_input::Input::open_with(bounds, &layout, devices)?,
                "seat0".to_string(),
            )
        };
        eprintln!(
            "[input] libinput on {read}, layout {}, {}",
            launch.layout,
            if input.composes() {
                "composing with the locale's table"
            } else {
                "no compose table: dead keys type nothing"
            }
        );
        session.compositor.set_input(input);
    }
    if let Some(dir) = &launch.dump_frames {
        fs::create_dir_all(dir)?;
        session.compositor.keep_frame = true;
    }
    if launch.gpu {
        // On a display, its card's own GPU (its buffers are scanned out there); else the GPU that
        // drives the displays (ADR 0031, decision 14).
        let card_gpu = session.compositor.screens[0]
            .display
            .as_ref()
            .and_then(|display| display.card().render_node());
        let compositor = match card_gpu {
            Some(device) => oxyos_gpu::VulkanCompositor::on_device(device)?,
            None => oxyos_gpu::VulkanCompositor::new()?,
        };
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
    // Clients' GPU buffers where a render node has timelines (docs/adr/0031-gpu-buffers.mdx).
    session.compositor.offer_gpu_buffers();
    // The session's shell first, as a session starts it: the application's window is then placed
    // in the work area its panels leave.
    let shell = match &launch.shell {
        Some(command) => Some(start_shell(&mut session, command)?),
        None => None,
    };
    // The screen reader before the application, as a session that runs one starts it; off, only
    // its socket, for Super+F5 to start it on.
    let speech = out.join("speech.jsonl");
    let reader_socket = match &launch.reader {
        Some(_) => Some(listen_for_reader(&mut session, &speech)?),
        None => None,
    };
    let reader_named = launch
        .reader
        .as_deref()
        .zip(reader_socket.as_deref())
        .map(|(program, socket)| (socket, program, launch.reader_locale.as_deref()));
    // Held apart from the other children: Super+F5 may stop it, and start it again.
    let mut screen_reader = Children(Vec::new());
    if let Some((socket, program, locale)) = reader_named.filter(|_| !launch.reader_off) {
        screen_reader.0.push(start_reader(
            &mut session,
            socket,
            program,
            locale,
            &speech,
        )?);
    }
    // A login screen has no application: its shell is the first child, which must not exit.
    let mut children = Children(Vec::new());
    if let Some(program) = launch.command.first() {
        let mut application = Command::new(program);
        application
            .args(&launch.command[1..])
            .env("OXYOS_COMPOSITOR_SOCKET", &session.socket)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        // It lists the fixture's installed applications too, as the shell does and as a session's
        // applications list the install service's (ADR 0030, decision 7): Settings names them.
        if let Some(root) = &session.installed_fixture {
            application.env("OXYOS_INSTALLED_FIXTURE", root);
        }
        children.0.push(application.spawn()?);
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
    let deadline = Instant::now() + LAUNCH_DEADLINE;

    let launched_ns = monotonic_ns();
    // A login screen has no window: the output stands for it, and its first frame is the one
    // its shell has already shown.
    let login = launch.command.is_empty();
    let primary = session.compositor.screens[0].rect;
    let first = match settle(
        &mut session,
        &mut children,
        &[],
        login,
        Wait {
            deadline,
            waiting_for: "the first frame",
            still: launch.launch_settle,
            sheet: false,
            named: None,
        },
        login.then_some(primary),
    ) {
        Ok(first) => first,
        Err(error) => {
            // What the output showed when the launch gave up: what never stopped changing.
            let _ = snapshot(&session.compositor, out, "launch-unsettled.png");
            return Err(error);
        }
    };
    let mut window = first.window;
    if let Some(command) = &launch.companion {
        start_companion(
            &mut session,
            &mut children,
            command,
            launch.companion_at,
            launch.companion_app.as_deref(),
        )?;
    }
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
    for (index, (step, settling)) in launch.steps.iter().enumerate() {
        // A step aimed at a node by its name, at where that node is now.
        session.reader.absorb(&mut session.compositor);
        let resolved = named_step(&session, step)?;
        let step = &resolved;
        let may_stay = &settling.may_stay;
        session.compositor.input_log.clear();
        session.compositor.text_input_log.clear();
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
            ime.chosen.clear();
            ime.turned.clear();
        }
        if let Some(sources) = session.input_sources.as_mut() {
            sources.selected.clear();
            sources.emoji = 0;
        }
        let typed_as_text = match devices.as_mut() {
            _ if matches!(step, Step::Keyboard { .. } | Step::ImChange { .. }) => {
                inject_input_method(&mut session, step)?;
                0
            }
            _ if matches!(step, Step::Wait(_) | Step::WaitNamed { .. }) => {
                // A wait for a node ends as soon as it is held; a plain wait, its time.
                let name = match step {
                    Step::WaitNamed { name, .. } => Some(name.as_str()),
                    _ => None,
                };
                let ms = match step {
                    Step::Wait(ms) | Step::WaitNamed { ms, .. } => *ms,
                    _ => 0,
                };
                let until = Instant::now() + Duration::from_millis(ms);
                loop {
                    if name.is_some_and(|name| named_point(&session, name).is_some()) {
                        break;
                    }
                    if Instant::now() >= until {
                        if let Some(name) = name {
                            return Err(format!(
                                "step {} ({}): the assistive technology held no node named {name:?} in {ms} ms",
                                index + 1,
                                step.describe()
                            )
                            .into());
                        }
                        break;
                    }
                    session.compositor.serve(&session.listener)?;
                    if let Some(pasteboard) = session.pasteboard.as_mut() {
                        pasteboard.serve(&mut session.compositor);
                    }
                    session.serve_launches();
                    session.serve_installs();
                    session.serve_background();
                    session.serve_preferences();
                    session.reader.absorb(&mut session.compositor);
                }
                0
            }
            _ if matches!(step, Step::Menu(_)) => {
                if let Step::Menu(path) = step {
                    choose_menu_item(&mut session.compositor, path)?;
                }
                0
            }
            _ if matches!(step, Step::Network(_)) => {
                if let Step::Network(what) = step {
                    session.network_act(what)?;
                }
                0
            }
            _ if matches!(step, Step::Sound(_)) => {
                if let Step::Sound(what) = step {
                    session
                        .played_sound
                        .as_ref()
                        .ok_or("a sound step needs --sound")?
                        .act(what)?;
                }
                0
            }
            _ if matches!(step, Step::Bluetooth(_)) => {
                if let Step::Bluetooth(what) = step {
                    session.bluetooth_act(what)?;
                }
                0
            }
            _ if matches!(step, Step::Power(_)) => {
                if let Step::Power(what) = step {
                    session.power_act(what)?;
                }
                0
            }
            _ if matches!(step, Step::Clipboard(_)) => {
                if let (Step::Clipboard(contents), Some(pasteboard)) =
                    (step, session.pasteboard.as_mut())
                {
                    pasteboard.copy(&mut session.compositor, contents.clone())?;
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
                    escape_after,
                    rest_ms,
                    held,
                } = step
                {
                    dragged = Some(drag_paced(
                        &mut session,
                        *from,
                        *to,
                        (*motions, *hz),
                        *escape_after,
                        *rest_ms,
                        *held,
                    )?);
                }
                0
            }
            _ if matches!(step, Step::ScrollPaced { .. }) => {
                if let Step::ScrollPaced {
                    at,
                    dy,
                    events,
                    hz,
                    lift,
                    hold_after_ms,
                } = step
                {
                    scrolled = Some(scroll_paced(
                        &mut session,
                        window,
                        *at,
                        *dy,
                        (*events, *hz),
                        *lift,
                        *hold_after_ms,
                    )?);
                }
                0
            }
            // A whole swipe through the virtual touchpad (`--uinput`): its fingers moving together,
            // which libinput recognizes and counts, as a person's.
            Some(devices)
                if matches!(
                    step,
                    Step::Swipe {
                        fingers: Some(_),
                        lift: Some(0),
                        ..
                    }
                ) =>
            {
                if let Step::Swipe {
                    fingers: Some(fingers),
                    dx,
                    dy,
                    steps,
                    hz,
                    ..
                } = step
                {
                    let compositor = &mut session.compositor;
                    devices.swipe(*fingers, (*dx, *dy), *steps, *hz, || {
                        compositor.read_input()
                    })?;
                }
                0
            }
            _ if matches!(step, Step::Swipe { .. }) => {
                if let Step::Swipe {
                    fingers,
                    dx,
                    dy,
                    steps,
                    hz,
                    lift,
                } = step
                {
                    swipe_paced(&mut session, *fingers, (*dx, *dy), (*steps, *hz), *lift)?;
                }
                0
            }
            _ if matches!(step, Step::HoldOutput { .. }) => {
                if let Step::HoldOutput { x, y, ms } = step {
                    hold_output(&mut session, (*x, *y), *ms)?;
                }
                0
            }
            _ if matches!(step, Step::DoubleClick { .. }) => {
                if let Step::DoubleClick {
                    at,
                    hold_ms,
                    gap_ms,
                } = step
                {
                    double_click(&mut session, window, *at, *hold_ms, *gap_ms)?;
                }
                0
            }
            _ if matches!(step, Step::PressPaced { .. }) => {
                if let Step::PressPaced {
                    at,
                    count,
                    interval_ms,
                } = step
                {
                    press_paced(&mut session, window, *at, *count, *interval_ms)?;
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
            None if matches!(step, Step::Strokes(_)) => {
                if let Step::Strokes(chords) = step {
                    let keyboard = session.keyboard.get_or_insert_with(|| {
                        oxyos_input::Keyboard::new(&oxyos_input::Layout::named(&launch.layout))
                    });
                    let keyboard = keyboard
                        .as_mut()
                        .map_err(|error| format!("the {} keymap: {error}", launch.layout))?;
                    // Dead keys are the engine's while an input method is current (ADR 0041).
                    keyboard.set_composing(
                        session
                            .input_methods
                            .as_ref()
                            .is_none_or(|methods| methods.composing()),
                    );
                    for codes in chords {
                        let presses = codes.iter().map(|code| (*code, true));
                        let releases = codes.iter().rev().map(|code| (*code, false));
                        for (code, pressed) in presses.chain(releases) {
                            for action in keyboard.actions(code, pressed) {
                                session.compositor.apply_input(action);
                            }
                        }
                    }
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
        // Super+F5 typed: the reader started or stopped before the step settles, as the session
        // switches it on its next turn.
        toggle_reader(&mut session, &mut screen_reader, reader_named, &speech)?;
        let what = format!("an answer to step {} ({})", index + 1, step.describe());
        let answer = settle(
            &mut session,
            &mut children,
            &before,
            // A swipe answers as it is paced, the compositor drawing the windows as the fingers
            // go: what settles is that nothing moves any more.
            *may_stay || matches!(step, Step::Swipe { .. }),
            Wait {
                deadline: Instant::now()
                    + settling
                        .until_named
                        .as_ref()
                        .map_or(STEP_DEADLINE, |(ms, _)| {
                            STEP_DEADLINE.max(Duration::from_millis(*ms))
                        }),
                waiting_for: &what,
                still: SETTLE,
                sheet: settling.sheet,
                named: settling.until_named.as_ref().map(|(_, name)| name.as_str()),
            },
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
        let clipboard = clipboard_json(&mut session)?;
        let screenshots = screenshots_json(&mut session);
        let urls = urls_json(&mut session);
        let signed_out = std::mem::take(&mut session.signed_out);
        let background = background_json(&mut session);
        let notifications = notifications_json(&session);
        let files_opened = files_opened_json(&mut session);
        let sounds = sounds_json(&session);
        let transactions = transactions_json(&session);
        answered.push(format!(
            "{{{},\"transactions\":{transactions},\"screenshots\":{screenshots},\"urls_opened\":{urls},\"files_opened\":{files_opened},\"signed_out\":{signed_out},\"background\":{background},\"notifications\":{notifications},\"sounds\":{sounds},\"changed_pixels\":{changed},\"t_inject\":{injected_ns},\"t_settled\":{},\"latency_ms\":{latency},\"flip_latency_ms\":{flip_latency},\"focus\":{},\"input_focus\":{},\"delivered\":{},\"popups\":{},\"window\":[{},{},{},{}],\"a11y\":{},\"announcements\":{announcements},\"typed_as_text\":{typed_as_text},\"field\":{},\"im\":{},\"input_sources\":{},\"autofill\":{},\"passkeys\":{},\"shell\":{},\"window_shown\":{},\"minimize_target\":{},\"window_states\":{},\"pointer_shape\":\"{:?}\",\"pointer_image\":{},\"transitions\":{},\"controls\":{},\"network\":{},\"sound\":{},\"bluetooth\":{},\"power\":{},\"displays_off\":{},\"preferences_file\":{},\"clipboard\":{clipboard},\"menus\":{},\"companion_menus\":{},\"chooser\":{},\"composed\":{}{},\"menu_frames\":{}{}{}{},\"drag_icons\":{},\"gpu_frames\":{{\"latched\":{},\"waited\":{}}}}}",
            step.json(),
            answer.settled_ns,
            focus_json(&session.compositor),
            input_focus_json(&session.compositor),
            delivered_json(&session),
            popups_json(&session.compositor),
            answer.window.x,
            answer.window.y,
            answer.window.width,
            answer.window.height,
            accessibility_json(&session.reader),
            field_json(&session.compositor),
            session.ime.as_ref().map_or("null".to_string(), |ime| ime.json()),
            session
                .input_sources
                .as_ref()
                .map_or("null".to_string(), |sources| sources.json()),
            autofill_json(&session),
            passkeys_json(&session),
            shell_json(&session.compositor),
            session.compositor.surface_rect(CLIENT, WINDOW).is_some(),
            minimize_target_json(&session.compositor),
            window_states_json(&session.compositor),
            session.compositor.scene.pointer_shape(),
            pointer_image_json(&session.compositor),
            transitions_json(&mut session.compositor),
            session.controls.as_ref().map_or("null".to_string(), controls_json),
            session.network_state(),
            session
                .played_sound
                .as_ref()
                .map_or("null".to_string(), |played| played.machine().to_string()),
            session.bluetooth_state(),
            session.power_state(),
            session.compositor.displays_off(),
            preferences_file_json(launch.preferences.as_deref()),
            menus_json(
                &session.compositor,
                Some(SurfaceKey {
                    client: CLIENT,
                    id: WINDOW,
                })
            ),
            menus_json(&session.compositor, companion_window(&session)),
            chooser_json(&session.compositor),
            composed_since(&session.compositor, injected_ns),
            views_json(&mut session.compositor),
            menu_frames_json(&mut session.compositor),
            dragged.map_or(String::new(), |drag| format!(",\"drag\":{drag}")),
            scrolled.map_or(String::new(), |scroll| format!(",\"scrolled\":{scroll}")),
            moved.map_or(String::new(), |motion| format!(",\"moved\":{motion}")),
            drag_icons_json(&session.compositor),
            // Frames of the application's GPU buffers so far (docs/adr/0031-gpu-buffers.mdx).
            session.compositor.gpu_frames(CLIENT).0,
            session.compositor.gpu_frames(CLIENT).1,
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
    // Nor the file choosers': their processes and the mount table their answers were checked by.
    session.end_choosers();
    // The assistive technology leaves with the session, like every other client.
    session.compositor.scene.disconnect(READER);
    if session.ime.take().is_some() {
        session.compositor.scene.disconnect(crate::ime::IME);
    }
    if session.pasteboard.take().is_some() {
        session
            .compositor
            .scene
            .disconnect(crate::pasteboard::PASTEBOARD);
        session.compositor.local_pipes.clear();
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
    // The reader, if on, ends with the session as the other clients do.
    children.0.append(&mut screen_reader.0);
    // What was routed until the session ends; finishing it ends the clients, whose last input
    // has been read by then.
    let deliveries = deliveries_json(&session.compositor);
    session.finish(&mut children)?;
    drop(seat);
    println!(
        "{{\"mode\":\"offscreen-cpu\",\"outputs\":[{}],\"drm\":{display},\"gpu\":{gpu},\"launched\":{:?},\"t_launch\":{launched_ns},\"first_frame_ms\":{:.1},\"window\":[{},{},{},{}],\"frames\":{frames},\"damaged_fraction_median\":{damaged:.3},\"frames_whole\":{whole},\"steps\":[{}],\"applications\":[{}],\"deliveries\":{deliveries},\"console_input_bytes\":{console_input},\"descriptors_settled\":true,\"snapshots\":{:?},\"status\":\"passed\"}}",
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
            "--triple-press",
            "7,8",
            "--double-drag",
            "1,2,30,4",
            "--drag-output",
            "10,20,90,100",
            "--scale",
            "180",
            "--idle",
            "300",
            "--wait-named",
            "45000",
            "Try again",
            "--move-window",
            "-10,5",
            "--resize-window",
            "800,600",
            "--expect-popup",
            "--allow-unchanged",
            "--until-sheet",
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
        let allowed: Vec<bool> = launch.steps.iter().map(|(_, s)| s.may_stay).collect();
        // `--until-sheet`, as `--allow-unchanged`, is the step's after it alone.
        let sheets = launch.steps.iter().filter(|(_, s)| s.sheet).count();
        assert_eq!(
            (sheets, launch.steps.last().map(|(_, s)| s.sheet)),
            (1, Some(true))
        );
        assert_eq!(
            allowed,
            [
                false, false, false, false, false, false, false, false, false, false, false, false,
                false, false, false, false, false, false, false, false, false, false, true
            ]
        );
        assert_eq!(
            steps,
            [
                Step::Press { x: 1, y: 2 },
                Step::Scroll {
                    x: 3,
                    y: 4,
                    dy: -50,
                    events: 0
                },
                Step::Pinch {
                    x: 5,
                    y: 6,
                    percent: 150,
                    degrees: -30,
                    fingers: 2,
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
                Step::TriplePress { x: 7, y: 8 },
                Step::DoubleDrag {
                    from: (1, 2),
                    to: (30, 4)
                },
                Step::DragOutput {
                    from: (10, 20),
                    to: (90, 100)
                },
                Step::Scale(180),
                Step::Idle(300),
                Step::WaitNamed {
                    name: "Try again".into(),
                    ms: 45000
                },
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
    fn until_named_is_the_step_after_it_alone() {
        let (_, launch) = Launch::parse(&args(&[
            "--key",
            "15",
            "--until-named",
            "120000",
            "Open Inbox",
            "--key",
            "57",
            "--key",
            "15",
            "--",
            "app",
        ]))
        .unwrap();
        let named: Vec<_> = launch
            .steps
            .iter()
            .map(|(_, s)| s.until_named.clone())
            .collect();
        assert_eq!(
            named,
            [None, Some((120000, "Open Inbox".to_string())), None]
        );
    }

    #[test]
    fn malformed_steps_are_rejected() {
        for bad in [
            &["--press", "1", "--", "app"][..],
            &["--scroll", "1,2", "--", "app"],
            &["--key", "x", "--", "app"],
            &["--key", "42+", "--", "app"],
            &["--drag", "1,2,3", "--", "app"],
            &["--wait-named", "soon", "Try again", "--", "app"],
            &["--until-named", "soon", "Open", "--key", "57", "--", "app"],
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
