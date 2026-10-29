//! C ABI over `oxyos-gestures`, react-native-gesture-handler's recognizers in the Bloom host. The
//! host's `RNGestureHandlerModule` passes JavaScript's handlers, settings and relations through,
//! feeds the pointer stream with the views under each new pointer, and reads back what to send
//! JavaScript, one output at a time.

use super::fail;
use oxyos_gestures::{
    Action, Data, Frame, Orchestrator, Output, Pointer, PointerType, Refused, Relations, State,
    Touch, Value,
};
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_void};

/// Pointer actions (`oxyos_gestures_touch`), Android's.
pub const OXYOS_GESTURE_DOWN: u8 = 0;
pub const OXYOS_GESTURE_POINTER_DOWN: u8 = 1;
pub const OXYOS_GESTURE_MOVE: u8 = 2;
pub const OXYOS_GESTURE_POINTER_UP: u8 = 3;
pub const OXYOS_GESTURE_UP: u8 = 4;
pub const OXYOS_GESTURE_CANCEL: u8 = 5;
/// Setting values' kinds (`OxyOSGestureValue.kind`).
pub const OXYOS_GESTURE_NULL: u8 = 0;
pub const OXYOS_GESTURE_BOOL: u8 = 1;
pub const OXYOS_GESTURE_NUMBER: u8 = 2;
pub const OXYOS_GESTURE_TEXT: u8 = 3;
pub const OXYOS_GESTURE_LIST: u8 = 4;
/// Outputs' kinds (`OxyOSGestureOutput.kind`).
pub const OXYOS_GESTURE_STATE_CHANGE: u8 = 1;
pub const OXYOS_GESTURE_UPDATE: u8 = 2;
pub const OXYOS_GESTURE_INTERCEPT: u8 = 3;
pub const OXYOS_GESTURE_CANCEL_RESPONDER: u8 = 4;
/// Which of an output's data it has (`OxyOSGestureOutput.fields`).
pub const OXYOS_GESTURE_HAS_POSITION: u8 = 1;
pub const OXYOS_GESTURE_HAS_TRANSLATION: u8 = 2;
pub const OXYOS_GESTURE_HAS_VELOCITY: u8 = 4;
pub const OXYOS_GESTURE_HAS_DURATION: u8 = 8;
pub const OXYOS_GESTURE_HAS_POINTER_INSIDE: u8 = 16;

/// A setting: its key (`key_length` bytes of UTF-8) and value. A bool's or a number's is
/// `number`; a list's (a hit slop's) is `list`, entry i set when bit i of `list_set` is; a text's
/// content is not needed.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSGestureValue {
    pub key: *const c_char,
    pub key_length: usize,
    pub kind: u8,
    pub list_set: u8,
    pub number: f64,
    pub list: [f64; 6],
}

/// A pointer at a point of the root, in logical pixels.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyOSGesturePointer {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// A view's frame in the root.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyOSGestureFrame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Writes view `view`'s frame to `out` and returns 1, or returns 0 for a view no longer there.
pub type OxyOSGestureFrameFn = Option<
    unsafe extern "C" fn(context: *mut c_void, view: i32, out: *mut OxyOSGestureFrame) -> u8,
>;

/// What to send JavaScript. A state change has `state` and `old_state`, an update `state`
/// (ACTIVE); both have the handler's data, `fields` saying which. Intercept and cancel-responder
/// outputs have `tag` alone (none for intercept).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyOSGestureOutput {
    pub kind: u8,
    pub state: u8,
    pub old_state: u8,
    pub pointer_type: u8,
    pub fields: u8,
    pub pointer_inside: u8,
    pub tag: i32,
    pub view: i32,
    pub number_of_pointers: u32,
    pub x: f32,
    pub y: f32,
    pub absolute_x: f32,
    pub absolute_y: f32,
    pub translation_x: f32,
    pub translation_y: f32,
    pub velocity_x: f32,
    pub velocity_y: f32,
    pub duration: u64,
}

/// The orchestrator, and what it produced not yet read.
pub struct OxyOSGestures {
    orchestrator: Orchestrator,
    outputs: VecDeque<Output>,
}

fn refused(error: Refused) -> c_int {
    match error {
        Refused::Unsupported(name) => fail(format!("{name} is not available on OxyOS")),
        Refused::Exists(tag) => fail(format!("a gesture handler with tag {tag} exists")),
        Refused::Unknown(tag) => fail(format!("no gesture handler has tag {tag}")),
    }
}

/// # Safety
/// `gestures` is null or came from `oxyos_gestures_new` and is not freed.
#[allow(unsafe_code)]
unsafe fn gestures<'a>(gestures: *mut OxyOSGestures) -> Option<&'a mut OxyOSGestures> {
    // SAFETY: per the contract, a non-null pointer is a live, unaliased orchestrator.
    unsafe { gestures.as_mut() }
}

/// # Safety
/// `pointer` points to `count` valid items (any pointer when `count` is 0).
#[allow(unsafe_code)]
unsafe fn slice<'a, T>(pointer: *const T, count: usize) -> Option<&'a [T]> {
    if count == 0 {
        Some(&[])
    } else if pointer.is_null() {
        None
    } else {
        // SAFETY: per the contract.
        Some(unsafe { std::slice::from_raw_parts(pointer, count) })
    }
}

/// The settings as the orchestrator takes them, borrowing their keys.
///
/// # Safety
/// `config` points to `count` valid settings, each `key` to `key_length` bytes.
#[allow(unsafe_code)]
unsafe fn settings<'a>(
    config: *const OxyOSGestureValue,
    count: usize,
) -> Result<Vec<(&'a str, Value)>, c_int> {
    // SAFETY: per the contract.
    let given = unsafe { slice(config, count) }.ok_or_else(|| fail("null settings"))?;
    let mut out = Vec::with_capacity(given.len());
    for setting in given {
        // SAFETY: per the contract, `key` points to `key_length` bytes.
        let key = unsafe { slice(setting.key.cast::<u8>(), setting.key_length) }
            .ok_or_else(|| fail("null setting key"))?;
        let key = std::str::from_utf8(key).map_err(|_| fail("a setting key is not UTF-8"))?;
        let value = match setting.kind {
            OXYOS_GESTURE_NULL => Value::Null,
            OXYOS_GESTURE_BOOL => Value::Bool(setting.number != 0.0),
            OXYOS_GESTURE_NUMBER => Value::Number(setting.number),
            OXYOS_GESTURE_TEXT => Value::Text(String::new()),
            OXYOS_GESTURE_LIST => Value::List(
                (0..6)
                    .map(|i| (setting.list_set & (1 << i) != 0).then_some(setting.list[i]))
                    .collect(),
            ),
            _ => return Err(fail("unknown setting kind")),
        };
        out.push((key, value));
    }
    Ok(out)
}

/// Marks in `refused` (`count` bytes, or null) the settings OxyOS cannot honour.
///
/// # Safety
/// `marks` is null or points to `count` writable bytes.
#[allow(unsafe_code)]
unsafe fn mark(marks: *mut u8, config: &[(&str, Value)], refused: &[String]) {
    if marks.is_null() {
        return;
    }
    for (index, (key, _)) in config.iter().enumerate() {
        // SAFETY: per the contract, `marks` has an entry for each setting.
        unsafe { *marks.add(index) = u8::from(refused.iter().any(|r| r == key)) };
    }
}

/// A new orchestrator, with no handlers.
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_gestures_new() -> *mut OxyOSGestures {
    Box::into_raw(Box::new(OxyOSGestures {
        orchestrator: Orchestrator::new(),
        outputs: VecDeque::new(),
    }))
}

/// Frees an orchestrator.
///
/// # Safety
/// `gestures` is null or came from `oxyos_gestures_new` and is freed once.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gestures_free(gestures: *mut OxyOSGestures) {
    if !gestures.is_null() {
        // SAFETY: the pointer came from Box::into_raw in oxyos_gestures_new and is freed once.
        drop(unsafe { Box::from_raw(gestures) });
    }
}

/// `createGestureHandler`: the handler named `name` (`name_length` bytes), with tag `tag`, set to
/// `count` settings. Each entry of `refused` (`count` bytes, or null) is set to 1 for a setting
/// OxyOS cannot honour, 0 otherwise. 0, or OXYOS_ERROR (a handler not ported, a tag in use).
///
/// # Safety
/// `gestures` came from `oxyos_gestures_new`; `name` points to `name_length` bytes; `config` to
/// `count` settings; `refused` is null or points to `count` writable bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gestures_create(
    gestures: *mut OxyOSGestures,
    name: *const c_char,
    name_length: usize,
    tag: i32,
    config: *const OxyOSGestureValue,
    count: usize,
    refused_settings: *mut u8,
) -> c_int {
    // SAFETY: per the contract.
    let Some(g) = (unsafe { self::gestures(gestures) }) else {
        return fail("null gestures");
    };
    // SAFETY: per the contract, `name` points to `name_length` bytes.
    let Some(name) = (unsafe { slice(name.cast::<u8>(), name_length) }) else {
        return fail("null handler name");
    };
    let Ok(name) = std::str::from_utf8(name) else {
        return fail("the handler name is not UTF-8");
    };
    // SAFETY: per the contract.
    let config = match unsafe { settings(config, count) } {
        Ok(config) => config,
        Err(code) => return code,
    };
    match g.orchestrator.create(name, tag, &config) {
        Ok(refused) => {
            // SAFETY: per the contract.
            unsafe { mark(refused_settings, &config, &refused) };
            0
        }
        Err(error) => refused(error),
    }
}

/// `setGestureHandlerConfig` (`reset` 1) or `updateGestureHandlerConfig` (0), as
/// `oxyos_gestures_create` sets. 0, or OXYOS_ERROR for a tag no handler has.
///
/// # Safety
/// As `oxyos_gestures_create`'s.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gestures_configure(
    gestures: *mut OxyOSGestures,
    tag: i32,
    config: *const OxyOSGestureValue,
    count: usize,
    reset: u8,
    refused_settings: *mut u8,
) -> c_int {
    // SAFETY: per the contract.
    let Some(g) = (unsafe { self::gestures(gestures) }) else {
        return fail("null gestures");
    };
    // SAFETY: per the contract.
    let config = match unsafe { settings(config, count) } {
        Ok(config) => config,
        Err(code) => return code,
    };
    match g.orchestrator.configure(tag, &config, reset != 0) {
        Ok(refused) => {
            // SAFETY: per the contract.
            unsafe { mark(refused_settings, &config, &refused) };
            0
        }
        Err(error) => refused(error),
    }
}

/// `attachGestureHandler`: handler `tag` recognizes on view `view`. 0, or OXYOS_ERROR.
///
/// # Safety
/// `gestures` came from `oxyos_gestures_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gestures_attach(
    gestures: *mut OxyOSGestures,
    tag: i32,
    view: i32,
) -> c_int {
    // SAFETY: per the contract.
    let Some(g) = (unsafe { self::gestures(gestures) }) else {
        return fail("null gestures");
    };
    match g.orchestrator.attach(tag, view) {
        Ok(()) => 0,
        Err(error) => refused(error),
    }
}

/// `configureRelations`: the handlers `tag` waits for, recognizes with, and blocks. 0, or
/// OXYOS_ERROR.
///
/// # Safety
/// `gestures` came from `oxyos_gestures_new`; each list points to its count of tags.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gestures_relate(
    gestures: *mut OxyOSGestures,
    tag: i32,
    wait_for: *const i32,
    wait_for_count: usize,
    simultaneous: *const i32,
    simultaneous_count: usize,
    blocks: *const i32,
    blocks_count: usize,
) -> c_int {
    // SAFETY: per the contract.
    let Some(g) = (unsafe { self::gestures(gestures) }) else {
        return fail("null gestures");
    };
    // SAFETY: per the contract, each list points to its count of tags.
    let lists = unsafe {
        (
            slice(wait_for, wait_for_count),
            slice(simultaneous, simultaneous_count),
            slice(blocks, blocks_count),
        )
    };
    let (Some(wait_for), Some(simultaneous_with), Some(blocks)) = lists else {
        return fail("null relations");
    };
    let relations = Relations {
        wait_for: wait_for.to_vec(),
        simultaneous_with: simultaneous_with.to_vec(),
        blocks: blocks.to_vec(),
    };
    match g.orchestrator.relate(tag, relations) {
        Ok(()) => 0,
        Err(error) => refused(error),
    }
}

/// `dropGestureHandler`.
///
/// # Safety
/// `gestures` came from `oxyos_gestures_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gestures_drop(gestures: *mut OxyOSGestures, tag: i32) {
    // SAFETY: per the contract.
    if let Some(g) = unsafe { self::gestures(gestures) } {
        g.orchestrator.drop_handler(tag);
    }
}

/// JavaScript sets handler `tag`'s state (`_setGestureStateSync`, `_setGestureStateAsync`).
/// 0, or OXYOS_ERROR for a state that is not one.
///
/// # Safety
/// `gestures` came from `oxyos_gestures_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gestures_set_state(
    gestures: *mut OxyOSGestures,
    tag: i32,
    state: i32,
) -> c_int {
    // SAFETY: per the contract.
    let Some(g) = (unsafe { self::gestures(gestures) }) else {
        return fail("null gestures");
    };
    let Some(state) = State::from_number(state) else {
        return fail("not a gesture state");
    };
    let out = g.orchestrator.set_state(tag, state);
    g.outputs.extend(out);
    0
}

/// One event of the pointer stream: `action` about pointer `index` of the `count` pointers down
/// (root coordinates), at `time` milliseconds, of `pointer_type` (react-native-gesture-handler's:
/// 0 touch, 1 stylus, 2 mouse). For a pointer going down, `hits` are the views under it, deepest
/// first, then that view's ancestors to the root. `frame` gives a view's frame, called with
/// `context`. 0, or OXYOS_ERROR.
///
/// # Safety
/// `gestures` came from `oxyos_gestures_new`; `pointers` points to `count` pointers and `hits`
/// to `hit_count` tags; `frame`, called with `context`, writes a frame to its `out` when it
/// returns 1.
#[allow(unsafe_code)]
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gestures_touch(
    gestures: *mut OxyOSGestures,
    action: u8,
    index: usize,
    pointers: *const OxyOSGesturePointer,
    count: usize,
    time: u64,
    pointer_type: u8,
    hits: *const i32,
    hit_count: usize,
    frame: OxyOSGestureFrameFn,
    context: *mut c_void,
) -> c_int {
    // SAFETY: per the contract.
    let Some(g) = (unsafe { self::gestures(gestures) }) else {
        return fail("null gestures");
    };
    let Some(frame) = frame else {
        return fail("null frame function");
    };
    // SAFETY: per the contract.
    let (Some(pointers), Some(hits)) =
        (unsafe { (slice(pointers, count), slice(hits, hit_count)) })
    else {
        return fail("null pointers or hits");
    };
    let action = match action {
        OXYOS_GESTURE_DOWN => Action::Down,
        OXYOS_GESTURE_POINTER_DOWN => Action::PointerDown,
        OXYOS_GESTURE_MOVE => Action::Move,
        OXYOS_GESTURE_POINTER_UP => Action::PointerUp,
        OXYOS_GESTURE_UP => Action::Up,
        OXYOS_GESTURE_CANCEL => Action::Cancel,
        _ => return fail("unknown pointer action"),
    };
    if action != Action::Cancel && index >= pointers.len() {
        return fail("the pointer acted on is not down");
    }
    let pointer_type = match pointer_type {
        0 => PointerType::Touch,
        1 => PointerType::Stylus,
        2 => PointerType::Mouse,
        _ => return fail("unknown pointer type"),
    };
    let touch = Touch {
        action,
        index,
        pointers: pointers
            .iter()
            .map(|p| Pointer {
                id: p.id,
                x: p.x,
                y: p.y,
            })
            .collect(),
        time,
        pointer_type,
    };
    let lookup = |view: i32| {
        let mut out = OxyOSGestureFrame::default();
        // SAFETY: per the contract, `frame` writes `out` when it returns 1.
        let found = unsafe { frame(context, view, &mut out) } != 0;
        found.then_some(Frame {
            x: out.x,
            y: out.y,
            width: out.width,
            height: out.height,
        })
    };
    let out = g.orchestrator.touch(&touch, hits, &lookup);
    g.outputs.extend(out);
    0
}

/// Time passed to `now` milliseconds: handlers' timers due fire.
///
/// # Safety
/// `gestures` came from `oxyos_gestures_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gestures_tick(gestures: *mut OxyOSGestures, now: u64) {
    // SAFETY: per the contract.
    if let Some(g) = unsafe { self::gestures(gestures) } {
        let out = g.orchestrator.tick(now);
        g.outputs.extend(out);
    }
}

/// When the next timer fires, in milliseconds, or -1 for none.
///
/// # Safety
/// `gestures` came from `oxyos_gestures_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gestures_next_deadline(gestures: *mut OxyOSGestures) -> i64 {
    // SAFETY: per the contract.
    unsafe { self::gestures(gestures) }
        .and_then(|g| g.orchestrator.next_deadline())
        .map_or(-1, |deadline| deadline.min(i64::MAX as u64) as i64)
}

/// 1 if a handler attached to `view`, whose frame is `frame`, reaches the root point (x, y) with
/// its hit slop: the host's hit testing counts the view as under it.
///
/// # Safety
/// `gestures` came from `oxyos_gestures_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gestures_reaches(
    gestures: *mut OxyOSGestures,
    view: i32,
    frame: OxyOSGestureFrame,
    x: f32,
    y: f32,
) -> u8 {
    let frame = Frame {
        x: frame.x,
        y: frame.y,
        width: frame.width,
        height: frame.height,
    };
    // SAFETY: per the contract.
    unsafe { self::gestures(gestures) }
        .is_some_and(|g| g.orchestrator.reaches(view, frame, x, y))
        .into()
}

/// Takes the next output: 1 with `*out` filled, or 0 when there is none.
///
/// # Safety
/// `gestures` came from `oxyos_gestures_new`; `out` points to a writable output.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_gestures_output(
    gestures: *mut OxyOSGestures,
    out: *mut OxyOSGestureOutput,
) -> c_int {
    // SAFETY: per the contract.
    let Some(g) = (unsafe { self::gestures(gestures) }) else {
        return fail("null gestures");
    };
    if out.is_null() {
        return fail("null output");
    }
    let Some(output) = g.outputs.pop_front() else {
        return 0;
    };
    let mut converted = OxyOSGestureOutput::default();
    match output {
        Output::StateChange {
            tag,
            view,
            state,
            old_state,
            data,
        } => {
            converted.kind = OXYOS_GESTURE_STATE_CHANGE;
            converted.state = state as u8;
            converted.old_state = old_state as u8;
            converted.tag = tag;
            converted.view = view;
            fill(&mut converted, &data);
        }
        Output::Update { tag, view, data } => {
            converted.kind = OXYOS_GESTURE_UPDATE;
            converted.state = State::Active as u8;
            converted.tag = tag;
            converted.view = view;
            fill(&mut converted, &data);
        }
        Output::Intercept => converted.kind = OXYOS_GESTURE_INTERCEPT,
        Output::CancelResponder { tag } => {
            converted.kind = OXYOS_GESTURE_CANCEL_RESPONDER;
            converted.tag = tag;
        }
    }
    // SAFETY: per the contract, `out` is writable.
    unsafe { *out = converted };
    1
}

fn fill(out: &mut OxyOSGestureOutput, data: &Data) {
    out.number_of_pointers = data.number_of_pointers;
    out.pointer_type = data.pointer_type as u8;
    if let Some(position) = data.position {
        out.fields |= OXYOS_GESTURE_HAS_POSITION;
        (out.x, out.y) = (position.x, position.y);
        (out.absolute_x, out.absolute_y) = (position.absolute_x, position.absolute_y);
    }
    if let Some((x, y)) = data.translation {
        out.fields |= OXYOS_GESTURE_HAS_TRANSLATION;
        (out.translation_x, out.translation_y) = (x, y);
    }
    if let Some((x, y)) = data.velocity {
        out.fields |= OXYOS_GESTURE_HAS_VELOCITY;
        (out.velocity_x, out.velocity_y) = (x, y);
    }
    if let Some(duration) = data.duration {
        out.fields |= OXYOS_GESTURE_HAS_DURATION;
        out.duration = duration;
    }
    if let Some(inside) = data.pointer_inside {
        out.fields |= OXYOS_GESTURE_HAS_POINTER_INSIDE;
        out.pointer_inside = inside.into();
    }
}
